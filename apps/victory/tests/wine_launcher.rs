//! End-to-end launcher tests using an isolated Wine prefix and the real provider DLLs.
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

// Log to files so neither Wine nor the host can block on an undrained pipe.
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
            dll_dir: required_path("VICTORY_TEST_DLL_DIR"),
            llvm: required_path("LLVM_MINGW"), temp }
    }

    fn checked(&self, command: &mut Command) -> Output {
        let out = run(command, self.temp.path());
        assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
        out
    }

    fn winepath(&self, path: &Path) -> String {
        let mut cmd = Command::new(&self.winepath);
        cmd.env("WINEPREFIX", &self.prefix).env("WINEDEBUG", "-all").arg("-w").arg(path);
        self.checked(&mut cmd).stdout.trim().to_owned()
    }

    fn build(&self) -> PathBuf {
        let dir = self.temp.path();
        let exports = [
            ("d3d12_xs", "D3D12XboxCreateDevice"),
            ("xg_xs", "XGCreateTextureComputer\n XGSuggestSwizzleMode"),
            ("xgameruntime", "QueryApiImpl"),
        ];
        for (name, symbols) in exports {
            let def = dir.join(format!("{name}.def"));
            fs::write(&def, format!("LIBRARY {name}.dll\nEXPORTS\n {symbols}\n")).unwrap();
            let mut cmd = Command::new(self.llvm.join("bin/llvm-dlltool"));
            cmd.args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(dir.join(format!("{name}.lib")));
            self.checked(&mut cmd);
        }
        let exe = dir.join("guest.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_tls_compute.c");
        let mut cmd = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        cmd.args(["-std=c11", "-fno-emulated-tls", "-O1"]).arg(source);
        for (name, _) in exports { cmd.arg(dir.join(format!("{name}.lib"))); }
        cmd.arg("-o").arg(&exe);
        self.checked(&mut cmd);
        exe
    }

    // The wrapper records the actual host PID, then replaces itself with the host.
    fn tracked_host(&self) -> PathBuf {
        let wrapper = self.temp.path().join("host-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\necho $$ > {}\nexec {} \"$@\"\n", quote(&self.host_pid), quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        wrapper
    }

    fn command(&self, exe: &Path, host: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine)
            .arg("--winepath").arg(&self.winepath).arg("--metal-host").arg(host)
            .arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix)
            .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .current_dir(self.temp.path()).env("WINEDEBUG", "-all")
            .env("VICTORY_WINE_METAL_ADDR", "invalid-inherited-address")
            .env("VICTORY_WINE_METAL_TOKEN", "invalid-inherited-token")
            .env("VICTORY_WINE_METAL_PRESENT", "0")
            .env("VICTORY_TEST_LAUNCHER_INHERITED", "preserved");
        cmd
    }

    fn assert_host_gone(&self) {
        let pid = fs::read_to_string(&self.host_pid).expect("host was started");
        let pid: u32 = pid.trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let alive = Command::new("/bin/kill").args(["-0", &pid.to_string()])
                .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success();
            if !alive { break; }
            assert!(Instant::now() < deadline, "host {pid} still alive after launcher exit");
            thread::sleep(Duration::from_millis(50));
        }
        fs::remove_file(&self.host_pid).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // A timeout or panic must not leave this test's host or private server behind.
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
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn launcher_runs_real_provider_tls_and_metal_compute() {
    let f = Fixture::new();
    let exe = f.build();
    let host = f.tracked_host();
    let mut cmd = f.command(&exe, &host);
    for (name, dll) in [("VICTORY_EXPECT_D3D12X_DLL", "d3d12_xs.dll"),
        ("VICTORY_EXPECT_XG_DLL", "xg_xs.dll"), ("VICTORY_EXPECT_RUNTIME_DLL", "xgameruntime.dll")] {
        cmd.env(name, f.winepath(&f.dll_dir.join(dll)));
    }
    cmd.arg("--").args(["argument with spaces", "--guest-option"]);
    let out = run(&mut cmd, f.temp.path());
    assert_eq!(out.status.code(), Some(42), "{}\n{}", out.stdout, out.stderr);
    for marker in ["D3D12X DLL identity and launcher environment passed", "Provider DLL identities and exports passed",
        "XG texture computer lifetime passed", "XSystem and XUser providers passed",
        "Provider threads and static TLS callbacks passed", "D3D12X synthetic compute readback 42"] {
        assert!(out.stdout.contains(marker), "missing {marker}\n{}\n{}", out.stdout, out.stderr);
    }
    f.assert_host_gone();
}

