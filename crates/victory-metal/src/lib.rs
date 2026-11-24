//! Native offscreen Metal triangle rendering, not a console GPU backend.
//!
//! `Renderer` retains its device, pipeline, render target and readback buffer
//! across draws. It supports float4 position vertices, optionally followed by
//! float2 texture coordinates, fragment b0 constants, and one 2D texture/sampler.

#![cfg(target_os = "macos")]

pub mod compute;
pub mod converter;
pub mod present;

use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::ptr::NonNull;

#[derive(Debug)]
pub struct RenderError(String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}
/// Byte layout and transfer function of a color image; shader values are logical RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    Rgba8,
    Bgra8,
    Rgba8Srgb,
    Bgra8Srgb,
}

impl ColorFormat {
    fn is_bgra(self) -> bool {
        matches!(self, Self::Bgra8 | Self::Bgra8Srgb)
    }

    fn native_value(self) -> u32 {
        match self {
            Self::Rgba8 => 0,
            Self::Bgra8 => 1,
            Self::Rgba8Srgb => 4,
            Self::Bgra8Srgb => 5,
        }
    }
}
/// Sampled texture encoding; BC formats contain native compressed blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    Bgra8,
    Bc1,
    Bc3,
    Rgba8Srgb,
    Bgra8Srgb,
    Bc1Srgb,
    Bc3Srgb,
}

impl From<ColorFormat> for TextureFormat {
    fn from(format: ColorFormat) -> Self {
        match format {
            ColorFormat::Rgba8 => Self::Rgba8,
            ColorFormat::Bgra8 => Self::Bgra8,
            ColorFormat::Rgba8Srgb => Self::Rgba8Srgb,
            ColorFormat::Bgra8Srgb => Self::Bgra8Srgb,
        }
    }
}

impl TextureFormat {
    fn native_value(self) -> u32 {
        match self {
            Self::Rgba8 => 0,
            Self::Bgra8 => 1,
            Self::Bc1 => 2,
            Self::Bc3 => 3,
            Self::Rgba8Srgb => 4,
            Self::Bgra8Srgb => 5,
            Self::Bc1Srgb => 6,
            Self::Bc3Srgb => 7,
        }
    }
}



