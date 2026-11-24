//! Real CLI mapping, binding and execution of our own cross-built PE.
#![cfg(windows)]

use std::process::Command;

#[test]
#[ignore = "requires a separately built synthetic PE; see docs/wine-loader-tests.md"]
fn executes_synthetic_pe_through_cli() {
    let fixture = std::env::var_os("VICTORY_TEST_CLI_PE")
        .expect("set VICTORY_TEST_CLI_PE to the synthetic fixture");
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .args(["run"])
        .arg(fixture)
        .arg("--execute")
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "status: {}\n{stdout}\n{stderr}", output.status);
    assert!(stdout.contains("bound          "), "{stdout}\n{stderr}");
    assert!(stdout.contains("0 missing"), "{stdout}\n{stderr}");
    assert!(stdout.contains("VIctory synthetic entry executed"), "{stdout}\n{stderr}");
    assert!(stdout.contains("title exited with 42"), "{stdout}\n{stderr}");
}
