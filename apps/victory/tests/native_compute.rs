#![cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::process::Command;

fn tool(dir: &std::path::Path, name: &str, args: &[&str]) {
    let output = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
#[ignore = "requires a trusted Apple Metal Shader Converter and Metal GPU"]
fn synthetic_console_compute_through_native_cli() {
    assert!(std::env::var_os("VICTORY_METAL_CONVERTER").is_some());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = include_str!("../../../tests/fixtures/windows_d3d12x_compute.c")
        .replace("static void print(const char *s, DWORD n) { DWORD w; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, n, &w, 0); }", "static void print(const char *s, DWORD n) { (void)s; (void)n; }")
        .replace("SLOT(dev,0x98,CreateUav)(dev,resource,0,&uav,handle);", "SLOT(dev,0x98,CreateUav)(dev,resource,0,0,handle);")
        .replace("SAY(\"D3D12X synthetic compute readback 42\\r\\n\"); return 42;", "typedef unsigned (*Release)(void *); SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap); SLOT(resource,0x10,Release)(resource); SLOT(dev,0x10,Release)(dev); return 42;")
        .replace("unsigned mainCRTStartup(void *unused) {\n    (void)unused;", "int marker=7; int *volatile relocated=&marker;\nunsigned mainCRTStartup(void) {\n    if (*relocated != 7) return 99;");
    std::fs::write(root.join("compute.c"), &source).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "compute.c", "-o", "compute.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:compute.exe", "compute.obj", "d3d.lib"]);
    let output = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("compute.exe")).arg("--execute").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixture entry returned 42"), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let trap = source.replace("    HeapDesc hd =", "    ((void (*)(void))(*(void ***)dev)[3])();\n    HeapDesc hd =");
    std::fs::write(root.join("trap.c"), trap).unwrap();
    tool(root, "x86_64-w64-mingw32-clang", &["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables", "-c", "trap.c", "-o", "trap.obj"]);
    tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:trap.exe", "trap.obj", "d3d.lib"]);
    let output = Command::new(env!("CARGO_BIN_EXE_victory")).arg("native-fixture").arg(root.join("trap.exe")).arg("--execute").output().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(output.status.signal(), Some(6), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported native D3D12.X vtable method"));
}
