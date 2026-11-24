//! Real normal-Wine title-provider relaunch through the supervised launcher.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Output { status: ExitStatus, stdout: String, stderr: String }

fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

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
fn quote(path: &Path) -> String { format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''")) }

struct Fixture {
    temp: tempfile::TempDir,
    prefix: PathBuf,
    wine: PathBuf,
    winepath: PathBuf,
    wineserver: PathBuf,
    host: PathBuf,
    converter: PathBuf,
    dll_dir: PathBuf,
    llvm: PathBuf,
    host_pid: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        fs::create_dir(&prefix).unwrap();
        let host_pid = temp.path().join("host.pid");
        Self { prefix, host_pid, wine: required_executable("VICTORY_TEST_WINE"),
            winepath: required_executable("VICTORY_TEST_WINEPATH"),
            wineserver: required_executable("VICTORY_TEST_WINESERVER"),
            host: required_path("VICTORY_TEST_METAL_HOST"),
            converter: required_path("VICTORY_TEST_METAL_CONVERTER"),
            dll_dir: required_path("VICTORY_TEST_DLL_DIR"), llvm: required_path("LLVM_MINGW"), temp }
    }
    fn checked(&self, command: &mut Command) -> Output {
        let out = run(command, self.temp.path());
        assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
        out
    }
    fn windows_path(&self, path: &Path) -> String {
        let mut command = Command::new(&self.winepath);
        command.arg("-w").arg(path).env("WINEPREFIX", &self.prefix)
            .env("WINEDEBUG", "-all").env("WINEDLLOVERRIDES", "mscoree,mshtml=");
        self.checked(&mut command).stdout.trim().to_owned()
    }
    fn build(&self) -> (PathBuf, PathBuf) {
        let dir = self.temp.path();
        let def = dir.join("xgameruntime.def");
        let lib = dir.join("xgameruntime.lib");
        fs::write(&def, "LIBRARY xgameruntime.dll\nEXPORTS\n QueryApiImpl\n").unwrap();
        let mut dlltool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        dlltool.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib);
        self.checked(&mut dlltool);
        let first = dir.join("first.exe");
        let second = dir.join("second.exe");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.args(["-std=c11", "-O1"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_relaunch.c"))
            .arg(lib).arg("-o").arg(&first);
        self.checked(&mut clang);
        fs::copy(&first, &second).unwrap();
        (first, second)
    }
    fn tracked_host(&self) -> PathBuf {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho $$ > {}\nexec {} \"$@\"\n", quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        wrapper
    }
    fn command(&self, first: &Path, target: &Path, marker: &Path) -> Command {
        let target = self.windows_path(target);
        let marker = self.windows_path(marker);
        let host = self.tracked_host();
        let mut command = Command::new(env!("CARGO_BIN_EXE_victory"));
        command.arg("wine-run").arg(first).arg("--wine").arg(&self.wine)
            .arg("--winepath").arg(&self.winepath).arg("--metal-host").arg(host)
            .arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search")
            .current_dir(self.temp.path()).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .env("VICTORY_WINE_METAL_PRESENT", "0")
            .env("VICTORY_RELAUNCH_TEST_TARGET", target).env("VICTORY_RELAUNCH_TEST_MARKER", marker);
        command
    }
    fn assert_host_gone(&self) {
        let pid: u32 = fs::read_to_string(&self.host_pid).expect("host was started").trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Command::new("/bin/kill").args(["-0", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success() {
            assert!(Instant::now() < deadline, "host {pid} still alive");
            thread::sleep(Duration::from_millis(50));
        }
        fs::remove_file(&self.host_pid).unwrap();
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
fn title_provider_relaunches_real_wine_guest_and_returns_final_status() {
    let f = Fixture::new();
    let (first, second) = f.build();
    let marker = f.temp.path().join("sentinel");
    let out = run(&mut f.command(&first, &second, &marker), f.temp.path());
    assert_eq!(out.status.code(), Some(42), "{}\n{}", out.stdout, out.stderr);
    let first_marker = out.stdout.find("relaunch first guest sentinel written");
    let second_marker = out.stdout.find("relaunch second guest image, arguments and sentinel passed");
    assert!(matches!((first_marker, second_marker), (Some(a), Some(b)) if a < b), "{}\n{}", out.stdout, out.stderr);
    assert_eq!(fs::read(marker).unwrap(), b"first guest completed before relaunch\0");
    f.assert_host_gone();
}

#[test]
#[ignore = "requires configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn nonexistent_relaunch_target_fails_after_first_guest() {
    let f = Fixture::new();
    let (first, _) = f.build();
    let marker = f.temp.path().join("sentinel");
    let out = run(&mut f.command(&first, &f.temp.path().join("missing.exe"), &marker), f.temp.path());
    assert!(!out.status.success(), "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("relaunch first guest sentinel written"), "{}\n{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("relaunch second guest image"), "{}\n{}", out.stdout, out.stderr);
    assert_eq!(fs::read(marker).unwrap(), b"first guest completed before relaunch\0");
    f.assert_host_gone();
}
