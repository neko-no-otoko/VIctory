//! Opt-in, synchronous Win32 file services for native guests.
//!
//! The scope and its handles belong to the calling thread. Guest-created
//! threads are not supported. This is not a process sandbox. The host must
//! not mutate the root trees concurrently: advisory locks coordinate only
//! cooperating file scopes, not arbitrary host processes.
//! Save handles hold shared advisory file locks; rename and deletion require
//! exclusive locks, including across cooperating processes. Mutations only
//! support single-link regular files, not directories or case-only renames.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::rc::Rc;

use victory_pe::ImportSymbol;

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_SHARING_VIOLATION: u32 = 32;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_NOT_SAME_DEVICE: u32 = 17;
const ERROR_INVALID_DATA: u32 = 13;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_DUP_NAME: u32 = 52;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_NEGATIVE_SEEK: u32 = 131;
const ERROR_INVALID_ADDRESS: u32 = 487;
const ERROR_FILE_INVALID: u32 = 1006;
const ERROR_MAPPED_ALIGNMENT: u32 = 1132;
const INVALID_HANDLE: usize = usize::MAX;

struct Context {
    root: File,
    save: Option<File>,
    handles: HashMap<usize, GuestFile>,
    searches: HashMap<usize, std::vec::IntoIter<FindData>>,
    mappings: HashMap<usize, GuestMapping>,
    views: HashMap<usize, GuestView>,
}

struct GuestFile {
    file: File,
    readable: bool,
    writable: bool,
    save: bool,
}

struct GuestMapping {
    file: File,
    size: u64,
}

struct GuestView {
    base: *mut c_void,
    size: usize,
}

impl Drop for GuestView {
    fn drop(&mut self) {
        // SAFETY: this object exclusively owns the successful mmap.
        unsafe { libc::munmap(self.base, self.size); }
    }
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
        Self::enter_with_save(root, None)
    }

    /// Opens distinct game and optional save roots for this thread.
    pub fn enter_with_save(root: &Path, save: Option<&Path>) -> io::Result<Self> {
        CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a file scope is already active"));
            }
            let open_root = |path: &Path| OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path);
            let root_file = open_root(root)?;
            let save_file = if let Some(save) = save {
                let root_path = std::fs::canonicalize(root)?;
                let save_path = std::fs::canonicalize(save)?;
                let save_file = open_root(save)?;
                let root_metadata = root_file.metadata()?;
                let save_metadata = save_file.metadata()?;
                if root_path.starts_with(&save_path) || save_path.starts_with(&root_path)
                    || (root_metadata.dev() == save_metadata.dev() && root_metadata.ino() == save_metadata.ino())
                {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "game and save roots must be distinct and non-overlapping"));
                }
                Some(save_file)
            } else {
                None
            };
            *slot = Some(Context { root: root_file, save: save_file, handles: HashMap::new(), searches: HashMap::new(), mappings: HashMap::new(), views: HashMap::new() });
            Ok(Self { _thread_bound: PhantomData })
        })
    }
}

impl Drop for FileScope {
    fn drop(&mut self) {
        CONTEXT.with(|slot| { slot.borrow_mut().take(); });
    }
}

/// Resolve last-error exports independently of file scope; file operations
/// remain available only while a scope is active.
pub(crate) fn resolve(dll: &str, symbol: &ImportSymbol) -> Option<usize> {
    if !dll.eq_ignore_ascii_case("kernel32.dll") { return None; }
    let ImportSymbol::Name { name, .. } = symbol else { return None };
    match name.as_str() {
        "GetLastError" => return Some(get_last_error as *const () as usize),
        "SetLastError" => return Some(set_last_error as *const () as usize),
        _ => {}
    }
    if !CONTEXT.with(|slot| slot.borrow().is_some()) { return None; }
    Some(match name.as_str() {
        "CreateFileW" => create_file_w as *const () as usize,
        "CreateDirectoryW" => create_directory_w as *const () as usize,
        "MoveFileExW" => move_file_ex_w as *const () as usize,
        "DeleteFileW" => delete_file_w as *const () as usize,
        "GetFileAttributesExW" => get_file_attributes_ex_w as *const () as usize,
        "FindFirstFileW" => find_first_file_w as *const () as usize,
        "FindNextFileW" => find_next_file_w as *const () as usize,
        "FindClose" => find_close as *const () as usize,
        "ReadFile" => read_file as *const () as usize,
        "WriteFile" => write_file as *const () as usize,
        "FlushFileBuffers" => flush_file_buffers as *const () as usize,
        "SetFilePointerEx" => set_file_pointer_ex as *const () as usize,
        "GetFileSizeEx" => get_file_size_ex as *const () as usize,
        "CreateFileMappingW" => create_file_mapping_w as *const () as usize,
        "MapViewOfFile" => map_view_of_file as *const () as usize,
        "UnmapViewOfFile" => unmap_view_of_file as *const () as usize,
        "CloseHandle" => close_handle as *const () as usize,
        _ => return None,
    })
}

pub(crate) fn fail(code: u32) {
    LAST_ERROR.with(|error| error.set(code));
}

fn error_code(error: &io::Error, intermediate: bool) -> u32 {
    match error.raw_os_error() {
        Some(libc::ENOENT) => if intermediate { ERROR_PATH_NOT_FOUND } else { ERROR_FILE_NOT_FOUND },
        Some(libc::ENOTDIR) => ERROR_PATH_NOT_FOUND,
        Some(libc::EEXIST) => ERROR_FILE_EXISTS,
        Some(libc::EACCES | libc::EPERM | libc::ELOOP) => ERROR_ACCESS_DENIED,
        Some(libc::EXDEV) => ERROR_NOT_SAME_DEVICE,
        Some(libc::ENOTSUP) => ERROR_NOT_SUPPORTED,
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

// Read from an independent directory description: dup() would share its
// seek position with the directory subsequently used for openat().
fn find_component(parent: &File, wanted: &str, intermediate: bool) -> Result<CString, u32> {
    // SAFETY: parent is an open directory and "." is NUL terminated.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 { return Err(error_code(&io::Error::last_os_error(), intermediate)); }
    // SAFETY: fd is owned and fdopendir takes ownership on success.
    let dir = unsafe { libc::fdopendir(fd) };
    if dir.is_null() {
        let error = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(error_code(&error, intermediate));
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) { unsafe { libc::closedir(self.0); } }
    }
    let dir = Directory(dir);
    let mut selected = None;
    loop {
        // SAFETY: errno is thread-local and dir remains live in this loop.
        unsafe { *libc::__error() = 0; }
        let entry = unsafe { libc::readdir(dir.0) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(0) { return Err(error_code(&error, intermediate)); }
            break;
        }
        // SAFETY: readdir supplies a NUL-terminated name until the next call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        consider_component(&mut selected, wanted.as_bytes(), name)?;
    }
    selected.ok_or(if intermediate { ERROR_PATH_NOT_FOUND } else { ERROR_FILE_NOT_FOUND })
}

fn consider_component(selected: &mut Option<CString>, wanted: &[u8], name: &CStr) -> Result<(), u32> {
    if name.to_bytes().eq_ignore_ascii_case(wanted) {
        if selected.is_some() { return Err(ERROR_DUP_NAME); }
        *selected = Some(name.to_owned());
    }
    Ok(())
}
// The save root lock serializes case-folded resolution with mutations and
// with acquiring the lifetime locks on open save files.
struct SaveLock<'a>(&'a File);
impl Drop for SaveLock<'_> {
    fn drop(&mut self) { unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN); } }
}

