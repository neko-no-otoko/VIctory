#![cfg(target_os = "macos")]

use std::process::Command;
use victory_metal::Renderer;
use victory_shader::native::{StageIo, Usage, VsInput};
use victory_shader::{Container, HwStage, NativeProgram, emit, io, ir, resource, structure};

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

// A translated GFX10 vertex shader forwards float4 POSITION to SV_Position.
fn vertex_dxil() -> Vec<u8> {
    let native = NativeProgram {
        stage: HwStage::Vs,
        wave64: false,
        vgprs: 8,
        layout: vec![],
        used: vec![],
        io: StageIo::Vs {
            inputs: vec![VsInput {
                usage: Usage::Generic,
                row: 0,
                mask: 15,
                vgpr: 4,
            }],
            outputs: vec![],
        },
        code: [0xf800_18cfu32, 0x0706_0504, 0xbf81_0000]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect(),
    };
    let signature = |name: &[u8], system: u32| {
        let mut sig = vec![0u8; 40];
        sig[0..4].copy_from_slice(&1u32.to_le_bytes());
        sig[4..8].copy_from_slice(&8u32.to_le_bytes());
        sig[12..16].copy_from_slice(&40u32.to_le_bytes());
        sig[20..24].copy_from_slice(&system.to_le_bytes());
        sig[24..28].copy_from_slice(&3u32.to_le_bytes());
        sig[32] = 15;
        sig.extend_from_slice(name);
        sig
    };
    let input = signature(b"POSITION\0", 0);
    let output = signature(b"SV_Position\0", 1);
    let second = 40 + 8 + input.len();
    let mut data = vec![0u8; 40];
    data[..4].copy_from_slice(b"DXBC");
    data[24..28].copy_from_slice(&((second + 8 + output.len()) as u32).to_le_bytes());
    data[28..32].copy_from_slice(&2u32.to_le_bytes());
    data[32..36].copy_from_slice(&40u32.to_le_bytes());
    data[36..40].copy_from_slice(&(second as u32).to_le_bytes());
    for (tag, sig) in [(b"ISG1", input), (b"OSG1", output)] {
        data.extend_from_slice(tag);
        data.extend_from_slice(&(sig.len() as u32).to_le_bytes());
        data.extend_from_slice(&sig);
    }
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
    assert!(Renderer::new(&[], "main", &[1], "main", 1, 1).is_err());
    assert!(Renderer::new(&[1], "main", &[1], "main", 0, 1).is_err());
    assert!(Renderer::new(&[1], "bad\0name", &[1], "main", 1, 1).is_err());
}

#[test]
#[ignore = "requires a Metal GPU, Apple's converter and a built victory CLI"]
fn translated_stages_render_vertex_buffer() {
    let cli = std::env::var_os("VICTORY_TEST_CLI").expect("set VICTORY_TEST_CLI");
    let converter =
        std::env::var_os("VICTORY_TEST_METAL_CONVERTER").expect("set VICTORY_TEST_METAL_CONVERTER");
    let tmp = tempfile::tempdir().unwrap();
    let mut libs = Vec::new();
    for (stage, data) in [("vertex", vertex_dxil()), ("fragment", fragment_dxil())] {
        let dxil = tmp.path().join(format!("{stage}.dxil"));
        let metallib = tmp.path().join(format!("{stage}.metallib"));
        std::fs::write(&dxil, data).unwrap();
        let output = Command::new(&cli)
            .arg("convert-metal")
            .arg(&dxil)
            .arg("--library")
            .arg(&converter)
            .arg("--stage")
            .arg(stage)
            .arg("--output")
            .arg(&metallib)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "converter: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        libs.push(std::fs::read(metallib).unwrap());
    }
    // Odd width verifies padded GPU readback is copied into tight rows.
    let mut renderer = Renderer::new(&libs[0], "main", &libs[1], "main", 5, 3).unwrap();
    let fullscreen = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
    let outside = [[2., 2., 0., 1.], [3., 2., 0., 1.], [2., 3., 0., 1.]];
    let mut pixels = vec![0; 5 * 3 * 4];
    renderer.draw(&fullscreen, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    let two_triangles = [
        fullscreen[0],
        fullscreen[1],
        fullscreen[2],
        outside[0],
        outside[1],
        outside[2],
    ];
    renderer.draw(&two_triangles, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    renderer.draw(&outside, &mut pixels).unwrap();
    assert!(pixels.iter().all(|p| *p == 0), "{pixels:?}");
    renderer.draw(&fullscreen, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    assert!(renderer.draw(&[], &mut pixels).is_err());
    assert!(renderer.draw(&fullscreen, &mut pixels[..4]).is_err());
    assert!(Renderer::new(&libs[0], "missing", &libs[1], "main", 1, 1).is_err());
}
