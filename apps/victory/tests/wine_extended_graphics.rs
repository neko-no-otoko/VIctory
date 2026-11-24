//! Indexed, depth, and blending coverage through the real Wine Metal proxy.
#![cfg(windows)]

use std::process::Command;

fn run(fixture: &std::ffi::OsStr, temp: &std::path::Path, expected: u32) {
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(fixture).arg("--execute")
        .current_dir(temp)
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.join("cache"))
        .env("VICTORY_DATA_DIR", temp.join("data"))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}\n{stderr}");
    assert!(output.status.success(), "status: {}\n{logs}", output.status);
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{logs}");
    if expected == 42 {
        assert!(logs.contains("Wine Metal proxy connected"), "{logs}");
    } else {
        assert!(!logs.contains("Wine Metal proxy connected"), "{logs}");
    }
    assert!(stdout.contains(&format!("title exited with {expected}")), "{logs}");
}

#[test]
#[ignore = "requires separately built graphics PEs and a running native Metal proxy"]
fn indexed_depth_and_blending_through_wine_metal_proxy() {
    std::env::var_os("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
    std::env::var_os("VICTORY_WINE_METAL_TOKEN").expect("set VICTORY_WINE_METAL_TOKEN");
    let temp = tempfile::tempdir().unwrap();
    for (valid, invalid) in [
        ("VICTORY_TEST_D3D12X_INDEXED_PE", "VICTORY_TEST_D3D12X_INDEXED_INVALID_PE"),
        ("VICTORY_TEST_D3D12X_DEPTH_PE", "VICTORY_TEST_D3D12X_DEPTH_INVALID_PE"),
        ("VICTORY_TEST_D3D12X_BLEND_PE", "VICTORY_TEST_D3D12X_BLEND_INVALID_PE"),
    ] {
        let fixture = std::env::var_os(valid).unwrap_or_else(|| panic!("set {valid}"));
        let rejected = std::env::var_os(invalid).unwrap_or_else(|| panic!("set {invalid}"));
        run(&fixture, temp.path(), 42);
        run(&rejected, temp.path(), 43);
    }
}
