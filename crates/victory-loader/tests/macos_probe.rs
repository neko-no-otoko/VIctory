#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::path::Path;
use std::process::Command;

use victory_loader::macos::Image;
use victory_loader::{ImportResolver, Provenance, Resolution};
use victory_pe::ImportSymbol;

extern "win64" fn host_adjust(value: i32) -> i32 {
    value + 2
}

struct Resolver;
impl ImportResolver for Resolver {
    fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
        if dll.eq_ignore_ascii_case("victory-synthetic.dll")
            && matches!(symbol, ImportSymbol::Name { name, .. } if name == "host_adjust")
        {
            Resolution::Found { address: host_adjust as *const () as usize, provenance: Provenance::Hle }
        } else {
            Resolution::Missing
        }
    }
}

struct Missing;
impl ImportResolver for Missing {
    fn resolve(&self, _: &str, _: &ImportSymbol) -> Resolution { Resolution::Missing }
}

fn run(tool: &str, args: &[&str], dir: &Path) {
    let output = Command::new(tool).args(args).current_dir(dir).output().unwrap_or_else(|e| panic!("{tool}: {e}"));
    assert!(output.status.success(), "{tool}: {}", String::from_utf8_lossy(&output.stderr));
}

fn fixture() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("tests/fixtures/macos_pe_fixture.c");
    std::fs::write(dir.path().join("host.def"), "LIBRARY victory-synthetic.dll\nEXPORTS\nhost_adjust\n").unwrap();
    run("llvm-dlltool", &["-m", "i386:x86-64", "-d", "host.def", "-l", "host.lib"], dir.path());
    run("x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", source.to_str().unwrap(), "-o", "fixture.obj"], dir.path());
    run("lld-link", &["/dll", "/entry:DllMain", "/nodefaultlib", "/dynamicbase", "/out:fixture.dll", "fixture.obj", "host.lib"], dir.path());
    std::fs::read(dir.path().join("fixture.dll")).unwrap()
}

fn assert_write_faults(address: *mut u8) {
    // SAFETY: the child performs only a volatile write and _exit after fork;
    // it cannot affect the parent's private mapping.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        // SAFETY: this intentionally tests whether the mapped page is writable.
        unsafe { address.write_volatile(0); libc::_exit(0) };
    }
    let mut status = 0;
    // SAFETY: status is valid and pid names our child.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFSIGNALED(status), "write unexpectedly succeeded: {status}");
    assert!(matches!(libc::WTERMSIG(status), libc::SIGSEGV | libc::SIGBUS), "unexpected signal: {status}");
}

#[test]
fn maps_relocates_binds_and_calls_real_pe() {
    let bytes = fixture();
    let parsed = victory_pe::ImageInfo::parse(&bytes).unwrap();
    assert!(parsed.has_relocations, "fixture must require relocations");
    assert_eq!(parsed.import_count(), 1, "fixture must import host_adjust");
    let image = Image::load(&bytes, &Resolver).unwrap();
    let value = image.export("fixture_value").unwrap().as_ptr() as *const i32;
    let pointer = image.export("fixture_pointer").unwrap().as_ptr() as *const *const i32;
    // SAFETY: the fixture declares these exports with the types used here and
    // the image is alive; relocations must have fixed the pointer to its value.
    unsafe { assert_eq!(*pointer, value); assert_eq!(*value, 40); }
    let entry = image.export("fixture_call").unwrap();
    // SAFETY: the self-authored fixture's export has this Windows x64 signature.
    let call: unsafe extern "win64" fn() -> i32 = unsafe { std::mem::transmute(entry.as_ptr()) };
    // SAFETY: imports are bound and the fixture has no initialization requirements.
    assert_eq!(unsafe { call() }, 42);
    assert_write_faults(value.cast_mut().cast());
    assert_write_faults(entry.as_ptr().cast());
    assert!(image.export("not_an_export").is_none());
}

#[test]
fn rejects_unresolved_imports_and_bad_images() {
    let bytes = fixture();
    let error = match Image::load(&bytes, &Missing) { Ok(_) => panic!("missing import accepted"), Err(e) => e };
    assert!(error.to_string().contains("unresolved import"), "{error}");
    assert!(Image::load(b"not a PE", &Resolver).is_err());
    let mut with_entry = bytes;
    let opt = victory_pe::layout::optional_header_offset(&with_entry).unwrap();
    with_entry[opt + 16..opt + 20].copy_from_slice(&1u32.to_le_bytes());
    let error = match Image::load(&with_entry, &Resolver) { Ok(_) => panic!("invalid entry point accepted"), Err(e) => e };
    assert!(error.to_string().contains("entry point"), "{error}");
}

