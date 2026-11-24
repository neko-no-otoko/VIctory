//! Normal Wine imports of the independently built GameInput DLL.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Output { status: ExitStatus, stdout: String, stderr: String }

fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").arg("-KILL").arg(format!("-{}", child.id()))
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

// Files prevent Wine diagnostics from filling a pipe and blocking the child.
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

struct Fixture { temp: tempfile::TempDir, prefix: PathBuf, dll_dir: PathBuf, llvm: PathBuf }
impl Drop for Fixture {
    fn drop(&mut self) {
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
            .expect("set VICTORY_TEST_HLE_DLL_DIR to the independently built DLL directory"))
            .expect("DLL directory");
        assert!(dll_dir.join("gameinput.dll").is_file(), "missing gameinput.dll in {}", dll_dir.display());
        let llvm = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW to the LLVM-MinGW root"));
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        Self { temp, prefix, dll_dir, llvm }
    }
    fn winepath(&self, path: &Path) -> String {
        let mut command = Command::new("winepath");
        command.env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "gameinput=n;mscoree,mshtml=")
            .arg("-w").arg(path);
        checked(&mut command, self.temp.path()).stdout.trim().to_owned()
    }
    fn build(&self) -> PathBuf {
        let dir = self.temp.path();
        // Wine 11's system32 GameInput placeholder shadows WINEPATH; use the
        // normal application-directory precedence for the native provider.
        fs::copy(self.dll_dir.join("gameinput.dll"), dir.join("gameinput.dll")).unwrap();
        // Separate aliases ensure that the linker emits both import forms for the same DLL.
        for (stem, exports) in [("named", "GameInputCreate\n"), ("ordinal", "OrdinalCreate @1 NONAME\n")] {
            let def = format!("{stem}.def");
            let lib = format!("{stem}.lib");
            fs::write(dir.join(&def), format!("LIBRARY GameInput.dll\nEXPORTS\n{exports}")).unwrap();
            let mut dlltool = Command::new(self.llvm.join("bin/llvm-dlltool"));
            dlltool.current_dir(dir).args(["-m", "i386:x86-64", "-d", &def, "-l", &lib]);
            checked(&mut dlltool, dir);
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_gameinput.c");
        let exe = dir.join("input_guest.exe");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.current_dir(dir).args(["-std=c11", "-fno-emulated-tls", "-O1"])
            .arg(source).args(["named.lib", "ordinal.lib", "-o"]).arg(&exe);
        checked(&mut clang, dir);
        let pin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_gameinput_pin.c");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.current_dir(dir).args(["-std=c11", "-fno-emulated-tls", "-O1"])
            .arg(pin).arg("-o").arg(dir.join("gameinput_pin.exe"));
        checked(&mut clang, dir);
        exe
    }
    fn execute(&self, exe: &Path, expected: &str, with_dll: bool) -> Output {
        let runtime = self.winepath(&self.llvm.join("x86_64-w64-mingw32/bin"));
        let path = if with_dll { format!("{};{runtime}", self.winepath(&self.dll_dir)) } else { runtime };
        let mut command = Command::new("wine");
        command.arg(exe).current_dir(exe.parent().unwrap()).env("WINEPREFIX", &self.prefix)
            .env("WINEDEBUG", "-all,err+module").env("WINEDLLOVERRIDES", "gameinput=n;mscoree,mshtml=")
            .env("WINEPATH", path).env("VICTORY_EXPECT_GAMEINPUT_DLL", expected);
        run(&mut command, self.temp.path())
    }
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW and an independently built GameInput.dll"]
fn normal_wine_imports_gameinput_by_name_and_ordinal() {
    let fixture = Fixture::new();
    let exe = fixture.build();
    let dll = fixture.winepath(&fixture.temp.path().join("gameinput.dll"));
    let output = fixture.execute(&exe, &dll, true);
    assert_eq!(output.status.code(), Some(0), "{}\n{}", output.stdout, output.stderr);
    assert!(output.stdout.contains("GameInput imports, singleton, readings, callbacks and pinned lifetime passed"), "{}\n{}", output.stdout, output.stderr);
    let wrong = fixture.execute(&exe, "Z:\\wrong\\GameInput.dll", true);
    assert!(!wrong.status.success(), "wrong DLL path unexpectedly accepted");
    let missing_dir = fixture.temp.path().join("without-provider");
    fs::create_dir(&missing_dir).unwrap();
    let missing_exe = missing_dir.join("input_guest.exe");
    fs::copy(&exe, &missing_exe).unwrap();
    let missing = fixture.execute(&missing_exe, &dll, false);
    let pin = fixture.execute(&fixture.temp.path().join("gameinput_pin.exe"), &dll, true);
    assert_eq!(pin.status.code(), Some(0), "{}\n{}", pin.stdout, pin.stderr);
    assert!(pin.stdout.contains("GameInput dynamic load remains pinned after FreeLibrary"), "{}\n{}", pin.stdout, pin.stderr);
    assert!(!missing.status.success(), "missing DLL unexpectedly loaded: {}\n{}", missing.stdout, missing.stderr);
    assert!(!missing.stdout.contains("GameInput imports, singleton"));
}
