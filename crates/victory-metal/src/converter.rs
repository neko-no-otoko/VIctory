//! Runtime DXIL conversion through Apple's Metal Shader Converter C API.

use std::ffi::{c_char, c_int, c_void, CStr};
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

// IRShaderReflectionResourceLocation from metal_irconverter.h.
#[repr(C)]
#[derive(Default)]
struct ResourceLocation {
    kind: u32,
    space: u32,
    slot: u32,
    offset: u32,
    size: u64,
    name: *const c_char,
}

// IRVersionedVSInfo / IRVSInfo_1_0 in metal_irconverter.h.
#[repr(C)]
struct VertexInputInfo {
    name: *const c_char,
    attribute_index: u8,
}

#[repr(C)]
struct VertexInfoV1 {
    instance_id_index: c_int,
    vertex_id_index: c_int,
    vertex_output_size_in_bytes: u32,
    needs_draw_params: bool,
    vertex_inputs: *mut VertexInputInfo,
    num_vertex_inputs: usize,
}

#[repr(C)]
struct VertexInfo {
    version: c_int,
    info: VertexInfoV1,
}

/// A converter-reported vertex semantic (including its decimal index) and
/// the Metal vertex attribute index assigned to it.
#[derive(Debug, Clone)]
pub struct VertexAttribute {
    pub name: String,
    pub index: u8,
}

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
    convert_impl(library, dxil, stage, None, false, None, false).map(|(data, _, _)| data)
}

/// Convert a vertex shader and obtain its authoritative Metal attribute indices.
/// `library` must be a trusted Metal Shader Converter library.
pub fn convert_vertex_with_reflection(
    library: &Path,
    dxil: &[u8],
) -> Result<(Vec<u8>, Vec<VertexAttribute>), String> {
    let (data, _, attributes) = convert_impl(library, dxil, Stage::Vertex, None, false, None, true)?;
    Ok((data, attributes.ok_or("missing vertex reflection")?))
}

/// Convert a fragment shader after verifying its automatic resource layout.
///
/// When `expected_b0` is true, the only supported resource is the converter's
/// space 0, slot 0 constant buffer at offset 0 with a 24-byte location.
/// Otherwise, the shader must have no fragment resources.
///
/// As with [`convert`], `library` must be a trusted converter library.
pub fn convert_fragment_with_b0(
    library: &Path,
    dxil: &[u8],
    expected_b0: bool,
) -> Result<Vec<u8>, String> {
    convert_impl(library, dxil, Stage::Fragment, Some(expected_b0), false, None, false)
        .map(|(data, _, _)| data)
}

/// Convert a sampled fragment shader using the same checked automatic b0 ABI.
///
/// Directly indexed resource and sampler heaps are not reported as resource
/// locations by the converter; callers must validate their indices separately.
/// Reflection must still report exactly the expected b0 location and no others.
pub fn convert_sampled_fragment(
    library: &Path,
    dxil: &[u8],
    expected_b0: bool,
) -> Result<Vec<u8>, String> {
    convert_impl(library, dxil, Stage::Fragment, Some(expected_b0), true, None, false)
        .map(|(data, _, _)| data)
}

/// Convert a compute shader and return its reflected root-CBV argument offset.
///
/// The only explicit resources may be b0 space0 (at offset zero) and the
/// requested b register in space1. Directly indexed UAV heaps are implicit.
/// `library` must be a trusted Metal Shader Converter library.
pub fn convert_compute_with_root_cbv(
    library: &Path,
    dxil: &[u8],
    root_slot: u8,
) -> Result<(Vec<u8>, usize), String> {
    if root_slot > 63 {
        return Err("root CBV slot must be in 0..64".into());
    }
    let (data, offset, _) = convert_impl(library, dxil, Stage::Compute, None, false, Some(root_slot), false)?;
    Ok((data, offset.ok_or("missing reflected root CBV")?))
}

