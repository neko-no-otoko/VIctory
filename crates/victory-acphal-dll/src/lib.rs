//! Wine-facing named exports for the AcpHal HLE implementation.
//!
//! Unknown names are absent; linker-assigned ordinals are not Xbox ordinals.
//! Non-Windows builds expose no AcpHal symbols. The underlying implementation
//! emulates the ACP command queues and XMA decoder, not the full audio hardware.

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows {
    use std::ffi::c_void;
    use std::sync::Once;

    use victory_hle::modules::audio::{self, Acp, ShapeContexts};
    use victory_hle::modules::xgameruntime::HRESULT;
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
    };

    // The HLE owns process-wide heap and decoder state, and returns an object
    // whose vtable points into this DLL. Neither may outlive its code or statics.
    fn pin() {
        static PIN: Once = Once::new();
        PIN.call_once(|| {
            let mut module = std::ptr::null_mut();
            // SAFETY: FROM_ADDRESS treats this function's address as an address
            // inside the module, not as a string. PIN keeps it loaded for life.
            let success = unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                    pin as *const () as *const u16,
                    &mut module,
                )
            };
            if success == 0 {
                eprintln!("VIctory AcpHal: cannot pin module: {}", std::io::Error::last_os_error());
                std::process::abort();
            }
        });
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn AcpHalAllocateShapeContexts(contexts: *mut ShapeContexts) -> HRESULT {
        pin();
        audio::AcpHalAllocateShapeContexts(contexts)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn AcpHalReleaseShapeContexts() -> HRESULT {
        pin();
        audio::AcpHalReleaseShapeContexts()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn AcpHalCreate(out: *mut *mut Acp) -> HRESULT {
        pin();
        audio::AcpHalCreate(out)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuAlloc(
        out: *mut *mut c_void,
        apu: *mut u32,
        size: u32,
        align: u32,
        flags: u32,
    ) -> HRESULT {
        pin();
        audio::ApuAlloc(out, apu, size, align, flags)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuCreateHeap(cached: u32, non_cached: u32) -> HRESULT {
        pin();
        audio::ApuCreateHeap(cached, non_cached)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuFree(ptr: *mut c_void) {
        pin();
        audio::ApuFree(ptr)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuIsVirtualAddressValid(ptr: *const c_void, align: u32) -> bool {
        pin();
        audio::ApuIsVirtualAddressValid(ptr, align)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuMapApuAddress(apu: u32) -> *mut c_void {
        pin();
        audio::ApuMapApuAddress(apu)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn ApuMapVirtualAddress(ptr: *const c_void) -> u32 {
        pin();
        audio::ApuMapVirtualAddress(ptr)
    }
}
