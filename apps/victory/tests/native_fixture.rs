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

#[test]
fn native_fixture_queries_offline_user() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("runtime.def"), "LIBRARY xgameruntime.dll\nEXPORTS\nQueryApiImpl\n").unwrap();
    std::fs::write(root.join("user.c"), r#"
struct Guid { unsigned a; unsigned short b,c; unsigned char d[8]; };
__declspec(dllimport) long QueryApiImpl(const struct Guid *, const struct Guid *, void **);
static const struct Guid provider = {0x01acd177,0x91f9,0x4763,{0xa3,0x8e,0xcc,0xbb,0x55,0xce,0x32,0xe0}};
static const struct Guid iid = {0xeb9bf948,0x18dc,0x4d82,{0xbb,0xcc,0x40,0xe0,0xa8,0x09,0xc4,0xc0}};
typedef long (*Find)(void *, unsigned long long, void **);
typedef long (*GetId)(void *, void *, unsigned long long *);
typedef long (*GetGamertag)(void *, void *, unsigned, unsigned long long, char *, unsigned long long *);
typedef long (*Duplicate)(void *, void *, void **);
typedef void (*Close)(void *, void *);
typedef int (*Compare)(void *, void *, void *);
typedef long (*Unsupported)(void *);
typedef unsigned (*Release)(void *);
unsigned entry(void) {
    void *object=0, *user=0, *duplicate=0, *missing=(void *)1;
    unsigned long long id=0, used=0;
    char name[7]={0}, small[6]={'x','x','x','x','x','x'};
    if (QueryApiImpl(&provider,&iid,&object) || !object) return 1;
    void **v=*(void ***)object;
    if (((Find)v[12])(object,0x9000000000001ULL,&user) || !user) return 2;
    if (((GetId)v[9])(object,user,&id) || id!=1) return 3;
    if (((GetId)v[11])(object,user,&id) || id!=0x9000000000001ULL) return 4;
    if ((unsigned long)((Find)v[10])(object,2,&missing)!=0x89245104UL || missing) return 5;
    if (((GetGamertag)v[15])(object,user,0,sizeof(name),name,&used) || used!=7 ||
        name[0]!='P' || name[1]!='l' || name[2]!='a' || name[3]!='y' ||
        name[4]!='e' || name[5]!='r' || name[6]) return 6;
    used=99;
    if ((unsigned long)((GetGamertag)v[15])(object,user,0,sizeof(small),small,&used)!=0x8007007aUL ||
        used!=99 || small[0]!='x' || small[5]!='x') return 7;
    if (((Duplicate)v[3])(object,user,&duplicate) || !duplicate) return 8;
    if (((Compare)v[5])(object,user,duplicate)!=0) return 9;
    ((Close)v[4])(object,duplicate);
    if (((GetId)v[11])(object,user,&id) || id!=0x9000000000001ULL) return 10;
    if ((unsigned long)((Unsupported)v[6])(object)!=0x80004001UL) return 11;
    ((Close)v[4])(object,user);
    if (((Release)v[2])(object)!=1) return 12;
    return 42;
}
"#).unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "runtime.def", "-l", "runtime.lib"]);
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "user.c", "-o", "user.obj"]);
    tool(root, "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:user.exe", "user.obj", "runtime.lib"]);
    let exe = root.join("user.exe");
    let executed = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--execute".as_ref()]);
    assert!(executed.status.success(), "{}", String::from_utf8_lossy(&executed.stderr));
    assert!(String::from_utf8_lossy(&executed.stdout).contains("fixture entry returned 42"), "{}", String::from_utf8_lossy(&executed.stdout));
}