#[test]
#[ignore = "requires explicitly configured Wine, provider DLLs and native Metal host/converter"]
fn launcher_rejects_bad_paths_and_cleans_up_failures() {
    let f = Fixture::new();
    let missing = f.temp.path().join("missing.exe");
    let host = f.tracked_host();
    let out = run(&mut f.command(&missing, &host), f.temp.path());
    assert!(!out.status.success(), "missing guest accepted");
    assert!(!out.stderr.trim().is_empty(), "missing guest produced no diagnostic");
    assert!(!f.host_pid.exists(), "host started for missing guest");

    let invalid = f.temp.path().join("invalid.exe");
    fs::write(&invalid, b"not a PE executable\n").unwrap();
    let out = run(&mut f.command(&invalid, &host), f.temp.path());
    assert!(!out.status.success(), "invalid guest accepted: {}\n{}", out.stdout, out.stderr);
    assert!(!f.host_pid.exists(), "host started for invalid guest");

    let exe = f.build();
    for setting in ["xg_xs=b", "unrelated xg_xs=b", "unrelated\txg_xs=b",
                    "*xg_xs=b", r"C:\windows\system32\XG_XS.dll=b"] {
        let mut conflicting = f.command(&exe, &host);
        conflicting.env("WINEDLLOVERRIDES", setting);
        let out = run(&mut conflicting, f.temp.path());
        assert!(!out.status.success(), "conflicting provider override accepted: {setting}");
        assert!(!out.stderr.trim().is_empty(), "conflicting override produced no diagnostic");
        assert!(!f.host_pid.exists(), "host started for conflicting override");
    }

    let shadow = f.temp.path().join("xg_xs.dll");
    fs::copy(f.dll_dir.join("xg_xs.dll"), &shadow).unwrap();
    let out = run(&mut f.command(&exe, &host), f.temp.path());
    assert!(!out.status.success(), "executable-directory provider shadow accepted");
    assert!(!out.stderr.trim().is_empty(), "provider shadow produced no diagnostic");
    assert!(!f.host_pid.exists(), "host started for provider shadow");
    fs::remove_file(shadow).unwrap();

    // NONAME makes dlltool emit a genuine import-by-ordinal thunk.
    let ordinal_def = f.temp.path().join("ordinal.def");
    let ordinal_lib = f.temp.path().join("ordinal.lib");
    let ordinal_source = f.temp.path().join("ordinal.c");
    let ordinal_exe = f.temp.path().join("ordinal.exe");
    fs::write(&ordinal_def, "LIBRARY xgameruntime.dll\nEXPORTS\n QueryApiImpl @1 NONAME\n").unwrap();
    fs::write(&ordinal_source, "#include <windows.h>\n__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);\nint main(void) { return (int)QueryApiImpl(0, 0, 0); }\n").unwrap();
    let mut dlltool = Command::new(f.llvm.join("bin/llvm-dlltool"));
    dlltool.args(["-m", "i386:x86-64", "-d"]).arg(&ordinal_def).arg("-l").arg(&ordinal_lib);
    f.checked(&mut dlltool);
    let mut compiler = Command::new(f.llvm.join("bin/x86_64-w64-mingw32-clang"));
    compiler.arg(&ordinal_source).arg(&ordinal_lib).arg("-o").arg(&ordinal_exe);
    f.checked(&mut compiler);
    let out = run(&mut f.command(&ordinal_exe, &host), f.temp.path());
    assert!(!out.status.success(), "provider ordinal import accepted");
    assert!(out.stderr.to_ascii_lowercase().contains("ordinal"), "{}\n{}", out.stdout, out.stderr);
    assert!(!f.host_pid.exists(), "host started for provider ordinal import");

    let out = run(&mut f.command(&exe, &f.temp.path().join("missing-host")), f.temp.path());
    assert!(!out.status.success(), "missing host accepted");
    assert!(!out.stderr.trim().is_empty(), "missing host produced no diagnostic");
    let out = run(&mut f.command(&exe, &host), f.temp.path());
    assert_eq!(out.status.code(), Some(24), "{}\n{}", out.stdout, out.stderr);
    f.assert_host_gone();

    let broken = f.temp.path().join("broken-host");
    fs::write(&broken, "#!/bin/sh\nexit 17\n").unwrap();
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o755)).unwrap();
    let out = run(&mut f.command(&exe, &broken), f.temp.path());
    assert!(!out.status.success(), "failed host accepted");
    assert!(!out.stderr.trim().is_empty(), "host failure produced no diagnostic");
}

