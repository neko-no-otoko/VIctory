//! A deliberately limited, process-wide Win32 virtual memory provider.
//!
//! Only mappings created here may be changed. Reservations and their page
//! state stay locked across host VM operations so release cannot race them.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::{LazyLock, Mutex};
use victory_pe::ImportSymbol;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_DECOMMIT: u32 = 0x4000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_NOACCESS: u32 = 1;
const PAGE_READONLY: u32 = 2;
const PAGE_READWRITE: u32 = 4;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_ADDRESS: u32 = 487;
const GRANULARITY: usize = 65536;

struct Reservation {
    size: usize,
    // Zero means reserved but uncommitted; other values are Win32 protections.
    pages: Vec<u32>,
}

static MAPPINGS: LazyLock<Mutex<BTreeMap<usize, Reservation>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
fn mappings() -> &'static Mutex<BTreeMap<usize, Reservation>> { &MAPPINGS }
fn page_size() -> usize {
    static PAGE: LazyLock<usize> = LazyLock::new(|| {
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(size > 0);
        size as usize
    });
    *PAGE
}
fn protection(value: u32) -> Option<i32> {
    match value {
        PAGE_NOACCESS => Some(libc::PROT_NONE),
        PAGE_READONLY => Some(libc::PROT_READ),
        PAGE_READWRITE => Some(libc::PROT_READ | libc::PROT_WRITE),
        _ => None,
    }
}
fn round_up(value: usize, align: usize) -> Option<usize> {
    value.checked_add(align - 1).map(|n| n / align * align)
}
// Round an arbitrary nonempty address range to its containing pages.
fn page_range(address: usize, size: usize, page: usize) -> Option<(usize, usize)> {
    if size == 0 { return None; }
    let end = round_up(address.checked_add(size)?, page)?;
    let start = address / page * page;
    Some((start, end.checked_sub(start)?))
}
fn containing(map: &BTreeMap<usize, Reservation>, start: usize, len: usize) -> Option<usize> {
    let (&base, reservation) = map.range(..=start).next_back()?;
    if start.checked_add(len)? <= base.checked_add(reservation.size)? { Some(base) } else { None }
}
fn error<T>(code: u32, result: T) -> T {
    crate::files::fail(code);
    result
}

pub(crate) fn resolve(dll: &str, symbol: &ImportSymbol) -> Option<usize> {
    if !dll.eq_ignore_ascii_case("kernel32.dll") { return None; }
    let ImportSymbol::Name { name, .. } = symbol else { return None };
    Some(match name.as_str() {
        "VirtualAlloc" => virtual_alloc as *const () as usize,
        "VirtualFree" => virtual_free as *const () as usize,
        "VirtualProtect" => virtual_protect as *const () as usize,
        _ => return None,
    })
}

