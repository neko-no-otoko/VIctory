//! Compile the same presentation workload as a real PE and run it in native-fixture.
#![cfg(all(target_os = "macos", target_arch = "x86_64"))]

use std::process::Command;

fn tool(dir: &std::path::Path, name: &str, args: &[&str]) {
    let result = Command::new(name).args(args).current_dir(dir).output().unwrap();
    assert!(result.status.success(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
}

#[test]
#[ignore = "requires llvm-mingw, Metal Shader Converter, an Aqua session and a presentation-enabled helper"]
fn real_pe_presents_on_native_fixture_main_thread() {
    std::env::var_os("VICTORY_METAL_CONVERTER").expect("set VICTORY_METAL_CONVERTER");
    std::env::var_os("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
    std::env::var_os("VICTORY_WINE_METAL_TOKEN").expect("set VICTORY_WINE_METAL_TOKEN");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("present.c"), include_str!("../../../tests/fixtures/windows_d3d12x_present_graphics.c")).unwrap();
    std::fs::write(root.join("windows_d3d12x_ordered_shaders.h"), include_str!("../../../tests/fixtures/windows_d3d12x_ordered_shaders.h")).unwrap();
    std::fs::write(root.join("d3d.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n").unwrap();
    tool(root, "llvm-dlltool", &["-m", "i386:x86-64", "-d", "d3d.def", "-l", "d3d.lib"]);
    for (variant, format) in [("rgba", "28"), ("rgba-srgb", "29"), ("bgra", "87"), ("bgra-srgb", "91")] {
        let define = format!("-DTARGET_FORMAT={format}");
        let mut args = vec!["-O2", "-DNATIVE_PRESENT", define.as_str(), "-fno-unwind-tables", "-fno-asynchronous-unwind-tables"];
        args.extend(["-c", "present.c", "-o", "present.obj"]);
        tool(root, "x86_64-w64-mingw32-clang", &args);
        tool(root, "lld-link", &["/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib", "/dynamicbase", "/out:present.exe", "present.obj", "d3d.lib"]);
        let output = Command::new(env!("CARGO_BIN_EXE_victory"))
            .arg("native-fixture").arg(root.join("present.exe")).arg("--execute")
            .output().unwrap();
        let logs = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{variant} status: {}\n{logs}", output.status);
        assert!(logs.contains("fixture entry returned 42"), "{variant}: {logs}");
    }
}