fn lock_save(root: &File) -> Result<SaveLock<'_>, u32> {
    if unsafe { libc::flock(root.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(error_code(&io::Error::last_os_error(), false));
    }
    Ok(SaveLock(root))
}

fn save_relative(path: &str) -> Result<&str, u32> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].eq_ignore_ascii_case(&b'T') || bytes[1] != b':'
        || !matches!(bytes[2], b'/' | b'\\')
    {
        return Err(ERROR_ACCESS_DENIED);
    }
    Ok(&path[3..])
}

// Resolve every parent as a directory without following links. Return the
// final component separately so callers can safely create or rename it.
fn save_parent(root: &File, path: &str) -> Result<(File, CString), u32> {
    if path.is_empty() { return Err(ERROR_INVALID_NAME); }
    if path.starts_with(['/', '\\']) || path.contains(':') { return Err(ERROR_ACCESS_DENIED); }
    let mut parts = path.split(['/', '\\']).peekable();
    let fd = unsafe { libc::openat(root.as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 { return Err(error_code(&io::Error::last_os_error(), true)); }
    let mut parent = unsafe { File::from_raw_fd(fd) };
    while let Some(part) = parts.next() {
        if part.is_empty() || part == "." || part == ".." { return Err(ERROR_ACCESS_DENIED); }
        if parts.peek().is_none() {
            return Ok((parent, CString::new(part).map_err(|_| ERROR_INVALID_NAME)?));
        }
        let name = find_component(&parent, part, true)?;
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstatat(parent.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } == 0
                && unsafe { stat.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFLNK
            {
                return Err(ERROR_ACCESS_DENIED);
            }
            return Err(error_code(&error, true));
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Err(ERROR_INVALID_NAME)
}

fn selected_name(parent: &File, name: &CStr) -> Result<Option<CString>, u32> {
    let name = name.to_str().map_err(|_| ERROR_INVALID_NAME)?;
    match find_component(parent, name, false) {
        Ok(name) => Ok(Some(name)),
        Err(ERROR_FILE_NOT_FOUND) => Ok(None),
        Err(code) => Err(code),
    }
}

// Only single-link regular files can be removed or replaced. The exclusive
// lock conflicts with every cooperating open save handle until it is closed.
fn lock_mutation_file(parent: &File, name: &CStr) -> Result<File, u32> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstatat(parent.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(error_code(&io::Error::last_os_error(), false));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG || stat.st_nlink != 1 {
        return Err(ERROR_ACCESS_DENIED);
    }
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 { return Err(error_code(&io::Error::last_os_error(), false)); }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|e| error_code(&e, false))?;
    if !metadata.is_file() || metadata.nlink() != 1
        || metadata.dev() != stat.st_dev as u64 || metadata.ino() != stat.st_ino as u64
    {
        return Err(ERROR_ACCESS_DENIED);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        return Err(if matches!(error.raw_os_error(), Some(libc::EWOULDBLOCK)) {
            ERROR_SHARING_VIOLATION
        } else { error_code(&error, false) });
    }
    Ok(file)
}

unsafe extern "win64" fn create_directory_w(path: *const u16, security: *const c_void) -> i32 {
    if !security.is_null() { fail(ERROR_NOT_SUPPORTED); return 0; }
    let result = (|| {
        let path = unsafe { guest_path(path) }?;
        let relative = save_relative(&path)?;
        CONTEXT.with(|slot| {
            let slot = slot.borrow();
            let root = slot.as_ref().ok_or(ERROR_INVALID_HANDLE)?.save.as_ref().ok_or(ERROR_ACCESS_DENIED)?;
            let _lock = lock_save(root)?;
            let (parent, name) = save_parent(root, relative)?;
            if let Some(existing) = selected_name(&parent, &name)? {
                let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe { libc::fstatat(parent.as_raw_fd(), existing.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
                    return Err(error_code(&io::Error::last_os_error(), false));
                }
                if unsafe { stat.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(ERROR_ACCESS_DENIED);
                }
                return Err(ERROR_ALREADY_EXISTS);
            }
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o777) } != 0 {
                let error = io::Error::last_os_error();
                return Err(if error.raw_os_error() == Some(libc::EEXIST) { ERROR_ALREADY_EXISTS } else { error_code(&error, false) });
            }
            Ok(())
        })
    })();
    match result { Ok(()) => 1, Err(code) => { fail(code); 0 } }
}

unsafe extern "win64" fn move_file_ex_w(source: *const u16, destination: *const u16, flags: u32) -> i32 {
    if flags & !1 != 0 { fail(ERROR_NOT_SUPPORTED); return 0; }
    let result = (|| {
        let source = unsafe { guest_path(source) }?;
        let destination = unsafe { guest_path(destination) }?;
        let source = save_relative(&source)?;
        let destination = save_relative(&destination)?;
        CONTEXT.with(|slot| {
            let slot = slot.borrow();
            let root = slot.as_ref().ok_or(ERROR_INVALID_HANDLE)?.save.as_ref().ok_or(ERROR_ACCESS_DENIED)?;
            let _lock = lock_save(root)?;
            let (source_parent, source_name) = save_parent(root, source)?;
            let (dest_parent, dest_name) = save_parent(root, destination)?;
            let source_name = selected_name(&source_parent, &source_name)?.ok_or(ERROR_FILE_NOT_FOUND)?;
            let dest_existing = selected_name(&dest_parent, &dest_name)?;
            let source_file = lock_mutation_file(&source_parent, &source_name)?;
            let _dest_file = if let Some(name) = &dest_existing {
                // Avoid trying to lock the same inode twice.
                let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe { libc::fstatat(dest_parent.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
                    return Err(error_code(&io::Error::last_os_error(), false));
                }
                let stat = unsafe { stat.assume_init() };
                let source_stat = source_file.metadata().map_err(|e| error_code(&e, false))?;
                if source_stat.dev() == stat.st_dev as u64 && source_stat.ino() == stat.st_ino as u64 {
                    return Err(if source_name.as_bytes() == dest_name.as_bytes() {
                        ERROR_ALREADY_EXISTS
                    } else {
                        ERROR_NOT_SUPPORTED
                    });
                }
                Some(lock_mutation_file(&dest_parent, name)?)
            } else { None };
            if dest_existing.is_some() && flags == 0 { return Err(ERROR_ALREADY_EXISTS); }
            let final_name = dest_existing.as_ref().unwrap_or(&dest_name);
            // RENAME_EXCL guarantees no replacement even if a non-cooperating
            // process races the lookup; replacing uses the atomic rename.
            let rename_flags = if flags == 0 { 0x0000_0004 } else { 0 }; // RENAME_EXCL
            let rc = unsafe { renameatx_np(source_parent.as_raw_fd(), source_name.as_ptr(), dest_parent.as_raw_fd(), final_name.as_ptr(), rename_flags) };
            if rc != 0 {
                let error = io::Error::last_os_error();
                return Err(if error.raw_os_error() == Some(libc::EEXIST) { ERROR_ALREADY_EXISTS } else { error_code(&error, false) });
            }
            Ok(())
        })
    })();
    match result { Ok(()) => 1, Err(code) => { fail(code); 0 } }
}

// Declared here because not all libc versions expose Darwin's renameatx_np.
unsafe extern "C" {
    fn renameatx_np(fromfd: libc::c_int, from: *const libc::c_char, tofd: libc::c_int, to: *const libc::c_char, flags: libc::c_uint) -> libc::c_int;
}

unsafe extern "win64" fn delete_file_w(path: *const u16) -> i32 {
    let result = (|| {
        let path = unsafe { guest_path(path) }?;
        let relative = save_relative(&path)?;
        CONTEXT.with(|slot| {
            let slot = slot.borrow();
            let root = slot.as_ref().ok_or(ERROR_INVALID_HANDLE)?.save.as_ref().ok_or(ERROR_ACCESS_DENIED)?;
            let _lock = lock_save(root)?;
            let (parent, name) = save_parent(root, relative)?;
            let name = selected_name(&parent, &name)?.ok_or(ERROR_FILE_NOT_FOUND)?;
            let _file = lock_mutation_file(&parent, &name)?;
            if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(error_code(&io::Error::last_os_error(), false));
            }
            Ok(())
        })
    })();
    match result { Ok(()) => 1, Err(code) => { fail(code); 0 } }
}

fn open_relative(root: &File, path: &str, readable: bool, writable: bool, disposition: u32) -> Result<(File, bool), u32> {
    if path.is_empty() { return Err(ERROR_INVALID_NAME); }
    if path.starts_with(['/', '\\']) || path.contains(':') {
        return Err(ERROR_ACCESS_DENIED);
    }
    let mut parts = path.split(['/', '\\']).peekable();
    let mut directory: Option<File> = None;
    while let Some(part) = parts.next() {
        if part.is_empty() || part == "." || part == ".." {
            return Err(ERROR_ACCESS_DENIED);
        }
        let intermediate = parts.peek().is_some();
        let parent = directory.as_ref().unwrap_or(root);
        let (name, created) = match find_component(parent, part, intermediate) {
            Ok(name) => {
                if !intermediate && disposition == 1 { return Err(ERROR_FILE_EXISTS); }
                (name, false)
            }
            Err(ERROR_FILE_NOT_FOUND) if !intermediate && matches!(disposition, 1 | 4) => {
                (CString::new(part).map_err(|_| ERROR_INVALID_NAME)?, true)
            }
            Err(code) => return Err(code),
        };
        let flags = (if intermediate || !writable { libc::O_RDONLY } else if readable { libc::O_RDWR } else { libc::O_WRONLY })
            | libc::O_NOFOLLOW | libc::O_CLOEXEC
            | if intermediate { libc::O_DIRECTORY } else { libc::O_NONBLOCK }
            | if created { libc::O_CREAT | libc::O_EXCL } else { 0 };
        // SAFETY: parent is an open directory and name is a NUL-terminated component.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o666) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if intermediate && error.raw_os_error() == Some(libc::ENOTDIR) {
                let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                // SAFETY: parent/name are live; lstat does not follow the link.
                let rc = unsafe { libc::fstatat(parent.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
                if rc == 0 && unsafe { stat.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(ERROR_ACCESS_DENIED);
                }
            }
            return Err(error_code(&error, intermediate));
        }
        // SAFETY: openat returned a new, owned file descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if intermediate {
            directory = Some(file);
        } else {
            let metadata = file.metadata().map_err(|e| error_code(&e, false))?;
            if !metadata.is_file() || (writable && metadata.nlink() != 1) { return Err(ERROR_ACCESS_DENIED); }
            return Ok((file, created));
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
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const READ_RIGHTS: u32 = 0x0012_0089;
    let readable = access & (GENERIC_READ | 1) != 0;
    let writable = access & GENERIC_WRITE != 0;
    if access & !(GENERIC_READ | GENERIC_WRITE | READ_RIGHTS) != 0 || !(readable || writable)
        || share & !7 != 0 || share & 1 == 0 || !security.is_null()
        || !matches!(disposition, 1 | 3 | 4) || (disposition != 3 && !writable)
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
        let bytes = path.as_bytes();
        let (drive, relative) = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            if bytes.get(2).is_none_or(|b| *b != b'/' && *b != b'\\') {
                return Err(ERROR_ACCESS_DENIED);
            }
            (Some(bytes[0].to_ascii_uppercase()), &path[3..])
        } else { (None, path.as_str()) };
        let save = drive == Some(b'T') && context.save.is_some();
        if writable && !save { return Err(ERROR_NOT_SUPPORTED); }
        if save && share != 3 { return Err(ERROR_NOT_SUPPORTED); }
        let root = if save { context.save.as_ref().unwrap() } else { &context.root };
        let _lock = if save { Some(lock_save(root)?) } else { None };
        let (file, created) = open_relative(root, relative, readable, writable, disposition)?;
        if save && unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                ERROR_SHARING_VIOLATION
            } else { error_code(&error, false) });
        }
        let handle = NEXT_HANDLE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value != INVALID_HANDLE).then(|| value + 1)
        }).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        context.handles.insert(handle, GuestFile { file, readable, writable, save });
        Ok((handle, created))
    });
    match result {
        Ok((handle, created)) => {
            if disposition == 4 { fail(if created { 0 } else { ERROR_ALREADY_EXISTS }); }
            handle
        }
        Err(code) => { fail(code); INVALID_HANDLE }
    }
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
        if !file.readable { return Err(ERROR_ACCESS_DENIED); }
        // SAFETY: guest supplies a writable buffer of count bytes.
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, count as usize) };
        file.file.read(buffer).map_err(|e| error_code(&e, false))
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

