//! Opt-in end-to-end console graphics submission on Metal.
use crate::object::{self, HRESULT, Iid, S_OK, This, slot};
use std::ffi::c_void;

#[repr(C)]
struct HeapProperties {
    kind: u32,
    cpu_pages: u32,
    pool: u32,
    creation_nodes: u32,
    visible_nodes: u32,
}
#[repr(C)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth: u16,
    mips: u16,
    format: u32,
    samples: u32,
    quality: u32,
    layout: u32,
    flags: u32,
}
#[repr(C)]
struct HeapDesc {
    kind: u32,
    count: u32,
    flags: u32,
    nodes: u32,
    placement: [u64; 2],
}
#[repr(C)]
struct QueueDesc {
    kind: u32,
    priority: i32,
    flags: u32,
    nodes: u32,
}
#[repr(C)]
struct StreamDesc {
    size: usize,
    stream: *const u8,
}
#[repr(C)]
struct InputElement {
    name: *const u8,
    semantic_index: u32,
    format: u32,
    slot: u32,
    offset: u32,
    class: u32,
    step: u32,
}
#[repr(C)]
struct VertexView {
    address: u64,
    size: u32,
    stride: u32,
}
#[repr(C)]
struct Footprint {
    offset: u64,
    format: u32,
    width: u32,
    height: u32,
    depth: u32,
    pitch: u32,
}
#[repr(C)]
struct CopyLocation {
    resource: This,
    kind: u32,
    footprint: Footprint,
}
#[repr(C)]
struct Barrier {
    kind: u32,
    flags: u32,
    resource: This,
    reserved: [u64; 2],
}

fn u32_at(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn signature(name: &[u8], system: u32) -> Vec<u8> {
    let mut sig = vec![0u8; 40];
    u32_at(&mut sig, 0, 1);
    u32_at(&mut sig, 4, 8);
    u32_at(&mut sig, 12, 40);
    u32_at(&mut sig, 20, system);
    u32_at(&mut sig, 24, 3);
    sig[32] = 15;
    sig.extend_from_slice(name);
    sig
}

// A real GFX10 FSSM program packaged in XDAT/DXBC, with the same stage
// instructions and signatures as victory-metal's translated render test.
fn shader(vertex: bool) -> Vec<u8> {
    let words: &[u32] = if vertex {
        &[0xf800_18cf, 0x0706_0504, 0xbf81_0000]
    } else {
        &[
            0x7e00_0202, // v_mov_b32 v0, s2
            0x7e02_0280,
            0x7e04_0280,
            0x7e06_02ff,
            0x3f80_0000,
            0xf800_180f,
            0x0302_0100,
            0xbf81_0000,
        ]
    };
    let mut payload = vec![0u8; 0x68];
    u64_at(&mut payload, 0, 6);
    for at in [0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] {
        u64_at(&mut payload, at, 0xdead_beef_dead_beef);
    }
    let record = payload.len();
    u64_at(
        &mut payload,
        if vertex { 0x28 } else { 0x30 },
        record as u64,
    );
    let stage = record + 0x54;
    let stage_len = if vertex { 0x18c } else { 0x14c + 8 * 404 };
    let code_at = stage + stage_len;
    payload.resize(code_at, 0);
    u64_at(&mut payload, record, code_at as u64);
    u64_at(&mut payload, record + 8, (words.len() * 4) as u64);
    u32_at(&mut payload, record + 0x14, 0x20); // wave32
    u32_at(&mut payload, record + 0x50, 8);
    if vertex {
        payload[stage + 0x24..stage + 0x29].copy_from_slice(&[6, 0, 15, 4, 0]);
        u32_at(&mut payload, stage + 0xc4, 1);
    } else {
        for mrt in 0..8 {
            u32_at(&mut payload, stage + 0x14c + mrt * 404, u32::MAX);
        }
    }
    if !vertex {
        // One root-constants parameter at slot 3, with s2 holding dword 2.
        u64_at(
            &mut payload,
            record + 0x28,
            (code_at + words.len() * 4) as u64,
        );
        u32_at(&mut payload, record + 0x38, 1);
    }
    for word in words {
        payload.extend_from_slice(&word.to_le_bytes());
    }
    if !vertex {
        let root = payload.len();
        payload.resize(root + 20, 0);
        u32_at(&mut payload, root + 8, 3); // three SGPRs
        payload[root + 17] = 3; // root parameter slot
    }
    let mut fssm = b"FSSM".to_vec();
    fssm.extend_from_slice(&0x101u32.to_le_bytes());
    fssm.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    fssm.extend_from_slice(&payload);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&fssm, 6);
    let mut blob = b"XBCH".to_vec();
    blob.extend_from_slice(&3u32.to_le_bytes());
    blob.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    blob.extend_from_slice(&(fssm.len() as u64).to_le_bytes());
    blob.extend_from_slice(&packed);
    let mut dxil = vec![0u8; 0x18];
    dxil.extend_from_slice(b"XDAT");
    for value in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] {
        dxil.extend_from_slice(&value.to_le_bytes());
    }
    dxil.extend_from_slice(&blob);
    let mut parts = vec![(b"DXIL", dxil)];
    if vertex {
        parts.push((b"ISG1", signature(b"POSITION\0", 0)));
    }
    parts.push((
        b"OSG1",
        if vertex {
            signature(b"SV_Position\0", 1)
        } else {
            signature(b"SV_Target\0", 64)
        },
    ));
    let mut container = vec![0u8; 32 + 4 * parts.len()];
    container[..4].copy_from_slice(b"DXBC");
    u32_at(&mut container, 20, 1);
    u32_at(&mut container, 28, parts.len() as u32);
    for (index, (tag, data)) in parts.into_iter().enumerate() {
        let offset = container.len() as u32;
        u32_at(&mut container, 32 + index * 4, offset);
        container.extend_from_slice(tag);
        container.extend_from_slice(&(data.len() as u32).to_le_bytes());
        container.extend_from_slice(&data);
    }
    let size = container.len() as u32;
    u32_at(&mut container, 24, size);
    container
}

