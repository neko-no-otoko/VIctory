#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::process::Command;
use victory_loader::{ImportResolver, Resolution, macos::Executable};
use victory_pe::ImportSymbol;

fn run(dir: &std::path::Path, tool: &str, args: &[&str]) {
    let output = Command::new(tool).args(args).current_dir(dir).output().unwrap();
    assert!(output.status.success(), "{tool}: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn executable_uses_both_services_and_missing_imports_still_fail() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("xg.def"), "LIBRARY xg_xs.dll\nEXPORTS\nXGSuggestSwizzleMode\nXGUnimplemented\n").unwrap();
    std::fs::write(dir.path().join("runtime.def"), "LIBRARY xgameruntime.dll\nEXPORTS\nQueryApiImpl\n").unwrap();
    std::fs::write(dir.path().join("program.c"), r#"
struct Desc { unsigned dim; unsigned long long align, width; unsigned height; unsigned short depth, mips; unsigned format, samples, quality, layout, flags; };
struct Guid { unsigned a; unsigned short b,c; unsigned char d[8]; };
__declspec(dllimport) long XGSuggestSwizzleMode(const struct Desc *, unsigned, unsigned *);
__declspec(dllimport) long QueryApiImpl(const struct Guid *, const struct Guid *, void **);
#ifdef MISSING
__declspec(dllimport) int XGUnimplemented(void);
#endif
static const struct Guid provider = {0xe349bd1a,0xfc20,0x4e40,{0xb9,0x9c,0x41,0x78,0xcc,0x6b,0x40,0x9f}};
static const struct Guid iid = {0x6fd71f09,0x7513,0x49f0,{0x89,0xbc,0xbf,0xaf,0x5d,0xf6,0xf8,0x52}};
int marker=7; int *volatile relocated_pointer = &marker;
unsigned synthetic_entry(void) {
    struct Desc d = {0}; unsigned mode=0; void *object=0; char buffer[7]; unsigned long long used=0;
    d.dim=2;
    if (*relocated_pointer != 7 || XGSuggestSwizzleMode(&d,0,&mode) || mode != 0x100) return 1;
    if (QueryApiImpl(&provider,&iid,&object) || !object) return 2;
    void **v=*(void ***)object;
    typedef long (*Get)(void *, unsigned long long, char *, unsigned long long *);
    if (((Get)v[4])(object,7,buffer,&used) || used != 7) return 3;
    if (buffer[0]!='R' || buffer[1]!='E' || buffer[2]!='T' || buffer[3]!='A' || buffer[4]!='I' || buffer[5]!='L' || buffer[6]) return 4;
#ifdef MISSING
    return XGUnimplemented();
#else
    return 42;
#endif
}
"#).unwrap();
    for (def, lib) in [("xg.def", "xg.lib"), ("runtime.def", "runtime.lib")] {
        run(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", def, "-l", lib]);
    }
    let resolver = victory_native::NativeResolver;
    let named = |name: &str| ImportSymbol::Name { hint: 0, name: name.into() };
    assert_eq!(resolver.resolve("XG_XS.DLL", &named("XGSuggestSwizzleMode")), victory_xg::NativeResolver.resolve("XG_XS.DLL", &named("XGSuggestSwizzleMode")));
    assert_eq!(resolver.resolve("XGAMERUNTIME.DLL", &named("QueryApiImpl")), victory_runtime::NativeResolver.resolve("XGAMERUNTIME.DLL", &named("QueryApiImpl")));
    assert!(matches!(resolver.resolve("xg_xs.dll", &named("XGUnimplemented")), Resolution::Missing));
    assert!(matches!(resolver.resolve("xgameruntime.dll", &named("XGameRuntimeInitialize")), Resolution::Missing));
    assert!(matches!(resolver.resolve("xg_xs.dll", &ImportSymbol::Ordinal(1)), Resolution::Missing));
    assert!(matches!(resolver.resolve("other.dll", &named("QueryApiImpl")), Resolution::Missing));

    for (object, exe, extra) in [("program.obj", "program.exe", ""), ("missing.obj", "missing.exe", "-DMISSING")] {
        let mut args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "program.c", "-o", object];
        if !extra.is_empty() { args.push(extra); }
        run(dir.path(), "x86_64-w64-mingw32-clang", &args);
        run(dir.path(), "lld-link", &["/entry:synthetic_entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", &format!("/out:{exe}"), object, "xg.lib", "runtime.lib"]);
    }
    let bytes = std::fs::read(dir.path().join("program.exe")).unwrap();
    let mut exe = Executable::load(&bytes, &[], &resolver).unwrap();
    // SAFETY: this self-authored entry and its imports use the Windows x64
    // ABI, return normally, and require no Windows process environment.
    assert_eq!(unsafe { exe.start().unwrap() }, 42);
    let bytes = std::fs::read(dir.path().join("missing.exe")).unwrap();
    let error = match Executable::load(&bytes, &[], &resolver) {
        Ok(_) => panic!("unimplemented import was accepted"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("XGUnimplemented"), "{error:#}");
}
