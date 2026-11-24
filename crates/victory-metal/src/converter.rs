//! Runtime DXIL conversion through Apple's Metal Shader Converter C API.

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

/// Shader stage to extract from the converted Metal library.
#[derive(Clone, Copy, Debug)]
pub enum Stage {
    Vertex,
    Fragment,
    Mesh,
    Compute,
}

impl Stage {
    // Values from the public metal_irconverter.h IRShaderStage enum.
    fn raw(self) -> c_int {
        match self {
            Self::Vertex => 1,
            Self::Fragment => 2,
            Self::Mesh => 5,
            Self::Compute => 8,
        }
    }
}

type Pointer = *mut c_void;
type Destroy = unsafe extern "C" fn(Pointer);

struct Handle {
    ptr: Pointer,
    destroy: Destroy,
}

impl Handle {
    fn new(ptr: Pointer, destroy: Destroy, what: &str) -> Result<Self, String> {
        if ptr.is_null() {
            return Err(format!("Metal Shader Converter could not create {what}"));
        }
        Ok(Self { ptr, destroy })
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Each handle owns a non-null converter object, and is dropped before
        // the library providing its destructor is unloaded.
        unsafe { (self.destroy)(self.ptr) }
    }
}

/// Convert DXIL into a Metal library using the converter at `library`.
///
/// Loading the supplied dynamic library executes its initializers; callers must
/// supply a trusted Apple Metal Shader Converter library.
pub fn convert(library: &Path, dxil: &[u8], stage: Stage) -> Result<Vec<u8>, String> {
    // SAFETY: the caller supplies the path to a trusted converter library.
    let lib = unsafe { libloading::Library::new(library) }
        .map_err(|error| format!("loading converter {}: {error}", library.display()))?;

    // SAFETY: these signatures are from Apple's public metal_irconverter.h.
    // The library outlives its symbols and every converter-owned handle.
    unsafe {
        macro_rules! api {
            ($name:literal, $ty:ty) => {
                *lib.get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|error| format!("loading converter symbol {}: {error}", $name))?
            };
        }
        let compiler_create = api!("IRCompilerCreate", unsafe extern "C" fn() -> Pointer);
        let compiler_destroy = api!("IRCompilerDestroy", Destroy);
        let object_create = api!(
            "IRObjectCreateFromDXIL",
            unsafe extern "C" fn(*const u8, usize, c_int) -> Pointer
        );
        let object_destroy = api!("IRObjectDestroy", Destroy);
        let compile = api!(
            "IRCompilerAllocCompileAndLink",
            unsafe extern "C" fn(Pointer, *const c_char, Pointer, *mut Pointer) -> Pointer
        );
        let error_code = api!("IRErrorGetCode", unsafe extern "C" fn(Pointer) -> u32);
        let error_destroy = api!("IRErrorDestroy", Destroy);
        let binary_create = api!("IRMetalLibBinaryCreate", unsafe extern "C" fn() -> Pointer);
        let binary_destroy = api!("IRMetalLibBinaryDestroy", Destroy);
        let get_binary = api!(
            "IRObjectGetMetalLibBinary",
            unsafe extern "C" fn(Pointer, c_int, Pointer) -> bool
        );
        let binary_size = api!(
            "IRMetalLibGetBytecodeSize",
            unsafe extern "C" fn(Pointer) -> usize
        );
        let binary_copy = api!(
            "IRMetalLibGetBytecode",
            unsafe extern "C" fn(Pointer, *mut u8) -> usize
        );

        let compiler = Handle::new(compiler_create(), compiler_destroy, "compiler")?;
        // OwnershipNone: the input slice outlives the DXIL object.
        let input = Handle::new(
            object_create(dxil.as_ptr(), dxil.len(), 0),
            object_destroy,
            "DXIL object",
        )?;
        let mut error = ptr::null_mut();
        let result = compile(compiler.ptr, ptr::null(), input.ptr, &mut error);
        // Own both outputs before checking the error, so neither can leak if
        // the converter returns an object and a diagnostic together.
        let result = if result.is_null() {
            None
        } else {
            Some(Handle::new(result, object_destroy, "converted object")?)
        };
        if !error.is_null() {
            let error = Handle::new(error, error_destroy, "error")?;
            return Err(format!(
                "Metal Shader Converter failed (error code {})",
                error_code(error.ptr)
            ));
        }
        let result = result.ok_or_else(|| {
            "Metal Shader Converter could not create converted object".to_string()
        })?;
        let binary = Handle::new(binary_create(), binary_destroy, "Metal library")?;
        if !get_binary(result.ptr, stage.raw(), binary.ptr) {
            return Err("converter produced no Metal library for the requested stage".into());
        }
        let size = binary_size(binary.ptr);
        if size == 0 {
            return Err("converter produced an empty Metal library".into());
        }
        let mut data = vec![0u8; size];
        if binary_copy(binary.ptr, data.as_mut_ptr()) != size {
            return Err("converter returned an incomplete Metal library".into());
        }
        Ok(data)
    }
}
