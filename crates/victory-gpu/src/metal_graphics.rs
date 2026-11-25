//! Restricted vertex/pixel pipelines for the offscreen Metal renderer.
use std::ffi::CStr;
use victory_shader::{Container, FourCC, HwStage, emit, io, ir, native, resource};
#[derive(Clone, Copy)]
pub(crate) struct GraphicsState {
    pub(crate) depth: Option<victory_metal::DepthState>,
    pub(crate) blend: victory_metal::BlendState,
    pub(crate) input_uv: bool,
    pub(crate) color_format: victory_metal::ColorFormat,
}

/// A statically accessed table entry and its independent direct-heap slot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TableEntry {
    pub(crate) slot: u8,
    pub(crate) offset: u32,
    pub(crate) heap_index: u32,
}

fn table_entries(slots: &[u8], accesses: &[(u8, u32)]) -> Result<Vec<TableEntry>, String> {
    let mut entries = Vec::new();
    let mut base = 0u32;
    for &slot in slots {
        let max = accesses.iter().filter(|(s, _)| *s == slot).map(|(_, offset)| *offset)
            .max().ok_or("missing shader resource access")?;
        let end = base.checked_add(max).and_then(|n| n.checked_add(1))
            .filter(|&n| n <= 16).ok_or("descriptor heap span exceeds 16 entries")?;
        for &(s, offset) in accesses {
            if s == slot {
                entries.push(TableEntry { slot, offset, heap_index: base + offset });
            }
        }
        base = end;
    }
    Ok(entries)
}

fn static_descriptor_index(address: &resource::Address, size: u32) -> Result<u32, String> {
    if address.dynamic.is_some() || address.offset % size != 0 {
        return Err("descriptor address must be static and descriptor-aligned".into());
    }
    Ok(address.offset / size)
}

#[cfg(test)]
mod descriptor_address_tests {
    use super::*;

    #[test]
    fn rejects_dynamic_and_unaligned_addresses() {
        let address = resource::Address { param: 0, offset: 32, dynamic: None };
        assert_eq!(static_descriptor_index(&address, 32).unwrap(), 1);
        assert!(static_descriptor_index(&resource::Address { offset: 31, ..address }, 32).is_err());
        assert!(static_descriptor_index(&resource::Address { dynamic: Some(ir::Value(0)), ..address }, 32).is_err());
    }

    #[test]
    fn table_spans_include_gaps_and_do_not_overlap() {
        let entries = table_entries(&[1, 3], &[(1, 2), (1, 0), (3, 1)]).unwrap();
        assert_eq!(entries.iter().map(|e| (e.slot, e.offset, e.heap_index)).collect::<Vec<_>>(),
            vec![(1, 2, 2), (1, 0, 0), (3, 1, 4)]);
        assert!(table_entries(&[1, 3], &[(1, 15), (3, 0)]).is_err());
        assert!(table_entries(&[1], &[(1, u32::MAX)]).is_err());
    }
}


pub(crate) struct Prepared {
    pub(crate) vertex: Vec<u8>,
    vertex_dxil: Vec<u8>,
    point_vertex: Option<Vec<u8>>,
    vertex_reflection: Option<Vec<victory_metal::converter::VertexAttribute>>,
    pub(crate) pixel: Vec<u8>,
    pub(crate) user_data: Vec<emit::UserData>,
    pub(crate) dwords: usize,
    pub(crate) texture_slot: Option<u8>,
    pub(crate) sampler_slot: Option<u8>,
    pub(crate) texture_slots: Vec<u8>,
    pub(crate) sampler_slots: Vec<u8>,
    pub(crate) texture_entries: Vec<TableEntry>,
    texture_dimensions: Vec<TextureDimension>,
    pub(crate) sampler_entries: Vec<TableEntry>,
    // Shader-wide flags conservatively pair every bound sampler and texture.
    explicit_lod: bool,
    explicit_lod_zero: bool,
    pub(crate) depth: Option<victory_metal::DepthState>,
    pub(crate) vertex_stride: u32,
    pub(crate) color_format: victory_metal::ColorFormat,
    pub(crate) blend: victory_metal::BlendState,
    pub(crate) cull: u32,
    pub(crate) front_ccw: bool,
    color_targets: Vec<victory_metal::RenderTarget>,
    pub(crate) legacy_attributes: Vec<victory_metal::VertexAttribute>,
    renderer: Option<(u32, u32, victory_metal::Renderer)>,
    input_renderer: Option<(u32, u32, bool, Vec<victory_metal::VertexAttribute>, victory_metal::Renderer)>,
    renderer_texture: [Option<SampledTextureKey>; 16],
    input_renderer_texture: [Option<SampledTextureKey>; 16],
}

/// A supplied input-layout element and the Metal attribute assigned to it.
pub(crate) struct ResolvedVertexInput {
    pub(crate) layout_index: usize,
    pub(crate) attribute_index: u8,
}

#[derive(Debug)]
struct VertexSignature {
    row: u8,
    mask: u8,
    layout_index: usize,
    reflected_name: String,
}

#[derive(Debug)]
struct Varying {
    vertex_row: u8,
    pixel_row: u8,
    mask: u8,
}

// The renderer owns Metal objects and is only accessed through the pipeline's
// mutex; its synchronous draws complete before another thread can use it.
unsafe impl Send for Prepared {}
pub(crate) enum Vertices<'a> {
    Position(&'a [[f32; 4]]),
    PositionUv(&'a [[f32; 6]]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextureDimension { D2, D2Array, Cube }

pub(crate) struct Texture<'a> {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub array_layers: Option<u32>,
    pub cube: bool,
    pub pixels: &'a [u8],
    pub filter: victory_metal::TextureFilter,
    pub format: victory_metal::TextureFormat,
    pub min_lod: f32,
    pub max_lod: f32,
}

impl Texture<'_> {
    fn matches_dimension(&self, dimension: TextureDimension) -> bool {
        match dimension {
            TextureDimension::D2 => !self.cube && self.array_layers.is_none(),
            TextureDimension::D2Array => !self.cube && self.array_layers.is_some(),
            TextureDimension::Cube => self.cube && self.array_layers == Some(6),
        }
    }
}

/// Identity is valid only for the lifetime of a Wine batch. Callers without
/// an explicit identity must upload on every draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampledTextureIdentity {
    pub resource: u64,
    pub generation: usize,
    pub first_mip: u32,
    pub first_slice: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampledTextureKey {
    identity: SampledTextureIdentity,
    width: u32,
    height: u32,
    levels: u32,
    array_layers: Option<u32>,
    cube: bool,
    format: victory_metal::TextureFormat,
    linear: bool,
    min_lod: u32,
    max_lod: u32,
}

