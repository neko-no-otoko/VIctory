//! Opt-in native Metal test of console texture and sampler descriptor tables.
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
    let mut sig = vec![0; 40];
    u32_at(&mut sig, 0, 1);
    u32_at(&mut sig, 4, 8);
    u32_at(&mut sig, 12, 40);
    u32_at(&mut sig, 20, system);
    u32_at(&mut sig, 24, 3);
    sig[32] = 15;
    sig.extend_from_slice(name);
    sig
}

// The sampled PS is the GFX10 program in victory-metal/tests/render.rs.
// Encode its actual root and resource metadata in the console's FSSM format.
fn shader(vertex: bool) -> Vec<u8> {
    let words: &[u32] = if vertex {
        &[0xf800_18cf, 0x0706_0504, 0xbf81_0000]
    } else {
        &[
            0x7e04_0200,
            0x7e06_02f0,
            0xbe82_0309,
            0xbe89_0380,
            0xbe83_0380,
            0xf408_0204,
            0xfa00_0000,
            0xf40c_0001,
            0xfa00_0000,
            0xf080_0f08,
            0x0040_0002,
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
    let code_at = stage + if vertex { 0x18c } else { 0x14c + 8 * 404 };
    payload.resize(code_at, 0);
    u64_at(&mut payload, record, code_at as u64);
    u64_at(&mut payload, record + 8, (words.len() * 4) as u64);
    u32_at(&mut payload, record + 0x14, 0x20);
    u32_at(&mut payload, record + 0x50, 8);
    if vertex {
        payload[stage + 0x24..stage + 0x29].copy_from_slice(&[6, 0, 15, 4, 0]);
        u32_at(&mut payload, stage + 0xc4, 1);
    } else {
        for mrt in 0..8 {
            u32_at(&mut payload, stage + 0x14c + mrt * 404, u32::MAX);
        }
    }
    for word in words {
        payload.extend_from_slice(&word.to_le_bytes());
    }
    if !vertex {
        let layout = payload.len();
        payload.resize(layout + 3 * 20 + 2 * 20, 0);
        u64_at(&mut payload, record + 0x28, layout as u64);
        u64_at(&mut payload, record + 0x30, (layout + 60) as u64);
        u32_at(&mut payload, record + 0x38, 3);
        u32_at(&mut payload, record + 0x3c, 2);
        for (i, (kind, user, slot)) in [(0, 0, 0), (3, 8, 1), (3, 9, 2)].into_iter().enumerate() {
            let at = layout + i * 20;
            u32_at(&mut payload, at, kind);
            u32_at(&mut payload, at + 8, 1);
            payload[at + 16] = user;
            payload[at + 17] = slot;
        }
        for (i, (kind, dwords, user, slot)) in [(1, 4, 8, 1), (0, 8, 9, 2)].into_iter().enumerate()
        {
            let at = layout + 60 + i * 20;
            payload[at] = kind;
            payload[at + 1] = 3;
            payload[at + 16] = dwords;
            payload[at + 17] = user;
            payload[at + 18] = slot;
        }
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
    for v in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] {
        dxil.extend_from_slice(&v.to_le_bytes());
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
    let mut container = vec![0; 32 + 4 * parts.len()];
    container[..4].copy_from_slice(b"DXBC");
    u32_at(&mut container, 20, 1);
    u32_at(&mut container, 28, parts.len() as u32);
    for (i, (tag, data)) in parts.into_iter().enumerate() {
        let offset = container.len() as u32;
        u32_at(&mut container, 32 + i * 4, offset);
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
fn console_samples_recorded_texture_and_sampler_tables() {
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
    let image = |width, height, flags| ResourceDesc {
        dimension: 3,
        alignment: 0,
        width,
        height,
        depth: 1,
        mips: 1,
        format: 28,
        samples: 1,
        quality: 0,
        layout: 1,
        flags,
    };
    let vb = make(&buffer(144), 0);
    let readback = make(&buffer(1024), 0);
    let target = make(&image(9, 4, 1), 4);
    let first = make(&image(2, 1, 0), 0x80);
    // Three texels ensure a non-tight, odd-width resource layout is sampled.
    let second = make(&image(3, 1, 0), 0x80);
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
        [-1.0 / 3.0, -1.0, 0.0, 1.0],
        [-2.0 / 3.0, 1.0, 0.0, 1.0],
        [-1.0 / 3.0, -1.0, 0.0, 1.0],
        [1.0 / 3.0, -1.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [1.0 / 3.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, 1.0],
        [2.0 / 3.0, 1.0, 0.0, 1.0],
    ];
    let vertex_data = map(vb);
    let output = map(readback);
    let target_data = map(target);
    let first_data = map(first);
    let second_data = map(second);
    unsafe {
        std::ptr::copy_nonoverlapping(vertices.as_ptr().cast::<u8>(), vertex_data, 144);
        std::ptr::write_bytes(output, 0x77, 1024);
        std::ptr::write_bytes(target_data, 0x30, 1024);
        std::ptr::write_bytes(first_data, 0xa5, 256);
        std::ptr::write_bytes(second_data, 0xa5, 256);
        std::ptr::copy_nonoverlapping([255u8, 0, 0, 255, 0, 0, 255, 255].as_ptr(), first_data, 8);
        std::ptr::copy_nonoverlapping(
            [0u8, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255].as_ptr(),
            second_data,
            12,
        );
    }
    let heap_iid = object::guid(
        0x8efb471d,
        0x616c,
        0x4f49,
        [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
    );
    let create_heap = slot::<
        extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT,
    >(dev, 0x70);
    let heap = |kind, flags| {
        let desc = HeapDesc {
            kind,
            count: 1,
            flags,
            nodes: 0,
            placement: [0; 2],
        };
        let mut out = null;
        assert_eq!(
            create_heap(dev, (&raw const desc).cast(), &heap_iid, &mut out),
            S_OK
        );
        out
    };
    let rtv_heap = heap(2, 0);
    let srv_a = heap(0, 1);
    let srv_b = heap(0, 1);
    let sampler_a = heap(1, 1);
    let sampler_b = heap(1, 1);
    let cpu = |h| slot::<extern "system" fn(This) -> u64>(h, 0x48)(h);
    let gpu = |h| slot::<extern "system" fn(This) -> u64>(h, 0x50)(h);
    let rtv = cpu(rtv_heap);
    slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0xa0)(
        dev,
        target,
        null.cast(),
        rtv,
    );
    let create_srv = slot::<extern "system" fn(This, This, *const c_void, u64)>(dev, 0x90);
    create_srv(dev, first, null.cast(), cpu(srv_a));
    create_srv(dev, second, null.cast(), cpu(srv_b));
    let create_sampler = slot::<extern "system" fn(This, *const [u32; 13], u64)>(dev, 0xb0);
    create_sampler(
        dev,
        &[0, 3, 3, 3, 0, 1, 8, 0, 0, 0, 0, 0, 0],
        cpu(sampler_a),
    );
    create_sampler(
        dev,
        &[0x15, 3, 3, 3, 0, 1, 8, 0, 0, 0, 0, 0, 0],
        cpu(sampler_b),
    );

    let vs = shader(true);
    let ps = shader(false);
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
    let mut list = null;
    assert_eq!(
        slot::<extern "system" fn(This, u32, *const Iid, *mut This) -> HRESULT>(dev, 0x48)(
            dev,
            0,
            &allocator_iid,
            &mut allocator
        ),
        S_OK
    );
    assert_eq!(
        slot::<extern "system" fn(This, u32, u32, This, This, *const Iid, *mut This) -> HRESULT>(
            dev, 0x60
        )(dev, 0, 0, allocator, pipeline, &list_iid, &mut list),
        S_OK
    );
    // A non-render-target texture cannot be transitioned to render-target state.
    let invalid = Barrier {
        kind: 0,
        flags: 0,
        resource: first,
        reserved: [u64::from(u32::MAX) | (0x80u64 << 32), 4],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &invalid);
    assert_eq!(
        slot::<extern "system" fn(This) -> HRESULT>(list, 0x48)(list),
        object::E_NOTIMPL
    );
    assert_eq!(
        slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50)(
            list, allocator, pipeline
        ),
        S_OK
    );
    let view = VertexView {
        address: slot::<extern "system" fn(This) -> u64>(vb, 0x58)(vb),
        size: 144,
        stride: 16,
    };
    slot::<extern "system" fn(This, u32)>(list, 0xa0)(list, 4);
    slot::<extern "system" fn(This, u32, *const f32)>(list, 0xa8)(
        list,
        1,
        [0.0, 0.0, 9.0, 4.0, 0.0, 1.0].as_ptr(),
    );
    slot::<extern "system" fn(This, u32, *const i32)>(list, 0xb0)(list, 1, [0, 0, 9, 4].as_ptr());
    slot::<extern "system" fn(This, u32, u32, *const VertexView)>(list, 0x180)(list, 0, 1, &view);
    slot::<extern "system" fn(This, u32, *const u64, i32, *const u64)>(list, 0x190)(
        list,
        1,
        &rtv,
        0,
        null.cast(),
    );
    let set_heaps = slot::<extern "system" fn(This, u32, *const This)>(list, 0x100);
    let set_table = slot::<extern "system" fn(This, u32, u64)>(list, 0x120);
    let constant = slot::<extern "system" fn(This, u32, u32, u32)>(list, 0x130);
    let draw = slot::<extern "system" fn(This, u32, u32, u32, u32)>(list, 0x60);
    set_heaps(list, 2, [srv_a, sampler_a].as_ptr());
    set_table(list, 1, gpu(sampler_a));
    set_table(list, 2, gpu(srv_a));
    constant(list, 0, 0.375f32.to_bits(), 0);
    draw(list, 3, 1, 0, 0);
    set_heaps(list, 2, [srv_a, sampler_b].as_ptr());
    set_table(list, 1, gpu(sampler_b));
    set_table(list, 2, gpu(srv_a));
    draw(list, 3, 1, 3, 0);
    set_heaps(list, 2, [srv_b, sampler_b].as_ptr());
    set_table(list, 1, gpu(sampler_b));
    set_table(list, 2, gpu(srv_b));
    constant(list, 0, (1.0f32 / 3.0).to_bits(), 0);
    draw(list, 3, 1, 6, 0);
    let transition = Barrier {
        kind: 0,
        flags: 0,
        resource: target,
        reserved: [u64::from(u32::MAX) | (4u64 << 32), 0x800],
    };
    slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0)(list, 1, &transition);
    let dst = CopyLocation {
        resource: readback,
        kind: 1,
        footprint: Footprint {
            offset: 0,
            format: 28,
            width: 9,
            height: 4,
            depth: 1,
            pitch: 256,
        },
    };
    let src = CopyLocation {
        resource: target,
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
    slot::<
        extern "system" fn(
            This,
            *const CopyLocation,
            u32,
            u32,
            u32,
            *const CopyLocation,
            *const c_void,
        ),
    >(list, 0x80)(list, &dst, 0, 0, 0, &src, null.cast());
    assert_eq!(
        slot::<extern "system" fn(This) -> HRESULT>(list, 0x48)(list),
        S_OK
    );
    assert_eq!(unsafe { *output.add(2 * 256 + 4) }, 0x77);
    assert_eq!(unsafe { *target_data.add(2 * 256 + 4) }, 0x30);
    // Texture contents are read at submission, not snapshotted while recording.
    unsafe { std::ptr::copy_nonoverlapping([0u8, 255, 0, 255].as_ptr(), second_data, 4) };
    // Recording owns its pipeline, descriptors and resources, not the caller.
    for o in [
        vb, readback, target, first, second, rtv_heap, srv_a, srv_b, sampler_a, sampler_b, pipeline,
    ] {
        release(o);
    }
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
    let pixel =
        |x: usize, y: usize| unsafe { std::slice::from_raw_parts(output.add(y * 256 + x * 4), 4) };
    // The first two draws differ only in filter; the third switches SRV.
    assert_eq!(pixel(1, 2), &[255, 0, 0, 255]);
    let filtered = pixel(4, 2);
    assert!(
        (190..=192).contains(&filtered[0])
            && filtered[1] == 0
            && (63..=65).contains(&filtered[2])
            && filtered[3] == 255,
        "linear sample: {filtered:?}"
    );
    let updated = pixel(7, 2);
    assert!(
        (127..=128).contains(&updated[0])
            && updated[1] == 255
            && (127..=128).contains(&updated[2])
            && updated[3] == 255,
        "updated texture: {updated:?}"
    );
    assert_eq!(pixel(0, 0), &[0x30; 4]);
    for y in 0..4 {
        assert_eq!(unsafe { *output.add(y * 256 + 36) }, 0x77);
    }
    // Recorded shader-visible descriptors cannot silently change before resubmission.
    create_srv(dev, second, null.cast(), cpu(srv_a));
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(
        slot::<extern "system" fn(This, This, u64) -> HRESULT>(queue, 0x70)(queue, fence, 2),
        object::E_NOTIMPL
    );
    assert_eq!(
        slot::<extern "system" fn(This) -> u64>(fence, 0x40)(fence),
        1
    );
    release(fence);
    release(list);
    release(allocator);
    release(queue);
    release(dev);
}