#[test]
#[ignore = "requires Apple Metal Shader Converter and a Metal GPU"]
fn console_triangle_draw_preserves_target_and_copies_to_padded_buffer() {
    assert!(
        std::env::var_os("VICTORY_METAL_CONVERTER").is_some(),
        "set VICTORY_METAL_CONVERTER"
    );
    let null: This = std::ptr::null();
    let release = |o: This| slot::<extern "system" fn(This) -> u32>(o, 0x10)(o);
    let mut dev = null;
    assert_eq!(
        crate::device::create(
            null.cast(),
            &crate::device::CHECKED_VERSION,
            &crate::device::IID,
            &mut dev
        ),
        S_OK
    );
    let resource_iid = object::guid(
        0x696442be,
        0xa72e,
        0x4059,
        [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad],
    );
    let props = HeapProperties {
        kind: 1,
        cpu_pages: 0,
        pool: 0,
        creation_nodes: 0,
        visible_nodes: 0,
    };
    let make_resource = slot::<
        extern "system" fn(
            This,
            *const c_void,
            u32,
            *const c_void,
            u32,
            *const c_void,
            *const Iid,
            *mut This,
        ) -> HRESULT,
    >(dev, 0xd8);
    let make = |desc: &ResourceDesc, state| {
        let mut out = null;
        assert_eq!(
            make_resource(
                dev,
                (&raw const props).cast(),
                0,
                (desc as *const ResourceDesc).cast(),
                state,
                null.cast(),
                &resource_iid,
                &mut out
            ),
            S_OK
        );
        out
    };
    let buffer = |width| ResourceDesc {
        dimension: 1,
        alignment: 0,
        width,
        height: 1,
        depth: 1,
        mips: 1,
        format: 0,
        samples: 1,
        quality: 0,
        layout: 1,
        flags: 0,
    };
    let vb = make(&buffer(96), 0);
    let readback = make(&buffer(2048), 0);
    let texture_desc = ResourceDesc {
        dimension: 3,
        alignment: 0,
        width: 5,
        height: 4,
        depth: 1,
        mips: 1,
        format: 28,
        samples: 1,
        quality: 0,
        layout: 1,
        flags: 1,
    };
    let texture = make(&texture_desc, 4);
    let clear_only = make(&texture_desc, 4);
    let map = |resource| {
        let mut ptr = std::ptr::null_mut();
        assert_eq!(
            slot::<extern "system" fn(This, u32, *const c_void, *mut *mut u8) -> HRESULT>(
                resource, 0x40
            )(resource, 0, null.cast(), &mut ptr),
            S_OK
        );
        assert!(!ptr.is_null());
        ptr
    };
    let vertices = [
        [-1.0f32, -1.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 1.0],
        [-0.5, 1.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, 1.0],
        [0.5, 1.0, 0.0, 1.0],
    ];
    let vertex_data = map(vb);
    let output = map(readback);
    let target = map(texture);
    let clear_only_target = map(clear_only);
    // The mapped target is initialized before recording, and readback is a sentinel.
    unsafe {
        std::ptr::copy_nonoverlapping(vertices.as_ptr().cast::<u8>(), vertex_data, 96);
        std::ptr::write_bytes(output, 0x77, 2048);
        for y in 0..4 {
            std::ptr::write_bytes(target.add(y * 256), 0x30 + y as u8, 20);
            std::ptr::write_bytes(clear_only_target.add(y * 256), 0x66, 20);
        }
    }
    let vertex_address = slot::<extern "system" fn(This) -> u64>(vb, 0x58)(vb);
    let heap_iid = object::guid(
        0x8efb471d,
        0x616c,
        0x4f49,
        [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
    );
    let heap_desc = HeapDesc {
        kind: 2,
        count: 2,
        flags: 0,
        nodes: 0,
        placement: [0; 2],
    };
    let mut heap = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x70
        )(dev, (&raw const heap_desc).cast(), &heap_iid, &mut heap),
        S_OK
    );
    let handle = slot::<extern "system" fn(This) -> u64>(heap, 0x48)(heap);
    slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0xa0)(
        dev,
        texture,
        null.cast(),
        handle,
    );
    slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0xa0)(
        dev,
        clear_only,
        null.cast(),
        handle + 128,
    );

    let vs = shader(true);
    let ps = shader(false);
    let host = victory_shader::emit::translate(&ps).unwrap();
    let library = std::env::var_os("VICTORY_METAL_CONVERTER").unwrap();
    assert!(
        victory_metal::converter::convert_fragment_with_b0(
            std::path::Path::new(&library),
            &host.dxil,
            false
        )
        .is_err()
    );
    let element = InputElement {
        name: b"POSITION\0".as_ptr(),
        semantic_index: 0,
        format: 2,
        slot: 0,
        offset: 0,
        class: 0,
        step: 0,
    };
    let mut stream = vec![0u8; 24 + 24 + 48 + 56 + 24 + 8 + 40];
    for (at, kind) in [
        (0, 1),
        (24, 2),
        (48, 10),
        (96, 11),
        (152, 12),
        (176, 14),
        (184, 15),
    ] {
        u32_at(&mut stream, at, kind);
    }
    u64_at(&mut stream, 8, vs.as_ptr() as u64);
    u64_at(&mut stream, 16, vs.len() as u64);
    u64_at(&mut stream, 32, ps.as_ptr() as u64);
    u64_at(&mut stream, 40, ps.len() as u64);
    u32_at(&mut stream, 52, 3);
    u32_at(&mut stream, 56, 1);
    u32_at(&mut stream, 76, 1);
    u64_at(&mut stream, 160, (&raw const element) as u64);
    u32_at(&mut stream, 168, 1);
    u32_at(&mut stream, 180, 3);
    u32_at(&mut stream, 188, 28);
    u32_at(&mut stream, 220, 1);
    let stream_desc = StreamDesc {
        size: stream.len(),
        stream: stream.as_ptr(),
    };
    let pipeline_iid = object::guid(
        0x765a30f3,
        0xf624,
        0x4c6f,
        [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
    );
    let mut pipeline = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x2b0
        )(
            dev,
            (&raw const stream_desc).cast(),
            &pipeline_iid,
            &mut pipeline
        ),
        S_OK
    );
    let allocator_iid = object::guid(
        0x6102dee4,
        0xaf59,
        0x4b09,
        [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24],
    );
    let list_iid = object::guid(
        0x7116d91c,
        0xe7e4,
        0x47ce,
        [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5],
    );
    let mut allocator = null;
    assert_eq!(
        slot::<extern "system" fn(This, u32, *const Iid, *mut This) -> HRESULT>(dev, 0x48)(
            dev,
            0,
            &allocator_iid,
            &mut allocator
        ),
        S_OK
    );
    let mut list = null;
    assert_eq!(
        slot::<extern "system" fn(This, u32, u32, This, This, *const Iid, *mut This) -> HRESULT>(
            dev, 0x60
        )(dev, 0, 0, allocator, pipeline, &list_iid, &mut list),
        S_OK
    );
    let view = VertexView {
        address: vertex_address,
        size: 96,
        stride: 16,
    };
    let bind = || {
        slot::<extern "system" fn(This, u32)>(list, 0xa0)(list, 4);
        slot::<extern "system" fn(This, u32, *const f32)>(list, 0xa8)(
            list,
            1,
            [0.0, 0.0, 5.0, 4.0, 0.0, 1.0].as_ptr(),
        );
        slot::<extern "system" fn(This, u32, *const i32)>(list, 0xb0)(
            list,
            1,
            [0, 0, 5, 4].as_ptr(),
        );
        slot::<extern "system" fn(This, u32, u32, *const VertexView)>(list, 0x180)(
            list, 0, 1, &view,
        );
        slot::<extern "system" fn(This, u32, *const u64, i32, *const u64)>(list, 0x190)(
            list,
            1,
            &handle,
            0,
            null.cast(),
        );
    };
    let draw = slot::<extern "system" fn(This, u32, u32, u32, u32)>(list, 0x60);
    let close = slot::<extern "system" fn(This) -> HRESULT>(list, 0x48);
    let reset = slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50);
    bind();
    draw(list, 3, 1, 0, 0); // Missing required root dword.
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    bind();
    slot::<extern "system" fn(This, u32, u32, u32)>(list, 0x130)(list, 2, 1, 0);
    draw(list, 3, 1, 0, 0); // Unsupported root slot.
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    let clear =
        slot::<extern "system" fn(This, u64, *const [f32; 4], u32, *const [i32; 4])>(list, 0x1a0);
    clear(list, handle, null.cast(), 0, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    let color = [0.0, 0.0, 0.0, 1.0];
    clear(list, handle, &color, 1, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    clear(list, handle, &color, 1, [[2, 0, 1, 1]].as_ptr());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    let wrong_state = Barrier {
        kind: 0,
        flags: 0,
        resource: texture,
        reserved: [u64::from(u32::MAX) | (4u64 << 32), 0x800],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &wrong_state);
    clear(list, handle, &color, 0, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, pipeline), S_OK);
    let mut background = [0.25, 0.5, 2.0, -1.0];
    clear(list, handle, &background, 0, null.cast());
    clear(list, handle + 128, &background, 0, null.cast());
    background = [1.0; 4];
    assert_eq!(background, [1.0; 4]);
    bind();
    let set_constants = slot::<extern "system" fn(This, u32, u32, *const c_void, u32)>(list, 0x140);
    let mut red = 0.25f32;
    set_constants(list, 3, 1, (&raw const red).cast(), 2);
    slot::<extern "system" fn(This, u32, u32, u32, u32)>(list, 0x60)(list, 3, 1, 0, 0);
    // Compute root state is separate from the graphics root state.
    slot::<extern "system" fn(This, u32, u32, u32)>(list, 0x128)(list, 3, 0xdeadbeef, 2);
    red = 0.75;
    slot::<extern "system" fn(This, u32, u32, u32)>(list, 0x130)(list, 3, red.to_bits(), 2);
    slot::<extern "system" fn(This, u32, u32, u32, u32)>(list, 0x60)(list, 3, 1, 3, 0);
    // Neither the source memory nor subsequent root state may alter recorded draws.
    red = 0.5;
    set_constants(list, 3, 1, (&raw const red).cast(), 2);
    red = 1.0;
    assert_eq!(red, 1.0);
    let transition = Barrier {
        kind: 0,
        flags: 0,
        resource: texture,
        reserved: [u64::from(u32::MAX) | (4u64 << 32), 0x800],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &transition);
    let footprint = |offset| Footprint {
        offset,
        format: 28,
        width: 5,
        height: 4,
        depth: 1,
        pitch: 256,
    };
    let dst = CopyLocation {
        resource: readback,
        kind: 1,
        footprint: footprint(0),
    };
    let before_clear = CopyLocation {
        resource: readback,
        kind: 1,
        footprint: footprint(1024),
    };
    let src = CopyLocation {
        resource: texture,
        kind: 0,
        footprint: Footprint {
            offset: 0,
            format: 0,
            width: 0,
            height: 0,
            depth: 0,
            pitch: 0,
        },
    };
    let copy = slot::<
        extern "system" fn(
            This,
            *const CopyLocation,
            u32,
            u32,
            u32,
            *const CopyLocation,
            *const c_void,
        ),
    >(list, 0x80);
    copy(list, &before_clear, 0, 0, 0, &src, null.cast());
    let to_target = Barrier {
        kind: 0,
        flags: 0,
        resource: texture,
        reserved: [u64::from(u32::MAX) | (0x800u64 << 32), 4],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &to_target);
    let mut rectangles = [[3, 2, 4, 3], [-2, -2, 1, 1], [9, 9, 12, 12]];
    let mut clear_color = [0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    clear(list, handle, &clear_color, 3, rectangles.as_ptr());
    // An entirely clipped rectangle must not be mistaken for a full clear.
    clear(list, handle, &color, 1, [[9, 9, 12, 12]].as_ptr());
    rectangles[0] = [0, 0, 5, 4];
    clear_color = [1.0; 4];
    assert_eq!(rectangles[0], [0, 0, 5, 4]);
    assert_eq!(clear_color, [1.0; 4]);
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &transition);
    copy(list, &dst, 0, 0, 0, &src, null.cast());
    assert_eq!(
        slot::<extern "system" fn(This) -> HRESULT>(list, 0x48)(list),
        S_OK
    );
    for y in 0..4 {
        assert_eq!(unsafe { *output.add(y * 256) }, 0x77);
        assert_eq!(unsafe { *target.add(y * 256) }, 0x30 + y as u8);
        assert_eq!(unsafe { *clear_only_target.add(y * 256) }, 0x66);
        assert_eq!(unsafe { *output.add(1024 + y * 256) }, 0x77);
    }
    // The recorded commands own every resource after the caller releases it.
    release(vb);
    release(texture);
    assert_eq!(release(clear_only), 1, "only the recorded clear retains it");
    release(readback);
    release(heap);
    release(pipeline);
    let queue_iid = object::guid(
        0x0ec870a6,
        0x5d7e,
        0x4c22,
        [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed],
    );
    let queue_desc = QueueDesc {
        kind: 0,
        priority: 0,
        flags: 0,
        nodes: 0,
    };
    let mut queue = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x40
        )(dev, (&raw const queue_desc).cast(), &queue_iid, &mut queue),
        S_OK
    );
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    let mut fence = null;
    assert_eq!(
        slot::<extern "system" fn(This, u64, u32, *const Iid, *mut This) -> HRESULT>(dev, 0x120)(
            dev,
            0,
            0,
            &crate::fence::FENCE_IIDS[0],
            &mut fence
        ),
        S_OK
    );
    assert_eq!(
        slot::<extern "system" fn(This, This, u64) -> HRESULT>(queue, 0x70)(queue, fence, 1),
        S_OK
    );
    // The list retains the readback buffer through inspection.
    let pixel =
        |x: usize, y: usize| unsafe { std::slice::from_raw_parts(output.add(y * 256 + x * 4), 4) };
    assert_eq!(pixel(1, 2), &[64, 0, 0, 255]);
    assert_eq!(pixel(3, 2), &[128, 0, 255, 0]);
    assert_eq!(pixel(0, 0), &[128, 0, 255, 0]);
    assert_eq!(pixel(2, 0), &[64, 128, 255, 0]);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(target.add(2 * 256 + 4), 4) },
        &[64, 0, 0, 255]
    );
    assert_eq!(
        unsafe { std::slice::from_raw_parts(target.add(2 * 4), 4) },
        &[64, 128, 255, 0]
    );
    let before = |x: usize, y: usize| unsafe {
        std::slice::from_raw_parts(output.add(1024 + y * 256 + x * 4), 4)
    };
    assert_eq!(before(1, 2), &[64, 0, 0, 255]);
    assert_eq!(before(3, 2), &[191, 0, 0, 255]);
    assert_eq!(before(0, 0), &[64, 128, 255, 0]);
    assert_eq!(
        unsafe { std::slice::from_raw_parts(clear_only_target.add(3 * 256 + 4 * 4), 4) },
        &[64, 128, 255, 0],
        "clear-only texture must survive until submission"
    );
    for y in 0..4 {
        assert_eq!(
            unsafe { *output.add(y * 256 + 20) },
            0x77,
            "row padding must not be overwritten"
        );
        assert_eq!(unsafe { *output.add(1024 + y * 256 + 20) }, 0x77);
    }
    // Textures explicitly transitioned to COPY_SOURCE do not decay like buffers.
    slot::<extern "system" fn(This) -> u32>(texture, 0x08)(texture);
    let mut state_heap = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x70
        )(
            dev,
            (&raw const heap_desc).cast(),
            &heap_iid,
            &mut state_heap
        ),
        S_OK
    );
    let state_handle = slot::<extern "system" fn(This) -> u64>(state_heap, 0x48)(state_heap);
    slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0xa0)(
        dev,
        texture,
        null.cast(),
        state_handle,
    );
    let reset = slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50);
    let close = slot::<extern "system" fn(This) -> HRESULT>(list, 0x48);
    let execute = slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50);
    let signal = slot::<extern "system" fn(This, This, u64) -> HRESULT>(queue, 0x70);
    let completed = slot::<extern "system" fn(This) -> u64>(fence, 0x40);
    assert_eq!(reset(list, allocator, null), S_OK);
    // Recording cannot know the queue's current state; submission must reject
    // clearing while the target is still COPY_SOURCE.
    let mut rejected_queue = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x40
        )(
            dev,
            (&raw const queue_desc).cast(),
            &queue_iid,
            &mut rejected_queue,
        ),
        S_OK
    );
    clear(list, state_handle, &color, 0, null.cast());
    assert_eq!(close(list), S_OK);
    release(state_heap);
    execute(rejected_queue, 1, &list);
    assert_eq!(signal(rejected_queue, fence, 2), object::E_NOTIMPL);
    assert_eq!(completed(fence), 1);
    release(rejected_queue);
    assert_eq!(reset(list, allocator, null), S_OK);
    let back = Barrier {
        kind: 0,
        flags: 0,
        resource: texture,
        reserved: [u64::from(u32::MAX) | (0x800u64 << 32), 4],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &back);
    assert_eq!(close(list), S_OK);
    execute(queue, 1, &list);
    assert_eq!(signal(queue, fence, 2), S_OK);
    assert_eq!(reset(list, allocator, null), S_OK);
    // The repeated before-state is wrong; a submission must fail, not fake decay.
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &back);
    assert_eq!(close(list), S_OK);
    execute(queue, 1, &list);
    assert_eq!(signal(queue, fence, 3), object::E_NOTIMPL);
    assert_eq!(completed(fence), 2);
    assert_eq!(reset(list, allocator, null), S_OK);
    release(texture);

    // Unsupported rasterizer state must poison the list instead of drawing.
    u32_at(&mut stream, 56, 2); // back-face culling
    let mut unsupported = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x2b0
        )(
            dev,
            (&raw const stream_desc).cast(),
            &pipeline_iid,
            &mut unsupported
        ),
        S_OK
    );
    slot::<extern "system" fn(This, This)>(list, 0xc8)(list, unsupported);
    assert_eq!(close(list), object::E_NOTIMPL);
    release(unsupported);
    release(fence);
    release(list);
    release(allocator);
    release(queue);
    release(dev);
}

