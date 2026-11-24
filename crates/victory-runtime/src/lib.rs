//! Shared offline system-information semantics and limited native guest API.

/// Offline values used by the existing XSystem implementation.
#[derive(Clone, Copy)]
pub enum SystemString { ConsoleId, SandboxId, DeviceId }

/// Write one NUL-terminated system value using the upstream buffer contract.
///
/// # Safety
/// Non-null pointers must be valid for the supplied sizes.
pub unsafe fn system_string(which: SystemString, size: usize, buffer: *mut u8, used: *mut usize) -> i32 {
    let value = match which {
        SystemString::ConsoleId => "00000000000000000000000000000000000001",
        SystemString::SandboxId => "RETAIL",
        SystemString::DeviceId => "VkljdG9yeSBvZmZsaW5lIGRldmljZSBpZGVudGl0eSE=",
    };
    let len = value.len() + 1;
    if buffer.is_null() || size < len { return 0x8007_007au32 as i32; }
    // SAFETY: caller provides writable buffer and optional count pointer.
    unsafe {
        buffer.copy_from_nonoverlapping(value.as_ptr(), value.len());
        buffer.add(value.len()).write(0);
        if !used.is_null() { used.write(len); }
    }
    0
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod native {
    use super::{SystemString, system_string};
    use std::ffi::c_void;
    use victory_loader::{ImportResolver, Provenance, Resolution};
    use victory_pe::ImportSymbol;
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Guid { a: u32, b: u16, c: u16, d: [u8; 8] }
    const PROVIDER: Guid = Guid { a: 0xe349bd1a, b: 0xfc20, c: 0x4e40, d: [0xb9,0x9c,0x41,0x78,0xcc,0x6b,0x40,0x9f] };
    const IID: Guid = Guid { a: 0x6fd71f09, b: 0x7513, c: 0x49f0, d: [0x89,0xbc,0xbf,0xaf,0x5d,0xf6,0xf8,0x52] };
    const IID_2026: Guid = Guid { a: 0x1861cf2e, b: 0xe18b, c: 0x4834, d: [0xa9,0xf5,0xb4,0xa4,0xe6,0xef,0xb4,0xcf] };
    #[repr(transparent)]
    #[allow(dead_code)] // Read through the guest's ABI, not from Rust.
    struct Table([*const (); 6]);
    // SAFETY: table contains immutable function addresses.
    unsafe impl Sync for Table {}
    #[repr(C)] struct Object { table: &'static Table }
    static OBJECT: Object = Object { table: &TABLE };
    static TABLE: Table = Table([missing as *const (), reference as *const (), reference as *const (), console_id as *const (), sandbox_id as *const (), device_id as *const ()]);
    unsafe extern "win64" fn missing(_: *const Object) -> i32 { 0x8000_4001u32 as i32 }
    unsafe extern "win64" fn reference(_: *const Object) -> u32 { 1 }
    unsafe extern "win64" fn console_id(_: *const Object, n: usize, p: *mut u8, used: *mut usize) -> i32 { unsafe { system_string(SystemString::ConsoleId,n,p,used) } }
    unsafe extern "win64" fn sandbox_id(_: *const Object, n: usize, p: *mut u8, used: *mut usize) -> i32 { unsafe { system_string(SystemString::SandboxId,n,p,used) } }
    unsafe extern "win64" fn device_id(_: *const Object, n: usize, p: *mut u8, used: *mut usize) -> i32 { unsafe { system_string(SystemString::DeviceId,n,p,used) } }
    unsafe extern "win64" fn query(provider: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> i32 {
        // SAFETY: guest supplies valid GUID pointers or null under API contract.
        let (Some(provider), Some(iid)) = (unsafe { provider.as_ref() }, unsafe { iid.as_ref() }) else { return 0x8007_0032u32 as i32 };
        if *provider != PROVIDER || (*iid != IID && *iid != IID_2026) || out.is_null() { return 0x8007_0032u32 as i32; }
        // SAFETY: non-null output is writable per guest API contract.
        unsafe { out.write((&raw const OBJECT).cast_mut().cast()) };
        0
    }
    /// Only the system-information provider is exposed; all other imports fail.
    pub struct NativeResolver;
    impl ImportResolver for NativeResolver {
        fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
            if dll.eq_ignore_ascii_case("xgameruntime.dll") && matches!(symbol, ImportSymbol::Name { name, .. } if name == "QueryApiImpl") {
                Resolution::Found { address: query as *const () as usize, provenance: Provenance::Hle }
            } else { Resolution::Missing }
        }
    }
}
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub use native::NativeResolver;
