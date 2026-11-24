#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::path::Path;
use std::process::Command;

fn put32(b: &mut [u8], at: usize, n: u32) { b[at..at+4].copy_from_slice(&n.to_le_bytes()); }
fn put64(b: &mut [u8], at: usize, n: u64) { b[at..at+8].copy_from_slice(&n.to_le_bytes()); }
fn signature(rows: &[(&[u8], u32, u32, u8)]) -> Vec<u8> {
    let mut s=vec![0;8+32*rows.len()]; put32(&mut s,0,rows.len() as u32); put32(&mut s,4,8);
    for (i,&(name,system,row,mask)) in rows.iter().enumerate() { let at=8+32*i; let off=s.len() as u32; put32(&mut s,at+4,off); put32(&mut s,at+12,system); put32(&mut s,at+16,3); put32(&mut s,at+20,row); s[at+24]=mask; s.extend_from_slice(name); }
    s
}
// Console GFX10 shaders: position/UV sampling and position/constant color.
fn shader(vertex: bool, color: bool) -> Vec<u8> {
    let words: &[u32]=match (vertex,color) {
        (true,false)=>&[0xf8000203,0x00000908,0xf80018cf,0x07060504,0xbf810000],
        (false,false)=>&[0xc8080000,0xc8090001,0xc80c0100,0xc80d0101,0xbe820309,0xbe890380,0xbe830380,0xf4080204,0xfa000000,0xf40c0001,0xfa000000,0xf0800f08,0x00400002,0xf800180f,0x03020100,0xbf810000],
        (true,true)=>&[0xf80018cf,0x07060504,0xbf810000],
        (false,true)=>&[0x7e000200,0x7e020201,0x7e040202,0x7e060203,0xf800180f,0x03020100,0xbf810000],
    };
    let mut p=vec![0;0x68]; put64(&mut p,0,6);
    for at in [8,0x10,0x18,0x20,0x28,0x30,0x48,0x60] { put64(&mut p,at,0xdeadbeefdeadbeef); }
    let record=p.len(); put64(&mut p,if vertex {0x28} else {0x30},record as u64);
    let stage=record+0x54; let code=stage+if vertex {0x18c} else {0x14c+8*404};
    p.resize(code,0); put64(&mut p,record,code as u64); put64(&mut p,record+8,(words.len()*4) as u64); put32(&mut p,record+0x14,0x20); put32(&mut p,record+0x50,8);
    if vertex {
        p[stage+0x24..stage+0x29].copy_from_slice(&[6,0,15,4,0]);
        if !color { p[stage+0x29..stage+0x2e].copy_from_slice(&[6,1,3,8,0]); }
        put32(&mut p,stage+0xc4,if color {1} else {2});
        if !color { p[stage+0xc8..stage+0xce].copy_from_slice(&[6,1,0,1,0,0]); put32(&mut p,stage+0x188,1); }
    } else {
        if !color { p[stage+0x24..stage+0x2c].copy_from_slice(&[6,0,3,0,0,0,0,0]); put32(&mut p,stage+0x124,1); put32(&mut p,stage+0x134,2); }
        for i in 0..8 { put32(&mut p,stage+0x14c+i*404,u32::MAX); }
        if color { put64(&mut p,record+0x28,(code+words.len()*4) as u64); put32(&mut p,record+0x38,1); }
    }
    for w in words { p.extend_from_slice(&w.to_le_bytes()); }
    if !vertex && color { let root=p.len(); p.resize(root+20,0); put32(&mut p,root+8,4); p[root+17]=3; }
    if !vertex && !color {
        let layout=p.len(); p.resize(layout+100,0); put64(&mut p,record+0x28,layout as u64); put64(&mut p,record+0x30,(layout+60) as u64); put32(&mut p,record+0x38,3); put32(&mut p,record+0x3c,2);
        for (i,(kind,user,slot)) in [(0,0,0),(3,8,1),(3,9,2)].into_iter().enumerate() { let at=layout+i*20; put32(&mut p,at,kind); put32(&mut p,at+8,1); p[at+16]=user; p[at+17]=slot; }
        for (i,(kind,dwords,user,slot)) in [(1,4,8,1),(0,8,9,2)].into_iter().enumerate() { let at=layout+60+i*20; p[at]=kind; p[at+1]=3; p[at+16]=dwords; p[at+17]=user; p[at+18]=slot; }
    }
    let mut f=b"FSSM".to_vec(); f.extend_from_slice(&0x101u32.to_le_bytes()); f.extend_from_slice(&(p.len() as u64).to_le_bytes()); f.extend_from_slice(&p);
    let packed=miniz_oxide::deflate::compress_to_vec_zlib(&f,6); let mut blob=b"XBCH".to_vec(); blob.extend_from_slice(&3u32.to_le_bytes()); blob.extend_from_slice(&(packed.len() as u64).to_le_bytes()); blob.extend_from_slice(&(f.len() as u64).to_le_bytes()); blob.extend_from_slice(&packed);
    let mut dxil=vec![0;0x18]; dxil.extend_from_slice(b"XDAT"); for n in [1u32,0x24,0x0d,0x30,blob.len() as u32] { dxil.extend_from_slice(&n.to_le_bytes()); } dxil.extend_from_slice(&blob);
    let mut parts=vec![(b"DXIL",dxil)];
    if vertex { parts.push((b"ISG1",if color {signature(&[(b"POSITION\0",0,0,15)])} else {signature(&[(b"POSITION\0",0,0,15),(b"TEXCOORD\0",0,1,3)])})); }
    else if !color { parts.push((b"ISG1",signature(&[(b"TEXCOORD\0",0,0,3)]))); }
    parts.push((b"OSG1",if vertex { if color {signature(&[(b"SV_Position\0",1,0,15)])} else {signature(&[(b"SV_Position\0",1,0,15),(b"TEXCOORD\0",0,1,3)])} } else {signature(&[(b"SV_Target\0",64,0,15)])}));
    let mut container=vec![0;32+4*parts.len()]; container[..4].copy_from_slice(b"DXBC"); put32(&mut container,20,1); put32(&mut container,28,parts.len() as u32);
    for (i,(tag,data)) in parts.into_iter().enumerate() { let offset=container.len() as u32; put32(&mut container,32+i*4,offset); container.extend_from_slice(tag); container.extend_from_slice(&(data.len() as u32).to_le_bytes()); container.extend_from_slice(&data); }
    let size=container.len() as u32; put32(&mut container,24,size); container
}
fn decode(x: f64) -> f64 { if x<=0.04045 {x/12.92} else {((x+0.055)/1.055).powf(2.4)} }
fn encode(x: f64) -> f64 { if x<=0.0031308 {12.92*x} else {1.055*x.powf(1.0/2.4)-0.055} }
fn byte(x: f64) -> u8 { (x.clamp(0.0,1.0)*255.0).round() as u8 }
fn encoded(v: [f64;4]) -> [u8;4] { [byte(encode(v[0])),byte(encode(v[1])),byte(encode(v[2])),byte(v[3])] }
fn blend(dst: [u8;4], src: [f64;4]) -> [u8;4] {
    let mut v=[0.0;4]; for c in 0..3 { v[c]=src[c]*src[3]+decode(dst[c] as f64/255.0)*(1.0-src[3]); }
    v[3]=src[3]+dst[3] as f64/255.0*(1.0-src[3]); encoded(v)
}
fn reference_header() -> String {
    // RGB565 endpoints expanded exactly; mip 1 deliberately differs from mip 0.
    let endpoints=[0x8a6au16,0x4c96,0xb38d]; let mut samples=Vec::new();
    for srgb in [false,true] { for &e in &endpoints { let r=((e>>11)&31) as u8; let g=((e>>5)&63) as u8; let b=(e&31) as u8; let rgb=[(r<<3)|(r>>2),(g<<2)|(g>>4),(b<<3)|(b>>2)]; let mut v=[0u8;3]; for c in 0..3 { v[c]=if srgb {byte(decode(rgb[c] as f64/255.0))} else {rgb[c]}; } samples.push(v); } }
    let full=encoded([0.0023,0.3487,0.7023,0.4137]); let rect=encoded([0.0013,0.6147,0.0893,0.6237]);
    let red=[0.75,0.125,0.25,0.5]; let green=[0.125,0.75,0.375,0.5];
    let targets=[full,rect,encoded([0.25,0.5,0.75,0.375]),blend(blend(full,red),green),blend(blend(full,green),red)];
    let rows=|v: &[[u8;4]]| v.iter().map(|r| format!("{{{},{},{},{}}}",r[0],r[1],r[2],r[3])).collect::<Vec<_>>().join(",");
    let rgb_rows=|v: &[[u8;3]]| v.iter().map(|r| format!("{{{},{},{}}}",r[0],r[1],r[2])).collect::<Vec<_>>().join(",");
    let groups=format!("{{{}}},{{{}}}",rgb_rows(&samples[..3]),rgb_rows(&samples[3..]));
    format!("static const unsigned short endpoints[3]={{0x8a6a,0x4c96,0xb38d}};\nstatic const unsigned char sample_expected[2][3][3]={{{}}};\nstatic const unsigned char target_expected[5][4]={{{}}};\n",groups,rows(&targets))
}
#[link(name="Metal",kind="framework")]
unsafe extern "C" { fn MTLCreateSystemDefaultDevice() -> *mut std::ffi::c_void; }
#[link(name="objc")]
unsafe extern "C" { fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void; fn objc_msgSend(); }
fn require_bc_hardware() { unsafe { let dev=MTLCreateSystemDefaultDevice(); assert!(!dev.is_null(),"no Metal GPU"); let sel=sel_registerName(b"supportsBCTextureCompression\0".as_ptr().cast()); let send: unsafe extern "C" fn(*mut std::ffi::c_void,*const std::ffi::c_void)->bool=std::mem::transmute(objc_msgSend as *const ()); assert!(send(dev,sel),"Metal GPU does not support native BC compression"); } }
fn tool(dir: &Path, name: &str, args: &[&str]) { let out=Command::new(name).args(args).current_dir(dir).output().unwrap(); assert!(out.status.success(),"{name}: {}",String::from_utf8_lossy(&out.stderr)); }
#[test]
#[ignore = "requires LLVM MinGW, a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_srgb_graphics_through_native_cli() {
    require_bc_hardware(); assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir=tempfile::tempdir().unwrap(); let root=dir.path();
    let arrays=[("vs",shader(true,false)),("ps",shader(false,false)),("color_vs",shader(true,true)),("color_ps",shader(false,true))].into_iter().map(|(name,bytes)| format!("static const unsigned char {name}[] = {{{}}};\n",bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(","))).collect::<String>();
    std::fs::write(root.join("shaders.h"),arrays).unwrap(); std::fs::write(root.join("reference.h"),reference_header()).unwrap();
    std::fs::write(root.join("srgb.c"),include_str!("../../../tests/fixtures/macos_d3d12x_srgb.c")).unwrap();
    std::fs::write(root.join("d3d.def"),"LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root,"llvm-dlltool",&["-m","i386:x86-64","-d","d3d.def","-l","d3d.lib"]);
    tool(root,"x86_64-w64-mingw32-clang",&["-O2","-fno-unwind-tables","-fno-asynchronous-unwind-tables","-c","srgb.c","-o","srgb.obj"]);
    tool(root,"lld-link",&["/entry:mainCRTStartup","/subsystem:console","/nodefaultlib","/dynamicbase","/out:srgb.exe","srgb.obj","d3d.lib"]);
    let output=Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("srgb.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success() && String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"),"status: {}\nstdout: {}\nstderr: {}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
}