extern "win64" fn virtual_alloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void {
    let null = ptr::null_mut();
    let Some(host_protect) = protection(protect) else { return error(ERROR_INVALID_PARAMETER, null) };
    if size == 0 || !matches!(kind, MEM_COMMIT | MEM_RESERVE | 0x3000) {
        return error(ERROR_INVALID_PARAMETER, null);
    }
    let page = page_size();
    let mut map = mappings().lock().unwrap();
    if kind & MEM_RESERVE != 0 {
        if !address.is_null() { return error(ERROR_INVALID_ADDRESS, null); }
        let Some(len) = round_up(size, page) else { return error(ERROR_NOT_ENOUGH_MEMORY, null) };
        let Some(total) = len.checked_add(GRANULARITY) else { return error(ERROR_NOT_ENOUGH_MEMORY, null) };
        let mut pages = Vec::new();
        if pages.try_reserve_exact(len / page).is_err() { return error(ERROR_NOT_ENOUGH_MEMORY, null); }
        pages.resize(len / page, 0);
        // Allocate extra space and trim it rather than using MAP_FIXED. All
        // unmaps are confined to the mapping just returned by mmap.
        let raw = unsafe { libc::mmap(ptr::null_mut(), total, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        if raw == libc::MAP_FAILED { return error(ERROR_NOT_ENOUGH_MEMORY, null); }
        let raw_addr = raw as usize;
        let Some(base) = round_up(raw_addr, GRANULARITY) else {
            unsafe { libc::munmap(raw, total); }
            return error(ERROR_NOT_ENOUGH_MEMORY, null);
        };
        let prefix = base - raw_addr;
        let suffix = total - prefix - len;
        unsafe {
            if prefix != 0 { libc::munmap(raw, prefix); }
            if suffix != 0 { libc::munmap((base + len) as *mut c_void, suffix); }
        }
        if kind & MEM_COMMIT != 0 {
            if unsafe { libc::mprotect(base as *mut c_void, len, host_protect) } != 0 {
                unsafe { libc::munmap(base as *mut c_void, len); }
                return error(ERROR_NOT_ENOUGH_MEMORY, null);
            }
            pages.fill(protect);
        }
        map.insert(base, Reservation { size: len, pages });
        return base as *mut c_void;
    }
    if address.is_null() { return error(ERROR_INVALID_ADDRESS, null); }
    let Some((start, len)) = page_range(address as usize, size, page) else { return error(ERROR_INVALID_PARAMETER, null) };
    let Some(base) = containing(&map, start, len) else { return error(ERROR_INVALID_ADDRESS, null) };
    let reservation = map.get_mut(&base).unwrap();
    let first = (start - base) / page;
    let end = first + len / page;
    // Recommitting a committed page is a no-op, including its protection.
    // Zero newly committed pages explicitly: Darwin's advice need not discard
    // contents immediately. Roll back all newly committed runs on failure.
    let mut i = first;
    while i < end {
        if reservation.pages[i] != 0 { i += 1; continue; }
        let begin = i;
        while i < end && reservation.pages[i] == 0 { i += 1; }
        let ptr = (base + begin * page) as *mut c_void;
        let bytes = (i - begin) * page;
        let ok = unsafe {
            if libc::mprotect(ptr, bytes, libc::PROT_READ | libc::PROT_WRITE) != 0 { false }
            else {
                ptr::write_bytes(ptr, 0, bytes);
                libc::mprotect(ptr, bytes, host_protect) == 0
            }
        };
        if !ok {
            for j in first..i {
                if reservation.pages[j] == 0 {
                    unsafe { libc::mprotect((base + j * page) as *mut c_void, page, libc::PROT_NONE); }
                }
            }
            return error(ERROR_NOT_ENOUGH_MEMORY, null);
        }
    }
    for state in &mut reservation.pages[first..end] {
        if *state == 0 { *state = protect; }
    }
    start as *mut c_void
}

extern "win64" fn virtual_free(address: *mut c_void, size: usize, kind: u32) -> i32 {
    if address.is_null() { return error(ERROR_INVALID_ADDRESS, 0); }
    let page = page_size();
    let mut map = mappings().lock().unwrap();
    if kind == MEM_RELEASE {
        if size != 0 { return error(ERROR_INVALID_PARAMETER, 0); }
        let base = address as usize;
        let Some(reservation) = map.get(&base) else { return error(ERROR_INVALID_ADDRESS, 0) };
        if unsafe { libc::munmap(address, reservation.size) } != 0 { return error(ERROR_INVALID_ADDRESS, 0); }
        map.remove(&base);
        return 1;
    }
    if kind != MEM_DECOMMIT { return error(ERROR_INVALID_PARAMETER, 0); }
    let Some((start, len)) = page_range(address as usize, size, page) else { return error(ERROR_INVALID_PARAMETER, 0) };
    let Some(base) = containing(&map, start, len) else { return error(ERROR_INVALID_ADDRESS, 0) };
    if unsafe { libc::mprotect(start as *mut c_void, len, libc::PROT_NONE) } != 0 { return error(ERROR_INVALID_ADDRESS, 0); }
    // Advice is only an optimization; recommit explicitly clears every page.
    unsafe { libc::madvise(start as *mut c_void, len, libc::MADV_DONTNEED); }
    let reservation = map.get_mut(&base).unwrap();
    let first = (start - base) / page;
    reservation.pages[first..first + len / page].fill(0);
    1
}

extern "win64" fn virtual_protect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32 {
    let Some(host_protect) = protection(protect) else { return error(ERROR_INVALID_PARAMETER, 0) };
    if address.is_null() || old.is_null() { return error(ERROR_INVALID_PARAMETER, 0); }
    let page = page_size();
    let Some((start, len)) = page_range(address as usize, size, page) else { return error(ERROR_INVALID_PARAMETER, 0) };
    let mut map = mappings().lock().unwrap();
    let Some(base) = containing(&map, start, len) else { return error(ERROR_INVALID_ADDRESS, 0) };
    let reservation = map.get_mut(&base).unwrap();
    let first = (start - base) / page;
    let pages = &mut reservation.pages[first..first + len / page];
    if pages.contains(&0) { return error(ERROR_INVALID_ADDRESS, 0); }
    if unsafe { libc::mprotect(start as *mut c_void, len, host_protect) } != 0 { return error(ERROR_INVALID_ADDRESS, 0); }
    // The guest must provide a writable old-protection pointer.
    unsafe { *old = pages[0]; }
    pages.fill(protect);
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_share_thread_local_error_without_file_scope() {
        let symbol = |name: &str| ImportSymbol::Name { hint: 0, name: name.into() };
        let get = crate::files::resolve("KERNEL32.DLL", &symbol("GetLastError")).unwrap();
        let set = crate::files::resolve("kernel32.dll", &symbol("SetLastError")).unwrap();
        assert!(crate::files::resolve("kernel32.dll", &symbol("CreateFileW")).is_none());
        assert!(resolve("kernel32.dll", &symbol("VirtualAlloc")).is_some());
        let get: extern "win64" fn() -> u32 = unsafe { std::mem::transmute(get) };
        let set: extern "win64" fn(u32) = unsafe { std::mem::transmute(set) };
        set(1234);
        assert_eq!(get(), 1234);
        assert!(virtual_alloc(ptr::null_mut(), 0, MEM_RESERVE, PAGE_READWRITE).is_null());
        assert_eq!(get(), ERROR_INVALID_PARAMETER);
        std::thread::spawn(move || {
            set(4321);
            assert_eq!(get(), 4321);
        }).join().unwrap();
        assert_eq!(get(), ERROR_INVALID_PARAMETER);
    }
    #[test]
    fn reserve_commit_protect_decommit_and_release() {
        let page = page_size();
        let base = virtual_alloc(ptr::null_mut(), page * 3, MEM_RESERVE, PAGE_READWRITE);
        assert!(!base.is_null());
        assert_eq!(base as usize % GRANULARITY, 0);
        assert_eq!(virtual_alloc(base, page * 2, MEM_COMMIT, PAGE_READWRITE), base);
        unsafe { *(base as *mut u8) = 42; *((base as usize + page) as *mut u8) = 77; }
        let mut old = 0;
        assert_eq!(virtual_protect(base, page, PAGE_READONLY, &mut old), 1);
        assert_eq!(old, PAGE_READWRITE);
        assert_eq!(virtual_alloc(base, page * 2, MEM_COMMIT, PAGE_READWRITE), base);
        assert_eq!(virtual_protect(base, page, PAGE_READWRITE, &mut old), 1);
        assert_eq!(old, PAGE_READONLY);
        assert_eq!(virtual_free(base, page, MEM_DECOMMIT), 1);
        assert_eq!(virtual_alloc(base, page, MEM_COMMIT, PAGE_READONLY), base);
        assert_eq!(unsafe { *(base as *const u8) }, 0);
        assert_eq!(unsafe { *((base as usize + page) as *const u8) }, 77);
        assert_eq!(virtual_protect(base, page * 3, PAGE_READWRITE, &mut old), 0);
        assert_eq!(virtual_free(base, 0, MEM_RELEASE), 1);
        assert_eq!(virtual_free(base, 0, MEM_RELEASE), 0);
    }
    #[test]
    fn rejects_unsupported_and_unowned_ranges() {
        let page = page_size();
        assert!(virtual_alloc(ptr::null_mut(), page, MEM_RESERVE, 0x40).is_null());
        assert!(virtual_alloc(ptr::null_mut(), page, 0x4000, PAGE_READWRITE).is_null());
        let base = virtual_alloc(ptr::null_mut(), page, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        assert!(!base.is_null());
        assert!(virtual_alloc(base, page, MEM_RESERVE, PAGE_READWRITE).is_null());
        assert_eq!(virtual_free(base, page, MEM_RELEASE), 0);
        assert_eq!(virtual_free(base, page * 2, MEM_DECOMMIT), 0);
        let mut old = 0;
        assert_eq!(virtual_protect(base, usize::MAX, PAGE_READONLY, &mut old), 0);
        assert_eq!(virtual_free(base, 0, MEM_RELEASE), 1);
    }
}
