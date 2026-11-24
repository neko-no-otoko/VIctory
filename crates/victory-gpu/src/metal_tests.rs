//! Synthetic console COM calls through device, list and queue into Metal.
use crate::object::{self, HRESULT, Iid, S_OK, This, slot};
use std::ffi::c_void;

#[repr(C)]
struct Bytecode {
    code: *const u8,
    size: usize,
}
#[repr(C)]
struct ComputeDesc {
    root: This,
    stage: Bytecode,
}
#[repr(C)]
struct QueueDesc {
    kind: u32,
    priority: i32,
    flags: u32,
    nodes: u32,
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
struct HeapProperties {
    kind: u32,
    cpu_pages: u32,
    pool: u32,
    creation_nodes: u32,
    visible_nodes: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
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
struct Barrier {
    kind: u32,
    flags: u32,
    resource: This,
    reserved: [u64; 2],
}

fn iid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Iid {
    object::guid(a, b, c, d)
}

// Synthetic FSSM compute: load UAV word 1, add s0, store in UAV word 0.
fn shader() -> Vec<u8> {
    let words = [
        0xbe85_0380u32,
        0xf408_0202,
        0xfa00_0000,
        0xe030_0004,
        0x8002_0000,
        0xbf8c_0070,
        0x4a00_0000,
        0xe070_0000,
        0x8002_0000,
        0xbf81_0000,
    ];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut payload = vec![0u8; 0x68];
    payload[0..8].copy_from_slice(&6u64.to_le_bytes());
    for slot in [0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x48, 0x60] {
        payload[slot..slot + 8].copy_from_slice(&0xdead_beef_dead_beefu64.to_le_bytes());
    }
    let record = payload.len();
    payload[0x60..0x68].copy_from_slice(&(record as u64).to_le_bytes());
    let code_at = record + 0x54 + 0x40;
    payload.resize(code_at, 0);
    payload[record..record + 8].copy_from_slice(&(code_at as u64).to_le_bytes());
    payload[record + 8..record + 16].copy_from_slice(&(code.len() as u64).to_le_bytes());
    payload[record + 0x14..record + 0x18].copy_from_slice(&0x20u32.to_le_bytes());
    payload[record + 0x50..record + 0x54].copy_from_slice(&8u32.to_le_bytes());
    let stage = record + 0x54;
    payload[stage + 0x24..stage + 0x28].copy_from_slice(&(5u32 << 1).to_le_bytes());
    for (i, n) in [1u32, 1, 1].iter().enumerate() {
        payload[stage + 0x28 + i * 4..stage + 0x2c + i * 4].copy_from_slice(&n.to_le_bytes());
    }
    payload.extend_from_slice(&code);
    let layout = payload.len();
    for (kind, user, slot) in [(0u32, 0u8, 0u8), (3u32, 4u8, 1u8)] {
        for n in [kind, 0, 1, 0] {
            payload.extend_from_slice(&n.to_le_bytes());
        }
        payload.extend_from_slice(&[user, slot, 0, 0]);
    }
    let used = payload.len();
    payload.extend_from_slice(&[3, 3, 0, 0]);
    for n in [0u32, 0, 0] {
        payload.extend_from_slice(&n.to_le_bytes());
    }
    payload.extend_from_slice(&[4, 4, 1, 0]);
    payload[record + 0x28..record + 0x30].copy_from_slice(&(layout as u64).to_le_bytes());
    payload[record + 0x30..record + 0x38].copy_from_slice(&(used as u64).to_le_bytes());
    payload[record + 0x38..record + 0x3c].copy_from_slice(&2u32.to_le_bytes());
    payload[record + 0x3c..record + 0x40].copy_from_slice(&1u32.to_le_bytes());
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
    for n in [1u32, 0x24, 0x0d, 0x30, blob.len() as u32] {
        dxil.extend_from_slice(&n.to_le_bytes());
    }
    dxil.extend_from_slice(&blob);
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

#[test]
#[ignore = "requires Apple Metal Shader Converter and a Metal GPU"]
fn console_compute_records_and_submits_on_metal() {
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
    let heap_iid = iid(
        0x8efb471d,
        0x616c,
        0x4f49,
        [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
    );
    let heap_desc = HeapDesc {
        kind: 0,
        count: 1,
        flags: 1,
        nodes: 0,
        placement: [0; 2],
    };
    let mut heap = null;
    let make_heap = slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
        dev, 0x70,
    );
    assert_eq!(
        make_heap(dev, (&raw const heap_desc).cast(), &heap_iid, &mut heap),
        S_OK
    );
    let handle = slot::<extern "system" fn(This) -> u64>(heap, 0x48)(heap);
    let props = HeapProperties {
        kind: 1,
        cpu_pages: 0,
        pool: 0,
        creation_nodes: 0,
        visible_nodes: 0,
    };
    let desc = ResourceDesc {
        dimension: 1,
        alignment: 0,
        width: 64,
        height: 1,
        depth: 1,
        mips: 1,
        format: 0,
        samples: 1,
        quality: 0,
        layout: 1,
        flags: 4,
    };
    let resource_iid = iid(
        0x696442be,
        0xa72e,
        0x4059,
        [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad],
    );
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
    let mut resource = null;
    assert_eq!(
        make_resource(
            dev,
            (&raw const props).cast(),
            0,
            (&raw const desc).cast(),
            0,
            null.cast(),
            &resource_iid,
            &mut resource
        ),
        S_OK
    );
    let mut source = null;
    let mut target = null;
    for out in [&mut source, &mut target] {
        assert_eq!(
            make_resource(
                dev,
                (&raw const props).cast(),
                0,
                (&raw const desc).cast(),
                0,
                null.cast(),
                &resource_iid,
                out
            ),
            S_OK
        );
    }
    let map = slot::<extern "system" fn(This, u32, *const c_void, *mut *mut u8) -> HRESULT>(
        resource, 0x40,
    );
    let mut input = std::ptr::null_mut();
    let mut output = std::ptr::null_mut();
    assert_eq!(map(source, 0, null.cast(), &mut input), S_OK);
    assert_eq!(map(target, 0, null.cast(), &mut output), S_OK);
    // SAFETY: both mapped buffers have 64 bytes.
    unsafe {
        std::ptr::write_bytes(input, 0x55, 64);
        std::ptr::write_bytes(output, 0x77, 64);
        for (i, value) in [0u32, 22, 33, 44].into_iter().enumerate() {
            input.add(8 + i * 4).cast::<u32>().write_unaligned(value);
        }
    }
    let mut mapped = std::ptr::null_mut();
    assert_eq!(
        slot::<extern "system" fn(This, u32, *const c_void, *mut *mut u8) -> HRESULT>(
            resource, 0x40
        )(resource, 0, null.cast(), &mut mapped),
        S_OK
    );
    // SAFETY: mapped resource is 64 bytes.
    unsafe {
        mapped.cast::<u32>().write_unaligned(0x12345678);
    }
    slot::<extern "system" fn(This, This, This, *const [u32; 10], u64)>(dev, 0x98)(
        dev,
        resource,
        null,
        null.cast(),
        handle,
    );
    let bytes = shader();
    let pipeline_desc = ComputeDesc {
        root: null,
        stage: Bytecode {
            code: bytes.as_ptr(),
            size: bytes.len(),
        },
    };
    let pipeline_iid = iid(
        0x765a30f3,
        0xf624,
        0x4c6f,
        [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
    );
    let mut pipeline = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x58
        )(
            dev,
            (&raw const pipeline_desc).cast(),
            &pipeline_iid,
            &mut pipeline
        ),
        S_OK
    );
    let allocator_iid = iid(
        0x6102dee4,
        0xaf59,
        0x4b09,
        [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24],
    );
    let mut allocator = null;
    assert_eq!(
        slot::<extern "system" fn(This, u32, *const Iid, *mut This) -> HRESULT>(dev, 0x48)(
            dev,
            2,
            &allocator_iid,
            &mut allocator
        ),
        S_OK
    );
    let list_iid = iid(
        0x7116d91c,
        0xe7e4,
        0x47ce,
        [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5],
    );
    let mut list = null;
    assert_eq!(
        slot::<extern "system" fn(This, u32, u32, This, This, *const Iid, *mut This) -> HRESULT>(
            dev, 0x60
        )(dev, 0, 2, allocator, pipeline, &list_iid, &mut list),
        S_OK
    );
    slot::<extern "system" fn(This, u32, *const This)>(list, 0x100)(list, 1, &heap);
    slot::<extern "system" fn(This, u32, u64)>(list, 0x118)(list, 1, handle);
    let constants = slot::<extern "system" fn(This, u32, u32, *const u32, u32)>(list, 0x138);
    let dispatch = slot::<extern "system" fn(This, u32, u32, u32)>(list, 0x70);
    let copy = slot::<extern "system" fn(This, This, u64, This, u64, u64)>(list, 0x78);
    let barrier = slot::<extern "system" fn(This, u32, *const Barrier) -> u32>(list, 0xd0);
    let uav_barrier = Barrier {
        kind: 2,
        flags: 0,
        resource,
        reserved: [0; 2],
    };
    let transition = |resource, before: u32, after: u32, subresource: u32| Barrier {
        kind: 0,
        flags: 0,
        resource,
        reserved: [
            u64::from(subresource) | (u64::from(before) << 32),
            u64::from(after),
        ],
    };
    copy(list, resource, 4, source, 8, 16);
    barrier(list, 1, &transition(resource, 0x400, 8, u32::MAX));
    constants(list, 0, 4, [8, 0, 0, 0].as_ptr(), 0);
    dispatch(list, 1, 1, 1);
    assert_eq!(barrier(list, 1, &uav_barrier), 0);
    slot::<extern "system" fn(This, This)>(list, 0xc8)(list, pipeline);
    constants(list, 0, 4, [41, 0, 0, 0].as_ptr(), 0);
    dispatch(list, 1, 1, 1);
    assert_eq!(barrier(list, 1, &uav_barrier), 0);
    barrier(list, 1, &transition(resource, 8, 0x800, u32::MAX));
    copy(list, target, 12, resource, 0, 20);
    let closed = slot::<extern "system" fn(This) -> HRESULT>(list, 0x48)(list);
    assert_eq!(
        closed,
        S_OK,
        "list error: {:?}",
        crate::list::metal_execute(list, 2, &mut Default::default())
    );
    // SAFETY: list holds the resource alive through submission.
    assert_eq!(unsafe { mapped.cast::<u32>().read_unaligned() }, 0x12345678);
    // SAFETY: the recorded copies retain both mapped buffers.
    assert_eq!(
        unsafe { output.add(12).cast::<u32>().read_unaligned() },
        0x77777777
    );
    // SAFETY: the source is mapped and retained; submission has not begun.
    unsafe { input.add(8).cast::<u32>().write_unaligned(1) };
    assert_eq!(release(source), 1);
    assert_eq!(release(target), 1);
    assert_eq!(release(resource), 8);
    assert_eq!(release(heap), 1);
    assert_eq!(release(pipeline), 0);
    let queue_iid = iid(
        0x0ec870a6,
        0x5d7e,
        0x4c22,
        [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed],
    );
    let queue_desc = QueueDesc {
        kind: 2,
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
    // SAFETY: list still holds the resource reference.
    assert_eq!(unsafe { mapped.cast::<u32>().read_unaligned() }, 42);
    // SAFETY: the list still owns the destination through its copy command.
    let copied: Vec<u32> = (0..5)
        .map(|i| unsafe { output.add(12 + i * 4).cast::<u32>().read_unaligned() })
        .collect();
    assert_eq!(copied, [42, 1, 22, 33, 44]);
    assert_eq!(
        unsafe { output.add(8).cast::<u32>().read_unaligned() },
        0x77777777
    );
    assert_eq!(
        unsafe { output.add(32).cast::<u32>().read_unaligned() },
        0x77777777
    );
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
    let signal = slot::<extern "system" fn(This, This, u64) -> HRESULT>(queue, 0x70);
    let completed = slot::<extern "system" fn(This) -> u64>(fence, 0x40);
    assert_eq!(signal(queue, fence, 7), S_OK);
    assert_eq!(completed(fence), 7);
    // The resource decayed to COMMON after the first submission. Two lists in one
    // submission share state and must not decay between lists.
    let mut lists = [null; 2];
    for (idx, (before, after)) in [(0, 0x400), (0x400, 0x800)].into_iter().enumerate() {
        assert_eq!(
            slot::<extern "system" fn(This, u32, u32, This, This, *const Iid, *mut This) -> HRESULT>(
                dev, 0x60
            )(dev, 0, 2, allocator, null, &list_iid, &mut lists[idx]),
            S_OK
        );
        barrier(
            lists[idx],
            1,
            &transition(resource, before, after, u32::MAX),
        );
        assert_eq!(
            slot::<extern "system" fn(This) -> HRESULT>(lists[idx], 0x48)(lists[idx]),
            S_OK
        );
    }
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 2, lists.as_ptr());
    assert_eq!(signal(queue, fence, 7), S_OK);
    for list in lists {
        assert_eq!(release(list), 0);
    }
    // Changing an already-recorded descriptor must not silently use the old buffer.
    slot::<extern "system" fn(This, This, This, *const [u32; 10], u64)>(dev, 0x98)(
        dev,
        null,
        null,
        null.cast(),
        handle,
    );
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(signal(queue, fence, 8), object::E_NOTIMPL);
    assert_eq!(completed(fence), 7);
    assert_eq!(release(queue), 0);
    queue = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x40
        )(dev, (&raw const queue_desc).cast(), &queue_iid, &mut queue),
        S_OK
    );
    assert_eq!(
        slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50)(list, allocator, null),
        S_OK
    );
    let mut a = null;
    let mut b = null;
    let no_uav = ResourceDesc { flags: 0, ..desc };
    for (out, initial, description) in [(&mut a, 0x400, &desc), (&mut b, 0, &no_uav)] {
        assert_eq!(
            make_resource(
                dev,
                (&raw const props).cast(),
                0,
                (description as *const ResourceDesc).cast(),
                initial,
                null.cast(),
                &resource_iid,
                out
            ),
            S_OK
        );
    }
    let reset = slot::<extern "system" fn(This, This, This) -> HRESULT>(list, 0x50);
    let close = slot::<extern "system" fn(This) -> HRESULT>(list, 0x48);
    for (dst, dst_offset, src, src_offset, length) in
        [(a, 61, b, 0, 4), (a, 0, b, u64::MAX, 2), (a, 0, a, 4, 4)]
    {
        copy(list, dst, dst_offset, src, src_offset, length);
        assert_eq!(close(list), object::E_NOTIMPL);
        assert_eq!(reset(list, allocator, null), S_OK);
    }
    for invalid in [
        transition(a, 0, 8, 0),
        transition(a, 0, 0, u32::MAX),
        transition(a, 0, 0x408, u32::MAX),
        transition(b, 0, 8, u32::MAX),
    ] {
        barrier(list, 1, &invalid);
        assert_eq!(close(list), object::E_NOTIMPL);
        assert_eq!(reset(list, allocator, null), S_OK);
    }
    copy(list, a, 0, b, 0, 4);
    barrier(list, 1, &transition(a, 8, 0x800, u32::MAX));
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    copy(list, a, 0, b, 0, 4);
    copy(list, b, 4, a, 4, 4); // Missing COPY_DEST -> COPY_SOURCE transition.
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    let split = Barrier {
        kind: 2,
        flags: 1,
        resource: a,
        reserved: [0; 2],
    };
    barrier(list, 1, &split);
    assert_eq!(close(list), object::E_NOTIMPL);
    assert_eq!(reset(list, allocator, null), S_OK);
    // The first transition is checked against the actual creation state at submission.
    barrier(list, 1, &transition(a, 0x800, 8, u32::MAX));
    assert_eq!(close(list), S_OK);
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(signal(queue, fence, 8), object::E_NOTIMPL);
    assert_eq!(completed(fence), 7);
    assert_eq!(reset(list, allocator, null), S_OK);
    assert_eq!(release(queue), 0);
    queue = null;
    assert_eq!(
        slot::<extern "system" fn(This, *const c_void, *const Iid, *mut This) -> HRESULT>(
            dev, 0x40
        )(dev, (&raw const queue_desc).cast(), &queue_iid, &mut queue),
        S_OK
    );
    barrier(list, 1, &transition(a, 0x400, 8, u32::MAX));
    assert_eq!(close(list), S_OK);
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(signal(queue, fence, 7), S_OK);
    assert_eq!(reset(list, allocator, null), S_OK);
    // A COMMON buffer permits a promotable before-state in a transition.
    barrier(list, 1, &transition(a, 0x800, 8, u32::MAX));
    assert_eq!(close(list), S_OK);
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(signal(queue, fence, 7), S_OK);
    assert_eq!(reset(list, allocator, null), S_OK);
    assert_eq!(release(a), 0);
    assert_eq!(release(b), 0);
    slot::<extern "system" fn(This, u32, u32, u32, u32)>(list, 0x60)(list, 3, 2, 0, 0); // Instancing is unsupported.
    assert_eq!(
        slot::<extern "system" fn(This) -> HRESULT>(list, 0x48)(list),
        object::E_NOTIMPL
    );
    slot::<extern "system" fn(This, u32, *const This)>(queue, 0x50)(queue, 1, &list);
    assert_eq!(signal(queue, fence, 8), object::E_NOTIMPL);
    assert_eq!(completed(fence), 7);
    assert_eq!(release(fence), 0);
    assert_eq!(release(list), 0);
    assert_eq!(release(allocator), 0);
    assert_eq!(release(queue), 0);
}
