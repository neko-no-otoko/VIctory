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
fn tool(dir: &Path, name: &str, args: &[&str]) {
    let out=Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
}
#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_mip_sampling_through_native_cli() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir=tempfile::tempdir().unwrap(); let root=dir.path();
    let arrays=[("vs",shader(true)),("ps",shader(false))].into_iter().map(|(name,bytes)| format!("static const unsigned char {name}[] = {{{}}};\n",bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))).collect::<String>();
    std::fs::write(root.join("shaders.h"),arrays).unwrap();
    std::fs::write(root.join("mips.c"),include_str!("../../../tests/fixtures/macos_d3d12x_mips.c")).unwrap();
    std::fs::write(root.join("d3d.def"),"LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root,"llvm-dlltool",&["-m","i386:x86-64","-d","d3d.def","-l","d3d.lib"]);
    tool(root,"x86_64-w64-mingw32-clang",&["-O2","-fno-unwind-tables","-fno-asynchronous-unwind-tables","-c","mips.c","-o","mips.obj"]);
    tool(root,"lld-link",&["/entry:mainCRTStartup","/subsystem:console","/nodefaultlib","/dynamicbase","/out:mips.exe","mips.obj","d3d.lib"]);
    let output=Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("mips.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success(),"status: {}\nstdout: {}\nstderr: {}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"),"stdout: {}\nstderr: {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
}