unsafe extern "win64" fn write_file(
    handle: usize, buffer: *const u8, count: u32, bytes_written: *mut u32, overlapped: *mut c_void,
) -> i32 {
    if !bytes_written.is_null() {
        // SAFETY: a non-null output pointer must be writable by the guest.
        unsafe { *bytes_written = 0; }
    }
    if !overlapped.is_null() { fail(ERROR_NOT_SUPPORTED); return 0; }
    if buffer.is_null() || bytes_written.is_null() { fail(ERROR_INVALID_PARAMETER); return 0; }
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let file = slot.as_mut().and_then(|c| c.handles.get_mut(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        if !file.writable { return Err(ERROR_ACCESS_DENIED); }
        let metadata = file.file.metadata().map_err(|e| error_code(&e, false))?;
        if !metadata.is_file() || metadata.nlink() != 1 { return Err(ERROR_ACCESS_DENIED); }
        // SAFETY: the guest supplies a readable buffer of count bytes.
        let buffer = unsafe { std::slice::from_raw_parts(buffer, count as usize) };
        file.file.write(buffer).map_err(|e| error_code(&e, false))
    });
    match result {
        Ok(n) => {
            unsafe { *bytes_written = n as u32; }
            1
        }
        Err(code) => { fail(code); 0 }
    }
}