impl SampledTextureKey {
    fn new(identity: SampledTextureIdentity, texture: &Texture<'_>) -> Self {
        Self {
            identity, width: texture.width, height: texture.height, levels: texture.levels,
            array_layers: texture.array_layers, cube: texture.cube, format: texture.format, linear: matches!(texture.filter, victory_metal::TextureFilter::Linear),
            min_lod: texture.min_lod.to_bits(), max_lod: texture.max_lod.to_bits(),
        }
    }
}

fn sampled_texture_cached(cached: Option<SampledTextureKey>, key: Option<SampledTextureKey>) -> bool {
    key.is_some() && cached == key
}

fn upload_sampled_texture(
    renderer: &mut victory_metal::Renderer, index: u32, texture: &Texture<'_>,
    identity: Option<SampledTextureIdentity>, cached: &mut Option<SampledTextureKey>,
) -> Result<(), String> {
    let key = identity.map(|identity| SampledTextureKey::new(identity, &texture));
    if sampled_texture_cached(*cached, key) {
        return Ok(());
    }
    // A failed upload must never leave a stale identity attached to a renderer.
    *cached = None;
    if texture.cube {
        if texture.array_layers != Some(6) || texture.width != texture.height {
            return Err("invalid cube texture view".into());
        }
        renderer.set_fragment_texture_cube_mips_at(
            index, texture.width, texture.height, texture.levels, texture.pixels, texture.format,
        ).map_err(|e| e.to_string())?;
    } else if let Some(layers) = texture.array_layers {
        renderer.set_fragment_texture_array_mips_at(
            index, texture.width, texture.height, texture.levels, layers, texture.pixels, texture.format,
        ).map_err(|e| e.to_string())?;
    } else {
        renderer.set_fragment_texture_mips_at(
            index, texture.width, texture.height, texture.levels, texture.pixels, texture.format,
        ).map_err(|e| e.to_string())?;
    }
    *cached = key;
    Ok(())
}

#[cfg(test)]
mod sampled_texture_cache_tests {
    use super::*;
    use victory_metal::{TextureFilter, TextureFormat};

    #[test]
    fn only_matching_explicit_identity_and_view_can_reuse_upload() {
        let identity = SampledTextureIdentity { resource: 1, generation: 0, first_mip: 0, first_slice: 0 };
        let mut texture = Texture {
            width: 8, height: 8, levels: 2, array_layers: None, cube: false, pixels: &[], filter: TextureFilter::Nearest,
            format: TextureFormat::Rgba8, min_lod: 0.0, max_lod: 0.0,
        };
        let original = SampledTextureKey::new(identity, &texture);
        let cached = Some(original);
        assert!(sampled_texture_cached(cached, Some(original)));
        assert!(!sampled_texture_cached(cached, None));
        assert!(!sampled_texture_cached(None, None));
        assert!(!sampled_texture_cached(None, Some(original))); // recreated renderer
        for changed in [
            SampledTextureIdentity { resource: 2, ..identity },
            SampledTextureIdentity { generation: 1, ..identity },
            SampledTextureIdentity { first_mip: 1, ..identity },
            SampledTextureIdentity { first_slice: 1, ..identity },
        ] {
            assert!(!sampled_texture_cached(cached, Some(SampledTextureKey::new(changed, &texture))));
        }
        texture.width = 4;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.width = 8;
        texture.height = 4;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.height = 8;
        texture.levels = 1;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.levels = 2;
        texture.array_layers = Some(1);
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.array_layers = None;
        texture.cube = true;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.cube = false;
        texture.filter = TextureFilter::Linear;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.filter = TextureFilter::Nearest;
        texture.format = TextureFormat::Bgra8;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.format = TextureFormat::Rgba8;
        texture.min_lod = 1.0;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
        texture.min_lod = 0.0;
        texture.max_lod = 1.0;
        assert_ne!(original, SampledTextureKey::new(identity, &texture));
    }
}


fn legacy_vertex_attributes(reflected: &[victory_metal::converter::VertexAttribute], input_uv: bool) -> Result<Vec<victory_metal::VertexAttribute>, String> {
    let names: &[(&str, u32, u32)] = if input_uv { &[("POSITION0", 2, 0), ("TEXCOORD0", 16, 16)] } else { &[("POSITION0", 2, 0)] };
    if reflected.len() != names.len() { return Err("converter vertex reflection does not match input signature".into()); }
    let mut attributes = Vec::with_capacity(names.len());
    for &(name, format, offset) in names {
        let a = reflected.iter().find(|a| a.name.eq_ignore_ascii_case(name)).ok_or("missing reflected vertex attribute")?;
        let index = a.index.checked_add(11).filter(|&i| i < 31).ok_or("reflected vertex attribute exceeds Metal limits")?;
        attributes.push(victory_metal::VertexAttribute { index: u32::from(index), format, slot: 0, offset, stride: if input_uv { 24 } else { 16 }, per_instance: false, step_rate: 0 });
    }
    Ok(attributes)
}

impl Prepared {
    pub(crate) fn new(vertex: &[u8], pixel: &[u8], graphics: GraphicsState) -> Result<Self, String> {
        Self::new_mrt(vertex, pixel, graphics, &[])
    }

