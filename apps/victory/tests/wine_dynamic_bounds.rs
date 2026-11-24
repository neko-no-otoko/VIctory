//! Wine-launched TLS compute checks against real Metal shader view bounds.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at + 8].copy_from_slice(&n.to_le_bytes()); }

// Four descriptors, in table order: UAV, SRV, UAV, SRV. The two SRVs
// carry independent runtime byte offsets at their first elements.
fn shader(_mode: usize, case: usize) -> Vec<u8> {
    let mut words: Vec<u32> = vec![
        0xbe850380, 0xf4080202, 0xfa000000, 0xf4080302, 0xfa000020,
        0xf4080402, 0xfa000040, 0xf4080502, 0xfa000060, 0xbf8c0070,
        0xe0300000, 0x80030200, // v2 = first SRV's offset
        0xe0300000, 0x80050300, // v3 = second SRV's offset
        0xbf8c0070,
    ];
    if matches!(case, 3 | 4 | 6 | 12) {
        let scalar = match case { 3 => 8, 4 => 16, _ => 0xffff_fffc };
        words.extend_from_slice(&[0xbe8603ff, scalar]); // s_mov_b32 s6, literal
    }
    if case == 7 || case == 10 {
        // Case 7 straddles the view; case 10 wraps on its second dword.
        let (op, operand) = if case == 7 { (0xe034000c, 0x80030000) }
                            else { (0xe0341000, 0x80030002) };
        words.extend_from_slice(&[op, operand, 0xbf8c0070, 0xbf8a0000,
            0xe0740008, 0x80020000, 0xe0740004, 0x80040000]);
    } else {
        let (read1, arg1, read2, arg2) = match case {
            2 => (0xe0300010, 0x80030000, 0xe030000c, 0x80050100),
            3 | 4 => (0xe0300000, 0x06030000, 0xe0300000, 0x06050100),
            6 => (0xe0300008, 0x06030000, 0xe0300008, 0x06050100),
            8 | 11 | 12 => (0xe0300004, 0x80030000, 0xe0300004, 0x80050100),
            9 => (0xe0302000, 0x80030002, 0xe0302000, 0x80050103),
            _ => (0xe0301000 | if case == 5 { 8 } else { 0 }, 0x80030002,
                  0xe0301000 | if case == 5 { 8 } else { 0 }, 0x80050103),
        };
        words.extend_from_slice(&[read1, arg1, read2, arg2, 0xbf8c0070, 0xbf8a0000]);
        if case == 8 {
            words.extend_from_slice(&[0xe0701000, 0x80020002, 0xe0701000, 0x80040103]);
        } else if case == 11 {
            words.extend_from_slice(&[0xe0701008, 0x80020002, 0xe0701008, 0x80040103]);
        } else if case == 12 {
            words.extend_from_slice(&[0xe0700008, 0x06020000, 0xe0700008, 0x06040100]);
        } else {
            words.extend_from_slice(&[0xe0700000, 0x80020000, 0xe0700000, 0x80040100]);
        }
    }
    words.push(0xbf810000);
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, 0x60, record as u64);
    let stage = record + 0x54; let code = stage + 0x40;
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record + 8, (words.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20); put32(&mut p, record + 0x50, 8);
    put32(&mut p, stage + 0x24, 5 << 1);
    for i in 0..3 { put32(&mut p, stage + 0x28 + 4 * i, 1); }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    let layout = p.len(); p.resize(layout + 20, 0);
    put32(&mut p, layout, 3); put32(&mut p, layout + 8, 1); p[layout + 16] = 4; p[layout + 17] = 7;
    let used = p.len(); p.resize(used + 4 * 20, 0);
    for (i, kind) in [3u8, 0, 3, 0].iter().copied().enumerate() {
        let at = used + i * 20; p[at] = kind; p[at + 1] = 3;
        put32(&mut p, at + 4, (i * 8) as u32); p[at + 16] = 4; p[at + 17] = 4; p[at + 18] = 7;
    }
    put64(&mut p, record + 0x28, layout as u64); put64(&mut p, record + 0x30, used as u64);
    put32(&mut p, record + 0x38, 1); put32(&mut p, record + 0x3c, 4);
    let mut f = b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18]; dxil.extend_from_slice(b"XDAT");
    for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); }
    dxil.extend_from_slice(&blob);
    let mut container = vec![0; 0x24]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container, 0x14, 1); put32(&mut container, 0x18, (0x2c + dxil.len()) as u32); put32(&mut container, 0x1c, 1); put32(&mut container, 0x20, 0x24);
    container.extend_from_slice(b"DXIL"); container.extend_from_slice(&(dxil.len() as u32).to_le_bytes()); container.extend_from_slice(&dxil); container
}

