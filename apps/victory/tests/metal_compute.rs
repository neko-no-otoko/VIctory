#![cfg(target_os = "macos")]

use std::process::Command;
use victory_shader::native::{ComputeIo, StageIo};
use victory_shader::{
    Container, HwStage, NativeProgram, ResourceType, RootParam, RootType, UsedResource, emit, io,
    ir, resource, structure,
};

/// No game data: a one-thread GFX10 compute shader computes 41 + 1 and stores
/// it through descriptor heap index zero. Root user-data dword 4 selects the
/// descriptor table. The corresponding Metal binding is checked in the probe.
fn emitted_dxil() -> Vec<u8> {
    let words: &[u32] = &[
        0xBE85_0380, // s_mov_b32 s5, 0
        0xF408_0202,
        0xFA00_0000, // s_load_dwordx4 s[8:11], s[4:5], 0
        0x7E00_02FF,
        41,          // v_mov_b32 v0, 41
        0x4A00_0081, // v_add_nc_u32 v0, 1, v0
        0xE070_0000,
        0x8002_0000, // buffer_store_dword v0, off, s[8:11], 0
        0xBF81_0000, // s_endpgm
    ];
    let native = NativeProgram {
        stage: HwStage::Cs,
        wave64: false,
        vgprs: 8,
        layout: vec![RootParam {
            root: RootType::Table,
            table_offset_dwords: 0,
            sgprs: 1,
            base_register: 0,
            user_data: 4,
            slot: 1,
        }],
        used: vec![UsedResource {
            kind: ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: 0,
            lower_bound: 0,
            stride: 0,
            dwords: 4,
            user_data: 4,
            slot: 1,
        }],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1,
            threads: [1, 1, 1],
        }),
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    };
    let program = ir::build(&native.code, native.wave64, native.vgprs).unwrap();
    let resources = resource::track(&program, &native.layout, native.stage).unwrap();
    let raised = io::raise(&program, &native).unwrap();
    let structured = structure::structure(&program).unwrap();
    let mut empty_container = vec![0u8; 0x20];
    empty_container[..4].copy_from_slice(b"DXBC");
    empty_container[0x18..0x1c].copy_from_slice(&0x20u32.to_le_bytes());
    let container = Container::parse(&empty_container).unwrap();
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
#[ignore = "requires Apple Metal Shader Converter, clang and a Metal GPU"]
fn synthetic_gfx10_store_executes_on_metal() {
    let library = std::env::var_os("VICTORY_TEST_METAL_CONVERTER")
        .expect("set VICTORY_TEST_METAL_CONVERTER to Apple's libmetalirconverter.dylib");
    let tmp = tempfile::tempdir().unwrap();
    let dxil = tmp.path().join("shader.dxil");
    let metallib = tmp.path().join("shader.metallib");
    let probe = tmp.path().join("metal_compute_probe");
    std::fs::write(&dxil, emitted_dxil()).unwrap();
    let converted = Command::new(env!("CARGO_BIN_EXE_victory"))
        .arg("convert-metal")
        .arg(&dxil)
        .arg("--library")
        .arg(&library)
        .args(["--stage", "compute", "--output"])
        .arg(&metallib)
        .output()
        .unwrap();
    assert!(
        converted.status.success(),
        "converter: {}",
        String::from_utf8_lossy(&converted.stderr)
    );
    let compiled = Command::new("xcrun")
        .args([
            "clang",
            "-fobjc-arc",
            "-framework",
            "Foundation",
            "-framework",
            "Metal",
        ])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/metal_compute_probe.m"
        ))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "clang: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = Command::new(&probe)
        .arg(&dxil)
        .arg(&metallib)
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "probe: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("result=42"));
}
