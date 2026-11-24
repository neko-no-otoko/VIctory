//! Restricted vertex/pixel pipelines for the offscreen Metal renderer.
use std::ffi::CStr;
use victory_shader::{Container, FourCC, HwStage, emit, io, ir, native, resource};
#[derive(Clone, Copy)]
pub(crate) struct GraphicsState {
    depth: Option<victory_metal::DepthState>,
    blend: victory_metal::BlendState,
    input_uv: bool,
    color_format: victory_metal::ColorFormat,
}


pub(crate) struct Prepared {
    vertex: Vec<u8>,
    pixel: Vec<u8>,
    pub(crate) user_data: Vec<emit::UserData>,
    pub(crate) dwords: usize,
    pub(crate) texture_slot: Option<u8>,
    pub(crate) sampler_slot: Option<u8>,
    pub(crate) depth: Option<victory_metal::DepthState>,
    pub(crate) vertex_stride: u32,
    pub(crate) color_format: victory_metal::ColorFormat,
    blend: victory_metal::BlendState,
    renderer: Option<(u32, u32, victory_metal::Renderer)>,
}

// The renderer owns Metal objects and is only accessed through the pipeline's
// mutex; its synchronous draws complete before another thread can use it.
unsafe impl Send for Prepared {}
pub(crate) enum Vertices<'a> {
    Position(&'a [[f32; 4]]),
    PositionUv(&'a [[f32; 6]]),
}

pub(crate) struct Texture<'a> {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub pixels: &'a [u8],
    pub filter: victory_metal::TextureFilter,
    pub format: victory_metal::TextureFormat,
    pub min_lod: f32,
    pub max_lod: f32,
}


impl Prepared {
    pub(crate) fn new(vertex: &[u8], pixel: &[u8], graphics: GraphicsState) -> Result<Self, String> {
        let varying = varying_signatures(vertex, pixel, graphics.input_uv)?;
        let (vertex_host, _, _) = validate(vertex, HwStage::Vs, varying.map(|v| v.0), graphics.input_uv)?;
        let (pixel_host, texture_slot, sampler_slot) =
            validate(pixel, HwStage::Ps, varying.map(|v| v.1), false)?;
        let vertex = convert(&vertex_host, HwStage::Vs, false)?;
        let pixel = convert(&pixel_host, HwStage::Ps, texture_slot.is_some())?;
        let dwords = pixel_host.user_data_dwords() as usize;
        let user_data = pixel_host.user_data;
        Ok(Self {
            vertex,
            pixel,
            user_data,
            dwords,
            texture_slot,
            sampler_slot,
            depth: graphics.depth,
            blend: graphics.blend,
            vertex_stride: if graphics.input_uv { 24 } else { 16 },
            color_format: graphics.color_format,
            renderer: None,
        })
    }

