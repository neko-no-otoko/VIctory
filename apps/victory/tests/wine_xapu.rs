//! Ordinary Wine-process integration coverage for the XApu provider.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Output { status: ExitStatus, stdout: String, stderr: String }
fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}
fn run(cmd: &mut Command, dir: &Path) -> Output {
    let stdout = dir.join("stdout.log");
    let stderr = dir.join("stderr.log");
    let mut child = cmd.process_group(0).stdout(File::create(&stdout).unwrap()).stderr(File::create(&stderr).unwrap()).spawn().expect("spawn command");
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            panic!("timed out: {cmd:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
        }
        thread::sleep(Duration::from_millis(50));
    };
    Output { status, stdout: fs::read_to_string(stdout).unwrap(), stderr: fs::read_to_string(stderr).unwrap() }
}
fn required_path(name: &str) -> PathBuf {
    fs::canonicalize(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}"))).unwrap_or_else(|e| panic!("invalid {name}: {e}"))
}
fn required_executable(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_file(), "invalid {name}: {}", path.display());
    if path.is_absolute() { path } else { std::env::current_dir().unwrap().join(path) }
}
fn quote(path: &Path) -> String { format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''")) }
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
        let f = Self { prefix, host_pid, wine: required_executable("VICTORY_TEST_WINE"),
            winepath: required_executable("VICTORY_TEST_WINEPATH"), wineserver: required_executable("VICTORY_TEST_WINESERVER"),
            host: required_path("VICTORY_TEST_METAL_HOST"), converter: required_path("VICTORY_TEST_METAL_CONVERTER"),
            dll_dir: required_path("VICTORY_TEST_DLL_DIR"), llvm: required_path("LLVM_MINGW"), temp };
        assert!(f.dll_dir.join("xapu.dll").is_file(), "missing xapu.dll");
        f
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
    fn build(&self, kind: &str) -> PathBuf {
        let dir = self.temp.path();
        let exe = dir.join(format!("{kind}.exe"));
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.args(["-std=c11", "-fno-emulated-tls", "-O1"]);
        let source = if kind == "pin" { "wine_xapu_pin.c" } else { "wine_xapu.c" };
        if kind != "pin" {
            let def = dir.join(format!("{kind}.def"));
            let lib = dir.join(format!("{kind}.lib"));
            let exports = if kind == "unsupported" { "VIctoryUnsupportedImport" } else {
                "XApuConnect\nXApuDisconnect\nXApuEnqueueCommand\nXApuDequeueResult\nXApuGetResultCount"
            };
            fs::write(&def, format!("LIBRARY xapu.dll\nEXPORTS\n{exports}\n")).unwrap();
            let mut tool = Command::new(self.llvm.join("bin/llvm-dlltool"));
            tool.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib);
            self.checked(&mut tool);
            if kind == "unsupported" { clang.arg("-DUNSUPPORTED_IMPORT"); }
            clang.arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(source)).arg(lib);
        } else {
            clang.arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(source));
        }
        clang.arg("-o").arg(&exe);
        self.checked(&mut clang);
        exe
    }
    fn build_ordinal(&self) -> PathBuf {
        let dir = self.temp.path();
        let def = dir.join("ordinal.def");
        let lib = dir.join("ordinal.lib");
        let source = dir.join("ordinal.c");
        let exe = dir.join("ordinal.exe");
        fs::write(&def, "LIBRARY xapu.dll\nEXPORTS\n XApuDisconnect @1 NONAME\n").unwrap();
        fs::write(&source, "#include <windows.h>\n__declspec(dllimport) HRESULT WINAPI XApuDisconnect(void *);\nint main(void) { return XApuDisconnect(0); }\n").unwrap();
        let mut tool = Command::new(self.llvm.join("bin/llvm-dlltool"));
        tool.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib);
        self.checked(&mut tool);
        let mut clang = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.arg(source).arg(lib).arg("-o").arg(&exe);
        self.checked(&mut clang);
        exe
    }
    fn execute(&self, exe: &Path, expected: &str) -> Output {
        self.execute_with_overrides(exe, expected, "mscoree,mshtml=")
    }
    fn execute_with_overrides(&self, exe: &Path, expected: &str, overrides: &str) -> Output {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho $$ > {}\nexec {} \"$@\"\n", quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine).arg("--winepath").arg(&self.winepath)
            .arg("--metal-host").arg(wrapper).arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search").current_dir(self.temp.path())
            .env("WINEDLLOVERRIDES", overrides).env("WINEDEBUG", "-all").env("VICTORY_WINE_METAL_PRESENT", "0")
            .env("VICTORY_EXPECT_XAPU_DLL", expected);
        run(&mut cmd, self.temp.path())
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
            if let Ok(mut child) = Command::new(&self.wineserver).arg(option).env("WINEPREFIX", &self.prefix).process_group(0).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
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
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, XApu DLL and Metal host/converter"]
fn wine_xapu_named_imports_and_pinned_dynamic_provider() {
    let f = Fixture::new();
    let named = f.build("named");
    let pin = f.build("pin");
    let expected = f.winepath(&f.dll_dir.join("xapu.dll"));
    for (exe, marker) in [(&named, "XApu named imports, TLS, Opus and queues passed"), (&pin, "XApu dynamic provider remains pinned")] {
        let out = f.execute(exe, &expected);
        assert_eq!(out.status.code(), Some(0), "{}\n{}", out.stdout, out.stderr);
        assert!(out.stdout.contains(marker), "{}\n{}", out.stdout, out.stderr);
    }
    let wrong = f.execute(&named, "Z:\\wrong\\xapu.dll");
    assert!(!wrong.status.success(), "wrong module identity accepted");
}
#[test]
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, XApu DLL and Metal host/converter"]
fn wine_xapu_unsupported_import_rejected_before_host() {
    let f = Fixture::new();
    let exe = f.build("unsupported");
    let out = f.execute(&exe, "unused");
    assert!(!out.status.success() && out.stderr.contains("does not export VIctoryUnsupportedImport"), "{}\n{}", out.stdout, out.stderr);
    assert!(!f.host_pid.exists(), "host started for unsupported import");
}

