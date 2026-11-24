//! Private bridge to the one XMem state shared with the Wine-facing xmem.dll.

use std::ffi::c_void;
use std::sync::LazyLock;

use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
    GetProcAddress, LoadLibraryW,
};

use victory_gpu::resource::{AllocRoutine, FreeRoutine};

type MapAlias = extern "system" fn(u64, u64, u64) -> u32;
struct Backend {
    alloc: AllocRoutine,
    free: FreeRoutine,
    map_alias: MapAlias,
}

// A successful LoadLibraryW reference is deliberately retained until process
// exit: GPU callbacks and XMem allocations may outlive the originating DLL.
static BACKEND: LazyLock<Result<Backend, String>> = LazyLock::new(load);

fn load() -> Result<Backend, String> {
    let mut module = std::ptr::null_mut();
    // SAFETY: FROM_ADDRESS interprets the supplied value as an address in
    // this DLL; `module` is a valid output, and UNCHANGED_REFCOUNT borrows it.
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            load as *const () as *const u16,
            &mut module,
        )
    };
    if ok == 0 { return Err(format!("locating D3D12.X DLL: {}", std::io::Error::last_os_error())); }
    let mut path = vec![0u16; 32768];
    // SAFETY: module is a live borrowed module; path is writable and sized.
    let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) } as usize;
    if len == 0 || len >= path.len() { return Err(format!("reading D3D12.X DLL path: {}", std::io::Error::last_os_error())); }
    path.truncate(len);
    let end = path.iter().rposition(|c| *c == b'\\' as u16 || *c == b'/' as u16)
        .ok_or_else(|| "D3D12.X DLL path has no directory".to_string())?;
    path.truncate(end + 1);
    path.extend("victory_xmem_backend.dll".encode_utf16());
    path.push(0);
    // SAFETY: path is an absolute, NUL-terminated path beside this DLL.
    let backend = unsafe { LoadLibraryW(path.as_ptr()) };
    if backend.is_null() { return Err(format!("loading private XMem backend beside D3D12.X: {}", std::io::Error::last_os_error())); }
    let mut pinned = std::ptr::null_mut();
    // SAFETY: backend is an address in the loaded module. Pinning keeps all
    // callback pointers valid even if another component calls FreeLibrary.
    if unsafe {
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            backend.cast(), &mut pinned)
    } == 0 {
        return Err(format!("pinning private XMem backend: {}", std::io::Error::last_os_error()));
    }
    let resolve = |name: &'static std::ffi::CStr| {
        // SAFETY: backend is a live module, name is NUL-terminated.
        unsafe { GetProcAddress(backend, name.as_ptr().cast()) }
            .ok_or_else(|| format!("private XMem backend missing {}", name.to_string_lossy()))
    };
    // SAFETY: these private backend exports use the explicit matching ABIs;
    // the module reference is retained for the process lifetime.
    unsafe {
        Ok(Backend {
            alloc: std::mem::transmute(resolve(c"XMemAlloc")?),
            free: std::mem::transmute(resolve(c"XMemFree")?),
            map_alias: std::mem::transmute(resolve(c"VIctoryXMemMapAlias")?),
        })
    }
}

fn map_alias(to: u64, from: u64, len: u64) -> bool {
    match &*BACKEND {
        Ok(backend) => (backend.map_alias)(to, from, len) != 0,
        Err(_) => false,
    }
}

pub(super) fn install() -> Result<(), &'static str> {
    match &*BACKEND {
        Ok(backend) => {
            victory_gpu::resource::use_allocator(backend.alloc, backend.free);
            victory_gpu::resource::use_page_mapper(map_alias);
            Ok(())
        }
        Err(error) => {
            eprintln!("D3D12.X: {error}");
            Err("XMem backend unavailable")
        }
    }
}
