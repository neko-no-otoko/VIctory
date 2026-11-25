//! Real mapped Windows guest through Wine and the supervised native Metal host.
#![cfg(target_os = "macos")]

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn required(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("set {name}"));
    fs::canonicalize(&value).unwrap_or_else(|e| panic!("invalid {name} ({value:?}): {e}"))
}

fn required_executable(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_absolute() && path.is_file(), "invalid {name}: {}", path.display());
    // Preserve argv[0]: Wine dispatches winepath and wineserver by basename.
    path
}

fn run(command: &mut Command, dir: &Path) -> Output {
    // Files avoid deadlocking on an undrained pipe while supervising the process.
    let stdout = dir.join("command stdout.log");
    let stderr = dir.join("command stderr.log");
    let mut child = command.process_group(0)
        .stdout(File::create(&stdout).unwrap()).stderr(File::create(&stderr).unwrap())
        .spawn().unwrap_or_else(|e| panic!("{command:?}: {e}"));
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())]).status();
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out: {command:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
        }
        thread::sleep(Duration::from_millis(50));
    };
    Output { status, stdout: fs::read(stdout).unwrap(), stderr: fs::read(stderr).unwrap() }
}

fn logs(output: &Output) -> String {
    format!("status: {}\n{}\n{}", output.status, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn checked(command: &mut Command, dir: &Path) {
    let output = run(command, dir);
    assert!(output.status.success(), "{command:?}\n{}", logs(&output));
}

struct Fixture {
    temp: tempfile::TempDir,
    dir: PathBuf,
    prefix: PathBuf,
    wine: PathBuf,
    winepath: PathBuf,
    wineserver: PathBuf,
    victory: PathBuf,
    host: PathBuf,
    converter: PathBuf,
    llvm: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("mapped test with spaces");
        fs::create_dir(&dir).unwrap();
        let prefix = dir.join("Wine prefix");
        fs::create_dir(&prefix).unwrap();
        fs::create_dir(dir.join("cache directory")).unwrap();
        fs::create_dir(dir.join("data directory")).unwrap();
        Self { temp, dir, prefix, wine: required_executable("VICTORY_TEST_WINE"),
            winepath: required_executable("VICTORY_TEST_WINEPATH"), wineserver: required_executable("VICTORY_TEST_WINESERVER"),
            victory: required("VICTORY_TEST_WINDOWS_VICTORY"), host: required("VICTORY_TEST_METAL_HOST"),
            converter: required("VICTORY_TEST_METAL_CONVERTER"), llvm: required("LLVM_MINGW") }
    }

    fn build(&self) -> (PathBuf, PathBuf, PathBuf) {
        for (name, exports) in [
            ("kernel32", "GetStdHandle\nWriteFile\nCreateFileW\nReadFile\nSetFilePointerEx\nGetLastError\nCreateEventW\nCreateThread\nGetTickCount64\nSleep\nSetEvent\nWaitForSingleObject\nWaitForMultipleObjects\nGetExitCodeThread\nCloseHandle"),
            ("d3d12_xs", "D3D12XboxCreateDevice"),
            ("xcrdapi", "DStorageGetFactory"),
        ] {
            let def = self.dir.join(format!("{name}.def"));
            fs::write(&def, format!("LIBRARY {name}.dll\nEXPORTS\n{exports}\n")).unwrap();
            checked(Command::new(self.llvm.join("bin/llvm-dlltool"))
                .args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l")
                .arg(self.dir.join(format!("{name}.lib"))), &self.dir);
        }
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let obj = self.dir.join("compute fixture.obj");
        checked(Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"))
            .args(["-O2", "-fno-emulated-tls", "-DVICTORY_TLS_ENTRY=tls_entry", "-DVICTORY_COMPUTE_INPUT=streamed_input", "-c"])
            .arg(fixtures.join("windows_d3d12x_tls_compute.c")).arg("-o").arg(&obj), &self.dir);
        let wrapper = self.dir.join("canonical factory wrapper.c");
        fs::write(&wrapper, r#"
#include <windows.h>
__declspec(dllimport) HRESULT WINAPI DStorageGetFactory(const GUID *, void **);
unsigned tls_entry(void *);
unsigned streamed_input;
unsigned mainCRTStartup(void *arg) {
    const GUID factory_iid = {0x62959d11,0xd45f,0x4884,{0xa0,0xa3,0xf7,0x86,0x2e,0xa2,0x43,0xae}};
    void *factory = 0;
    if (DStorageGetFactory(&factory_iid, &factory) || !factory) return 31;
    typedef ULONG (WINAPI *Release)(void *);
    if (((Release)(*(void ***)factory)[2])(factory) != 1) return 32;
    HANDLE missing = CreateFileW(L"G:\\missing-stream.bin", GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, 0, 0);
    if (missing != INVALID_HANDLE_VALUE || GetLastError() != ERROR_FILE_NOT_FOUND) return 34;
    HANDLE file = CreateFileW(L"G:\\stream data.bin", GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, 0, 0);
    if (file == INVALID_HANDLE_VALUE) return 35;
    LARGE_INTEGER offset; offset.QuadPart = 4;
    unsigned char bytes[8] = {0xee,0xee,0xee,0xee,0xee,0xee,0xee,0xee};
    DWORD count = 0;
    if (!SetFilePointerEx(file, offset, 0, FILE_BEGIN) ||
        !ReadFile(file, bytes + 1, 6, &count, 0) || count != 6 ||
        bytes[0] != 0xee || bytes[7] != 0xee) return 36;
    for (unsigned i = 0; i < 6; ++i) if (bytes[i+1] != i+1) return 37;
    streamed_input = bytes[1];
    offset.QuadPart = 14;
    if (!SetFilePointerEx(file, offset, 0, FILE_BEGIN) ||
        !ReadFile(file, bytes, sizeof(bytes), &count, 0) || count != 2 ||
        bytes[0] != 11 || bytes[1] != 12 ||
        !ReadFile(file, bytes, sizeof(bytes), &count, 0) || count != 0 ||
        !CloseHandle(file)) return 38;
    static const char file_message[] = "G: HLE file reads and missing-file error passed\r\n";
    DWORD file_written;
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), file_message, sizeof(file_message)-1, &file_written, 0) ||
        file_written != sizeof(file_message)-1) return 39;
    static const char message[] = "DirectStorage canonical HLE factory passed\r\n";
    DWORD written;
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), message, sizeof(message)-1, &written, 0) ||
        written != sizeof(message)-1) return 33;
    return tls_entry(arg);
}
"#).unwrap();
        let wrapper_obj = self.dir.join("canonical factory wrapper.obj");
        checked(Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"))
            .args(["-O2", "-c"]).arg(wrapper).arg("-o").arg(&wrapper_obj), &self.dir);
        let exe = self.dir.join("compute guest.exe");
        checked(Command::new(self.llvm.join("bin/lld-link"))
            .args(["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/include:_tls_used"])
            .arg(format!("/out:{}", exe.display())).arg(obj).arg(wrapper_obj)
            .arg(self.dir.join("kernel32.lib")).arg(self.dir.join("d3d12_xs.lib")).arg(self.dir.join("xcrdapi.lib"))
            .arg(self.llvm.join("x86_64-w64-mingw32/lib/libmingw32.a")), &self.dir);
        fs::write(self.dir.join("stream data.bin"), [0xa0,0xa1,0xa2,0xa3,1,2,3,4,5,6,7,8,9,10,11,12]).unwrap();

        let mut failures = Vec::new();
        for code in [13, 256] {
            let fail_source = self.dir.join(format!("failure {code}.c"));
            fs::write(&fail_source, format!("static unsigned value = {code}; static unsigned *volatile ptr = &value;\nunsigned mainCRTStartup(void *unused) {{ (void)unused; return *ptr; }}\n")).unwrap();
            let fail_obj = self.dir.join(format!("failure {code}.obj"));
            checked(Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"))
                .args(["-O2", "-c"]).arg(fail_source).arg("-o").arg(&fail_obj), &self.dir);
            let fail_exe = self.dir.join(format!("failure {code}.exe"));
            checked(Command::new(self.llvm.join("bin/lld-link"))
                .args(["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib"])
                .arg(format!("/out:{}", fail_exe.display())).arg(fail_obj), &self.dir);
            failures.push(fail_exe);
        }
        (exe, failures.remove(0), failures.remove(0))
    }

    fn command(&self, guest: &Path, wine: &Path) -> Command {
        self.command_with_host(guest, wine, &self.host)
    }

    fn command_with_host(&self, guest: &Path, wine: &Path, host: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_victory"));
        command.arg("wine-mapped-run").arg(guest)
            .arg("--victory-exe").arg(&self.victory)
            .arg("--wine").arg(wine).arg("--winepath").arg(&self.winepath)
            .arg("--metal-host").arg(host).arg("--converter").arg(&self.converter)
            .arg("--wine-prefix").arg(&self.prefix)
            .arg("--runtime-dir").arg(self.llvm.join("x86_64-w64-mingw32/bin"))
            .arg("--cache-dir").arg(self.dir.join("cache directory"))
            .arg("--data-dir").arg(self.dir.join("data directory"))
            .arg("--console").arg("series-x")
            .current_dir(&self.dir)
            .env_remove("VICTORY_CONFIG").env("WINEDLLOVERRIDES", "xcrdapi=n")
            .env("WINEPATH", "untrusted-provider-path")
            .env_remove("VICTORY_WINE_METAL_ADDR").env_remove("VICTORY_WINE_METAL_TOKEN")
            .env("WINEDEBUG", "-all").env("VICTORY_LOG", "info")
            .env_remove("VICTORY_SKIP_CPU_CHECK");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Stop the prefix's Wine server before tempfile removes the prefix.
        let _ = Command::new(&self.wineserver).arg("-k").env("WINEPREFIX", &self.prefix)
            .stdout(Stdio::null()).stderr(Stdio::null()).status();
        let _ = &self.temp;
    }
}

