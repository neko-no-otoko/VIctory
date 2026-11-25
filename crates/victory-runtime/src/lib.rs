//! Shared offline system-information semantics and limited native guest API.
pub mod user;

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
    const USER_PROVIDER: Guid = Guid { a: 0x01acd177, b: 0x91f9, c: 0x4763, d: [0xa3,0x8e,0xcc,0xbb,0x55,0xce,0x32,0xe0] };
    const USER_IID: Guid = Guid { a: 0xeb9bf948, b: 0x18dc, c: 0x4d82, d: [0xbb,0xcc,0x40,0xe0,0xa8,0x09,0xc4,0xc0] };
    const TAG_IID: Guid = Guid { a: 0xcef4fac0, b: 0x7676, c: 0x4a94, d: [0xa1,0x19,0x4c,0x43,0xf9,0xeb,0x5b,0x74] };
    #[repr(transparent)]
    struct UserTable<const N: usize>([*const (); N]);
    unsafe impl<const N: usize> Sync for UserTable<N> {}
    #[repr(C)] struct UserObject<const N: usize> { table: &'static UserTable<N> }
    static USER_OBJECT: UserObject<35> = UserObject { table: &USER_TABLE };
    static TAG_OBJECT: UserObject<4> = UserObject { table: &TAG_TABLE };
    static USER_HANDLE: u8 = 0;
    fn handle() -> *const u8 { &USER_HANDLE }
    unsafe fn put<T>(out: *mut T, value: T) -> i32 {
        if out.is_null() { return 0x8007_0057u32 as i32; }
        unsafe { out.write(value) }; 0
    }
    unsafe extern "win64" fn duplicate(_: *const (), _: *const u8, out: *mut *const u8) -> i32 { unsafe { put(out, handle()) } }
    unsafe extern "win64" fn close(_: *const (), _: *const u8) {}
    unsafe extern "win64" fn compare(_: *const (), a: *const u8, b: *const u8) -> i32 { i32::from(a != b) }
    unsafe extern "win64" fn local_id(_: *const (), _: *const u8, out: *mut u64) -> i32 { unsafe { put(out, super::user::LOCAL_ID) } }
    unsafe extern "win64" fn id(_: *const (), _: *const u8, out: *mut u64) -> i32 { unsafe { put(out, super::user::XUID) } }
    unsafe extern "win64" fn find_local(_: *const (), id: u64, out: *mut *const u8) -> i32 {
        if super::user::find_local_id(id) { unsafe { put(out, handle()) } } else { super::user::USER_NOT_FOUND }
    }
    unsafe extern "win64" fn find_id(_: *const (), id: u64, out: *mut *const u8) -> i32 {
        if super::user::find_id(id) { unsafe { put(out, handle()) } } else { super::user::USER_NOT_FOUND }
    }
    unsafe extern "win64" fn guest(_: *const (), _: *const u8, out: *mut bool) -> i32 { unsafe { put(out, false) } }
    unsafe extern "win64" fn state(_: *const (), _: *const u8, out: *mut u32) -> i32 { unsafe { put(out, 0) } }
    unsafe extern "win64" fn gamertag(_: *const (), _: *const u8, component: u32, size: usize, buffer: *mut u8, used: *mut usize) -> i32 {
        unsafe { super::user::write_gamertag(component, size, buffer, used) }
    }
    unsafe extern "win64" fn unsupported_bool(_: *const ()) -> bool { std::process::abort() }
    static USER_TABLE: UserTable<35> = UserTable([
        missing as *const (), reference as *const (), reference as *const (),
        duplicate as *const (), close as *const (), compare as *const (), missing as *const (),
        missing as *const (), missing as *const (), local_id as *const (), find_local as *const (),
        id as *const (), find_id as *const (), guest as *const (), state as *const (), gamertag as *const (),
        missing as *const (), missing as *const (), missing as *const (), missing as *const (),
        missing as *const (), missing as *const (), missing as *const (), missing as *const (),
        missing as *const (), missing as *const (), missing as *const (), missing as *const (),
        missing as *const (), missing as *const (), missing as *const (), missing as *const (),
        missing as *const (), missing as *const (), unsupported_bool as *const (),
    ]);
    static TAG_TABLE: UserTable<4> = UserTable([missing as *const (), reference as *const (), reference as *const (), gamertag as *const ()]);
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
        if out.is_null() { return 0x8007_0032u32 as i32; }
        let object: *const c_void = if *provider == PROVIDER && (*iid == IID || *iid == IID_2026) {
            (&raw const OBJECT).cast()
        } else if *provider == USER_PROVIDER && (*iid == USER_IID || *iid == USER_PROVIDER) {
            (&raw const USER_OBJECT).cast()
        } else if *provider == USER_PROVIDER && *iid == TAG_IID {
            (&raw const TAG_OBJECT).cast()
        } else { return 0x8007_0032u32 as i32; };
        unsafe { out.write(object.cast_mut()) };
        0
    }
    /// Exposes restricted system-information and offline user providers.
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
