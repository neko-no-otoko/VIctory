//! Native PE fixture for per-lane raw-buffer bounds and LDS synchronization.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at + 8].copy_from_slice(&n.to_le_bytes()); }

// Sixty-four lanes load independent byte offsets from the UAV. The first 62
// address distinct in-bounds words (including byte 1020); the last two address
// byte 1024 and 0x1_0000_0000 after the instruction's immediate 4. Each lane
// writes its load to LDS, synchronizes, and reads the corresponding lane in the
// other half of the group. Stores exercise the same dynamic bounds. This
// crosses SIMD groups on 32-wide hardware, but not necessarily on 64-wide.
// DS encodings use the opcode in bits 25:18, with address/data/dst VGPRs in
// bytes 0/1/3 of the second word (gfx10/fields.rs).
const WORDS: &[u32] = &[
    0xbe850380,             // s_mov_b32 s5, 0
    0xf4080202, 0xfa000000, // s_load_dwordx4 s[8:11], s[4:5], 0
    0xbf8c0070,             // s_waitcnt
    0x34020082,             // v_lshlrev_b32 v1, 2, v0 (lane * 4)
    0xe0301000, 0x80020201, // buffer_load_dword v2, v1, s[8:11], 0 offen
    0xbf8c0070,             // s_waitcnt
    0xe0301004, 0x80020302, // buffer_load_dword v3, v2, s[8:11], 4 offen
    0xbf8c0070,             // s_waitcnt
    0xd8340000, 0x00000301, // ds_write_b32 v1, v3
    0xbf8c0070,             // s_waitcnt
    0xbf8a0000,             // s_barrier
    0x3a0802ff, 0x00000080, // v_xor_b32 v4, 128, v1 (other half's LDS address)
    0xd8d80000, 0x03000004, // ds_read_b32 v3, v4
    0xbf8c0070,             // s_waitcnt
    0xe0701100, 0x80020301, // buffer_store_dword v3, v1, s[8:11], 256 offen
    0xe0701004, 0x80020302, // buffer_store_dword v3, v2, s[8:11], 4 offen
    0xbf810000,             // s_endpgm
];

fn shader() -> Vec<u8> {
    let mut p = vec![0; 0x68];
    put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len();
    put64(&mut p, 0x60, record as u64);
    let stage = record + 0x54;
    let code = stage + 0x40;
    p.resize(code, 0);
    put64(&mut p, record, code as u64);
    put64(&mut p, record + 8, (WORDS.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20);
    put32(&mut p, record + 0x50, 8);
    put32(&mut p, stage + 0x24, (5 << 1) | (1 << 15)); // user SGPRs, 512 bytes LDS
    for (i, threads) in [64, 1, 1].into_iter().enumerate() { put32(&mut p, stage + 0x28 + 4 * i, threads); }
    for w in WORDS { p.extend_from_slice(&w.to_le_bytes()); }
    let layout = p.len();
    p.resize(layout + 20, 0);
    put32(&mut p, layout, 3); // descriptor table in s4, root slot 2
    put32(&mut p, layout + 8, 1);
    p[layout + 16] = 4;
    p[layout + 17] = 2;
    let used = p.len();
    p.resize(used + 20, 0);
    p[used] = 3; // UAV
    p[used + 1] = 3; // table
    p[used + 16] = 4;
    p[used + 17] = 4;
    p[used + 18] = 2;
    put64(&mut p, record + 0x28, layout as u64);
    put64(&mut p, record + 0x30, used as u64);
    put32(&mut p, record + 0x38, 1);
    put32(&mut p, record + 0x3c, 1);
    let mut f = b"FSSM".to_vec();
    f.extend_from_slice(&0x101u32.to_le_bytes());
    f.extend_from_slice(&(p.len() as u64).to_le_bytes());
    f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec();
    blob.extend_from_slice(&3u32.to_le_bytes());
    blob.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    blob.extend_from_slice(&(f.len() as u64).to_le_bytes());
    blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18];
    dxil.extend_from_slice(b"XDAT");
    for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); }
    dxil.extend_from_slice(&blob);
    let mut container = vec![0; 0x24];
    container[..4].copy_from_slice(b"DXBC");
    put32(&mut container, 0x14, 1);
    put32(&mut container, 0x18, (0x2c + dxil.len()) as u32);
    put32(&mut container, 0x1c, 1);
    put32(&mut container, 0x20, 0x24);
    container.extend_from_slice(b"DXIL");
    container.extend_from_slice(&(dxil.len() as u32).to_le_bytes());
    container.extend_from_slice(&dxil);
    container
}

fn tool(dir: &Path, name: &str, args: &[&str]) {
    let out = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn native_group_barrier_and_dynamic_uav_bounds() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let array = shader().iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    std::fs::write(root.join("shaders.h"), format!("static const unsigned char shader[] = {{{array}}};\n")).unwrap();
    std::fs::write(root.join("group_barrier.c"), include_str!("../../../tests/fixtures/macos_d3d12x_group_barrier.c")).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "group_barrier.c", "-o", "group_barrier.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:group_barrier.exe", "group_barrier.obj", "d3d.lib"]);
    let stdout = std::fs::File::create(root.join("stdout")).unwrap();
    let stderr = std::fs::File::create(root.join("stderr")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("native-fixture").arg(root.join("group_barrier.exe")).arg("--execute")
        .stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr)).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native group barrier timed out\nstdout: {}\nstderr: {}",
                std::fs::read_to_string(root.join("stdout")).unwrap(),
                std::fs::read_to_string(root.join("stderr")).unwrap());
        }
        thread::sleep(Duration::from_millis(50));
    };
    let stdout = std::fs::read_to_string(root.join("stdout")).unwrap();
    let stderr = std::fs::read_to_string(root.join("stderr")).unwrap();
    assert!(status.success() && stdout.contains("fixture entry returned 42"), "status {status}\nstdout: {stdout}\nstderr: {stderr}");
}