#[test]
fn invokes_attach_and_detach_and_handles_failure() {
    let bytes = fixture();
    let parsed = victory_pe::ImageInfo::parse(&bytes).unwrap();
    let value_rva = parsed.exports.iter().find(|e| e.name.as_deref() == Some("fixture_value")).unwrap().rva.unwrap() as usize;
    let mut image = Image::load(&bytes, &Resolver).unwrap();
    let exported = |name| image.export(name).unwrap().as_ptr();
    let attaches = exported("fixture_attaches").cast::<i32>();
    let detaches = exported("fixture_detaches").cast::<i32>();
    let fail = exported("fixture_fail_attach").cast::<i32>();
    let module = exported("fixture_module").cast::<*mut std::ffi::c_void>();
    let reserved = exported("fixture_reserved").cast::<*mut std::ffi::c_void>();
    let expected_base = (exported("fixture_value") as usize - value_rva) as *mut std::ffi::c_void;
    // SAFETY: the self-authored fixture exports these typed, writable globals.
    unsafe {
        assert_eq!(*attaches, 0);
        assert!(image.detach().is_err());
        *fail = 1;
        assert!(image.attach().is_err());
        assert_eq!(*attaches, 1);
        assert_eq!(*detaches, 0);
        assert!(image.detach().is_err());
        *fail = 0;
        image.attach().unwrap();
        assert_eq!(*attaches, 2);
        assert_eq!(*module, expected_base);
        assert!((*reserved).is_null());
        assert!(image.attach().is_err());
        image.detach().unwrap();
        assert_eq!(*detaches, 1);
        assert_eq!(*module, expected_base);
        assert!((*reserved).is_null());
        assert!(image.detach().is_err());
    }
}

#[test]
fn entrypoint_free_image_has_explicit_lifecycle() {
    let mut bytes = fixture();
    let opt = victory_pe::layout::optional_header_offset(&bytes).unwrap();
    bytes[opt + 16..opt + 20].copy_from_slice(&0u32.to_le_bytes());
    let mut image = Image::load(&bytes, &Resolver).unwrap();
    // SAFETY: this fixture has no entry point, and no code runs.
    unsafe {
        image.attach().unwrap();
        assert!(image.attach().is_err());
        image.detach().unwrap();
        assert!(image.detach().is_err());
    }
}

