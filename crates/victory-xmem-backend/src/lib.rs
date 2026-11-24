//! Process-shared XMem state for the Wine-facing DLLs.
//!
//! The named XMem exports are internal forwarding targets, not an ordinal ABI.
//! VIctoryXMemMapAlias is a private interface for VIctory's GPU implementation.

#[cfg(windows)]
mod windows {
    #![allow(non_snake_case)]

    use std::ffi::c_void;
    use std::sync::Once;
    use victory_hle::{modules::xmem::{self, BasicInformation, WorkingSet}, runtime};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    fn initialize() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            // SAFETY: a null name requests the process executable's module.
            let image = unsafe { GetModuleHandleW(std::ptr::null()) };
            if image.is_null() {
                eprintln!("VIctory XMem: GetModuleHandleW(NULL) failed");
                std::process::abort();
            }
            runtime::set_main_image(image as usize);
        });
    }

    macro_rules! forward {
        ($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?) => {
            #[unsafe(no_mangle)]
            pub extern "system" fn $name($($arg: $ty),*) $(-> $ret)? {
                initialize();
                xmem::$name($($arg),*)
            }
        };
    }

    forward!(XMemAlloc(size: usize, attributes: u64) -> *mut c_void);
    forward!(XMemFree(address: *mut c_void, attributes: u64));
    forward!(XMemAllocDefault(size: usize, attributes: u64) -> *mut c_void);
    forward!(XMemFreeDefault(address: *mut c_void, attributes: u64));
    forward!(XMemIsToolsMemAvailable() -> i32);
    forward!(XMemAllocatePhysicalPages(flags: u64, count: *mut usize, pages: *mut usize) -> i32);
    forward!(XMemFreePhysicalPages(n: usize, pages: *const usize) -> i32);
    forward!(XMemMapPhysicalPages(address: *mut c_void, n: usize, pages: *const usize) -> *mut c_void);
    forward!(XMemMapPhysicalPagesScatter(addresses: *const *mut c_void, n: usize, pages: *const usize) -> i32);
    forward!(XMemVirtualAlloc(base: *mut c_void, size: usize, kind: u32, flags: u64, protect: u32) -> *mut c_void);
    forward!(XMemVirtualQuery(process: *mut c_void, address: *const c_void, out: *mut BasicInformation) -> i32);
    forward!(XMemGetWorkingSetStatistics(which: u32, out: *mut WorkingSet) -> i32);
    forward!(XMemTransferMemory(kind: u32, size: usize) -> i32);

    #[unsafe(no_mangle)]
    pub extern "system" fn VIctoryXMemMapAlias(to: u64, from: u64, len: u64) -> u32 {
        initialize();
        u32::from(xmem::map_alias(to, from, len))
    }
}
