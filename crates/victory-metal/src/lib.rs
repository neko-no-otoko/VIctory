//! Small native Metal offscreen primitive, not a console GPU backend.
//!
//! Renders a procedural triangle from user-provided Metal vertex source and
//! a converted fragment metallib, returning tightly packed RGBA8 pixels. No
//! resources, vertex buffers, depth, blending or presentation are bound.

#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString, c_char};
use std::fmt;

#[derive(Debug)]
pub struct RenderError(String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

unsafe extern "C" {
    fn victory_metal_render(
        vertex_source: *const c_char,
        vertex_entry: *const c_char,
        fragment: *const u8,
        fragment_len: usize,
        fragment_entry: *const c_char,
        width: u32,
        height: u32,
        pixels: *mut u8,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
}

/// Render a single procedural triangle to RGBA8Unorm; rows are top-to-bottom.
/// The vertex function must use `vertex_id` rather than vertex buffers.
/// The fragment function must not require resources or function constants.
pub fn render_triangle(
    vertex_source: &str,
    vertex_entry: &str,
    fragment_metallib: &[u8],
    fragment_entry: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, RenderError> {
    if width == 0 || height == 0 || fragment_metallib.is_empty() {
        return Err(RenderError(
            "nonzero dimensions and a fragment metallib are required".into(),
        ));
    }
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| RenderError("image dimensions overflow".into()))?;
    let text = |s: &str| {
        CString::new(s).map_err(|_| RenderError("source or entry name contains NUL".into()))
    };
    let vertex_source = text(vertex_source)?;
    let vertex_entry = text(vertex_entry)?;
    let fragment_entry = text(fragment_entry)?;
    let mut pixels = vec![0u8; size];
    let mut error = [0 as c_char; 1024];
    // SAFETY: all pointers refer to live allocations with the specified sizes;
    // the synchronous native implementation does not retain them after return.
    let ok = unsafe {
        victory_metal_render(
            vertex_source.as_ptr(),
            vertex_entry.as_ptr(),
            fragment_metallib.as_ptr(),
            fragment_metallib.len(),
            fragment_entry.as_ptr(),
            width,
            height,
            pixels.as_mut_ptr(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if ok {
        Ok(pixels)
    } else {
        // SAFETY: the native function always NUL-terminates this initialized buffer.
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
        Err(RenderError(if message.is_empty() {
            "Metal rendering failed".into()
        } else {
            message.into_owned()
        }))
    }
}
