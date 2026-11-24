#![cfg(target_os = "macos")]

use std::process::Command;
use victory_metal::render_triangle;
use victory_shader::native::StageIo;
use victory_shader::{Container, HwStage, NativeProgram, emit, io, ir, resource, structure};

const VERTEX: &str = r#"
#include <metal_stdlib>
using namespace metal;
vertex float4 triangle(uint id [[vertex_id]]) {
    float2 p[3] = {float2(-1.0, -1.0), float2(3.0, -1.0), float2(-1.0, 3.0)};
    return float4(p[id], 0.0, 1.0);
}
"#;

// Synthetic GFX10 pixel shader exporting RGBA (1, 0, 0, 1), no game data.
fn fragment_dxil() -> Vec<u8> {
    let words: &[u32] = &[
        0x7e00_02ff,
        0x3f80_0000, // v_mov_b32 v0, 1.0f
        0x7e02_0280, // v_mov_b32 v1, 0
        0x7e04_0280, // v_mov_b32 v2, 0
        0x7e06_02ff,
        0x3f80_0000, // v_mov_b32 v3, 1.0f
        0xf800_180f,
        0x0302_0100, // exp mrt0, v0, v1, v2, v3 done vm
        0xbf81_0000,
    ];
    let native = NativeProgram {
        stage: HwStage::Ps,
        wave64: false,
        vgprs: 4,
        layout: vec![],
        used: vec![],
        io: StageIo::Ps {
            inputs: vec![],
            input_addr: 0,
            db_shader_control: 0,
            patches: vec![],
        },
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    };
    // Minimal OSG1 signature with SV_Target0 float4 in register 0.
    let mut sig = vec![0u8; 40];
    sig[0..4].copy_from_slice(&1u32.to_le_bytes());
    sig[4..8].copy_from_slice(&8u32.to_le_bytes());
    sig[12..16].copy_from_slice(&40u32.to_le_bytes());
    sig[20..24].copy_from_slice(&64u32.to_le_bytes());
    sig[24..28].copy_from_slice(&3u32.to_le_bytes());
    sig[32] = 15;
    sig.extend_from_slice(b"SV_Target\0");
    let mut data = vec![0u8; 36];
    data[..4].copy_from_slice(b"DXBC");
    data[24..28].copy_from_slice(&((44 + sig.len()) as u32).to_le_bytes());
    data[28..32].copy_from_slice(&1u32.to_le_bytes());
    data[32..36].copy_from_slice(&36u32.to_le_bytes());
    data.extend_from_slice(b"OSG1");
    data.extend_from_slice(&(sig.len() as u32).to_le_bytes());
    data.extend_from_slice(&sig);
    let container = Container::parse(&data).unwrap();
    let program = ir::build(&native.code, native.wave64, native.vgprs).unwrap();
    let resources = resource::track(&program, &native.layout, native.stage).unwrap();
    let raised = io::raise(&program, &native).unwrap();
    let structured = structure::structure(&program).unwrap();
    emit::emit(
        &container,
        &native,
        &program,
        &structured,
        &raised,
        &resources,
    )
    .unwrap()
    .dxil
}

#[test]
fn rejects_invalid_arguments_before_gpu_work() {
    assert!(render_triangle(VERTEX, "triangle", &[], "main", 1, 1).is_err());
    assert!(render_triangle(VERTEX, "triangle", &[1], "main", 0, 1).is_err());
    assert!(render_triangle("bad\0source", "triangle", &[1], "main", 1, 1).is_err());
}

#[test]
#[ignore = "requires a Metal GPU, Apple's converter and a built victory CLI"]
fn translated_fragment_renders_pixels() {
    let cli =
        std::env::var_os("VICTORY_TEST_CLI").expect("set VICTORY_TEST_CLI to the victory binary");
    let converter =
        std::env::var_os("VICTORY_TEST_METAL_CONVERTER").expect("set VICTORY_TEST_METAL_CONVERTER");
    let tmp = tempfile::tempdir().unwrap();
    let dxil = tmp.path().join("fragment.dxil");
    let metallib = tmp.path().join("fragment.metallib");
    std::fs::write(&dxil, fragment_dxil()).unwrap();
    let result = Command::new(cli)
        .arg("convert-metal")
        .arg(&dxil)
        .arg("--library")
        .arg(converter)
        .args(["--stage", "fragment", "--output"])
        .arg(&metallib)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "converter: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let fragment = std::fs::read(metallib).unwrap();
    // Odd width verifies the padded GPU readback stride is removed correctly.
    let pixels = render_triangle(VERTEX, "triangle", &fragment, "main", 5, 3).unwrap();
    assert_eq!(pixels.len(), 5 * 3 * 4);
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    assert!(render_triangle(VERTEX, "missing", &fragment, "main", 1, 1).is_err());
    assert!(render_triangle(VERTEX, "triangle", &fragment, "main", 0, 1).is_err());
}
