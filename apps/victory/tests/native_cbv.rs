#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::path::Path;
use std::process::Command;

fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at + 8].copy_from_slice(&n.to_le_bytes()); }

// GFX10: build a buffer descriptor from root address s[0:1], read the CBV
// dword at byte 16 into s6, load UAV word 1 through the table in s4, add, and store
// the result in UAV word 0. Both resource descriptions are real FSSM metadata.
const WORDS: &[u32] = &[
        0xbe8c0300,             // s_mov_b32 s12, s0
        0x880dff01, 0x00100000, // s_or_b32 s13, s1, 0x100000
        0xbe8e03c1,             // s_mov_b32 s14, -1
        0xbe8f03ff, 0x11014fac, // s_mov_b32 s15, 0x11014fac
        0xf4200186, 0xfa000010, // s_buffer_load_dword s6, s[12:15], 16
        0xbe850380,             // s_mov_b32 s5, 0
        0xf4080202, 0xfa000000, // s_load_dwordx4 s[8:11], s[4:5], 0
        0xe0300004, 0x80020000, // buffer_load_dword v0, off, s[8:11], 4
        0xbf8c0070,             // s_waitcnt
        0x4a000006,             // v_add_nc_u32 v0, s6, v0
        0xe0700000, 0x80020000, // buffer_store_dword v0, off, s[8:11], 0
        0xbf810000,
];

fn shader() -> Vec<u8> { shader_with_words(WORDS) }

