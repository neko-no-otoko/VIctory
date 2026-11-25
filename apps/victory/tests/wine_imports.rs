//! Offline import diagnostics using self-authored AMD64 PE images.
//!
//! Run explicitly with `LLVM_MINGW=/path/to/llvm-mingw cargo test -p victory
//! --test wine_imports -- --ignored`. The toolchain must provide
//! bin/x86_64-w64-mingw32-clang and bin/llvm-dlltool. No Wine, Metal service,
//! system DLLs, or externally built VIctory providers are needed.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

struct Fixture {
    temp: tempfile::TempDir,
    bin: PathBuf,
    providers: PathBuf,
    guest: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW to an LLVM-MinGW installation"));
        let temp = tempfile::tempdir().unwrap();
        let providers = temp.path().join("providers");
        let guest = temp.path().join("guest");
        fs::create_dir(&providers).unwrap();
        fs::create_dir(&guest).unwrap();
        let f = Self { temp, bin: root.join("bin"), providers, guest };
        let empty = "int DllMain(void) { return 1; }\n";
        for name in ["d3d12_xs.dll", "xg_xs.dll", "xgameruntime.dll", "xmem.dll", "acphal.dll", "xapu.dll"] {
            f.compile_dll(name, empty, &[], &f.providers);
        }
        let mut backend = String::from(empty);
        for name in ["XMemAlloc", "XMemFree", "XMemAllocDefault", "XMemFreeDefault", "XMemIsToolsMemAvailable", "XMemAllocatePhysicalPages", "XMemFreePhysicalPages", "XMemMapPhysicalPages", "XMemMapPhysicalPagesScatter", "XMemVirtualAlloc", "XMemVirtualQuery", "XMemGetWorkingSetStatistics", "XMemTransferMemory", "VIctoryXMemMapAlias"] {
            backend.push_str(&format!("__declspec(dllexport) void {name}(void) {{}}\n"));
        }
        f.compile_dll("victory_xmem_backend.dll", &backend, &[], &f.providers);
        f
    }

    fn checked(&self, cmd: &mut Command) {
        let out = cmd.output().unwrap_or_else(|e| panic!("could not run {cmd:?}: {e}"));
        assert!(out.status.success(), "{cmd:?} failed: {}\n{}\n{}", out.status,
            String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }

    fn import_lib(&self, id: &str, dll: &str, exports: &str) -> PathBuf {
        let def = self.temp.path().join(format!("{id}.def"));
        let lib = self.temp.path().join(format!("{id}.lib"));
        fs::write(&def, format!("LIBRARY {dll}\nEXPORTS\n{exports}\n")).unwrap();
        self.checked(Command::new(self.bin.join("llvm-dlltool"))
            .args(["-m", "i386:x86-64", "-d"]).arg(def).arg("-l").arg(&lib));
        lib
    }

    fn compile_dll(&self, name: &str, source: &str, libs: &[&Path], dir: &Path) -> PathBuf {
        let source_path = self.temp.path().join(format!("{name}.c"));
        fs::write(&source_path, source).unwrap();
        let output = dir.join(name);
        let mut cmd = Command::new(self.bin.join("x86_64-w64-mingw32-clang"));
        cmd.args(["-shared", "-nostdlib", "-Wl,--entry,DllMain", "-Wl,--file-alignment,4096"]).arg(source_path);
        for lib in libs { cmd.arg(lib); }
        cmd.arg("-o").arg(&output);
        self.checked(&mut cmd);
        output
    }

    fn compile_exe(&self, source: &str, libs: &[&Path]) -> PathBuf {
        let source_path = self.temp.path().join("title.c");
        let output = self.guest.join("title.exe");
        fs::write(&source_path, source).unwrap();
        let mut cmd = Command::new(self.bin.join("x86_64-w64-mingw32-clang"));
        cmd.args(["-nostdlib", "-Dmain=fixture_entry", "-Wl,--entry,fixture_entry"]).arg(source_path);
        for lib in libs { cmd.arg(lib); }
        cmd.arg("-o").arg(&output);
        self.checked(&mut cmd);
        output
    }

    fn inspect(&self, exe: &Path, guests: &[&str]) -> (Output, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_victory"));
        cmd.arg("wine-imports").arg(exe).arg("--dll-dir").arg(&self.providers);
        for guest in guests { cmd.arg("--guest-dll").arg(guest); }
        // Deliberately prevent accidental interaction with Wine configuration.
        cmd.env_remove("WINEPREFIX").env_remove("WINEPATH");
        let out = cmd.output().expect("run offline import diagnostic");
        let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        (out, text)
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn align(n: u32, alignment: u32) -> u32 { (n + alignment - 1) & !(alignment - 1) }

// Convert the ordinary descriptors in a generated DLL to equivalent RVA-based
// delay descriptors. The linker supplies all names/thunks; this only constructs
// the delay directory, avoiding a dependency on a particular delay-load runtime.
fn make_delay_imports(path: &Path) {
    let mut data = fs::read(path).unwrap();
    let pe = u32_at(&data, 0x3c) as usize;
    let coff = pe + 4;
    let optional = coff + 20;
    assert_eq!(u16_at(&data, optional), 0x20b);
    let sections = optional + u16_at(&data, coff + 16) as usize;
    let count = u16_at(&data, coff + 2) as usize;
    let rva_offset = |rva: u32, bytes: &[u8]| -> usize {
        (0..count).map(|i| sections + i * 40).find_map(|s| {
            let va = u32_at(bytes, s + 12);
            let size = u32_at(bytes, s + 16);
            (rva >= va && rva - va < size).then(|| (u32_at(bytes, s + 20) + rva - va) as usize)
        }).expect("import RVA must be file-backed")
    };
    let directories = optional + 112;
    let import_rva = u32_at(&data, directories + 8);
    assert_ne!(import_rva, 0);
    // object reads delay descriptors, names and thunks from the same section.
    let section = (0..count).map(|i| sections + i * 40).find(|&s| {
        let start = u32_at(&data, s + 12);
        import_rva >= start && import_rva - start < u32_at(&data, s + 16)
    }).expect("import descriptor section");
    let mut descriptor = rva_offset(import_rva, &data);
    let mut delay = Vec::new();
    loop {
        let original = u32_at(&data, descriptor);
        let name = u32_at(&data, descriptor + 12);
        let iat = u32_at(&data, descriptor + 16);
        if name == 0 { break; }
        for field in [1, name, 0, iat, if original == 0 { iat } else { original }, 0, 0, 0] {
            delay.extend_from_slice(&field.to_le_bytes());
        }
        descriptor += 20;
    }
    assert!(!delay.is_empty());
    delay.extend_from_slice(&[0; 32]);
    let raw_start = u32_at(&data, section + 20);
    let virtual_start = u32_at(&data, section + 12);
    let relative = align(u32_at(&data, section + 8), 8);
    let new_virtual = relative + delay.len() as u32;
    assert!(new_virtual <= u32_at(&data, section + 16), "insufficient import-section padding");
    let append = raw_start + relative;
    data[append as usize..append as usize + delay.len()].copy_from_slice(&delay);
    put32(&mut data, section + 8, new_virtual);
    put32(&mut data, directories + 8, 0);
    put32(&mut data, directories + 12, 0);
    put32(&mut data, directories + 13 * 8, virtual_start + relative);
    put32(&mut data, directories + 13 * 8 + 4, delay.len() as u32);
    fs::write(path, data).unwrap();
}

fn contains_ci(text: &str, needle: &str) {
    assert!(text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()), "missing {needle:?} in:\n{text}");
}

#[test]
#[ignore = "requires LLVM_MINGW LLVM-MinGW cross compiler and llvm-dlltool"]
fn reports_transitive_ordinary_and_delay_imports_through_case_insensitive_cycle() {
    let f = Fixture::new();
    let b = f.import_lib("b", "MiXeD.dll", "BFunc");
    let c = f.import_lib("c", "cycle.dll", "CFunc");
    let xmem = f.import_lib("xmem_import", "xmem.dll", "MissingXMemName\nOrdinalProbe @77 NONAME");
    let xg = f.import_lib("xg_import", "xg_xs.dll", "MissingXgDelayName");
    let chad = f.import_lib("chad_import", "chad.dll", "UnimplementedChad");
    f.compile_dll("mixed.DLL", "__declspec(dllimport) int CFunc(void);\n__declspec(dllimport) int MissingXMemName(void);\n__declspec(dllimport) int OrdinalProbe(void);\n__declspec(dllexport) int BFunc(void) { return CFunc()+MissingXMemName()+OrdinalProbe(); }\nint DllMain(void) { return 1; }", &[&c, &xmem], &f.guest);
    let delayed = f.compile_dll("cycle.dll", "__declspec(dllimport) int BFunc(void);\n__declspec(dllimport) int MissingXgDelayName(void);\n__declspec(dllimport) int UnimplementedChad(void);\n__declspec(dllexport) int CFunc(void) { return BFunc()+MissingXgDelayName()+UnimplementedChad(); }\nint DllMain(void) { return 1; }", &[&b, &xg, &chad], &f.guest);
    make_delay_imports(&delayed);
    let exe = f.compile_exe("__declspec(dllimport) int BFunc(void);\nint main(void) { return BFunc(); }", &[&b]);
    let (out, text) = f.inspect(&exe, &["MIXED.dll"]);
    assert!(!out.status.success(), "unsupported imports must fail: {text}");
    for needle in ["mixed.dll", "ordinary", "xmem.dll", "MissingXMemName", "#77", "cycle.dll", "delay", "xg_xs.dll", "MissingXgDelayName", "chad.dll", "UnimplementedChad", "cycle"] {
        contains_ci(&text, needle);
    }
    contains_ci(&text, "guest import cycle");
    // The launcher must reject the same transitive imports before running any
    // Wine, winepath or Metal-host executable, even when search is permitted.
    let marker = f.temp.path().join("started");
    let sentinel = f.temp.path().join("sentinel");
    fs::write(&sentinel, format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display())).unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o755)).unwrap();
    let prefix = f.temp.path().join("prefix");
    fs::create_dir(&prefix).unwrap();
    let launch = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("wine-run").arg(&exe)
        .arg("--wine").arg(&sentinel).arg("--winepath").arg(&sentinel)
        .arg("--metal-host").arg(&sentinel).arg("--converter").arg(&sentinel)
        .arg("--dll-dir").arg(&f.providers).arg("--wine-prefix").arg(&prefix)
        .arg("--allow-unverified-dll-search").arg("--guest-dll").arg("MIXED.dll")
        .output().expect("run launcher preflight");
    let launch_text = format!("{}\n{}", String::from_utf8_lossy(&launch.stdout), String::from_utf8_lossy(&launch.stderr));
    assert!(!launch.status.success(), "launcher must reject unsupported imports: {launch_text}");
    contains_ci(&launch_text, "MissingXMemName");
    contains_ci(&launch_text, "MissingXgDelayName");
    assert!(!marker.exists(), "preflight must not execute Wine, winepath, or the Metal host");
    // Each issue must carry its own provenance, not just occur elsewhere in a summary.
    for (symbol, image, kind, module) in [("MissingXMemName", "mixed.dll", "ordinary", "xmem.dll"), ("#77", "mixed.dll", "ordinary", "xmem.dll"), ("MissingXgDelayName", "cycle.dll", "delay", "xg_xs.dll"), ("UnimplementedChad", "cycle.dll", "delay", "chad.dll")] {
        let line = text.lines().find(|line| line.to_ascii_lowercase().contains(&symbol.to_ascii_lowercase())).unwrap();
        for part in [image, kind, module] { contains_ci(line, part); }
    }
}

