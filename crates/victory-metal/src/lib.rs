//! Native offscreen Metal triangle rendering, not a console GPU backend.
//!
//! `Renderer` retains its device, pipeline, render target and readback buffer
//! across draws. It supports one float4 vertex attribute, fragment b0
//! constants, and one 2D texture/sampler in the converter's resource heaps.

#![cfg(target_os = "macos")]

pub mod compute;
pub mod converter;

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
        rgba: *const u8,
        linear_filter: bool,
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
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
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
                            rects.as_ptr(), rects.len(), error.as_mut_ptr(), error.len())
    };
    if ok { Ok(()) } else { Err(native_error(&error)) }
}


/// A persistent Metal pipeline and RGBA8Unorm offscreen target.
///
/// Both metallibs must be produced by Apple Metal Shader Converter. Vertex
/// attribute 0 is a per-vertex float4 at buffer slot 6. Supported fragment
/// bindings are `b0, space0` at offset 0 in the converter's automatic linear
/// argument buffer, and texture and sampler heap index 0. The caller must
/// verify those layouts via converter reflection and translated shader metadata.
/// The output is tightly packed RGBA8, top row first.
pub struct Renderer {
    handle: NonNull<c_void>,
    pixel_len: usize,
    fragment_constant_dwords: usize,
    fragment_texture: bool,
    texture_bound: bool,
    width: u32,
    height: u32,
    viewport: [f64; 6],
    scissor: [u32; 4],
}

/// Texture filtering; address mode is clamp-to-edge and LOD is fixed to zero.
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
            width,
            height,
            viewport: [0.0, 0.0, f64::from(width), f64::from(height), 0.0, 1.0],
            scissor: [0, 0, width, height],
        })
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
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| RenderError("texture dimensions overflow".into()))?;
        if !self.fragment_texture || width == 0 || height == 0 || rgba.len() != size {
            return Err(RenderError("expected enabled texture binding, nonzero dimensions and tightly packed RGBA8 texels".into()));
        }
        let mut error = [0 as c_char; 1024];
        // SAFETY: the renderer and validated input buffer live through the
        // synchronous upload, which retains its own Metal texture.
        let ok = unsafe {
            victory_metal_set_texture(
                self.handle.as_ptr(),
                width,
                height,
                rgba.as_ptr(),
                matches!(filter, TextureFilter::Linear),
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

    /// Draw a triangle list over the existing tightly packed RGBA8 pixels.
    /// Pixels not covered by the triangles retain their original values.
    pub fn draw_preserving(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        self.draw_inner(vertices, fragment_constants, pixels, true)
    }

    /// Draw indexed triangles over the existing tightly packed RGBA8 pixels.
    /// Each index plus `base_vertex` must identify an element of `vertices`.
    pub fn draw_indexed_preserving(
        &mut self,
        vertices: &[[f32; 4]],
        indices: IndexData<'_>,
        base_vertex: i32,
        fragment_constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
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

    fn draw_inner(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
        preserve: bool,
    ) -> Result<(), RenderError> {
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