#[test]
#[ignore = "requires a Metal GPU"]
fn console_clear_without_graphics_pipeline() {
    let null: This = std::ptr::null();
    let release = |o: This| slot::<extern "system" fn(This) -> u32>(o, 0x10)(o);
    let mut dev = null;
    assert_eq!(crate::device::create(null.cast(), &crate::device::CHECKED_VERSION,
                                     &crate::device::IID, &mut dev), S_OK);
    let props = HeapProperties { kind: 1, cpu_pages: 0, pool: 0, creation_nodes: 0, visible_nodes: 0 };
    let desc = ResourceDesc {
        dimension: 3, alignment: 0, width: 3, height: 2, depth: 1, mips: 1,
        format: 28, samples: 1, quality: 0, layout: 1, flags: 1,
    };
    let resource_iid = object::guid(0x696442be, 0xa72e, 0x4059,
                                    [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
    let mut texture = null;
    assert_eq!(slot::<extern "system" fn(This, *const c_void, u32, *const c_void, u32,
        *const c_void, *const Iid, *mut This) -> HRESULT>(dev, 0xd8)(
            dev, (&raw const props).cast(), 0, (&raw const desc).cast(), 4, null.cast(),
            &resource_iid, &mut texture), S_OK);
    let mut pixels = std::ptr::null_mut();
    assert_eq!(slot::<extern "system" fn(This, u32, *const c_void, *mut *mut u8) -> HRESULT>(
        texture, 0x40)(texture, 0, null.cast(), &mut pixels), S_OK);
    assert!(!pixels.is_null());
    unsafe { std::ptr::write_bytes(pixels, 0x55, 512) };
    let heap_iid = object::guid(0x8efb471d, 0x616c, 0x4f49,
                                [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51]);
    let heap_desc = HeapDesc { kind: 2, count: 1, flags: 0, nodes: 0, placement: [0; 2] };
    let mut heap = null;
    assert_eq!(slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
        dev, 0x70)(dev, (&raw const heap_desc).cast(), &heap_iid, &mut heap), S_OK);
    let handle = slot::<extern "system" fn(This) -> u64>(heap, 0x48)(heap);
    slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0xa0)(
        dev, texture, null.cast(), handle);
    let allocator_iid = object::guid(0x6102dee4, 0xaf59, 0x4b09,
                                     [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
    let list_iid = object::guid(0x7116d91c, 0xe7e4, 0x47ce,
                                [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5]);
    let mut allocator = null;
    assert_eq!(slot::<extern "system" fn(This, u32, *const Iid, *mut This) -> HRESULT>(
        dev, 0x48)(dev, 0, &allocator_iid, &mut allocator), S_OK);
    let mut list = null;
    assert_eq!(slot::<extern "system" fn(This, u32, u32, This, This, *const Iid, *mut This)
        -> HRESULT>(dev, 0x60)(dev, 0, 0, allocator, null, &list_iid, &mut list), S_OK);
    let clear = slot::<extern "system" fn(This, u64, *const [f32; 4], u32, *const [i32; 4])>(
        list, 0x1a0);
    let close = slot::<extern "system" fn(This) -> HRESULT>(list, 0x48);
    let reset = slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50);
    clear(list, handle, null.cast(), 0, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    clear(list, handle, &[1.0; 4], 1, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    clear(list, handle, &[1.0; 4], 1, [[2, 0, 1, 1]].as_ptr());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    let to_copy = Barrier {
        kind: 0, flags: 0, resource: texture,
        reserved: [u64::from(u32::MAX) | (4u64 << 32), 0x800],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &to_copy);
    clear(list, handle, &[1.0; 4], 0, null.cast());
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    clear(list, handle, &[1.0, 0.0, 0.0, 1.0], 0, null.cast());
    clear(list, handle, &[0.0, 1.0, 0.0, 1.0], 1, [[1, 0, 3, 1]].as_ptr());
    assert_eq!(close(list), S_OK);
    assert_eq!(unsafe { *pixels }, 0x55, "recording must not execute");
    assert_eq!(release(texture), 2, "two recorded clears retain the texture");
    release(heap);
    let queue_iid = object::guid(0x0ec870a6, 0x5d7e, 0x4c22,
                                 [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);
    let queue_desc = QueueDesc { kind: 0, priority: 0, flags: 0, nodes: 0 };
    let mut queue = null;
    assert_eq!(slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
        dev, 0x40)(dev, (&raw const queue_desc).cast(), &queue_iid, &mut queue), S_OK);
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    let mut fence = null;
    assert_eq!(slot::<extern "system" fn(This, u64, u32, *const Iid, *mut This) -> HRESULT>(
        dev, 0x120)(dev, 0, 0, &crate::fence::FENCE_IIDS[0], &mut fence), S_OK);
    assert_eq!(slot::<extern "system" fn(This, This, u64) -> HRESULT>(queue, 0x70)(
        queue, fence, 1), S_OK);
    let pixel = |x: usize, y: usize| unsafe {
        std::slice::from_raw_parts(pixels.add(y * 256 + x * 4), 4)
    };
    assert_eq!(pixel(0, 0), &[255, 0, 0, 255]);
    assert_eq!(pixel(1, 0), &[0, 255, 0, 255]);
    assert_eq!(pixel(2, 1), &[255, 0, 0, 255]);
    assert_eq!(unsafe { *pixels.add(12) }, 0x55);
    release(fence);
    release(queue);
    release(list);
    release(allocator);
    release(dev);
}