extern "win64" fn flush_file_buffers(handle: usize) -> i32 {
    let result = CONTEXT.with(|slot| {
        let slot = slot.borrow();
        let file = slot.as_ref().and_then(|c| c.handles.get(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        if !file.writable { return Err(ERROR_ACCESS_DENIED); }
        file.file.sync_all().map_err(|e| error_code(&e, false))
    });
    match result { Ok(()) => 1, Err(code) => { fail(code); 0 } }
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
        file.file.seek(seek).map_err(|e| {
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
        file.file.metadata().map(|m| m.len()).map_err(|e| error_code(&e, false))
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
    let closed = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().is_some_and(|c| c.handles.remove(&handle).is_some() || c.mappings.remove(&handle).is_some())
    });
    if closed { 1 } else { fail(ERROR_INVALID_HANDLE); 0 }
}

unsafe extern "win64" fn create_file_mapping_w(
    handle: usize, security: *const c_void, protection: u32, size_high: u32, size_low: u32, name: *const u16,
) -> usize {
    if !security.is_null() || !name.is_null() || protection != 2 {
        fail(ERROR_NOT_SUPPORTED);
        return 0;
    }
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut().ok_or(ERROR_INVALID_HANDLE)?;
        let file = context.handles.get(&handle).ok_or(ERROR_INVALID_HANDLE)?;
        if !file.readable || file.save { return Err(ERROR_ACCESS_DENIED); }
        let length = file.file.metadata().map_err(|e| error_code(&e, false))?.len();
        let requested = (u64::from(size_high) << 32) | u64::from(size_low);
        let size = if requested == 0 { length } else { requested };
        if size == 0 { return Err(ERROR_FILE_INVALID); }
        if size > length { return Err(ERROR_ACCESS_DENIED); }
        let file = file.file.try_clone().map_err(|e| error_code(&e, false))?;
        let mapping = NEXT_HANDLE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value != INVALID_HANDLE).then(|| value + 1)
        }).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        context.mappings.insert(mapping, GuestMapping { file, size });
        Ok(mapping)
    });
    match result {
        Ok(handle) => handle,
        Err(code) => { fail(code); 0 }
    }
}

unsafe extern "win64" fn map_view_of_file(
    handle: usize, access: u32, offset_high: u32, offset_low: u32, size: usize,
) -> *mut c_void {
    if access != 4 { fail(ERROR_NOT_SUPPORTED); return std::ptr::null_mut(); }
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut().ok_or(ERROR_INVALID_HANDLE)?;
        let mapping = context.mappings.get(&handle).ok_or(ERROR_INVALID_HANDLE)?;
        let offset = (u64::from(offset_high) << 32) | u64::from(offset_low);
        if offset & 0xffff != 0 { return Err(ERROR_MAPPED_ALIGNMENT); }
        if offset >= mapping.size { return Err(ERROR_ACCESS_DENIED); }
        let remaining = mapping.size - offset;
        let length = if size == 0 { usize::try_from(remaining).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)? } else { size };
        if u64::try_from(length).map_err(|_| ERROR_INVALID_PARAMETER)? > remaining {
            return Err(ERROR_ACCESS_DENIED);
        }
        let offset = libc::off_t::try_from(offset).map_err(|_| ERROR_INVALID_PARAMETER)?;
        // SAFETY: the descriptor is open and the checked range is within the mapping.
        let base = unsafe { libc::mmap(std::ptr::null_mut(), length, libc::PROT_READ, libc::MAP_PRIVATE, mapping.file.as_raw_fd(), offset) };
        if base == libc::MAP_FAILED { return Err(error_code(&io::Error::last_os_error(), false)); }
        context.views.insert(base as usize, GuestView { base, size: length });
        Ok(base)
    });
    match result {
        Ok(base) => base,
        Err(code) => { fail(code); std::ptr::null_mut() }
    }
}

