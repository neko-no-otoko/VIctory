//! Wine's native Win32 file access through an already configured title drive.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CONTENT: &[u8] = b"VIctory title-root fixture: UTF-8 \xc3\xa9\n";

struct Output { status: ExitStatus, stdout: String, stderr: String }

fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

// Redirect to files so verbose Wine output cannot fill a pipe.
fn run(command: &mut Command, dir: &Path) -> Output {
    let stdout = dir.join("stdout.log");
    let stderr = dir.join("stderr.log");
    let mut child = command.process_group(0).stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap()).spawn().expect("spawn command");
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            panic!("command timed out: {command:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
        }
        thread::sleep(Duration::from_millis(50));
    };
    Output { status, stdout: fs::read_to_string(stdout).unwrap(), stderr: fs::read_to_string(stderr).unwrap() }
}

fn required_path(name: &str) -> PathBuf {
    fs::canonicalize(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
        .unwrap_or_else(|e| panic!("invalid {name}: {e}"))
}

fn required_executable(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_file(), "invalid {name}: {}", path.display());
    if path.is_absolute() { path } else { std::env::current_dir().unwrap().join(path) }
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().expect("UTF-8 test path").replace('\'', "'\\''"))
}

struct Fixture {
    temp: tempfile::TempDir,
    prefix: PathBuf,
    root: PathBuf,
    wine: PathBuf,
    winepath: PathBuf,
    wineserver: PathBuf,
    host: PathBuf,
    converter: PathBuf,
    dll_dir: PathBuf,
    llvm: PathBuf,
    marker: PathBuf,
    host_pid: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        let root = temp.path().join("title");
        fs::create_dir(&prefix).unwrap();
        fs::create_dir_all(root.join("MiXeD")).unwrap();
        fs::write(root.join("MiXeD/Café.DAT"), CONTENT).unwrap();
        let marker = temp.path().join("host-started");
        let host_pid = temp.path().join("host.pid");
        let f = Self { prefix, root, marker, host_pid,
            wine: required_executable("VICTORY_TEST_WINE"), winepath: required_executable("VICTORY_TEST_WINEPATH"),
            wineserver: required_executable("VICTORY_TEST_WINESERVER"), host: required_path("VICTORY_TEST_METAL_HOST"),
            converter: required_path("VICTORY_TEST_METAL_CONVERTER"), dll_dir: required_path("VICTORY_TEST_DLL_DIR"),
            llvm: required_path("LLVM_MINGW"), temp };
        // winepath initializes the private prefix before the test configures its drive.
        let mut init = Command::new(&f.winepath);
        init.arg("-w").arg(f.temp.path()).env("WINEPREFIX", &f.prefix)
            .env("WINEDEBUG", "-all").env("WINEDLLOVERRIDES", "mscoree,mshtml=");
        f.checked(&mut init);
        assert!(f.prefix.join("dosdevices").is_dir());
        f
    }

    fn checked(&self, command: &mut Command) -> Output {
        let out = run(command, self.temp.path());
        assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
        out
    }

    fn build(&self) -> PathBuf {
        let exe = self.temp.path().join("guest.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_title_root.c");
        let mut cmd = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        cmd.args(["-std=c11", "-fno-emulated-tls", "-O1"]).arg(source).arg("-o").arg(&exe);
        self.checked(&mut cmd);
        exe
    }

    fn drive(&self) -> PathBuf { self.prefix.join("dosdevices/g:") }

    fn tracked_host(&self) -> PathBuf {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho started > {}\necho $$ > {}\nexec {} \"$@\"\n",
            quote(&self.marker), quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        wrapper
    }

    fn command(&self, exe: &Path, host: &Path, root: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine)
            .arg("--winepath").arg(&self.winepath).arg("--metal-host").arg(host)
            .arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--title-root").arg(root)
            .arg("--allow-unverified-dll-search").current_dir(self.temp.path())
            .env("WINEDEBUG", "-all").env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .env("VICTORY_WINE_METAL_PRESENT", "0");
        cmd
    }

    fn assert_host_gone(&self) {
        let pid: u32 = fs::read_to_string(&self.host_pid).expect("host PID").trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let alive = Command::new("/bin/kill").args(["-0", &pid.to_string()])
                .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success();
            if !alive { break; }
            assert!(Instant::now() < deadline, "host {pid} still alive after launcher exit");
            thread::sleep(Duration::from_millis(50));
        }
        fs::remove_file(&self.host_pid).unwrap();
    }

    fn assert_untouched(&self, exe: &Path, original_exe: &[u8]) {
        assert_eq!(fs::read(exe).unwrap(), original_exe);
        assert_eq!(fs::read(self.root.join("MiXeD/Café.DAT")).unwrap(), CONTENT);
        let names = |path: &Path| {
            let mut names: Vec<_> = fs::read_dir(path).unwrap().map(|e| e.unwrap().file_name()).collect();
            names.sort();
            names
        };
        assert_eq!(names(&self.root), vec![std::ffi::OsString::from("MiXeD")]);
        assert_eq!(names(&self.root.join("MiXeD")), vec![std::ffi::OsString::from("Café.DAT")]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(&self.host_pid) {
            if let Ok(pid) = pid.trim().parse::<u32>() {
                let _ = Command::new("/bin/kill").args(["-KILL", &pid.to_string()])
                    .stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }
        for option in ["-k", "-w"] {
            if let Ok(mut child) = Command::new(&self.wineserver).arg(option).env("WINEPREFIX", &self.prefix)
                .process_group(0).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                        _ => { kill_group(&mut child); break; }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn title_root_is_visible_to_native_windows_file_apis() {
    let f = Fixture::new();
    let exe = f.build();
    let original_exe = fs::read(&exe).unwrap();
    let drive = f.drive();
    symlink(&f.root, &drive).unwrap();
    let link_inode = fs::symlink_metadata(&drive).unwrap().ino();
    let target = fs::read_link(&drive).unwrap();
    let host = f.tracked_host();
    let out = f.checked(&mut f.command(&exe, &host, &f.root));
    assert!(out.stdout.contains("title-root Win32 paths, sharing and TLS passed"), "{}\n{}", out.stdout, out.stderr);
    assert!(f.marker.is_file(), "Metal host was not started");
    f.assert_host_gone();
    assert_eq!(fs::read_link(&drive).unwrap(), target);
    assert_eq!(fs::symlink_metadata(&drive).unwrap().ino(), link_inode);
    f.assert_untouched(&exe, &original_exe);
}

#[test]
#[ignore = "requires configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn invalid_title_roots_fail_before_starting_metal() {
    let f = Fixture::new();
    let exe = f.build();
    let original_exe = fs::read(&exe).unwrap();
    let host = f.tracked_host();
    let drive = f.drive();
    let other = f.temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let file = f.temp.path().join("not-a-directory");
    fs::write(&file, b"file").unwrap();
    let absent = f.temp.path().join("absent");

    // Each case has valid launcher dependencies; only the root or drive is invalid.
    for case in ["absent root", "file root", "missing drive", "mismatched drive", "dangling drive", "file drive", "non-symlink drive"] {
        if fs::symlink_metadata(&drive).is_ok() {
            if fs::symlink_metadata(&drive).unwrap().file_type().is_dir() { fs::remove_dir(&drive).unwrap(); }
            else { fs::remove_file(&drive).unwrap(); }
        }
        match case {
            "absent root" | "file root" => symlink(&f.root, &drive).unwrap(),
            "mismatched drive" => symlink(&other, &drive).unwrap(),
            "dangling drive" => symlink(&absent, &drive).unwrap(),
            "file drive" => symlink(&file, &drive).unwrap(),
            "non-symlink drive" => fs::create_dir(&drive).unwrap(),
            _ => {}
        }
        let root = match case { "absent root" => &absent, "file root" => &file, _ => &f.root };
        let before = fs::symlink_metadata(&drive).ok().map(|m| (m.ino(), m.file_type().is_symlink()));
        let target = fs::read_link(&drive).ok();
        let out = run(&mut f.command(&exe, &host, root), f.temp.path());
        assert!(!out.status.success(), "{case}: unexpectedly succeeded\n{}\n{}", out.stdout, out.stderr);
        assert!(!f.marker.exists(), "{case}: Metal host started\n{}\n{}", out.stdout, out.stderr);
        assert_eq!(fs::symlink_metadata(&drive).ok().map(|m| (m.ino(), m.file_type().is_symlink())), before, "{case}");
        assert_eq!(fs::read_link(&drive).ok(), target, "{case}");
        f.assert_untouched(&exe, &original_exe);
    }
}
