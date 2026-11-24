use std::process::Command;

fn run(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_victory"))
        .args(args)
        .output()
        .unwrap()
}

// A synthetic PE32+ DLL with one named export; it contains no game data.
fn image() -> Vec<u8> {
    let mut d = vec![0u8; 0x400];
    d[0..2].copy_from_slice(b"MZ");
    d[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    d[0x40..0x44].copy_from_slice(b"PE\0\0");
    d[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    d[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    d[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    d[0x56..0x58].copy_from_slice(&0x2022u16.to_le_bytes());
    let opt = 0x58;
    d[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    d[opt + 24..opt + 32].copy_from_slice(&0x1_8000_0000u64.to_le_bytes());
    d[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    d[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    d[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    d[opt + 60..opt + 64].copy_from_slice(&0x200u32.to_le_bytes());
    d[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    d[opt + 112..opt + 116].copy_from_slice(&0x1000u32.to_le_bytes());
    d[opt + 116..opt + 120].copy_from_slice(&0x80u32.to_le_bytes());
    let sec = opt + 240;
    d[sec..sec + 6].copy_from_slice(b".edata");
    d[sec + 8..sec + 12].copy_from_slice(&0x200u32.to_le_bytes());
    d[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    d[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
    d[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
    let body = &mut d[0x200..];
    for (at, value) in [
        (12, 0x1040u32),
        (16, 1),
        (20, 1),
        (24, 1),
        (28, 0x1028),
        (32, 0x102c),
        (36, 0x1030),
        (0x28, 0x1200),
        (0x2c, 0x1044),
    ] {
        body[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    body[0x40..0x44].copy_from_slice(b"t.d\0");
    body[0x44..0x4a].copy_from_slice(b"alpha\0");
    d
}

#[test]
fn inspect_reports_image_and_named_export() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.dll");
    std::fs::write(&path, image()).unwrap();
    let output = run(&[
        "inspect".as_ref(),
        path.as_os_str(),
        "--imports".as_ref(),
        "--exports".as_ref(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "blake3",
        "kind           dll",
        ".edata",
        "imports        0 functions",
        "exports        1",
        "alpha",
        "0x1200",
    ] {
        assert!(out.contains(expected), "missing {expected:?} in {out}");
    }
}

#[test]
fn lib_exports_lists_short_import_objects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.lib");
    let strings = b"TestFunction\0Test.dll\0";
    let mut member = vec![0u8; 20];
    member[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
    member[6..8].copy_from_slice(&0x8664u16.to_le_bytes());
    member[12..16].copy_from_slice(&(strings.len() as u32).to_le_bytes());
    member[18..20].copy_from_slice(&4u16.to_le_bytes()); // IMPORT_NAME
    member.extend_from_slice(strings);
    let mut archive = b"!<arch>\n".to_vec();
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        "test.obj/",
        0,
        0,
        0,
        0,
        member.len()
    );
    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(&member);
    if member.len() % 2 != 0 {
        archive.push(b'\n');
    }
    std::fs::write(&path, archive).unwrap();
    let output = run(&["lib-exports".as_ref(), path.as_os_str(), "--list".as_ref()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("1 exports from 1 DLLs"), "{out}");
    assert!(
        out.contains("test.dll") && out.contains("TestFunction"),
        "{out}"
    );
}

#[test]
fn inspect_rejects_invalid_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.exe");
    std::fs::write(&path, b"not a PE").unwrap();
    let output = run(&["inspect".as_ref(), path.as_os_str()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parsing"));
    let missing = dir.path().join("missing.exe");
    let output = run(&["inspect".as_ref(), missing.as_os_str()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("reading"));
}
