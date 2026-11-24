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
    root_cbv: Option<(u8, u64, usize)>,
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
        let mut uavs = p.used.iter().filter(|u| u.kind == ResourceType::Uav);
        let uav = uavs.next().ok_or("missing UAV descriptor")?;
        if uavs.next().is_some() || uav.root != RootType::Table || uav.stride != 0
            || uav.table_offset_dwords != 0 || uav.dwords != 4
            || p.used.iter().any(|u| u.kind != ResourceType::Uav
                && !(u.kind == ResourceType::Cbv && u.root == RootType::Descriptor))
        {
            return Err("only one raw UAV table and an optional root CBV are supported".into());
        }
        let program = ir::build(&p.code, p.wave64, p.vgprs).map_err(|e| e.to_string())?;
        let traced = resource::track(&program, &p.layout, p.stage).map_err(|e| e.to_string())?;
        let descriptor = traced.descriptors.iter()
            .find(|d| matches!(d.source, resource::Source::Loaded { first: 0, .. }))
            .ok_or("expected a UAV descriptor access")?;
        let resource::Source::Loaded { load, first: 0 } = descriptor.source else { unreachable!() };
        if traced.scalar.len() != 1 {
            return Err("unsupported scalar memory access".into());
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
            || root.slot != uav.slot
            || root.user_data != uav.user_data
        {
            return Err("only descriptor index zero is supported".into());
        }
        let mut cbv_slot = None;
        let mut cbv_required = 0u64;
        for d in &traced.descriptors {
            if d.role != resource::Role::Resource {
                return Err("unsupported compute descriptor role".into());
            }
            if d.source == descriptor.source {
                continue;
            }
            let resource::Source::Built { address, bits: 0x100000, words: [u32::MAX, 0x11014fac] } = &d.source else {
                return Err("unsupported compute buffer descriptor".into());
            };
            let cbv_root = p.layout.get(address.param).ok_or("missing root CBV layout")?;
            if cbv_root.root != RootType::Descriptor || cbv_root.sgprs != 2
                || cbv_root.slot >= 64 || cbv_root.slot == root.slot
                || address.dynamic.is_some()
                || cbv_slot.is_some_and(|slot| slot != cbv_root.slot)
                || !p.used.iter().any(|u| u.kind == ResourceType::Cbv
                    && u.root == RootType::Descriptor && u.slot == cbv_root.slot
                    && u.user_data == cbv_root.user_data)
            {
                return Err("unsupported root CBV layout".into());
            }
            let inst = program.blocks.get(d.site.0).and_then(|b| b.insts.get(d.site.1))
                .ok_or("invalid root CBV access")?;
            let count = match inst.name() {
                "s_buffer_load_dword" => 1u64,
                "s_buffer_load_dwordx2" => 2,
                "s_buffer_load_dwordx4" => 4,
                "s_buffer_load_dwordx8" => 8,
                "s_buffer_load_dwordx16" => 16,
                _ => return Err("only scalar root CBV loads are supported".into()),
            };
            let victory_shader::gfx10::Fields::Smem { offset, soffset, .. } = &inst.fields else {
                return Err("invalid root CBV instruction".into());
            };
            if !matches!(soffset, victory_shader::gfx10::Operand::Scalar { code, .. } if *code == victory_shader::gfx10::NULL) {
                return Err("dynamic root CBV offsets are unsupported".into());
            }
            let start = u64::from(address.offset).checked_add(u64::from(*offset as u32)).ok_or("CBV offset overflow")?;
            let end = start.checked_add(count * 4).and_then(|n| n.checked_next_multiple_of(16)).ok_or("CBV range overflow")?;
            if start % 4 != 0 || end > 65536 {
                return Err("root CBV load is out of range".into());
            }
            cbv_required = cbv_required.max(end);
            cbv_slot = Some(cbv_root.slot);
        }
        let host = emit::translate(container).map_err(|e| e.to_string())?;
        if !host.samplers.is_empty()
            || host.root_cbvs.as_slice() != cbv_slot.as_slice()
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
        let (metallib, root_cbv) = if let Some(slot) = cbv_slot {
            let (metallib, offset) = victory_metal::converter::convert_compute_with_root_cbv(
                std::path::Path::new(&path), &host.dxil, slot)?;
            (metallib, Some((slot, cbv_required, offset)))
        } else {
            (victory_metal::converter::convert(
                std::path::Path::new(&path), &host.dxil,
                victory_metal::converter::Stage::Compute)?, None)
        };
        let compute = victory_metal::compute::Compute::new(&metallib, "main")?;
        Ok(Self {
            compute,
            dwords: (host.user_data_dwords() as usize).max(4),
            user_data: host.user_data,
            table_slot: root.slot,
            root_cbv,
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
const VERTEX_BUFFER: u32 = 1;
const INDEX_BUFFER: u32 = 2;
const RENDER_TARGET: u32 = 4;
const DEPTH_WRITE: u32 = 0x10;
const PIXEL_SHADER_RESOURCE: u32 = 0x80;
const UAV: u32 = 0x8;
const COPY_DEST: u32 = 0x400;
const COPY_SOURCE: u32 = 0x800;

fn color_format(resource: This) -> Option<victory_metal::ColorFormat> {
    match crate::resource::metal_color_format(resource)? {
        28 => Some(victory_metal::ColorFormat::Rgba8),
        29 => Some(victory_metal::ColorFormat::Rgba8Srgb),
        87 => Some(victory_metal::ColorFormat::Bgra8),
        91 => Some(victory_metal::ColorFormat::Bgra8Srgb),
        _ => None,
    }
}

fn sampled_format(resource: This) -> Option<victory_metal::TextureFormat> {
    match crate::resource::metal_sampled_format(resource)? {
        28 => Some(victory_metal::TextureFormat::Rgba8),
        29 => Some(victory_metal::TextureFormat::Rgba8Srgb),
        87 => Some(victory_metal::TextureFormat::Bgra8),
        91 => Some(victory_metal::TextureFormat::Bgra8Srgb),
        71 => Some(victory_metal::TextureFormat::Bc1),
        72 => Some(victory_metal::TextureFormat::Bc1Srgb),
        77 => Some(victory_metal::TextureFormat::Bc3),
        78 => Some(victory_metal::TextureFormat::Bc3Srgb),
        _ => None,
    }
}

fn state_of(resource: This) -> Result<(bool, u32), String> {
    if let Some(state) =
        crate::resource::metal_buffer_state(resource, |s| s.load(Ordering::Relaxed))
    {
        return Ok((false, state));
    }
    crate::resource::metal_texture_state(resource, |s| (true, s.load(Ordering::Relaxed)))
        .ok_or_else(|| "unsupported resource for Metal state tracking".into())
}
fn store_state(resource: This, texture: bool, state: u32) {
    if texture {
        crate::resource::metal_texture_state(resource, |s| s.store(state, Ordering::Relaxed));
    } else {
        crate::resource::metal_buffer_state(resource, |s| s.store(state, Ordering::Relaxed));
    }
}
fn supports_state(texture: bool, state: u32) -> bool {
    if texture {
        matches!(
            state,
            COMMON | RENDER_TARGET | DEPTH_WRITE | COPY_SOURCE | PIXEL_SHADER_RESOURCE
        )
    } else {
        matches!(
            state,
            COMMON | VERTEX_BUFFER | INDEX_BUFFER | 3 | UAV | COPY_DEST | COPY_SOURCE
        )
    }
}
fn can_promote(texture: bool, state: u32) -> bool {
    !texture || matches!(state, COMMON | COPY_SOURCE | PIXEL_SHADER_RESOURCE)
}

fn state_permits(current: u32, required: u32) -> bool {
    current == required
        || (matches!(required, VERTEX_BUFFER | INDEX_BUFFER)
            && current == (VERTEX_BUFFER | INDEX_BUFFER))
}
struct Touched {
    resource: ResourceRef,
    texture: bool,
    decay: bool,
}
/// All lists in one ExecuteCommandLists call share promotions and decay together.
#[derive(Default)]
pub(crate) struct Submission {
    touched: HashMap<usize, Touched>,
}
impl Submission {
    fn access(&mut self, resource: This, required: u32) -> Result<(), String> {
        let (texture, current) = state_of(resource)?;
        if !state_permits(current, required) && !(current == COMMON && can_promote(texture, required)) {
            return Err(format!(
                "resource state {current:#x} does not permit {required:#x}"
            ));
        }
        let decay = !texture
            || (current == COMMON && matches!(required, COPY_SOURCE | PIXEL_SHADER_RESOURCE));
        self.touched
            .entry(resource as usize)
            .or_insert_with(|| Touched {
                resource: ResourceRef::new(resource),
                texture,
                decay,
            });
        if current == COMMON {
            store_state(resource, texture, required);
        }
        Ok(())
    }
    fn transition(&mut self, resource: This, before: u32, after: u32) -> Result<(), String> {
        let (texture, current) = state_of(resource)?;
        if current != before && !(current == COMMON && can_promote(texture, before)) {
            return Err(format!(
                "transition expected {before:#x}, found {current:#x}"
            ));
        }
        let touched = self
            .touched
            .entry(resource as usize)
            .or_insert_with(|| Touched {
                resource: ResourceRef::new(resource),
                texture,
                decay: !texture,
            });
        if texture {
            touched.decay = false;
        }
        store_state(resource, texture, after);
        Ok(())
    }
}
impl Drop for Submission {
    fn drop(&mut self) {
        for touched in self.touched.values() {
            if touched.decay {
                store_state(touched.resource.ptr(), touched.texture, COMMON);
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VertexView {
    pub address: u64,
    pub size: u32,
    pub stride: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct IndexView {
    pub address: u64,
    pub size: u32,
    pub format: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min: f32,
    pub max: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Scissor {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
struct VertexBuffer {
    resource: ResourceRef,
    offset: u64,
    size: u32,
    stride: u32,
}
struct IndexBuffer {
    resource: ResourceRef,
    offset: u64,
    size: u32,
    element_size: u32,
}
struct IndexedDraw {
    resource: ResourceRef,
    offset: u64,
    count: u32,
    element_size: u32,
    base_vertex: i32,
}

struct Sampled {
    _view_heap: Arc<HeapRef>,
    _sampler_heap: Arc<HeapRef>,
    view_handle: u64,
    sampler_handle: u64,
    view: View,
    sampler: View,
    texture: ResourceRef,
    filter: victory_metal::TextureFilter,
    first_mip: u32,
    mip_count: u32,
    min_lod: f32,
    max_lod: f32,
}

#[derive(Default)]
enum Root {
    #[default]
    None,
    Constants(Vec<Option<u32>>),
    Table(u64),
    Cbv { resource: ResourceRef, offset: u64, size: u64 },
}
enum Command {
    ClearTarget {
        target: ResourceRef,
        color: [f32; 4],
        rects: Vec<[u32; 4]>,
    },
    ClearDepth {
        target: ResourceRef,
        depth: f32,
        rects: Vec<[u32; 4]>,
    },
    Draw {
        pipeline: Arc<Mutex<crate::metal_graphics::Prepared>>,
        vertex: ResourceRef,
        vertex_offset: u64,
        vertices: u32,
        vertex_stride: u32,
        target: ResourceRef,
        depth: Option<ResourceRef>,
        indexed: Option<IndexedDraw>,
        sampled: Option<Sampled>,
        constants: Vec<u32>,
        viewport: [f64; 6],
        scissor: [u32; 4],
    },
    ReadTexture {
        target: ResourceRef,
        buffer: ResourceRef,
        offset: u64,
        pitch: u32,
    },
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
        cbv: Option<(ResourceRef, u64, u64)>,
        constants: Vec<u32>,
        groups: [u32; 3],
    },
}

#[derive(Default)]
pub(crate) struct State {
    pipeline: Option<Arc<Mutex<Prepared>>>,
    graphics: Option<Arc<Mutex<crate::metal_graphics::Prepared>>>,
    vertex: Option<VertexBuffer>,
    index: Option<IndexBuffer>,
    target: Option<ResourceRef>,
    depth_target: Option<ResourceRef>,
    viewport: Option<Viewport>,
    scissor: Option<Scissor>,
    triangle_list: bool,
    scratch: Vec<u8>,
    texture_scratch: Vec<u8>,
    depth_scratch: Vec<f32>,
    heap: Option<Arc<HeapRef>>,
    sampler_heap: Option<Arc<HeapRef>>,
    roots: Vec<Root>,
    graphics_roots: Vec<Root>,
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
        let (texture, _) = state_of(resource)?;
        if !supports_state(texture, required) {
            return Err("unsupported resource state".into());
        }
        if required == UAV && !crate::resource::metal_buffer_allows_uav(resource) {
            return Err("buffer was not created for unordered access".into());
        }
        let state = self
            .local_states
            .entry(resource as usize)
            .or_insert(required);
        if !state_permits(*state, required) && !(*state == COMMON && can_promote(texture, required)) {
            return Err(format!(
                "recorded buffer state {:#x} does not permit {required:#x}",
                *state
            ));
        }
        if *state == COMMON {
            *state = required;
        }
        Ok(())
    }
    pub(crate) fn transition(&mut self, resource: This, subresource: u32, before: u32, after: u32) {
        if !self.open() {
            return;
        }
        let Ok((texture, _)) = state_of(resource) else {
            self.reject("unsupported resource for transition");
            return;
        };
        if subresource != u32::MAX
            || !supports_state(texture, before)
            || !supports_state(texture, after)
            || before == after
        {
            self.reject("unsupported resource transition");
            return;
        }
        if (before == UAV || after == UAV) && !crate::resource::metal_buffer_allows_uav(resource) {
            self.reject("buffer was not created for unordered access");
            return;
        }
        if texture
            && ((before == RENDER_TARGET || after == RENDER_TARGET)
                && crate::resource::metal_texture(resource).is_none()
                || (before == PIXEL_SHADER_RESOURCE || after == PIXEL_SHADER_RESOURCE)
                    && crate::resource::metal_sampled_mip_count(resource).is_none()
                || (before == DEPTH_WRITE || after == DEPTH_WRITE)
                    && crate::resource::metal_depth_texture(resource).is_none())
        {
            self.reject("texture usage does not match its creation flags");
            return;
        }
        if self
            .local_states
            .get(&(resource as usize))
            .is_some_and(|state| {
                *state != before && !(*state == COMMON && can_promote(texture, before))
            })
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
        self.pipeline = None;
        self.graphics = None;
        if crate::pipeline::metal_is_graphics(pipeline) {
            match crate::pipeline::metal_graphics(pipeline) {
                Ok(p) => self.graphics = Some(p),
                Err(e) => self.reject(&e),
            }
        } else {
            match crate::pipeline::metal_compute(pipeline) {
                Ok(p) => self.pipeline = Some(p),
                Err(e) => self.reject(&e),
            }
        }
    }
    pub(crate) fn set_topology(&mut self, topology: u32) {
        if !self.open() {
            return;
        }
        if topology != 4 {
            self.reject("only triangle-list topology is supported");
            return;
        }
        self.triangle_list = true;
    }
    pub(crate) fn set_viewport(&mut self, viewport: Viewport) {
        if !self.open() {
            return;
        }
        let x = f64::from(viewport.x);
        let y = f64::from(viewport.y);
        let width = f64::from(viewport.width);
        let height = f64::from(viewport.height);
        if ![x, y, width, height, f64::from(viewport.min), f64::from(viewport.max)]
            .iter().all(|v| v.is_finite())
            || width <= 0.0 || height <= 0.0
            || x < -32768.0 || y < -32768.0
            || x + width > 32767.0 || y + height > 32767.0
            || viewport.min < 0.0 || viewport.max > 1.0 || viewport.min > viewport.max
        {
            self.reject("invalid viewport bounds or depth range");
            return;
        }
        self.viewport = Some(viewport);
    }
    pub(crate) fn set_scissor(&mut self, scissor: Scissor) {
        if !self.open() {
            return;
        }
        if scissor.left > scissor.right || scissor.top > scissor.bottom {
            self.reject("inverted scissor rectangle");
            return;
        }
        self.scissor = Some(scissor);
    }
    pub(crate) fn set_vertex(&mut self, view: VertexView) {
        if !self.open() {
            return;
        }
        if !matches!(view.stride, 16 | 24) || view.size < view.stride || view.address % 4 != 0 {
            self.reject("expected an aligned supported vertex buffer");
            return;
        }
        let Some((resource, offset)) =
            crate::resource::metal_buffer_at(view.address, u64::from(view.size))
        else {
            self.reject("vertex buffer address is not a live committed buffer");
            return;
        };
        // metal_buffer_at returned an owned reference.
        self.vertex = Some(VertexBuffer {
            resource: ResourceRef(resource as usize),
            offset,
            size: view.size,
            stride: view.stride,
        });
    }
    pub(crate) fn set_index(&mut self, view: Option<IndexView>) {
        if !self.open() {
            return;
        }
        let Some(view) = view else {
            self.index = None;
            return;
        };
        let element_size = match view.format {
            57 => 2,
            42 => 4,
            _ => return self.reject("only R16_UINT and R32_UINT indices are supported"),
        };
        if view.size == 0 || view.address % u64::from(element_size) != 0 || view.size % element_size != 0 {
            return self.reject("index buffer view is empty or misaligned");
        }
        let Some((resource, offset)) =
            crate::resource::metal_buffer_at(view.address, u64::from(view.size))
        else {
            return self.reject("index buffer address is not a live committed buffer");
        };
        self.index = Some(IndexBuffer {
            resource: ResourceRef(resource as usize),
            offset,
            size: view.size,
            element_size,
        });
    }
    pub(crate) fn set_target(&mut self, handle: u64, depth_handle: Option<u64>) {
        if !self.open() {
            return;
        }
        let Some(rtv) = descriptor::rtv_record(handle) else {
            self.reject("invalid render-target descriptor");
            return;
        };
        let Some(format) = crate::resource::metal_color_format(rtv.resource) else {
            return self.reject("expected an RGBA8 or BGRA8 render target");
        };
        if !rtv.metal_color8(format) || crate::resource::metal_texture(rtv.resource).is_none() {
            self.reject("invalid color render-target view");
            return;
        }
        let depth = if let Some(handle) = depth_handle {
            let Some(dsv) = descriptor::dsv_record(handle) else {
                return self.reject("invalid depth descriptor");
            };
            if !dsv.metal_depth32() || crate::resource::metal_depth_texture(dsv.resource).is_none() {
                return self.reject("expected a D32_FLOAT depth target");
            }
            Some(ResourceRef::new(dsv.resource))
        } else {
            None
        };
        self.target = Some(ResourceRef::new(rtv.resource));
        self.depth_target = depth;
    }
    pub(crate) fn clear_target(&mut self, handle: u64, color: [f32; 4], rects: &[Scissor]) {
        if !self.open() {
            return;
        }
        if let Err(error) = self.record_clear_target(handle, color, rects) {
            self.reject(&error);
        }
    }
    fn record_clear_target(
        &mut self,
        handle: u64,
        color: [f32; 4],
        rects: &[Scissor],
    ) -> Result<(), String> {
        let rtv = descriptor::rtv_record(handle).ok_or("invalid render-target descriptor")?;
        let format = crate::resource::metal_color_format(rtv.resource).ok_or("unsupported color format")?;
        let (_, width, height, _) = crate::resource::metal_texture(rtv.resource)
            .filter(|_| rtv.metal_color8(format))
            .ok_or("invalid color render-target view")?;
        if rects.iter().any(|r| r.left > r.right || r.top > r.bottom) {
            return Err("invalid render-target clear rectangle".into());
        }
        let clip = |v: i32, limit: u32| (v.max(0) as u32).min(limit);
        let rects = rects
            .iter()
            .map(|r| {
                [
                    clip(r.left, width),
                    clip(r.top, height),
                    clip(r.right, width),
                    clip(r.bottom, height),
                ]
            })
            .collect();
        // Clamp clear values without quantizing: sRGB conversion happens on
        // the Metal attachment and must see the original linear precision.
        let color = color.map(|c| if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) });
        self.use_state(rtv.resource, RENDER_TARGET)?;
        self.commands.push(Command::ClearTarget {
            target: ResourceRef::new(rtv.resource),
            color,
            rects,
        });
        Ok(())
    }
    pub(crate) fn clear_depth(&mut self, handle: u64, flags: u32, depth: f32, rects: &[Scissor]) {
        if !self.open() {
            return;
        }
        if let Err(error) = self.record_clear_depth(handle, flags, depth, rects) {
            self.reject(&error);
        }
    }
    fn record_clear_depth(&mut self, handle: u64, flags: u32, depth: f32, rects: &[Scissor]) -> Result<(), String> {
        if flags != 1 || !depth.is_finite() || !(0.0..=1.0).contains(&depth) {
            return Err("only finite depth-only clears in [0,1] are supported".into());
        }
        let dsv = descriptor::dsv_record(handle).ok_or("invalid depth descriptor")?;
        let (_, width, height, _) = crate::resource::metal_depth_texture(dsv.resource)
            .filter(|_| dsv.metal_depth32()).ok_or("expected a D32_FLOAT depth target")?;
        if rects.iter().any(|r| r.left > r.right || r.top > r.bottom) {
            return Err("invalid depth clear rectangle".into());
        }
        let clip = |v: i32, limit: u32| (v.max(0) as u32).min(limit);
        let rects = rects.iter().map(|r| [
            clip(r.left, width), clip(r.top, height),
            clip(r.right, width), clip(r.bottom, height),
        ]).collect();
        self.use_state(dsv.resource, DEPTH_WRITE)?;
        self.commands.push(Command::ClearDepth { target: ResourceRef::new(dsv.resource), depth, rects });
        Ok(())
    }
    pub(crate) fn draw(&mut self, count: u32, instances: u32, first: u32, first_instance: u32) {
        if !self.open() {
            return;
        }
        if let Err(error) = self.record_draw(count, instances, first, first_instance, None) {
            self.reject(&error);
        }
    }
    pub(crate) fn draw_indexed(&mut self, count: u32, instances: u32, first: u32, base_vertex: i32, first_instance: u32) {
        if !self.open() {
            return;
        }
        if let Err(error) = self.record_draw(count, instances, first, first_instance, Some(base_vertex)) {
            self.reject(&error);
        }
    }
    fn record_draw(
        &mut self,
        count: u32,
        instances: u32,
        first: u32,
        first_instance: u32,
        base_vertex: Option<i32>,
    ) -> Result<(), String> {
        if !self.triangle_list
            || count == 0
            || count % 3 != 0
            || instances != 1
            || first_instance != 0
        {
            return Err("only nonempty, non-instanced triangle lists are supported".into());
        }
        let pipeline = self.graphics.clone().ok_or("no graphics pipeline")?;
        let vertex = self.vertex.as_ref().ok_or("no vertex buffer")?;
        let vertex_stride = vertex.stride;
        if pipeline.lock().unwrap_or_else(|e| e.into_inner()).vertex_stride != vertex_stride {
            return Err("vertex stride does not match pipeline input layout".into());
        }
        let (vertex_offset, vertices, indexed) = if let Some(base_vertex) = base_vertex {
            let index = self.index.as_ref().ok_or("no index buffer")?;
            let start = u64::from(first) * u64::from(index.element_size);
            let bytes = u64::from(count) * u64::from(index.element_size);
            if start.checked_add(bytes).is_none_or(|end| end > u64::from(index.size)) {
                return Err("draw exceeds index buffer view".into());
            }
            let offset = index.offset.checked_add(start).ok_or("index offset overflow")?;
            (vertex.offset, vertex.size / vertex_stride, Some(IndexedDraw {
                resource: ResourceRef::new(index.resource.ptr()),
                offset, count, element_size: index.element_size, base_vertex,
            }))
        } else {
            let start = u64::from(first) * u64::from(vertex_stride);
            let bytes = u64::from(count) * u64::from(vertex_stride);
            if start.checked_add(bytes).is_none_or(|end| end > u64::from(vertex.size)) {
                return Err("draw exceeds vertex buffer view".into());
            }
            (vertex.offset.checked_add(start).ok_or("vertex offset overflow")?, count, None)
        };
        let vertex = ResourceRef::new(vertex.resource.ptr());
        let target = ResourceRef::new(self.target.as_ref().ok_or("no render target")?.ptr());
        let (_, width, height, _) =
            crate::resource::metal_texture(target.ptr()).ok_or("invalid render target")?;
        let viewport = self.viewport.ok_or("missing viewport")?;
        let scissor = self.scissor.ok_or("missing scissor")?;
        let clip = |value: i32, limit: u32| i64::from(value).clamp(0, i64::from(limit)) as u32;
        let left = clip(scissor.left, width);
        let top = clip(scissor.top, height);
        let right = clip(scissor.right, width);
        let bottom = clip(scissor.bottom, height);
        let scissor = [left, top, right - left, bottom - top];
        let viewport = [
            f64::from(viewport.x), f64::from(viewport.y),
            f64::from(viewport.width), f64::from(viewport.height),
            f64::from(viewport.min), f64::from(viewport.max),
        ];
        let prepared = pipeline.lock().unwrap_or_else(|e| e.into_inner());
        if color_format(target.ptr()) != Some(prepared.color_format) {
            return Err("pipeline color format does not match render target".into());
        }
        if let Some(bound) = &self.depth_target {
            let (_, dw, dh, _) = crate::resource::metal_depth_texture(bound.ptr())
                .ok_or("invalid depth target")?;
            if (dw, dh) != (width, height) {
                return Err("depth and color target sizes differ".into());
            }
        }
        let depth = if prepared.depth.is_some() {
            Some(ResourceRef::new(self.depth_target.as_ref().ok_or("missing depth target")?.ptr()))
        } else {
            None
        };
        if self.graphics_roots.iter().enumerate().any(|(index, root)| {
            !matches!(root, Root::None)
                && !prepared
                    .user_data
                    .iter()
                    .any(|u| usize::from(u.slot) == index)
        }) {
            return Err("unsupported graphics root parameter".into());
        }
        let mut constants = vec![0; prepared.dwords];
        for u in &prepared.user_data {
            constants[u.index as usize] = match (u.root, self.graphics_roots.get(u.slot as usize)) {
                (RootType::Constants, Some(Root::Constants(words))) => words
                    .get(u.dword as usize)
                    .copied()
                    .flatten()
                    .ok_or("missing graphics root constant dword")?,
                (RootType::Table, Some(Root::Table(_))) => 0,
                _ => return Err("missing or unsupported graphics root argument".into()),
            };
        }
        let sampled = match (prepared.texture_slot, prepared.sampler_slot) {
            (Some(texture_slot), Some(sampler_slot)) => {
                let view_heap = self.heap.clone().ok_or("missing view heap")?;
                let sampler_heap = self.sampler_heap.clone().ok_or("missing sampler heap")?;
                let table = |slot: u8, heap: &HeapRef| -> Result<u64, String> {
                    let Some(Root::Table(handle)) = self.graphics_roots.get(slot as usize) else {
                        return Err("missing graphics descriptor table".into());
                    };
                    let heap = descriptor::heap(heap.ptr());
                    if !heap.owns_memory() || *handle != heap.start {
                        return Err(
                            "only owned descriptor heaps at index zero are supported".into()
                        );
                    }
                    Ok(*handle)
                };
                let view_handle = table(texture_slot, &view_heap)?;
                let sampler_handle = table(sampler_slot, &sampler_heap)?;
                let view = descriptor::view(view_handle).ok_or("missing SRV descriptor")?;
                let sampler =
                    descriptor::view(sampler_handle).ok_or("missing sampler descriptor")?;
                let View::Shader(texture, desc) = &view else {
                    return Err("expected a texture SRV".into());
                };
                let texture = *texture as This;
                let format = crate::resource::metal_sampled_format(texture).ok_or("unsupported sampled texture format")?;
                let total_mips = crate::resource::metal_sampled_mip_count(texture)
                    .ok_or("unsupported sampled texture layout")?;
                let (first_mip, mip_count) = if let Some(desc) = desc {
                    if desc[0] != format || desc[1] != 4 || desc[2] != 0x1688
                        || desc[6] != 0 || desc[7] != 0
                    {
                        return Err("unsupported texture SRV description".into());
                    }
                    let first = desc[4];
                    let remaining = total_mips.checked_sub(first).filter(|&n| n != 0)
                        .ok_or("texture SRV first mip is out of range")?;
                    let count = if desc[5] == u32::MAX { remaining } else { desc[5] };
                    if count == 0 || count > remaining {
                        return Err("texture SRV mip range is out of bounds".into());
                    }
                    (first, count)
                } else {
                    (0, total_mips)
                };
                let View::Sampler(words) = sampler else {
                    return Err("expected a standard sampler".into());
                };
                if words[1..11] != [3, 3, 3, 0, 1, 8, 0, 0, 0, 0] {
                    return Err("unsupported sampler settings".into());
                }
                let min_lod = f32::from_bits(words[11]);
                let max_lod = f32::from_bits(words[12]);
                if !min_lod.is_finite() || !max_lod.is_finite()
                    || min_lod < 0.0 || min_lod > max_lod
                {
                    return Err("unsupported sampler LOD range".into());
                }
                let filter = match words[0] {
                    0 => victory_metal::TextureFilter::Nearest,
                    0x15 => victory_metal::TextureFilter::Linear,
                    _ => return Err("unsupported sampler filter".into()),
                };
                Some(Sampled {
                    _view_heap: view_heap,
                    _sampler_heap: sampler_heap,
                    view_handle,
                    sampler_handle,
                    view,
                    sampler: View::Sampler(words),
                    texture: ResourceRef::new(texture),
                    filter,
                    first_mip,
                    mip_count,
                    min_lod,
                    max_lod,
                })
            }
            (None, None) => None,
            _ => return Err("incomplete texture/sampler binding".into()),
        };
        drop(prepared);
        self.use_state(vertex.ptr(), VERTEX_BUFFER)?;
        if let Some(indexed) = &indexed {
            self.use_state(indexed.resource.ptr(), INDEX_BUFFER)?;
        }
        self.use_state(target.ptr(), RENDER_TARGET)?;
        if let Some(depth) = &depth {
            self.use_state(depth.ptr(), DEPTH_WRITE)?;
        }
        if let Some(sampled) = &sampled {
            self.use_state(sampled.texture.ptr(), PIXEL_SHADER_RESOURCE)?;
        }
        self.commands.push(Command::Draw {
            pipeline,
            vertex,
            vertex_offset,
            vertices,
            vertex_stride,
            indexed,
            constants,
            sampled,
            viewport,
            scissor,
            depth,
            target,
        });
        Ok(())
    }
    pub(crate) fn read_texture(
        &mut self,
        target: This,
        buffer: This,
        footprint: crate::resource::PlacedFootprint,
    ) {
        if !self.open() {
            return;
        }
        let Some((_, width, height, _)) = crate::resource::metal_texture(target) else {
            self.reject("unsupported texture readback");
            return;
        };
        let bytes = u64::from(footprint.row_pitch)
            .checked_mul(u64::from(height.saturating_sub(1)))
            .and_then(|n| n.checked_add(u64::from(width) * 4));
        let size = crate::resource::buffer_bytes(buffer).map(|(_, size)| size);
        if Some(footprint.format) != crate::resource::metal_color_format(target)
            || footprint.width != width
            || footprint.height != height
            || footprint.depth != 1
            || footprint.row_pitch % 256 != 0
            || u64::from(footprint.row_pitch) < u64::from(width) * 4
            || footprint.offset % 512 != 0
            || bytes
                .and_then(|n| footprint.offset.checked_add(n))
                .zip(size)
                .is_none_or(|(end, size)| end > size)
        {
            self.reject("invalid texture readback footprint");
            return;
        }
        if let Err(error) = self
            .use_state(target, COPY_SOURCE)
            .and_then(|()| self.use_state(buffer, COPY_DEST))
        {
            self.reject(&error);
            return;
        }
        self.commands.push(Command::ReadTexture {
            target: ResourceRef::new(target),
            buffer: ResourceRef::new(buffer),
            offset: footprint.offset,
            pitch: footprint.row_pitch,
        });
    }
    pub(crate) fn set_heaps(&mut self, heaps: &[This]) {
        if !self.open() {
            return;
        }
        let mut view = None;
        let mut sampler = None;
        for &heap in heaps {
            if heap.is_null() {
                self.reject("null descriptor heap");
                return;
            }
            let info = descriptor::heap(heap);
            if info.is_shader_visible_view_heap() && view.is_none() {
                view = Some(Arc::new(HeapRef::new(heap)));
            } else if info.is_shader_visible_sampler_heap() && sampler.is_none() {
                sampler = Some(Arc::new(HeapRef::new(heap)));
            } else {
                self.reject("unsupported or duplicate descriptor heap");
                return;
            }
        }
        let changed = self.heap.as_ref().map(|h| h.ptr()) != view.as_ref().map(|h| h.ptr())
            || self.sampler_heap.as_ref().map(|h| h.ptr()) != sampler.as_ref().map(|h| h.ptr());
        if changed {
            for root in self.roots.iter_mut().chain(self.graphics_roots.iter_mut()) {
                if matches!(root, Root::Table(_)) {
                    *root = Root::None;
                }
            }
        }
        self.heap = view;
        self.sampler_heap = sampler;
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
    pub(crate) fn set_compute_cbv(&mut self, slot: u32, address: u64) {
        if !self.open() {
            return;
        }
        if slot >= 64 || address % 16 != 0 {
            self.reject("invalid root CBV slot or alignment");
            return;
        }
        let Some((resource, offset)) = crate::resource::metal_buffer_at(address, 16) else {
            self.reject("root CBV address is not a live committed buffer");
            return;
        };
        let resource = ResourceRef(resource as usize);
        let Some((_, size)) = crate::resource::buffer_bytes(resource.ptr()) else {
            self.reject("invalid root CBV buffer");
            return;
        };
        if self.roots.len() <= slot as usize {
            self.roots.resize_with(slot as usize + 1, Root::default);
        }
        self.roots[slot as usize] = Root::Cbv {
            resource, offset, size: (size - offset).min(65536),
        };
    }
    pub(crate) fn set_graphics_table(&mut self, slot: u32, handle: u64) {
        if !self.open() {
            return;
        }
        if slot >= 64 {
            self.reject("graphics root slot out of range");
            return;
        }
        if self.graphics_roots.len() <= slot as usize {
            self.graphics_roots
                .resize_with(slot as usize + 1, Root::default);
        }
        self.graphics_roots[slot as usize] = Root::Table(handle);
    }
    pub(crate) fn set_constants(&mut self, slot: u32, values: &[u32], offset: u32) {
        if !self.open() {
            return;
        }
        if let Err(error) = write_constants(&mut self.roots, slot, values, offset) {
            self.reject(error);
        }
    }
    pub(crate) fn set_graphics_constants(&mut self, slot: u32, values: &[u32], offset: u32) {
        if !self.open() {
            return;
        }
        if let Err(error) = write_constants(&mut self.graphics_roots, slot, values, offset) {
            self.reject(error);
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
        let cbv = if let Some((slot, required, _)) = p.root_cbv {
            let Some(Root::Cbv { resource: cbv, offset, size }) = self.roots.get(slot as usize) else {
                return Err("missing compute root CBV".into());
            };
            if *size < required || cbv.ptr() == resource {
                return Err("root CBV range is too small or aliases the UAV".into());
            }
            Some((ResourceRef::new(cbv.ptr()), *offset, required))
        } else {
            None
        };
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
        if let Some((cbv, _, _)) = &cbv {
            self.use_state(cbv.ptr(), VERTEX_BUFFER)?;
        }
        let resource = ResourceRef::new(resource);
        drop(p);
        self.commands.push(Command::Dispatch {
            pipeline,
            _heap: bound_heap,
            handle,
            resource,
            cbv,
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
                Command::ClearTarget {
                    target,
                    color,
                    rects,
                } => {
                    submission.access(target.ptr(), RENDER_TARGET)?;
                    let (texture, width, height, pitch) =
                        crate::resource::metal_texture(target.ptr())
                            .ok_or("invalid render target")?;
                    let row = (width as usize).checked_mul(4).ok_or("image row overflow")?;
                    let len = (height as usize - 1)
                        .checked_mul(pitch)
                        .and_then(|n| n.checked_add(row))
                        .ok_or("image size overflow")?;
                    // SAFETY: the retained linear texture contains all rows of
                    // its validated RGBA8 layout, including inter-row padding.
                    let pixels = unsafe { std::slice::from_raw_parts_mut(texture, len) };
                    victory_metal::clear_color_float(color_format(target.ptr()).ok_or("invalid color format")?, width, height, pitch, pixels, *color, rects)
                        .map_err(|e| e.to_string())?;
                }
                Command::ClearDepth { target, depth, rects } => {
                    submission.access(target.ptr(), DEPTH_WRITE)?;
                    let (texture, width, height, pitch) =
                        crate::resource::metal_depth_texture(target.ptr()).ok_or("invalid depth target")?;
                    let row = (width as usize).checked_mul(4).ok_or("depth row overflow")?;
                    let len = (height as usize - 1).checked_mul(pitch)
                        .and_then(|n| n.checked_add(row)).ok_or("depth size overflow")?;
                    // SAFETY: retained depth allocation includes each validated padded row.
                    let pixels = unsafe { std::slice::from_raw_parts_mut(texture, len) };
                    victory_metal::clear_depth32(width, height, pitch, pixels, *depth, rects)
                        .map_err(|e| e.to_string())?;
                }
                Command::Draw {
                    pipeline,
                    vertex,
                    vertex_offset,
                    vertices,
                    vertex_stride,
                    indexed,
                    constants,
                    sampled,
                    viewport,
                    scissor,
                    depth,
                    target,
                } => {
                    submission.access(vertex.ptr(), VERTEX_BUFFER)?;
                    let binding = if let Some(sampled) = sampled {
                        if descriptor::view(sampled.view_handle).as_ref() != Some(&sampled.view)
                            || descriptor::view(sampled.sampler_handle).as_ref()
                                != Some(&sampled.sampler)
                        {
                            return Err("sampled descriptors changed after recording".into());
                        }
                        submission.access(sampled.texture.ptr(), PIXEL_SHADER_RESOURCE)?;
                        let (width, height) = crate::resource::metal_sampled_copy_mips(
                            sampled.texture.ptr(), sampled.first_mip, sampled.mip_count,
                            &mut self.texture_scratch,
                        ).ok_or("failed to stage sampled texture mips")?;
                        Some(crate::metal_graphics::Texture {
                            width,
                            height,
                            levels: sampled.mip_count,
                            pixels: self.texture_scratch.as_slice(),
                            filter: sampled.filter,
                            format: sampled_format(sampled.texture.ptr()).ok_or("invalid sampled texture format")?,
                            min_lod: sampled.min_lod,
                            max_lod: sampled.max_lod,
                        })
                    } else {
                        None
                    };
                    submission.access(target.ptr(), RENDER_TARGET)?;
                    let (buffer, size) = crate::resource::buffer_bytes(vertex.ptr())
                        .ok_or("invalid vertex buffer")?;
                    let vertex_bytes = u64::from(*vertices) * u64::from(*vertex_stride);
                    if vertex_offset
                        .checked_add(vertex_bytes)
                        .is_none_or(|end| end > size)
                    {
                        return Err("vertex buffer range changed".into());
                    }
                    let (texture, width, height, pitch) =
                        crate::resource::metal_texture(target.ptr())
                            .ok_or("invalid render target")?;
                    let row =
                        usize::try_from(u64::from(width) * 4).map_err(|_| "image row overflow")?;
                    let len = row
                        .checked_mul(height as usize)
                        .ok_or("image size overflow")?;
                    self.scratch.resize(len, 0);
                    for y in 0..height as usize {
                        // SAFETY: texture layout and tightly packed staging ranges were checked.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                texture.add(y * pitch),
                                self.scratch.as_mut_ptr().add(y * row),
                                row,
                            )
                        };
                    }
                    // SAFETY: the buffer is retained, the offset is aligned and the range was checked.
                    let verts = unsafe {
                        let ptr = buffer.add(*vertex_offset as usize);
                        if *vertex_stride == 24 {
                            crate::metal_graphics::Vertices::PositionUv(
                                std::slice::from_raw_parts(ptr.cast::<[f32; 6]>(), *vertices as usize))
                        } else {
                            crate::metal_graphics::Vertices::Position(
                                std::slice::from_raw_parts(ptr.cast::<[f32; 4]>(), *vertices as usize))
                        }
                    };
                    let indices = if let Some(indexed) = indexed {
                        submission.access(indexed.resource.ptr(), INDEX_BUFFER)?;
                        let (data, size) = crate::resource::buffer_bytes(indexed.resource.ptr())
                            .ok_or("invalid index buffer")?;
                        let bytes = u64::from(indexed.count) * u64::from(indexed.element_size);
                        if indexed.offset.checked_add(bytes).is_none_or(|end| end > size) {
                            return Err("index buffer range changed".into());
                        }
                        // SAFETY: the retained buffer range and element alignment were validated.
                        let data = unsafe { data.add(indexed.offset as usize) };
                        let values = unsafe {
                            if indexed.element_size == 2 {
                                victory_metal::IndexData::U16(std::slice::from_raw_parts(data.cast(), indexed.count as usize))
                            } else {
                                victory_metal::IndexData::U32(std::slice::from_raw_parts(data.cast(), indexed.count as usize))
                            }
                        };
                        Some((values, indexed.base_vertex))
                    } else {
                        None
                    };
                    let depth_layout = if let Some(depth) = depth {
                        submission.access(depth.ptr(), DEPTH_WRITE)?;
                        let (data, dw, dh, depth_pitch) = crate::resource::metal_depth_texture(depth.ptr())
                            .ok_or("invalid depth target")?;
                        if (dw, dh) != (width, height) {
                            return Err("depth and color target sizes differ".into());
                        }
                        self.depth_scratch.resize(len / 4, 0.0);
                        for y in 0..height as usize {
                            // SAFETY: both the retained padded texture and tight staging buffer contain this row.
                            unsafe {
                                std::ptr::copy_nonoverlapping(data.add(y * depth_pitch), self.depth_scratch.as_mut_ptr().cast::<u8>().add(y * row), row);
                            }
                        }
                        Some((data, depth_pitch))
                    } else {
                        None
                    };
                    pipeline.lock().unwrap_or_else(|e| e.into_inner()).draw(
                        width,
                        height,
                        verts,
                        indices,
                        *viewport,
                        *scissor,
                        depth_layout.map(|_| self.depth_scratch.as_mut_slice()),
                        constants,
                        binding,
                        &mut self.scratch,
                    )?;
                    if let Some((data, depth_pitch)) = depth_layout {
                        for y in 0..height as usize {
                            // SAFETY: same checked depth and staging row ranges as above.
                            unsafe {
                                std::ptr::copy_nonoverlapping(self.depth_scratch.as_ptr().cast::<u8>().add(y * row), data.add(y * depth_pitch), row);
                            }
                        }
                    }
                    for y in 0..height as usize {
                        // SAFETY: same checked texture and staging ranges as above.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                self.scratch.as_ptr().add(y * row),
                                texture.add(y * pitch),
                                row,
                            )
                        };
                    }
                }
                Command::ReadTexture {
                    target,
                    buffer,
                    offset,
                    pitch,
                } => {
                    submission.access(target.ptr(), COPY_SOURCE)?;
                    submission.access(buffer.ptr(), COPY_DEST)?;
                    let (texture, width, height, texture_pitch) =
                        crate::resource::metal_texture(target.ptr())
                            .ok_or("invalid readback texture")?;
                    let (dst, size) = crate::resource::buffer_bytes(buffer.ptr())
                        .ok_or("invalid readback buffer")?;
                    let row = u64::from(width) * 4;
                    let end = offset
                        .checked_add(u64::from(*pitch) * u64::from(height - 1))
                        .and_then(|v| v.checked_add(row))
                        .ok_or("readback range overflow")?;
                    if end > size {
                        return Err("readback exceeds buffer".into());
                    }
                    for y in 0..height as usize {
                        // SAFETY: both resources are retained and all row ranges were checked.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                texture.add(y * texture_pitch),
                                dst.add(*offset as usize + y * *pitch as usize),
                                row as usize,
                            )
                        };
                    }
                }
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
                    cbv,
                    constants,
                    groups,
                } => {
                    if !matches!(descriptor::view(*handle), Some(View::Unordered(r, 0, None)) if r == resource.0)
                    {
                        return Err("UAV descriptor changed after recording".into());
                    }
                    submission.access(resource.ptr(), UAV)?;
                    if let Some((cbv, _, _)) = cbv.as_ref() {
                        submission.access(cbv.ptr(), VERTEX_BUFFER)?;
                    }
                    let (ptr, len) = crate::resource::buffer_bytes(resource.ptr())
                        .ok_or("UAV resource no longer valid")?;
                    let len = usize::try_from(len).map_err(|_| "UAV buffer is too large")?;
                    // SAFETY: the command holds a reference to the live resource,
                    // and queue submission is synchronous and exclusive.
                    let buffer = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
                    let mut p = pipeline.lock().unwrap_or_else(|e| e.into_inner());
                    let threads = p.threads;
                    if let Some((cbv, offset, size)) = cbv.as_ref() {
                        let (data, available) = crate::resource::buffer_bytes(cbv.ptr())
                            .ok_or("root CBV resource no longer valid")?;
                        if offset.checked_add(*size).is_none_or(|end| end > available) {
                            return Err("root CBV range changed".into());
                        }
                        let (_, _, argument_offset) = p.root_cbv.ok_or("missing reflected root CBV")?;
                        // SAFETY: retained CBV is distinct from the writable UAV and the range is checked.
                        let data = unsafe { std::slice::from_raw_parts(data.add(*offset as usize), *size as usize) };
                        p.compute.dispatch_with_root_cbv(buffer, constants, *groups, threads, data, argument_offset)?;
                    } else {
                        p.compute.dispatch(buffer, constants, *groups, threads)?;
                    }
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

fn write_constants(
    roots: &mut Vec<Root>,
    slot: u32,
    values: &[u32],
    offset: u32,
) -> Result<(), &'static str> {
    let end = (offset as usize)
        .checked_add(values.len())
        .ok_or("constant range overflow")?;
    if slot >= 64 || end > 64 {
        return Err("constant range out of bounds");
    }
    if roots.len() <= slot as usize {
        roots.resize_with(slot as usize + 1, Root::default);
    }
    if !matches!(roots[slot as usize], Root::Constants(_)) {
        roots[slot as usize] = Root::Constants(Vec::new());
    }
    let Root::Constants(words) = &mut roots[slot as usize] else {
        unreachable!()
    };
    words.resize(words.len().max(end), None);
    for (word, value) in words[offset as usize..end].iter_mut().zip(values) {
        *word = Some(*value);
    }
    Ok(())
}
