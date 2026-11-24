//! Restricted vertex/pixel pipelines for the offscreen Metal renderer.
use std::ffi::CStr;
use victory_shader::{Container, FourCC, HwStage, emit, io, ir, native, resource};

pub(crate) struct Prepared {
    vertex: Vec<u8>,
    pixel: Vec<u8>,
    pub(crate) user_data: Vec<emit::UserData>,
    pub(crate) dwords: usize,
    pub(crate) texture_slot: Option<u8>,
    pub(crate) sampler_slot: Option<u8>,
    pub(crate) depth: Option<victory_metal::DepthState>,
    renderer: Option<(u32, u32, victory_metal::Renderer)>,
}

// The renderer owns Metal objects and is only accessed through the pipeline's
// mutex; its synchronous draws complete before another thread can use it.
unsafe impl Send for Prepared {}

impl Prepared {
    pub(crate) fn new(vertex: &[u8], pixel: &[u8], depth: Option<victory_metal::DepthState>) -> Result<Self, String> {
        let (vertex, _, _, _, _) = convert(vertex, HwStage::Vs)?;
        let (pixel, user_data, dwords, texture_slot, sampler_slot) = convert(pixel, HwStage::Ps)?;
        Ok(Self {
            vertex,
            pixel,
            user_data,
            dwords,
            texture_slot,
            sampler_slot,
            depth,
            renderer: None,
        })
    }

