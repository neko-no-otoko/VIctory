//! Narrow macOS compute command path. Unsupported commands poison the list.
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::descriptor::{self, View};
use crate::object::{self, This};
use victory_shader::{
    Container, FourCC, HwStage, ResourceType, RootType, emit, ir, native, resource,
};

pub(crate) struct Prepared {
    compute: victory_metal::compute::Compute,
    user_data: Vec<emit::UserData>,
    dwords: usize,
    table_slot: u8,
    threads: [u32; 3],
}

impl Prepared {
    pub(crate) fn new(container: &[u8]) -> Result<Self, String> {
        let c = Container::parse(container).map_err(|e| e.to_string())?;
        let dxil = c.part(FourCC::DXIL).ok_or("missing DXIL part")?;
        let programs = victory_shader::native_programs(dxil.data).map_err(|e| e.to_string())?;
        if programs.len() != 1 || programs[0].stage != HwStage::Cs {
            return Err("expected exactly one compute program".into());
        }
        let p = &programs[0];
        let native::StageIo::Cs(io) = &p.io else {
            return Err("missing compute metadata".into());
        };
        if p.used.len() != 1
            || p.used[0].kind != ResourceType::Uav
            || p.used[0].root != RootType::Table
            || p.used[0].stride != 0
            || p.used[0].table_offset_dwords != 0
            || p.used[0].dwords != 4
        {
            return Err("only one raw UAV in a descriptor table is supported".into());
        }
        let program = ir::build(&p.code, p.wave64, p.vgprs).map_err(|e| e.to_string())?;
        let traced = resource::track(&program, &p.layout, p.stage).map_err(|e| e.to_string())?;
        let Some(descriptor) = traced.descriptors.first() else {
            return Err("expected a UAV descriptor access".into());
        };
        let resource::Source::Loaded { load, first: 0 } = descriptor.source else {
            return Err("only loaded UAV descriptors are supported".into());
        };
        if traced.scalar.len() != 1
            || traced
                .descriptors
                .iter()
                .any(|d| d.role != resource::Role::Resource || d.source != descriptor.source)
        {
            return Err("only accesses to the same UAV descriptor are supported".into());
        }
        let Some((_, addr)) = traced.scalar.iter().find(|(site, _)| *site == load) else {
            return Err("missing UAV load".into());
        };
        let Some(root) = p.layout.get(addr.param) else {
            return Err("missing UAV root parameter".into());
        };
        if addr.offset != 0
            || addr.dynamic.is_some()
            || root.root != RootType::Table
            || root.slot != p.used[0].slot
            || root.user_data != p.used[0].user_data
        {
            return Err("only descriptor index zero is supported".into());
        }
        let host = emit::translate(container).map_err(|e| e.to_string())?;
        if !host.samplers.is_empty()
            || !host.root_cbvs.is_empty()
            || !host.root_srvs.is_empty()
            || host.user_data.iter().any(|u| {
                !matches!(u.root, RootType::Constants | RootType::Table)
                    || (u.root == RootType::Table && u.slot != root.slot)
            })
        {
            return Err("unsupported compute shader bindings".into());
        }
        let path =
            std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
        let metallib = victory_metal::converter::convert(
            std::path::Path::new(&path),
            &host.dxil,
            victory_metal::converter::Stage::Compute,
        )?;
        let compute = victory_metal::compute::Compute::new(&metallib, "main")?;
        Ok(Self {
            compute,
            dwords: (host.user_data_dwords() as usize).max(4),
            user_data: host.user_data,
            table_slot: root.slot,
            threads: io.threads,
        })
    }
}

struct ResourceRef(usize);
impl ResourceRef {
    fn new(resource: This) -> Self {
        object::add_ref(resource);
        Self(resource as usize)
    }
    fn ptr(&self) -> This {
        self.0 as This
    }
}
impl Drop for ResourceRef {
    fn drop(&mut self) {
        crate::resource::resource_release(self.ptr());
    }
}
struct HeapRef(usize);
impl HeapRef {
    fn new(heap: This) -> Self {
        object::add_ref(heap);
        Self(heap as usize)
    }
    fn ptr(&self) -> This {
        self.0 as This
    }
}
impl Drop for HeapRef {
    fn drop(&mut self) {
        descriptor::release(self.ptr());
    }
}

const COMMON: u32 = 0;
const UAV: u32 = 0x8;
const COPY_DEST: u32 = 0x400;
const COPY_SOURCE: u32 = 0x800;

fn supported_state(state: u32) -> bool {
    matches!(state, COMMON | UAV | COPY_DEST | COPY_SOURCE)
}