#[test]
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn launcher_stops_wine_when_the_real_host_exits() {
    let f = Fixture::new();
    let exe = f.build();
    let ready = f.temp.path().join("guest-ready");
    let wrapper = f.temp.path().join("failing-host");
    // Wait until the PE guest itself is running before terminating its host.
    fs::write(&wrapper, format!(
        "#!/bin/sh\necho $$ > {}\n( i=0; while [ ! -f {} ] && [ \"$i\" -lt 60 ]; do sleep 1; i=$((i+1)); done; if [ -f {} ]; then kill -TERM $$; fi ) >/dev/null 2>&1 &\nexec {} \"$@\"\n",
        quote(&f.host_pid), quote(&ready), quote(&ready), quote(&f.host))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let mut cmd = f.command(&exe, &wrapper);
    cmd.env("VICTORY_TEST_GUEST_READY", f.winepath(&ready)).arg("--").arg("--wait-for-host-exit");
    let out = run(&mut cmd, f.temp.path());
    assert!(ready.exists(), "guest never started: {}\n{}", out.stdout, out.stderr);
    assert!(!out.status.success(), "host exit accepted: {}\n{}", out.stdout, out.stderr);
    assert!(out.stderr.contains("Metal host exited"), "{}\n{}", out.stdout, out.stderr);
    f.assert_host_gone();
}

#[test]
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn sigterm_stops_the_guest_and_real_host() {
    struct Launcher(Option<Child>);
    impl Drop for Launcher {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 { kill_group(child); }
        }
    }

    let f = Fixture::new();
    let exe = f.build();
    let ready = f.temp.path().join("guest-ready");
    let host = f.tracked_host();
    let stdout = f.temp.path().join("signal-stdout.log");
    let stderr = f.temp.path().join("signal-stderr.log");
    let mut cmd = f.command(&exe, &host);
    cmd.env("VICTORY_TEST_GUEST_READY", f.winepath(&ready))
        .arg("--").arg("--wait-for-host-exit")
        .process_group(0).stdout(File::create(&stdout).unwrap()).stderr(File::create(&stderr).unwrap());
    let mut launcher = Launcher(Some(cmd.spawn().expect("spawn launcher")));
    let deadline = Instant::now() + Duration::from_secs(180);
    while !ready.exists() {
        if let Some(status) = launcher.0.as_mut().unwrap().try_wait().unwrap() {
            launcher.0.take();
            panic!("launcher exited before guest started: {status}\n{}\n{}",
                fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
        }
        assert!(Instant::now() < deadline, "guest did not become ready");
        thread::sleep(Duration::from_millis(50));
    }

    let pid = launcher.0.as_ref().unwrap().id().to_string();
    assert!(Command::new("/bin/kill").args(["-TERM", &pid]).status().unwrap().success(),
        "could not signal launcher {pid}");
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = launcher.0.as_mut().unwrap().try_wait().unwrap() {
            launcher.0.take();
            break status;
        }
        assert!(Instant::now() < deadline, "launcher did not exit after SIGTERM");
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(143), "{}\n{}",
        fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
    f.assert_host_gone();
}

#[test]
#[ignore = "requires explicitly configured Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn concurrent_launchers_use_distinct_ports_and_tokens() {
    struct Launcher(Option<Child>);
    impl Drop for Launcher {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 { kill_group(child); }
        }
    }

    let fixtures = [Fixture::new(), Fixture::new()];
    let exes: Vec<_> = fixtures.iter().map(Fixture::build).collect();
    let ready: Vec<_> = fixtures.iter().map(|f| f.temp.path().join("guest-ready")).collect();
    let hosts: Vec<_> = fixtures.iter().map(Fixture::tracked_host).collect();
    let guest_paths: Vec<_> = fixtures.iter().zip(&ready).map(|(f, p)| f.winepath(p)).collect();
    let mut launchers = Vec::new();
    for i in 0..2 {
        let f = &fixtures[i];
        let mut cmd = f.command(&exes[i], &hosts[i]);
        cmd.env("VICTORY_TEST_GUEST_READY", &guest_paths[i]).arg("--").arg("--wait-for-host-exit")
            .process_group(0)
            .stdout(File::create(f.temp.path().join("concurrent-stdout.log")).unwrap())
            .stderr(File::create(f.temp.path().join("concurrent-stderr.log")).unwrap());
        launchers.push(Launcher(Some(cmd.spawn().expect("spawn launcher"))));
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        for launcher in &mut launchers {
            if let Some(status) = launcher.0.as_mut().unwrap().try_wait().unwrap() {
                launcher.0.take();
                panic!("launcher exited before both guests were ready: {status}");
            }
        }
        if ready.iter().all(|p| p.exists()) { break; }
        assert!(Instant::now() < deadline, "concurrent guests did not become ready");
        thread::sleep(Duration::from_millis(50));
    }
    let connections: Vec<_> = ready.iter().map(|p| fs::read_to_string(p).unwrap()).collect();
    let mut addresses = Vec::new();
    let mut tokens = Vec::new();
    for connection in &connections {
        let (addr, token) = connection.split_once('\n').expect("connection file format");
        let port: u16 = addr.strip_prefix("127.0.0.1:").expect("loopback address")
            .parse().expect("numeric port");
        assert_ne!(port, 0, "host must use an assigned port");
        assert!(token.len() == 32 && token.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "token must contain 32 lowercase hexadecimal digits");
        addresses.push(port);
        tokens.push(token);
    }
    assert!(addresses[0] != addresses[1], "concurrent hosts reused a port");
    assert!(tokens[0] != tokens[1], "concurrent launchers reused a token");

    for launcher in &launchers {
        let pid = launcher.0.as_ref().unwrap().id().to_string();
        assert!(Command::new("/bin/kill").args(["-TERM", &pid]).status().unwrap().success(),
            "could not signal launcher {pid}");
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    for launcher in &mut launchers {
        loop {
            if let Some(status) = launcher.0.as_mut().unwrap().try_wait().unwrap() {
                launcher.0.take();
                assert_eq!(status.code(), Some(143), "launcher did not propagate SIGTERM");
                break;
            }
            assert!(Instant::now() < deadline, "launcher did not exit after SIGTERM");
            thread::sleep(Duration::from_millis(50));
        }
    }
    for f in &fixtures { f.assert_host_gone(); }
}
