#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::process::Command;
use victory_loader::{ImportResolver, Resolution};
use victory_pe::ImportSymbol;

fn run(dir: &std::path::Path, tool: &str, args: &[&str]) {
    let out = Command::new(tool).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{tool}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn guest_calls_real_shared_xg_and_unported_names_remain_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("xg.def"), "LIBRARY xg_xs.dll\nEXPORTS\nXGSuggestSwizzleMode\nXGCreateTextureComputer\n").unwrap();
    std::fs::write(dir.path().join("guest.c"), r#"
struct Desc { unsigned dim; unsigned long long align, width; unsigned height; unsigned short depth, mips; unsigned format, samples, quality, layout, flags; };
__declspec(dllimport) long XGSuggestSwizzleMode(const struct Desc *, unsigned, unsigned *);
__declspec(dllimport) long XGCreateTextureComputer(const struct Desc *, void **);
static unsigned char layout[0x1b40] __attribute__((aligned(16)));
static unsigned char tiled[131072], linear[65536], result[65536];
int marker; __declspec(dllexport) int *pointer = &marker;
__declspec(dllexport) unsigned guest(void) {
    struct Desc d = {0}; unsigned out = 0;
    d.dim = 2; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x100) return 1;
    d.dim = 3; d.flags = 1; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x11b) return 2;
    d.flags = 2; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x118) return 3;
    d.flags = 0; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x105) return 4;
    if ((unsigned long)XGSuggestSwizzleMode(0, 0, &out) != 0x80070057ul) return 5;
    void *computer = 0;
    if ((unsigned long)XGCreateTextureComputer(0, &computer) != 0x80070057ul) return 7;
    d.width=256; d.height=256; d.depth=1; d.mips=0; d.format=98; d.samples=1; d.layout=0x10a;
    if (XGCreateTextureComputer(&d, &computer) || !computer) return 8;
    void **table = *(void ***)computer;
    typedef unsigned (*Ref)(void *);
    typedef long (*Layout)(void *, void *);
    if (((Ref)table[0])(computer) != 2) return 9;
    if (((Layout)table[2])(computer, layout)) return 10;
    if (*(unsigned long long *)layout != 131072 || *(unsigned long long *)(layout+8) != 65536 || *(unsigned *)(layout+16) != 9 || *(unsigned *)(layout+20) != 1) return 11;
    if ((unsigned long)((Layout)table[2])(computer, 0) != 0x80070057ul) return 12;
    typedef long (*Into)(void *, unsigned char *, unsigned, unsigned, const unsigned char *, unsigned, unsigned);
    typedef long (*From)(void *, unsigned char *, unsigned, unsigned, unsigned, const unsigned char *, unsigned, unsigned);
    for (unsigned i=0; i<65536; i++) ((volatile unsigned char *)linear)[i]=(unsigned char)i;
    if (((Into)table[8])(computer, tiled, 0, 0, linear, 1024, 0)) return 14;
    if (((From)table[9])(computer, result, 0, 0, 0, tiled, 1024, 0)) return 15;
    for (unsigned i=0; i<65536; i++) if (((volatile unsigned char *)result)[i] != (unsigned char)i) return 16;
    if ((unsigned long)((long (*)(void *))table[3])(computer) != 0x80004001ul) return 17;
    if (((Ref)table[1])(computer) != 1 || ((Ref)table[1])(computer) != 0) return 13;
    if ((unsigned long)XGSuggestSwizzleMode(&d, 0, 0) != 0x80070057ul) return 6;
    return 42;
}
"#).unwrap();
    run(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "xg.def", "-l", "xg.lib"]);
    run(dir.path(), "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "guest.c", "-o", "guest.obj"]);
    run(dir.path(), "lld-link", &["/dll", "/noentry", "/nodefaultlib", "/dynamicbase", "/out:guest.dll", "guest.obj", "xg.lib"]);
    let bytes = std::fs::read(dir.path().join("guest.dll")).unwrap();
    let resolver = victory_xg::NativeResolver;
    let unknown = ImportSymbol::Name { hint: 0, name: "XGUnknown".into() };
    assert!(matches!(resolver.resolve("xg_xs.dll", &unknown), Resolution::Missing));
    assert!(matches!(resolver.resolve("xg_xs.dll", &ImportSymbol::Ordinal(1)), Resolution::Missing));
    let xgame = ImportSymbol::Name { hint: 0, name: "XGameRuntimeInitialize".into() };
    assert!(matches!(resolver.resolve("xgameruntime.dll", &xgame), Resolution::Missing));
    let image = victory_loader::macos::Image::load(&bytes, &resolver).unwrap();
    let entry = image.export("guest").unwrap();
    // SAFETY: our synthetic export uses this exact Windows x64 signature.
    let call: unsafe extern "win64" fn() -> u32 = unsafe { std::mem::transmute(entry.as_ptr()) };
    assert_eq!(unsafe { call() }, 42);
}
