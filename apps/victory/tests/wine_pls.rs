//! Offline saves from normally launched Wine guests, isolated by selected root.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

unsafe extern "C" { fn flock(fd: i32, operation: i32) -> i32; }
struct Output { status: ExitStatus, stdout: String, stderr: String }
fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill(); let _ = child.wait();
}
fn run(command: &mut Command, dir: &Path) -> Output {
    let stdout = dir.join("stdout.log"); let stderr = dir.join("stderr.log");
    let mut child = command.process_group(0).stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap()).spawn().expect("spawn command");
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            panic!("timed out: {command:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
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
    let p = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(p.is_file(), "invalid {name}: {}", p.display());
    if p.is_absolute() { p } else { std::env::current_dir().unwrap().join(p) }
}
fn quote(path: &Path) -> String { format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''")) }
struct Fixture {
    temp: tempfile::TempDir, prefix: PathBuf, work: PathBuf, wine: PathBuf, winepath: PathBuf,
    wineserver: PathBuf, host: PathBuf, converter: PathBuf, dll_dir: PathBuf, llvm: PathBuf,
    marker: PathBuf, host_pid: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix"); let work = temp.path().join("work");
        fs::create_dir(&prefix).unwrap(); fs::create_dir(&work).unwrap();
        let f = Self { marker: temp.path().join("host-started"), host_pid: temp.path().join("host.pid"),
            prefix, work, wine: required_executable("VICTORY_TEST_WINE"),
            winepath: required_executable("VICTORY_TEST_WINEPATH"), wineserver: required_executable("VICTORY_TEST_WINESERVER"),
            host: required_path("VICTORY_TEST_METAL_HOST"), converter: required_path("VICTORY_TEST_METAL_CONVERTER"),
            dll_dir: required_path("VICTORY_TEST_DLL_DIR"), llvm: required_path("LLVM_MINGW"), temp };
        let mut init = Command::new(&f.winepath);
        init.arg("-w").arg(f.temp.path()).env("WINEPREFIX", &f.prefix)
            .env("WINEDEBUG", "-all").env("WINEDLLOVERRIDES", "mscoree,mshtml=");
        f.checked(&mut init); f
    }
    fn checked(&self, command: &mut Command) -> Output {
        let out = run(command, self.temp.path());
        assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr); out
    }
    fn build(&self) -> PathBuf {
        let dir = self.temp.path(); let def = dir.join("xgameruntime.def"); let lib = dir.join("xgameruntime.lib");
        fs::write(&def, "LIBRARY xgameruntime.dll\nEXPORTS\n QueryApiImpl\n").unwrap();
        let mut tool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        tool.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib); self.checked(&mut tool);
        let guest = dir.join("guest"); fs::create_dir(&guest).unwrap();
        fs::write(guest.join("MicrosoftGame.config"), "<Game><TitleId>639AA8DD</TitleId><PersistentLocalStorage><SizeMB>1</SizeMB></PersistentLocalStorage></Game>\n").unwrap();
        let exe = guest.join("guest.exe");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.args(["-std=c11", "-fno-emulated-tls", "-O1"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_pls.c"))
            .arg(lib).arg("-o").arg(&exe); self.checked(&mut clang); exe
    }
    fn tracked_host(&self) -> PathBuf {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho started > {}\necho $$ > {}\nexec {} \"$@\"\n", quote(&self.marker), quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap(); wrapper
    }
    fn command(&self, exe: &Path, host: &Path, mode: &str, root: Option<&Path>) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine).arg("--winepath").arg(&self.winepath)
            .arg("--metal-host").arg(host).arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search")
            .current_dir(&self.work).env("WINEDEBUG", "-all").env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .env("VICTORY_WINE_METAL_PRESENT", "0").env("VICTORY_PLS_TEST_MODE", mode)
            .env("VICTORY_WINE_TITLE_DATA", "Z:\\inherited-should-not-be-used")
            .env("VICTORY_WINE_PLS_DRIVE", "P");
        if let Some(root) = root { cmd.arg("--save-root").arg(root); } cmd
    }
    fn assert_host_gone(&self) {
        let pid: u32 = fs::read_to_string(&self.host_pid).expect("host PID").trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Command::new("/bin/kill").args(["-0", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success() {
            assert!(Instant::now() < deadline, "host {pid} still alive"); thread::sleep(Duration::from_millis(50));
        }
        fs::remove_file(&self.host_pid).unwrap(); fs::remove_file(&self.marker).unwrap();
    }
    fn invoke(&self, exe: &Path, host: &Path, mode: &str, root: Option<&Path>, marker: &str) {
        let mut command = self.command(exe, host, mode, root);
        if mode != "unsupported" { command.args(["--storage-drive", "P"]); }
        let out = self.checked(&mut command);
        assert!(out.stdout.contains(marker), "{}\n{}", out.stdout, out.stderr);
        self.assert_host_gone();
        assert_eq!(fs::read_dir(&self.work).unwrap().count(), 0, "guest wrote into working directory");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(&self.host_pid) {
            if let Ok(pid) = pid.trim().parse::<u32>() {
                let _ = Command::new("/bin/kill").args(["-KILL", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }
        for option in ["-k", "-w"] {
            if let Ok(mut child) = Command::new(&self.wineserver).arg(option).env("WINEPREFIX", &self.prefix)
                .process_group(0).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop { match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                    _ => { kill_group(&mut child); break; }
                }}
            }
        }
    }
}

#[test]
#[ignore = "requires configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn pls_persists_on_the_selected_drive_and_isolated_roots() {
    let f = Fixture::new(); let exe = f.build(); let host = f.tracked_host();
    let a = f.temp.path().join("save a"); let b = f.temp.path().join("save b");
    for root in [&a, &b] { fs::create_dir_all(root.join("storage")).unwrap(); }
    let manifest = exe.parent().unwrap().join("MicrosoftGame.config");
    let original = fs::read(&manifest).unwrap();
    let mapping = f.prefix.join("dosdevices/p:");
    f.invoke(&exe, &host, "unsupported", Some(&a), "PLS passed");
    for root in [&a, &b] {
        if fs::symlink_metadata(&mapping).is_ok() { fs::remove_file(&mapping).unwrap(); }
        symlink(root.join("storage"), &mapping).unwrap();
        f.invoke(&exe, &host, "write", Some(root), "PLS passed");
        assert_eq!(fs::read(root.join("storage/pls.bin")).unwrap(), b"wine persistent local storage\0");
        f.invoke(&exe, &host, "read", Some(root), "PLS passed");
    }
    fs::remove_file(&mapping).unwrap(); symlink(a.join("storage"), &mapping).unwrap();
    f.invoke(&exe, &host, "read", Some(&a), "PLS passed");
    assert_eq!(fs::read(manifest).unwrap(), original);
}

#[test]
#[ignore = "requires configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn invalid_drive_configuration_and_lock_are_rejected_before_host_start() {
    let f = Fixture::new(); let exe = f.build(); let host = f.tracked_host();
    let root = f.temp.path().join("root"); let other = f.temp.path().join("other");
    fs::create_dir(&root).unwrap(); fs::create_dir(&other).unwrap();
    let mapping = f.prefix.join("dosdevices/p:");
    let reject = |args: &[&str], save: Option<&Path>| {
        let mut cmd = f.command(&exe, &host, "write", save); cmd.args(args);
        let out = run(&mut cmd, f.temp.path());
        assert!(!out.status.success() && !f.marker.exists(), "{args:?}: {}\n{}", out.stdout, out.stderr);
    };
    reject(&["--storage-drive", "P"], None);
    for drive in ["", "PP", "1", "P:", "/", "C", "Z"] {
        reject(&["--storage-drive", drive], Some(&root));
    }
    reject(&["--storage-drive", "P"], Some(&root));
    fs::create_dir(root.join("storage")).unwrap();
    reject(&["--storage-drive", "P"], Some(&root));
    symlink(&other, &mapping).unwrap();
    reject(&["--storage-drive", "P"], Some(&root));
    fs::remove_file(&mapping).unwrap(); symlink(root.join("storage"), &mapping).unwrap();
    let duplicate = f.prefix.join("dosdevices/q:");
    symlink(root.join("storage"), &duplicate).unwrap();
    reject(&["--storage-drive", "P"], Some(&root));
    fs::remove_file(duplicate).unwrap();
    reject(&["--storage-drive", "P", "--storage-drive", "P"], Some(&root));
    let locked = File::open(&root).unwrap();
    assert_eq!(unsafe { flock(locked.as_raw_fd(), 2 | 4) }, 0);
    reject(&["--storage-drive", "P"], Some(&root));
    drop(locked);
    assert!(!root.join("storage/pls.bin").exists());
    assert_eq!(fs::read_dir(&f.work).unwrap().count(), 0);
}