    pub(crate) fn draw(
        &mut self,
        width: u32,
        height: u32,
        vertices: Vertices<'_>,
        indices: Option<(victory_metal::IndexData<'_>, i32)>,
        viewport: [f64; 6],
        scissor: [u32; 4],
        mut depth: Option<&mut [f32]>,
        constants: &[u32],
        texture: Option<Texture<'_>>,
        pixels: &mut [u8],
    ) -> Result<(), String> {
        if texture.is_some() != self.texture_slot.is_some() {
            return Err("texture binding does not match pixel shader".into());
        }
        if depth.is_some() != self.depth.is_some() {
            return Err("depth attachment does not match pipeline".into());
        }
        if !matches!(&self.renderer, Some((w, h, _)) if *w == width && *h == height) {
            let create = if self.vertex_stride == 24 {
                victory_metal::Renderer::new_configured_uv_format
            } else {
                victory_metal::Renderer::new_configured_format
            };
            let renderer = create(
                &self.vertex, "main", &self.pixel, "main", width, height,
                self.dwords as u32, self.texture_slot.is_some(), self.depth, self.blend, self.color_format,
            ).map_err(|e| e.to_string())?;
            self.renderer = Some((width, height, renderer));
        }
        let renderer = &mut self.renderer.as_mut().unwrap().2;
        renderer.set_viewport_scissor(viewport, scissor).map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref() {
            renderer.set_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some(texture) = texture {
            renderer
                .set_fragment_texture_mips(texture.width, texture.height, texture.levels, texture.pixels,
                    texture.filter, texture.format, texture.min_lod, texture.max_lod)
                .map_err(|e| e.to_string())?;
        }
        match (vertices, indices) {
            (Vertices::Position(v), Some((indices, base))) => renderer.draw_indexed_preserving(v, indices, base, constants, pixels),
            (Vertices::Position(v), None) => renderer.draw_preserving(v, constants, pixels),
            (Vertices::PositionUv(v), indices) => renderer.draw_uv_preserving(v, indices, constants, pixels),
        }.map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref_mut() {
            renderer.read_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
/// A restricted offscreen Wine graphics pipeline with POSITION-only vertices.
///
/// Only pixel shader root constants at slot 3 are supported; descriptor
/// tables, depth, blending and textures are not supported.
#[cfg(target_arch = "x86_64")]
pub struct WineMetalGraphics {
    prepared: Prepared,
}

#[cfg(target_arch = "x86_64")]
impl WineMetalGraphics {
    /// Translate and prepare the console vertex and pixel shaders.
    pub fn new(vertex: &[u8], pixel: &[u8]) -> Result<Self, String> {
        for (bytes, stage) in [(vertex, HwStage::Vs), (pixel, HwStage::Ps)] {
            if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
                return Err("invalid graphics shader length".into());
            }
            let container = Container::parse(bytes).map_err(|e| e.to_string())?;
            let dxil = container.part(FourCC::DXIL).ok_or("missing DXIL part")?;
            let programs = victory_shader::native_programs(dxil.data).map_err(|e| e.to_string())?;
            if programs.len() != 1 || programs[0].stage != stage
                || !programs[0].used.is_empty()
                || programs[0].layout.iter().any(|root| {
                    stage != HwStage::Ps || root.root != native::RootType::Constants || root.slot != 3
                })
            {
                return Err("unsupported graphics shader resource layout".into());
            }
        }
        let prepared = Prepared::new(vertex, pixel, GraphicsState {
            depth: None,
            blend: victory_metal::BlendState { enabled: false, write_mask: 15, alpha_source: false },
            input_uv: false,
            color_format: victory_metal::ColorFormat::Rgba8,
        })?;
        if prepared.texture_slot.is_some() || prepared.sampler_slot.is_some()
            || prepared.user_data.iter().any(|u| u.root != native::RootType::Constants || u.slot != 3)
        {
            return Err("unsupported graphics shader bindings".into());
        }
        Ok(Self { prepared })
    }

    /// Clear and draw one triangle, returning tightly packed RGBA8 pixels.
    pub fn draw(
        &mut self,
        width: u32,
        height: u32,
        clear: [f32; 4],
        vertices: &[[f32; 4]],
        root_constants: &[u32],
    ) -> Result<Vec<u8>, String> {
        let pitch = (width as usize).checked_mul(4).ok_or("image row overflow")?;
        let len = pitch.checked_mul(height as usize).ok_or("image size overflow")?;
        if width == 0 || height == 0 || len > 16 * 1024 * 1024
            || vertices.len() != 3 || root_constants.len() > 64
        {
            return Err("invalid graphics request dimensions or lengths".into());
        }
        let mut constants = vec![0; self.prepared.dwords];
        for u in &self.prepared.user_data {
            constants[u.index as usize] = *root_constants.get(u.dword as usize)
                .ok_or("missing graphics root constant dword")?;
        }
        let mut pixels = vec![0; len];
        victory_metal::clear_color_float(
            victory_metal::ColorFormat::Rgba8, width, height, pitch, &mut pixels, clear, &[],
        ).map_err(|e| e.to_string())?;
        self.prepared.draw(
            width, height, Vertices::Position(vertices), None,
            [0.0, 0.0, f64::from(width), f64::from(height), 0.0, 1.0],
            [0, 0, width, height], None, &constants, None, &mut pixels,
        )?;
        Ok(pixels)
    }
}


fn word(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes = data
        .get(at..at.checked_add(4).ok_or("offset overflow")?)
        .ok_or("truncated metadata")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

// Rows are local to each stage; semantic names must match independently.
fn signature(
    container: &Container<'_>,
    part: FourCC,
    required: Option<(&[u8], u32)>,
    optional_position: bool,
    allow_varying: bool,
) -> Result<Option<u8>, String> {
    let Some(data) = container.part(part).map(|p| p.data) else {
        return if required.is_none() { Ok(None) } else { Err(format!("missing {part} signature")) };
    };
    let count = word(data, 0)? as usize;
    if count > 2 {
        return Err(format!("unsupported {part} signature element count"));
    }
    let base = if count == 0 { 0 } else { word(data, 4)? as usize };
    let mut found_required = false;
    let mut position = false;
    let mut varying = None;
    for i in 0..count {
        let at = base.checked_add(i * 32).ok_or("signature offset overflow")?;
        let name_at = word(data, at + 4)? as usize;
        let bytes = data.get(name_at..).ok_or("signature name out of bounds")?;
        let end = bytes.iter().position(|&b| b == 0).ok_or("unterminated signature name")?;
        let name = &bytes[..end];
        let row = word(data, at + 20)?;
        let mask = *data.get(at + 24).ok_or("truncated signature")?;
        if word(data, at)? != 0 || word(data, at + 8)? != 0
            || word(data, at + 16)? != 3 || word(data, at + 28)? != 0
        {
            return Err(format!("unsupported {part} signature metadata"));
        }
        let expected = required.filter(|(expected, _)| name.eq_ignore_ascii_case(expected));
        if expected.is_some() || (optional_position && name.eq_ignore_ascii_case(b"SV_Position")) {
            let system = expected.map_or(1, |(_, system)| system);
            if position || word(data, at + 12)? != system || row != 0 || mask != 15 {
                return Err(format!("unsupported {part} system signature"));
            }
            position = true;
            found_required = expected.is_some();
        } else if allow_varying && name.eq_ignore_ascii_case(b"TEXCOORD")
            && word(data, at + 12)? == 0 && mask == 3 && row <= u8::MAX as u32
            && data.get(at + 26..at + 28) == Some(&[0, 0])
            && varying.is_none()
        {
            varying = Some(row as u8);
        } else {
            return Err(format!("unsupported {part} signature element"));
        }
    }
    if required.is_some() && !found_required {
        return Err(format!("missing required {part} signature element"));
    }
    if position && varying == Some(0) {
        return Err(format!("overlapping {part} signature rows"));
    }
    Ok(varying)
}

fn varying_signatures(vertex: &[u8], pixel: &[u8], input_uv: bool) -> Result<Option<(u8, u8)>, String> {
    let vs = Container::parse(vertex).map_err(|e| e.to_string())?;
    let ps = Container::parse(pixel).map_err(|e| e.to_string())?;
    let vertex_uv = signature(&vs, FourCC::ISG1, Some((b"POSITION", 0)), false, input_uv)?;
    if vertex_uv != input_uv.then_some(1) {
        return Err("vertex input signature does not match input layout".into());
    }
    let output = signature(&vs, FourCC::OSG1, Some((b"SV_Position", 1)), false, true)?;
    let input = signature(&ps, FourCC::ISG1, None, true, true)?;
    signature(&ps, FourCC::OSG1, Some((b"SV_Target", 64)), false, false)?;
    match (output, input) {
        (None, None) => Ok(None),
        // This converter path is verified for VS output register 1 linked to
        // PS input register 0; reject unverified register layouts explicitly.
        (Some(1), Some(0)) => Ok(Some((1, 0))),
        _ => Err("vertex and pixel TEXCOORD0 signatures do not match".into()),
    }
}

fn validate(
    bytes: &[u8],
    stage: HwStage,
    varying: Option<u8>,
    input_uv: bool,
) -> Result<(emit::HostShader, Option<u8>, Option<u8>), String> {
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
    match (&p.io, stage, varying) {
        (native::StageIo::Vs { inputs, outputs }, HwStage::Vs, row)
            if inputs.len() == (if input_uv { 2 } else { 1 })
                && inputs[0].usage == native::Usage::Generic
                && inputs[0].row == 0
                && inputs[0].mask == 15
                && (!input_uv || (inputs[0].vgpr == 4
                    && inputs[1].usage == native::Usage::Generic && inputs[1].row == 1
                    && inputs[1].mask == 3 && inputs[1].vgpr == 8))
                && match row {
                    None => outputs.is_empty(),
                    Some(row) => outputs.len() == 1
                        && outputs[0].usage == native::Usage::Generic
                        && outputs[0].row == row
                        && !outputs[0].fp16 && !outputs[0].pack2,
                } => {}
        (native::StageIo::Ps { inputs, input_addr, .. }, HwStage::Ps, row)
            if match row {
                None => inputs.is_empty(),
                Some(row) => inputs.len() == 1
                    && inputs[0].usage == native::Usage::Generic
                    && inputs[0].row == row && inputs[0].mask == 3
                    && !inputs[0].flat && !inputs[0].fp16 && !inputs[0].pack2
                    && input_addr & 2 != 0 && input_addr & !0xf02 == 0,
            } => {}
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
        HwStage::Vs => !matches!(input, io::Input::Attribute { row: 0, .. })
            && !(input_uv && matches!(input, io::Input::Attribute { row: 1, channel: 0 | 1 })),
        HwStage::Ps => !matches!(input, io::Input::Position(_))
            && !(varying.is_some() && matches!(input, io::Input::Barycentric {
                kind: io::Barycentric::PerspectiveCenter, index: 0 | 1
            })),
        _ => true,
    }) {
        return Err("unsupported shader system inputs".into());
    }
    let mut channels = 0u8;
    for (site, interp) in &raised.interps {
        let io::Interp::Interpolated { attribute, barycentric, first } = interp else {
            return Err("flat interpolation is unsupported".into());
        };
        if stage != HwStage::Ps || Some(attribute.row) != varying
            || attribute.usage != native::Usage::Generic || attribute.channel > 1
            || attribute.high || *barycentric != io::Barycentric::PerspectiveCenter
            || first.is_empty() || program.blocks[site.0].insts[site.1].name() != "v_interp_p2_f32"
            || first.iter().any(|&(b, i)| program.blocks[b].insts[i].name() != "v_interp_p1_f32")
        {
            return Err("unsupported TEXCOORD0 interpolation".into());
        }
        channels |= 1 << attribute.channel;
    }
    if (stage == HwStage::Ps && varying.is_some() && channels != 3)
        || (varying.is_none() && !raised.interps.is_empty())
    {
        return Err("missing TEXCOORD0 interpolation channels".into());
    }
    for (b, block) in program.blocks.iter().enumerate() {
        for (i, inst) in block.insts.iter().enumerate() {
            if inst.name().starts_with("v_interp_")
                && !raised.interps.iter().any(|(site, interp)| {
                    *site == (b, i) || matches!(interp, io::Interp::Interpolated { first, .. } if first.contains(&(b, i)))
                })
            {
                return Err("unpaired or unsupported interpolation instruction".into());
            }
        }
    }
    let mut param_export = false;
    for export in &raised.exports {
        let valid = match (stage, export.target) {
            (HwStage::Vs, io::Target::Position(0) | io::Target::Null) => true,
            (HwStage::Vs, io::Target::Unread) => varying.is_none(),
            (HwStage::Vs, io::Target::Param { usage: native::Usage::Generic, row })
                if Some(row) == varying => {
                    param_export = true;
                    !export.compressed && export.values[0].is_some()
                        && export.values[1].is_some() && export.values[2].is_none()
                        && export.values[3].is_none()
                }
            (HwStage::Ps, io::Target::Color(0) | io::Target::Null) => true,
            _ => false,
        };
        if !valid { return Err("unsupported shader export".into()); }
    }
    if (stage == HwStage::Vs && varying.is_some() && !param_export)
        || !raised.exports.iter().any(|e| match stage {
            HwStage::Vs => matches!(e.target, io::Target::Position(0)),
            _ => matches!(e.target, io::Target::Color(0)),
        })
    {
        return Err("missing shader export".into());
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
    let dwords = host.user_data_dwords() as usize;
    if dwords > 64 {
        return Err("fragment user-data buffer is too large".into());
    }
    Ok((host, texture.map(|u| u.slot), sampler.map(|u| u.slot)))
}

fn convert(host: &emit::HostShader, stage: HwStage, sampled: bool) -> Result<Vec<u8>, String> {
    let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
    let dwords = host.user_data_dwords() as usize;
    if stage == HwStage::Vs {
        victory_metal::converter::convert(
            std::path::Path::new(&path),
            &host.dxil,
            victory_metal::converter::Stage::Vertex,
        )
    } else if sampled {
        victory_metal::converter::convert_sampled_fragment(
            std::path::Path::new(&path),
            &host.dxil,
            dwords != 0,
        )
    } else {
        victory_metal::converter::convert_fragment_with_b0(
            std::path::Path::new(&path),
            &host.dxil,
            dwords != 0,
        )
    }
}

#[derive(Default)]
pub(crate) struct StreamState {
    seen: u32,
    error: Option<String>,
    depth: Option<victory_metal::DepthState>,
    depth_format: u32,
    blend: Option<victory_metal::BlendState>,
    input_uv: bool,
    color_format: Option<victory_metal::ColorFormat>,
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
            // One RT; D3D ignores RT1..7 when independent blending is off.
            8 => {
                let (enabled, logic, mask) = (get(12), get(16), data[48]);
                let basic = get(4) == 0 && get(8) == 0 && enabled <= 1 && logic == 0 && mask <= 15;
                let factors = enabled == 0 || (
                    get(20) == 5 && get(24) == 6 && get(28) == 1
                    && matches!(get(32), 2 | 5) && get(36) == 6 && get(40) == 1
                );
                if basic && factors {
                    self.blend = Some(victory_metal::BlendState {
                        enabled: enabled == 1, write_mask: mask,
                        alpha_source: enabled == 1 && get(32) == 5,
                    });
                    true
                } else {
                    false
                }
            }
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
            12 => match unsafe { input_layout(data) } {
                Some(uv) => { self.input_uv = uv; true }
                None => false,
            },
            13 => get(4) == 0,
            14 => get(4) == 3,
            15 => {
                self.color_format = match get(4) {
                    28 => Some(victory_metal::ColorFormat::Rgba8),
                    29 => Some(victory_metal::ColorFormat::Rgba8Srgb),
                    87 => Some(victory_metal::ColorFormat::Bgra8),
                    91 => Some(victory_metal::ColorFormat::Bgra8Srgb),
                    _ => None,
                };
                self.color_format.is_some() && (1..8).all(|i| get(4 + i * 4) == 0) && get(36) == 1
            }
            16 => { self.depth_format = get(4); matches!(self.depth_format, 0 | 40) },
            17 => get(4) == 1 && get(8) == 0,
            9 => get(4) == u32::MAX,
            _ => false,
        };
        if !valid {
            self.reject(&format!("unsupported graphics stream subobject {kind}"));
        }
    }

    pub(crate) fn finish(self) -> Result<GraphicsState, String> {
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
        Ok(GraphicsState {
            depth: self.depth,
            input_uv: self.input_uv,
            color_format: self.color_format.ok_or("missing color format")?,
            blend: self.blend.unwrap_or(victory_metal::BlendState {
                enabled: false, write_mask: 15, alpha_source: false,
            }),
        })
    }
}

unsafe fn input_layout(data: &[u8]) -> Option<bool> {
    let ptr = u64::from_le_bytes(data[8..16].try_into().ok()?) as *const u8;
    let count = word(data, 16).ok()?;
    if ptr.is_null() || !matches!(count, 1 | 2) {
        return None;
    }
    // SAFETY: the guest supplies `count` D3D12_INPUT_ELEMENT_DESC entries
    // and NUL-terminated semantic names for pipeline creation.
    let valid = |at: *const u8, semantic: &[u8], expected: [u32; 6]| {
        let name = unsafe { at.cast::<*const i8>().read_unaligned() };
        if name.is_null() || !unsafe { CStr::from_ptr(name) }.to_bytes().eq_ignore_ascii_case(semantic) {
            return false;
        }
        let element = unsafe { std::slice::from_raw_parts(at, 32) };
        expected.iter().enumerate().all(|(i, v)| word(element, 8 + i * 4).ok() == Some(*v))
    };
    if !valid(ptr, b"POSITION", [0, 2, 0, 0, 0, 0]) {
        return None;
    }
    if count == 2 && !valid(unsafe { ptr.add(32) }, b"TEXCOORD", [0, 16, 0, 16, 0, 0]) {
        return None;
    }
    Some(count == 2)
}
