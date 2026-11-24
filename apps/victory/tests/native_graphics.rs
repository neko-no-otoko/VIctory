#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::process::Command;

fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at + 4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at + 8].copy_from_slice(&n.to_le_bytes()); }
fn signature(name: &[u8], system: u32) -> Vec<u8> {
    let mut s = vec![0; 40];
    put32(&mut s, 0, 1); put32(&mut s, 4, 8); put32(&mut s, 12, 40);
    put32(&mut s, 20, system); put32(&mut s, 24, 3); s[32] = 15;
    s.extend_from_slice(name); s
}
// Matches the self-authored GFX10 programs in the Metal graphics tests.
fn shader(vertex: bool, textured: bool) -> Vec<u8> {
    let words: &[u32] = if vertex { &[0xf80018cf, 0x07060504, 0xbf810000] } else if textured {
        &[0x7e040200, 0x7e0602f0, 0xbe820309, 0xbe890380, 0xbe830380, 0xf4080204, 0xfa000000, 0xf40c0001, 0xfa000000, 0xf0800f08, 0x00400002, 0xf800180f, 0x03020100, 0xbf810000]
    } else {
        &[0x7e000202, 0x7e020280, 0x7e040280, 0x7e0602ff, 0x3f800000, 0xf800180f, 0x03020100, 0xbf810000]
    };
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, if vertex { 0x28 } else { 0x30 }, record as u64);
    let stage = record + 0x54; let code = stage + if vertex { 0x18c } else { 0x14c + 8 * 404 };
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record + 8, (words.len() * 4) as u64);
    put32(&mut p, record + 0x14, 0x20); put32(&mut p, record + 0x50, 8);
    if vertex { p[stage + 0x24..stage + 0x29].copy_from_slice(&[6, 0, 15, 4, 0]); put32(&mut p, stage + 0xc4, 1); }
    else { for i in 0..8 { put32(&mut p, stage + 0x14c + i * 404, u32::MAX); }
        if !textured { put64(&mut p, record + 0x28, (code + words.len() * 4) as u64); put32(&mut p, record + 0x38, 1); } }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    if !vertex && !textured { let root = p.len(); p.resize(root + 20, 0); put32(&mut p, root + 8, 3); p[root + 17] = 3; }
    if !vertex && textured {
        let layout = p.len(); p.resize(layout + 100, 0);
        put64(&mut p, record + 0x28, layout as u64); put64(&mut p, record + 0x30, (layout + 60) as u64);
        put32(&mut p, record + 0x38, 3); put32(&mut p, record + 0x3c, 2);
        for (i, (kind, user, slot)) in [(0, 0, 0), (3, 8, 1), (3, 9, 2)].into_iter().enumerate() {
            let at = layout + i * 20; put32(&mut p, at, kind); put32(&mut p, at + 8, 1); p[at + 16] = user; p[at + 17] = slot;
        }
        for (i, (kind, dwords, user, slot)) in [(1, 4, 8, 1), (0, 8, 9, 2)].into_iter().enumerate() {
            let at = layout + 60 + i * 20; p[at] = kind; p[at + 1] = 3; p[at + 16] = dwords; p[at + 17] = user; p[at + 18] = slot;
        }
    }
    let mut f = b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&f, 6);
    let mut blob = b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil = vec![0; 0x18]; dxil.extend_from_slice(b"XDAT"); for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); } dxil.extend_from_slice(&blob);
    let mut parts = vec![(b"DXIL", dxil)]; if vertex { parts.push((b"ISG1", signature(b"POSITION\0", 0))); }
    parts.push((b"OSG1", if vertex { signature(b"SV_Position\0", 1) } else { signature(b"SV_Target\0", 64) }));
    let mut container = vec![0; 32 + 4 * parts.len()]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container, 20, 1); put32(&mut container, 28, parts.len() as u32);
    for (i, (tag, data)) in parts.into_iter().enumerate() { let offset = container.len() as u32; put32(&mut container, 32 + i * 4, offset); container.extend_from_slice(tag); container.extend_from_slice(&(data.len() as u32).to_le_bytes()); container.extend_from_slice(&data); }
    let size = container.len() as u32; put32(&mut container, 24, size); container
}
fn tool(dir: &std::path::Path, name: &str, args: &[&str]) {
    let out = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
}
#[test]
#[ignore = "requires a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_console_graphics_through_native_cli() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap(); let root = dir.path();
    let arrays = [("vs", shader(true, false)), ("ps", shader(false, false))].into_iter().map(|(name, bytes)| {
        format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
    }).collect::<String>();
    std::fs::write(root.join("shaders.h"), arrays).unwrap();
    std::fs::write(root.join("graphics.c"), include_str!("../../../tests/fixtures/macos_d3d12x_graphics.c")).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "graphics.c", "-o", "graphics.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:graphics.exe", "graphics.obj", "d3d.lib"]);
    let output = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("graphics.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let trap = include_str!("../../../tests/fixtures/macos_d3d12x_graphics.c").replace(
        "    D3D12_VERTEX_BUFFER_VIEW view=",
        "    ((void (*)(void))(*(void ***)list)[18])();\n    D3D12_VERTEX_BUFFER_VIEW view=",
    );
    std::fs::write(root.join("trap.c"), trap).unwrap();
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "trap.c", "-o", "trap.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:trap.exe", "trap.obj", "d3d.lib"]);
    let trap = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("trap.exe")).arg("--execute").output().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(trap.status.signal(), Some(6), "{}", String::from_utf8_lossy(&trap.stderr));
    assert!(String::from_utf8_lossy(&trap.stderr).contains("unsupported native D3D12.X vtable method"));
}

#[test]
#[ignore = "requires a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_textured_graphics_through_native_cli() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap(); let root = dir.path();
    let arrays = [("vs", shader(true, true)), ("ps", shader(false, true))].into_iter().map(|(name, bytes)| {
        format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
    }).collect::<String>();
    std::fs::write(root.join("shaders.h"), arrays).unwrap();
    let source = include_str!("../../../tests/fixtures/macos_d3d12x_texture.c");
    std::fs::write(root.join("texture.c"), source).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "texture.c", "-o", "texture.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:texture.exe", "texture.obj", "d3d.lib"]);
    let output = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("texture.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let trap = source.replace("    D3D12_VERTEX_BUFFER_VIEW view=", "    ((void (*)(void))(*(void ***)dev)[17])();\n    D3D12_VERTEX_BUFFER_VIEW view=");
    std::fs::write(root.join("trap.c"), trap).unwrap();
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "trap.c", "-o", "trap.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:trap.exe", "trap.obj", "d3d.lib"]);
    let trap = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("trap.exe")).arg("--execute").output().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(trap.status.signal(), Some(6), "{}", String::from_utf8_lossy(&trap.stderr));
    assert!(String::from_utf8_lossy(&trap.stderr).contains("unsupported native D3D12.X vtable method"));
}