struct Output { status: ExitStatus, stdout: String, stderr: String }

fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

// Files avoid blocking on undrained Wine or host output pipes.
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

    fn checked(&self, command: &mut Command) {
        let out = run(command, self.temp.path());
        assert!(out.status.success(), "{command:?}: {}\n{}\n{}", out.status, out.stdout, out.stderr);
    }

    fn build(&self) -> PathBuf {
        let dir = self.temp.path();
        let mut header = String::new();
        for mode in 0..4 { for case in 0..13 {
            let array = shader(mode, case).iter().map(u8::to_string).collect::<Vec<_>>().join(",");
            header.push_str(&format!("static const unsigned char s{mode}_{case}[] = {{{array}}};\n"));
        }}
        header.push_str("static const struct { const unsigned char *bytes; unsigned long long size; } shaders[4][13] = {\n");
        for mode in 0..4 { header.push_str("{"); for case in 0..13 {
            header.push_str(&format!("{{s{mode}_{case}, sizeof(s{mode}_{case})}},"));
        } header.push_str("},\n"); }
        header.push_str("};\n");
        fs::write(dir.join("shaders.h"), header).unwrap();
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
        let exe = dir.join("dynamic-bounds.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_dynamic_bounds.c");
        let mut cmd = Command::new(self.llvm.join("bin/x86_64-w64-mingw32-clang"));
        cmd.args(["-std=c11", "-fno-emulated-tls", "-O1", "-I"]).arg(dir).arg(source);
        for (name, _) in exports { cmd.arg(dir.join(format!("{name}.lib"))); }
        cmd.arg("-o").arg(&exe);
        self.checked(&mut cmd);
        exe
    }

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
            .arg("--wine-prefix").arg(&self.prefix).arg("--allow-unverified-dll-search")
            .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .current_dir(self.temp.path()).env("WINEDEBUG", "-all")
            .env("VICTORY_WINE_METAL_ADDR", "invalid-inherited-address")
            .env("VICTORY_WINE_METAL_TOKEN", "invalid-inherited-token")
            .env("VICTORY_WINE_METAL_PRESENT", "0");
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
fn launcher_executes_dynamic_raw_bounds() {
    let f = Fixture::new();
    let exe = f.build();
    let host = f.tracked_host();
    for case in 0..13 {
        let mut cmd = f.command(&exe, &host);
        cmd.arg("--").arg("3").arg(case.to_string());
        let out = run(&mut cmd, f.temp.path());
        let context = format!("case {case}: {}\n{}", out.stdout, out.stderr);
        assert!(f.host_pid.is_file(), "host was not started: {context}");
        f.assert_host_gone();
        assert!(out.stdout.contains("D3D12X bounds dispatch armed"), "{context}");
        if case == 9 {
            assert!(!out.status.success() && out.status.code() != Some(12), "{context}");
            assert!(!out.stdout.contains("D3D12X bounds dynamic passed"), "{context}");
            assert!(out.stderr.contains("proxy status"), "{context}");
        } else {
            assert_eq!(out.status.code(), Some(42), "{context}");
            assert!(out.stdout.contains("D3D12X bounds dynamic passed"), "{context}");
        }
    }
}