unsafe extern "win64" fn unmap_view_of_file(base: *const c_void) -> i32 {
    let removed = CONTEXT.with(|slot| {
        slot.borrow_mut().as_mut().and_then(|c| c.views.remove(&(base as usize))).is_some()
    });
    if removed { 1 } else { fail(ERROR_INVALID_ADDRESS); 0 }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime { low: u32, high: u32 }

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AttributeData {
    attributes: u32,
    creation: FileTime,
    access: FileTime,
    write: FileTime,
    size_high: u32,
    size_low: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FindData {
    attributes: u32,
    creation: FileTime,
    access: FileTime,
    write: FileTime,
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alternate_name: [u16; 14],
}

fn file_time(seconds: i64, nanos: i64) -> Result<FileTime, u32> {
    if !(0..1_000_000_000).contains(&nanos) { return Err(ERROR_INVALID_DATA); }
    let ticks = (i128::from(seconds) + 11_644_473_600) * 10_000_000 + i128::from(nanos / 100);
    let ticks = u64::try_from(ticks).map_err(|_| ERROR_INVALID_DATA)?;
    Ok(FileTime { low: ticks as u32, high: (ticks >> 32) as u32 })
}

fn attributes(stat: &libc::stat) -> Result<AttributeData, u32> {
    let kind = stat.st_mode & libc::S_IFMT;
    let attributes = match kind {
        libc::S_IFREG => 0x80,
        libc::S_IFDIR => 0x10,
        libc::S_IFLNK => return Err(ERROR_ACCESS_DENIED),
        _ => return Err(ERROR_NOT_SUPPORTED),
    };
    let size = if kind == libc::S_IFDIR { 0 } else {
        u64::try_from(stat.st_size).map_err(|_| ERROR_INVALID_DATA)?
    };
    Ok(AttributeData {
        attributes,
        creation: file_time(stat.st_birthtime, stat.st_birthtime_nsec)?,
        access: file_time(stat.st_atime, stat.st_atime_nsec)?,
        write: file_time(stat.st_mtime, stat.st_mtime_nsec)?,
        size_high: (size >> 32) as u32,
        size_low: size as u32,
    })
}

fn stat_at(parent: &File, name: &CStr) -> Result<libc::stat, u32> {
    let mut stat = std::mem::MaybeUninit::uninit();
    if unsafe { libc::fstatat(parent.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(error_code(&io::Error::last_os_error(), false));
    }
    Ok(unsafe { stat.assume_init() })
}

fn find_record(name: &CStr, stat: &libc::stat) -> Result<FindData, u32> {
    let data = attributes(stat)?;
    let name = name.to_str().map_err(|_| ERROR_INVALID_NAME)?;
    let mut wide = [0; 260];
    let mut length = 0;
    for unit in name.encode_utf16() {
        if length == wide.len() - 1 { return Err(ERROR_INVALID_NAME); }
        wide[length] = unit;
        length += 1;
    }
    Ok(FindData {
        attributes: data.attributes, creation: data.creation, access: data.access,
        write: data.write, size_high: data.size_high, size_low: data.size_low,
        reserved: [0; 2], name: wide, alternate_name: [0; 14],
    })
}

// Match CreateFileW's drive aliases, including T: mapping to game without a save root.
fn discovery_root<'a>(context: &'a Context, path: &'a str) -> Result<(&'a File, &'a str, bool), u32> {
    let bytes = path.as_bytes();
    let (drive, relative) = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.get(2).is_none_or(|b| !matches!(b, b'/' | b'\\')) { return Err(ERROR_ACCESS_DENIED); }
        (Some(bytes[0].to_ascii_uppercase()), &path[3..])
    } else { (None, path) };
    let save = drive == Some(b'T') && context.save.is_some();
    Ok((if save { context.save.as_ref().unwrap() } else { &context.root }, relative, save))
}

fn discovery_parent(root: &File, path: &str) -> Result<(File, CString), u32> {
    if path.split(['/', '\\']).any(|part| part.contains(['*', '?'])) { return Err(ERROR_NOT_SUPPORTED); }
    save_parent(root, path)
}

unsafe extern "win64" fn get_file_attributes_ex_w(path: *const u16, level: u32, output: *mut AttributeData) -> i32 {
    if level != 0 { fail(ERROR_NOT_SUPPORTED); return 0; }
    if output.is_null() { fail(ERROR_INVALID_PARAMETER); return 0; }
    let result = unsafe { guest_path(path) }.and_then(|path| CONTEXT.with(|slot| {
        let slot = slot.borrow();
        if path.is_empty() { return Err(ERROR_INVALID_NAME); }
        let context = slot.as_ref().ok_or(ERROR_INVALID_HANDLE)?;
        let (root, relative, save) = discovery_root(context, &path)?;
        let _lock = if save { Some(lock_save(root)?) } else { None };
        let stat = if relative.is_empty() {
            stat_at(root, c".")?
        } else {
            let (parent, name) = discovery_parent(root, relative)?;
            let name = selected_name(&parent, &name)?.ok_or(ERROR_FILE_NOT_FOUND)?;
            stat_at(&parent, &name)?
        };
        attributes(&stat)
    }));
    match result {
        Ok(data) => { unsafe { output.write(data); } 1 }
        Err(code) => { fail(code); 0 }
    }
}

fn collect_matches(root: &File, path: &str) -> Result<Vec<FindData>, u32> {
    if path.is_empty() { return Err(ERROR_INVALID_NAME); }
    if path.starts_with(['/', '\\']) || path.contains(':') { return Err(ERROR_ACCESS_DENIED); }
    if path.split(['/', '\\']).any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(ERROR_ACCESS_DENIED);
    }
    let (directory, pattern) = match path.rsplit_once(['/', '\\']) {
        Some((dir, pattern)) => (dir, pattern),
        None => ("", path),
    };
    if directory.contains(['*', '?']) || pattern.contains('?') { return Err(ERROR_NOT_SUPPORTED); }
    let wildcard = pattern.contains('*');
    let prefix = if pattern == "*" || pattern == "*.*" { "" }
        else if let Some(prefix) = pattern.strip_suffix('*') {
            if prefix.contains('*') || prefix.ends_with('.') { return Err(ERROR_NOT_SUPPORTED); }
            prefix
        } else {
            if wildcard { return Err(ERROR_NOT_SUPPORTED); }
            pattern
        };
    if pattern.is_empty() { return Err(ERROR_INVALID_NAME); }
    let probe = if directory.is_empty() { "__find_probe__".to_owned() } else { format!("{directory}/__find_probe__") };
    let (parent, _) = discovery_parent(root, &probe)?;
    if !wildcard {
        let name = find_component(&parent, pattern, false)?;
        return Ok(vec![find_record(&name, &stat_at(&parent, &name)?)?]);
    }
    let fd = unsafe { libc::openat(parent.as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 { return Err(error_code(&io::Error::last_os_error(), true)); }
    let dir = unsafe { libc::fdopendir(fd) };
    if dir.is_null() {
        let error = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(error_code(&error, true));
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory { fn drop(&mut self) { unsafe { libc::closedir(self.0); } } }
    let dir = Directory(dir);
    let mut records = Vec::new();
    let mut names = HashSet::new();
    loop {
        unsafe { *libc::__error() = 0; }
        let entry = unsafe { libc::readdir(dir.0) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(0) { return Err(error_code(&error, true)); }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." || bytes.len() < prefix.len()
            || !bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()) { continue; }
        let stat = stat_at(&parent, name)?;
        if stat.st_mode & libc::S_IFMT == libc::S_IFLNK { continue; }
        if !names.insert(bytes.to_ascii_lowercase()) { return Err(ERROR_DUP_NAME); }
        records.push(find_record(name, &stat)?);
    }
    if records.is_empty() { return Err(ERROR_FILE_NOT_FOUND); }
    Ok(records)
}

unsafe extern "win64" fn find_first_file_w(path: *const u16, output: *mut FindData) -> usize {
    if output.is_null() { fail(ERROR_INVALID_PARAMETER); return INVALID_HANDLE; }
    let result = unsafe { guest_path(path) }.and_then(|path| CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut().ok_or(ERROR_INVALID_HANDLE)?;
        let (root, relative, save) = discovery_root(context, &path)?;
        let _lock = if save { Some(lock_save(root)?) } else { None };
        let mut records = collect_matches(root, relative)?.into_iter();
        let first = records.next().ok_or(ERROR_FILE_NOT_FOUND)?;
        drop(_lock);
        let handle = NEXT_HANDLE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value != INVALID_HANDLE).then(|| value + 1)
        }).map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        context.searches.insert(handle, records);
        Ok((handle, first))
    }));
    match result {
        Ok((handle, data)) => { unsafe { output.write(data); } handle }
        Err(code) => { fail(code); INVALID_HANDLE }
    }
}

