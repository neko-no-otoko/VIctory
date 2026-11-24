#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use victory_loader::macos::Image;
use victory_loader::{ImportResolver, Provenance, Resolution};
use victory_pe::ImportSymbol;

static CALLS: AtomicUsize = AtomicUsize::new(0);

extern "win64" fn host_adjust(value: i32) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
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
    run("lld-link", &["/dll", "/noentry", "/nodefaultlib", "/dynamicbase", "/out:fixture.dll", "fixture.obj", "host.lib"], dir.path());
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
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
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
    let error = match Image::load(&with_entry, &Resolver) { Ok(_) => panic!("entry point accepted"), Err(e) => e };
    assert!(error.to_string().contains("entry points"), "{error}");
}
