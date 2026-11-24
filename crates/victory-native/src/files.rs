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
use std::collections::HashMap;
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
const ERROR_NEGATIVE_SEEK: u32 = 131;
const INVALID_HANDLE: usize = usize::MAX;

struct Context {
    root: File,
    save: Option<File>,
    handles: HashMap<usize, GuestFile>,
}

struct GuestFile {
    file: File,
    readable: bool,
    writable: bool,
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
            *slot = Some(Context { root: root_file, save: save_file, handles: HashMap::new() });
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
        "CreateDirectoryW" => create_directory_w as *const () as usize,
        "MoveFileExW" => move_file_ex_w as *const () as usize,
        "DeleteFileW" => delete_file_w as *const () as usize,
        "ReadFile" => read_file as *const () as usize,
        "WriteFile" => write_file as *const () as usize,
        "FlushFileBuffers" => flush_file_buffers as *const () as usize,
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
        context.handles.insert(handle, GuestFile { file, readable, writable });
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