#[test]
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, XApu DLL and Metal host/converter"]
fn wine_xapu_rejects_collisions_and_unverified_ordinals() {
    let mut f = Fixture::new();
    let named = f.build("named");
    let shadow = f.temp.path().join("xapu.dll");
    fs::copy(f.dll_dir.join("xapu.dll"), &shadow).unwrap();
    let out = f.execute(&named, "unused");
    assert!(!out.status.success() && out.stderr.contains("provider DLL conflict"), "{}", out.stderr);
    assert!(!f.host_pid.exists(), "host started with XApu collision");
    fs::remove_file(shadow).unwrap();
    let ordinal = f.build_ordinal();
    let out = f.execute(&ordinal, "unused");
    assert!(!out.status.success() && out.stderr.contains("ordinal"), "{}", out.stderr);
    assert!(!f.host_pid.exists(), "host started for unverified XApu ordinal");
    let out = f.execute_with_overrides(&named, "unused", "unrelated XAPU=b");
    assert!(!out.status.success() && out.stderr.contains("conflicting WINEDLLOVERRIDES"), "{}", out.stderr);
    assert!(!f.host_pid.exists(), "host started with XApu override conflict");
    let incomplete = f.temp.path().join("incomplete-providers");
    fs::create_dir(&incomplete).unwrap();
    for name in ["d3d12_xs.dll", "xg_xs.dll", "xgameruntime.dll", "xmem.dll", "victory_xmem_backend.dll", "acphal.dll"] {
        fs::copy(f.dll_dir.join(name), incomplete.join(name)).unwrap();
    }
    f.dll_dir = incomplete;
    let out = f.execute(&named, "unused");
    assert!(!out.status.success() && out.stderr.contains("xapu.dll"), "{}", out.stderr);
    assert!(!f.host_pid.exists(), "host started without XApu provider");
}
