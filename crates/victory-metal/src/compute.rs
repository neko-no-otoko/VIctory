//! Synchronous compute dispatch for shaders produced by Metal Shader Converter.
//!
//! Legacy dispatch uses a writable raw buffer at heap index 0. Raw heap
//! dispatch accepts ordered SRV and UAV entries. Callers must check the
//! converted shader's binding layout.

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
        cbv: *const u8,
        cbv_len: usize,
        cbv_offset: usize,
        groups: *const u32,
        threads: *const u32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_compute_dispatch_raw_views(
        handle: *mut c_void,
        output: *mut u8,
        output_len: usize,
        input: *const u8,
        input_len: usize,
        srv_offset: usize,
        srv_len: usize,
        uav_offset: usize,
        uav_len: usize,
        constants: *const u32,
        constant_bytes: usize,
        groups: *const u32,
        threads: *const u32,
        error: *mut c_char,
        error_len: usize,
    ) -> bool;
    fn victory_metal_compute_dispatch_raw_heap(
        handle: *mut c_void,
        resources: *const NativeRawHeapBuffer,
        resource_count: usize,
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

/// A full resource snapshot and its raw view, at its index in the descriptor heap.
pub struct RawHeapBuffer<'a> {
    pub bytes: &'a mut [u8],
    pub offset: usize,
    pub length: usize,
    pub writable: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRawHeapBuffer {
    bytes: *mut u8,
    size: usize,
    offset: usize,
    length: usize,
    writable: u32,
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
        self.dispatch_impl(buffer, constants, groups, threads, None)
    }

    /// Dispatches with a raw SRV and UAV at resource heap indices 0 and 1.
    /// View offsets and lengths are in bytes relative to their respective buffers.
    pub fn dispatch_raw_views(
        &mut self,
        output: &mut [u8],
        input: &[u8],
        srv: (usize, usize),
        uav: (usize, usize),
        constants: &[u32],
        groups: [u32; 3],
        threads: [u32; 3],
    ) -> Result<(), String> {
        if output.is_empty() || input.is_empty()
            || output.len() > u32::MAX as usize || input.len() > u32::MAX as usize
        {
            return Err("buffer size must be nonzero and fit in 32 bits".into());
        }
        if srv.1 == 0 || srv.0.checked_add(srv.1).is_none_or(|end| end > input.len())
            || uav.1 == 0 || uav.0.checked_add(uav.1).is_none_or(|end| end > output.len())
        {
            return Err("raw view must be nonempty and contained in its buffer".into());
        }
        if constants.len() < 4 || constants.len() % 4 != 0
            || constants.len() > (u32::MAX / 4) as usize
        {
            return Err("constant size must be at least 16 bytes, a multiple of 16 bytes, and fit in 32 bits".into());
        }
        if groups.contains(&0) || threads.contains(&0) {
            return Err("group and thread dimensions must be nonzero".into());
        }
        let mut error = [0 as c_char; 1024];
        // The native call synchronously uploads both buffers and copies output back.
        let ok = unsafe {
            victory_metal_compute_dispatch_raw_views(
                self.handle.as_ptr(), output.as_mut_ptr(), output.len(), input.as_ptr(),
                input.len(), srv.0, srv.1, uav.0, uav.1, constants.as_ptr(),
                constants.len() * 4, groups.as_ptr(), threads.as_ptr(), error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    /// Dispatches with ordered raw heap entries, copying writable resources back.
    pub fn dispatch_raw_heap(
        &mut self,
        resources: &mut [RawHeapBuffer<'_>],
        constants: &[u32],
        groups: [u32; 3],
        threads: [u32; 3],
    ) -> Result<(), String> {
        if resources.len() > 16 {
            return Err("at most 16 raw heap entries are supported".into());
        }
        if constants.len() > (u32::MAX / 4) as usize {
            return Err("constant size must fit in 32 bits".into());
        }
        if groups.contains(&0) || threads.contains(&0) {
            return Err("group and thread dimensions must be nonzero".into());
        }
        let empty = NativeRawHeapBuffer {
            bytes: std::ptr::null_mut(), size: 0, offset: 0, length: 0, writable: 0,
        };
        let mut native = [empty; 16];
        for (entry, resource) in native.iter_mut().zip(resources.iter_mut()) {
            if resource.bytes.is_empty() || resource.bytes.len() > u32::MAX as usize {
                return Err("buffer size must be nonzero and fit in 32 bits".into());
            }
            if resource.length == 0 || resource.offset.checked_add(resource.length)
                .is_none_or(|end| end > resource.bytes.len())
            {
                return Err("raw view must be nonempty and contained in its buffer".into());
            }
            *entry = NativeRawHeapBuffer {
                bytes: resource.bytes.as_mut_ptr(),
                size: resource.bytes.len(),
                offset: resource.offset,
                length: resource.length,
                writable: u32::from(resource.writable),
            };
        }
        let mut error = [0 as c_char; 1024];
        // Resources and constants remain borrowed until this synchronous call returns.
        let ok = unsafe {
            victory_metal_compute_dispatch_raw_heap(
                self.handle.as_ptr(), native.as_ptr(), resources.len(), constants.as_ptr(),
                constants.len() * 4, groups.as_ptr(), threads.as_ptr(),
                error.as_mut_ptr(), error.len(),
            )
        };
        if ok { Ok(()) } else { Err(native_error(&error)) }
    }

    /// Dispatches with a root CBV at the argument offset returned by
    /// `converter::convert_compute_with_root_cbv`. The CBV bytes are uploaded
    /// for this synchronous dispatch and may contain at most 64 KiB.
    pub fn dispatch_with_root_cbv(
        &mut self,
        buffer: &mut [u8],
        constants: &[u32],
        groups: [u32; 3],
        threads: [u32; 3],
        cbv: &[u8],
        cbv_offset: usize,
    ) -> Result<(), String> {
        if cbv.is_empty() || cbv.len() > 65536 {
            return Err("root CBV size must be in 1..=65536 bytes".into());
        }
        if cbv_offset % 8 != 0 || cbv_offset.checked_add(24).is_none() {
            return Err("invalid root CBV argument offset".into());
        }
        self.dispatch_impl(buffer, constants, groups, threads, Some((cbv, cbv_offset)))
    }

    fn dispatch_impl(
        &mut self,
        buffer: &mut [u8],
        constants: &[u32],
        groups: [u32; 3],
        threads: [u32; 3],
        cbv: Option<(&[u8], usize)>,
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
        let (cbv_ptr, cbv_len, cbv_offset) = match cbv {
            Some((bytes, offset)) => (bytes.as_ptr(), bytes.len(), offset),
            None => (std::ptr::null(), 0, 0),
        };
        // All pointers remain valid until the native synchronous dispatch returns.
        let ok = unsafe {
            victory_metal_compute_dispatch(
                self.handle.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                constants.as_ptr(),
                constants.len() * 4,
                cbv_ptr,
                cbv_len,
                cbv_offset,
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