/// All lists in one ExecuteCommandLists call share promotions and decay together.
#[derive(Default)]
pub(crate) struct Submission {
    touched: HashMap<usize, ResourceRef>,
}
impl Submission {
    fn access(&mut self, resource: This, required: u32) -> Result<(), String> {
        let current = crate::resource::metal_buffer_state(resource, |s| s.load(Ordering::Relaxed))
            .ok_or("only committed default-heap buffers are supported")?;
        if current != COMMON && current != required {
            return Err(format!(
                "buffer state {current:#x} does not permit {required:#x}"
            ));
        }
        self.touch(resource);
        if current == COMMON {
            crate::resource::metal_buffer_state(resource, |s| s.store(required, Ordering::Relaxed));
        }
        Ok(())
    }
    fn transition(&mut self, resource: This, before: u32, after: u32) -> Result<(), String> {
        let current = crate::resource::metal_buffer_state(resource, |s| s.load(Ordering::Relaxed))
            .ok_or("only committed default-heap buffers are supported")?;
        if current != COMMON && current != before {
            return Err(format!(
                "transition expected {before:#x}, found {current:#x}"
            ));
        }
        self.touch(resource);
        crate::resource::metal_buffer_state(resource, |s| s.store(after, Ordering::Relaxed));
        Ok(())
    }
    fn touch(&mut self, resource: This) {
        self.touched
            .entry(resource as usize)
            .or_insert_with(|| ResourceRef::new(resource));
    }
}
impl Drop for Submission {
    fn drop(&mut self) {
        // D3D12 buffers decay to COMMON after the entire submission completes.
        for resource in self.touched.values() {
            crate::resource::metal_buffer_state(resource.ptr(), |s| {
                s.store(COMMON, Ordering::Relaxed)
            });
        }
    }
}

#[derive(Default)]
enum Root {
    #[default]
    None,
    Constants(Vec<Option<u32>>),
    Table(u64),
}
enum Command {
    Transition {
        resource: ResourceRef,
        before: u32,
        after: u32,
    },
    Copy {
        dst: ResourceRef,
        dst_offset: u64,
        src: ResourceRef,
        src_offset: u64,
        len: u64,
    },
    Barrier {
        _resource: Option<ResourceRef>,
    },
    Dispatch {
        pipeline: Arc<Mutex<Prepared>>,
        _heap: Arc<HeapRef>,
        handle: u64,
        resource: ResourceRef,
        constants: Vec<u32>,
        groups: [u32; 3],
    },
}

