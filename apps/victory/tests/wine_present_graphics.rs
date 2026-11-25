//! Opt-in TLS-bearing PE rendering and PresentX through the real Wine Metal helper.
#![cfg(any(windows, all(target_os = "macos", target_arch = "x86_64")))]

#[cfg(windows)]
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

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod normal_wine {
    use std::fs::{self, File};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};
    use std::time::{Duration, Instant};

    fn run(cmd: &mut Command, dir: &Path) -> Output {
        let stdout = dir.join("stdout.log");
        let stderr = dir.join("stderr.log");
        let mut child = cmd.process_group(0).stdout(Stdio::from(File::create(&stdout).unwrap()))
            .stderr(Stdio::from(File::create(&stderr).unwrap())).spawn().expect("spawn command");
        let deadline = Instant::now() + Duration::from_secs(180);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() { break status; }
            if Instant::now() >= deadline {
                let _ = Command::new("/bin/kill").arg("-KILL").arg(format!("-{}", child.id())).status();
                let _ = child.kill();
                let _ = child.wait();
                panic!("timed out: {cmd:?}\n{}\n{}", fs::read_to_string(&stdout).unwrap_or_default(), fs::read_to_string(&stderr).unwrap_or_default());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        Output { status, stdout: fs::read(stdout).unwrap(), stderr: fs::read(stderr).unwrap() }
    }

    fn checked(cmd: &mut Command, dir: &Path) -> Output {
        let output = run(cmd, dir);
        assert!(output.status.success(), "{cmd:?}: {}\n{}\n{}", output.status,
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        output
    }

    struct Prefix(PathBuf);
    impl Drop for Prefix {
        fn drop(&mut self) {
            for option in ["-k", "-w"] {
                let _ = Command::new("wineserver").arg(option).env("WINEPREFIX", &self.0)
                    .stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }
    }

    #[test]
    #[ignore = "requires Wine, LLVM-MinGW, real d3d12_xs.dll and a running presentation-enabled Metal helper in an Aqua session"]
    fn normal_wine_presents_all_typed_color_formats() {
        for name in ["VICTORY_WINE_METAL_ADDR", "VICTORY_WINE_METAL_TOKEN"] {
            assert!(std::env::var_os(name).is_some_and(|v| !v.is_empty()), "set {name}");
        }
        let llvm = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW"));
        let dll = fs::canonicalize(std::env::var_os("VICTORY_TEST_D3D12X_DLL").expect("set VICTORY_TEST_D3D12X_DLL"))
            .expect("D3D12X DLL path");
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let prefix = Prefix(dir.join("prefix"));
        fs::copy(&dll, dir.join("d3d12_xs.dll")).unwrap();
        let backend = dll.parent().unwrap().join("victory_xmem_backend.dll");
        fs::copy(&backend, dir.join("victory_xmem_backend.dll"))
            .unwrap_or_else(|e| panic!("copy {}: {e}", backend.display()));
        fs::write(dir.join("d3d12_xs.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\n D3D12XboxCreateDevice\n").unwrap();
        checked(Command::new(llvm.join("bin/llvm-dlltool")).current_dir(dir)
            .args(["-m", "i386:x86-64", "-d", "d3d12_xs.def", "-l", "d3d12_xs.lib"]), dir);
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/windows_d3d12x_present_graphics.c");
        let mut winepath = Command::new("winepath");
        winepath.env("WINEPREFIX", &prefix.0).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=").arg("-w")
            .arg(llvm.join("x86_64-w64-mingw32/bin"));
        let runtime = checked(&mut winepath, dir);
        let runtime = String::from_utf8(runtime.stdout).unwrap();
        for format in [28, 29, 87, 91] {
            let define = format!("-DTARGET_FORMAT={format}");
            checked(Command::new(llvm.join("bin/x86_64-w64-mingw32-clang")).current_dir(dir)
                .args(["-std=c11", "-fno-emulated-tls", "-O1", "-DNORMAL_WINE_PRESENT", &define])
                .arg(&source).arg("d3d12_xs.lib").args(["-o", "present.exe"]), dir);
            let output = run(Command::new("wine").current_dir(dir).arg(dir.join("present.exe"))
                .env("WINEPREFIX", &prefix.0).env("WINEDEBUG", "-all")
                .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=")
                .env("MVK_CONFIG_LOG_LEVEL", "0").env("WINEPATH", runtime.trim()), dir);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let logs = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(output.status.code(), Some(42), "format {format}: {logs}");
            assert!(stdout.contains("TLS compiler state and thread callbacks passed"), "format {format}: {logs}");
            assert!(stdout.contains("PresentX ordered graphics, source preservation and resize passed"), "format {format}: {logs}");
        }
    }

    #[test]
    #[ignore = "requires Wine, LLVM-MinGW, provider DLLs and a fresh presentation helper with drawable capture enabled"]
    fn normal_wine_presents_uhd_and_scaled_drawable() {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let addr = std::env::var("VICTORY_WINE_METAL_ADDR").expect("set VICTORY_WINE_METAL_ADDR");
        let token = std::env::var("VICTORY_WINE_METAL_TOKEN").expect("set VICTORY_WINE_METAL_TOKEN");
        assert_eq!(token.len(), 32);
        let mut auth = [0u8; 16];
        for (i, byte) in auth.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&token[2*i..2*i+2], 16).unwrap();
        }
        let capture = PathBuf::from(std::env::var_os("VICTORY_TEST_DRAWABLE_DIR").expect("set VICTORY_TEST_DRAWABLE_DIR"));
        assert!(!capture.join("drawable-00000000.bgra").exists(), "capture directory must be empty");
        // Prime the helper with a small presentation so its drawable stays
        // bounded while the following UHD source is downscaled into it.
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        for value in [0x3147_4d56u32, 4, 64, 64, 28, 256, 16384, 0] {
            stream.write_all(&value.to_le_bytes()).unwrap();
        }
        stream.write_all(&auth).unwrap();
        stream.write_all(&vec![0; 16384]).unwrap();
        let mut header = [0u8; 16];
        stream.read_exact(&mut header).unwrap();
        let word = |n| u32::from_le_bytes(header[n..n+4].try_into().unwrap());
        assert_eq!(word(0), 0x3147_4d56);
        let (status, len, message_len) = (word(4), word(8) as usize, word(12) as usize);
        assert!(len <= 16 * 1024 * 1024 + 12 && message_len <= 4096);
        let mut payload = vec![0; len + message_len];
        stream.read_exact(&mut payload).unwrap();
        assert_eq!(status, 0, "priming presentation failed: {:?}", &payload[len..]);
        drop(stream);

        let llvm = PathBuf::from(std::env::var_os("LLVM_MINGW").expect("set LLVM_MINGW"));
        let dll = fs::canonicalize(std::env::var_os("VICTORY_TEST_D3D12X_DLL").expect("set VICTORY_TEST_D3D12X_DLL")).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let prefix = Prefix(dir.join("prefix"));
        fs::copy(&dll, dir.join("d3d12_xs.dll")).unwrap();
        fs::copy(dll.parent().unwrap().join("victory_xmem_backend.dll"), dir.join("victory_xmem_backend.dll")).unwrap();
        fs::write(dir.join("d3d12_xs.def"), "LIBRARY d3d12_xs.dll\nEXPORTS\n D3D12XboxCreateDevice\n").unwrap();
        checked(Command::new(llvm.join("bin/llvm-dlltool")).current_dir(dir)
            .args(["-m", "i386:x86-64", "-d", "d3d12_xs.def", "-l", "d3d12_xs.lib"]), dir);
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/windows_d3d12x_present_graphics.c");
        checked(Command::new(llvm.join("bin/x86_64-w64-mingw32-clang")).current_dir(dir)
            .args(["-std=c11", "-fno-emulated-tls", "-O1", "-DNORMAL_WINE_PRESENT", "-DUHD_PRESENT"])
            .arg(source).arg("d3d12_xs.lib").args(["-o", "uhd.exe"]), dir);
        let runtime = checked(Command::new("winepath").env("WINEPREFIX", &prefix.0).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=").arg("-w")
            .arg(llvm.join("x86_64-w64-mingw32/bin")), dir);
        let output = run(Command::new("wine").current_dir(dir).arg(dir.join("uhd.exe"))
            .env("WINEPREFIX", &prefix.0).env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "d3d12_xs=n;mscoree,mshtml=")
            .env("MVK_CONFIG_LOG_LEVEL", "0").env("WINEPATH", String::from_utf8(runtime.stdout).unwrap().trim()), dir);
        let text = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.status.code(), Some(42), "{text}");
        assert!(text.contains("Wine UHD PresentX passed"), "{text}");

        let bytes = fs::read(capture.join("drawable-00000001.bgra")).expect("UHD guest drawable capture");
        assert!(bytes.len() >= 8);
        let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert!(width >= 8 && height >= 8 && width < 3840 && height < 2160);
        assert_eq!(bytes.len(), 8 + width * height * 4);
        for (x, y) in [(width/8, height/8), (3*width/8, 5*height/8),
                       (5*width/8, 3*height/8), (7*width/8, 7*height/8)] {
            let sx = ((2*x+1)*3840)/(2*width);
            let sy = ((2*y+1)*2160)/(2*height);
            let (bx, by) = (sx/64, sy/64);
            let expected = [(bx*31+by*3) as u8, (bx*7+by*29) as u8,
                            (bx*17+by*13) as u8, (64+(bx+by)%192) as u8];
            let at = 8 + (y*width+x)*4;
            assert_eq!(&bytes[at..at+4], &expected, "drawable ({x},{y}), source ({sx},{sy})");
        }
    }
}