#[test]
fn explicit_dll_dependencies_attach_in_order_and_reject_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("failed")).unwrap();
    std::fs::write(root.join("provider.def"), "LIBRARY provider.dll\nEXPORTS\nis_live\nnote_attach\nnote_detach\n").unwrap();
    std::fs::write(root.join("consumer.def"), "LIBRARY consumer.dll\nEXPORTS\nconsumer_call\n").unwrap();
    std::fs::write(root.join("provider.c"), r#"
static int live, attached, detached;
int *volatile relocate = &live;
__declspec(dllexport) int is_live(void) { return *relocate; }
__declspec(dllexport) void note_attach(void) { attached = 1; }
__declspec(dllexport) void note_detach(void) { detached = 1; }
int DllMain(void *m, unsigned long reason, void *r) {
    (void)m; (void)r;
    if (reason == 1) live = 1;
    if (reason == 0) { if (attached && !detached) __builtin_trap(); live = 0; }
    return 1;
}
"#).unwrap();
    std::fs::write(root.join("consumer.c"), r#"
__declspec(dllimport) int is_live(void);
__declspec(dllimport) void note_attach(void);
__declspec(dllimport) void note_detach(void);
static int live;
int *volatile relocate = &live;
__declspec(dllexport) int consumer_call(void) { return *relocate && is_live() ? 42 : 0; }
int DllMain(void *m, unsigned long reason, void *r) {
    (void)m; (void)r;
    if (reason == 1) {
        if (!is_live()) return 0;
#ifdef FAIL_ATTACH
        return 0;
#else
        live = 1; note_attach();
#endif
    }
    if (reason == 0) { if (!is_live()) __builtin_trap(); note_detach(); live = 0; }
    return 1;
}
"#).unwrap();
    std::fs::write(root.join("program.c"), r#"
__declspec(dllimport) int consumer_call(void);
int value = 7; int *volatile relocate = &value;
unsigned entry(void) { return *relocate == 7 ? consumer_call() : 0; }
"#).unwrap();
    for (def, lib) in [("provider.def", "provider.lib"), ("consumer.def", "consumer.lib")] {
        tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", def, "-l", lib]);
    }
    for (source, object, define) in [("provider.c", "provider.obj", ""), ("consumer.c", "consumer.obj", ""), ("consumer.c", "failed.obj", "-DFAIL_ATTACH"), ("program.c", "program.obj", "")] {
        let mut args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", source, "-o", object];
        if !define.is_empty() { args.push(define); }
        tool(root, "x86_64-w64-mingw32-clang", &args);
    }
    tool(root, "lld-link", &["/dll", "/entry:DllMain", "/nodefaultlib", "/dynamicbase", "/out:PROVIDER.DLL", "provider.obj"]);
    tool(root, "lld-link", &["/dll", "/entry:DllMain", "/nodefaultlib", "/dynamicbase", "/out:consumer.dll", "consumer.obj", "provider.lib"]);
    tool(root, "lld-link", &["/dll", "/entry:DllMain", "/nodefaultlib", "/dynamicbase", "/out:failed/consumer.dll", "failed.obj", "provider.lib"]);
    tool(root, "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:program.exe", "program.obj", "consumer.lib"]);
    let exe = root.join("program.exe");
    let provider = root.join("PROVIDER.DLL");
    let consumer = root.join("consumer.dll");
    let failed = root.join("failed/consumer.dll");
    // Deliberately provide the dependent DLL first and vary the provider's
    // filename case; attachment must still follow the import graph.
    let args = ["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), consumer.as_os_str(), "--dll".as_ref(), provider.as_os_str()];
    let dry = cli(&args);
    assert!(dry.status.success(), "{}", String::from_utf8_lossy(&dry.stderr));
    assert!(String::from_utf8_lossy(&dry.stdout).contains("entry not executed"));
    let mut exec_args = args.to_vec();
    exec_args.push("--execute".as_ref());
    let executed = cli(&exec_args);
    assert!(executed.status.success(), "{}", String::from_utf8_lossy(&executed.stderr));
    assert!(String::from_utf8_lossy(&executed.stdout).contains("fixture entry returned 42"));
    // This DLL rejects process attach, so mapping succeeds only if dry-run
    // leaves DLL entry points untouched.
    let failed_dry = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), failed.as_os_str(), "--dll".as_ref(), provider.as_os_str()]);
    assert!(failed_dry.status.success(), "{}", String::from_utf8_lossy(&failed_dry.stderr));
    let failed_attach = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), failed.as_os_str(), "--dll".as_ref(), provider.as_os_str(), "--execute".as_ref()]);
    assert!(!failed_attach.status.success());
    assert!(String::from_utf8_lossy(&failed_attach.stderr).contains("attaching consumer.dll"));
    let missing = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), consumer.as_os_str()]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("is_live"));
    let duplicate = root.join("different/CONSUMER.DLL");
    let duplicate = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), consumer.as_os_str(), "--dll".as_ref(), duplicate.as_os_str()]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate DLL filename"));
    let invalid = root.join("invalid.dll");
    std::fs::write(&invalid, b"not a PE").unwrap();
    assert!(!cli(&["native-fixture".as_ref(), exe.as_os_str(), "--dll".as_ref(), invalid.as_os_str()]).status.success());
}
