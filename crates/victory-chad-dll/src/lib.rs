//! Wine-facing named exports for the chad HLE implementation.
//!
//! Unknown names are absent; linker-assigned ordinals are not Xbox ordinals.
//! Non-Windows builds expose no chad symbols.

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows {
    use std::ffi::c_void;
    use std::sync::Once;

    use victory_hle::modules::dsp::{self, Command, Dsp, Status, Stream, StreamDesc};
    use victory_hle::modules::xgameruntime::HRESULT;
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
    };

    // DSP and stream handles own state whose code must remain loaded after FreeLibrary.
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
                eprintln!("VIctory chad: cannot pin module: {}", std::io::Error::last_os_error());
                std::process::abort();
            }
        });
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XDspConnectWithMaximumStreamLimit(
        base: *mut c_void,
        base_length: u32,
        ir_seconds: u32,
        max_streams: u32,
        max_channels: u32,
        out: *mut *mut Dsp,
    ) -> HRESULT {
        pin();
        dsp::XDspConnectWithMaximumStreamLimit(
            base, base_length, ir_seconds, max_streams, max_channels, out,
        )
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XDspDisconnect(dsp: *mut Dsp) -> HRESULT {
        pin();
        dsp::XDspDisconnect(dsp)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XDspActivate(
        dsp: *mut Dsp,
        desc: *const StreamDesc,
        status: *mut *const Status,
        stream: *mut *mut Stream,
    ) -> HRESULT {
        pin();
        dsp::XDspActivate(dsp, desc, status, stream)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XDspDeactivate(stream: *mut Stream) -> HRESULT {
        pin();
        dsp::XDspDeactivate(stream)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XDspSubmitCommand(
        stream: *mut Stream,
        command: *const Command,
        ticket: *mut u32,
    ) -> HRESULT {
        pin();
        dsp::XDspSubmitCommand(stream, command, ticket)
    }
}
