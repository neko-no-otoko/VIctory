//! Opt-in, synchronous, read-only Win32 file services for native guests.
//!
//! The scope and its handles belong to the calling thread. Guest-created
//! threads are not supported. This is not a process sandbox.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::rc::Rc;

use victory_pe::ImportSymbol;

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_INVALID_DATA: u32 = 13;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_NEGATIVE_SEEK: u32 = 131;
const INVALID_HANDLE: usize = usize::MAX;

struct Context {
    root: File,
    handles: HashMap<usize, File>,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
    static LAST_ERROR: Cell<u32> = const { Cell::new(0) };
}

// Never recycle an identifier, even across threads or successive scopes.
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(0x4649_0000_0000_0001);

/// Owns the root and all guest file handles opened on this thread.
///
/// Only one scope may be active on a thread; dropping it closes every handle.
/// The root is opened immediately, so later changes to its pathname do not
/// redirect the scope. The scope cannot be moved to another thread.
pub struct FileScope {
    _thread_bound: PhantomData<Rc<()>>,
}

impl FileScope {
    pub fn enter(root: &Path) -> io::Result<Self> {
        CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a file scope is already active"));
            }
            let root = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(root)?;
            *slot = Some(Context { root, handles: HashMap::new() });
            Ok(Self { _thread_bound: PhantomData })
        })
    }
}

impl Drop for FileScope {
    fn drop(&mut self) {
        CONTEXT.with(|slot| { slot.borrow_mut().take(); });
    }
}

/// Resolve only the supported kernel32 exports, and only within a scope.
pub(crate) fn resolve(dll: &str, symbol: &ImportSymbol) -> Option<usize> {
    if !dll.eq_ignore_ascii_case("kernel32.dll") || !CONTEXT.with(|slot| slot.borrow().is_some()) {
        return None;
    }
    let ImportSymbol::Name { name, .. } = symbol else { return None };
    Some(match name.as_str() {
        "CreateFileW" => create_file_w as *const () as usize,
        "ReadFile" => read_file as *const () as usize,
        "SetFilePointerEx" => set_file_pointer_ex as *const () as usize,
        "GetFileSizeEx" => get_file_size_ex as *const () as usize,
        "CloseHandle" => close_handle as *const () as usize,
        "GetLastError" => get_last_error as *const () as usize,
        "SetLastError" => set_last_error as *const () as usize,
        _ => return None,
    })
}

fn fail(code: u32) {
    LAST_ERROR.with(|error| error.set(code));
}

fn error_code(error: &io::Error, intermediate: bool) -> u32 {
    match error.raw_os_error() {
        Some(libc::ENOENT) => if intermediate { ERROR_PATH_NOT_FOUND } else { ERROR_FILE_NOT_FOUND },
        Some(libc::ENOTDIR) => ERROR_PATH_NOT_FOUND,
        Some(libc::EACCES | libc::EPERM | libc::ELOOP) => ERROR_ACCESS_DENIED,
        Some(libc::ENOMEM | libc::EMFILE | libc::ENFILE) => ERROR_NOT_ENOUGH_MEMORY,
        Some(libc::ENAMETOOLONG) => ERROR_INVALID_NAME,
        Some(libc::EINVAL | libc::EOVERFLOW) => ERROR_INVALID_PARAMETER,
        _ => ERROR_INVALID_DATA,
    }
}

// The native guest is trusted to supply readable, NUL-terminated pointers.
unsafe fn guest_path(path: *const u16) -> Result<String, u32> {
    if path.is_null() { return Err(ERROR_INVALID_PARAMETER); }
    let mut len = 0;
    // Win32 extended paths are at most 32,767 UTF-16 code units.
    while len < 32_767 {
        // SAFETY: the guest supplies a readable NUL-terminated UTF-16 string.
        if unsafe { *path.add(len) } == 0 { break; }
        len += 1;
    }
    if len == 32_767 { return Err(ERROR_INVALID_NAME); }
    // SAFETY: the length was established by reading the guest string above.
    let units = unsafe { std::slice::from_raw_parts(path, len) };
    String::from_utf16(units).map_err(|_| ERROR_INVALID_NAME)
}

