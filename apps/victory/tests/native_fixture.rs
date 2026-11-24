#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::process::{Command, Output};

fn tool(dir: &std::path::Path, name: &str, args: &[&str]) {
    let output = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
}

fn cli(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_victory")).args(args).output().unwrap()
}

#[test]
fn controlled_executable_uses_both_services_only_with_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("xg.def"), "LIBRARY xg_xs.dll\nEXPORTS\nXGSuggestSwizzleMode\nXGUnimplemented\n").unwrap();
    std::fs::write(dir.path().join("runtime.def"), "LIBRARY xgameruntime.dll\nEXPORTS\nQueryApiImpl\n").unwrap();
    std::fs::write(dir.path().join("fixture.c"), r#"
struct Desc { unsigned dim; unsigned long long align, width; unsigned height; unsigned short depth, mips; unsigned format, samples, quality, layout, flags; };
struct Guid { unsigned a; unsigned short b,c; unsigned char d[8]; };
__declspec(dllimport) long XGSuggestSwizzleMode(const struct Desc *, unsigned, unsigned *);
__declspec(dllimport) long QueryApiImpl(const struct Guid *, const struct Guid *, void **);
#ifdef MISSING
__declspec(dllimport) int XGUnimplemented(void);
#endif
static const struct Guid provider = {0xe349bd1a,0xfc20,0x4e40,{0xb9,0x9c,0x41,0x78,0xcc,0x6b,0x40,0x9f}};
static const struct Guid iid = {0x6fd71f09,0x7513,0x49f0,{0x89,0xbc,0xbf,0xaf,0x5d,0xf6,0xf8,0x52}};
int marker=7; int *volatile relocated_pointer=&marker;
unsigned entry(void) {
#ifdef TRAP
    if (*relocated_pointer == 7) __builtin_trap();
#endif
    struct Desc desc={0}; unsigned mode=0; void *object=0; char buffer[7]; unsigned long long used=0;
    desc.dim=2;
    if (*relocated_pointer!=7 || XGSuggestSwizzleMode(&desc,0,&mode) || mode!=0x100) return 1;
    if (QueryApiImpl(&provider,&iid,&object) || !object) return 2;
    void **v=*(void ***)object;
    typedef long (*Get)(void *, unsigned long long, char *, unsigned long long *);
    if (((Get)v[4])(object,7,buffer,&used) || used!=7) return 3;
    if (buffer[0]!='R' || buffer[1]!='E' || buffer[2]!='T' || buffer[3]!='A' || buffer[4]!='I' || buffer[5]!='L' || buffer[6]) return 4;
#ifdef MISSING
    return XGUnimplemented();
#else
    return 42;
#endif
}
"#).unwrap();
    for (def, lib) in [("xg.def", "xg.lib"), ("runtime.def", "runtime.lib")] {
        tool(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", def, "-l", lib]);
    }
    for (object, exe, define) in [("fixture.obj", "fixture.exe", ""), ("missing.obj", "missing.exe", "-DMISSING"), ("trap.obj", "trap.exe", "-DTRAP")] {
        let mut args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "fixture.c", "-o", object];
        if !define.is_empty() { args.push(define); }
        tool(dir.path(), "x86_64-w64-mingw32-clang", &args);
        tool(dir.path(), "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", &format!("/out:{exe}"), object, "xg.lib", "runtime.lib"]);
    }
    let fixture = dir.path().join("fixture.exe");
    let trap = dir.path().join("trap.exe");
    let dry = cli(&["native-fixture".as_ref(), trap.as_os_str()]);
    assert!(dry.status.success(), "{}", String::from_utf8_lossy(&dry.stderr));
    assert!(String::from_utf8_lossy(&dry.stdout).contains("entry not executed"));
    assert!(String::from_utf8_lossy(&dry.stderr).contains("not Windows process startup or game support"));
    let executed = cli(&["native-fixture".as_ref(), fixture.as_os_str(), "--execute".as_ref()]);
    assert!(executed.status.success(), "{}", String::from_utf8_lossy(&executed.stderr));
    assert!(String::from_utf8_lossy(&executed.stdout).contains("fixture entry returned 42"));
    let missing = dir.path().join("missing.exe");
    let failed = cli(&["native-fixture".as_ref(), missing.as_os_str(), "--execute".as_ref()]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("XGUnimplemented"));
    let invalid = dir.path().join("invalid.exe");
    std::fs::write(&invalid, b"not a PE").unwrap();
    assert!(!cli(&["native-fixture".as_ref(), invalid.as_os_str(), "--execute".as_ref()]).status.success());
}