    pub(crate) fn draw(
        &mut self,
        width: u32,
        height: u32,
        vertices: &[[f32; 4]],
        indices: Option<(victory_metal::IndexData<'_>, i32)>,
        viewport: [f64; 6],
        scissor: [u32; 4],
        mut depth: Option<&mut [f32]>,
        constants: &[u32],
        texture: Option<(u32, u32, &[u8], victory_metal::TextureFilter)>,
        pixels: &mut [u8],
    ) -> Result<(), String> {
        if texture.is_some() != self.texture_slot.is_some() {
            return Err("texture binding does not match pixel shader".into());
        }
        if depth.is_some() != self.depth.is_some() {
            return Err("depth attachment does not match pipeline".into());
        }
        if !matches!(&self.renderer, Some((w, h, _)) if *w == width && *h == height) {
            let renderer = if let Some(state) = self.depth {
                victory_metal::Renderer::new_depth(
                    &self.vertex, "main", &self.pixel, "main", width, height,
                    self.dwords as u32, self.texture_slot.is_some(), state,
                )
            } else {
                victory_metal::Renderer::new(
                    &self.vertex, "main", &self.pixel, "main", width, height,
                    self.dwords as u32, self.texture_slot.is_some(),
                )
            }.map_err(|e| e.to_string())?;
            self.renderer = Some((width, height, renderer));
        }
        let renderer = &mut self.renderer.as_mut().unwrap().2;
        renderer.set_viewport_scissor(viewport, scissor).map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref() {
            renderer.set_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some((width, height, texels, filter)) = texture {
            renderer
                .set_fragment_texture(width, height, texels, filter)
                .map_err(|e| e.to_string())?;
        }
        match indices {
            Some((indices, base)) => renderer.draw_indexed_preserving(vertices, indices, base, constants, pixels),
            None => renderer.draw_preserving(vertices, constants, pixels),
        }.map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref_mut() {
            renderer.read_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn word(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes = data
        .get(at..at.checked_add(4).ok_or("offset overflow")?)
        .ok_or("truncated metadata")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

// Signature records are the standard 32-byte ISG1/OSG1 layout. Requiring a
// single full register prevents silently dropping interpolants or outputs.
fn signature(
    container: &Container<'_>,
    part: FourCC,
    expected: Option<(&[u8], u32)>,
) -> Result<(), String> {
    let Some(data) = container.part(part).map(|p| p.data) else {
        return if expected.is_none() {
            Ok(())
        } else {
            Err(format!("missing {part} signature"))
        };
    };
    let count = word(data, 0)?;
    if expected.is_none() && count == 0 {
        return Ok(());
    }
    let (name, system) = expected.ok_or_else(|| format!("unexpected {part} signature"))?;
    if count != 1 {
        return Err(format!("{part} must have exactly one element"));
    }
    let at = word(data, 4)? as usize;
    let name_at = word(data, at + 4)? as usize;
    let bytes = data.get(name_at..).ok_or("signature name out of bounds")?;
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or("unterminated signature name")?;
    if !bytes[..end].eq_ignore_ascii_case(name)
        || word(data, at)? != 0
        || word(data, at + 8)? != 0
        || word(data, at + 12)? != system
        || word(data, at + 16)? != 3
        || word(data, at + 20)? != 0
        || data.get(at + 24) != Some(&15)
        || word(data, at + 28)? != 0
    {
        return Err(format!("unsupported {part} signature"));
    }
    Ok(())
}

fn convert(
    bytes: &[u8],
    stage: HwStage,
) -> Result<(Vec<u8>, Vec<emit::UserData>, usize, Option<u8>, Option<u8>), String> {
    let c = Container::parse(bytes).map_err(|e| e.to_string())?;
    let dxil = c.part(FourCC::DXIL).ok_or("missing DXIL part")?;
    let programs = victory_shader::native_programs(dxil.data).map_err(|e| e.to_string())?;
    if programs.len() != 1 || programs[0].stage != stage {
        return Err("expected exactly one native program of the requested stage".into());
    }
    let p = &programs[0];
    let mut texture = None;
    let mut sampler = None;
    for used in &p.used {
        let target = match used.kind {
            native::ResourceType::Srv if used.dwords == 8 => &mut texture,
            native::ResourceType::Sampler if used.dwords == 4 => &mut sampler,
            _ => return Err("unsupported pixel resource".into()),
        };
        if stage != HwStage::Ps
            || target.is_some()
            || used.root != native::RootType::Table
            || used.table_offset_dwords != 0
            || used.lower_bound != 0
            || used.stride != 0
            || used.slot >= 64
        {
            return Err("only one texture and sampler at table index zero are supported".into());
        }
        *target = Some(used);
    }
    if texture.is_some() != sampler.is_some()
        || texture.zip(sampler).is_some_and(|(t, s)| t.slot == s.slot)
    {
        return Err("texture and sampler must use distinct tables".into());
    }
    let mut constants_slot = None;
    for root in &p.layout {
        if stage == HwStage::Vs || root.slot >= 64 {
            return Err("unsupported root layout".into());
        }
        match root.root {
            native::RootType::Constants => {
                if constants_slot.is_some_and(|slot| slot != root.slot)
                    || texture.is_some_and(|t| t.slot == root.slot)
                    || sampler.is_some_and(|s| s.slot == root.slot)
                {
                    return Err("constants must use a distinct root slot".into());
                }
                constants_slot = Some(root.slot);
            }
            native::RootType::Table => {
                if root.sgprs != 1
                    || root.table_offset_dwords != 0
                    || root.base_register != 0
                    || ![texture, sampler]
                        .into_iter()
                        .flatten()
                        .any(|u| u.slot == root.slot && u.user_data == root.user_data)
                {
                    return Err("unsupported descriptor table layout".into());
                }
            }
            _ => return Err("unsupported root layout".into()),
        }
    }
    for (i, root) in p.layout.iter().enumerate() {
        let start = u32::from(root.user_data);
        let end = start
            .checked_add(root.sgprs)
            .ok_or("root layout overflow")?;
        if end > 64
            || p.layout[..i].iter().any(|other| {
                let other_start = u32::from(other.user_data);
                let other_end = other_start.saturating_add(other.sgprs);
                root.slot != other.slot && start < other_end && other_start < end
            })
        {
            return Err("overlapping or out-of-range root user data".into());
        }
    }
    for used in [texture, sampler].into_iter().flatten() {
        if p.layout
            .iter()
            .filter(|r| {
                r.root == native::RootType::Table
                    && r.slot == used.slot
                    && r.user_data == used.user_data
            })
            .count()
            != 1
        {
            return Err("missing or ambiguous descriptor table".into());
        }
    }
    match (&p.io, stage) {
        (native::StageIo::Vs { inputs, outputs }, HwStage::Vs)
            if inputs.len() == 1
                && inputs[0].usage == native::Usage::Generic
                && inputs[0].row == 0
                && inputs[0].mask == 15
                && outputs.is_empty() =>
        {
            signature(&c, FourCC::ISG1, Some((b"POSITION", 0)))?;
            signature(&c, FourCC::OSG1, Some((b"SV_Position", 1)))?;
        }
        (native::StageIo::Ps { inputs, .. }, HwStage::Ps) if inputs.is_empty() => {
            // Position may be declared without being used by the pixel shader.
            let ps_input = c.part(FourCC::ISG1).map(|p| p.data);
            if ps_input.map(|d| word(d, 0)).transpose()?.unwrap_or(0) == 0 {
                signature(&c, FourCC::ISG1, None)?;
            } else {
                signature(&c, FourCC::ISG1, Some((b"SV_Position", 1)))?;
            }
            signature(&c, FourCC::OSG1, Some((b"SV_Target", 64)))?;
        }
        _ => return Err("unsupported native stage inputs or outputs".into()),
    }
    let program = ir::build(&p.patched_code(), p.wave64, p.vgprs).map_err(|e| e.to_string())?;
    let traced = resource::track(&program, &p.layout, p.stage).map_err(|e| e.to_string())?;
    if texture.is_none() {
        if !traced.scalar.is_empty() || !traced.descriptors.is_empty() {
            return Err("shader accesses resources".into());
        }
    } else {
        let mut loads = Vec::new();
        for (role, used) in [
            (resource::Role::Resource, texture.unwrap()),
            (resource::Role::Sampler, sampler.unwrap()),
        ] {
            let mut source = None;
            for descriptor in traced.descriptors.iter().filter(|d| d.role == role) {
                let resource::Source::Loaded { load, first: 0 } = descriptor.source else {
                    return Err("descriptor must be loaded directly from its table".into());
                };
                if source.is_some_and(|previous| previous != load) {
                    return Err("multiple descriptor loads are unsupported".into());
                }
                let inst = &program.blocks[descriptor.site.0].insts[descriptor.site.1];
                if !matches!(
                    &inst.fields,
                    victory_shader::gfx10::Fields::Image { dim: 1, .. }
                ) || !matches!(
                    inst.name(),
                    "image_sample"
                        | "image_sample_lz"
                        | "image_sample_l"
                        | "image_sample_b"
                        | "image_sample_d"
                ) {
                    return Err("only non-comparison 2D texture sampling is supported".into());
                }
                source = Some(load);
            }
            let load = source.ok_or("missing texture or sampler access")?;
            let load_inst = &program.blocks[load.0].insts[load.1];
            let expected_load = if role == resource::Role::Resource {
                "s_load_dwordx8"
            } else {
                "s_load_dwordx4"
            };
            if load_inst.name() != expected_load {
                return Err("unsupported descriptor load width".into());
            }
            let (_, addr) = traced
                .scalar
                .iter()
                .find(|(site, _)| *site == load)
                .ok_or("missing descriptor load")?;
            let root = p.layout.get(addr.param).ok_or("invalid descriptor table")?;
            if addr.offset != 0
                || addr.dynamic.is_some()
                || root.root != native::RootType::Table
                || root.slot != used.slot
                || root.user_data != used.user_data
            {
                return Err("only static descriptor index zero is supported".into());
            }
            loads.push(load);
        }
        if loads[0] == loads[1]
            || traced.scalar.len() != 2
            || traced.scalar.iter().any(|(site, _)| !loads.contains(site))
            || traced.descriptors.iter().any(|d| {
                !traced
                    .descriptors
                    .iter()
                    .any(|other| other.site == d.site && other.role != d.role)
            })
        {
            return Err("unexpected shader resource access".into());
        }
    }
    let raised = io::raise(&program, p).map_err(|e| e.to_string())?;
    if raised.inputs.iter().any(|(_, input)| match stage {
        HwStage::Vs => !matches!(input, io::Input::Attribute { row: 0, .. }),
        HwStage::Ps => !matches!(input, io::Input::Position(_)),
        _ => true,
    }) {
        return Err("unsupported shader system inputs".into());
    }
    let valid_target = |t| match stage {
        HwStage::Vs => matches!(
            t,
            io::Target::Position(0) | io::Target::Unread | io::Target::Null
        ),
        HwStage::Ps => matches!(t, io::Target::Color(0) | io::Target::Null),
        _ => false,
    };
    if !raised.interps.is_empty()
        || raised.exports.iter().any(|e| !valid_target(e.target))
        || !raised.exports.iter().any(|e| match stage {
            HwStage::Vs => matches!(e.target, io::Target::Position(0)),
            _ => matches!(e.target, io::Target::Color(0)),
        })
    {
        return Err("unsupported shader exports or interpolation".into());
    }
    let host = emit::translate(bytes).map_err(|e| e.to_string())?;
    if host.stage != stage
        || (stage == HwStage::Vs && !host.user_data.is_empty())
        || host.user_data.iter().any(|u| {
            u.slot >= 64
                || u.dword >= 64
                || match u.root {
                    native::RootType::Constants => Some(u.slot) != constants_slot,
                    native::RootType::Table => {
                        u.dword != 0
                            || ![texture, sampler]
                                .into_iter()
                                .flatten()
                                .any(|r| r.slot == u.slot)
                    }
                    _ => true,
                }
        })
        || !host.samplers.is_empty()
        || !host.static_samplers.is_empty()
        || !host.root_cbvs.is_empty()
        || !host.root_srvs.is_empty()
    {
        return Err("translated shader has unsupported bindings".into());
    }
    let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
    let dwords = host.user_data_dwords() as usize;
    if dwords > 64 {
        return Err("fragment user-data buffer is too large".into());
    }
    let metallib = if stage == HwStage::Vs {
        victory_metal::converter::convert(
            std::path::Path::new(&path),
            &host.dxil,
            victory_metal::converter::Stage::Vertex,
        )?
    } else if texture.is_some() {
        victory_metal::converter::convert_sampled_fragment(
            std::path::Path::new(&path),
            &host.dxil,
            dwords != 0,
        )?
    } else {
        victory_metal::converter::convert_fragment_with_b0(
            std::path::Path::new(&path),
            &host.dxil,
            dwords != 0,
        )?
    };
    Ok((
        metallib,
        host.user_data,
        dwords,
        texture.map(|u| u.slot),
        sampler.map(|u| u.slot),
    ))
}

#[derive(Default)]
pub(crate) struct StreamState {
    seen: u32,
    error: Option<String>,
    depth: Option<victory_metal::DepthState>,
    depth_format: u32,
}

impl StreamState {
    pub(crate) fn reject(&mut self, reason: &str) {
        if self.error.is_none() {
            self.error = Some(reason.into());
        }
    }

    /// `at` points to a complete subobject and any referenced input layout is
    /// valid for the duration of pipeline creation, as required by D3D12.
    pub(crate) unsafe fn read(&mut self, kind: u32, at: *const u8, size: usize) {
        if self.seen & (1 << kind) != 0 {
            self.reject("duplicate pipeline subobject");
            return;
        }
        self.seen |= 1 << kind;
        let data = unsafe { std::slice::from_raw_parts(at, size) };
        let get = |offset| word(data, offset).unwrap();
        let valid = match kind {
            0 => data[8..16] == [0; 8],
            1 | 2 => true,
            // Solid, no culling, no bias, depth clipping on, no MSAA or conservative raster.
            10 => {
                get(4) == 3
                    && get(8) == 1
                    && get(16) == 0
                    && get(20) == 0
                    && get(24) == 0
                    && get(28) == 1
                    && get(32) == 0
                    && get(36) == 0
                    && get(40) == 0
                    && get(44) == 0
            }
            // Depth enable, write mask and comparison; stencil is unsupported.
            11 => {
                let (enabled, write, compare, stencil) = (get(4), get(8), get(12), get(16));
                if enabled == 1 && write <= 1 && (1..=8).contains(&compare) && stencil == 0 {
                    self.depth = Some(victory_metal::DepthState { write: write == 1, compare });
                    true
                } else {
                    enabled == 0 && stencil == 0
                }
            }
            12 => unsafe { input_layout(data) },
            13 => get(4) == 0,
            14 => get(4) == 3,
            15 => get(4) == 28 && (1..8).all(|i| get(4 + i * 4) == 0) && get(36) == 1,
            16 => { self.depth_format = get(4); matches!(self.depth_format, 0 | 40) },
            17 => get(4) == 1 && get(8) == 0,
            9 => get(4) == u32::MAX,
            _ => false,
        };
        if !valid {
            self.reject(&format!("unsupported graphics stream subobject {kind}"));
        }
    }

    pub(crate) fn finish(self) -> Result<Option<victory_metal::DepthState>, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let required = [1, 2, 10, 11, 12, 14, 15]
            .into_iter()
            .fold(0, |m, k| m | (1 << k));
        if self.seen & required != required {
            return Err("graphics stream lacks required shader, rasterizer, depth, input, topology or target state".into());
        }
        if (self.depth.is_some() && self.depth_format != 40)
            || (self.depth.is_none() && self.depth_format != 0)
        {
            return Err("depth format does not match depth state".into());
        }
        Ok(self.depth)
    }
}

unsafe fn input_layout(data: &[u8]) -> bool {
    let ptr = u64::from_le_bytes(data[8..16].try_into().unwrap()) as *const u8;
    if ptr.is_null() || word(data, 16).ok() != Some(1) {
        return false;
    }
    // D3D12_INPUT_ELEMENT_DESC: name pointer followed by six u32 fields.
    let name = unsafe { ptr.cast::<*const i8>().read_unaligned() };
    if name.is_null()
        || unsafe { CStr::from_ptr(name) }
            .to_bytes()
            .eq_ignore_ascii_case(b"POSITION")
            == false
    {
        return false;
    }
    let element = unsafe { std::slice::from_raw_parts(ptr, 32) };
    [0, 2, 0, 0, 0, 0]
        .iter()
        .enumerate()
        .all(|(i, expected)| word(element, 8 + i * 4).ok() == Some(*expected))
}
