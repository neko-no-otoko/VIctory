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
int marker = 7; int *volatile relocated_marker = &marker;
unsigned entry(void) {
    void *object=0, *user=0, *duplicate=0, *missing=(void *)1;
    unsigned long long id=0, used=0;
    char name[7]={0}, small[6]={'x','x','x','x','x','x'};
    if (*relocated_marker != 7) return 13;
    if (QueryApiImpl(&provider,&iid,&object) || !object) return 1;
    void **v=*(void ***)object;
    if (((Find)v[12])(object,0x9000000000001ULL,&user) || !user) return 2;
    if (((GetId)v[9])(object,user,&id) || id!=1) return 3;
    if (((GetId)v[11])(object,user,&id) || id!=0x9000000000001ULL) return 4;
    if ((unsigned long)((Find)v[10])(object,2,&missing)!=0x89245104UL || missing!=(void *)1) return 5;
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

#[test]
fn native_fixture_reads_scoped_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("café.bin"), b"0123456789").unwrap();
    std::fs::write(dir.path().join("outside.bin"), b"outside").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside.bin"), root.join("link")).unwrap();
    std::fs::create_dir(root.join("MixedDir")).unwrap();
    std::fs::write(root.join("MixedDir").join("Data.bin"), b"drive").unwrap();
    std::os::unix::fs::symlink(dir.path(), root.join("dirlink")).unwrap();
    std::fs::write(root.join("Collision"), b"one").unwrap();
    let case_distinct = match std::fs::OpenOptions::new().write(true).create_new(true).open(root.join("collision")) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => panic!("creating case-distinct fixture: {error}"),
    };
    std::fs::write(dir.path().join("kernel32.def"), "LIBRARY kernel32.dll\nEXPORTS\nCreateFileW\nReadFile\nSetFilePointerEx\nGetFileSizeEx\nCloseHandle\nGetLastError\n").unwrap();
    std::fs::write(dir.path().join("files.c"), r#"
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;
__declspec(dllimport) HANDLE CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
__declspec(dllimport) BOOL ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
__declspec(dllimport) BOOL SetFilePointerEx(HANDLE, long long, long long *, DWORD);
__declspec(dllimport) BOOL GetFileSizeEx(HANDLE, long long *);
__declspec(dllimport) BOOL CloseHandle(HANDLE);
__declspec(dllimport) DWORD GetLastError(void);
#define INVALID ((HANDLE)-1)
#define READ 0x80000000UL
#define WRITE 0x40000000UL
#define OPEN_EXISTING 3
#define NORMAL 0x80
static const WCHAR name[] = {'c','a','f',0xe9,'.','b','i','n',0};
static const WCHAR missing[] = {'m','i','s','s','i','n','g',0};
static const WCHAR traversal[] = {'.','.','/','o','u','t','s','i','d','e','.','b','i','n',0};
static const WCHAR link[] = {'l','i','n','k',0};
static const WCHAR drive[] = {'Z',':','\\','m','I','x','E','d','D','i','R','\\','d','A','t','A','.','B','I','N',0};
static const WCHAR relative[] = {'m','I','x','E','d','D','i','R','/','d','A','t','A','.','B','I','N',0};
static const WCHAR drive_traversal[] = {'C',':','\\','M','i','x','e','d','D','i','r','\\','.','.','\\','c','a','f',0xe9,'.','b','i','n',0};
static const WCHAR drive_link[] = {'C',':','\\','d','i','r','l','i','n','k','\\','o','u','t','s','i','d','e','.','b','i','n',0};
static const WCHAR drive_relative[] = {'C',':','c','a','f',0xe9,'.','b','i','n',0};
static const WCHAR unc[] = {'\\','\\','s','e','r','v','e','r','\\','s','h','a','r','e',0};
#ifdef CASE_COLLISIONS
static const WCHAR collision_exact[] = {'C','o','l','l','i','s','i','o','n',0};
static const WCHAR collision_folded[] = {'C','O','L','L','I','S','I','O','N',0};
#endif
int marker = 7; int *volatile relocated_marker = &marker;
unsigned entry(void) {
    HANDLE h, bad;
    DWORD n;
    long long size, position;
    char buf[16];
    unsigned long long overlapped[4] = {0};
    if (*relocated_marker != 7) return 1;
    h = CreateFileW(name, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (h == INVALID || !h) return 2;
    size = -1;
    if (!GetFileSizeEx(h, &size) || size != 10) return 3;
    n = 99;
    if (!ReadFile(h, buf, 4, &n, 0) || n != 4 ||
        buf[0] != '0' || buf[1] != '1' || buf[2] != '2' || buf[3] != '3') return 4;
    position = -1;
    if (!SetFilePointerEx(h, -3, &position, 2) || position != 7) return 5;
    n = 99;
    if (!ReadFile(h, buf, sizeof(buf), &n, 0) || n != 3 ||
        buf[0] != '7' || buf[1] != '8' || buf[2] != '9') return 6;
    n = 99;
    if (!ReadFile(h, buf, sizeof(buf), &n, 0) || n != 0) return 7;
    if (!SetFilePointerEx(h, 1, &position, 0) || position != 1) return 8;
    if (!SetFilePointerEx(h, 2, &position, 1) || position != 3) return 9;
    n = 99;
    if (!ReadFile(h, buf, 3, &n, 0) || n != 3 ||
        buf[0] != '3' || buf[1] != '4' || buf[2] != '5') return 10;
    if (ReadFile(h, buf, 1, &n, overlapped) || GetLastError() != 50) return 11;
    if (!CloseHandle(h)) return 12;
    if (CloseHandle(h) || GetLastError() != 6) return 13;
    if (ReadFile(h, buf, 1, &n, 0) || GetLastError() != 6) return 14;
    if (GetFileSizeEx(h, &size) || GetLastError() != 6) return 15;
    if (SetFilePointerEx(h, 0, &position, 0) || GetLastError() != 6) return 16;
    if (CloseHandle((HANDLE)12345) || GetLastError() != 6) return 17;
    bad = CreateFileW(missing, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 2) return 18;
    bad = CreateFileW(traversal, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 19;
    bad = CreateFileW(name, WRITE, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 50) return 20;
    bad = CreateFileW(name, READ, 1, 0, 2, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 50) return 21;
    bad = CreateFileW(name, READ, 1, 0, OPEN_EXISTING, 0x40000000UL, 0);
    if (bad != INVALID || GetLastError() != 50) return 22;
    bad = CreateFileW(link, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 23;
    h = CreateFileW(drive, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (h == INVALID || !h) return 24;
    n = 0;
    if (!ReadFile(h, buf, 5, &n, 0) || n != 5 ||
        buf[0] != 'd' || buf[1] != 'r' || buf[2] != 'i' || buf[3] != 'v' || buf[4] != 'e') return 25;
    if (!CloseHandle(h)) return 26;
    h = CreateFileW(relative, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (h == INVALID || !h) return 27;
    n = 0;
    if (!ReadFile(h, buf, 5, &n, 0) || n != 5 ||
        buf[0] != 'd' || buf[1] != 'r' || buf[2] != 'i' || buf[3] != 'v' || buf[4] != 'e') return 28;
    if (!CloseHandle(h)) return 29;
    bad = CreateFileW(drive_traversal, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 30;
    bad = CreateFileW(drive_link, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 31;
    bad = CreateFileW(drive_relative, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 32;
    bad = CreateFileW(unc, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 5) return 33;
#ifdef CASE_COLLISIONS
    bad = CreateFileW(collision_exact, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 52) return 34;
    bad = CreateFileW(collision_folded, READ, 1, 0, OPEN_EXISTING, NORMAL, 0);
    if (bad != INVALID || GetLastError() != 52) return 35;
#endif
    return 42;
}
"#).unwrap();
    tool(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "kernel32.def", "-l", "kernel32.lib"]);
    let mut compile_args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "files.c", "-o", "files.obj"];
    if case_distinct { compile_args.push("-DCASE_COLLISIONS"); }
    tool(dir.path(), "x86_64-w64-mingw32-clang", &compile_args);
    tool(dir.path(), "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:files.exe", "files.obj", "kernel32.lib"]);
    let exe = dir.path().join("files.exe");
    let no_root = cli(&["native-fixture".as_ref(), exe.as_os_str()]);
    assert!(!no_root.status.success(), "file APIs must require --file-root");
    let executed = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--file-root".as_ref(), root.as_os_str(), "--execute".as_ref()]);
    assert!(executed.status.success(), "{}", String::from_utf8_lossy(&executed.stderr));
    assert!(String::from_utf8_lossy(&executed.stdout).contains("fixture entry returned 42"), "{}", String::from_utf8_lossy(&executed.stdout));
}

#[test]
fn native_fixture_writes_only_to_separate_save_root() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let save = dir.path().join("save");
    std::fs::create_dir(&game).unwrap();
    std::fs::create_dir(&save).unwrap();
    std::fs::write(game.join("game.bin"), b"game").unwrap();
    std::fs::write(dir.path().join("outside.bin"), b"outside").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside.bin"), save.join("link")).unwrap();
    std::os::unix::fs::symlink(dir.path(), save.join("dirlink")).unwrap();
    std::fs::hard_link(game.join("game.bin"), save.join("hardlink")).unwrap();
    std::fs::write(dir.path().join("kernel32.def"), "LIBRARY kernel32.dll\nEXPORTS\nCreateFileW\nWriteFile\nReadFile\nSetFilePointerEx\nFlushFileBuffers\nCloseHandle\nGetLastError\n").unwrap();
    std::fs::write(dir.path().join("save.c"), r#"
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;
__declspec(dllimport) HANDLE CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
__declspec(dllimport) BOOL WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) BOOL ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
__declspec(dllimport) BOOL SetFilePointerEx(HANDLE, long long, long long *, DWORD);
__declspec(dllimport) BOOL FlushFileBuffers(HANDLE);
__declspec(dllimport) BOOL CloseHandle(HANDLE);
__declspec(dllimport) DWORD GetLastError(void);
#define INVALID ((HANDLE)-1)
#define READ 0x80000000UL
#define WRITE 0x40000000UL
#define SHARE 3
#define NORMAL 0x80
#define CREATE_NEW 1
#define OPEN_EXISTING 3
#define OPEN_ALWAYS 4
static const WCHAR saved[] = {'T',':','\\','S','a','v','e','.','d','a','t',0};
static const WCHAR folded[] = {'t',':','\\','s','A','V','E','.','D','A','T',0};
static const WCHAR game[] = {'C',':','\\','g','a','m','e','.','b','i','n',0};
static const WCHAR relative[] = {'g','a','m','e','.','b','i','n',0};
static const WCHAR traversal[] = {'T',':','\\','.','.','\\','o','u','t','s','i','d','e','.','b','i','n',0};
static const WCHAR link[] = {'T',':','\\','l','i','n','k',0};
static const WCHAR dirlink[] = {'T',':','\\','d','i','r','l','i','n','k','\\','o','u','t','s','i','d','e','.','b','i','n',0};
static const WCHAR hardlink[] = {'T',':','\\','h','a','r','d','l','i','n','k',0};
int marker = 7; int *volatile relocated_marker = &marker;
unsigned entry(void) {
    HANDLE h;
    DWORD n;
    long long pos;
    char buf[8];
    unsigned long long overlapped[4] = {0};
    if (*relocated_marker != 7) return 1;
#ifdef CREATE
    h = CreateFileW(saved, READ | WRITE, SHARE, 0, CREATE_NEW, NORMAL, 0);
    if (h == INVALID || !h) return 2;
    n = 99;
    if (!WriteFile(h, "hello", 5, &n, 0) || n != 5) return 3;
    if (!FlushFileBuffers(h)) return 4;
    if (!SetFilePointerEx(h, 0, &pos, 0) || pos != 0) return 5;
    if (!ReadFile(h, buf, 5, &n, 0) || n != 5 || buf[0] != 'h' || buf[4] != 'o') return 6;
    if (WriteFile(h, "x", 1, &n, overlapped) || GetLastError() != 50) return 7;
    if (!CloseHandle(h)) return 8;
    if (WriteFile(h, "x", 1, &n, 0) || GetLastError() != 6) return 9;
    if (FlushFileBuffers(h) || GetLastError() != 6) return 10;
    if (CloseHandle(h) || GetLastError() != 6) return 11;
#else
    h = CreateFileW(folded, READ | WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0);
    if (h == INVALID || !h) return 12;
    if (!ReadFile(h, buf, 5, &n, 0) || n != 5 || buf[0] != 'h' || buf[4] != 'o') return 13;
    if (!CloseHandle(h)) return 14;
    h = CreateFileW(folded, WRITE, SHARE, 0, CREATE_NEW, NORMAL, 0);
    if (h != INVALID || GetLastError() != 80) return 15;
    h = CreateFileW(folded, READ | WRITE, SHARE, 0, OPEN_ALWAYS, NORMAL, 0);
    if (h == INVALID || !h) return 16;
    if (!CloseHandle(h)) return 17;
    if (CreateFileW(game, WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 18;
    if (CreateFileW(relative, WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 19;
    if (CreateFileW(traversal, WRITE, SHARE, 0, CREATE_NEW, NORMAL, 0) != INVALID) return 20;
    if (CreateFileW(link, WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 21;
    if (CreateFileW(dirlink, WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 22;
    if (CreateFileW(hardlink, WRITE, SHARE, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 23;
    if (CreateFileW(saved, WRITE, 1, 0, OPEN_EXISTING, NORMAL, 0) != INVALID) return 24;
    if (CreateFileW(saved, WRITE, SHARE, 0, OPEN_EXISTING, 0x40000000UL, 0) != INVALID) return 25;
    if (CreateFileW(saved, WRITE, SHARE, 0, 2, NORMAL, 0) != INVALID) return 26;
    if (CreateFileW(saved, WRITE, SHARE, 0, 5, NORMAL, 0) != INVALID) return 27;
#endif
    return 42;
}
"#).unwrap();
    tool(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "kernel32.def", "-l", "kernel32.lib"]);
    for (obj, exe, define) in [("create.obj", "create.exe", "-DCREATE"), ("verify.obj", "verify.exe", "")] {
        let mut args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "save.c", "-o", obj];
        if !define.is_empty() { args.push(define); }
        tool(dir.path(), "x86_64-w64-mingw32-clang", &args);
        tool(dir.path(), "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", &format!("/out:{exe}"), obj, "kernel32.lib"]);
    }
    let create = dir.path().join("create.exe");
    let verify = dir.path().join("verify.exe");
    let run = |exe: &std::path::Path| cli(&["native-fixture".as_ref(), exe.as_os_str(), "--file-root".as_ref(), game.as_os_str(), "--save-root".as_ref(), save.as_os_str(), "--execute".as_ref()]);
    for exe in [&create, &verify] {
        let output = run(exe);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "{}", String::from_utf8_lossy(&output.stdout));
    }
    assert_eq!(std::fs::read(save.join("Save.dat")).unwrap(), b"hello");
    assert_eq!(std::fs::read(game.join("game.bin")).unwrap(), b"game");
    assert_eq!(std::fs::read(dir.path().join("outside.bin")).unwrap(), b"outside");
    let race_save = dir.path().join("race-save");
    std::fs::create_dir(&race_save).unwrap();
    let mut children = Vec::new();
    for _ in 0..4 {
        children.push(Command::new(env!("CARGO_BIN_EXE_victory"))
            .args(["native-fixture".as_ref(), create.as_os_str(), "--file-root".as_ref(), game.as_os_str(), "--save-root".as_ref(), race_save.as_os_str(), "--execute".as_ref()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn());
    }
    let mut winners = 0;
    for child in children {
        let output = child.unwrap().wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("fixture entry returned 42") { winners += 1; }
        else { assert!(stdout.contains("fixture entry returned 2"), "{stdout}"); }
    }
    assert_eq!(winners, 1);
    assert_eq!(std::fs::read(race_save.join("Save.dat")).unwrap(), b"hello");
    for (root, save_root) in [(&game, &game), (&game, &dir.path().to_path_buf()), (&dir.path().to_path_buf(), &save)] {
        let output = cli(&["native-fixture".as_ref(), verify.as_os_str(), "--file-root".as_ref(), root.as_os_str(), "--save-root".as_ref(), save_root.as_os_str(), "--execute".as_ref()]);
        assert!(!output.status.success(), "overlapping roots must be rejected");
    }
    let no_game = cli(&["native-fixture".as_ref(), verify.as_os_str(), "--save-root".as_ref(), save.as_os_str(), "--execute".as_ref()]);
    assert!(!no_game.status.success(), "--save-root must require --file-root");
}

#[test]
fn native_fixture_mutates_only_scoped_save_paths() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let save = dir.path().join("save");
    std::fs::create_dir(&game).unwrap();
    std::fs::create_dir(&save).unwrap();
    std::fs::write(game.join("game.bin"), b"game").unwrap();
    std::fs::write(dir.path().join("outside.bin"), b"outside").unwrap();
    std::fs::write(save.join("source.bin"), b"source").unwrap();
    std::fs::write(save.join("destination.bin"), b"destination").unwrap();
    std::fs::hard_link(game.join("game.bin"), save.join("hardlink")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside.bin"), save.join("link")).unwrap();
    std::os::unix::fs::symlink(dir.path(), save.join("dirlink")).unwrap();
    std::fs::write(dir.path().join("kernel32.def"), "LIBRARY kernel32.dll\nEXPORTS\nCreateDirectoryW\nMoveFileExW\nDeleteFileW\n").unwrap();
    std::fs::write(dir.path().join("save_ops.c"), r#"
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;
__declspec(dllimport) BOOL CreateDirectoryW(const WCHAR *, void *);
__declspec(dllimport) BOOL MoveFileExW(const WCHAR *, const WCHAR *, DWORD);
__declspec(dllimport) BOOL DeleteFileW(const WCHAR *);
#define W(s) ((const WCHAR *)L##s)
int marker = 7; int *volatile relocated_marker = &marker;
unsigned entry(void) {
    if (*relocated_marker != 7) return 1;
#ifdef CREATE
    if (!CreateDirectoryW(W("T:\\Folder"), 0)) return 2;
    if (CreateDirectoryW(W("t:\\fOLDER"), 0)) return 3;
    if (!MoveFileExW(W("T:\\source.bin"), W("t:\\folder\\renamed.bin"), 0)) return 4;
#elif defined(REJECT)
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\destination.bin"), 0)) return 5;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\other.bin"), 2)) return 6;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\other.bin"), 0x80000000UL)) return 7;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("C:\\game.bin"), 1)) return 8;
    if (MoveFileExW(W("C:\\game.bin"), W("T:\\other.bin"), 0)) return 9;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("relative.bin"), 0)) return 10;
    if (MoveFileExW(W("relative.bin"), W("T:\\other.bin"), 0)) return 11;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\..\\outside.bin"), 1)) return 12;
    if (MoveFileExW(W("T:\\..\\outside.bin"), W("T:\\other.bin"), 0)) return 13;
    if (MoveFileExW(W("T:\\link"), W("T:\\other.bin"), 0)) return 14;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\link"), 1)) return 15;
    if (MoveFileExW(W("T:\\hardlink"), W("T:\\other.bin"), 0)) return 16;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\hardlink"), 1)) return 17;
    if (MoveFileExW(W("T:\\Folder\\renamed.bin"), W("T:\\dirlink\\outside.bin"), 1)) return 18;
    if (DeleteFileW(W("T:\\link"))) return 19;
    if (DeleteFileW(W("T:\\hardlink"))) return 20;
    if (DeleteFileW(W("T:\\dirlink\\outside.bin"))) return 21;
    if (DeleteFileW(W("T:\\..\\outside.bin"))) return 22;
    if (DeleteFileW(W("C:\\game.bin"))) return 23;
    if (DeleteFileW(W("game.bin"))) return 24;
    if (CreateDirectoryW(W("C:\\newdir"), 0)) return 25;
    if (CreateDirectoryW(W("newdir"), 0)) return 26;
    if (CreateDirectoryW(W("T:\\..\\newdir"), 0)) return 27;
    if (CreateDirectoryW(W("T:\\dirlink\\newdir"), 0)) return 28;
    if (CreateDirectoryW(W("T:\\otherdir"), (void *)1)) return 29;
#elif defined(REPLACE)
    if (!MoveFileExW(W("t:\\folder\\RENAMED.bin"), W("T:\\destination.bin"), 1)) return 30;
#else
    if (!DeleteFileW(W("t:\\DESTINATION.bin"))) return 31;
    if (DeleteFileW(W("T:\\destination.bin"))) return 32;
#endif
    return 42;
}
"#).unwrap();
    tool(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "kernel32.def", "-l", "kernel32.lib"]);
    for (name, define) in [("create", "-DCREATE"), ("reject", "-DREJECT"), ("replace", "-DREPLACE"), ("delete", "")] {
        let obj = format!("{name}.obj");
        let exe = format!("{name}.exe");
        let mut args = vec!["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "save_ops.c", "-o", &obj];
        if !define.is_empty() { args.push(define); }
        tool(dir.path(), "x86_64-w64-mingw32-clang", &args);
        tool(dir.path(), "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", &format!("/out:{exe}"), &obj, "kernel32.lib"]);
    }
    let run = |name: &str| {
        let exe = dir.path().join(format!("{name}.exe"));
        let output = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--file-root".as_ref(), game.as_os_str(), "--save-root".as_ref(), save.as_os_str(), "--execute".as_ref()]);
        assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "{name}: {}", String::from_utf8_lossy(&output.stdout));
    };
    run("create");
    assert!(!save.join("source.bin").exists());
    assert_eq!(std::fs::read(save.join("Folder/renamed.bin")).unwrap(), b"source");
    run("reject");
    assert_eq!(std::fs::read(save.join("Folder/renamed.bin")).unwrap(), b"source");
    assert_eq!(std::fs::read(save.join("destination.bin")).unwrap(), b"destination");
    assert!(!save.join("other.bin").exists());
    assert!(!save.join("otherdir").exists());
    assert!(!dir.path().join("newdir").exists());
    assert!(!game.join("newdir").exists());
    assert_eq!(std::fs::read(game.join("game.bin")).unwrap(), b"game");
    assert_eq!(std::fs::read(dir.path().join("outside.bin")).unwrap(), b"outside");
    run("replace");
    assert!(!save.join("Folder/renamed.bin").exists());
    assert_eq!(std::fs::read(save.join("destination.bin")).unwrap(), b"source");
    run("delete");
    assert!(!save.join("destination.bin").exists());
    assert_eq!(std::fs::read(game.join("game.bin")).unwrap(), b"game");
    assert_eq!(std::fs::read(dir.path().join("outside.bin")).unwrap(), b"outside");
}

#[test]
fn native_fixture_discovers_scoped_files() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let save = dir.path().join("save");
    std::fs::create_dir(&game).unwrap();
    std::fs::create_dir(&save).unwrap();
    std::fs::create_dir(game.join("Folder")).unwrap();
    std::fs::write(game.join("Folder").join("child"), b"child").unwrap();
    std::fs::write(game.join("Alpha.bin"), b"12345").unwrap();
    std::fs::write(game.join("café😀.txt"), b"unicode").unwrap();
    std::fs::write(save.join("Save.dat"), b"save data").unwrap();
    std::fs::create_dir(save.join("Saves")).unwrap();
    std::fs::write(dir.path().join("outside"), b"outside").unwrap();
    for root in [&game, &save] {
        std::os::unix::fs::symlink(dir.path().join("outside"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("dirlink")).unwrap();
    }
    let modified = std::fs::metadata(game.join("Alpha.bin")).unwrap().modified().unwrap()
        .duration_since(std::time::UNIX_EPOCH).unwrap();
    let filetime = 116_444_736_000_000_000u64 + modified.as_secs() * 10_000_000
        + u64::from(modified.subsec_nanos()) / 100;
    std::fs::write(dir.path().join("kernel32.def"), "LIBRARY kernel32.dll\nEXPORTS\nGetFileAttributesExW\nFindFirstFileW\nFindNextFileW\nFindClose\nCloseHandle\nGetLastError\n").unwrap();
    let source = r#"
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef struct { DWORD low, high; } FILETIME;
typedef struct {
    DWORD attributes;
    FILETIME creation, access, write;
    DWORD size_high, size_low;
} ATTR_DATA;
typedef struct {
    DWORD attributes;
    FILETIME creation, access, write;
    DWORD size_high, size_low, reserved0, reserved1;
    WCHAR name[260], alternate[14];
} FIND_DATA;
typedef char attr_size_check[sizeof(ATTR_DATA) == 36 ? 1 : -1];
typedef char find_size_check[sizeof(FIND_DATA) == 592 ? 1 : -1];
__declspec(dllimport) BOOL GetFileAttributesExW(const WCHAR *, int, void *);
__declspec(dllimport) HANDLE FindFirstFileW(const WCHAR *, FIND_DATA *);
__declspec(dllimport) BOOL FindNextFileW(HANDLE, FIND_DATA *);
__declspec(dllimport) BOOL FindClose(HANDLE);
__declspec(dllimport) BOOL CloseHandle(HANDLE);
__declspec(dllimport) DWORD GetLastError(void);
#define W(s) ((const WCHAR *)L##s)
#define INVALID ((HANDLE)-1)
#define DIR 0x10
int marker = 7; int *volatile relocated_marker = &marker;
static int equal(const WCHAR *a, const WCHAR *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}
static unsigned long long ticks(FILETIME t) {
    return ((unsigned long long)t.high << 32) | t.low;
}
static int game_record(FIND_DATA *d) {
    if (equal(d->name, W("Alpha.bin")))
        return !(d->attributes & DIR) && !d->size_high && d->size_low == 5 &&
               ticks(d->write) == @FILETIME@ULL && ticks(d->creation) && ticks(d->access) ? 1 : -1;
    if (equal(d->name, W("caf\u00e9\U0001f600.txt")))
        return !(d->attributes & DIR) && !d->size_high && d->size_low == 7 ? 2 : -1;
    if (equal(d->name, W("Folder")))
        return (d->attributes & DIR) && !d->size_high && !d->size_low ? 4 : -1;
    return -1;
}
static int save_record(FIND_DATA *d) {
    if (equal(d->name, W("Save.dat")))
        return !(d->attributes & DIR) && !d->size_high && d->size_low == 9 ? 1 : -1;
    if (equal(d->name, W("Saves")))
        return (d->attributes & DIR) && !d->size_high && !d->size_low ? 2 : -1;
    return -1;
}
static int enumerate(const WCHAR *pattern, int save, int expected) {
    FIND_DATA d;
    HANDLE h = FindFirstFileW(pattern, &d);
    int seen = 0, bit;
    if (h == INVALID || !h) return 0;
    do {
        bit = save ? save_record(&d) : game_record(&d);
        if (bit < 0 || (seen & bit)) { FindClose(h); return 0; }
        seen |= bit;
    } while (FindNextFileW(h, &d));
    if (GetLastError() != 18 || seen != expected) { FindClose(h); return 0; }
    if (FindNextFileW(h, &d) || GetLastError() != 18) { FindClose(h); return 0; }
    if (!FindClose(h)) return 0;
    if (FindClose(h) || GetLastError() != 6) return 0;
    if (FindNextFileW(h, &d) || GetLastError() != 6) return 0;
    return 1;
}
unsigned entry(void) {
    ATTR_DATA a;
    FIND_DATA d;
    HANDLE h, other;
    if (*relocated_marker != 7) return 1;
    if (!GetFileAttributesExW(W("c:\\aLPHA.BIN"), 0, &a) ||
        (a.attributes & DIR) || a.size_high || a.size_low != 5 ||
        ticks(a.write) != @FILETIME@ULL || !ticks(a.creation) || !ticks(a.access)) return 2;
    if (!GetFileAttributesExW(W("C:\\folder"), 0, &a) ||
        !(a.attributes & DIR) || a.size_high || a.size_low) return 3;
    if (!GetFileAttributesExW(W("T:\\sAVE.DAT"), 0, &a) ||
        (a.attributes & DIR) || a.size_high || a.size_low != 9) return 4;
    if (!GetFileAttributesExW(W("T:\\saves"), 0, &a) || !(a.attributes & DIR)) return 5;
    if (!GetFileAttributesExW(W("caf\u00e9\U0001f600.txt"), 0, &a) || a.size_low != 7) return 6;
    if (GetFileAttributesExW(W("caf\u00c9\U0001f600.txt"), 0, &a) || GetLastError() != 2) return 7;
    if (GetFileAttributesExW(W("Alpha.bin"), 1, &a) || GetLastError() != 50) return 8;
    if (!enumerate(W("C:\\*"), 0, 7)) return 9;
    if (!enumerate(W("C:\\*.*"), 0, 7)) return 10;
    if (!enumerate(W("c:\\aL*"), 0, 1)) return 11;
    if (!enumerate(W("C:\\ALPHA.BIN"), 0, 1)) return 12;
    if (!enumerate(W("T:\\*"), 1, 3)) return 13;
    if (!enumerate(W("T:\\*.*"), 1, 3)) return 14;
    if (!enumerate(W("t:\\save.d*"), 1, 1)) return 15;
    h = FindFirstFileW(W("C:\\no-match*"), &d);
    if (h != INVALID || GetLastError() != 2) return 16;
    h = FindFirstFileW(W("C:\\a?pha.bin"), &d);
    if (h != INVALID || GetLastError() != 50) return 17;
    h = FindFirstFileW(W("C:\\*pha*"), &d);
    if (h != INVALID || GetLastError() != 50) return 18;
    h = FindFirstFileW(W("C:\\Fol*\\child"), &d);
    if (h != INVALID || GetLastError() != 50) return 19;
    h = FindFirstFileW(W("T:\\sa?e*"), &d);
    if (h != INVALID || GetLastError() != 50) return 20;
    if (GetFileAttributesExW(W("C:\\link"), 0, &a) || GetLastError() != 5) return 21;
    h = FindFirstFileW(W("C:\\link"), &d);
    if (h != INVALID || GetLastError() != 5) return 22;
    h = FindFirstFileW(W("T:\\link"), &d);
    if (h != INVALID || GetLastError() != 5) return 23;
    if (GetFileAttributesExW(W("T:\\dirlink\\outside"), 0, &a) || GetLastError() != 5) return 24;
    h = FindFirstFileW(W("C:\\dirlink\\*"), &d);
    if (h != INVALID || GetLastError() != 5) return 25;
    if (GetFileAttributesExW(W("C:\\..\\outside"), 0, &a) || GetLastError() != 5) return 26;
    h = FindFirstFileW(W("T:\\..\\*"), &d);
    if (h != INVALID || GetLastError() != 5) return 27;
    h = FindFirstFileW(W("C:\\Alpha.bin"), &d);
    if (h == INVALID || !h) return 28;
    if (CloseHandle(h) || GetLastError() != 6) return 29;
    if (!FindClose(h)) return 30;
    if (FindClose((HANDLE)12345) || GetLastError() != 6) return 31;
    h = FindFirstFileW(W("C:\\*"), &d);
    other = FindFirstFileW(W("T:\\*"), &d);
    if (h == INVALID || !h || other == INVALID || !other) return 32;
    if (!FindClose(h)) return 33;
    if (!FindNextFileW(other, &d)) return 34;
    if (!FindClose(other)) return 35;
    return 42;
}
"#.replace("@FILETIME@", &filetime.to_string());
    std::fs::write(dir.path().join("discover.c"), source).unwrap();
    tool(dir.path(), "llvm-dlltool", &["-m", "i386:x86-64", "-d", "kernel32.def", "-l", "kernel32.lib"]);
    tool(dir.path(), "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "discover.c", "-o", "discover.obj"]);
    tool(dir.path(), "lld-link", &["/entry:entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:discover.exe", "discover.obj", "kernel32.lib"]);
    let exe = dir.path().join("discover.exe");
    let output = cli(&["native-fixture".as_ref(), exe.as_os_str(), "--file-root".as_ref(), game.as_os_str(), "--save-root".as_ref(), save.as_os_str(), "--execute".as_ref()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "{}", String::from_utf8_lossy(&output.stdout));
}