fn shader_with_words(words: &[u32]) -> Vec<u8> {
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, 0x60, record as u64);
    let stage = record + 0x54; let code = stage + 0x40;
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record + 8, (words.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20); put32(&mut p, record + 0x50, 8);
    put32(&mut p, stage + 0x24, 5 << 1);
    for i in 0..3 { put32(&mut p, stage + 0x28 + 4 * i, 1); }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    let layout = p.len(); p.resize(layout + 40, 0);
    for (i, (kind, sgprs, user, slot)) in [(1, 2, 0, 1), (3, 1, 4, 2)].into_iter().enumerate() {
        let at = layout + i * 20; put32(&mut p, at, kind); put32(&mut p, at + 8, sgprs); p[at + 16] = user; p[at + 17] = slot;
    }
    let used = p.len(); p.resize(used + 40, 0);
    for (i, (kind, root, user, slot)) in [(2, 1, 0, 1), (3, 3, 4, 2)].into_iter().enumerate() {
        let at = used + i * 20; p[at] = kind; p[at + 1] = root; p[at + 16] = 4; p[at + 17] = user; p[at + 18] = slot;
    }
    put64(&mut p, record + 0x28, layout as u64); put64(&mut p, record + 0x30, used as u64);
    put32(&mut p, record + 0x38, 2); put32(&mut p, record + 0x3c, 2);
    let mut f = b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18]; dxil.extend_from_slice(b"XDAT");
    for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); }
    dxil.extend_from_slice(&blob);
    let mut container = vec![0; 0x24]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container, 0x14, 1); put32(&mut container, 0x18, (0x2c + dxil.len()) as u32); put32(&mut container, 0x1c, 1); put32(&mut container, 0x20, 0x24);
    container.extend_from_slice(b"DXIL"); container.extend_from_slice(&(dxil.len() as u32).to_le_bytes()); container.extend_from_slice(&dxil); container
}
fn tool(dir: &Path, name: &str, args: &[&str]) {
    let out = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_root_cbv_through_native_cli() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap(); let root = dir.path();
    let array = shader().iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    std::fs::write(root.join("shaders.h"), format!("static const unsigned char shader[] = {{{array}}};\n")).unwrap();
    std::fs::write(root.join("cbv.c"), include_str!("../../../tests/fixtures/macos_d3d12x_cbv.c")).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    for case in 0..=8 {
        tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", &format!("-DCASE={case}"), "-c", "cbv.c", "-o", "cbv.obj"]);
        tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:cbv.exe", "cbv.obj", "d3d.lib"]);
        let out = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("cbv.exe")).arg("--execute").output().unwrap();
        if case == 5 || case == 6 {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(out.status.signal(), Some(6), "case {case}: {}", String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported native D3D12.X vtable method"));
        } else {
            assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("fixture entry returned 42"), "case {case}: status {}\nstdout: {}\nstderr: {}", out.status, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
    }
}

#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn native_compute_guards_single_uav_accesses_and_rejects_dynamic_cbv() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);

    // The native path accepts only a whole-buffer UAV; use an 8-byte
    // resource so byte 8 is the first out-of-range address.
    let original = include_str!("../../../tests/fixtures/macos_d3d12x_cbv.c");
    let flags = "rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;";
    assert!(original.contains(flags));
    let source = original.replace(flags, "rd.Width=8; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;");
    let pipeline_check = "if (SLOT(dev,0x58,CreatePipeline)(dev,&pd,&IID_ID3D12PipelineState,&pipeline)!=S_OK || !pipeline) return 7;";
    let result_check = "if (result[0]!=42 || result[1]!=1) return 13;";
    let close_check = "#if (CASE >= 1 && CASE <= 4) || CASE == 7 || CASE == 8";
    assert!(source.contains(pipeline_check));
    assert!(source.contains(result_check));
    assert!(source.contains(close_check));

    for (case, expected) in [
        ("control", (42, 1)),
        ("uav_load", (41, 1)),
        ("uav_store", (7, 1)),
        ("uav_partial_load", (41, 1)),
        ("uav_partial_store", (7, 42)),
        ("scalar_load_in", (42, 1)),
        ("scalar_load_out", (41, 1)),
        ("scalar_store_in", (7, 42)),
        ("scalar_store_out", (7, 1)),
        ("vector_load_in", (42, 1)),
        ("vector_load_out", (41, 1)),
        ("vector_store_in", (7, 42)),
        ("vector_store_out", (7, 1)),
        ("scalar_load_wrap", (41, 1)),
        ("scalar_store_wrap", (7, 1)),
        ("vector_load_wrap", (41, 1)),
        ("vector_store_wrap", (7, 1)),
        ("cbv_dynamic", (0, 0)),
    ] {
        let mut words = WORDS.to_vec();
        match case {
            "control" => {}
            "uav_load" => words[11] = 0xe0300008,
            "uav_store" => words[15] = 0xe0700008,
            // The second dword is outside the eight-byte UAV. Observe the
            // loaded second component through the ordinary in-bounds store.
            "uav_partial_load" => { words[11] = 0xe0340004; words[14] = 0x4a000206; }
            "uav_partial_store" => { words[15] = 0xe0740004; }
            // s2 and v1 supply runtime scalar and per-thread byte offsets.
            "scalar_load_in" => { words[12] = 0x02020000; words.splice(0..0, [0xbe820380]); }
            "scalar_load_out" => { words[12] = 0x02020000; words.splice(0..0, [0xbe820384]); }
            "scalar_store_in" => { words[16] = 0x02020000; words.splice(0..0, [0xbe820384]); }
            "scalar_store_out" => { words[16] = 0x02020000; words.splice(0..0, [0xbe820388]); }
            "vector_load_in" => { words[11] = 0xe0301004; words[12] = 0x80020001; words.splice(0..0, [0x7e020280]); }
            "vector_load_out" => { words[11] = 0xe0301004; words[12] = 0x80020001; words.splice(0..0, [0x7e020284]); }
            "vector_store_in" => { words[15] = 0xe0701000; words[16] = 0x80020001; words.splice(0..0, [0x7e020284]); }
            "vector_store_out" => { words[15] = 0xe0701000; words[16] = 0x80020001; words.splice(0..0, [0x7e020288]); }
            // 0xfffffffc + 4 must not wrap back to the first dword.
            "scalar_load_wrap" => { words[12] = 0x02020000; words.splice(0..0, [0xbe8203ff, 0xfffffffc]); }
            "scalar_store_wrap" => { words[15] = 0xe0700004; words[16] = 0x02020000; words.splice(0..0, [0xbe8203ff, 0xfffffffc]); }
            "vector_load_wrap" => { words[11] = 0xe0301004; words[12] = 0x80020001; words.splice(0..0, [0x7e0202ff, 0xfffffffc]); }
            "vector_store_wrap" => { words[15] = 0xe0701004; words[16] = 0x80020001; words.splice(0..0, [0x7e0202ff, 0xfffffffc]); }
            // Root CBV starts at byte 16; dynamic CBV access remains unsupported.
            "cbv_dynamic" => { words[7] = 0x04000000; words.splice(0..0, [0xbe8203b0]); }
            _ => unreachable!(),
        }
        let array = shader_with_words(&words).iter().map(u8::to_string).collect::<Vec<_>>().join(",");
        std::fs::write(root.join("shaders.h"), format!("static const unsigned char shader[] = {{{array}}};\n")).unwrap();
        let fixture = if case == "cbv_dynamic" {
            source.replace(pipeline_check, "HRESULT hr=SLOT(dev,0x58,CreatePipeline)(dev,&pd,&IID_ID3D12PipelineState,&pipeline); if (hr==E_NOTIMPL && !pipeline) return 42; if (hr!=S_OK || !pipeline) return 7;")
                .replace(close_check, "#if 1")
        } else {
            source.replace(result_check, &format!("if (result[0]!={} || result[1]!={}) return 13;", expected.0, expected.1))
        };
        std::fs::write(root.join("cbv.c"), fixture).unwrap();
        tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-DCASE=0", "-c", "cbv.c", "-o", "cbv.obj"]);
        tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:cbv.exe", "cbv.obj", "d3d.lib"]);
        let out = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("cbv.exe")).arg("--execute").output().unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("fixture entry returned 42"), "case {case}: status {}\nstdout: {}\nstderr: {}", out.status, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
