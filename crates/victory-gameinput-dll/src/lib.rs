//! Normal-Wine GameInput entry point backed by the existing HLE provider.
//!
//! The HLE hub and its polling thread live for the process lifetime. Pin this
//! DLL before the first call that could initialize the hub; no initialization
//! is performed in DllMain.

#[cfg(windows)]
mod windows {
    use std::sync::LazyLock;

    use victory_hle::modules::gameinput::{self, Object};
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
    };

    const E_FAIL: i32 = 0x80004005_u32 as i32;

    fn pin_module() -> bool {
        static PINNED: LazyLock<bool> = LazyLock::new(|| {
            let mut module = std::ptr::null_mut();
            // SAFETY: FROM_ADDRESS interprets the second argument as a code
            // address, not a UTF-16 string; module points to valid output storage.
            unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                    GameInputCreate as *const () as *const u16,
                    &mut module,
                ) != 0
            }
        });
        *PINNED
    }

    // The .def gives this one symbol both its name and PE ordinal 1.
    #[unsafe(no_mangle)]
    pub extern "system" fn GameInputCreate(out: *mut *const Object) -> i32 {
        if !out.is_null() {
            // SAFETY: the caller supplied the output slot required by this ABI.
            unsafe { out.write(std::ptr::null()); }
        }
        // The upstream null-argument path does not touch the hub. Preserve its
        // result without permanently pinning a DLL on an invalid call.
        if out.is_null() {
            return gameinput::GameInputCreate(out);
        }
        if !pin_module() {
            return E_FAIL;
        }
        gameinput::GameInputCreate(out)
    }
}
