//! Opt-in TLS-bearing PE rendering and PresentX through the real Wine Metal helper.
#![cfg(windows)]

#[test]
#[ignore = "requires a separately built TLS presentation PE and a running presentation-enabled Metal helper"]
fn translated_graphics_present_x_preserves_source_and_orders_queue() {
    let fixture = std::env::var_os("VICTORY_TEST_D3D12X_PRESENT_PE").expect("set VICTORY_TEST_D3D12X_PRESENT_PE");
    std::env::var_os("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
    std::env::var_os("VICTORY_WINE_METAL_TOKEN").expect("set VICTORY_WINE_METAL_TOKEN");
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(fixture).arg("--execute")
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let logs = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "status: {}\n{logs}", output.status);
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{logs}");
    assert!(logs.contains("Wine Metal proxy connected"), "{logs}");
    assert!(stdout.contains("TLS compiler state and thread callbacks passed"), "{logs}");
    assert!(stdout.contains("PresentX ordered graphics, source preservation and resize passed"), "{logs}");
    assert!(stdout.contains("title exited with 42"), "{logs}");
}
