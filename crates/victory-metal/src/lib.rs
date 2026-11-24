//! Native offscreen Metal triangle rendering, not a console GPU backend.
//!
//! `Renderer` retains its device, pipeline, render target and readback buffer
//! across draws. It supports one float4 vertex attribute and a fragment b0
//! constant buffer using the converter's linear argument-buffer layout.

#![cfg(target_os = "macos")]

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
        error: *mut c_char,
        error_len: usize,
    ) -> *mut c_void;
    fn victory_metal_draw(
        renderer: *mut c_void,
        vertices: *const [f32; 4],
        vertex_count: u32,
        fragment_constants: *const u32,
        pixels: *mut u8,
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

/// A persistent Metal pipeline and RGBA8Unorm offscreen target.
///
/// Both metallibs must be produced by Apple Metal Shader Converter. Vertex
/// attribute 0 is a per-vertex float4 at buffer slot 6. The only supported
/// shader resource is fragment `b0, space0` at offset 0 in the converter's
/// automatic linear argument buffer; the caller must verify that layout via
/// converter reflection. The output is tightly packed RGBA8, top row first.
pub struct Renderer {
    handle: NonNull<c_void>,
    pixel_len: usize,
    fragment_constant_dwords: usize,
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
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let handle = NonNull::new(ptr).ok_or_else(|| native_error(&error))?;
        Ok(Self {
            handle,
            pixel_len,
            fragment_constant_dwords: fragment_constant_dwords as usize,
        })
    }

    /// Draw a triangle list with the configured fragment b0 constants.
    /// The exact configured number of dwords must be supplied on every draw.
    pub fn draw(
        &mut self,
        vertices: &[[f32; 4]],
        fragment_constants: &[u32],
        pixels: &mut [u8],
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
        let count =
            u32::try_from(vertices.len()).map_err(|_| RenderError("too many vertices".into()))?;
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
