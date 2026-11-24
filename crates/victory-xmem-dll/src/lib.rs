//! Wine-facing XMem named exports. Unsupported names are not exported;
//! linker-assigned ordinals are not the console ordinal ABI.
//!
//! All calls go through one pinned backend beside this DLL so that XMem and
//! the VIctory GPU share the backend's physical-page state.

#[cfg(windows)]
mod windows {
    #![allow(non_snake_case)]

    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;
    use std::sync::LazyLock;
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
        GetModuleHandleExW, GetProcAddress, LoadLibraryW,
    };

    // Only pointers cross this boundary; the backend owns the layouts.
    #[repr(C)]
    pub struct BasicInformation { _private: [u8; 0] }
    #[repr(C)]
    pub struct WorkingSet { _private: [u8; 0] }

    fn fail(message: &str) -> ! {
        eprintln!("VIctory XMem: {message}: {}", std::io::Error::last_os_error());
        std::process::abort();
    }

    fn load_backend() -> HMODULE {
        let mut frontend = std::ptr::null_mut();
        // SAFETY: FROM_ADDRESS interprets the name as an address inside this DLL;
        // UNCHANGED_REFCOUNT leaves its existing loader lifetime unchanged.
        if unsafe { GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            load_backend as *const () as *const u16, &mut frontend,
        ) } == 0 { fail("cannot locate frontend module"); }

        let mut buffer = vec![0u16; 512];
        let path = loop {
            // SAFETY: buffer is writable for the supplied length and frontend is live.
            let len = unsafe { GetModuleFileNameW(frontend, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            if len == 0 { fail("cannot get frontend module path"); }
            if len < buffer.len() {
                break PathBuf::from(OsString::from_wide(&buffer[..len]));
            }
            if buffer.len() >= 32768 { fail("frontend module path is too long"); }
            buffer.resize((buffer.len() * 2).min(32768), 0);
        };
        let Some(dir) = path.parent() else { fail("frontend module path has no directory"); };
        let backend_path = dir.join("victory_xmem_backend.dll");
        let wide: Vec<u16> = backend_path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: wide is a null-terminated absolute path, alive for the call.
        let backend = unsafe { LoadLibraryW(wide.as_ptr()) };
        if backend.is_null() { fail("cannot load companion victory_xmem_backend.dll"); }
        let mut pinned = std::ptr::null_mut();
        // SAFETY: the loaded module base is an address within the backend; PIN
        // guarantees that all resolved function pointers remain valid for the process.
        if unsafe { GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            backend as *const u16, &mut pinned,
        ) } == 0 { fail("cannot pin XMem backend"); }
        backend
    }

    macro_rules! exports {
        ($(fn $name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)+) => {
            struct Backend {
                $($name: unsafe extern "system" fn($($ty),*) $(-> $ret)?,)+
            }
            fn backend() -> &'static Backend {
                static BACKEND: LazyLock<Backend> = LazyLock::new(|| {
                    let module = load_backend();
                    Backend {
                        $($name: {
                            // SAFETY: module is pinned and the name is NUL-terminated.
                            let address = unsafe { GetProcAddress(module, concat!(stringify!($name), "\0").as_ptr()) };
                            let Some(address) = address else { fail(concat!("backend missing ", stringify!($name))); };
                            // SAFETY: the companion exports this name with the declared ABI.
                            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, unsafe extern "system" fn($($ty),*) $(-> $ret)?>(address) }
                        },)+
                    }
                });
                &BACKEND
            }
            $(#[unsafe(no_mangle)]
            pub extern "system" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: backend symbols are resolved with matching signatures and pinned.
                unsafe { (backend().$name)($($arg),*) }
            })+
        };
    }

    exports! {
        fn XMemAlloc(size: usize, attributes: u64) -> *mut c_void;
        fn XMemFree(address: *mut c_void, attributes: u64);
        fn XMemAllocDefault(size: usize, attributes: u64) -> *mut c_void;
        fn XMemFreeDefault(address: *mut c_void, attributes: u64);
        fn XMemIsToolsMemAvailable() -> i32;
        fn XMemAllocatePhysicalPages(flags: u64, count: *mut usize, pages: *mut usize) -> i32;
        fn XMemFreePhysicalPages(n: usize, pages: *const usize) -> i32;
        fn XMemMapPhysicalPages(address: *mut c_void, n: usize, pages: *const usize) -> *mut c_void;
        fn XMemMapPhysicalPagesScatter(addresses: *const *mut c_void, n: usize, pages: *const usize) -> i32;
        fn XMemVirtualAlloc(base: *mut c_void, size: usize, kind: u32, flags: u64, protect: u32) -> *mut c_void;
        fn XMemVirtualQuery(process: *mut c_void, address: *const c_void, out: *mut BasicInformation) -> i32;
        fn XMemGetWorkingSetStatistics(which: u32, out: *mut WorkingSet) -> i32;
        fn XMemTransferMemory(kind: u32, size: usize) -> i32;
    }
}