    pub(crate) fn new_mrt(vertex: &[u8], pixel: &[u8], graphics: GraphicsState,
        extra: &[(victory_metal::ColorFormat, victory_metal::BlendState)]) -> Result<Self, String> {
        let count = extra.len() + 1;
        validate_color_signature(pixel, count)?;
        let varying = varying_signatures(vertex, pixel, graphics.input_uv)?;
        let (vertex_host, _, _, _, _, _, _) = validate(vertex, HwStage::Vs, &varying, graphics.input_uv, None, count)?;
        let (pixel_host, texture_slots, sampler_slots, texture_entries, sampler_entries, texture_dimensions, explicit_lod) =
            validate(pixel, HwStage::Ps, &varying, false, None, count)?;
        let (vertex, reflected, legacy_attributes) = if extra.is_empty() {
            (convert(&vertex_host, HwStage::Vs, false)?, None, Vec::new())
        } else {
            let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
            let (vertex, reflected) = victory_metal::converter::convert_vertex_with_reflection(std::path::Path::new(&path), &vertex_host.dxil)?;
            let attributes = legacy_vertex_attributes(&reflected, graphics.input_uv)?;
            (vertex, Some(reflected), attributes)
        };
        let pixel = convert(&pixel_host, HwStage::Ps, !texture_slots.is_empty())?;
        let dwords = pixel_host.user_data_dwords() as usize;
        let user_data = pixel_host.user_data;
        Ok(Self {
            vertex,
            vertex_dxil: vertex_host.dxil,
            point_vertex: None,
            vertex_reflection: reflected,
            pixel,
            user_data,
            dwords,
            texture_slot: texture_slots.first().copied(),
            sampler_slot: sampler_slots.first().copied(),
            texture_slots, sampler_slots,
            texture_entries, sampler_entries, texture_dimensions,
            explicit_lod: explicit_lod.0, explicit_lod_zero: explicit_lod.1,
            depth: graphics.depth,
            blend: graphics.blend,
            cull: 1,
            front_ccw: false,
            vertex_stride: if graphics.input_uv { 24 } else { 16 },
            color_format: graphics.color_format,
            color_targets: std::iter::once((graphics.color_format, graphics.blend)).chain(extra.iter().copied()).map(|(format, blend)| victory_metal::RenderTarget { format, blend }).collect(),
            legacy_attributes,
            renderer: None,
            input_renderer: None,
            renderer_texture: [None; 16],
            input_renderer_texture: [None; 16],
        })
    }

    /// Prepare against D3D input elements; indices come only from converter reflection.
    /// The returned mapping identifies each consumed element in `inputs`.
    pub(crate) fn new_inputs(
        vertex: &[u8], pixel: &[u8], graphics: GraphicsState,
        inputs: &[crate::wine_transport::VertexInput],
    ) -> Result<(Self, Vec<ResolvedVertexInput>), String> {
        Self::new_inputs_mrt(vertex, pixel, graphics, inputs, &[])
    }

    pub(crate) fn new_inputs_mrt(
        vertex: &[u8], pixel: &[u8], graphics: GraphicsState,
        inputs: &[crate::wine_transport::VertexInput],
        extra: &[(victory_metal::ColorFormat, victory_metal::BlendState)],
    ) -> Result<(Self, Vec<ResolvedVertexInput>), String> {
        let count = extra.len() + 1;
        validate_color_signature(pixel, count)?;
        let signatures = vertex_signatures(vertex, inputs)?;
        let varying = varying_signatures_for_stages(vertex, pixel)?;
        let (vertex_host, _, _, _, _, _, _) = validate(vertex, HwStage::Vs, &varying, false, Some(&signatures), count)?;
        let (pixel_host, texture_slots, sampler_slots, texture_entries, sampler_entries, texture_dimensions, explicit_lod) =
            validate(pixel, HwStage::Ps, &varying, false, None, count)?;
        let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
        let (vertex, reflected) = victory_metal::converter::convert_vertex_with_reflection(
            std::path::Path::new(&path), &vertex_host.dxil,
        )?;
        if reflected.len() != signatures.len() {
            return Err("converter vertex reflection does not match the input signature".into());
        }
        let mut resolved = Vec::with_capacity(signatures.len());
        for sig in &signatures {
            let attribute = reflected.iter().find(|a| a.name.eq_ignore_ascii_case(&sig.reflected_name))
                .ok_or("converter vertex reflection is missing an input semantic")?;
            // Converter reflection numbers inputs from zero; Metal vertex
            // fetch reserves attributes 0..10 (kIRStageInAttributeStartIndex).
            let attribute_index = attribute.index.checked_add(11)
                .filter(|&index| index < 31)
                .ok_or("reflected vertex attribute exceeds Metal vertex-fetch limits")?;
            resolved.push(ResolvedVertexInput { layout_index: sig.layout_index, attribute_index });
        }
        let pixel = convert(&pixel_host, HwStage::Ps, !texture_slots.is_empty())?;
        let dwords = pixel_host.user_data_dwords() as usize;
        Ok((Self {
            vertex, vertex_dxil: vertex_host.dxil, point_vertex: None, vertex_reflection: Some(reflected),
            pixel, user_data: pixel_host.user_data, dwords,
            texture_slot: texture_slots.first().copied(), sampler_slot: sampler_slots.first().copied(),
            texture_slots, sampler_slots,
            texture_entries, sampler_entries, texture_dimensions,
            explicit_lod: explicit_lod.0, explicit_lod_zero: explicit_lod.1,
            depth: graphics.depth, blend: graphics.blend, vertex_stride: 0,
            cull: 1, front_ccw: false,
            color_format: graphics.color_format, renderer: None, input_renderer: None,
            color_targets: std::iter::once((graphics.color_format, graphics.blend)).chain(extra.iter().copied()).map(|(format, blend)| victory_metal::RenderTarget { format, blend }).collect(),
            legacy_attributes: Vec::new(),
            renderer_texture: [None; 16], input_renderer_texture: [None; 16],
        }, resolved))
    }

