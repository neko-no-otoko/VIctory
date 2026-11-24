#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::process::Command;
use victory_loader::{ImportResolver, Resolution};
use victory_pe::ImportSymbol;
fn run(dir: &std::path::Path, tool: &str, args: &[&str]) {
    let out = Command::new(tool).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{tool}: {}", String::from_utf8_lossy(&out.stderr));
}
#[test]
fn guest_queries_system_information() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("xg.def"), "LIBRARY xgameruntime.dll\nEXPORTS\nQueryApiImpl\n").unwrap();
    std::fs::write(dir.path().join("guest.c"), r#"
struct Guid { unsigned a; unsigned short b,c; unsigned char d[8]; };
__declspec(dllimport) long QueryApiImpl(const struct Guid *, const struct Guid *, void **);
static const struct Guid provider = {0xe349bd1a,0xfc20,0x4e40,{0xb9,0x9c,0x41,0x78,0xcc,0x6b,0x40,0x9f}};
static const struct Guid iid = {0x6fd71f09,0x7513,0x49f0,{0x89,0xbc,0xbf,0xaf,0x5d,0xf6,0xf8,0x52}};
static const struct Guid unknown = {0,0,0,{0,0,0,0,0,0,0,0}};
int marker; __declspec(dllexport) int *pointer = &marker;
__declspec(dllexport) unsigned guest(void) {
 void *obj=0; if (QueryApiImpl(&provider,&iid,&obj) || !obj) return 1;
 void **v=*(void ***)obj; typedef long (*Get)(void *, unsigned long long, char *, unsigned long long *);
 char buffer[64]; unsigned long long used=99;
 if (((Get)v[4])(obj,6,buffer,&used) != (long)0x8007007aul || used != 99) return 2;
 if (((Get)v[4])(obj,7,buffer,&used) || used != 7) return 3;
 if (buffer[0]!='R' || buffer[1]!='E' || buffer[2]!='T' || buffer[3]!='A' || buffer[4]!='I' || buffer[5]!='L' || buffer[6]) return 4;
 if (((Get)v[3])(obj,64,buffer,&used) || used != 39) return 5;
 if (((Get)v[5])(obj,64,buffer,&used) || used != 45) return 6;
 if ((unsigned long)QueryApiImpl(0,&iid,&obj) != 0x80070032ul) return 7;
 if ((unsigned long)QueryApiImpl(&provider,&iid,0) != 0x80070032ul) return 8;
 if ((unsigned long)QueryApiImpl(&unknown,&iid,&obj) != 0x80070032ul) return 10;
 if ((unsigned long)((Get)v[4])(obj,7,0,&used) != 0x8007007aul) return 11;
 if (((unsigned (*)(void *))v[2])(obj) != 1) return 9;
 return 42;
}
"#).unwrap();
    run(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "xg.def", "-l", "xg.lib"]);
    run(dir.path(), "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "guest.c", "-o", "guest.obj"]);
    run(dir.path(), "lld-link", &["/dll", "/noentry", "/nodefaultlib", "/dynamicbase", "/out:guest.dll", "guest.obj", "xg.lib"]);
    let resolver = victory_runtime::NativeResolver;
    let unknown = ImportSymbol::Name { hint: 0, name: "XGameRuntimeInitialize".into() };
    assert!(matches!(resolver.resolve("xgameruntime.dll", &unknown), Resolution::Missing));
    let image = victory_loader::macos::Image::load(&std::fs::read(dir.path().join("guest.dll")).unwrap(), &resolver).unwrap();
    let entry = image.export("guest").unwrap();
    // SAFETY: this synthetic export has the stated ABI and returns normally.
    let call: unsafe extern "win64" fn() -> u32 = unsafe { std::mem::transmute(entry.as_ptr()) };
    assert_eq!(unsafe { call() }, 42);
}
