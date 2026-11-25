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
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(42), "status: {}\n{stdout}\n{stderr}", output.status);
    assert!(stdout.contains("bound          "), "{stdout}\n{stderr}");
    assert!(stdout.contains("1 hle,"), "{stdout}\n{stderr}");
    assert!(stdout.contains("0 missing"), "{stdout}\n{stderr}");
    assert!(stdout.contains("VIctory synthetic entry executed"), "{stdout}\n{stderr}");
    assert!(stdout.contains("XGameRuntimeInitialize returned S_OK"), "{stdout}\n{stderr}");
    assert!(stdout.contains("title exited with 42"), "{stdout}\n{stderr}");
}

#[test]
#[ignore = "requires a separately built D3D12X PE and working D3D12 host; see docs/wine-loader-tests.md"]
fn executes_synthetic_d3d12x_through_cli() {
    let fixture = std::env::var_os("VICTORY_TEST_D3D12X_PE")
        .expect("set VICTORY_TEST_D3D12X_PE to the synthetic D3D12X fixture");
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(fixture).arg("--execute")
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(42), "status: {}\n{stdout}\n{stderr}", output.status);
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{stdout}\n{stderr}");
    assert!(stdout.contains("host D3D12 device ready") || stderr.contains("host D3D12 device ready"), "{stdout}\n{stderr}");
    assert!(!stdout.contains("no host command list") && !stderr.contains("no host command list"), "{stdout}\n{stderr}");
    assert!(!stdout.contains("no host GPU runs") && !stderr.contains("no host GPU runs"), "{stdout}\n{stderr}");
    assert!(stdout.contains("D3D12X device list and fence passed"), "{stdout}\n{stderr}");
    assert!(stdout.contains("title exited with 42"), "{stdout}\n{stderr}");
}

#[test]
#[ignore = "requires a separately built compute PE and working D3D12 host; see docs/wine-loader-tests.md"]
fn executes_synthetic_d3d12x_compute_through_cli() {
    let fixture = std::env::var_os("VICTORY_TEST_D3D12X_COMPUTE_PE")
        .expect("set VICTORY_TEST_D3D12X_COMPUTE_PE to the synthetic compute fixture");
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(fixture).arg("--execute")
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}\n{stderr}");
    assert_eq!(output.status.code(), Some(42), "status: {}\n{logs}", output.status);
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{logs}");
    assert!(logs.contains("host D3D12 device ready"), "{logs}");
    assert!(logs.contains("shader translated") && logs.contains("host pipeline shaders"), "{logs}");
    assert!(!logs.contains("dispatch without a host pipeline dropped"), "{logs}");
    assert!(stdout.contains("D3D12X synthetic compute readback 42"), "{logs}");
    assert!(stdout.contains("title exited with 42"), "{logs}");
}

#[test]
#[ignore = "requires a TLS compute PE and running native Metal proxy; see docs/wine-loader-tests.md"]
fn executes_tls_d3d12x_compute_through_wine_metal_proxy() {
    let fixture = std::env::var_os("VICTORY_TEST_D3D12X_TLS_COMPUTE_PE")
        .expect("set VICTORY_TEST_D3D12X_TLS_COMPUTE_PE to the synthetic TLS compute fixture");
    std::env::var_os("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
    let mut wrong_token = std::env::var("VICTORY_WINE_METAL_TOKEN")
        .expect("set VICTORY_WINE_METAL_TOKEN").into_bytes();
    assert_eq!(wrong_token.len(), 32, "token must be 32 hex digits");
    wrong_token[0] = if wrong_token[0] == b'0' { b'1' } else { b'0' };
    let wrong_token = String::from_utf8(wrong_token).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(&fixture).arg("--execute")
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}\n{stderr}");
    assert_eq!(output.status.code(), Some(42), "status: {}\n{logs}", output.status);
    assert!(logs.contains("Wine Metal proxy connected"), "{logs}");
    assert!(stdout.contains("1 hle,") && stdout.contains("0 missing"), "{logs}");
    assert!(stdout.contains("TLS compiler state and thread callbacks passed"), "{logs}");
    assert!(stdout.contains("D3D12X synthetic compute readback 42"), "{logs}");
    assert!(stdout.contains("title exited with 42"), "{logs}");

    let denied = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("run").arg(&fixture).arg("--execute")
        .current_dir(temp.path())
        .env_remove("VICTORY_CONFIG")
        .env("VICTORY_LOG", "info")
        .env("VICTORY_CACHE_DIR", temp.path().join("cache"))
        .env("VICTORY_DATA_DIR", temp.path().join("data"))
        .env("VICTORY_WINE_METAL_TOKEN", wrong_token)
        .output().unwrap();
    let denied_logs = format!("{}\n{}", String::from_utf8_lossy(&denied.stdout),
        String::from_utf8_lossy(&denied.stderr));
    assert!(!denied.status.success(), "wrong token was accepted: {denied_logs}");
    assert!(denied_logs.contains("TLS compiler state and thread callbacks passed"), "{denied_logs}");
    assert!(!denied_logs.contains("Wine Metal proxy connected"), "{denied_logs}");
    assert!(!denied_logs.contains("D3D12X synthetic compute readback 42"), "{denied_logs}");
}
