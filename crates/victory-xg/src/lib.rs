//! Shared XG texture-mode selection and limited native guest resolver.
use victory_formats::texture::Swizzle;

/// Console texture description, with the layout field carrying the swizzle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ResourceDesc {
    pub dimension: u32,
    pub alignment: u64,
    pub width: u64,
    pub height: u32,
    pub depth_or_array: u16,
    pub mips: u16,
    pub format: u32,
    pub samples: u32,
    pub quality: u32,
    pub layout: u32,
    pub flags: u32,
}
const _: () = assert!(size_of::<ResourceDesc>() == 0x38);

/// Upstream's older optimal-mode rule; the newer console API is not fully verified.
pub fn suggested(desc: &ResourceDesc) -> Swizzle {
    if desc.dimension == 2 { Swizzle::LINEAR }
    else if desc.flags & 2 != 0 { Swizzle::SW_64KB_Z_X }
    else if desc.flags & 1 != 0 { Swizzle::SW_64KB_R_X }
    else { Swizzle::SW_4KB_S }
}

/// Apply the shared console-compatible output and error semantics.
///
/// # Safety
/// Non-null pointers must be valid for reading and writing respectively.
pub unsafe fn suggest(desc: *const ResourceDesc, out: *mut u32) -> i32 {
    // SAFETY: guaranteed for non-null pointers by the caller.
    match unsafe { (desc.as_ref(), out.as_mut()) } {
        (Some(desc), Some(out)) => { *out = 0x100 | suggested(desc).0; 0 }
        _ => 0x8007_0057u32 as i32,
    }
}

/// Explicitly limited resolver for a Windows-ABI guest in an x86-64 macOS process.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub struct NativeResolver;

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
unsafe extern "win64" fn xg_suggest(desc: *const ResourceDesc, _: u32, out: *mut u32) -> i32 {
    // SAFETY: guest caller supplies pointers under the API's contract.
    unsafe { suggest(desc, out) }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
impl victory_loader::ImportResolver for NativeResolver {
    fn resolve(&self, dll: &str, symbol: &victory_pe::ImportSymbol) -> victory_loader::Resolution {
        if dll.eq_ignore_ascii_case("xg_xs.dll") && matches!(symbol, victory_pe::ImportSymbol::Name { name, .. } if name == "XGSuggestSwizzleMode") {
            victory_loader::Resolution::Found { address: xg_suggest as *const () as usize, provenance: victory_loader::Provenance::Hle }
        } else { victory_loader::Resolution::Missing }
    }
}
