//! Normal Wine process importing the independently built XG and runtime DLLs.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Output {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").arg("-KILL").arg(format!("-{}", child.id()))
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

// Files avoid deadlock when Wine emits more than a pipe can hold.
fn run(command: &mut Command, dir: &Path) -> Output {
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
    Output { status, stdout: fs::read_to_string(stdout).unwrap(), stderr: fs::read_to_string(stderr).unwrap() }
}

fn checked(command: &mut Command, dir: &Path) -> Output {
    let out = run(command, dir);
    assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
    out
}

struct Fixture {
    temp: tempfile::TempDir,
    prefix: PathBuf,
    dll_dir: PathBuf,
    llvm: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Stop only this fixture's Wine server before its prefix is removed.
        for option in ["-k", "-w"] {
            if let Ok(mut child) = Command::new("wineserver").arg(option)
                .env("WINEPREFIX", &self.prefix).process_group(0)
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
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
        let dll_dir = fs::canonicalize(std::env::var_os("VICTORY_TEST_HLE_DLL_DIR")
            .expect("set VICTORY_TEST_HLE_DLL_DIR to the native directory containing the provider DLLs"))
            .expect("provider DLL directory");
        assert!(dll_dir.is_dir(), "provider DLL path is not a directory");
        for name in ["xg_xs.dll", "xgameruntime.dll"] {
            assert!(dll_dir.join(name).is_file(), "missing {}", dll_dir.join(name).display());
        }
        let llvm = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW to the LLVM-MinGW root"));
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        Self { temp, prefix, dll_dir, llvm }
    }

    fn winepath(&self, path: &Path) -> String {
        let mut command = Command::new("winepath");
        command.env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "xg_xs,xgameruntime=n;mscoree,mshtml=")
            .arg("-w").arg(path);
        checked(&mut command, self.temp.path()).stdout.trim().to_owned()
    }

    fn build(&self) -> PathBuf {
        let dir = self.temp.path();
        for (name, exports) in [
            ("xg_xs", "XGCreateTextureComputer\nXGSuggestSwizzleMode\n"),
            ("xgameruntime", "QueryApiImpl\n"),
        ] {
            let def = format!("{name}.def");
            let lib = format!("{name}.lib");
            fs::write(dir.join(&def), format!("LIBRARY {name}.dll\nEXPORTS\n{exports}")).unwrap();
            let mut dlltool = Command::new(self.llvm.join("bin/llvm-dlltool"));
            dlltool.current_dir(dir).args(["-m", "i386:x86-64", "-d", &def, "-l", &lib]);
            checked(&mut dlltool, dir);
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_process_providers.c");
        let exe = dir.join("providers.exe");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.current_dir(dir).args(["-std=c11", "-fno-emulated-tls", "-O1"])
            .arg(source).args(["xg_xs.lib", "xgameruntime.lib", "-o"]).arg(&exe);
        checked(&mut clang, dir);
        exe
    }

    fn execute(&self, exe: &Path, xg: &str, runtime: &str, with_dlls: bool) -> Output {
        let llvm_runtime = self.winepath(&self.llvm.join("x86_64-w64-mingw32/bin"));
        let winepath = if with_dlls { format!("{};{llvm_runtime}", self.winepath(&self.dll_dir)) } else { llvm_runtime };
        let mut command = Command::new("wine");
        command.arg(exe).current_dir(self.temp.path())
            .env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "xg_xs,xgameruntime=n;mscoree,mshtml=")
            .env("WINEPATH", winepath).env("VICTORY_EXPECT_XG_DLL", xg)
            .env("VICTORY_EXPECT_RUNTIME_DLL", runtime);
        run(&mut command, self.temp.path())
    }
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW and independently built XG and XGameRuntime DLLs"]
fn normal_wine_process_imports_real_providers() {
    let fixture = Fixture::new();
    let exe = fixture.build();
    let xg = fixture.winepath(&fixture.dll_dir.join("xg_xs.dll"));
    let runtime = fixture.winepath(&fixture.dll_dir.join("xgameruntime.dll"));
    let output = fixture.execute(&exe, &xg, &runtime, true);
    assert_eq!(output.status.code(), Some(0), "{}\n{}", output.stdout, output.stderr);
    for marker in ["Provider DLL identities and exports passed", "XG texture computer lifetime passed",
        "XSystem and XUser providers passed", "Provider threads and static TLS callbacks passed"] {
        assert!(output.stdout.contains(marker), "missing {marker}:\n{}\n{}", output.stdout, output.stderr);
    }
    for (wrong_xg, wrong_runtime) in [("Z:\\wrong\\xg_xs.dll", runtime.as_str()), (xg.as_str(), "Z:\\wrong\\xgameruntime.dll")] {
        let wrong = fixture.execute(&exe, wrong_xg, wrong_runtime, true);
        assert_eq!(wrong.status.code(), Some(20), "{}\n{}", wrong.stdout, wrong.stderr);
        assert!(wrong.stdout.contains("Provider DLL path or exports mismatch"));
    }
    let missing = fixture.execute(&exe, &xg, &runtime, false);
    assert!(!missing.status.success(), "missing DLLs unexpectedly loaded: {}\n{}", missing.stdout, missing.stderr);
    assert!(!missing.stdout.contains("Provider DLL identities and exports passed"));
}
