//! Synchronous compute dispatch for shaders produced by Metal Shader Converter.
//!
//! The shader must use a writable raw buffer at resource heap index 0 and a
//! b0 constant buffer at offset 0 of the automatic linear argument buffer.
//! Callers are responsible for checking the converted shader's binding layout.

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::NonNull;

unsafe extern "C" {
    fn victory_metal_compute_create(
        metallib: *const u8,
        metallib_len: usize,
        entry: *const c_char,
        error: *mut c_char,
        error_len: usize,
    ) -> *mut c_void;
    fn victory_metal_compute_dispatch(
        handle: *mut c_void,
        buffer: *mut u8,
        buffer_len: usize,
        constants: *const u32,
        constant_bytes: usize,
        groups: *const u32,
        threads: *const u32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_compute_destroy(handle: *mut c_void);
}

fn native_error(error: &[c_char]) -> String {
    // Native functions NUL-terminate the initialized error buffer.
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
    if message.is_empty() {
        "Metal compute failed".into()
    } else {
        message.into_owned()
    }
}

/// A retained Metal compute pipeline with reusable shared buffers.
pub struct Compute {
    handle: NonNull<c_void>,
}
// Metal objects may be used from any thread; dispatch requires exclusive access
// and completes before returning, so moving the retained handle is safe.
unsafe impl Send for Compute {}

impl Compute {
    pub fn new(metallib: &[u8], entry: &str) -> Result<Compute, String> {
        if metallib.is_empty() || entry.is_empty() {
            return Err("a metallib and entry name are required".into());
        }
        let entry = CString::new(entry).map_err(|_| "entry name contains NUL".to_string())?;
        let mut error = [0 as c_char; 1024];
        // Inputs remain alive for the call; the native handle owns its pipeline.
        let handle = unsafe {
            victory_metal_compute_create(
                metallib.as_ptr(),
                metallib.len(),
                entry.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        Ok(Self {
            handle: NonNull::new(handle).ok_or_else(|| native_error(&error))?,
        })
    }

    /// Uploads the buffer and constants, waits for execution, and copies the result back.
    pub fn dispatch(
        &mut self,
        buffer: &mut [u8],
        constants: &[u32],
        groups: [u32; 3],
        threads: [u32; 3],
    ) -> Result<(), String> {
        if buffer.is_empty() || buffer.len() > u32::MAX as usize {
            return Err("buffer size must be nonzero and fit in 32 bits".into());
        }
        if constants.len() < 4
            || constants.len() % 4 != 0
            || constants.len() > (u32::MAX / 4) as usize
        {
            return Err("constant size must be at least 16 bytes, a multiple of 16 bytes, and fit in 32 bits".into());
        }
        if groups.contains(&0) || threads.contains(&0) {
            return Err("group and thread dimensions must be nonzero".into());
        }
        let mut error = [0 as c_char; 1024];
        // All pointers remain valid until the native synchronous dispatch returns.
        let ok = unsafe {
            victory_metal_compute_dispatch(
                self.handle.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                constants.as_ptr(),
                constants.len() * 4,
                groups.as_ptr(),
                threads.as_ptr(),
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

impl Drop for Compute {
    fn drop(&mut self) {
        // The handle is uniquely owned and released once.
        unsafe { victory_metal_compute_destroy(self.handle.as_ptr()) }
    }
}