    fn validate_explicit_lod(&self, textures: &[(Texture<'_>, Option<SampledTextureIdentity>)],
        samplers: &[(victory_metal::TextureFilter, bool, f32, f32, [victory_metal::SamplerAddress; 3])]) -> Result<(), String> {
        let last_mip = textures.iter().map(|(texture, _)| texture.levels.saturating_sub(1)).max().unwrap_or(0);
        if last_mip > 0 {
            if self.explicit_lod && samplers.iter().any(|s| s.2 > 0. || s.3 < last_mip as f32) {
                return Err("explicit LOD sampling with sampler LOD clamp on a multi-mip texture is unsupported".into());
            }
            if self.explicit_lod_zero && samplers.iter().any(|s| s.2 > 0.) {
                return Err("explicit level-zero sampling with positive sampler minimum LOD on a multi-mip texture is unsupported".into());
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_legacy_attributes(&mut self) -> Result<(), String> {
        if self.vertex_reflection.is_none() {
            let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
            let (_, reflected) = victory_metal::converter::convert_vertex_with_reflection(
                std::path::Path::new(&path), &self.vertex_dxil,
            )?;
            self.legacy_attributes = legacy_vertex_attributes(&reflected, self.vertex_stride == 24)?;
            self.vertex_reflection = Some(reflected);
        }
        Ok(())
    }

    pub(crate) fn draw(
        &mut self, width: u32, height: u32, vertices: Vertices<'_>,
        indices: Option<(victory_metal::IndexData<'_>, i32)>, viewport: [f64; 6],
        scissor: [u32; 4], depth: Option<&mut [f32]>, stencil: Option<&mut [u8]>,
        stencil_reference: u32, constants: &[u32], texture: Option<Texture<'_>>,
        pixels: &mut [u8],
    ) -> Result<(), String> {
        if self.texture_entries.iter().any(|e| e.offset != 0)
            || self.sampler_entries.iter().any(|e| e.offset != 0)
        {
            return Err("native graphics supports only descriptor index zero".into());
        }
        let identity = None;
        let sampler = texture.as_ref().map(|t| (t.filter, matches!(t.filter, victory_metal::TextureFilter::Linear), t.min_lod, t.max_lod, [victory_metal::SamplerAddress::Clamp; 3]));
        let textures = texture.map(|t| (t, identity));
        self.draw_multi(width, height, vertices, indices, viewport, scissor, depth, stencil,
            stencil_reference, constants, textures.as_slice(), sampler.as_slice(), pixels)
    }

    pub(crate) fn draw_multi(
        &mut self,
        width: u32,
        height: u32,
        vertices: Vertices<'_>,
        indices: Option<(victory_metal::IndexData<'_>, i32)>,
        viewport: [f64; 6],
        scissor: [u32; 4],
        mut depth: Option<&mut [f32]>,
        mut stencil: Option<&mut [u8]>,
        stencil_reference: u32,
        constants: &[u32],
        textures: &[(Texture<'_>, Option<SampledTextureIdentity>)],
        samplers: &[(victory_metal::TextureFilter, bool, f32, f32, [victory_metal::SamplerAddress; 3])],
        pixels: &mut [u8],
    ) -> Result<(), String> {
        if textures.len() != self.texture_entries.len() || samplers.len() != self.sampler_entries.len() {
            return Err("texture or sampler binding count does not match pixel shader".into());
        }
        if textures.iter().zip(&self.texture_dimensions).any(|((texture, _), &dimension)| !texture.matches_dimension(dimension)) {
            return Err("shader texture dimension does not match binding".into());
        }
        self.validate_explicit_lod(textures, samplers)?;
        if self.vertex_stride == 0 {
            return Err("generic input pipeline requires layout-aware rendering".into());
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
            self.renderer_texture = [None; 16];
            self.renderer = Some((width, height, renderer));
        }
        let renderer = &mut self.renderer.as_mut().unwrap().2;
        renderer.set_rasterizer(self.cull, self.front_ccw).map_err(|e| e.to_string())?;
        renderer.set_viewport_scissor(viewport, scissor).map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref() {
            renderer.set_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some(pixels) = stencil.as_deref_mut() {
            renderer.stencil_pixels(pixels, true).map_err(|e| e.to_string())?;
            renderer.set_stencil_reference(stencil_reference);
        }
        for ((texture, identity), entry) in textures.iter().zip(&self.texture_entries) {
            let index = entry.heap_index as usize;
            upload_sampled_texture(renderer, entry.heap_index, texture, *identity, &mut self.renderer_texture[index])?;
        }
        for (&(filter, mip_linear, min_lod, max_lod, address), entry) in samplers.iter().zip(&self.sampler_entries) {
            renderer.set_sampler_lod(entry.heap_index, filter, mip_linear, min_lod, max_lod, address).map_err(|e| e.to_string())?;
        }
        match (vertices, indices) {
            (Vertices::Position(v), Some((indices, base))) => renderer.draw_indexed_preserving(v, indices, base, constants, pixels),
            (Vertices::Position(v), None) => renderer.draw_preserving(v, constants, pixels),
            (Vertices::PositionUv(v), indices) => renderer.draw_uv_preserving(v, indices, constants, pixels),
        }.map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref_mut() {
            renderer.read_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some(pixels) = stencil.as_deref_mut() {
            renderer.stencil_pixels(pixels, false).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Draw using the caller's resolved Metal attributes and input-slot buffers.
    pub(crate) fn draw_inputs(
        &mut self,
        width: u32,
        height: u32,
        attributes: &[victory_metal::VertexAttribute],
        buffers: &[victory_metal::VertexBuffer<'_>],
        indices: Option<(victory_metal::IndexData<'_>, i32)>,
        first_vertex: u32,
        vertex_count: u32,
        instance_count: u32,
        start_instance: u32,
        viewport: [f64; 6],
        scissor: [u32; 4],
        mut depth: Option<&mut [f32]>,
        mut stencil: Option<&mut [u8]>,
        stencil_reference: u32,
        constants: &[u32],
        textures: &[(Texture<'_>, Option<SampledTextureIdentity>)],
        samplers: &[(victory_metal::TextureFilter, bool, f32, f32, [victory_metal::SamplerAddress; 3])],
        pixels: &mut [u8],
        preserve: bool,
    ) -> Result<(), String> {
        self.draw_inputs_mrt(width, height, attributes, buffers, indices, 4, first_vertex, vertex_count,
            instance_count, start_instance, viewport, scissor, depth, stencil, stencil_reference,
            constants, textures, samplers, &mut [pixels], preserve)
    }

    pub(crate) fn draw_inputs_mrt(
        &mut self, width: u32, height: u32, attributes: &[victory_metal::VertexAttribute],
        buffers: &[victory_metal::VertexBuffer<'_>], indices: Option<(victory_metal::IndexData<'_>, i32)>,
        topology: u32, first_vertex: u32, vertex_count: u32, instance_count: u32, start_instance: u32,
        viewport: [f64; 6], scissor: [u32; 4], mut depth: Option<&mut [f32]>,
        mut stencil: Option<&mut [u8]>, stencil_reference: u32, constants: &[u32],
        textures: &[(Texture<'_>, Option<SampledTextureIdentity>)],
        samplers: &[(victory_metal::TextureFilter, bool, f32, f32, [victory_metal::SamplerAddress; 3])], pixels: &mut [&mut [u8]], preserve: bool,
    ) -> Result<(), String> {
        if textures.len() != self.texture_entries.len() || samplers.len() != self.sampler_entries.len() {
            return Err("texture or sampler binding count does not match pixel shader".into());
        }
        if textures.iter().zip(&self.texture_dimensions).any(|((texture, _), &dimension)| !texture.matches_dimension(dimension)) {
            return Err("shader texture dimension does not match binding".into());
        }
        self.validate_explicit_lod(textures, samplers)?;
        if pixels.len() != self.color_targets.len() {
            return Err("color attachment count does not match pipeline".into());
        }
        
        if depth.is_some() != self.depth.is_some() {
            return Err("depth attachment does not match pipeline".into());
        }
        let matches_layout = |cached: &[victory_metal::VertexAttribute]| {
            cached.len() == attributes.len() && cached.iter().zip(attributes).all(|(a, b)| {
                a.index == b.index && a.format == b.format && a.slot == b.slot
                    && a.offset == b.offset && a.stride == b.stride
                    && a.per_instance == b.per_instance && a.step_rate == b.step_rate
            })
        };
        let point = topology == 1;
        if point && self.point_vertex.is_none() {
            let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
            let (vertex, reflected) = victory_metal::converter::convert_point_vertex_with_reflection(
                std::path::Path::new(&path), &self.vertex_dxil,
            )?;
            let normal = self.vertex_reflection.as_ref().ok_or("missing vertex reflection")?;
            if reflected.len() != normal.len() || reflected.iter().any(|a|
                !normal.iter().any(|b| a.index == b.index && a.name.eq_ignore_ascii_case(&b.name)))
            {
                return Err("point vertex reflection does not match input layout".into());
            }
            self.point_vertex = Some(vertex);
        }
        if !matches!(&self.input_renderer, Some((w, h, cached_point, layout, _))
            if *w == width && *h == height && *cached_point == point && matches_layout(layout))
        {
            let vertex = if point { self.point_vertex.as_ref().unwrap() } else { &self.vertex };
            let renderer = victory_metal::Renderer::new_configured_mrt(
                vertex, "main", &self.pixel, "main", width, height,
                self.dwords as u32, self.texture_slot.is_some(), self.depth, &self.color_targets, attributes,
            ).map_err(|e| e.to_string())?;
            self.input_renderer_texture = [None; 16];
            self.input_renderer = Some((width, height, point, attributes.to_vec(), renderer));
        }
        let renderer = &mut self.input_renderer.as_mut().unwrap().4;
        renderer.set_rasterizer(self.cull, self.front_ccw).map_err(|e| e.to_string())?;
        renderer.set_viewport_scissor(viewport, scissor).map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref() {
            renderer.set_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some(pixels) = stencil.as_deref_mut() {
            renderer.stencil_pixels(pixels, true).map_err(|e| e.to_string())?;
            renderer.set_stencil_reference(stencil_reference);
        }
        for ((texture, identity), entry) in textures.iter().zip(&self.texture_entries) {
            let index = entry.heap_index as usize;
            upload_sampled_texture(renderer, entry.heap_index, texture, *identity, &mut self.input_renderer_texture[index])?;
        }
        for (&(filter, mip_linear, min_lod, max_lod, address), entry) in samplers.iter().zip(&self.sampler_entries) {
            renderer.set_sampler_lod(entry.heap_index, filter, mip_linear, min_lod, max_lod, address).map_err(|e| e.to_string())?;
        }
        renderer.draw_buffers_mrt_topology(
            buffers, indices, first_vertex, vertex_count, instance_count, start_instance, topology,
            constants, pixels, preserve,
        ).map_err(|e| e.to_string())?;
        if let Some(pixels) = depth.as_deref_mut() {
            renderer.read_depth_pixels(pixels).map_err(|e| e.to_string())?;
        }
        if let Some(pixels) = stencil.as_deref_mut() {
            renderer.stencil_pixels(pixels, false).map_err(|e| e.to_string())?;
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
            blend: victory_metal::BlendState { enabled: false, write_mask: 15, factors: [2,1,1,2,1,1] },
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
            [0, 0, width, height], None, None, 0, &constants, None, &mut pixels,
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

// The converter names inputs as the semantic followed by its decimal index.
fn vertex_signatures(
    vertex: &[u8], inputs: &[crate::wine_transport::VertexInput],
) -> Result<Vec<VertexSignature>, String> {
    let container = Container::parse(vertex).map_err(|e| e.to_string())?;
    let data = container.part(FourCC::ISG1).ok_or("missing ISG1 signature")?.data;
    let count = word(data, 0)? as usize;
    if count > 31 || inputs.len() > 32 {
        return Err("too many vertex input elements".into());
    }
    for (i, input) in inputs.iter().enumerate() {
        if input.semantic.is_empty() || !input.semantic.is_ascii()
            || inputs[..i].iter().any(|other| other.semantic_index == input.semantic_index
                && other.semantic.eq_ignore_ascii_case(&input.semantic))
        {
            return Err("invalid or duplicate input layout semantic".into());
        }
    }
    let base = word(data, 4)? as usize;
    let mut result: Vec<VertexSignature> = Vec::with_capacity(count);
    for i in 0..count {
        let at = base.checked_add(i * 32).ok_or("signature offset overflow")?;
        let name_at = word(data, at + 4)? as usize;
        let tail = data.get(name_at..).ok_or("signature name out of bounds")?;
        let end = tail.iter().position(|&b| b == 0).ok_or("unterminated signature name")?;
        let name = std::str::from_utf8(&tail[..end]).map_err(|_| "invalid signature name")?;
        let index = word(data, at + 8)?;
        let row = word(data, at + 20)?;
        let mask = *data.get(at + 24).ok_or("truncated signature")?;
        if name.is_empty() || !name.is_ascii() || word(data, at)? != 0
            || word(data, at + 12)? != 0 || word(data, at + 16)? != 3
            || word(data, at + 28)? != 0 || row > u8::MAX as u32
            || !matches!(mask, 1 | 3 | 7 | 15)
            || data.get(at + 26..at + 28) != Some(&[0, 0])
        {
            return Err("unsupported vertex input signature metadata".into());
        }
        let layout_index = inputs.iter().position(|input| input.semantic_index == index
            && input.semantic.eq_ignore_ascii_case(name)).ok_or("vertex input semantic missing from layout")?;
        if !matches!((inputs[layout_index].format, mask), (2, 15) | (16 | 34 | 35, 3)) {
            return Err("vertex input format does not match signature components".into());
        }
        let reflected_name = format!("{name}{index}");
        if result.iter().any(|other| other.row == row as u8
            || other.layout_index == layout_index || other.reflected_name.eq_ignore_ascii_case(&reflected_name))
        {
            return Err("ambiguous vertex input signature".into());
        }
        result.push(VertexSignature { row: row as u8, mask, layout_index, reflected_name });
    }
    Ok(result)
}

fn varying_signatures(vertex: &[u8], pixel: &[u8], input_uv: bool) -> Result<Vec<Varying>, String> {
    let vs = Container::parse(vertex).map_err(|e| e.to_string())?;
    let vertex_uv = signature(&vs, FourCC::ISG1, Some((b"POSITION", 0)), false, input_uv)?;
    if vertex_uv != input_uv.then_some(1) {
        return Err("vertex input signature does not match input layout".into());
    }
    varying_signatures_for_stages(vertex, pixel)
}

struct StageVarying {
    name: String,
    index: u32,
    row: u8,
    mask: u8,
}

// Only unpacked float32 signatures are accepted. System position occupies
// its own row; every generic row has one unambiguous semantic and index.
fn stage_varyings(container: &Container<'_>, part: FourCC, require_position: bool) -> Result<Vec<StageVarying>, String> {
    let Some(data) = container.part(part).map(|p| p.data) else {
        return if require_position { Err(format!("missing {part} signature")) } else { Ok(Vec::new()) };
    };
    let count = word(data, 0)? as usize;
    if count > 17 { return Err(format!("too many {part} signature elements")); }
    let base = if count == 0 { 0 } else { word(data, 4)? as usize };
    let mut position = false;
    let mut entries: Vec<StageVarying> = Vec::with_capacity(count);
    for i in 0..count {
        let at = base.checked_add(i * 32).ok_or("signature offset overflow")?;
        let tail = data.get(word(data, at + 4)? as usize..).ok_or("signature name out of bounds")?;
        let end = tail.iter().position(|&b| b == 0).ok_or("unterminated signature name")?;
        let name = std::str::from_utf8(&tail[..end]).map_err(|_| "invalid signature name")?;
        let index = word(data, at + 8)?;
        let system = word(data, at + 12)?;
        let row = word(data, at + 20)?;
        let mask = *data.get(at + 24).ok_or("truncated signature")?;
        if name.is_empty() || !name.is_ascii() || word(data, at)? != 0
            || word(data, at + 16)? != 3 || word(data, at + 28)? != 0
            || data.get(at + 26..at + 28) != Some(&[0, 0])
        {
            return Err(format!("unsupported {part} signature metadata"));
        }
        if name.eq_ignore_ascii_case("SV_Position") {
            if position || index != 0 || system != 1 || row != 0 || mask != 15 {
                return Err(format!("unsupported {part} position signature"));
            }
            position = true;
        } else {
            if system != 0 || name.as_bytes().get(..3).is_some_and(|p| p.eq_ignore_ascii_case(b"SV_"))
                || row > u8::MAX as u32 || !matches!(mask, 1 | 3 | 7 | 15)
                || entries.iter().any(|e| e.row == row as u8
                    || (e.index == index && e.name.eq_ignore_ascii_case(name)))
            {
                return Err(format!("unsupported or ambiguous {part} varying"));
            }
            entries.push(StageVarying { name: name.to_owned(), index, row: row as u8, mask });
        }
    }
    if (require_position && !position) || (position && entries.iter().any(|e| e.row == 0)) {
        return Err(format!("missing or overlapping {part} position signature"));
    }
    Ok(entries)
}

fn varying_signatures_for_stages(vertex: &[u8], pixel: &[u8]) -> Result<Vec<Varying>, String> {
    let vs = Container::parse(vertex).map_err(|e| e.to_string())?;
    let ps = Container::parse(pixel).map_err(|e| e.to_string())?;
    let output = stage_varyings(&vs, FourCC::OSG1, true)?;
    let input = stage_varyings(&ps, FourCC::ISG1, false)?;
    if output.len() != input.len() { return Err("vertex and pixel varying signatures do not match".into()); }
    output.iter().map(|out| {
        let input = input.iter().find(|input| input.index == out.index
            && input.name.eq_ignore_ascii_case(&out.name) && input.mask == out.mask)
            .ok_or("vertex and pixel varying signatures do not match")?;
        Ok(Varying { vertex_row: out.row, pixel_row: input.row, mask: out.mask })
    }).collect()
}

fn validate_color_signature(pixel: &[u8], count: usize) -> Result<(), String> {
    if !(1..=8).contains(&count) { return Err("unsupported color target count".into()); }
    let container = Container::parse(pixel).map_err(|e| e.to_string())?;
    let data = container.part(FourCC::OSG1).ok_or("missing pixel output signature")?.data;
    if word(data, 0)? as usize != count { return Err("pixel output count does not match pipeline".into()); }
    let base = word(data, 4)? as usize;
    let mut seen = 0u16;
    for i in 0..count {
        let at = base.checked_add(i * 32).ok_or("signature offset overflow")?;
        let tail = data.get(word(data, at + 4)? as usize..).ok_or("signature name out of bounds")?;
        let end = tail.iter().position(|&b| b == 0).ok_or("unterminated signature name")?;
        let index = word(data, at + 8)? as usize;
        if !tail[..end].eq_ignore_ascii_case(b"SV_Target") || index >= count
            || seen & (1 << index) != 0 || word(data, at)? != 0
            || word(data, at + 12)? != 64 || word(data, at + 16)? != 3
            || word(data, at + 20)? as usize != index
            || data.get(at + 24) != Some(&15) || word(data, at + 28)? != 0
        { return Err("unsupported pixel output signature".into()); }
        seen |= 1 << index;
    }
    Ok(())
}

fn validate(
    bytes: &[u8],
    stage: HwStage,
    varying: &[Varying],
    input_uv: bool,
    vertex_signatures: Option<&[VertexSignature]>,
    color_count: usize,
) -> Result<(emit::HostShader, Vec<u8>, Vec<u8>, Vec<TableEntry>, Vec<TableEntry>, Vec<TextureDimension>, (bool, bool)), String> {
    let c = Container::parse(bytes).map_err(|e| e.to_string())?;
    let dxil = c.part(FourCC::DXIL).ok_or("missing DXIL part")?;
    let programs = victory_shader::native_programs(dxil.data).map_err(|e| e.to_string())?;
    if programs.len() != 1 || programs[0].stage != stage {
        return Err("expected exactly one native program of the requested stage".into());
    }
    let p = &programs[0];
    // Native metadata has no register-space field. Only its table-local
    // register zero is established; do not infer additional register spaces.
    let mut textures = Vec::new();
    let mut samplers = Vec::new();
    for used in &p.used {
        let target = match used.kind {
            native::ResourceType::Srv if used.dwords == 8 => &mut textures,
            native::ResourceType::Sampler if used.dwords == 4 => &mut samplers,
            _ => return Err("unsupported pixel resource".into()),
        };
        if stage != HwStage::Ps || used.root != native::RootType::Table
            || used.table_offset_dwords != 0 || used.lower_bound != 0
            || used.stride != 0 || used.slot >= 64
            || target.iter().any(|u: &&native::UsedResource| u.slot == used.slot)
        {
            return Err("unsupported or ambiguous descriptor table metadata".into());
        }
        target.push(used);
    }
    if textures.len() > 16 || samplers.len() > 16
        || textures.is_empty() != samplers.is_empty()
        || textures.iter().any(|t| samplers.iter().any(|s| t.slot == s.slot))
    {
        return Err("invalid texture and sampler tables".into());
    }
    textures.sort_unstable_by_key(|u| u.slot);
    samplers.sort_unstable_by_key(|u| u.slot);
    let mut constants_slot = None;
    for root in &p.layout {
        if stage == HwStage::Vs || root.slot >= 64 {
            return Err("unsupported root layout".into());
        }
        match root.root {
            native::RootType::Constants => {
                if constants_slot.is_some_and(|slot| slot != root.slot)
                    || textures.iter().any(|t| t.slot == root.slot)
                    || samplers.iter().any(|s| s.slot == root.slot)
                {
                    return Err("constants must use a distinct root slot".into());
                }
                constants_slot = Some(root.slot);
            }
            native::RootType::Table => {
                if root.sgprs != 1
                    || root.table_offset_dwords != 0
                    || root.base_register != 0
                    || !textures.iter().chain(&samplers)
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
    for used in textures.iter().chain(&samplers) {
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
            if (if let Some(signatures) = vertex_signatures {
                inputs.len() == signatures.len() && inputs.iter().enumerate().all(|(i, input)| {
                    input.usage == native::Usage::Generic
                        && signatures.iter().any(|s| s.row == input.row && s.mask == input.mask)
                        && input.vgpr >= 4
                        && u16::from(input.vgpr) + (u8::BITS - input.mask.leading_zeros()) as u16 <= 256
                        && inputs[..i].iter().all(|other| {
                            input.row != other.row && (0..4).all(|a| (0..4).all(|b| {
                                input.mask & (1 << a) == 0 || other.mask & (1 << b) == 0
                                    || u16::from(input.vgpr) + a != u16::from(other.vgpr) + b
                            }))
                        })
                })
            } else {
                inputs.len() == (if input_uv { 2 } else { 1 })
                    && inputs[0].usage == native::Usage::Generic
                    && inputs[0].row == 0
                    && inputs[0].mask == 15
                    && (!input_uv || (inputs[0].vgpr == 4
                        && inputs[1].usage == native::Usage::Generic && inputs[1].row == 1
                        && inputs[1].mask == 3 && inputs[1].vgpr == 8))
            })
                && outputs.len() == varying.len()
                && outputs.iter().enumerate().all(|(i, output)| {
                    output.usage == native::Usage::Generic && !output.fp16 && !output.pack2
                        && varying.iter().any(|v| v.vertex_row == output.row)
                        && outputs[..i].iter().all(|other| other.row != output.row && other.param != output.param)
                }) => {}
        (native::StageIo::Ps { inputs, input_addr, .. }, HwStage::Ps)
            if inputs.len() == varying.len()
                && input_addr & !0xf02 == 0
                && (inputs.iter().all(|i| i.flat) || input_addr & 2 != 0)
                && inputs.iter().enumerate().all(|(i, input)| {
                    input.usage == native::Usage::Generic && !input.fp16 && !input.pack2
                        && varying.iter().any(|v| v.pixel_row == input.row && v.mask == input.mask)
                        && inputs[..i].iter().all(|other| other.row != input.row && other.attr != input.attr)
                }) => {}
        _ => return Err(format!("unsupported native stage inputs or outputs: {:?}, vgprs {}, signatures {:?}", p.io, p.vgprs, vertex_signatures)),
    }
    let program = ir::build(&p.patched_code(), p.wave64, p.vgprs).map_err(|e| e.to_string())?;
    let traced = resource::track(&program, &p.layout, p.stage).map_err(|e| e.to_string())?;
    let mut loads = Vec::new();
    let mut seen = Vec::new();
    let mut texture_accesses = Vec::new();
    let mut texture_dimensions = Vec::new();
    let mut sampler_accesses = Vec::new();
    let mut explicit_lod = (false, false);
    for descriptor in &traced.descriptors {
        let resource::Source::Loaded { load, first: 0 } = descriptor.source else {
            return Err("descriptor must be loaded directly from its table".into());
        };
        let inst = &program.blocks[descriptor.site.0].insts[descriptor.site.1];
        let dimension = match &inst.fields {
            victory_shader::gfx10::Fields::Image { dim: 1, .. } => TextureDimension::D2,
            victory_shader::gfx10::Fields::Image { dim: 3, .. } => TextureDimension::Cube,
            victory_shader::gfx10::Fields::Image { dim: 5, .. } => TextureDimension::D2Array,
            _ => return Err("only 2D, 2D array and cube texture sampling is supported".into()),
        };
        if !matches!(inst.name(), "image_sample" | "image_sample_lz" | "image_sample_l" | "image_sample_b" | "image_sample_d") {
            return Err("only non-comparison texture sampling is supported".into());
        }
        explicit_lod.0 |= inst.name() == "image_sample_l";
        explicit_lod.1 |= inst.name() == "image_sample_lz";
        let expected_load = match descriptor.role {
            resource::Role::Resource => "s_load_dwordx8",
            resource::Role::Sampler => "s_load_dwordx4",
            _ => return Err("unsupported descriptor role".into()),
        };
        if program.blocks[load.0].insts[load.1].name() != expected_load {
            return Err("unsupported descriptor load width".into());
        }
        let (_, addr) = traced.scalar.iter().find(|(site, _)| *site == load)
            .ok_or("missing descriptor load")?;
        let root = p.layout.get(addr.param).ok_or("invalid descriptor table")?;
        let used = match descriptor.role {
            resource::Role::Resource => &textures,
            resource::Role::Sampler => &samplers,
            _ => unreachable!(),
        };
        if root.root != native::RootType::Table
            || !used.iter().any(|u| root.slot == u.slot && root.user_data == u.user_data)
        {
            return Err("descriptor load does not match its table metadata".into());
        }
        let (size, accesses) = match descriptor.role {
            resource::Role::Resource => (32, &mut texture_accesses),
            resource::Role::Sampler => (16, &mut sampler_accesses),
            _ => unreachable!(),
        };
        let index = static_descriptor_index(addr, size)?;
        if descriptor.role == resource::Role::Resource {
            if let Some((_, _, previous)) = texture_dimensions.iter().find(|&&(slot, offset, _)| slot == root.slot && offset == index) {
                if *previous != dimension { return Err("conflicting texture dimensions for descriptor".into()); }
            } else {
                texture_dimensions.push((root.slot, index, dimension));
            }
        }
        if !accesses.contains(&(root.slot, index)) { accesses.push((root.slot, index)); }
        if !loads.contains(&load) { loads.push(load); }
        if !seen.contains(&root.slot) { seen.push(root.slot); }
        if !traced.descriptors.iter().any(|other| other.site == descriptor.site && other.role != descriptor.role) {
            return Err("unpaired texture sample".into());
        }
    }
    if traced.scalar.iter().any(|(site, _)| !loads.contains(site))
        || textures.iter().chain(&samplers).any(|u| !seen.contains(&u.slot))
    {
        return Err("unexpected or missing shader resource access".into());
    }
    let raised = io::raise(&program, p).map_err(|e| e.to_string())?;
    if raised.inputs.iter().any(|(_, input)| match stage {
        HwStage::Vs => if let Some(signatures) = vertex_signatures {
            !matches!(input, io::Input::Attribute { row, channel }
                if signatures.iter().any(|s| s.row == *row && s.mask & (1 << channel) != 0))
        } else {
            !matches!(input, io::Input::Attribute { row: 0, .. })
                && !(input_uv && matches!(input, io::Input::Attribute { row: 1, channel: 0 | 1 }))
        },
        HwStage::Ps => !matches!(input, io::Input::Position(_))
            && !(!varying.is_empty() && matches!(input, io::Input::Barycentric {
                kind: io::Barycentric::PerspectiveCenter, index: 0 | 1
            })),
        _ => true,
    }) {
        return Err("unsupported shader system inputs".into());
    }
    let mut channels = vec![0u8; varying.len()];
    for (site, interp) in &raised.interps {
        let (attribute, flat) = match interp {
            io::Interp::Interpolated { attribute, barycentric, first }
                if *barycentric == io::Barycentric::PerspectiveCenter
                    && !first.is_empty()
                    && program.blocks[site.0].insts[site.1].name() == "v_interp_p2_f32"
                    && first.iter().all(|&(b, i)| program.blocks[b].insts[i].name() == "v_interp_p1_f32") => (attribute, false),
            io::Interp::Vertex { attribute, vertex: 2 } => (attribute, true),
            _ => return Err("unsupported varying interpolation".into()),
        };
        let index = varying.iter().position(|v| v.pixel_row == attribute.row)
            .ok_or("interpolation references an undeclared varying")?;
        let native_flat = match &p.io {
            native::StageIo::Ps { inputs, .. } => inputs.iter().find(|i| i.row == attribute.row).map(|i| i.flat),
            _ => None,
        };
        if stage != HwStage::Ps || attribute.usage != native::Usage::Generic || attribute.high
            || attribute.channel >= 4 || varying[index].mask & (1 << attribute.channel) == 0
            || native_flat != Some(flat)
        {
            return Err("varying interpolation does not match its signature and mode".into());
        }
        channels[index] |= 1 << attribute.channel;
    }
    if stage == HwStage::Ps && channels.iter().zip(varying).any(|(&mask, v)| mask != v.mask) {
        return Err("missing varying interpolation channels".into());
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
    let mut param_exports = vec![0u8; varying.len()];
    let mut color_exports = 0u16;
    for export in &raised.exports {
        let valid = match (stage, export.target) {
            (HwStage::Vs, io::Target::Position(0) | io::Target::Null) => true,
            (HwStage::Vs, io::Target::Unread) => varying.is_empty(),
            (HwStage::Vs, io::Target::Param { usage: native::Usage::Generic, row }) => {
                if let Some(index) = varying.iter().position(|v| v.vertex_row == row) {
                    let mask = export.values.iter().enumerate().fold(0u8, |mask, (c, v)| mask | (u8::from(v.is_some()) << c));
                    param_exports[index] |= mask;
                    !export.compressed && mask == varying[index].mask
                } else { false }
            }
            (HwStage::Ps, io::Target::Color(n)) if (n as usize) < color_count => {
                color_exports |= 1 << n;
                color_count == 1 || !export.compressed
            }
            (HwStage::Ps, io::Target::Null) => true,
            _ => false,
        };
        if !valid { return Err("unsupported shader export".into()); }
    }
    if (stage == HwStage::Vs && (param_exports.iter().zip(varying).any(|(&mask, v)| mask != v.mask)
        || !raised.exports.iter().any(|e| matches!(e.target, io::Target::Position(0)))))
        || (stage == HwStage::Ps && color_exports != (1u16 << color_count) - 1)
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
                            || !textures.iter().chain(&samplers).any(|r| r.slot == u.slot)
                    }
                    _ => true,
                }
        })
        || textures.iter().chain(&samplers).any(|r| {
            host.user_data.iter().filter(|u| u.root == native::RootType::Table && u.slot == r.slot && u.dword == 0).count() != 1
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
    let texture_slots: Vec<_> = textures.iter().map(|u| u.slot).collect();
    let sampler_slots: Vec<_> = samplers.iter().map(|u| u.slot).collect();
    let texture_entries = table_entries(&texture_slots, &texture_accesses)?;
    let sampler_entries = table_entries(&sampler_slots, &sampler_accesses)?;
    let dimensions = texture_entries.iter().map(|entry| {
        texture_dimensions.iter().find(|&&(slot, offset, _)| slot == entry.slot && offset == entry.offset)
            .map(|&(_, _, dimension)| dimension).ok_or("missing texture dimension")
    }).collect::<Result<Vec<_>, _>>()?;
    Ok((host, texture_slots, sampler_slots, texture_entries, sampler_entries, dimensions, explicit_lod))
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
                let factors = [get(20),get(24),get(28),get(32),get(36),get(40)];
                if let Some(blend) = basic.then(|| crate::wine_transport::Blend::new(enabled == 1, mask, factors)).flatten() {
                    self.blend = Some(victory_metal::BlendState {
                        enabled: blend.enabled, write_mask: blend.write_mask, factors: blend.factors,
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
                    self.depth = Some(victory_metal::DepthState { write: write == 1, compare, stencil: None });
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
                enabled: false, write_mask: 15, factors: [2,1,1,2,1,1],
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
