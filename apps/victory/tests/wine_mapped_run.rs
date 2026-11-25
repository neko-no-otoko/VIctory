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

    fn build_imports(&self) {
        for (name, exports) in [
            ("kernel32", "GetStdHandle\nWriteFile\nCreateFileW\nReadFile\nSetFilePointerEx\nGetLastError\nGetCommandLineA\nGetEnvironmentVariableW\nGetCurrentProcessId\nOpenProcess\nCreateEventW\nCreateThread\nGetTickCount64\nSleep\nSetEvent\nWaitForSingleObject\nWaitForMultipleObjects\nGetExitCodeThread\nCloseHandle"),
            ("d3d12_xs", "D3D12XboxCreateDevice"),
            ("xcrdapi", "DStorageGetFactory"),
            ("xgameruntime", "QueryApiImpl"),
        ] {
            let def = self.dir.join(format!("{name}.def"));
            fs::write(&def, format!("LIBRARY {name}.dll\nEXPORTS\n{exports}\n")).unwrap();
            checked(Command::new(self.llvm.join("bin/llvm-dlltool"))
                .args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l")
                .arg(self.dir.join(format!("{name}.lib"))), &self.dir);
        }
    }

    fn build(&self) -> (PathBuf, PathBuf, PathBuf) {
        self.build_imports();
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
        self.command_for_console(guest, wine, host, "series-x")
    }

    fn command_for_console(&self, guest: &Path, wine: &Path, host: &Path, console: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_victory"));
        command.arg("wine-mapped-run").arg(guest)
            .arg("--victory-exe").arg(&self.victory)
            .arg("--wine").arg(wine).arg("--winepath").arg(&self.winepath)
            .arg("--metal-host").arg(host).arg("--converter").arg(&self.converter)
            .arg("--wine-prefix").arg(&self.prefix)
            .arg("--runtime-dir").arg(self.llvm.join("x86_64-w64-mingw32/bin"))
            .arg("--cache-dir").arg(self.dir.join("cache directory"))
            .arg("--data-dir").arg(self.dir.join("data directory"))
            .arg("--console").arg(console)
            .current_dir(&self.dir)
            .env_remove("VICTORY_CONFIG").env("WINEDLLOVERRIDES", "xcrdapi=n")
            .env("WINEPATH", "untrusted-provider-path")
            .env_remove("VICTORY_WINE_METAL_ADDR").env_remove("VICTORY_WINE_METAL_TOKEN")
            .env("VICTORY_WINE_METAL_PRESENT", "0").env_remove("VICTORY_WINE_METAL_DRAWABLE_DIR")
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

#[test]
#[ignore = "requires an Aqua session, Windows victory.exe, Wine, LLVM-MinGW and the native Metal host and converter"]
fn mapped_graphics_present_drawable_contains_guest_pixels() {
    let f = Fixture::new();
    // Reuse the same import libraries as the compute fixture.
    f.build_imports();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/windows_d3d12x_present_graphics.c");
    let object = f.dir.join("present fixture.obj");
    checked(Command::new(f.llvm.join("bin/x86_64-w64-mingw32-clang"))
        .args(["-O2", "-fno-emulated-tls", "-c"]).arg(source).arg("-o").arg(&object), &f.dir);
    let guest = f.dir.join("present guest.exe");
    checked(Command::new(f.llvm.join("bin/lld-link"))
        .args(["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/include:_tls_used"])
        .arg(format!("/out:{}", guest.display())).arg(object)
        .arg(f.dir.join("kernel32.lib")).arg(f.dir.join("d3d12_xs.lib"))
        .arg(f.llvm.join("x86_64-w64-mingw32/lib/libmingw32.a")), &f.dir);

    let capture_dir = f.dir.join("drawable captures");
    fs::create_dir(&capture_dir).unwrap();
    let output = run(f.command(&guest, &f.wine)
        .env("VICTORY_SKIP_CPU_CHECK", "1")
        .env("VICTORY_WINE_METAL_PRESENT", "1")
        .env("VICTORY_WINE_METAL_DRAWABLE_DIR", &capture_dir), &f.dir);
    let text = logs(&output);
    assert_eq!(output.status.code(), Some(42), "{text}");
    assert!(text.contains("Wine Metal proxy connected"), "{text}");
    assert!(text.contains("TLS compiler state and thread callbacks passed"), "{text}");
    assert!(text.contains("PresentX ordered graphics, source preservation and resize passed"), "{text}");
    assert!(text.contains("mapped title result: 42"), "{text}");
    for (index, x, expected) in [(0, 10, [0, 0, 255, 255]), (1, 32, [0, 255, 0, 255])] {
        let bytes = fs::read(capture_dir.join(format!("drawable-{index:08}.bgra"))).unwrap();
        assert!(bytes.len() >= 8);
        let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert!(width >= 64 && height >= 64);
        assert_eq!(bytes.len(), 8 + width * height * 4);
        let pixel = |x: usize, y: usize| {
            let at = 8 + (y * width + x) * 4;
            &bytes[at..at + 4]
        };
        assert_eq!(pixel(width * x / 64, height / 2), expected, "{text}");
        assert_eq!(pixel(width * 56 / 64, height / 2), [255, 128, 64, 0], "{text}");
    }
    assert!(!capture_dir.join("drawable-00000002.bgra").exists(), "{text}");

    let headless = run(f.command(&guest, &f.wine)
        .env("VICTORY_SKIP_CPU_CHECK", "1")
        .env("VICTORY_WINE_METAL_PRESENT", "0")
        .env_remove("VICTORY_WINE_METAL_DRAWABLE_DIR"), &f.dir);
    let text = logs(&headless);
    assert_eq!(headless.status.code(), Some(32), "{text}");
    assert!(text.contains("mapped title result: 32"), "{text}");
    assert!(!text.contains("PresentX ordered graphics, source preservation and resize passed"), "{text}");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_link = f.dir.join("repository symlink");
    std::os::unix::fs::symlink(repository, &repo_link).unwrap();
    for forbidden in [repository, repo_link.as_path()] {
        let output = run(f.command(&guest, &f.wine)
            .env("VICTORY_SKIP_CPU_CHECK", "1")
            .env("VICTORY_WINE_METAL_PRESENT", "1")
            .env("VICTORY_WINE_METAL_DRAWABLE_DIR", forbidden), &f.dir);
        let text = logs(&output);
        assert!(!output.status.success(), "{text}");
        assert!(text.contains("refusing to use") && text.contains("inside the VIctory source tree"), "{text}");
        assert!(!text.contains("title exited with") && !text.contains("mapped title result:"), "{text}");
    }
}

#[test]
#[ignore = "requires Windows victory.exe, Wine, LLVM-MinGW and native Metal host and converter"]
fn mapped_relaunch_preserves_state_and_supervision() {
    let f = Fixture::new();
    f.build_imports();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/windows_d3d12x_compute.c");
    let compute = f.dir.join("relaunch compute.obj");
    checked(Command::new(f.llvm.join("bin/x86_64-w64-mingw32-clang"))
        .args(["-O2", "-DmainCRTStartup=compute_entry", "-c"]).arg(fixture).arg("-o").arg(&compute), &f.dir);
    let source = f.dir.join("mapped relaunch.c");
    fs::write(&source, r#"
#include <windows.h>
__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);
unsigned compute_entry(void *);
typedef void (WINAPI *Launch)(void *, const char *, const char *, const void *);
typedef struct { unsigned short major, minor, build, revision; } Version;
typedef struct { Version os, hosting_os; char family[64], form[64]; } Analytics;
static int contains(const char *s, const char *needle) {
    for (; *s; ++s) { const char *a=s, *b=needle; while (*b && *a==*b) { ++a; ++b; } if (!*b) return 1; }
    return 0;
}
unsigned mainCRTStartup(void *unused) {
    const GUID title = {0x973a344e,0x24bf,0x4d0f,{0x84,0x57,0x56,0xc5,0x34,0x89,0x2b,0x29}};
    const GUID iid = {0x2549f142,0x6419,0x4a06,{0x97,0xb5,0x93,0x1a,0xab,0x7c,0x2f,0x34}};
    const GUID analytics = {0xb884675d,0xb738,0x4a9c,{0x81,0x5d,0x9a,0x9a,0x1e,0x0c,0x6c,0x9b}};
    void *object = 0; Analytics info;
    typedef Analytics *(WINAPI *GetInfo)(void *, Analytics *);
    if (QueryApiImpl(&analytics,&analytics,&object) || !object ||
        ((GetInfo)(*(void ***)object)[3])(object,&info)!=&info ||
        !contains(info.form,"Xbox Series S")) return 51;
    if (compute_entry(unused)!=42) return 52;
#ifdef INVALID_TARGET
    const WCHAR *path=L"T:\\invalid-relaunch.bin";
#else
    const WCHAR *path=L"T:\\relaunch.bin";
#endif
    WCHAR cache_path[1024];
    DWORD length=GetEnvironmentVariableW(L"VICTORY_CACHE_DIR",cache_path,1024);
    if (!length || length > 980) return 63;
#ifdef INVALID_TARGET
    const WCHAR suffix[]=L"\\invalid-relaunch-cache.bin";
#else
    const WCHAR suffix[]=L"\\relaunch-cache.bin";
#endif
    for (unsigned i=0;i<sizeof(suffix)/sizeof(suffix[0]);++i) cache_path[length+i]=suffix[i];
    HANDLE file=CreateFileW(path,GENERIC_READ|GENERIC_WRITE,0,0,OPEN_EXISTING,0,0);
    DWORD count=0, pid=0;
    if (file==INVALID_HANDLE_VALUE) {
        file=CreateFileW(path,GENERIC_WRITE,0,0,CREATE_NEW,0,0);
        if (file==INVALID_HANDLE_VALUE) return 53;
        pid=GetCurrentProcessId();
        if (!WriteFile(file,&pid,sizeof(pid),&count,0) || count!=sizeof(pid) || !CloseHandle(file)) return 54;
        HANDLE cache=CreateFileW(cache_path,GENERIC_WRITE,0,0,CREATE_NEW,0,0);
        if (cache==INVALID_HANDLE_VALUE ||
            !WriteFile(cache,&pid,sizeof(pid),&count,0) || count!=sizeof(pid) || !CloseHandle(cache)) return 64;
        object=0;
        if (QueryApiImpl(&title,&iid,&object) || !object) return 55;
#ifdef INVALID_TARGET
        ((Launch)(*(void ***)object)[4])(object,"Z:\\definitely-missing-title.exe","",0);
#else
        ((Launch)(*(void ***)object)[4])(object,"","--phase \"two words\" \"\" -x",0);
#endif
        return 56;
    }
    if (!ReadFile(file,&pid,sizeof(pid),&count,0) || count!=sizeof(pid) || !pid || !CloseHandle(file)) return 57;
    DWORD cache_pid=0;
    HANDLE cache=CreateFileW(cache_path,GENERIC_READ,0,0,OPEN_EXISTING,0,0);
    if (cache==INVALID_HANDLE_VALUE ||
        !ReadFile(cache,&cache_pid,sizeof(cache_pid),&count,0) || count!=sizeof(cache_pid) ||
        cache_pid!=pid || !CloseHandle(cache)) return 65;
    if (pid==GetCurrentProcessId()) return 58;
    HANDLE old=OpenProcess(SYNCHRONIZE,FALSE,pid);
    if (old) { DWORD state=WaitForSingleObject(old,0); CloseHandle(old); if (state!=WAIT_OBJECT_0) return 59; }
    else if (GetLastError()!=ERROR_INVALID_PARAMETER) return 60;
    if (!contains(GetCommandLineA(),"--phase \"two words\" \"\" -x")) return 61;
    static const char message[]="mapped relaunch persistence, arguments, console and prior exit passed\r\n";
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),message,sizeof(message)-1,&count,0)) return 62;
    return 256;
}
"#).unwrap();
    let mut guests = Vec::new();
    for invalid in [false, true] {
        let obj = f.dir.join(format!("relaunch-{invalid}.obj"));
        let mut compiler = Command::new(f.llvm.join("bin/x86_64-w64-mingw32-clang"));
        compiler.args(["-O2", "-c"]);
        if invalid { compiler.arg("-DINVALID_TARGET"); }
        checked(compiler.arg(&source).arg("-o").arg(&obj), &f.dir);
        let exe = f.dir.join(format!("relaunch-{invalid}.exe"));
        checked(Command::new(f.llvm.join("bin/lld-link"))
            .args(["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib"])
            .arg(format!("/out:{}", exe.display())).arg(obj).arg(&compute)
            .arg(f.dir.join("kernel32.lib")).arg(f.dir.join("xgameruntime.lib"))
            .arg(f.dir.join("d3d12_xs.lib")), &f.dir);
        guests.push(exe);
    }
    let host_wrapper = f.dir.join("observed metal host");
    let host_pids = f.dir.join("metal host pids");
    let quote = |p: &Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
    fs::write(&host_wrapper, format!("#!/bin/sh\nprintf '%s\\n' \"$$\" >> {}\nexec {} \"$@\"\n", quote(&host_pids), quote(&f.host))).unwrap();
    fs::set_permissions(&host_wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let output = run(f.command_for_console(&guests[0], &f.wine, &host_wrapper, "series-s")
        .env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
    let text = logs(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("mapped relaunch persistence, arguments, console and prior exit passed"), "{text}");
    assert_eq!(text.matches("D3D12X synthetic compute readback 42").count(), 2, "{text}");
    assert_eq!(text.matches("mapped title result:").count(), 1, "{text}");
    assert!(text.contains("mapped title result: 256"), "{text}");
    assert_eq!(fs::read(f.dir.join("data directory/00000000/temp/relaunch.bin")).unwrap().len(), 4);
    assert_eq!(fs::read(f.dir.join("cache directory/relaunch-cache.bin")).unwrap().len(), 4);
    let pids = fs::read_to_string(&host_pids).unwrap();
    let pids: Vec<i32> = pids.lines().map(|s| s.parse().unwrap()).collect();
    assert_eq!(pids.len(), 1, "{text}");
    assert_eq!(unsafe { libc::kill(pids[0], 0) }, -1, "Metal helper survived launcher exit");
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));

    let output = run(f.command_for_console(&guests[1], &f.wine, &f.host, "series-s")
        .env("VICTORY_SKIP_CPU_CHECK", "1"), &f.dir);
    let text = logs(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("mapped relaunch supports only the original title executable"), "{text}");
    assert!(!text.contains("mapped title result:"), "{text}");
}