#[test]
#[ignore = "requires LLVM_MINGW LLVM-MinGW cross compiler and llvm-dlltool"]
fn distinguishes_unclassified_imports_missing_guest_and_case_collision() {
    let f = Fixture::new();
    let system = f.import_lib("system", "KERNEL32.dll", "GetCurrentProcessId");
    let exe = f.compile_exe("__declspec(dllimport) int GetCurrentProcessId(void);\nint main(void) { return GetCurrentProcessId(); }", &[&system]);
    let (out, text) = f.inspect(&exe, &[]);
    assert!(out.status.success(), "external Wine imports are not confirmed failures: {text}");
    for needle in ["title.exe", "ordinary", "kernel32.dll", "GetCurrentProcessId", "unresolved"] { contains_ci(&text, needle); }
    let system_line = text.lines().find(|line| line.contains("GetCurrentProcessId")).unwrap();
    for needle in ["title.exe", "ordinary", "kernel32.dll", "unresolved"] { contains_ci(system_line, needle); }
    let (out, text) = f.inspect(&exe, &["Absent.dll"]);
    assert!(!out.status.success(), "explicitly required missing DLL must fail: {text}");
    contains_ci(&text, "Absent.dll");
    contains_ci(&text, "not found");
    f.compile_dll("Collision.dll", "int DllMain(void) { return 1; }", &[], &f.guest);
    f.compile_dll("collision.DLL", "int DllMain(void) { return 1; }", &[], &f.guest);
    // Case-insensitive host filesystems cannot represent this ambiguity.
    if fs::read_dir(&f.guest).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("collision.dll")).count() == 2 {
        let (out, text) = f.inspect(&exe, &["COLLISION.dll"]);
        assert!(!out.status.success(), "ambiguous case collision must fail: {text}");
        contains_ci(&text, "collision.dll");
        contains_ci(&text, "ambig");
        eprintln!("case-colliding guest DLLs rejected");
    } else {
        eprintln!("case-collision subcase unavailable on this filesystem");
    }
}

#[test]
fn wine_commands_do_not_enable_native_guest_execution_on_arm64() {
    let output = Command::new(env!("CARGO_BIN_EXE_victory")).arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("wine-imports"), "{help}");
    assert!(help.contains("wine-run"), "{help}");
    if cfg!(target_arch = "aarch64") {
        assert!(!help.contains("native-fixture"), "{help}");
        assert!(!help.contains("native-census"), "{help}");
    }
}
