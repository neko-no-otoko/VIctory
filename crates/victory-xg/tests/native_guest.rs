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
    std::fs::write(dir.path().join("xg.def"), "LIBRARY xg_xs.dll\nEXPORTS\nXGSuggestSwizzleMode\n").unwrap();
    std::fs::write(dir.path().join("guest.c"), r#"
struct Desc { unsigned dim; unsigned long long align, width; unsigned height; unsigned short depth, mips; unsigned format, samples, quality, layout, flags; };
__declspec(dllimport) long XGSuggestSwizzleMode(const struct Desc *, unsigned, unsigned *);
int marker; __declspec(dllexport) int *pointer = &marker;
__declspec(dllexport) unsigned guest(void) {
    struct Desc d = {0}; unsigned out = 0;
    d.dim = 2; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x100) return 1;
    d.dim = 3; d.flags = 1; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x11b) return 2;
    d.flags = 2; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x118) return 3;
    d.flags = 0; if (XGSuggestSwizzleMode(&d, 0, &out) || out != 0x105) return 4;
    if ((unsigned long)XGSuggestSwizzleMode(0, 0, &out) != 0x80070057ul) return 5;
    if ((unsigned long)XGSuggestSwizzleMode(&d, 0, 0) != 0x80070057ul) return 6;
    return 42;
}
"#).unwrap();
    run(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "xg.def", "-l", "xg.lib"]);
    run(dir.path(), "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "guest.c", "-o", "guest.obj"]);
    run(dir.path(), "lld-link", &["/dll", "/noentry", "/nodefaultlib", "/dynamicbase", "/out:guest.dll", "guest.obj", "xg.lib"]);
    let bytes = std::fs::read(dir.path().join("guest.dll")).unwrap();
    let resolver = victory_xg::NativeResolver;
    let unknown = ImportSymbol::Name { hint: 0, name: "XGCreateTextureComputer".into() };
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
