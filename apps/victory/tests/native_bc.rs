#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::path::Path;
use std::process::Command;

fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at+4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at+8].copy_from_slice(&n.to_le_bytes()); }
fn signature(rows: &[(&[u8], u32, u32, u8)]) -> Vec<u8> {
    let mut s = vec![0; 8 + 32 * rows.len()];
    put32(&mut s, 0, rows.len() as u32); put32(&mut s, 4, 8);
    for (i, &(name, system, row, mask)) in rows.iter().enumerate() {
        let at = 8 + 32*i; let offset = s.len() as u32;
        put32(&mut s, at+4, offset); put32(&mut s, at+12, system);
        put32(&mut s, at+16, 3); put32(&mut s, at+20, row); s[at+24] = mask;
        s.extend_from_slice(name);
    }
    s
}
// The same console GFX10 position/UV and image_sample programs as native_uv.
fn shader(vertex: bool) -> Vec<u8> {
    let words: &[u32] = if vertex { &[0xf8000203, 0x00000908, 0xf80018cf, 0x07060504, 0xbf810000] }
        else { &[0xc8080000, 0xc8090001, 0xc80c0100, 0xc80d0101, 0xbe820309, 0xbe890380, 0xbe830380, 0xf4080204, 0xfa000000, 0xf40c0001, 0xfa000000, 0xf0800f08, 0x00400002, 0xf800180f, 0x03020100, 0xbf810000] };
    let mut p = vec![0; 0x68]; put64(&mut p, 0, 6);
    for at in [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] { put64(&mut p, at, 0xdeadbeefdeadbeef); }
    let record = p.len(); put64(&mut p, if vertex { 0x28 } else { 0x30 }, record as u64);
    let stage = record + 0x54; let code = stage + if vertex { 0x18c } else { 0x14c + 8*404 };
    p.resize(code, 0); put64(&mut p, record, code as u64); put64(&mut p, record+8, (words.len()*4) as u64);
    put32(&mut p, record+0x14, 0x20); put32(&mut p, record+0x50, 8);
    if vertex {
        p[stage+0x24..stage+0x29].copy_from_slice(&[6,0,15,4,0]); p[stage+0x29..stage+0x2e].copy_from_slice(&[6,1,3,8,0]);
        put32(&mut p, stage+0xc4, 2); p[stage+0xc8..stage+0xce].copy_from_slice(&[6,1,0,1,0,0]); put32(&mut p, stage+0x188, 1);
    } else {
        p[stage+0x24..stage+0x2c].copy_from_slice(&[6,0,3,0,0,0,0,0]); put32(&mut p, stage+0x124, 1); put32(&mut p, stage+0x134, 2);
        for i in 0..8 { put32(&mut p, stage+0x14c+i*404, u32::MAX); }
    }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    if !vertex {
        let layout = p.len(); p.resize(layout+100, 0); put64(&mut p, record+0x28, layout as u64); put64(&mut p, record+0x30, (layout+60) as u64);
        put32(&mut p, record+0x38, 3); put32(&mut p, record+0x3c, 2);
        for (i,(kind,user,slot)) in [(0,0,0),(3,8,1),(3,9,2)].into_iter().enumerate() { let at=layout+i*20; put32(&mut p,at,kind); put32(&mut p,at+8,1); p[at+16]=user; p[at+17]=slot; }
        for (i,(kind,dwords,user,slot)) in [(1,4,8,1),(0,8,9,2)].into_iter().enumerate() { let at=layout+60+i*20; p[at]=kind; p[at+1]=3; p[at+16]=dwords; p[at+17]=user; p[at+18]=slot; }
    }
    let mut f=b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed=miniz_oxide::deflate::compress_to_vec_zlib(&f,6);
    let mut blob=b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil=vec![0;0x18]; dxil.extend_from_slice(b"XDAT"); for n in [1u32,0x24,0x0d,0x30,blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); } dxil.extend_from_slice(&blob);
    let parts=if vertex { vec![(b"DXIL",dxil),(b"ISG1",signature(&[(b"POSITION\0",0,0,15),(b"TEXCOORD\0",0,1,3)])),(b"OSG1",signature(&[(b"SV_Position\0",1,0,15),(b"TEXCOORD\0",0,1,3)]))] } else { vec![(b"DXIL",dxil),(b"ISG1",signature(&[(b"TEXCOORD\0",0,0,3)])),(b"OSG1",signature(&[(b"SV_Target\0",64,0,15)]))] };
    let mut container=vec![0;32+4*parts.len()]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container,20,1); put32(&mut container,28,parts.len() as u32);
    for (i,(tag,data)) in parts.into_iter().enumerate() { let offset=container.len() as u32; put32(&mut container,32+i*4,offset); container.extend_from_slice(tag); container.extend_from_slice(&(data.len() as u32).to_le_bytes()); container.extend_from_slice(&data); }
    let size=container.len() as u32; put32(&mut container,24,size); container
}
// The format library produces the guest allocation, not the sampled colors:
// the BC bytes and expected pixels are independently specified in the fixture.
#[allow(dead_code)]
#[path = "../../../crates/victory-formats/src/texture.rs"]
mod texture;