fn consumer(fail: bool) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("provider.def"), "LIBRARY provider.dll\nEXPORTS\nfixture_is_live\nfixture_note_detach\nfixture_call\n").unwrap();
    std::fs::write(dir.path().join("consumer.c"), format!(r#"
__declspec(dllimport) int fixture_is_live(void);
__declspec(dllimport) void fixture_note_detach(void);
__declspec(dllimport) int fixture_call(void);
int consumer_value;
__declspec(dllexport) int *consumer_pointer = &consumer_value;
__declspec(dllexport) int consumer_call(void) {{ return fixture_call(); }}
int DllMain(void *m, unsigned long reason, void *r) {{
    (void)m; (void)r;
    if (reason == 1) return fixture_is_live() && {};
    if (reason == 0) fixture_note_detach();
    return 1;
}}
"#, if fail { 0 } else { 1 })).unwrap();
    run("llvm-dlltool", &["-m", "i386:x86-64", "-d", "provider.def", "-l", "provider.lib"], dir.path());
    run("x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "consumer.c", "-o", "consumer.obj"], dir.path());
    run("lld-link", &["/dll", "/entry:DllMain", "/nodefaultlib", "/dynamicbase", "/out:consumer.dll", "consumer.obj", "provider.lib"], dir.path());
    std::fs::read(dir.path().join("consumer.dll")).unwrap()
}

#[test]
fn dependency_order_and_failure_rollback() {
    use victory_loader::macos::ModuleSet;
    let provider = fixture();
    for fail in [false, true] {
        let consumer = consumer(fail);
        let mut set = ModuleSet::load(&[("consumer.dll", &consumer), ("provider.dll", &provider)], &Resolver).unwrap();
        let detaches = set.export("provider.dll", "fixture_detaches").unwrap().as_ptr().cast::<i32>();
        let saw = set.export("provider.dll", "fixture_detach_saw_consumer").unwrap().as_ptr().cast::<i32>();
        // SAFETY: synthetic modules and their Windows-ABI imports are known.
        unsafe {
            if fail {
                assert!(set.attach().is_err());
                assert_eq!(*detaches, 1);
                assert_eq!(*saw, 0);
            } else {
                set.attach().unwrap();
                let entry = set.export("consumer.dll", "consumer_call").unwrap();
                let call: unsafe extern "win64" fn() -> i32 = std::mem::transmute(entry.as_ptr());
                assert_eq!(call(), 42);
                set.detach().unwrap();
                assert_eq!(*detaches, 1);
                assert_eq!(*saw, 1);
            }
        }
    }
}

#[test]
fn rejects_cycles_and_forwarded_exports() {
    use victory_loader::macos::ModuleSet;
    let cyclic = fixture();
    let error = match ModuleSet::load(&[("victory-synthetic.dll", &cyclic)], &Resolver) {
        Ok(_) => panic!("cycle accepted"), Err(e) => e,
    };
    assert!(error.to_string().contains("cyclic DLL dependency"), "{error}");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("forward.def"), "LIBRARY provider.dll\nEXPORTS\nfixture_is_live=other.foo\nfixture_note_detach\nfixture_call\n").unwrap();
    std::fs::write(dir.path().join("forward.c"), "int value; __declspec(dllexport) int *pointer = &value; int fixture_call(void) { return 42; } void fixture_note_detach(void) {}").unwrap();
    run("x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "forward.c", "-o", "forward.obj"], dir.path());
    run("lld-link", &["/dll", "/noentry", "/nodefaultlib", "/dynamicbase", "/def:forward.def", "/out:forward.dll", "forward.obj"], dir.path());
    let forward = std::fs::read(dir.path().join("forward.dll")).unwrap();
    let consumer = consumer(false);
    let error = match ModuleSet::load(&[("consumer.dll", &consumer), ("provider.dll", &forward)], &Resolver) {
        Ok(_) => panic!("forwarded export accepted"), Err(e) => e,
    };
    assert!(error.to_string().contains("forwarded export"), "{error}");
}

#[test]
fn maps_and_starts_synthetic_executable_with_dependency() {
    use victory_loader::macos::Executable;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("provider.def"), "LIBRARY provider.dll\nEXPORTS\nfixture_is_live\nfixture_call\n").unwrap();
    std::fs::write(dir.path().join("program.c"), r#"
__declspec(dllimport) int fixture_is_live(void);
__declspec(dllimport) int fixture_call(void);
int value = 7;
int *volatile relocated_pointer = &value;
unsigned synthetic_entry(void) {
    return fixture_is_live() && *relocated_pointer == 7 ? fixture_call() : 0;
}
"#).unwrap();
    run("llvm-dlltool", &["-m", "i386:x86-64", "-d", "provider.def", "-l", "provider.lib"], dir.path());
    run("x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "program.c", "-o", "program.obj"], dir.path());
    run("lld-link", &["/entry:synthetic_entry", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:program.exe", "program.obj", "provider.lib"], dir.path());
    let bytes = std::fs::read(dir.path().join("program.exe")).unwrap();
    let info = victory_pe::ImageInfo::parse(&bytes).unwrap();
    assert!(!info.is_dll() && info.entry_rva != 0 && info.has_relocations);
    let provider = fixture();
    assert!(Executable::load(&provider, &[], &Resolver).is_err());
    let mut missing_entry = bytes.clone();
    let opt = victory_pe::layout::optional_header_offset(&missing_entry).unwrap();
    missing_entry[opt + 16..opt + 20].copy_from_slice(&0u32.to_le_bytes());
    assert!(Executable::load(&missing_entry, &[("provider.dll", &provider)], &Resolver).is_err());
    let mut exe = Executable::load(&bytes, &[("provider.dll", &provider)], &Resolver).unwrap();
    // SAFETY: the self-authored EXE entry returns normally with this exact
    // Windows ABI; its sole dependency and imports are trusted fixtures.
    unsafe {
        assert_eq!(exe.start().unwrap(), 42);
        assert!(exe.start().is_err());
    }
}
