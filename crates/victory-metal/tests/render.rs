#![cfg(target_os = "macos")]

use std::process::Command;
use victory_metal::{Renderer, TextureFilter};
use victory_shader::native::{StageIo, Usage, VsInput};
use victory_shader::{
    Container, HwStage, NativeProgram, ResourceType, RootParam, RootType, UsedResource, emit, io,
    ir, resource, structure,
};

// GFX10 fragments: constant color, b0-driven color, or a texture sample.
fn fragment_dxil(use_constants: bool, sampled: bool) -> Vec<u8> {
    let words: Vec<u32> = if sampled {
        vec![
            0x7e04_0200, // v_mov_b32 v2, s0 (u coordinate)
            0x7e06_02f0, // v_mov_b32 v3, 0.5 (v coordinate)
            0xbe82_0309, // s_mov_b32 s2, s9 (texture heap)
            0xbe89_0380, // s_mov_b32 s9, 0
            0xbe83_0380, // s_mov_b32 s3, 0
            0xf408_0204,
            0xfa00_0000, // s_load_dwordx4 sampler descriptor
            0xf40c_0001,
            0xfa00_0000, // s_load_dwordx8 texture descriptor
            0xf080_0f08,
            0x0040_0002, // image_sample v[0:3], v[2:3]
            0xf800_180f,
            0x0302_0100,
            0xbf81_0000,
        ]
    } else {
        let mut words = if use_constants {
            vec![0x7e00_0200] // v_mov_b32 v0, s0
        } else {
            vec![0x7e00_02ff, 0x3f80_0000]
        };
        words.extend_from_slice(&[
            0x7e02_0280,
            0x7e04_0280,
            0x7e06_02ff,
            0x3f80_0000,
            0xf800_180f,
            0x0302_0100,
            0xbf81_0000,
        ]);
        words
    };
    let root = |kind, user_data, slot| RootParam {
        root: kind,
        table_offset_dwords: 0,
        sgprs: 1,
        base_register: 0,
        user_data,
        slot,
    };
    let used = |kind, user_data, slot, dwords| UsedResource {
        kind,
        root: RootType::Table,
        table_offset_dwords: 0,
        lower_bound: 0,
        stride: 0,
        dwords,
        user_data,
        slot,
    };
    let native = NativeProgram {
        stage: HwStage::Ps,
        wave64: false,
        vgprs: 8,
        layout: if sampled {
            vec![
                root(RootType::Constants, 0, 0),
                root(RootType::Table, 8, 1),
                root(RootType::Table, 9, 2),
            ]
        } else if use_constants {
            vec![root(RootType::Constants, 0, 0)]
        } else {
            vec![]
        },
        used: if sampled {
            vec![
                used(ResourceType::Sampler, 8, 1, 4),
                used(ResourceType::Srv, 9, 2, 8),
            ]
        } else {
            vec![]
        },
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

// Check the exact automatic-layout assumption before testing the GPU binding.
fn assert_fragment_b0_layout(library: &std::ffi::OsStr, dxil: &[u8]) {
    use std::ffi::{c_char, c_int, c_void};
    type Ptr = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Location {
        kind: u32,
        space: u32,
        slot: u32,
        offset: u32,
        size: u64,
        name: *const c_char,
    }
    // SAFETY: these are Apple's documented public C API signatures. The
    // explicitly supplied library and all input bytes outlive the objects.
    unsafe {
        let lib = libloading::Library::new(library).unwrap();
        macro_rules! api {
            ($name:literal, $ty:ty) => {
                *lib.get::<$ty>(concat!($name, "\0").as_bytes()).unwrap()
            };
        }
        let create_compiler = api!("IRCompilerCreate", unsafe extern "C" fn() -> Ptr);
        let destroy_compiler = api!("IRCompilerDestroy", unsafe extern "C" fn(Ptr));
        let create_input = api!(
            "IRObjectCreateFromDXIL",
            unsafe extern "C" fn(*const u8, usize, c_int) -> Ptr
        );
        let destroy_object = api!("IRObjectDestroy", unsafe extern "C" fn(Ptr));
        let compile = api!(
            "IRCompilerAllocCompileAndLink",
            unsafe extern "C" fn(Ptr, *const c_char, Ptr, *mut Ptr) -> Ptr
        );
        let destroy_error = api!("IRErrorDestroy", unsafe extern "C" fn(Ptr));
        let create_reflection = api!("IRShaderReflectionCreate", unsafe extern "C" fn() -> Ptr);
        let destroy_reflection = api!("IRShaderReflectionDestroy", unsafe extern "C" fn(Ptr));
        let reflect = api!(
            "IRObjectGetReflection",
            unsafe extern "C" fn(Ptr, c_int, Ptr) -> bool
        );
        let count = api!(
            "IRShaderReflectionGetResourceCount",
            unsafe extern "C" fn(Ptr) -> usize
        );
        let locations = api!(
            "IRShaderReflectionGetResourceLocations",
            unsafe extern "C" fn(Ptr, *mut Location)
        );
        let compiler = create_compiler();
        let input = create_input(dxil.as_ptr(), dxil.len(), 0);
        assert!(!compiler.is_null() && !input.is_null());
        let mut error = std::ptr::null_mut();
        let compiled = compile(compiler, std::ptr::null(), input, &mut error);
        assert!(
            !compiled.is_null(),
            "converter failed to compile reflection fixture"
        );
        let reflection = create_reflection();
        assert!(!reflection.is_null() && reflect(compiled, 2, reflection)); // fragment
        assert_eq!(count(reflection), 1);
        let mut location = Location::default();
        locations(reflection, &mut location);
        assert_eq!(
            (
                location.kind,
                location.space,
                location.slot,
                location.offset,
                location.size
            ),
            (2, 0, 0, 0, 24)
        );
        destroy_reflection(reflection);
        destroy_object(compiled);
        if !error.is_null() {
            destroy_error(error);
        }
        destroy_object(input);
        destroy_compiler(compiler);
    }
}

#[test]
fn rejects_invalid_arguments_before_gpu_work() {
    assert!(Renderer::new(&[], "main", &[1], "main", 1, 1, 0, false).is_err());
    assert!(Renderer::new(&[1], "main", &[1], "main", 0, 1, 0, false).is_err());
    assert!(Renderer::new(&[1], "bad\0name", &[1], "main", 1, 1, 0, false).is_err());
    assert!(Renderer::new(&[1], "main", &[1], "main", 1, 1, 1, false).is_err());
}

#[test]
#[ignore = "requires a Metal GPU, Apple's converter and a built victory CLI"]
fn translated_stages_render_vertex_buffer() {
    let cli = std::env::var_os("VICTORY_TEST_CLI").expect("set VICTORY_TEST_CLI");
    let converter =
        std::env::var_os("VICTORY_TEST_METAL_CONVERTER").expect("set VICTORY_TEST_METAL_CONVERTER");
    let uniform_dxil = fragment_dxil(true, false);
    let sample_dxil = fragment_dxil(true, true);
    assert_fragment_b0_layout(&converter, &uniform_dxil);
    assert_fragment_b0_layout(&converter, &sample_dxil);
    let tmp = tempfile::tempdir().unwrap();
    let mut libs = Vec::new();
    for (stage, data) in [
        ("vertex", vertex_dxil()),
        ("fragment", fragment_dxil(false, false)),
        ("fragment", uniform_dxil),
        ("fragment", sample_dxil),
    ] {
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
    let mut renderer = Renderer::new(&libs[0], "main", &libs[1], "main", 5, 3, 0, false).unwrap();
    let fullscreen = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
    let outside = [[2., 2., 0., 1.], [3., 2., 0., 1.], [2., 3., 0., 1.]];
    let mut pixels = vec![0; 5 * 3 * 4];
    renderer.draw(&fullscreen, &[], &mut pixels).unwrap();
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
    renderer.draw(&two_triangles, &[], &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    renderer.draw(&outside, &[], &mut pixels).unwrap();
    assert!(pixels.iter().all(|p| *p == 0), "{pixels:?}");
    renderer.draw(&fullscreen, &[], &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    assert!(renderer.draw(&[], &[], &mut pixels).is_err());
    assert!(renderer.draw(&fullscreen, &[], &mut pixels[..4]).is_err());
    assert!(Renderer::new(&libs[0], "missing", &libs[1], "main", 1, 1, 0, false).is_err());
    assert!(
        renderer
            .set_fragment_texture(1, 1, &[0; 4], TextureFilter::Nearest)
            .is_err()
    );

    let mut uniform_renderer =
        Renderer::new(&libs[0], "main", &libs[2], "main", 5, 3, 4, false).unwrap();
    let red = [1.0f32.to_bits(), 0, 0, 0];
    let black = [0.0f32.to_bits(), 0, 0, 0];
    uniform_renderer
        .draw(&fullscreen, &red, &mut pixels)
        .unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    uniform_renderer
        .draw(&fullscreen, &black, &mut pixels)
        .unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [0, 0, 0, 255]),
        "{pixels:?}"
    );
    uniform_renderer
        .draw(&fullscreen, &red, &mut pixels)
        .unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    assert!(
        uniform_renderer
            .draw(&fullscreen, &red[..3], &mut pixels)
            .is_err()
    );

    let mut textured = Renderer::new(&libs[0], "main", &libs[3], "main", 5, 3, 12, true).unwrap();
    let mut constants = [0u32; 12];
    constants[0] = 0.25f32.to_bits();
    assert!(textured.draw(&fullscreen, &constants, &mut pixels).is_err());
    assert!(
        textured
            .set_fragment_texture(2, 1, &[1, 2], TextureFilter::Nearest)
            .is_err()
    );
    let texels = [255, 0, 0, 255, 0, 255, 0, 255];
    textured
        .set_fragment_texture(2, 1, &texels, TextureFilter::Nearest)
        .unwrap();
    textured.draw(&fullscreen, &constants, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{pixels:?}"
    );
    constants[0] = 0.75f32.to_bits();
    textured.draw(&fullscreen, &constants, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [0, 255, 0, 255]),
        "{pixels:?}"
    );
    textured
        .set_fragment_texture(2, 1, &texels, TextureFilter::Linear)
        .unwrap();
    constants[0] = 0.5f32.to_bits();
    textured.draw(&fullscreen, &constants, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| (127..=128).contains(&p[0])
            && (127..=128).contains(&p[1])
            && p[2..] == [0, 255]),
        "{pixels:?}"
    );
    let swapped = [0, 0, 255, 255, 255, 0, 0, 255];
    textured
        .set_fragment_texture(2, 1, &swapped, TextureFilter::Nearest)
        .unwrap();
    constants[0] = 0.25f32.to_bits();
    textured.draw(&fullscreen, &constants, &mut pixels).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == [0, 0, 255, 255]),
        "{pixels:?}"
    );
}