unsafe extern "C" {
    fn victory_metal_create(
        vertex: *const u8,
        vertex_len: usize,
        vertex_entry: *const c_char,
        fragment: *const u8,
        fragment_len: usize,
        fragment_entry: *const c_char,
        width: u32,
        height: u32,
        fragment_constant_dwords: u32,
        fragment_texture: bool,
        depth_compare: u32,
        depth_write: bool,
        blend_enabled: bool,
        write_mask: u8,
        alpha_source: bool,
        additive: bool,
        vertex_uv: bool,
        format: u32,
        error: *mut c_char,
        error_len: usize,
    ) -> *mut c_void;
    fn victory_metal_draw(
        renderer: *mut c_void,
        vertices: *const [f32; 4],
        vertex_count: u32,
        fragment_constants: *const u32,
        pixels: *mut u8,
        preserve: bool,
        viewport: *const f64,
        scissor: *const u32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_draw_indexed(
        renderer: *mut c_void,
        vertices: *const [f32; 4],
        vertex_count: u32,
        indices: *const c_void,
        index_count: u32,
        index_size: u32,
        base_vertex: i32,
        fragment_constants: *const u32,
        pixels: *mut u8,
        viewport: *const f64,
        scissor: *const u32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_set_texture(
        renderer: *mut c_void,
        width: u32,
        height: u32,
        levels: u32,
        rgba: *const u8,
        linear_filter: bool,
        format: u32,
        min_lod: f32,
        max_lod: f32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_clear(
        width: u32,
        height: u32,
        pitch: usize,
        pixels: *mut u8,
        color: *const u8,
        rects: *const [u32; 4],
        count: usize,
        bgra: bool,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_clear_float(
        width: u32, height: u32, pitch: usize, pixels: *mut u8,
        color: *const f32, rects: *const [u32; 4], count: usize,
        format: u32, error: *mut c_char, error_len: usize,
    ) -> bool;
    fn victory_metal_depth_pixels(renderer: *mut c_void, pixels: *mut f32, upload: bool,
                                  error: *mut c_char, error_len: usize) -> bool;
    fn victory_metal_clear_depth(width: u32, height: u32, pitch: usize, pixels: *mut u8,
                                value: f32, rects: *const [u32; 4], count: usize,
                                error: *mut c_char, error_len: usize) -> bool;
    fn victory_metal_destroy(renderer: *mut c_void);
}

fn native_error(error: &[c_char]) -> RenderError {
    // SAFETY: the native functions always NUL-terminate this initialized buffer.
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
    RenderError(if message.is_empty() {
        "Metal rendering failed".into()
    } else {
        message.into_owned()
    })
}
/// Clear clipped RGBA8 rectangles on Metal, preserving all other pixels and row padding.
/// An empty rectangle list clears the entire image.
pub fn clear_rgba8(
    width: u32,
    height: u32,
    pitch: usize,
    pixels: &mut [u8],
    color: [u8; 4],
    rects: &[[u32; 4]],
) -> Result<(), RenderError> {
    clear_color8(ColorFormat::Rgba8, width, height, pitch, pixels, color, rects)
}

/// Clear clipped rectangles on a Metal color texture, preserving other pixels
/// and row padding. `color` is logical RGBA regardless of the byte layout.
/// An empty rectangle list clears the entire image.
pub fn clear_color8(
    format: ColorFormat,
    width: u32,
    height: u32,
    pitch: usize,
    pixels: &mut [u8],
    color: [u8; 4],
    rects: &[[u32; 4]],
) -> Result<(), RenderError> {
    let row = (width as usize).checked_mul(4);
    let len = height.checked_sub(1).and_then(|h| pitch.checked_mul(h as usize))
        .and_then(|n| row.and_then(|r| n.checked_add(r)));
    if width == 0 || height == 0 || row.is_none_or(|r| pitch < r)
        || len.is_none_or(|n| pixels.len() < n)
        || rects.iter().any(|r| r[0] > r[2] || r[1] > r[3] || r[2] > width || r[3] > height)
    {
        return Err(RenderError("invalid RGBA8 clear dimensions or rectangle".into()));
    }
    let mut error = [0 as c_char; 1024];
    // SAFETY: the validated image and rectangle slices remain live throughout
    // the synchronous operation; the native implementation respects pitch.
    let ok = unsafe {
        victory_metal_clear(width, height, pitch, pixels.as_mut_ptr(), color.as_ptr(),
                            rects.as_ptr(), rects.len(), format.is_bgra(), error.as_mut_ptr(), error.len())
    };
    if ok { Ok(()) } else { Err(native_error(&error)) }
}

/// Clear with linear RGBA floats using a Metal render target of `format`.
/// sRGB targets encode RGB on store; alpha remains linear. Rectangles are
/// [left, top, right, bottom], in bounds, and an empty list clears the image.
/// Pixels outside rectangles and row padding are preserved as raw bytes.
pub fn clear_color_float(
    format: ColorFormat, width: u32, height: u32, pitch: usize, pixels: &mut [u8],
    color: [f32; 4], rects: &[[u32; 4]],
) -> Result<(), RenderError> {
    let row = (width as usize).checked_mul(4);
    let len = height.checked_sub(1).and_then(|h| pitch.checked_mul(h as usize))
        .and_then(|n| row.and_then(|r| n.checked_add(r)));
    if width == 0 || height == 0 || row.is_none_or(|r| pitch < r)
        || len.is_none_or(|n| pixels.len() < n)
        || rects.iter().any(|r| r[0] > r[2] || r[1] > r[3] || r[2] > width || r[3] > height)
    {
        return Err(RenderError("invalid RGBA8 clear dimensions or rectangle".into()));
    }
    let mut error = [0 as c_char; 1024];
    // SAFETY: all slices are validated and remain live for the synchronous call.
    let ok = unsafe {
        victory_metal_clear_float(width, height, pitch, pixels.as_mut_ptr(), color.as_ptr(),
                                  rects.as_ptr(), rects.len(), format.native_value(),
                                  error.as_mut_ptr(), error.len())
    };
    if ok { Ok(()) } else { Err(native_error(&error)) }
}
/// Clear a Depth32Float image on the GPU, preserving pixels outside the rectangles and padding.
pub fn clear_depth32(width: u32, height: u32, pitch: usize, pixels: &mut [u8],
                     value: f32, rects: &[[u32; 4]]) -> Result<(), RenderError> {
    let row = (width as usize).checked_mul(4);
    let len = height.checked_sub(1).and_then(|h| pitch.checked_mul(h as usize))
        .and_then(|n| row.and_then(|r| n.checked_add(r)));
    if width == 0 || height == 0 || row.is_none_or(|r| pitch < r)
        || len.is_none_or(|n| pixels.len() < n) || !value.is_finite() || !(0.0..=1.0).contains(&value)
        || rects.iter().any(|r| r[0] > r[2] || r[1] > r[3] || r[2] > width || r[3] > height)
    {
        return Err(RenderError("invalid depth clear dimensions, value or rectangle".into()));
    }
    let mut error = [0 as c_char; 1024];
    // SAFETY: validated buffers remain live for the synchronous operation.
    let ok = unsafe { victory_metal_clear_depth(width, height, pitch, pixels.as_mut_ptr(), value,
                                                rects.as_ptr(), rects.len(), error.as_mut_ptr(), error.len()) };
    if ok { Ok(()) } else { Err(native_error(&error)) }
}

/// D3D12 depth comparison (1 through 8) and depth-write enable.
#[derive(Clone, Copy, Debug)]
pub struct DepthState {
    pub write: bool,
    pub compare: u32,
}

/// Restricted source-alpha or additive blending and D3D RGBA channel write mask.
#[derive(Clone, Copy, Debug)]
pub struct BlendState {
    pub enabled: bool,
    pub write_mask: u8,
    pub alpha_source: bool,
    pub additive: bool,
}


/// A persistent Metal pipeline and RGBA8/BGRA8 unorm or sRGB offscreen target.
///
/// Both metallibs must be produced by Apple Metal Shader Converter. Metal
/// attribute 11 is a per-vertex float4 at buffer slot 6, offset 0. The UV
/// layout additionally supplies attribute 12 as a float2 at offset 16, with
/// stride 24 rather than 16. Supported fragment bindings are `b0, space0`
/// at offset 0 in the converter's automatic linear argument buffer, and
/// texture and sampler heap index 0. The caller must verify those layouts via
/// converter reflection and translated shader metadata.
/// The output is tightly packed in the configured color format, top row first.
pub struct Renderer {
    handle: NonNull<c_void>,
    pixel_len: usize,
    fragment_constant_dwords: usize,
    fragment_texture: bool,
    texture_bound: bool,
    depth_enabled: bool,
    vertex_uv: bool,
    width: u32,
    height: u32,
    viewport: [f64; 6],
    scissor: [u32; 4],
}

/// Texture filtering; address mode is clamp-to-edge.
#[derive(Clone, Copy)]
pub enum TextureFilter {
    Nearest,
    Linear,
}

/// Indices for a triangle-list draw.
pub enum IndexData<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl Renderer {
    pub fn new(
        vertex_metallib: &[u8],
        vertex_entry: &str,
        fragment_metallib: &[u8],
        fragment_entry: &str,
        width: u32,
        height: u32,
        fragment_constant_dwords: u32,
        fragment_texture: bool,
    ) -> Result<Self, RenderError> {
        Self::new_configured(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                             width, height, fragment_constant_dwords, fragment_texture, None,
                             BlendState { enabled: false, write_mask: 15, alpha_source: false, additive: false })
    }

    pub fn new_depth(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, state: DepthState,
    ) -> Result<Self, RenderError> {
        Self::new_configured(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                             width, height, fragment_constant_dwords, fragment_texture, Some(state),
                             BlendState { enabled: false, write_mask: 15, alpha_source: false, additive: false })
    }

    pub fn new_configured(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, depth: Option<DepthState>, blend: BlendState,
    ) -> Result<Self, RenderError> {
        Self::new_configured_format(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                                    width, height, fragment_constant_dwords, fragment_texture,
                                    depth, blend, ColorFormat::Rgba8)
    }

    /// Create a position-only pipeline with the specified color target format.
    pub fn new_configured_format(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, depth: Option<DepthState>, blend: BlendState, format: ColorFormat,
    ) -> Result<Self, RenderError> {
        Self::new_configured_inner(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                                   width, height, fragment_constant_dwords, fragment_texture,
                                   depth, blend, false, format)
    }

    /// Create a pipeline with float4 position and independent float2 UV vertices.
    pub fn new_configured_uv(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, depth: Option<DepthState>, blend: BlendState,
    ) -> Result<Self, RenderError> {
        Self::new_configured_uv_format(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                                       width, height, fragment_constant_dwords, fragment_texture,
                                       depth, blend, ColorFormat::Rgba8)
    }

    /// Create a position/UV pipeline with the specified color target format.
    pub fn new_configured_uv_format(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, depth: Option<DepthState>, blend: BlendState, format: ColorFormat,
    ) -> Result<Self, RenderError> {
        Self::new_configured_inner(vertex_metallib, vertex_entry, fragment_metallib, fragment_entry,
                                   width, height, fragment_constant_dwords, fragment_texture,
                                   depth, blend, true, format)
    }

    fn new_configured_inner(
        vertex_metallib: &[u8], vertex_entry: &str, fragment_metallib: &[u8],
        fragment_entry: &str, width: u32, height: u32, fragment_constant_dwords: u32,
        fragment_texture: bool, depth: Option<DepthState>, blend: BlendState, vertex_uv: bool,
        format: ColorFormat,
    ) -> Result<Self, RenderError> {
        if depth.is_some_and(|state| !(1..=8).contains(&state.compare)) {
            return Err(RenderError("invalid depth comparison".into()));
        }
        if blend.write_mask & !15 != 0 {
            return Err(RenderError("invalid color write mask".into()));
        }
        if width == 0 || height == 0 || vertex_metallib.is_empty() || fragment_metallib.is_empty() {
            return Err(RenderError(
                "nonzero dimensions and both metallibs are required".into(),
            ));
        }
        if fragment_constant_dwords % 4 != 0 || fragment_constant_dwords > u32::MAX / 4 {
            return Err(RenderError(
                "fragment constant size must be a multiple of 16 bytes and fit in 32 bits".into(),
            ));
        }
        let pixel_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| RenderError("image dimensions overflow".into()))?;
        let entry =
            |s: &str| CString::new(s).map_err(|_| RenderError("entry name contains NUL".into()));
        let vertex_entry = entry(vertex_entry)?;
        let fragment_entry = entry(fragment_entry)?;
        let mut error = [0 as c_char; 1024];
        // SAFETY: input slices and strings remain alive for the synchronous call;
        // Metal owns the resulting libraries and pipeline independently.
        let ptr = unsafe {
            victory_metal_create(
                vertex_metallib.as_ptr(),
                vertex_metallib.len(),
                vertex_entry.as_ptr(),
                fragment_metallib.as_ptr(),
                fragment_metallib.len(),
                fragment_entry.as_ptr(),
                width,
                height,
                fragment_constant_dwords,
                fragment_texture,
                depth.map_or(0, |s| s.compare),
                depth.is_some_and(|s| s.write),
                blend.enabled,
                blend.write_mask,
                blend.alpha_source,
                blend.additive,
                vertex_uv,
                format.native_value(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let handle = NonNull::new(ptr).ok_or_else(|| native_error(&error))?;
        Ok(Self {
            handle,
            pixel_len,
            fragment_constant_dwords: fragment_constant_dwords as usize,
            fragment_texture,
            texture_bound: false,
            depth_enabled: depth.is_some(),
            vertex_uv,
            width,
            height,
            viewport: [0.0, 0.0, f64::from(width), f64::from(height), 0.0, 1.0],
            scissor: [0, 0, width, height],
        })
    }

    /// Upload tightly packed depth values to the persistent attachment.
    pub fn set_depth_pixels(&mut self, pixels: &[f32]) -> Result<(), RenderError> {
        if !self.depth_enabled || pixels.len() != self.pixel_len / 4 {
            return Err(RenderError("expected an enabled depth attachment and exact-sized pixels".into()));
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: native upload only reads the exact-sized slice synchronously.
        let ok = unsafe { victory_metal_depth_pixels(self.handle.as_ptr(), pixels.as_ptr().cast_mut(), true,
                                                     error.as_mut_ptr(), error.len()) };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    /// Download the persistent depth attachment into tightly packed pixels.
    pub fn read_depth_pixels(&mut self, pixels: &mut [f32]) -> Result<(), RenderError> {
        if !self.depth_enabled || pixels.len() != self.pixel_len / 4 {
            return Err(RenderError("expected an enabled depth attachment and exact-sized pixels".into()));
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: native download writes exactly the checked slice synchronously.
        let ok = unsafe { victory_metal_depth_pixels(self.handle.as_ptr(), pixels.as_mut_ptr(), false,
                                                     error.as_mut_ptr(), error.len()) };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    /// Set the viewport and target-clipped scissor used by subsequent draws.
    /// A zero-width or zero-height scissor clips all fragments.
    pub fn set_viewport_scissor(
        &mut self,
        viewport: [f64; 6],
        scissor: [u32; 4],
    ) -> Result<(), RenderError> {
        if !viewport.iter().all(|value| value.is_finite())
            || viewport[2] <= 0.0 || viewport[3] <= 0.0
            || viewport[4] < 0.0 || viewport[5] > 1.0 || viewport[4] > viewport[5]
        {
            return Err(RenderError("invalid viewport".into()));
        }
        if scissor[0].checked_add(scissor[2]).map_or(true, |end| end > self.width)
            || scissor[1].checked_add(scissor[3]).map_or(true, |end| end > self.height)
        {
            return Err(RenderError("scissor exceeds render target".into()));
        }
        self.viewport = viewport;
        self.scissor = scissor;
        Ok(())
    }

    /// Upload RGBA8 texels and bind the sole fragment texture and sampler.
    /// The converted fragment shader must expect a 2D-array texture view.
    pub fn set_fragment_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        filter: TextureFilter,
    ) -> Result<(), RenderError> {
        self.set_fragment_texture_format(width, height, rgba, filter, ColorFormat::Rgba8)
    }

    /// Upload tightly packed texels in the specified byte layout.
    pub fn set_fragment_texture_format(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
        filter: TextureFilter,
        format: ColorFormat,
    ) -> Result<(), RenderError> {
        self.set_fragment_texture_mips(width, height, 1, pixels, filter, format.into(), 0.0, 0.0)
    }

    /// Upload a tightly packed mip chain as a 2D-array texture with one slice.
    /// Levels are consecutive, starting at `width` by `height`. BC levels use
    /// tightly packed 4x4 blocks, including partial blocks at the edges.
    pub fn set_fragment_texture_mips(
        &mut self,
        width: u32,
        height: u32,
        levels: u32,
        pixels: &[u8],
        filter: TextureFilter,
        format: TextureFormat,
        min_lod: f32,
        max_lod: f32,
    ) -> Result<(), RenderError> {
        if !self.fragment_texture || width == 0 || height == 0 || levels == 0
            || levels > u32::BITS - width.max(height).leading_zeros()
            || !min_lod.is_finite() || !max_lod.is_finite()
            || min_lod < 0.0 || min_lod > max_lod
        {
            return Err(RenderError("invalid texture dimensions, mip count, binding or LOD range".into()));
        }
        let (mut w, mut h, mut size) = (width, height, 0usize);
        for _ in 0..levels {
            let (columns, rows, bytes_per_block) = match format {
                TextureFormat::Rgba8 | TextureFormat::Bgra8 | TextureFormat::Rgba8Srgb | TextureFormat::Bgra8Srgb => (w as usize, h as usize, 4),
                TextureFormat::Bc1 | TextureFormat::Bc1Srgb => ((w as usize).div_ceil(4), (h as usize).div_ceil(4), 8),
                TextureFormat::Bc3 | TextureFormat::Bc3Srgb => ((w as usize).div_ceil(4), (h as usize).div_ceil(4), 16),
            };
            size = columns.checked_mul(rows)
                .and_then(|n| n.checked_mul(bytes_per_block))
                .and_then(|n| size.checked_add(n))
                .ok_or_else(|| RenderError("texture dimensions overflow".into()))?;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        if pixels.len() != size {
            return Err(RenderError("expected tightly packed mip data".into()));
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: the renderer and validated input buffer live through the
        // synchronous upload, which retains its own Metal texture.
        let ok = unsafe {
            victory_metal_set_texture(
                self.handle.as_ptr(),
                width,
                height,
                levels,
                pixels.as_ptr(),
                matches!(filter, TextureFilter::Linear),
                format.native_value(),
                min_lod,
                max_lod,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok {
            self.texture_bound = true;
            Ok(())
        } else {
            Err(native_error(&error))
        }
    }

    /// Draw a triangle list with the configured fragment b0 constants.
    /// The exact configured number of dwords must be supplied on every draw.
    /// Pixels not covered by the triangles are cleared to transparent black.
    pub fn draw(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        self.draw_inner(vertices, fragment_constants, pixels, false)
    }

    /// Draw a triangle list over existing tightly packed pixels in the configured format.
    /// Pixels not covered by the triangles retain their original values.
    pub fn draw_preserving(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        self.draw_inner(vertices, fragment_constants, pixels, true)
    }

    /// Draw indexed triangles over existing tightly packed pixels in the configured format.
    /// Each index plus `base_vertex` must identify an element of `vertices`.
    pub fn draw_indexed_preserving(
        &mut self,
        vertices: &[[f32; 4]],
        indices: IndexData<'_>,
        base_vertex: i32,
        fragment_constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        if self.vertex_uv {
            return Err(RenderError("renderer requires position and UV vertices".into()));
        }
        let valid_index = |index: u32| {
            let vertex = i64::from(index) + i64::from(base_vertex);
            vertex >= 0 && (vertex as u64) < vertices.len() as u64
        };
        let (ptr, len, index_size, valid) = match indices {
            IndexData::U16(data) => (data.as_ptr().cast::<c_void>(), data.len(), 2u32,
                                     data.iter().all(|&i| valid_index(u32::from(i)))),
            IndexData::U32(data) => (data.as_ptr().cast::<c_void>(), data.len(), 4u32,
                                     data.iter().all(|&i| valid_index(i))),
        };
        if vertices.is_empty() || len == 0 || len % 3 != 0 || !valid
            || pixels.len() != self.pixel_len
            || fragment_constants.len() != self.fragment_constant_dwords
        {
            return Err(RenderError("expected valid nonempty indexed triangles and exact-sized constants and output buffers".into()));
        }
        if self.fragment_texture && !self.texture_bound {
            return Err(RenderError("fragment texture has not been bound".into()));
        }
        let vertex_count = u32::try_from(vertices.len())
            .map_err(|_| RenderError("too many vertices".into()))?;
        let index_count = u32::try_from(len)
            .map_err(|_| RenderError("too many indices".into()))?;
        vertices.len().checked_mul(std::mem::size_of::<[f32; 4]>())
            .ok_or_else(|| RenderError("vertex buffer size overflow".into()))?;
        len.checked_mul(index_size as usize)
            .ok_or_else(|| RenderError("index buffer size overflow".into()))?;
        if self.scissor[2] == 0 || self.scissor[3] == 0 {
            return Ok(());
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: all input and output slices remain live for the synchronous
        // call; counts, byte sizes, and indexed vertex bounds were checked.
        let ok = unsafe {
            victory_metal_draw_indexed(
                self.handle.as_ptr(), vertices.as_ptr(), vertex_count, ptr,
                index_count, index_size, base_vertex, fragment_constants.as_ptr(),
                pixels.as_mut_ptr(), self.viewport.as_ptr(), self.scissor.as_ptr(),
                error.as_mut_ptr(), error.len(),
            )
        };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    /// Draw position/UV triangles over the existing pixels, optionally indexed.
    /// Each index plus its base vertex must identify an element of `vertices`.
    pub fn draw_uv_preserving(
        &mut self,
        vertices: &[[f32; 6]],
        indices: Option<(IndexData<'_>, i32)>,
        constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        if !self.vertex_uv {
            return Err(RenderError("renderer requires position-only vertices".into()));
        }
        if vertices.is_empty() || pixels.len() != self.pixel_len
            || constants.len() != self.fragment_constant_dwords
        {
            return Err(RenderError("expected nonempty vertices and exact-sized constants and output buffers".into()));
        }
        if self.fragment_texture && !self.texture_bound {
            return Err(RenderError("fragment texture has not been bound".into()));
        }
        let vertex_count = u32::try_from(vertices.len())
            .map_err(|_| RenderError("too many vertices".into()))?;
        vertices.len().checked_mul(std::mem::size_of::<[f32; 6]>())
            .ok_or_else(|| RenderError("vertex buffer size overflow".into()))?;
        let indexed = if let Some((indices, base_vertex)) = indices {
            let valid_index = |index: u32| {
                let vertex = i64::from(index) + i64::from(base_vertex);
                vertex >= 0 && (vertex as u64) < vertices.len() as u64
            };
            let (ptr, len, size, valid) = match indices {
                IndexData::U16(data) => (data.as_ptr().cast::<c_void>(), data.len(), 2u32,
                                         data.iter().all(|&i| valid_index(u32::from(i)))),
                IndexData::U32(data) => (data.as_ptr().cast::<c_void>(), data.len(), 4u32,
                                         data.iter().all(|&i| valid_index(i))),
            };
            if len == 0 || len % 3 != 0 || !valid {
                return Err(RenderError("expected valid nonempty indexed triangles".into()));
            }
            let count = u32::try_from(len).map_err(|_| RenderError("too many indices".into()))?;
            len.checked_mul(size as usize)
                .ok_or_else(|| RenderError("index buffer size overflow".into()))?;
            Some((ptr, count, size, base_vertex))
        } else {
            if vertices.len() % 3 != 0 {
                return Err(RenderError("expected a nonempty triangle list".into()));
            }
            None
        };
        if self.scissor[2] == 0 || self.scissor[3] == 0 {
            return Ok(());
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: the renderer uses the UV layout, and all slices, sizes and
        // indexed vertex bounds remain valid for the synchronous native draw.
        let ok = unsafe {
            if let Some((ptr, count, size, base_vertex)) = indexed {
                victory_metal_draw_indexed(
                    self.handle.as_ptr(), vertices.as_ptr().cast(), vertex_count, ptr,
                    count, size, base_vertex, constants.as_ptr(), pixels.as_mut_ptr(),
                    self.viewport.as_ptr(), self.scissor.as_ptr(), error.as_mut_ptr(), error.len(),
                )
            } else {
                victory_metal_draw(
                    self.handle.as_ptr(), vertices.as_ptr().cast(), vertex_count,
                    constants.as_ptr(), pixels.as_mut_ptr(), true, self.viewport.as_ptr(),
                    self.scissor.as_ptr(), error.as_mut_ptr(), error.len(),
                )
            }
        };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    fn draw_inner(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
        preserve: bool,
    ) -> Result<(), RenderError> {
        if self.vertex_uv {
            return Err(RenderError("renderer requires position and UV vertices".into()));
        }
        if vertices.is_empty()
            || vertices.len() % 3 != 0
            || pixels.len() != self.pixel_len
            || fragment_constants.len() != self.fragment_constant_dwords
        {
            return Err(RenderError(
                "expected a nonempty triangle list and exact-sized constants and output buffers"
                    .into(),
            ));
        }
        if self.fragment_texture && !self.texture_bound {
            return Err(RenderError("fragment texture has not been bound".into()));
        }
        let count =
            u32::try_from(vertices.len()).map_err(|_| RenderError("too many vertices".into()))?;
        if preserve && (self.scissor[2] == 0 || self.scissor[3] == 0) {
            return Ok(());
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: the renderer is live, the buffers remain valid for the
        // synchronous draw, and their lengths were checked above.
        let ok = unsafe {
            victory_metal_draw(
                self.handle.as_ptr(),
                vertices.as_ptr(),
                count,
                fragment_constants.as_ptr(),
                pixels.as_mut_ptr(),
                preserve,
                self.viewport.as_ptr(),
                self.scissor.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok {
            Ok(())
        } else {
            Err(native_error(&error))
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: the handle is uniquely owned and is released exactly once.
        unsafe { victory_metal_destroy(self.handle.as_ptr()) }
    }
}
