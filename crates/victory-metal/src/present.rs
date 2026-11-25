//! On-screen Metal presentation for the loopback helper.
//!
//! `run_app` must run on the process main thread. Its worker may call `present`
//! synchronously; pixels are borrowed only until that call returns. The window is
//! created on the first frame, remains user-resizable, and is not recreated after
//! the user closes it. Frames are stretched to the view's backing-pixel size.

use crate::{native_error, RenderError};
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

static RUNNING: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn victory_present_run(start: extern "C" fn(*mut c_void), context: *mut c_void,
                           error: *mut c_char, error_len: usize) -> i32;
    fn victory_present_stop();
    fn victory_present_close();
    fn victory_present_resize(width: u32, height: u32, error: *mut c_char, error_len: usize) -> i32;
    fn victory_present_frame(width: u32, height: u32, format: u32, pitch: u32,
                             pixels: *const u8, capture: i32,
                             out_width: *mut u32, out_height: *mut u32,
                             out_pixels: *mut *mut u8, out_len: *mut usize,
                             error: *mut c_char, error_len: usize) -> i32;
    fn victory_present_free(pixels: *mut u8);
}

/// Backing drawable dimensions and, when requested, tightly packed BGRA8 pixels.
pub struct PresentResult {
    pub width: u32,
    pub height: u32,
    pub capture: Vec<u8>,
}

/// Run AppKit on the main thread until `worker` returns. Compute-only users
/// should not call this function and therefore do not initialize AppKit.
pub fn run_app<F: FnOnce() + Send + 'static>(worker: F) -> Result<(), RenderError> {
    struct Context<F> {
        worker: Option<F>,
        thread: Option<JoinHandle<()>>,
        start_error: Option<String>,
    }
    extern "C" fn start<F: FnOnce() + Send + 'static>(context: *mut c_void) {
        let context = unsafe { &mut *(context as *mut Context<F>) };
        let worker = context.worker.take().expect("present worker started twice");
        RUNNING.store(true, Ordering::Release);
        match thread::Builder::new().spawn(move || {
            struct Stop;
            impl Drop for Stop {
                fn drop(&mut self) {
                    RUNNING.store(false, Ordering::Release);
                    unsafe { victory_present_stop() }
                }
            }
            let _stop = Stop;
            worker();
        }) {
            Ok(thread) => context.thread = Some(thread),
            Err(error) => {
                RUNNING.store(false, Ordering::Release);
                context.start_error = Some(error.to_string());
                unsafe { victory_present_stop() };
            }
        }
    }
    let mut context = Context { worker: Some(worker), thread: None, start_error: None };
    let mut error = [0; 512];
    let status = unsafe { victory_present_run(start::<F>, &mut context as *mut _ as *mut c_void,
                                              error.as_mut_ptr(), error.len()) };
    if let Some(error) = context.start_error {
        return Err(RenderError(format!("could not start presentation worker: {error}")));
    }
    if let Some(thread) = context.thread {
        if thread.join().is_err() {
            return Err(RenderError("presentation worker panicked".into()));
        }
    }
    if status != 0 { return Err(native_error(&error)); }
    Ok(())
}

/// Present a top-row-first DXGI RGBA8 UNORM (28), RGBA8 UNORM_SRGB (29),
/// BGRA8 UNORM (87), or BGRA8 UNORM_SRGB (91) image. The sRGB formats'
/// encoded color bytes and alpha are preserved without sRGB decoding.
/// `pitch` is the source row stride in bytes. Completion, including optional GPU
/// drawable readback, precedes return.
pub fn present(width: u32, height: u32, dxgi_format: u32, pitch: u32,
               pixels: &[u8], capture: bool) -> Result<PresentResult, RenderError> {
    if !RUNNING.load(Ordering::Acquire) {
        return Err(RenderError("presentation application is not running".into()));
    }
    let required = victory_formats::presentation::source_len(width, height, pitch);
    if !matches!(dxgi_format, 28 | 29 | 87 | 91) || required.is_none_or(|n| pixels.len() < n) {
        return Err(RenderError("invalid presentation image".into()));
    }
    let (mut out_width, mut out_height, mut out_len) = (0, 0, 0);
    let mut out_pixels = ptr::null_mut();
    let mut error = [0; 512];
    let status = unsafe { victory_present_frame(width, height, dxgi_format, pitch, pixels.as_ptr(),
        i32::from(capture), &mut out_width, &mut out_height, &mut out_pixels, &mut out_len,
        error.as_mut_ptr(), error.len()) };
    if status != 0 { return Err(native_error(&error)); }
    let data = if out_len == 0 { Vec::new() } else {
        let data = unsafe { std::slice::from_raw_parts(out_pixels, out_len).to_vec() };
        unsafe { victory_present_free(out_pixels) };
        data
    };
    Ok(PresentResult { width: out_width, height: out_height, capture: data })
}

/// Resize the presentation window's content area in AppKit points.
pub fn resize(width: u32, height: u32) -> Result<(), RenderError> {
    if !RUNNING.load(Ordering::Acquire) {
        return Err(RenderError("presentation application is not running".into()));
    }
    let mut error = [0; 512];
    let status = unsafe { victory_present_resize(width, height, error.as_mut_ptr(), error.len()) };
    if status != 0 { return Err(native_error(&error)); }
    Ok(())
}

/// Close the window through AppKit's normal close path. Call from the worker
/// while `run_app` is running, after a frame has created the window.
/// Subsequent presentation attempts return an error.
pub fn close() -> Result<(), RenderError> {
    if !RUNNING.load(Ordering::Acquire) {
        return Err(RenderError("presentation application is not running".into()));
    }
    unsafe { victory_present_close() }
    Ok(())
}
