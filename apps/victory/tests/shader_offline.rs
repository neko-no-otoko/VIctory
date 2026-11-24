use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_victory"))
        .current_dir(dir)
        .env_remove("VICTORY_PAIRS")
        .env_remove("VICTORY_EMIT_DUMP")
        .args(args)
        .output()
        .unwrap()
}

// Synthetic compute program: one GFX10 s_endpgm, wrapped in the same
// FSSM/XBCH/XDAT/DXBC layers as an Xbox shader, with no title data.
fn shader() -> Vec<u8> {
    let mut payload = vec![0u8; 0x68];
    payload[..8].copy_from_slice(&6u64.to_le_bytes());
    for slot in [0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] {
        payload[slot..slot + 8].copy_from_slice(&0xdead_beef_dead_beefu64.to_le_bytes());
    }
    let record = payload.len();
    payload[0x60..0x68].copy_from_slice(&(record as u64).to_le_bytes());
    let code_at = record + 0x54 + 0x40;
    payload.resize(code_at, 0);
    payload[record..record + 8].copy_from_slice(&(code_at as u64).to_le_bytes());
    payload[record + 8..record + 16].copy_from_slice(&4u64.to_le_bytes());
    payload[record + 0x50..record + 0x54].copy_from_slice(&2u32.to_le_bytes());
    let stage = record + 0x54;
    for (i, n) in [64u32, 1, 1].iter().enumerate() {
        payload[stage + 0x28 + i * 4..stage + 0x2c + i * 4].copy_from_slice(&n.to_le_bytes());
    }
    payload.extend_from_slice(&0xbf81_0000u32.to_le_bytes());
    let mut fssm = b"FSSM".to_vec();
    fssm.extend_from_slice(&0x101u32.to_le_bytes());
    fssm.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    fssm.extend_from_slice(&payload);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&fssm, 6);
    let mut xbch = b"XBCH".to_vec();
    xbch.extend_from_slice(&3u32.to_le_bytes());
    xbch.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    xbch.extend_from_slice(&(fssm.len() as u64).to_le_bytes());
    xbch.extend_from_slice(&packed);
    let mut dxil = vec![0u8; 0x18];
    dxil.extend_from_slice(b"XDAT");
    for n in [1u32, 0x24, 0x0d, 0x30, xbch.len() as u32] {
        dxil.extend_from_slice(&n.to_le_bytes());
    }
    dxil.extend_from_slice(&xbch);
    let mut container = vec![0u8; 0x24];
    container[..4].copy_from_slice(b"DXBC");
    container[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
    container[0x18..0x1c].copy_from_slice(&((0x2c + dxil.len()) as u32).to_le_bytes());
    container[0x1c..0x20].copy_from_slice(&1u32.to_le_bytes());
    container[0x20..0x24].copy_from_slice(&0x24u32.to_le_bytes());
    container.extend_from_slice(b"DXIL");
    container.extend_from_slice(&(dxil.len() as u32).to_le_bytes());
    container.extend_from_slice(&dxil);
    container
}

fn sgd5(data: &[u8], corrupt: bool) -> Vec<u8> {
    let packed = if corrupt {
        vec![0x07]
    } else {
        miniz_oxide::deflate::compress_to_vec(data, 6)
    };
    let n = (packed.len() + 12) as u32;
    let mut out = b"SGD5\x05test\0\x3c".to_vec();
    for x in [n, 0x000d_dce7, n, data.len() as u32] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend_from_slice(&packed);
    out
}

#[test]
fn analysis_disassembles_raw_and_packed_programs() {
    let dir = tempfile::tempdir().unwrap();
    let mut archive = b"prefix".to_vec();
    archive.extend_from_slice(&shader());
    std::fs::write(dir.path().join("raw.bin"), archive).unwrap();
    std::fs::write(dir.path().join("packed.sgd"), sgd5(&shader(), false)).unwrap();
    std::fs::write(dir.path().join("corrupt.sgd"), sgd5(&shader(), true)).unwrap();
    let output = run(dir.path(), &["shader-info", ".", "--disasm"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        out.contains("containers found       2 (in 2 files)"),
        "{out}"
    );
    assert!(
        out.contains("decoded programs     2 (2 instructions)"),
        "{out}"
    );
    assert!(out.contains("s_endpgm") && out.contains("test"), "{out}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("corrupt record"));
}

#[test]
fn translation_processes_native_code_and_deduplicates() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.bin"), shader()).unwrap();
    std::fs::write(dir.path().join("b.sgd"), sgd5(&shader(), false)).unwrap();
    let output = run(dir.path(), &["translate", ".", "--jobs", "1", "--list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        out.contains("containers   2 (1 duplicates skipped)"),
        "{out}"
    );
    assert!(out.contains("programs     1 translated of 1"), "{out}");
    assert!(out.contains("1 emitted of 1 compute programs"), "{out}");
}

#[cfg(not(windows))]
#[test]
fn windows_validation_and_pipelines_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--validate", "--pipelines"] {
        let output = run(dir.path(), &["translate", ".", flag]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("require Windows"));
    }
}

/// Set VICTORY_TEST_METAL_CONVERTER to Apple's dylib to exercise the real API.
#[cfg(target_os = "macos")]
#[test]
fn apple_converter_translates_emitted_compute_dxil() {
    let Some(library) = std::env::var_os("VICTORY_TEST_METAL_CONVERTER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("shader.bin");
    let metal = dir.path().join("shader.metallib");
    std::fs::write(&input, shader()).unwrap();
    let emitted = Command::new(env!("CARGO_BIN_EXE_victory"))
        .env_remove("VICTORY_PAIRS")
        .env("VICTORY_EMIT_DUMP", dir.path())
        .args(["translate", "--list", "--jobs", "1"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        emitted.status.success(),
        "{}",
        String::from_utf8_lossy(&emitted.stderr)
    );
    let dxil = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "dxil"))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("convert-metal")
        .arg(&dxil)
        .arg("--library")
        .arg(&library)
        .arg("--stage")
        .arg("compute")
        .arg("--output")
        .arg(&metal)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(std::fs::read(&metal).unwrap().starts_with(b"MTLB"));

    let wrong_stage = dir.path().join("wrong.metallib");
    let output = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("convert-metal")
        .arg(&dxil)
        .arg("--library")
        .arg(&library)
        .arg("--stage")
        .arg("vertex")
        .arg("--output")
        .arg(&wrong_stage)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!wrong_stage.exists());
}
