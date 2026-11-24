//! Normal Wine processes importing the independently built D3D12.X DLL.
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

    fn build(&self, source: &str) -> PathBuf {
        let dir = self.temp.path();
        fs::write(dir.join("d3d12_xs.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\n D3D12XboxCreateDevice\n").unwrap();
        let mut dlltool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        dlltool.current_dir(dir).args(["-m", "i386:x86-64", "-d", "d3d12_xs.def", "-l", "d3d12_xs.lib"]);
        checked(&mut dlltool, dir);
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let compiler = self.llvm.join("bin/x86_64-w64-mingw32-clang");
        let mut dependency = Command::new(&compiler);
        dependency.current_dir(dir).args(["-std=c11", "-fno-emulated-tls", "-O1", "-shared"])
            .arg(fixtures.join("wine_process_tls_dep.c"))
            .args(["-Wl,--out-implib,wine_process_tls_dep.lib", "-o", "wine_process_tls_dep.dll"]);
        checked(&mut dependency, dir);
        let exe = dir.join("guest.exe");
        let mut clang = Command::new(compiler);
        clang.args(["-std=c11", "-fno-emulated-tls", "-O1"]).arg(fixtures.join(source))
            .arg(dir.join("d3d12_xs.lib")).arg(dir.join("wine_process_tls_dep.lib")).arg("-o").arg(&exe);
        checked(&mut clang, dir);
        exe
    }

    fn execute(&self, exe: &Path, expected: &str, with_dll: bool) -> ResultOutput {
        let runtime = self.winepath(&self.llvm.join("x86_64-w64-mingw32/bin"));
        let winepath = if with_dll { format!("{};{runtime}", self.winepath(self.dll.parent().unwrap())) } else { runtime };
        let mut cmd = Command::new("wine");
        cmd.arg(exe).current_dir(self.temp.path())
            .env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=")
            .env("MVK_CONFIG_LOG_LEVEL", "0")
            .env("WINEPATH", winepath).env("VICTORY_EXPECT_D3D12X_DLL", expected);
        run(&mut cmd, self.temp.path())
    }
}

fn assert_status(output: &ResultOutput, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}\n{}", output.stdout, output.stderr);
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW and the independently built D3D12.X DLL"]
fn normal_wine_process_loads_dll_and_runs_tls() {
    let fixture = Fixture::new();
    let exe = fixture.build("wine_process_tls.c");
    let expected = fixture.winepath(&fixture.dll);
    let output = fixture.execute(&exe, &expected, true);
    assert_status(&output, 0);
    assert!(output.stdout.contains("D3D12X DLL path, exports and error passed"), "{}", output.stdout);
    assert!(output.stdout.contains("TLS compiler state and thread callbacks passed"), "{}", output.stdout);
    assert!(output.stdout.contains("Separate DLL TLS state and thread callbacks passed"), "{}", output.stdout);
    assert!(output.stdout.contains("Wine locale callback preserved both TLS images"), "{}", output.stdout);
    assert!(output.stdout.contains("DLL TLS process detach preserved state"), "{}", output.stdout);

    let wrong = fixture.execute(&exe, "Z:\\not-the-loaded-dll\\d3d12_xs.dll", true);
    assert_status(&wrong, 20);
    assert!(wrong.stdout.contains("D3D12X DLL path mismatch"), "{}", wrong.stdout);
    let missing = fixture.execute(&exe, &expected, false);
    assert!(!missing.status.success(), "missing DLL unexpectedly loaded: {}\n{}", missing.stdout, missing.stderr);
    assert!(!missing.stdout.contains("TLS compiler state and thread callbacks passed"));
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW, D3D12.X DLL and a running native Metal proxy"]
fn normal_wine_process_runs_tls_and_metal_compute() {
    for name in ["VICTORY_WINE_METAL_ADDR", "VICTORY_WINE_METAL_TOKEN"] {
        assert!(std::env::var_os(name).filter(|v: &OsString| !v.is_empty()).is_some(), "set {name}");
    }
    let fixture = Fixture::new();
    let exe = fixture.build("wine_process_compute_tls.c");
    let expected = fixture.winepath(&fixture.dll);
    let output = fixture.execute(&exe, &expected, true);
    assert_status(&output, 42);
    for marker in ["D3D12X DLL path, exports and error passed", "TLS compiler state and thread callbacks passed", "Separate DLL TLS state and thread callbacks passed", "Wine locale callback preserved both TLS images", "D3D12X synthetic compute readback 42", "DLL TLS process detach preserved state"] {
        assert!(output.stdout.contains(marker), "{}\n{}", output.stdout, output.stderr);
    }
}
