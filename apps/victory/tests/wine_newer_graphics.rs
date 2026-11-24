//! Normal Wine Device11 graphics and PresentX through the independently built DLL.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct ResultOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}
fn kill_group(child: &mut std::process::Child) {
    let _ = Command::new("/bin/kill").arg("-KILL").arg(format!("-{}", child.id()))
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

// Files rather than pipes prevent a verbose Wine process from blocking on a full pipe.
fn run(command: &mut Command, dir: &Path) -> ResultOutput {
    let stdout = dir.join("stdout.log");
    let stderr = dir.join("stderr.log");
    let mut child = command.process_group(0).stdout(Stdio::from(File::create(&stdout).unwrap()))
        .stderr(Stdio::from(File::create(&stderr).unwrap())).spawn().expect("spawn command");
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            panic!("command timed out: {command:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
        }
        thread::sleep(Duration::from_millis(50));
    };
    ResultOutput { status, stdout: fs::read_to_string(stdout).unwrap(), stderr: fs::read_to_string(stderr).unwrap() }
}

fn checked(command: &mut Command, dir: &Path) -> ResultOutput {
    let output = run(command, dir);
    assert!(output.status.success(), "{command:?}: {}\n{}\n{}", output.status, output.stdout, output.stderr);
    output
}

struct Fixture {
    temp: tempfile::TempDir,
    prefix: PathBuf,
    dll: PathBuf,
    llvm: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only contact the server for this fixture's private prefix, including
        // during unwinding. Reap it before TempDir removes the prefix.
        for option in ["-k", "-w"] {
            let child = Command::new("wineserver").arg(option)
                .env("WINEPREFIX", &self.prefix).process_group(0)
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            if let Ok(mut child) = child {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Err(_) => { kill_group(&mut child); break; }
                        Ok(None) if Instant::now() >= deadline => { kill_group(&mut child); break; }
                        Ok(None) => thread::sleep(Duration::from_millis(50)),
                    }
                }
            }
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let dll = fs::canonicalize(std::env::var_os("VICTORY_TEST_D3D12X_DLL").expect("set VICTORY_TEST_D3D12X_DLL to the native path of d3d12_xs.dll")).expect("DLL path");
        assert_eq!(dll.file_name().unwrap().to_string_lossy().to_ascii_lowercase(), "d3d12_xs.dll");
        let llvm = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW to the LLVM-MinGW root"));
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        Self { temp, prefix, dll, llvm }
    }

    fn winepath(&self, path: &Path) -> String {
        let mut cmd = Command::new("winepath");
        cmd.env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=")
            .env("MVK_CONFIG_LOG_LEVEL", "0").arg("-w").arg(path);
        checked(&mut cmd, self.temp.path()).stdout.trim().to_owned()
    }

    fn build(&self, invalid_flags: bool) -> PathBuf {
        let dir = self.temp.path();
        fs::write(dir.join("d3d12_xs.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\n D3D12XboxCreateDevice\n").unwrap();
        let mut dlltool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        dlltool.current_dir(dir).args(["-m", "i386:x86-64", "-d", "d3d12_xs.def", "-l", "d3d12_xs.lib"]);
        checked(&mut dlltool, dir);
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_newer_graphics.c");
        let exe = dir.join("guest.exe");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.current_dir(dir).args(["-std=c11", "-fno-emulated-tls", "-O1"]);
        if invalid_flags { clang.arg("-DINVALID_SUBMISSION_FLAGS"); }
        clang.arg(source).arg(dir.join("d3d12_xs.lib")).arg("-o").arg(&exe);
        checked(&mut clang, dir);
        exe
    }

    fn execute(&self, exe: &Path) -> ResultOutput {
        let runtime = self.winepath(&self.llvm.join("x86_64-w64-mingw32/bin"));
        let dll_dir = self.winepath(self.dll.parent().unwrap());
        let expected = self.winepath(&self.dll);
        let mut cmd = Command::new("wine");
        cmd.arg(exe).current_dir(self.temp.path())
            .env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=")
            .env("MVK_CONFIG_LOG_LEVEL", "0")
            .env("WINEPATH", format!("{dll_dir};{runtime}"))
            .env("VICTORY_EXPECT_D3D12X_DLL", expected);
        run(&mut cmd, self.temp.path())
    }
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW, real d3d12_xs.dll, Metal Shader Converter and a running presentation-enabled Metal helper in an Aqua session"]
fn normal_wine_device11_draws_and_presents() {
    for name in ["VICTORY_WINE_METAL_ADDR", "VICTORY_WINE_METAL_TOKEN"] {
        assert!(std::env::var_os(name).filter(|v: &OsString| !v.is_empty()).is_some(), "set {name}");
    }
    let fixture = Fixture::new();
    let exe = fixture.build(false);
    let output = fixture.execute(&exe);
    let logs = format!("{}\n{}", output.stdout, output.stderr);
    assert_eq!(output.status.code(), Some(42), "{logs}");
    for marker in ["Real imported D3D12X DLL path passed", "TLS compiler state and thread callbacks passed",
        "Device11 COM identity and unsupported IID passed", "QueueX and ListX descriptor and IID rejection passed",
        "ExecuteCommandListsX token and PresentX passed", "D3D12X synthetic graphics pixels and padding passed"] {
        assert!(output.stdout.contains(marker), "missing {marker}: {logs}");
    }
    let invalid = fixture.build(true);
    let rejected = fixture.execute(&invalid);
    let rejected_logs = format!("{}\n{}", rejected.stdout, rejected.stderr);
    assert!(!rejected.status.success(), "{rejected_logs}");
    assert!(rejected_logs.contains("unsupported command submission flags"), "{rejected_logs}");
    assert!(!rejected.stdout.contains("Unsupported submission flags were accepted"), "{rejected_logs}");
}