fn open_relative(root: &File, path: &str) -> Result<File, u32> {
    if path.is_empty() { return Err(ERROR_INVALID_NAME); }
    // Reject rooted, drive, device and traversal paths rather than normalize
    // them. Both Windows separators are recognized on the Unix host.
    if path.starts_with(['/', '\\']) || path.contains(':') {
        return Err(ERROR_ACCESS_DENIED);
    }
    let mut parts = path.split(['/', '\\']).peekable();
    let mut directory: Option<File> = None;
    while let Some(part) = parts.next() {
        if part.is_empty() || part == "." || part == ".." {
            return Err(ERROR_ACCESS_DENIED);
        }
        let name = CString::new(part).map_err(|_| ERROR_INVALID_NAME)?;
        let intermediate = parts.peek().is_some();
        let parent = directory.as_ref().unwrap_or(root);
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC
            | if intermediate { libc::O_DIRECTORY } else { libc::O_NONBLOCK };
        // SAFETY: parent is an open directory and name is a NUL-terminated component.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 { return Err(error_code(&io::Error::last_os_error(), intermediate)); }
        // SAFETY: openat returned a new, owned file descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if intermediate {
            directory = Some(file);
        } else {
            let metadata = file.metadata().map_err(|e| error_code(&e, false))?;
            if !metadata.is_file() { return Err(ERROR_ACCESS_DENIED); }
            return Ok(file);
        }
    }
    Err(ERROR_INVALID_NAME)
}

unsafe extern "win64" fn create_file_w(
    path: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    disposition: u32,
    flags: u32,
    template: *mut c_void,
) -> usize {
    // Only read access, OPEN_EXISTING and ordinary file attributes are supported.
    const GENERIC_READ: u32 = 0x8000_0000;
    const READ_RIGHTS: u32 = 0x0012_0089;
    // Every supported handle allows other reads. Exclusive sharing would
    // require enforcing Windows share modes across aliases and host processes.
    if access & !(GENERIC_READ | READ_RIGHTS) != 0 || access & (GENERIC_READ | 1) == 0
        || share & !7 != 0 || share & 1 == 0 || !security.is_null() || disposition != 3
        || flags & !0x80 != 0 || !template.is_null()
    {
        fail(ERROR_NOT_SUPPORTED);
        return INVALID_HANDLE;
    }
    // SAFETY: guest pointer validity is part of the native guest contract.
    let path = match unsafe { guest_path(path) } {
        Ok(path) => path,
        Err(code) => { fail(code); return INVALID_HANDLE; }
    };
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut().ok_or(ERROR_INVALID_HANDLE)?;
        let file = open_relative(&context.root, &path)?;
        let handle = NEXT_HANDLE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value != INVALID_HANDLE).then(|| value + 1)
        }).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        context.handles.insert(handle, file);
        Ok(handle)
    });
    match result { Ok(handle) => handle, Err(code) => { fail(code); INVALID_HANDLE } }
}

unsafe extern "win64" fn read_file(
    handle: usize, buffer: *mut u8, count: u32, bytes_read: *mut u32, overlapped: *mut c_void,
) -> i32 {
    if !bytes_read.is_null() {
        // SAFETY: a non-null output pointer must be writable by the guest contract.
        unsafe { *bytes_read = 0; }
    }
    if !overlapped.is_null() { fail(ERROR_NOT_SUPPORTED); return 0; }
    if buffer.is_null() || bytes_read.is_null() { fail(ERROR_INVALID_PARAMETER); return 0; }
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let file = slot.as_mut().and_then(|c| c.handles.get_mut(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        // SAFETY: guest supplies a writable buffer of count bytes.
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, count as usize) };
        file.read(buffer).map_err(|e| error_code(&e, false))
    });
    match result {
        Ok(n) => {
            // SAFETY: checked above; the guest owns the output pointer.
            unsafe { *bytes_read = n as u32; }
            1
        }
        Err(code) => { fail(code); 0 }
    }
}

unsafe extern "win64" fn set_file_pointer_ex(handle: usize, distance: i64, new_position: *mut i64, method: u32) -> i32 {
    let seek = match method {
        0 if distance < 0 => { fail(ERROR_NEGATIVE_SEEK); return 0; }
        0 => SeekFrom::Start(distance as u64),
        1 => SeekFrom::Current(distance),
        2 => SeekFrom::End(distance),
        _ => { fail(ERROR_INVALID_PARAMETER); return 0; }
    };
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let file = slot.as_mut().and_then(|c| c.handles.get_mut(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        file.seek(seek).map_err(|e| {
            if e.raw_os_error() == Some(libc::EINVAL) { ERROR_NEGATIVE_SEEK } else { error_code(&e, false) }
        })
    });
    match result {
        Ok(position) => {
            if !new_position.is_null() {
                // SAFETY: a non-null output pointer must be writable by the guest contract.
                unsafe { *new_position = position as i64; }
            }
            1
        }
        Err(code) => { fail(code); 0 }
    }
}

