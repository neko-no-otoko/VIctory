//! Wine-facing named exports for the xapu HLE implementation.
//!
//! Unknown names are absent; linker-assigned ordinals are not Xbox ordinals.
//! Non-Windows builds expose no xapu symbols.

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows {
    use std::sync::Once;

    use victory_hle::modules::apu::{self, ApuResult, CommandId, ConnectInput, ConnectOutput, Connection};
    use victory_hle::modules::xgameruntime::HRESULT;
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
    };

    // Connections own decoder state whose code must remain loaded after FreeLibrary.
    fn pin() {
        static PIN: Once = Once::new();
        PIN.call_once(|| {
            let mut module = std::ptr::null_mut();
            // SAFETY: FROM_ADDRESS interprets the function pointer as an address
            // within this module, and PIN retains the module for the process lifetime.
            let success = unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                    pin as *const () as *const u16,
                    &mut module,
                )
            };
            if success == 0 {
                eprintln!("VIctory xapu: cannot pin module: {}", std::io::Error::last_os_error());
                std::process::abort();
            }
        });
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XApuConnect(
        input: *const ConnectInput,
        output: *mut ConnectOutput,
        handle: *mut *mut Connection,
    ) -> HRESULT {
        pin();
        apu::XApuConnect(input, output, handle)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XApuDisconnect(handle: *mut Connection) -> HRESULT {
        pin();
        apu::XApuDisconnect(handle)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XApuEnqueueCommand(
        handle: *mut Connection,
        command: *const CommandId,
        total: *mut u32,
    ) -> HRESULT {
        pin();
        apu::XApuEnqueueCommand(handle, command, total)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XApuDequeueResult(handle: *mut Connection, out: *mut ApuResult) -> HRESULT {
        pin();
        apu::XApuDequeueResult(handle, out)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XApuGetResultCount(handle: *mut Connection, total: *mut u32) -> u32 {
        pin();
        apu::XApuGetResultCount(handle, total)
    }
}
