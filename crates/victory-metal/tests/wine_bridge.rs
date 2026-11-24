#![cfg(target_os = "macos")]

// Run on a Metal-capable macOS host with Wine and LLVM-MinGW on PATH:
// WINE=/path/to/wine cargo test -p victory-metal --test wine_bridge -- --ignored
#[allow(dead_code)]
#[path = "../examples/wine_metal_bridge.rs"]
mod bridge;

use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

// The test executable acts as a subprocess so a stuck Wine guest can be killed.
#[test]
fn bridge_subprocess() {
    let Some(guest) = std::env::var_os("VICTORY_WINE_BRIDGE_TEST_GUEST") else {
        assert!(bridge::host::run(std::iter::empty::<std::ffi::OsString>()).is_err());
        return;
    };
    let wine = std::env::var_os("WINE").expect("WINE must name the Wine executable");
    bridge::host::run([wine, guest]).expect("Wine Metal bridge failed");
}

#[test]
#[ignore = "requires Wine, LLVM-MinGW, and a Metal GPU; see command above"]
fn wine_guest_tls_and_metal_readback() {
    let wine = std::env::var_os("WINE").expect("set WINE to the Wine executable path");
    let temp = tempfile::tempdir().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dll = temp.path().join("victory_metal_bridge.dll");
    let import = temp.path().join("libvictory_metal_bridge.dll.a");
    let guest = temp.path().join("wine_metal_guest.exe");
    let status = Command::new("x86_64-w64-mingw32-clang")
        .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-shared"])
        .arg(fixtures.join("wine_metal_bridge.c"))
        .arg("-o").arg(&dll)
        .arg(format!("-Wl,--out-implib,{}", import.display()))
        .status().expect("run LLVM-MinGW clang for bridge DLL");
    assert!(status.success(), "bridge DLL compilation failed: {status}");
    let status = Command::new("x86_64-w64-mingw32-clang")
        .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-fno-emulated-tls"])
        .arg(fixtures.join("wine_metal_guest.c")).arg(&import)
        .arg("-o").arg(&guest)
        .status().expect("run LLVM-MinGW clang for guest");
    assert!(status.success(), "guest compilation failed: {status}");

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bridge_subprocess", "--nocapture"])
        .env("VICTORY_WINE_BRIDGE_TEST_GUEST", &guest)
        .env("WINEPREFIX", temp.path().join("prefix"))
        .env("WINE", &wine)
        .env("WINEDEBUG", "-all")
        .stdin(Stdio::null())
        .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
        .env("MVK_CONFIG_LOG_LEVEL", "0")
        .process_group(0)
        .spawn().expect("start bridge subprocess");
    // Wine's server can outlive the guest; target only this test's prefix.
    let stop_server = || {
        let wineserver = std::path::Path::new(&wine).parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join("wineserver"))
            .unwrap_or_else(|| "wineserver".into());
        let _ = Command::new(wineserver).arg("-k")
            .env("WINEPREFIX", temp.path().join("prefix"))
            .stdin(Stdio::null()).stdout(Stdio::null()).status();
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().expect("wait for bridge subprocess") {
            stop_server();
            assert!(status.success(), "Wine guest or GPU readback failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            // The bridge and its Wine descendants share this private process group.
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // SAFETY: a negative PID targets only the group created above.
            unsafe { kill(-(child.id() as i32), 9) };
            let _ = child.kill();
            let _ = child.wait();
            stop_server();
            panic!("Wine Metal bridge timed out");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