unsafe extern "win64" fn get_file_size_ex(handle: usize, size: *mut i64) -> i32 {
    if size.is_null() { fail(ERROR_INVALID_PARAMETER); return 0; }
    let result = CONTEXT.with(|slot| {
        let slot = slot.borrow();
        let file = slot.as_ref().and_then(|c| c.handles.get(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        file.metadata().map(|m| m.len()).map_err(|e| error_code(&e, false))
    });
    match result {
        Ok(length) => {
            // SAFETY: the guest supplies a writable size pointer.
            unsafe { *size = length as i64; }
            1
        }
        Err(code) => { fail(code); 0 }
    }
}

extern "win64" fn close_handle(handle: usize) -> i32 {
    let closed = CONTEXT.with(|slot| slot.borrow_mut().as_mut().and_then(|c| c.handles.remove(&handle)).is_some());
    if closed { 1 } else { fail(ERROR_INVALID_HANDLE); 0 }
}

extern "win64" fn get_last_error() -> u32 { LAST_ERROR.with(Cell::get) }
extern "win64" fn set_last_error(code: u32) { LAST_ERROR.with(|error| error.set(code)); }

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::ptr::{null, null_mut};

    fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn open(path: &str) -> usize {
        // SAFETY: the UTF-16 buffer stays alive for the call.
        unsafe { create_file_w(wide(path).as_ptr(), 0x8000_0000, 1, null(), 3, 0x80, null_mut()) }
    }

    #[test]
    fn scoped_reads_seeks_and_handle_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/data"), b"hello").unwrap();
        let scope = FileScope::enter(dir.path()).unwrap();
        assert!(FileScope::enter(dir.path()).is_err());
        set_last_error(42);
        // SAFETY: test pointers refer to live buffers.
        unsafe {
            let handle = open("nested\\data");
            assert_ne!(handle, INVALID_HANDLE);
            assert_eq!(get_last_error(), 42);
            let mut size = 0;
            assert_eq!(get_file_size_ex(handle, &mut size), 1);
            assert_eq!(size, 5);
            let mut buffer = [0; 8];
            let mut read = 0;
            assert_eq!(read_file(handle, buffer.as_mut_ptr(), 3, &mut read, null_mut()), 1);
            assert_eq!((&buffer[..3], read), (&b"hel"[..], 3));
            let mut position = -1;
            assert_eq!(set_file_pointer_ex(handle, -1, &mut position, 2), 1);
            assert_eq!(position, 4);
            assert_eq!(read_file(handle, buffer.as_mut_ptr(), 8, &mut read, null_mut()), 1);
            assert_eq!((buffer[0], read), (b'o', 1));
            assert_eq!(read_file(handle, buffer.as_mut_ptr(), 8, &mut read, null_mut()), 1);
            assert_eq!(read, 0);
            assert_eq!(set_file_pointer_ex(handle, -1, null_mut(), 0), 0);
            assert_eq!(get_last_error(), ERROR_NEGATIVE_SEEK);
            assert_eq!(close_handle(handle), 1);
            assert_eq!(close_handle(handle), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
            let outstanding = open("nested/data");
            drop(scope);
            let _next = FileScope::enter(dir.path()).unwrap();
            assert_eq!(close_handle(outstanding), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
        }
    }

    #[test]
    fn rejects_escapes_symlinks_and_unsupported_io() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file"), b"data").unwrap();
        symlink("file", dir.path().join("link")).unwrap();
        let _scope = FileScope::enter(dir.path()).unwrap();
        // SAFETY: test pointers refer to live buffers.
        unsafe {
            for path in ["../file", "/file", "C:\\file", "link"] {
                assert_eq!(open(path), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            }
            assert_eq!(create_file_w(wide("file").as_ptr(), 0x8000_0000, 1, null(), 2, 0, null_mut()), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            let handle = open("file");
            let mut buffer = [0; 1];
            let mut read = 9;
            assert_eq!(read_file(handle, buffer.as_mut_ptr(), 1, &mut read, 1usize as *mut c_void), 0);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            assert_eq!(read, 0);
            assert_eq!(read_file(handle, null_mut(), 1, &mut read, null_mut()), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_PARAMETER);
        }
    }
}