#[derive(Default)]
pub(crate) struct State {
    pipeline: Option<Arc<Mutex<Prepared>>>,
    heap: Option<Arc<HeapRef>>,
    roots: Vec<Root>,
    local_states: HashMap<usize, u32>,
    commands: Vec<Command>,
    error: Option<String>,
    closed: bool,
}
impl State {
    pub(crate) fn reject(&mut self, reason: &str) {
        if self.error.is_none() {
            self.error = Some(reason.into());
        }
    }
    fn open(&mut self) -> bool {
        if self.closed {
            self.reject("command list is closed");
            return false;
        }
        self.error.is_none()
    }
    fn use_state(&mut self, resource: This, required: u32) -> Result<(), String> {
        if crate::resource::metal_buffer_state(resource, |_| ()).is_none() {
            return Err("only committed default-heap buffers are supported".into());
        }
        if required == UAV && !crate::resource::metal_buffer_allows_uav(resource) {
            return Err("buffer was not created for unordered access".into());
        }
        let state = self
            .local_states
            .entry(resource as usize)
            .or_insert(required);
        if *state != COMMON && *state != required {
            return Err(format!(
                "recorded buffer state {:#x} does not permit {required:#x}",
                *state
            ));
        }
        *state = required;
        Ok(())
    }
    pub(crate) fn transition(&mut self, resource: This, subresource: u32, before: u32, after: u32) {
        if !self.open() {
            return;
        }
        if subresource != u32::MAX
            || !supported_state(before)
            || !supported_state(after)
            || before == after
        {
            self.reject("unsupported buffer transition");
            return;
        }
        if crate::resource::metal_buffer_state(resource, |_| ()).is_none() {
            self.reject("transitions support only committed default-heap buffers");
            return;
        }
        if (before == UAV || after == UAV) && !crate::resource::metal_buffer_allows_uav(resource) {
            self.reject("buffer was not created for unordered access");
            return;
        }
        if self
            .local_states
            .get(&(resource as usize))
            .is_some_and(|state| *state != COMMON && *state != before)
        {
            self.reject("transition before-state does not match earlier commands");
            return;
        }
        self.local_states.insert(resource as usize, after);
        self.commands.push(Command::Transition {
            resource: ResourceRef::new(resource),
            before,
            after,
        });
    }
    pub(crate) fn set_pipeline(&mut self, pipeline: This) {
        if !self.open() {
            return;
        }
        match crate::pipeline::metal_compute(pipeline) {
            Ok(p) => self.pipeline = Some(p),
            Err(e) => self.reject(&e),
        }
    }
    pub(crate) fn set_heaps(&mut self, heaps: &[This]) {
        if !self.open() {
            return;
        }
        if heaps.len() != 1
            || heaps[0].is_null()
            || !descriptor::heap(heaps[0]).is_shader_visible_view_heap()
        {
            self.reject("expected one shader-visible view heap");
            return;
        }
        self.heap = Some(Arc::new(HeapRef::new(heaps[0])));
    }
    pub(crate) fn set_table(&mut self, slot: u32, handle: u64) {
        if !self.open() {
            return;
        }
        if slot >= 64 {
            self.reject("root slot out of range");
            return;
        }
        if self.roots.len() <= slot as usize {
            self.roots.resize_with(slot as usize + 1, Root::default);
        }
        self.roots[slot as usize] = Root::Table(handle);
    }
    pub(crate) fn set_constants(&mut self, slot: u32, values: &[u32], offset: u32) {
        if !self.open() {
            return;
        }
        let Some(end) = (offset as usize).checked_add(values.len()) else {
            self.reject("constant range overflow");
            return;
        };
        if slot >= 64 || end > 64 {
            self.reject("constant range out of bounds");
            return;
        }
        if self.roots.len() <= slot as usize {
            self.roots.resize_with(slot as usize + 1, Root::default);
        }
        if !matches!(self.roots[slot as usize], Root::Constants(_)) {
            self.roots[slot as usize] = Root::Constants(Vec::new());
        }
        let Root::Constants(words) = &mut self.roots[slot as usize] else {
            unreachable!()
        };
        words.resize(words.len().max(end), None);
        for (word, value) in words[offset as usize..end].iter_mut().zip(values) {
            *word = Some(*value);
        }
    }
    pub(crate) fn copy_buffer(
        &mut self,
        dst: This,
        dst_offset: u64,
        src: This,
        src_offset: u64,
        len: u64,
    ) {
        if !self.open() {
            return;
        }
        if let Err(error) = copy_ranges(dst, dst_offset, src, src_offset, len) {
            self.reject(&error);
            return;
        }
        if let Err(error) = self
            .use_state(src, COPY_SOURCE)
            .and_then(|()| self.use_state(dst, COPY_DEST))
        {
            self.reject(&error);
            return;
        }
        self.commands.push(Command::Copy {
            dst: ResourceRef::new(dst),
            dst_offset,
            src: ResourceRef::new(src),
            src_offset,
            len,
        });
    }
    pub(crate) fn uav_barrier(&mut self, resource: This) {
        if !self.open() {
            return;
        }
        if !resource.is_null()
            && (crate::resource::metal_buffer_state(resource, |_| ()).is_none()
                || !crate::resource::metal_buffer_allows_uav(resource))
        {
            self.reject("UAV barriers support only committed default-heap UAV buffers");
            return;
        }
        self.commands.push(Command::Barrier {
            _resource: (!resource.is_null()).then(|| ResourceRef::new(resource)),
        });
    }
    pub(crate) fn dispatch(&mut self, groups: [u32; 3]) {
        if !self.open() {
            return;
        }
        if let Err(e) = self.record_dispatch(groups) {
            self.reject(&e);
        }
    }
    fn record_dispatch(&mut self, groups: [u32; 3]) -> Result<(), String> {
        if groups.contains(&0) {
            return Err("zero dispatch dimensions are unsupported".into());
        }
        let pipeline = self.pipeline.clone().ok_or("no compute pipeline")?;
        let p = pipeline.lock().unwrap_or_else(|e| e.into_inner());
        let bound_heap = self.heap.clone().ok_or("no descriptor heap")?;
        let heap = descriptor::heap(bound_heap.ptr());
        let Some(Root::Table(handle)) = self.roots.get(p.table_slot as usize) else {
            return Err("missing UAV descriptor table".into());
        };
        if *handle != heap.start || heap.index(*handle) != Some(0) {
            return Err("only the first heap descriptor is supported".into());
        }
        let Some(View::Unordered(resource, 0, None)) = descriptor::view(*handle) else {
            return Err("expected a whole-buffer UAV without a counter".into());
        };
        let resource = resource as This;
        if resource.is_null() || crate::resource::buffer_bytes(resource).is_none() {
            return Err("UAV is not a buffer".into());
        }
        let mut constants = vec![0; p.dwords];
        for u in &p.user_data {
            constants[u.index as usize] = match (u.root, self.roots.get(u.slot as usize)) {
                (RootType::Constants, Some(Root::Constants(v))) => v
                    .get(u.dword as usize)
                    .copied()
                    .flatten()
                    .ok_or("missing root constant")?,
                (RootType::Table, Some(Root::Table(h))) if *h == heap.start => 0,
                _ => return Err("missing or unsupported root argument".into()),
            };
        }
        let handle = *handle;
        self.use_state(resource, UAV)?;
        let resource = ResourceRef::new(resource);
        drop(p);
        self.commands.push(Command::Dispatch {
            pipeline,
            _heap: bound_heap,
            handle,
            resource,
            constants,
            groups,
        });
        Ok(())
    }
    pub(crate) fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Err("command list is already closed".into());
        }
        self.closed = true;
        match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    pub(crate) fn execute(&mut self, submission: &mut Submission) -> Result<(), String> {
        if !self.closed {
            return Err("command list is open".into());
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        for command in &mut self.commands {
            match command {
                Command::Transition {
                    resource,
                    before,
                    after,
                } => {
                    submission.transition(resource.ptr(), *before, *after)?;
                }
                Command::Copy {
                    dst,
                    dst_offset,
                    src,
                    src_offset,
                    len,
                } => {
                    submission.access(src.ptr(), COPY_SOURCE)?;
                    submission.access(dst.ptr(), COPY_DEST)?;
                    let (to, from, len) =
                        copy_ranges(dst.ptr(), *dst_offset, src.ptr(), *src_offset, *len)?;
                    // SAFETY: checked ranges are live and non-overlapping; submission is synchronous.
                    unsafe { std::ptr::copy_nonoverlapping(from, to, len) };
                }
                // Each preceding dispatch has already completed, so the barrier's ordering is met.
                Command::Barrier { .. } => {}
                Command::Dispatch {
                    pipeline,
                    _heap: _,
                    handle,
                    resource,
                    constants,
                    groups,
                } => {
                    if !matches!(descriptor::view(*handle), Some(View::Unordered(r, 0, None)) if r == resource.0)
                    {
                        return Err("UAV descriptor changed after recording".into());
                    }
                    submission.access(resource.ptr(), UAV)?;
                    let (ptr, len) = crate::resource::buffer_bytes(resource.ptr())
                        .ok_or("UAV resource no longer valid")?;
                    let len = usize::try_from(len).map_err(|_| "UAV buffer is too large")?;
                    // SAFETY: the command holds a reference to the live resource,
                    // and queue submission is synchronous and exclusive.
                    let buffer = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
                    let mut p = pipeline.lock().unwrap_or_else(|e| e.into_inner());
                    let threads = p.threads;
                    p.compute.dispatch(buffer, constants, *groups, threads)?;
                }
            }
        }
        Ok(())
    }
}

