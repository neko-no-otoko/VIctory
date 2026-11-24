//! End-to-end XMem imports in an ordinary Wine process launched by VIctory.
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
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().expect("UTF-8 test path").replace('\'', "'\\''"))
}

struct Fixture {
    temp: tempfile::TempDir, prefix: PathBuf, wine: PathBuf, winepath: PathBuf,
    wineserver: PathBuf, host: PathBuf, converter: PathBuf, dll_dir: PathBuf,
    llvm: PathBuf, host_pid: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        fs::create_dir(&prefix).unwrap();
        let host_pid = temp.path().join("host.pid");
        let fixture = Self { prefix, host_pid, wine: required_executable("VICTORY_TEST_WINE"),
            winepath: required_executable("VICTORY_TEST_WINEPATH"),
            wineserver: required_executable("VICTORY_TEST_WINESERVER"),
            host: required_path("VICTORY_TEST_METAL_HOST"),
            converter: required_path("VICTORY_TEST_METAL_CONVERTER"),
            dll_dir: required_path("VICTORY_TEST_DLL_DIR"), llvm: required_path("LLVM_MINGW"), temp };
        for name in ["xmem.dll", "victory_xmem_backend.dll"] {
            assert!(fixture.dll_dir.join(name).is_file(), "missing {name} in {}", fixture.dll_dir.display());
        }
        fixture
    }
    fn checked(&self, cmd: &mut Command) -> Output {
        let out = run(cmd, self.temp.path());
        assert!(out.status.success(), "{cmd:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
        out
    }
    fn winepath(&self, path: &Path) -> String {
        let mut cmd = Command::new(&self.winepath);
        cmd.env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all").arg("-w").arg(path);
        self.checked(&mut cmd).stdout.trim().to_owned()
    }
    fn build(&self, unsupported: bool) -> PathBuf {
        let dir = self.temp.path();
        let (stem, exports) = if unsupported {
            ("unsupported", "VIctoryUnsupportedImport")
        } else {
            ("named", "XMemAlloc\nXMemFree\nXMemAllocDefault\nXMemFreeDefault\nXMemIsToolsMemAvailable\nXMemAllocatePhysicalPages\nXMemFreePhysicalPages\nXMemMapPhysicalPages\nXMemMapPhysicalPagesScatter\nXMemVirtualAlloc\nXMemVirtualQuery\nXMemGetWorkingSetStatistics\nXMemTransferMemory")
        };
        let def = dir.join(format!("{stem}.def"));
        let lib = dir.join(format!("{stem}.lib"));
        fs::write(&def, format!("LIBRARY xmem.dll\nEXPORTS\n{exports}\n")).unwrap();
        let mut tool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        tool.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib);
        self.checked(&mut tool);
        let exe = dir.join(format!("{stem}.exe"));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_xmem.c");
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.args(["-std=c11", "-fno-emulated-tls", "-O1"]);
        if unsupported { clang.arg("-DUNSUPPORTED_IMPORT"); }
        clang.arg(source).arg(lib).arg("-o").arg(&exe);
        self.checked(&mut clang);
        exe
    }
    fn execute(&self, exe: &Path, xmem: &str, backend: &str) -> Output {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho $$ > {}\nexec {} \"$@\"\n", quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine)
            .arg("--winepath").arg(&self.winepath).arg("--metal-host").arg(wrapper)
            .arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search")
            .current_dir(self.temp.path()).env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .env("WINEDEBUG", "-all").env("VICTORY_WINE_METAL_PRESENT", "0")
            .env("VICTORY_EXPECT_XMEM_DLL", xmem).env("VICTORY_EXPECT_XMEM_BACKEND_DLL", backend);
        run(&mut cmd, self.temp.path())
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
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, XMem provider DLLs and Metal host/converter"]
fn wine_launcher_imports_xmem_and_shares_backend_page_state() {
    let fixture = Fixture::new();
    let exe = fixture.build(false);
    let unsupported = fixture.build(true);
    let dll = fixture.winepath(&fixture.dll_dir.join("xmem.dll"));
    let backend = fixture.winepath(&fixture.dll_dir.join("victory_xmem_backend.dll"));
    let out = fixture.execute(&exe, &dll, &backend);
    assert_eq!(out.status.code(), Some(0), "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("XMem named imports, TLS, hooks, virtual memory and shared physical aliases passed"), "{}\n{}", out.stdout, out.stderr);
    let wrong = fixture.execute(&exe, "Z:\\wrong\\xmem.dll", &backend);
    assert!(!wrong.status.success(), "wrong module identity accepted: {}\n{}", wrong.stdout, wrong.stderr);
    let missing = fixture.execute(&unsupported, &dll, &backend);
    assert!(!missing.status.success(), "unsupported import unexpectedly succeeded: {}\n{}", missing.stdout, missing.stderr);
}
