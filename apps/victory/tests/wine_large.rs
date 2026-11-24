//! Large streamed resource snapshots through the actual Wine launcher and Metal host.
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
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}
fn run(command: &mut Command, dir: &Path) -> Output {
    let stdout = dir.join("stdout.log");
    let stderr = dir.join("stderr.log");
    let mut child = command.process_group(0).stdout(File::create(&stdout).unwrap()).stderr(File::create(&stderr).unwrap()).spawn().expect("spawn command");
    let deadline = Instant::now() + Duration::from_secs(300);
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
    fs::canonicalize(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}"))).unwrap_or_else(|e| panic!("invalid {name}: {e}"))
}
fn required_executable(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_file(), "invalid {name}: {}", path.display());
    if path.is_absolute() { path } else { std::env::current_dir().unwrap().join(path) }
}
fn quote(path: &Path) -> String { format!("'{}'", path.to_str().expect("UTF-8 test path").replace('\'', "'\\''")) }
struct Fixture {
    temp: tempfile::TempDir, prefix: PathBuf, wine: PathBuf, winepath: PathBuf,
    wineserver: PathBuf, host: PathBuf, converter: PathBuf, dll_dir: PathBuf, llvm: PathBuf, host_pid: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix"); fs::create_dir(&prefix).unwrap();
        let host_pid = temp.path().join("host.pid");
        Self { prefix, host_pid, wine: required_executable("VICTORY_TEST_WINE"), winepath: required_executable("VICTORY_TEST_WINEPATH"),
            wineserver: required_executable("VICTORY_TEST_WINESERVER"), host: required_path("VICTORY_TEST_METAL_HOST"),
            converter: required_path("VICTORY_TEST_METAL_CONVERTER"), dll_dir: required_path("VICTORY_TEST_DLL_DIR"), llvm: required_path("LLVM_MINGW"), temp }
    }
    fn checked(&self, cmd: &mut Command) {
        let out=run(cmd,self.temp.path());
        assert!(out.status.success(), "{cmd:?}: {}\n{}\n{}",out.status,out.stdout,out.stderr);
    }
    fn build(&self) -> PathBuf {
        let dir=self.temp.path();
        let def=dir.join("d3d12_xs.def");
        fs::write(&def,"LIBRARY d3d12_xs.dll\nEXPORTS\n D3D12XboxCreateDevice\n").unwrap();
        let lib=dir.join("d3d12_xs.lib");
        let mut dlltool=Command::new(self.llvm.join("bin/llvm-dlltool"));
        dlltool.args(["-m","i386:x86-64","-d"]).arg(def).arg("-l").arg(&lib); self.checked(&mut dlltool);
        let exe=dir.join("large.exe");
        let source=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_large.c");
        let mut clang=Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        clang.args(["-std=c11","-fno-emulated-tls","-O1"]).arg(source).arg(lib).arg("-o").arg(&exe); self.checked(&mut clang);
        exe
    }
    fn tracked_host(&self) -> PathBuf {
        let wrapper=self.temp.path().join("host-wrapper");
        fs::write(&wrapper,format!("#!/bin/sh\necho $$ > {}\nexec {} \"$@\"\n",quote(&self.host_pid),quote(&self.host))).unwrap();
        fs::set_permissions(&wrapper,fs::Permissions::from_mode(0o755)).unwrap(); wrapper
    }
    fn command(&self, exe: &Path, host: &Path) -> Command {
        let mut cmd=Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-run").arg(exe).arg("--wine").arg(&self.wine).arg("--winepath").arg(&self.winepath)
            .arg("--metal-host").arg(host).arg("--converter").arg(&self.converter).arg("--dll-dir").arg(&self.dll_dir)
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search")
            .env("WINEDLLOVERRIDES","mscoree,mshtml=").current_dir(self.temp.path()).env("WINEDEBUG","-all")
            .env("VICTORY_WINE_METAL_ADDR","invalid-inherited-address").env("VICTORY_WINE_METAL_TOKEN","invalid-inherited-token")
            .env("VICTORY_WINE_METAL_PRESENT","0"); cmd
    }
    fn assert_host_gone(&self) {
        let pid=fs::read_to_string(&self.host_pid).expect("host was started");
        let pid: u32=pid.trim().parse().unwrap(); let deadline=Instant::now()+Duration::from_secs(5);
        loop {
            let alive=Command::new("/bin/kill").args(["-0",&pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success();
            if !alive { break; }
            assert!(Instant::now()<deadline,"host {pid} still alive after launcher exit"); thread::sleep(Duration::from_millis(50));
        }
        fs::remove_file(&self.host_pid).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(pid)=fs::read_to_string(&self.host_pid) {
            if let Ok(pid)=pid.trim().parse::<u32>() { let _=Command::new("/bin/kill").args(["-KILL",&pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
        }
        for option in ["-k","-w"] {
            if let Ok(mut child)=Command::new(&self.wineserver).arg(option).env("WINEPREFIX",&self.prefix).process_group(0).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                let deadline=Instant::now()+Duration::from_secs(10);
                loop { match child.try_wait() { Ok(Some(_))=>break, Ok(None) if Instant::now()<deadline=>thread::sleep(Duration::from_millis(50)), _=>{ kill_group(&mut child); break; } } }
            }
        }
    }
}
#[test]
#[ignore = "requires Wine, LLVM-MinGW, provider DLLs and native Metal host/converter"]
fn launcher_streams_large_resources_and_rejects_invalid_bounds() {
    let f=Fixture::new(); let exe=f.build(); let host=f.tracked_host();
    let out=run(&mut f.command(&exe,&host),f.temp.path());
    assert_eq!(out.status.code(),Some(42),"{}\n{}",out.stdout,out.stderr);
    assert!(out.stdout.contains("D3D12X large streamed sampling passed"),"{}\n{}",out.stdout,out.stderr);
    f.assert_host_gone();
    for variant in ["--invalid-pitch","--invalid-range","--resource-limit","--batch-limit"] {
        let mut cmd=f.command(&exe,&host); cmd.arg("--").arg(variant);
        let out=run(&mut cmd,f.temp.path());
        assert!(out.stdout.contains("large invalid case armed"),"{variant}: {}\n{}",out.stdout,out.stderr);
        assert!(out.status.code()==Some(43) && out.stdout.contains("large invalid case rejected")
            || !out.status.success() && out.stderr.contains("Wine Metal proxy:"),
            "{variant}: {}\n{}\n{}",out.status,out.stdout,out.stderr);
        f.assert_host_gone();
    }
}
