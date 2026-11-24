//! End-to-end varying linkage through the real Wine Metal proxy and host.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// Synthetic GFX10 programs in the same FSSM/DXBC envelope as native_varying.
// VS: POSITION -> pos, POSITION.xy -> TEXCOORD2, TEXCOORD0.xy -> COLOR1.
// PS: perspective interpolate TEXCOORD2; COLOR1 is perspective or flat.
fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at + 8].copy_from_slice(&n.to_le_bytes()); }

fn signature(rows: &[(&[u8], u32, u32, u32, u8)]) -> Vec<u8> {
    let mut s = vec![0; 8 + 32 * rows.len()];
    put32(&mut s, 0, rows.len() as u32); put32(&mut s, 4, 8);
    for (i, &(name, index, system, row, mask)) in rows.iter().enumerate() {
        let at = 8 + 32 * i; let offset = s.len() as u32;
        put32(&mut s, at + 4, offset); put32(&mut s, at + 8, index);
        put32(&mut s, at + 12, system); put32(&mut s, at + 16, 3);
        put32(&mut s, at + 20, row); s[at + 24] = mask;
        s.extend_from_slice(name);
    }
    s
}

fn shader(vertex: bool, mismatch: bool, flat: bool, reordered: bool, wide: bool) -> Vec<u8> {
    let words: &[u32] = if vertex && wide {
        // Export all four position components as param0 as well as pos0.
        &[0xf800020f, 0x07060504, 0xf8000213, 0x00000908,
          0xf80018cf, 0x07060504, 0xbf810000]
    } else if vertex {
        // exp param0, v4,v5; exp param1, v8,v9; exp pos0, v4..v7
        &[0xf8000203, 0x00000504, 0xf8000213, 0x00000908,
          0xf80018cf, 0x07060504, 0xbf810000]
    } else if wide {
        // Interpolate all four attr0 channels; z/w go to red/green.
        &[0xc8180000, 0xc8190001, 0xc81c0100, 0xc81d0101,
          0xc8080200, 0xc8090201, 0xc80c0300, 0xc80d0301,
          0xc8100400, 0xc8110401, 0xc8140500, 0xc8150501,
          0xf800180f, 0x05040302, 0xbf810000]
    } else if flat {
        // attr1 uses p0 (first vertex) rather than interpolated values.
        &[0xc8080000, 0xc8090001, 0xc80c0100, 0xc80d0101,
          0xc8120402, 0xc8160502, 0xf800180f, 0x05040302, 0xbf810000]
    } else {
        // P1/P2 for attr0.xy -> v2,v3 and attr1.xy -> v4,v5.
        // Keep barycentrics in v0/v1 until all interpolations are complete.
        &[0xc8080000, 0xc8090001, 0xc80c0100, 0xc80d0101,
          0xc8100400, 0xc8110401, 0xc8140500, 0xc8150501,
          0xf800180f, 0x05040302, 0xbf810000]
    };
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, if vertex { 0x28 } else { 0x30 }, record as u64);
    let stage = record + 0x54; let code = stage + if vertex { 0x18c } else { 0x14c + 8 * 404 };
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record + 8, (words.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20); put32(&mut p, record + 0x50, 10);
    if vertex {
        p[stage + 0x24..stage + 0x2e].copy_from_slice(&[6, 0, 15, 4, 0, 6, 1, 3, 8, 0]);
        put32(&mut p, stage + 0xc4, 2);
        p[stage + 0xc8..stage + 0xd4].copy_from_slice(&[6, 1, 0, 1, 0, 0, 6, 2, 1, 2, 0, 0]);
        put32(&mut p, stage + 0x188, 2);
    } else {
        p[stage + 0x24..stage + 0x34].copy_from_slice(&[6, 0, 3, 0, 0, 0, 0, 0, 6, 1, 3, 1, 0, 0, 0, 0]);
        if wide { p[stage + 0x26] = 15; }
        if reordered {
            // Attribute numbers still select the same values, but PS signature
            // rows are the reverse of the original semantic order.
            p[stage + 0x25] = 1;
            p[stage + 0x2d] = 0;
        }
        if flat { p[stage + 0x33] = 1; }
        put32(&mut p, stage + 0x124, 2); put32(&mut p, stage + 0x134, 2);
        for i in 0..8 { put32(&mut p, stage + 0x14c + i * 404, u32::MAX); }
    }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    let mut f = b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18]; dxil.extend_from_slice(b"XDAT"); for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); } dxil.extend_from_slice(&blob);
    let mut parts = vec![(b"DXIL", dxil)];
    if vertex {
        parts.push((b"ISG1", signature(&[(b"POSITION\0", 0, 0, 0, 15), (b"TEXCOORD\0", 0, 0, 1, 3)])));
        parts.push((b"OSG1", signature(&[(b"SV_Position\0", 0, 1, 0, 15), (b"TEXCOORD\0", 2, 0, 1, if wide { 15 } else { 3 }), (b"COLOR\0", 1, 0, 2, 3)])));
    } else {
        let inputs = if reordered {
            signature(&[(b"COLOR\0", 1, 0, 0, 3), (b"TEXCOORD\0", 2, 0, 1, 3)])
        } else {
            signature(&[(b"TEXCOORD\0", 2, 0, 0, if wide { 15 } else { 3 }), (b"COLOR\0", if mismatch { 2 } else { 1 }, 0, 1, 3)])
        };
        parts.push((b"ISG1", inputs));
        parts.push((b"OSG1", signature(&[(b"SV_Target\0", 0, 64, 0, 15)])));
    }
    let mut container = vec![0; 32 + 4 * parts.len()]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container, 20, 1); put32(&mut container, 28, parts.len() as u32);
    for (i, (tag, data)) in parts.into_iter().enumerate() { let offset = container.len() as u32; put32(&mut container, 32 + i * 4, offset); container.extend_from_slice(tag); container.extend_from_slice(&(data.len() as u32).to_le_bytes()); container.extend_from_slice(&data); }
    let size = container.len() as u32; put32(&mut container, 24, size); container
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
        let arrays = [("vs", shader(true, false, false, false, false)), ("ps", shader(false, false, false, false, false)), ("ps_flat", shader(false, false, true, false, false)), ("ps_mismatch", shader(false, true, false, false, false)), ("ps_reordered", shader(false, false, false, true, false)), ("vs_wide", shader(true, false, false, false, true)), ("ps_wide", shader(false, false, false, false, true))].into_iter().map(|(name, bytes)| {
            format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
        }).collect::<String>();
        fs::write(dir.join("shaders.h"), arrays).unwrap();
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
        let exe = dir.join("varying.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_varying.c");
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
fn launcher_runs_multiple_varyings_and_rejects_mismatch() {
    let f = Fixture::new();
    let exe = f.build();
    let host = f.tracked_host();
    for (variant, marker) in [(None, "D3D12X multiple varyings passed"),
                              (Some("--flat"), "D3D12X flat varying passed"),
                              (Some("--reordered"), "D3D12X reordered varyings passed"),
                              (Some("--wide"), "D3D12X wide varying passed")] {
        let mut cmd = f.command(&exe, &host);
        if let Some(variant) = variant { cmd.arg("--").arg(variant); }
        let out = run(&mut cmd, f.temp.path());
        f.assert_host_gone();
        assert_eq!(out.status.code(), Some(42), "{variant:?}: {}\n{}", out.stdout, out.stderr);
        assert!(out.stdout.contains(marker), "{variant:?}: {}\n{}", out.stdout, out.stderr);
    }
    let mut cmd = f.command(&exe, &host);
    cmd.args(["--", "--mismatch"]);
    let out = run(&mut cmd, f.temp.path());
    f.assert_host_gone();
    assert!(!out.status.success(), "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("D3D12X varying mismatch armed"), "{}\n{}", out.stdout, out.stderr);
    assert!(out.stderr.contains("vertex and pixel varying signatures do not match"), "{}\n{}", out.stdout, out.stderr);
}
