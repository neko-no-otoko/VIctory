//! Offline DXIL conversion through the user's Apple Metal Shader Converter.

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;

// Values from Apple's public metal_irconverter.h IRShaderStage enum.
#[derive(Clone, Copy, ValueEnum)]
pub enum Stage {
    Vertex,
    Fragment,
    Mesh,
    Compute,
}

impl Stage {
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
    fn new(ptr: Pointer, destroy: Destroy, what: &str) -> Result<Self> {
        ensure!(
            !ptr.is_null(),
            "Metal Shader Converter could not create {what}"
        );
        Ok(Self { ptr, destroy })
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: each handle owns a non-null object returned by the converter,
        // with its corresponding destructor, while the library is still loaded.
        unsafe { (self.destroy)(self.ptr) }
    }
}

pub fn convert(input: &Path, output: &Path, library: &Path, stage: Stage) -> Result<()> {
    victory_core::ensure_outside_repo(output)?;
    let bytes = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    // SAFETY: loading a dylib executes its initializers. The user explicitly
    // supplies the path to the trusted Apple converter to load.
    let lib = unsafe { libloading::Library::new(library) }
        .with_context(|| format!("loading converter {}", library.display()))?;
    // SAFETY: these are the documented C signatures in metal_irconverter.h;
    // `lib` outlives every symbol and all converter-owned objects below.
    unsafe {
        macro_rules! api {
            ($name:literal, $ty:ty) => {
                *lib.get::<$ty>(concat!($name, "\0").as_bytes())
                    .with_context(|| concat!("loading converter symbol ", $name))?
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
        // OwnershipNone: `bytes` outlives the input object.
        let dxil = Handle::new(
            object_create(bytes.as_ptr(), bytes.len(), 0),
            object_destroy,
            "DXIL object",
        )?;
        let mut error = ptr::null_mut();
        let result = compile(compiler.ptr, ptr::null(), dxil.ptr, &mut error);
        if !error.is_null() {
            let error = Handle::new(error, error_destroy, "error")?;
            let code = error_code(error.ptr);
            if result.is_null() {
                bail!("Metal Shader Converter failed (error code {code})");
            }
        }
        let result = Handle::new(result, object_destroy, "converted object")?;
        let binary = Handle::new(binary_create(), binary_destroy, "Metal library")?;
        ensure!(
            get_binary(result.ptr, stage.raw(), binary.ptr),
            "converter produced no Metal library for the requested stage"
        );
        let size = binary_size(binary.ptr);
        ensure!(size != 0, "converter produced an empty Metal library");
        let mut data = vec![0u8; size];
        ensure!(
            binary_copy(binary.ptr, data.as_mut_ptr()) == size,
            "converter returned an incomplete Metal library"
        );
        std::fs::write(output, &data).with_context(|| format!("writing {}", output.display()))?;
    }
    println!("wrote {}", output.display());
    Ok(())
}