fn tiled(swizzle: u32, bc3: bool) -> Vec<u8> {
    use texture::{Desc, Dimension, Layout, Swizzle};
    let bytes = if bc3 { 16 } else { 8 };
    let layout = Layout::new(&Desc { dimension: Dimension::Tex2d, width: 32, height: 32,
        depth: 1, mips: 6, element_bytes: bytes, block: 4, samples: 1,
        swizzle: Swizzle(swizzle) }).expect("BC tiled layout");
    let mut data = vec![0xcc; layout.size as usize];
    let colors: [u16; 6] = [0xf800, 0x07e0, 0x001f, 0xffff, 0xf800, 0x07e0];
    for mip in 0..6 {
        let blocks = ((32u32 >> mip) + 3) / 4;
        for y in 0..blocks { for x in 0..blocks {
            let offset = layout.offset(mip, x, y, 0).expect("BC tile address") as usize;
            let block = &mut data[offset..offset + bytes as usize];
            block.fill(0);
            let color_at = if bc3 {
                // Alpha endpoint 0 is 255; endpoint 1 is 0. Pixel (1,0)
                // selects endpoint 1, every other pixel selects endpoint 0.
                block[0] = 255; block[1] = 0; block[2] = 8;
                8
            } else { 0 };
            let color = (if mip == 0 && x == 4 && y == 3 { 0x001f } else { colors[mip as usize] }).to_le_bytes();
            block[color_at..color_at + 2].copy_from_slice(&color);
            block[color_at + 2..color_at + 4].copy_from_slice(&color);
        }}
    }
    data
}

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" { fn MTLCreateSystemDefaultDevice() -> *mut std::ffi::c_void; }
#[link(name = "objc")]
unsafe extern "C" {
    fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
    fn objc_msgSend();
}
fn require_bc_hardware() {
    unsafe {
        let device = MTLCreateSystemDefaultDevice();
        assert!(!device.is_null(), "no Metal GPU available for BC sampling");
        let selector = sel_registerName(b"supportsBCTextureCompression\0".as_ptr().cast());
        let send: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> bool =
            std::mem::transmute(objc_msgSend as *const ());
        assert!(send(device, selector), "Metal GPU does not support native BC texture compression");
    }
}
fn tool(dir: &Path, name: &str, args: &[&str]) {
    let out=Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
}
#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_bc_sampling_through_native_cli() {
    require_bc_hardware();
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some(), "VICTORY_METAL_CONVERTER must name a trusted Apple Metal Shader Converter");
    let dir=tempfile::tempdir().unwrap(); let root=dir.path();
    let arrays=[("vs",shader(true)),("ps",shader(false))].into_iter().map(|(name,bytes)| format!("static const unsigned char {name}[] = {{{}}};\n",bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))).collect::<String>();
    std::fs::write(root.join("shaders.h"),arrays).unwrap();
    let arrays = [("bc_4k_1", tiled(5, false)), ("bc_4k_3", tiled(5, true)),
                  ("bc_64k_1", tiled(10, false)), ("bc_64k_3", tiled(10, true))]
        .into_iter().map(|(name, bytes)| format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))).collect::<String>();
    std::fs::write(root.join("bc_tiles.h"), arrays).unwrap();
    std::fs::write(root.join("bc.c"),include_str!("../../../tests/fixtures/macos_d3d12x_bc.c")).unwrap();
    std::fs::write(root.join("d3d.def"),"LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root,"llvm-dlltool",&["-m","i386:x86-64","-d","d3d.def","-l","d3d.lib"]);
    tool(root,"x86_64-w64-mingw32-clang",&["-O2","-fno-unwind-tables","-fno-asynchronous-unwind-tables","-c","bc.c","-o","bc.obj"]);
    tool(root,"lld-link",&["/entry:mainCRTStartup","/subsystem:console","/nodefaultlib","/dynamicbase","/out:bc.exe","bc.obj","d3d.lib"]);
    let output=Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("bc.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success(),"status: {}\nstdout: {}\nstderr: {}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"),"stdout: {}\nstderr: {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
}