#[test]
#[ignore = "requires a Windows victory.exe, Wine, LLVM-MinGW and the native Metal host and converter"]
fn mapped_windows_guest_uses_hle_and_real_metal() {
    let f = Fixture::new();
    let (guest, failure, failure_256) = f.build();

    // Observe the environment at the actual Wine boundary, then exec real Wine.
    let wrapper = f.dir.join("wine observing wrapper");
    let observed = f.dir.join("wine environment");
    let quote = |p: &Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
    fs::write(&wrapper, format!("#!/bin/sh\nprintf '%s\\n%s\\n%s\\n' \"${{VICTORY_SKIP_CPU_CHECK-unset}}\" \"${{WINEDLLOVERRIDES-unset}}\" \"${{WINEPATH-unset}}\" > {}\nexec {} \"$@\"\n", quote(&observed), quote(&f.wine))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    let output = run(f.command(&guest, &wrapper).env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
    let text = logs(&output);
    assert_eq!(output.status.code(), Some(42), "{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| line.contains("bound          compute guest.exe:") && line.contains("4 hle,") && line.contains("0 missing")), "{text}");
    assert!(text.contains("Wine Metal proxy connected"), "{text}");
    assert!(stdout.contains("DirectStorage canonical HLE factory passed"), "{text}");
    assert!(stdout.contains("G: HLE file reads and missing-file error passed"), "{text}");
    assert!(matches!(fs::symlink_metadata(f.prefix.join("dosdevices/g:")), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
        "guest must use VIctory's virtual G:, not a Wine drive");
    assert!(stdout.contains("TLS compiler state and thread callbacks passed"), "{text}");
    assert!(stdout.contains("D3D12X synthetic compute readback 42"), "{text}");
    assert!(stdout.contains("title exited with 42"), "{text}");
    assert!(stdout.contains("mapped title result: 42"), "{text}");
    assert!(!text.contains("dispatch without a host pipeline dropped"), "{text}");
    let env = fs::read_to_string(&observed).unwrap();
    let lines: Vec<_> = env.lines().collect();
    assert_eq!(lines.len(), 3, "{env:?}");
    assert_eq!(&lines[..2], ["1", "unset"]);
    assert!(!lines[2].contains(';') && lines[2].ends_with("\\bin") && !lines[2].contains("untrusted-provider-path"), "{env:?}");

    let failed = run(f.command(&failure, &f.wine).env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
    let text = logs(&failed);
    assert_eq!(failed.status.code(), Some(13), "{text}");
    assert!(text.contains("title exited with 13"), "{text}");
    assert!(text.contains("mapped title result: 13"), "{text}");
    assert!(!text.contains("D3D12X synthetic compute readback 42"), "{text}");
    let failed = run(f.command(&failure_256, &f.wine).env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
    let text = logs(&failed);
    assert!(!failed.status.success() && text.contains("mapped title result: 256"), "{text}");

    for missing in [f.dir.join("missing guest.exe"), f.dir.join("invalid guest.exe")] {
        if missing.ends_with("invalid guest.exe") { fs::write(&missing, b"not a PE").unwrap(); }
        let output = run(f.command(&missing, &f.wine).env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
        let text = logs(&output);
        assert!(!output.status.success(), "{text}");
        assert!(!text.contains("title exited with"), "{text}");
        assert!(!text.contains("D3D12X synthetic compute readback 42"), "{text}");
    }

    let mut no_host = f.command_with_host(&guest, &f.wine, &f.dir.join("missing host"));
    let output = run(&mut no_host, &f.dir);
    let text = logs(&output);
    assert!(!output.status.success(), "{text}");
    assert!(!text.contains("title exited with"), "{text}");
    assert!(!text.contains("D3D12X synthetic compute readback 42"), "{text}");

    // No implicit bypass: inspect what the launcher passes to Wine when the
    // caller deliberately removes the opt-in, regardless of CPU capabilities.
    fs::remove_file(&observed).unwrap();
    let output = run(f.command(&failure, &wrapper).env_remove("VICTORY_SKIP_CPU_CHECK"), &f.dir);
    let env = fs::read_to_string(&observed).unwrap();
    let lines: Vec<_> = env.lines().collect();
    assert_eq!(lines.len(), 3, "{}\n{env:?}", logs(&output));
    assert_eq!(&lines[..2], ["unset", "unset"]);
    assert!(!lines[2].contains(';') && lines[2].ends_with("\\bin") && !lines[2].contains("untrusted-provider-path"), "{env:?}");
}