unsafe extern "win64" fn find_next_file_w(handle: usize, output: *mut FindData) -> i32 {
    if output.is_null() { fail(ERROR_INVALID_PARAMETER); return 0; }
    let result = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let search = slot.as_mut().and_then(|c| c.searches.get_mut(&handle)).ok_or(ERROR_INVALID_HANDLE)?;
        search.next().ok_or(ERROR_NO_MORE_FILES)
    });
    match result {
        Ok(data) => { unsafe { output.write(data); } 1 }
        Err(code) => { fail(code); 0 }
    }
}

extern "win64" fn find_close(handle: usize) -> i32 {
    let closed = CONTEXT.with(|slot| slot.borrow_mut().as_mut().and_then(|c| c.searches.remove(&handle)).is_some());
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
    fn read_only_mappings_obey_bounds_and_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        let mut data = vec![0u8; 0x20010];
        data[0x10000] = 42;
        std::fs::write(game.join("data"), &data).unwrap();
        std::fs::write(game.join("empty"), []).unwrap();
        std::fs::write(save.join("data"), b"save").unwrap();
        let scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            let file = open("data");
            let mapping = create_file_mapping_w(file, null(), 2, 0, 0, null());
            assert_ne!(mapping, 0);
            assert_eq!(close_handle(file), 1);
            assert!(map_view_of_file(mapping, 2, 0, 0, 1).is_null());
            assert!(map_view_of_file(mapping, 4, 0, 1, 1).is_null());
            assert_eq!(get_last_error(), ERROR_MAPPED_ALIGNMENT);
            assert!(map_view_of_file(mapping, 4, 0, 0x20000, 17).is_null());
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert!(map_view_of_file(mapping, 4, u32::MAX, 0xffff0000, 0).is_null());
            let view = map_view_of_file(mapping, 4, 0, 0x10000, 0);
            assert!(!view.is_null());
            assert_eq!(*(view as *const u8), 42);
            assert_eq!(close_handle(mapping), 1);
            assert_eq!(*(view as *const u8), 42);
            assert_eq!(unmap_view_of_file((view as *const u8).add(1).cast()), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_ADDRESS);
            assert_eq!(unmap_view_of_file(view), 1);
            assert_eq!(unmap_view_of_file(view), 0);

            let file = open("data");
            assert_eq!(create_file_mapping_w(file, null(), 4, 0, 0, null()), 0);
            assert_eq!(create_file_mapping_w(file, 1usize as *const c_void, 2, 0, 0, null()), 0);
            assert_eq!(create_file_mapping_w(file, null(), 2, 0, 0, wide("name").as_ptr()), 0);
            assert_eq!(create_file_mapping_w(file, null(), 2, 0, 0x20011, null()), 0);
            let limited = create_file_mapping_w(file, null(), 2, 0, 0x10001, null());
            assert_ne!(limited, 0);
            assert!(map_view_of_file(limited, 4, 0, 0x10000, 2).is_null());
            let remaining = map_view_of_file(limited, 4, 0, 0x10000, 0);
            assert!(!remaining.is_null());
            assert_eq!(*(remaining as *const u8), 42);
            assert_eq!(close_handle(limited), 1);
            assert_eq!(close_handle(file), 1);
            let empty = open("empty");
            assert_eq!(create_file_mapping_w(empty, null(), 2, 0, 0, null()), 0);
            assert_eq!(get_last_error(), ERROR_FILE_INVALID);
            assert_eq!(close_handle(empty), 1);
            let save_file = create_file_w(wide("T:\\data").as_ptr(), 0x8000_0000, 3, null(), 3, 0x80, null_mut());
            assert_eq!(create_file_mapping_w(save_file, null(), 2, 0, 0, null()), 0);
            assert_eq!(close_handle(save_file), 1);
            assert_eq!(create_file_mapping_w(INVALID_HANDLE, null(), 2, 0, 0, null()), 0);
            drop(scope);
            assert_eq!(unmap_view_of_file(remaining), 0);
        }
    }

    #[test]
    fn discovery_layout_and_timestamps() {
        assert_eq!(std::mem::size_of::<AttributeData>(), 36);
        assert_eq!(std::mem::size_of::<FindData>(), 592);
        assert_eq!(std::mem::offset_of!(FindData, name), 44);
        assert_eq!(std::mem::offset_of!(FindData, alternate_name), 564);
        let epoch = file_time(0, 123_456_789).unwrap();
        assert_eq!((u64::from(epoch.high) << 32) | u64::from(epoch.low), 116_444_736_001_234_567);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("é😀.dat"), b"12345").unwrap();
        let _scope = FileScope::enter(dir.path()).unwrap();
        unsafe {
            let mut attrs = AttributeData::default();
            set_last_error(1234);
            assert_eq!(get_file_attributes_ex_w(wide("é😀.DAT").as_ptr(), 0, &mut attrs), 1);
            assert_eq!(get_last_error(), 1234);
            assert_eq!((attrs.attributes, attrs.size_high, attrs.size_low), (0x80, 0, 5));
            let metadata = std::fs::metadata(dir.path().join("é😀.dat")).unwrap();
            let expected = file_time(metadata.mtime(), metadata.mtime_nsec()).unwrap();
            assert_eq!((attrs.write.low, attrs.write.high), (expected.low, expected.high));
            let mut record = std::mem::MaybeUninit::<FindData>::uninit();
            let h = find_first_file_w(wide("é😀.dat").as_ptr(), record.as_mut_ptr());
            assert_ne!(h, INVALID_HANDLE);
            let record = record.assume_init();
            assert_eq!(&record.name[..wide("é😀.dat").len()], wide("é😀.dat"));
            assert_eq!(record.alternate_name, [0; 14]);
            assert_eq!(record.size_low, 5);
            assert_eq!(find_close(h), 1);
            assert_eq!(get_file_attributes_ex_w(wide("C:\\").as_ptr(), 0, &mut attrs), 1);
            assert_eq!(attrs.attributes, 0x10);
            assert_eq!((attrs.size_high, attrs.size_low), (0, 0));
            assert_eq!(get_file_attributes_ex_w(wide("é😀.dat").as_ptr(), 1, &mut attrs), 0);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            assert_eq!(get_file_attributes_ex_w(wide("é😀.dat").as_ptr(), 0, null_mut()), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_PARAMETER);
        }
    }

    #[test]
    fn discovery_patterns_handles_roots_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(game.join("Alpha"), b"game").unwrap();
        std::fs::write(game.join("Alpine"), b"data").unwrap();
        std::fs::write(save.join("Alpha"), b"save!").unwrap();
        symlink("Alpha", game.join("Alink")).unwrap();
        let scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            let mut data = std::mem::MaybeUninit::<FindData>::uninit();
            for pattern in ["a?", "a*b", "**", "a*.*", "a.*", "*\\x"] {
                assert_eq!(find_first_file_w(wide(pattern).as_ptr(), data.as_mut_ptr()), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            }
            for pattern in ["Alink", "C:\\Alink"] {
                assert_eq!(find_first_file_w(wide(pattern).as_ptr(), data.as_mut_ptr()), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            }
            let mut attrs = AttributeData::default();
            assert_eq!(get_file_attributes_ex_w(wide("Alink").as_ptr(), 0, &mut attrs), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(get_file_attributes_ex_w(wide("T:\\alpha").as_ptr(), 0, &mut attrs), 1);
            assert_eq!(attrs.size_low, 5);
            set_last_error(456);
            let h = find_first_file_w(wide("aL*").as_ptr(), data.as_mut_ptr());
            assert_ne!(h, INVALID_HANDLE);
            assert_eq!(get_last_error(), 456);
            assert_eq!(close_handle(h), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
            assert_eq!(find_next_file_w(h, data.as_mut_ptr()), 1);
            assert_eq!(find_next_file_w(h, data.as_mut_ptr()), 0);
            assert_eq!(get_last_error(), ERROR_NO_MORE_FILES);
            assert_eq!(find_close(h), 1);
            assert_eq!(find_close(h), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
            let h = find_first_file_w(wide("*.*").as_ptr(), data.as_mut_ptr());
            assert_ne!(h, INVALID_HANDLE);
            drop(scope);
            let _scope = FileScope::enter(&game).unwrap();
            assert_eq!(find_close(h), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
            assert_eq!(get_file_attributes_ex_w(wide("T:\\alpha").as_ptr(), 0, &mut attrs), 1);
            assert_eq!(attrs.size_low, 4);
            assert_eq!(find_first_file_w(wide("missing\\*").as_ptr(), data.as_mut_ptr()), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_PATH_NOT_FOUND);
            assert_eq!(find_first_file_w(wide("absent*").as_ptr(), data.as_mut_ptr()), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_FILE_NOT_FOUND);
            assert_eq!(find_next_file_w(h, data.as_mut_ptr()), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
            let root = OpenOptions::new().read(true).open(&game).unwrap();
            let stat = stat_at(&root, c".").unwrap();
            let too_long = CString::new("x".repeat(260)).unwrap();
            assert!(matches!(find_record(&too_long, &stat), Err(ERROR_INVALID_NAME)));
        }
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
        symlink(dir.path(), dir.path().join("dirlink")).unwrap();
        let _scope = FileScope::enter(dir.path()).unwrap();
        // SAFETY: test pointers refer to live buffers.
        unsafe {
            for path in ["../file", "/file", "C:file", "\\\\server\\file", "G:\\..\\file", "link", "G:\\dirlink\\file"] {
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

    unsafe fn save_open(path: &str, disposition: u32) -> usize {
        unsafe { create_file_w(wide(path).as_ptr(), 0xc000_0000, 3, null(), disposition, 0x80, null_mut()) }
    }

    #[test]
    fn save_roots_must_be_distinct_and_disjoint() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let nested = game.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        symlink(&game, dir.path().join("alias")).unwrap();
        assert!(FileScope::enter_with_save(&game, Some(&game)).is_err());
        assert!(FileScope::enter_with_save(&game, Some(&nested)).is_err());
        assert!(FileScope::enter_with_save(&nested, Some(&game)).is_err());
        assert!(FileScope::enter_with_save(&game, Some(&dir.path().join("alias/nested"))).is_err());
    }

    #[test]
    fn save_io_permissions_and_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(game.join("data"), b"game").unwrap();
        let scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            for path in ["data", "C:\\data"] {
                assert_eq!(save_open(path, 4), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            }
            assert_eq!(save_open("T:\\..\\data", 4), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            let h = save_open("t:\\Save.DAT", 4);
            assert_ne!(h, INVALID_HANDLE);
            assert_eq!(get_last_error(), 0);
            let mut n = 0;
            assert_eq!(write_file(h, b"saved".as_ptr(), 5, &mut n, null_mut()), 1);
            assert_eq!(n, 5);
            assert_eq!(flush_file_buffers(h), 1);
            assert_eq!(save_open("T:\\save.dat", 1), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_FILE_EXISTS);
            let h2 = save_open("T:\\save.dat", 4);
            assert_ne!(h2, INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_ALREADY_EXISTS);
            assert_eq!(close_handle(h2), 1);
            std::fs::hard_link(save.join("Save.DAT"), game.join("linked")).unwrap();
            assert_eq!(write_file(h, b"x".as_ptr(), 1, &mut n, null_mut()), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(save_open("T:\\Save.DAT", 3), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            drop(scope);
            assert_eq!(flush_file_buffers(h), 0);
            assert_eq!(get_last_error(), ERROR_INVALID_HANDLE);
        }
        assert_eq!(std::fs::read(save.join("Save.DAT")).unwrap(), b"saved");
        assert_eq!(std::fs::read(game.join("data")).unwrap(), b"game");
        let _scope = FileScope::enter(&game).unwrap();
        unsafe {
            assert_ne!(open("T:\\data"), INVALID_HANDLE);
            assert_eq!(save_open("T:\\data", 3), INVALID_HANDLE);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
        }
    }

    #[test]
    fn save_create_serializes_case_insensitive_names() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = ["T:\\File", "T:\\file"].into_iter().map(|path| {
            let game = game.clone();
            let save = save.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let _scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
                barrier.wait();
                let h = unsafe { save_open(path, 1) };
                if h == INVALID_HANDLE { assert_eq!(get_last_error(), ERROR_FILE_EXISTS); false } else { true }
            })
        }).collect();
        assert_eq!(threads.into_iter().map(|t| t.join().unwrap()).filter(|success| *success).count(), 1);
        assert_eq!(std::fs::read_dir(save).unwrap().count(), 1);
    }

    #[test]
    fn save_rejects_symlinks_and_unsupported_modes() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(game.join("data"), b"game").unwrap();
        symlink(game.join("data"), save.join("link")).unwrap();
        symlink(&game, save.join("dirlink")).unwrap();
        let _scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            for path in ["T:\\link", "T:\\dirlink\\data"] {
                assert_eq!(save_open(path, 3), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            }
            for (share, disposition, flags) in [(1, 3, 0), (3, 2, 0), (3, 5, 0), (3, 3, 0x4000_0000)] {
                assert_eq!(create_file_w(wide("T:\\link").as_ptr(), 0x4000_0000, share, null(), disposition, flags, null_mut()), INVALID_HANDLE);
                assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            }
            let h = open("data");
            let mut n = 1;
            assert_eq!(write_file(h, b"x".as_ptr(), 1, &mut n, null_mut()), 0);
            assert_eq!(n, 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(flush_file_buffers(h), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
        }
    }

    #[test]
    fn save_directory_move_delete_and_restrictions() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(game.join("game"), b"game").unwrap();
        let _scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            let mkdir = |path: &str| create_directory_w(wide(path).as_ptr(), null());
            let mv = |from: &str, to: &str, flags| move_file_ex_w(wide(from).as_ptr(), wide(to).as_ptr(), flags);
            let delete = |path: &str| delete_file_w(wide(path).as_ptr());
            assert_eq!(mkdir("T:\\Folder"), 1);
            assert_eq!(mkdir("t:\\folder"), 0);
            assert_eq!(get_last_error(), ERROR_ALREADY_EXISTS);
            assert_eq!(mkdir("T:\\missing\\child"), 0);
            assert_eq!(get_last_error(), ERROR_PATH_NOT_FOUND);
            assert_eq!(mkdir("folder"), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(create_directory_w(wide("T:\\other").as_ptr(), 1usize as *const c_void), 0);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            std::fs::write(save.join("Folder/Source"), b"source").unwrap();
            std::fs::write(save.join("Folder/Dest"), b"dest").unwrap();
            assert_eq!(mv("T:\\folder\\source", "T:\\folder\\dest", 0), 0);
            assert_eq!(get_last_error(), ERROR_ALREADY_EXISTS);
            assert_eq!(std::fs::read(save.join("Folder/Dest")).unwrap(), b"dest");
            assert_eq!(mv("T:\\folder\\source", "T:\\folder\\dest", 2), 0);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            assert_eq!(mv("T:\\folder\\source", "C:\\game", 1), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(mv("game", "T:\\folder\\new", 0), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(mv("T:\\folder\\source", "T:\\folder\\dest", 1), 1);
            assert_eq!(std::fs::read(save.join("Folder/Dest")).unwrap(), b"source");
            assert_eq!(mv("T:\\folder\\dest", "T:\\folder\\DEST", 1), 0);
            assert_eq!(get_last_error(), ERROR_NOT_SUPPORTED);
            assert_eq!(mv("T:\\folder\\dest", "T:\\folder\\new", 0), 1);
            assert_eq!(delete("T:\\FOLDER\\NEW"), 1);
            assert_eq!(delete("T:\\folder\\new"), 0);
            assert_eq!(get_last_error(), ERROR_FILE_NOT_FOUND);
            assert_eq!(delete("T:\\folder"), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(delete("C:\\game"), 0);
            assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            assert_eq!(std::fs::read(game.join("game")).unwrap(), b"game");
        }
    }

    #[test]
    fn save_mutations_reject_links_and_open_handles() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(save.join("source"), b"source").unwrap();
        std::fs::write(save.join("dest"), b"dest").unwrap();
        std::fs::write(save.join("linked"), b"linked").unwrap();
        std::fs::hard_link(save.join("linked"), game.join("alias")).unwrap();
        symlink("source", save.join("symlink")).unwrap();
        symlink(&game, save.join("dirlink")).unwrap();
        let _scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            let mv = |from: &str, to: &str| move_file_ex_w(wide(from).as_ptr(), wide(to).as_ptr(), 1);
            let delete = |path: &str| delete_file_w(wide(path).as_ptr());
            for path in ["T:\\symlink", "T:\\linked", "T:\\dirlink\\alias"] {
                assert_eq!(delete(path), 0);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
                assert_eq!(mv(path, "T:\\new"), 0);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
                assert_eq!(mv("T:\\source", path), 0);
                assert_eq!(get_last_error(), ERROR_ACCESS_DENIED);
            }
            let source = save_open("T:\\source", 3);
            assert_ne!(source, INVALID_HANDLE);
            assert_eq!(delete("T:\\source"), 0);
            assert_eq!(get_last_error(), ERROR_SHARING_VIOLATION);
            assert_eq!(mv("T:\\source", "T:\\new"), 0);
            assert_eq!(get_last_error(), ERROR_SHARING_VIOLATION);
            assert_eq!(close_handle(source), 1);
            let dest = save_open("T:\\dest", 3);
            assert_ne!(dest, INVALID_HANDLE);
            assert_eq!(mv("T:\\source", "T:\\dest"), 0);
            assert_eq!(get_last_error(), ERROR_SHARING_VIOLATION);
            assert_eq!(close_handle(dest), 1);
            assert_eq!(mv("T:\\source", "T:\\dest"), 1);
        }
    }

    #[test]
    fn save_mutation_respects_other_scope_handles() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let save = dir.path().join("save");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir(&save).unwrap();
        std::fs::write(save.join("file"), b"data").unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (close_tx, close_rx) = std::sync::mpsc::channel();
        let thread_game = game.clone();
        let thread_save = save.clone();
        let thread = std::thread::spawn(move || {
            let _scope = FileScope::enter_with_save(&thread_game, Some(&thread_save)).unwrap();
            let handle = unsafe { save_open("T:\\file", 3) };
            assert_ne!(handle, INVALID_HANDLE);
            ready_tx.send(()).unwrap();
            close_rx.recv().unwrap();
            assert_eq!(close_handle(handle), 1);
        });
        ready_rx.recv().unwrap();
        let _scope = FileScope::enter_with_save(&game, Some(&save)).unwrap();
        unsafe {
            assert_eq!(delete_file_w(wide("T:\\file").as_ptr()), 0);
            assert_eq!(get_last_error(), ERROR_SHARING_VIOLATION);
            close_tx.send(()).unwrap();
            thread.join().unwrap();
            assert_eq!(delete_file_w(wide("T:\\file").as_ptr()), 1);
        }
    }

    #[test]
    fn rejects_case_collisions_even_for_exact_match() {
        let mut selected = None;
        consider_component(&mut selected, b"File", c"File").unwrap();
        assert_eq!(consider_component(&mut selected, b"File", c"FILE"), Err(ERROR_DUP_NAME));
        let mut selected = None;
        consider_component(&mut selected, "É".as_bytes(), c"é").unwrap();
        assert!(selected.is_none(), "non-ASCII bytes are not case folded");
    }
}