fn convert_impl(
    library: &Path,
    dxil: &[u8],
    stage: Stage,
    expected_b0: Option<bool>,
    force_texture_array: bool,
    root_cbv: Option<u8>,
    reflect_vertex: bool,
) -> Result<(Vec<u8>, Option<usize>, Option<Vec<VertexAttribute>>), String> {
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
        if force_texture_array {
            // IRCompatibilityFlagForceTextureArray (metal_irconverter.h): keep the
            // descriptor ABI stable across converter 2.x and 3.x.
            let set_flags = api!(
                "IRCompilerSetCompatibilityFlags",
                unsafe extern "C" fn(Pointer, u32)
            );
            set_flags(compiler.ptr, 1 << 8);
        }
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
        let mut cbv_offset = None;
        let mut vertex_attributes = None;
        if expected_b0.is_some() || root_cbv.is_some() || reflect_vertex {
            let reflection_create = api!(
                "IRShaderReflectionCreate",
                unsafe extern "C" fn() -> Pointer
            );
            let reflection_destroy = api!("IRShaderReflectionDestroy", Destroy);
            let reflect = api!(
                "IRObjectGetReflection",
                unsafe extern "C" fn(Pointer, c_int, Pointer) -> bool
            );
            let resource_count = api!(
                "IRShaderReflectionGetResourceCount",
                unsafe extern "C" fn(Pointer) -> usize
            );
            let resource_locations = api!(
                "IRShaderReflectionGetResourceLocations",
                unsafe extern "C" fn(Pointer, *mut ResourceLocation)
            );
            let reflection =
                Handle::new(reflection_create(), reflection_destroy, "shader reflection")?;
            if !reflect(result.ptr, stage.raw(), reflection.ptr) {
                return Err("converter could not reflect shader".into());
            }
            if reflect_vertex {
                let copy = api!("IRShaderReflectionCopyVertexInfo",
                    unsafe extern "C" fn(Pointer, c_int, *mut VertexInfo) -> bool);
                let release = api!("IRShaderReflectionReleaseVertexInfo",
                    unsafe extern "C" fn(*mut VertexInfo) -> bool);
                let mut info: VertexInfo = std::mem::zeroed();
                if !copy(reflection.ptr, 1, &mut info) {
                    return Err("converter could not copy vertex reflection version 1".into());
                }
                let attributes = (|| -> Result<Vec<VertexAttribute>, String> {
                    if info.version != 1 || info.info.num_vertex_inputs > 31
                        || (info.info.num_vertex_inputs != 0 && info.info.vertex_inputs.is_null())
                    {
                        return Err("invalid converter vertex reflection".into());
                    }
                    let inputs = if info.info.num_vertex_inputs == 0 {
                        &[][..]
                    } else {
                        std::slice::from_raw_parts(info.info.vertex_inputs, info.info.num_vertex_inputs)
                    };
                    let mut attributes: Vec<VertexAttribute> = Vec::with_capacity(inputs.len());
                    for input in inputs {
                        if input.name.is_null() || input.attribute_index >= 31 {
                            return Err("invalid reflected vertex attribute".into());
                        }
                        let name = CStr::from_ptr(input.name).to_str()
                            .map_err(|_| "invalid reflected vertex semantic")?;
                        if name.is_empty() || attributes.iter().any(|a| {
                            a.index == input.attribute_index || a.name.eq_ignore_ascii_case(name)
                        }) {
                            return Err("duplicate or empty reflected vertex semantic".into());
                        }
                        attributes.push(VertexAttribute { name: name.into(), index: input.attribute_index });
                    }
                    Ok(attributes)
                })();
                if !release(&mut info) {
                    return Err("converter could not release vertex reflection".into());
                }
                vertex_attributes = Some(attributes?);
            }
            let count = resource_count(reflection.ptr);
            if let Some(expected_b0) = expected_b0 {
                if count != usize::from(expected_b0) {
                    return Err(format!(
                        "unsupported fragment resource layout: expected {} resources, found {count}",
                        usize::from(expected_b0)
                    ));
                }
                if expected_b0 {
                    let mut location = ResourceLocation::default();
                    resource_locations(reflection.ptr, &mut location);
                    if (
                        location.kind,
                        location.space,
                        location.slot,
                        location.offset,
                        location.size,
                    ) != (2, 0, 0, 0, 24)
                    {
                        return Err(format!(
                            "unsupported fragment resource layout: kind {}, space {}, slot {}, offset {}, size {}",
                            location.kind,
                            location.space,
                            location.slot,
                            location.offset,
                            location.size
                        ));
                    }
                }
            } else if let Some(slot) = root_cbv {
                if !(1..=2).contains(&count) {
                    return Err(format!("unsupported compute resource count: {count}"));
                }
                let mut locations = [ResourceLocation::default(), ResourceLocation::default()];
                resource_locations(reflection.ptr, locations.as_mut_ptr());
                let mut b0 = false;
                for location in &locations[..count] {
                    if location.kind != 2 || location.size != 24 || location.offset % 8 != 0 {
                        return Err("unsupported compute resource descriptor".into());
                    }
                    match (location.space, location.slot) {
                        (0, 0) if !b0 && location.offset == 0 => b0 = true,
                        (1, s) if s == u32::from(slot) && cbv_offset.is_none() => {
                            cbv_offset = Some(location.offset as usize);
                        }
                        _ => return Err("unexpected compute resource binding".into()),
                    }
                }
                let offset = cbv_offset.ok_or("missing reflected root CBV")?;
                if b0 && offset < 24 {
                    return Err("overlapping compute resource descriptors".into());
                }
                offset.checked_add(24).ok_or("compute argument offset overflow")?;
            }
        }
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
        Ok((data, cbv_offset, vertex_attributes))
    }
}
