//! End-to-end varying linkage through the real Wine Metal proxy and host.
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

// Four single-descriptor tables, all at offset zero: samplers slots 1/3,
// textures slots 2/4. User SGPRs 8..11 hold the four table bases.
fn shader(vertex: bool) -> Vec<u8> {
    let words: &[u32] = if vertex {
        &[0xf8000203, 0x00000908, 0xf80018cf, 0x07060504, 0xbf810000]
    } else {
        &[
            0xc8080000, 0xc8090001, 0xc80c0100, 0xc80d0101,
            // Preserve user data in even/odd scalar address pairs.
            0xbe820308, 0xbe840309, 0xbe86030a, 0xbe88030b,
            0xbe830380, 0xbe850380, 0xbe870380, 0xbe890380,
            0xf4080601, 0xfa000000, // s[24:27] = sampler slot 1
            0xf40c0402, 0xfa000000, // s[16:23] = texture slot 2
            0xf4080703, 0xfa000000, // s[28:31] = sampler slot 3
            0xf40c0804, 0xfa000000, // s[32:39] = texture slot 4
            0xf0800f08, 0x00c40402, // v[4:7] = sample(v[2:3], s[16:23], s[24:27])
            0xf0800f08, 0x00e80802, // v[8:11] = sample(v[2:3], s[32:39], s[28:31])
            0xf800180f, 0x09080504, // export first two channels of each
            0xbf810000,
        ]
    };
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, if vertex { 0x28 } else { 0x30 }, record as u64);
    let stage = record + 0x54; let code = stage + if vertex { 0x18c } else { 0x14c + 8 * 404 };
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record + 8, (words.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20); put32(&mut p, record + 0x50, 12);
    if vertex {
        p[stage + 0x24..stage + 0x2e].copy_from_slice(&[6, 0, 15, 4, 0, 6, 1, 3, 8, 0]);
        put32(&mut p, stage + 0xc4, 2);
        p[stage + 0xc8..stage + 0xce].copy_from_slice(&[6, 1, 0, 1, 0, 0]); put32(&mut p, stage + 0x188, 1);
    } else {
        p[stage + 0x24..stage + 0x2c].copy_from_slice(&[6, 0, 3, 0, 0, 0, 0, 0]);
        put32(&mut p, stage + 0x124, 1); put32(&mut p, stage + 0x134, 2);
        for i in 0..8 { put32(&mut p, stage + 0x14c + i * 404, u32::MAX); }
    }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    if !vertex {
        let layout = p.len(); p.resize(layout + 180, 0);
        put64(&mut p, record + 0x28, layout as u64); put64(&mut p, record + 0x30, (layout + 100) as u64);
        put32(&mut p, record + 0x38, 5); put32(&mut p, record + 0x3c, 4);
        for (i, (kind, user, slot)) in [(0, 0, 0), (3, 8, 1), (3, 9, 2), (3, 10, 3), (3, 11, 4)].into_iter().enumerate() {
            let at = layout + i * 20; put32(&mut p, at, kind); put32(&mut p, at + 8, 1); p[at + 16] = user; p[at + 17] = slot;
        }
        for (i, (kind, dwords, user, slot)) in [(1, 4, 8, 1), (0, 8, 9, 2), (1, 4, 10, 3), (0, 8, 11, 4)].into_iter().enumerate() {
            let at = layout + 100 + i * 20; p[at] = kind; p[at + 1] = 3; p[at + 16] = dwords; p[at + 17] = user; p[at + 18] = slot;
        }
    }
    let mut f = b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18]; dxil.extend_from_slice(b"XDAT"); for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); } dxil.extend_from_slice(&blob);
    let mut parts = vec![(b"DXIL", dxil)];
    if vertex {
        parts.push((b"ISG1", signature(&[(b"POSITION\0", 0, 0, 0, 15), (b"TEXCOORD\0", 0, 0, 1, 3)])));
        parts.push((b"OSG1", signature(&[(b"SV_Position\0", 0, 1, 0, 15), (b"TEXCOORD\0", 0, 0, 1, 3)])));
    } else {
        parts.push((b"ISG1", signature(&[(b"TEXCOORD\0", 0, 0, 0, 3)])));
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
        let arrays = [("vs", shader(true)), ("ps", shader(false))].into_iter().map(|(name, bytes)| {
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
        let exe = dir.join("multitexture.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/wine_launcher_multitexture.c");
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
fn launcher_runs_independent_texture_and_sampler_tables() {
    let f = Fixture::new();
    let exe = f.build();
    let host = f.tracked_host();
    let out = run(&mut f.command(&exe, &host), f.temp.path());
    f.assert_host_gone();
    assert_eq!(out.status.code(), Some(42), "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("D3D12X multitexture passed"), "{}\n{}", out.stdout, out.stderr);
    for (variant, diagnostic) in [
        ("--missing-texture", "shader texture or sampler slot mismatch"),
        ("--missing-sampler", "shader texture or sampler slot mismatch"),
        ("--changed-texture", "SRV descriptor changed"),
        ("--changed-sampler", "sampler descriptor changed"),
    ] {
        let mut cmd = f.command(&exe, &host); cmd.args(["--", variant]);
        let out = run(&mut cmd, f.temp.path()); f.assert_host_gone();
        assert!(out.stdout.contains("D3D12X invalid multitexture armed"), "{variant}: {}\n{}", out.stdout, out.stderr);
        assert!(!out.status.success(), "{variant}: {}\n{}", out.stdout, out.stderr);
        assert!(!out.stdout.contains("D3D12X multitexture passed"), "{variant}: {}\n{}", out.stdout, out.stderr);
        assert!(out.stderr.contains("Wine Metal proxy:") && out.stderr.contains(diagnostic),
            "{variant}: {}: {}\n{}", out.status, out.stdout, out.stderr);
    }
}