fn copy_ranges(
    dst: This,
    dst_offset: u64,
    src: This,
    src_offset: u64,
    len: u64,
) -> Result<(*mut u8, *const u8, usize), String> {
    if dst == src {
        return Err("same-resource buffer copies are unsupported".into());
    }
    let range = |resource, offset: u64| -> Result<(*mut u8, usize, usize), String> {
        let (base, size) =
            crate::resource::buffer_bytes(resource).ok_or("copy requires buffer resources")?;
        let end = offset.checked_add(len).ok_or("copy range overflow")?;
        if end > size {
            return Err("copy range exceeds buffer".into());
        }
        let offset = usize::try_from(offset).map_err(|_| "copy offset is too large")?;
        let count = usize::try_from(len).map_err(|_| "copy length is too large")?;
        let start = (base as usize)
            .checked_add(offset)
            .ok_or("copy address overflow")?;
        let end = start.checked_add(count).ok_or("copy address overflow")?;
        Ok((start as *mut u8, start, end))
    };
    let (to, a, b) = range(dst, dst_offset)?;
    let (from, c, d) = range(src, src_offset)?;
    if len != 0 && a < d && c < b {
        return Err("overlapping buffer copies are unsupported".into());
    }
    Ok((to, from.cast_const(), len as usize))
}
