//! Synthetic TLS-bearing D3D12.X graphics guest through the Wine Metal proxy.
#![cfg(windows)]

use std::process::{Command, Output};

fn run(fixture: &std::ffi::OsStr, temp: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(fixture).arg("--execute")
        .current_dir(temp)
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.join("cache"))
        .env("VICTORY_DATA_DIR", temp.join("data"))
        .output().unwrap()
}

#[test]
#[ignore = "requires separately built TLS graphics PEs and a running native Metal proxy"]
fn executes_tls_graphics_through_wine_metal_proxy() {
    let fixture = std::env::var_os("VICTORY_TEST_D3D12X_TLS_GRAPHICS_PE")
        .expect("set VICTORY_TEST_D3D12X_TLS_GRAPHICS_PE to the synthetic TLS graphics fixture");
    let invalid = std::env::var_os("VICTORY_TEST_D3D12X_TLS_GRAPHICS_INVALID_PE")
        .expect("set VICTORY_TEST_D3D12X_TLS_GRAPHICS_INVALID_PE to the invalid-topology fixture");
    std::env::var_os("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
    std::env::var_os("VICTORY_WINE_METAL_TOKEN").expect("set VICTORY_WINE_METAL_TOKEN");
    let temp = tempfile::tempdir().unwrap();
    let output = run(&fixture, temp.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}\n{stderr}");
    assert!(output.status.success(), "status: {}\n{logs}", output.status);
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{logs}");
    assert!(logs.contains("Wine Metal proxy connected"), "{logs}");
    assert!(stdout.contains("TLS compiler state and thread callbacks passed"), "{logs}");
    assert!(stdout.contains("D3D12X synthetic graphics pixels and padding passed"), "{logs}");
    assert!(stdout.contains("title exited with 42"), "{logs}");

    let rejected = run(&invalid, temp.path());
    let rejected_logs = format!("{}\n{}", String::from_utf8_lossy(&rejected.stdout), String::from_utf8_lossy(&rejected.stderr));
    assert!(rejected.status.success(), "status: {}\n{rejected_logs}", rejected.status);
    assert!(rejected_logs.contains("D3D12X graphics command list rejected"), "{rejected_logs}");
    assert!(rejected_logs.contains("title exited with 10"), "{rejected_logs}");
    assert!(rejected_logs.contains("TLS compiler state and thread callbacks passed"), "{rejected_logs}");
    assert!(!rejected_logs.contains("D3D12X synthetic graphics pixels and padding passed"), "{rejected_logs}");
    assert!(!rejected_logs.contains("title exited with 42"), "{rejected_logs}");
}
