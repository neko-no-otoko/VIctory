//! Launch a trusted Wine guest with a private Metal bridge host.
//!
//! Wine's usual DLL search order still applies; WINEPATH does not prove which
//! DLL was loaded. Neither the guest nor its providers are sandboxed.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const PROVIDERS: [&str; 6] = ["d3d12_xs.dll", "xg_xs.dll", "xgameruntime.dll", "xmem.dll", "acphal.dll", "xapu.dll"];
const XMEM_BACKEND: &str = "victory_xmem_backend.dll";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
const MAX_OUTPUT: usize = 64 * 1024;
static SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn record_signal(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
}

struct SignalGuard {
    int: libc::sigaction,
    term: libc::sigaction,
    hup: libc::sigaction,
}

impl SignalGuard {
    fn install() -> Result<Self> {
        SIGNAL.store(0, Ordering::Relaxed);
        // SAFETY: sigaction structures are initialized before use. The handler
        // only stores to a lock-free atomic and performs no unsafe operations.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = record_signal as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut int = std::mem::zeroed();
            let mut term = std::mem::zeroed();
            let mut hup = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut int) != 0 {
                return Err(io::Error::last_os_error()).context("installing SIGINT handler");
            }
            if libc::sigaction(libc::SIGTERM, &action, &mut term) != 0 {
                let error = io::Error::last_os_error();
                libc::sigaction(libc::SIGINT, &int, std::ptr::null_mut());
                return Err(error).context("installing SIGTERM handler");
            }
            if libc::sigaction(libc::SIGHUP, &action, &mut hup) != 0 {
                let error = io::Error::last_os_error();
                libc::sigaction(libc::SIGINT, &int, std::ptr::null_mut());
                libc::sigaction(libc::SIGTERM, &term, std::ptr::null_mut());
                return Err(error).context("installing SIGHUP handler");
            }
            Ok(Self { int, term, hup })
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        // SAFETY: both saved dispositions were initialized by successful sigaction calls.
        unsafe {
            libc::sigaction(libc::SIGINT, &self.int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.term, std::ptr::null_mut());
            libc::sigaction(libc::SIGHUP, &self.hup, std::ptr::null_mut());
        }
    }
}

#[derive(Debug)]
struct Interrupted(i32);
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interrupted by signal {}", self.0)
    }
}
impl std::error::Error for Interrupted {}

fn check_signal() -> Result<()> {
    let signal = SIGNAL.load(Ordering::Relaxed);
    if signal != 0 { return Err(Interrupted(signal).into()); }
    Ok(())
}

// Keep an exited leader waitable until its group has been terminated. This
// pins the group ID while signaling, so it cannot refer to a reused PID.
struct ManagedChild {
    child: Child,
    reaped: bool,
}
impl ManagedChild {
    fn spawn(command: &mut Command, description: &str) -> Result<Self> {
        check_signal()?;
        command.process_group(0);
        let child = command.spawn().with_context(|| format!("starting {description}"))?;
        Ok(Self { child, reaped: false })
    }
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if self.reaped { return Ok(None); }
        // SAFETY: siginfo is initialized and waitid writes only to that object.
        // WNOWAIT leaves the child waitable until group teardown is complete.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(libc::P_PID, self.child.id() as libc::id_t, &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) != 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted { return Ok(None); }
                return Err(error).context("observing child process");
            }
            if info.si_pid == 0 { return Ok(None); }
            let raw = match info.si_code {
                libc::CLD_EXITED => info.si_status << 8,
                libc::CLD_KILLED => info.si_status,
                libc::CLD_DUMPED => info.si_status | 0x80,
                code => bail!("unexpected child wait status code {code}"),
            };
            Ok(Some(ExitStatus::from_raw(raw)))
        }
    }
    fn signal_group(&self, signal: i32) {
        // SAFETY: a negative PID addresses only the group created for this child.
        unsafe { libc::kill(-(self.child.id() as i32), signal); }
    }
    fn stop(&mut self, signal: i32) {
        if self.reaped { return; }
        self.signal_group(signal);
        let deadline = Instant::now() + GRACE;
        while Instant::now() < deadline {
            if self.try_wait().ok().flatten().is_some() { break; }
            std::thread::sleep(POLL);
        }
        self.signal_group(libc::SIGKILL);
        let deadline = Instant::now() + GRACE;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => { self.reaped = true; break; }
                Err(_) => break,
                Ok(None) => std::thread::sleep(POLL),
            }
        }
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        let signal = SIGNAL.load(Ordering::Relaxed);
        self.stop(if signal == 0 { libc::SIGTERM } else { signal });
    }
}

fn regular_file(path: &Path, name: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(path).with_context(|| format!("resolving {name} {}", path.display()))?;
    if !fs::metadata(&path).with_context(|| format!("reading {name} metadata"))?.is_file() {
        bail!("{name} is not a regular file: {}", path.display());
    }
    Ok(path)
}
fn executable_file(path: &Path, name: &str) -> Result<PathBuf> {
    // Preserve the supplied basename: Wine's multi-call binary dispatches
    // commands such as winepath according to argv[0].
    let path = if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().context("reading current directory")?.join(path)
    };
    if !fs::metadata(&path).with_context(|| format!("reading {name} metadata"))?.is_file() {
        bail!("{name} is not a regular file: {}", path.display());
    }
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("{name} path contains a NUL byte"))?;
    // SAFETY: c_path is a valid NUL-terminated pathname for the duration of access.
    if unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } != 0 {
        return Err(io::Error::last_os_error())
            .with_context(|| format!("{name} is not executable: {}", path.display()));
    }
    Ok(path)
}
fn directory(path: &Path, name: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(path).with_context(|| format!("resolving {name} {}", path.display()))?;
    if !fs::metadata(&path).with_context(|| format!("reading {name} metadata"))?.is_dir() {
        bail!("{name} is not a directory: {}", path.display());
    }
    Ok(path)
}

fn clean_bridge_env(command: &mut Command) {
    command.env_remove("VICTORY_WINE_METAL_ADDR")
        .env_remove("VICTORY_WINE_METAL_TOKEN")
        .env_remove("VICTORY_WINE_TITLE_DATA")
        .env_remove("VICTORY_WINE_RELAUNCH_ADDR")
        .env_remove("VICTORY_WINE_PLS_DRIVE");
}

struct Relaunch {
    exe: String,
    args: String,
}

// This channel is for a trusted guest, not a same-user security boundary.
// A bounded read prevents a partial frame from indefinitely stalling the
// supervising loop; authentication uses the same private token as the bridge.
fn receive_relaunch(listener: &TcpListener, token: &str) -> Result<Option<Relaunch>> {
    let (mut stream, peer) = match listener.accept() {
        Ok(connection) => connection,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
        Err(e) => return Err(e).context("accepting Wine relaunch request"),
    };
    if !peer.ip().is_loopback() { bail!("non-loopback Wine relaunch request"); }
    // Accepted sockets can inherit nonblocking mode; read_exact needs the
    // bounded blocking mode below rather than a race with the sender.
    stream.set_nonblocking(false).context("setting relaunch connection blocking")?;
    stream.set_read_timeout(Some(GRACE)).context("setting relaunch read timeout")?;
    stream.set_write_timeout(Some(GRACE)).context("setting relaunch write timeout")?;
    let mut header = [0u8; 48];
    stream.read_exact(&mut header).context("reading Wine relaunch header")?;
    if &header[..8] != b"VILNCH01" || &header[8..40] != token.as_bytes() {
        bail!("invalid Wine relaunch authentication or protocol");
    }
    let exe_len = u32::from_le_bytes(header[40..44].try_into().expect("fixed header")) as usize;
    let args_len = u32::from_le_bytes(header[44..48].try_into().expect("fixed header")) as usize;
    if exe_len > 32767 || args_len > 32767 { bail!("Wine relaunch request exceeds size limit"); }
    let mut data = vec![0; exe_len + args_len];
    stream.read_exact(&mut data).context("reading Wine relaunch request")?;
    if data.contains(&0) { bail!("Wine relaunch request contains a NUL byte"); }
    let exe = std::str::from_utf8(&data[..exe_len]).context("invalid Wine relaunch executable encoding")?.to_owned();
    let args = std::str::from_utf8(&data[exe_len..]).context("invalid Wine relaunch arguments encoding")?.to_owned();
    let path = exe.as_bytes();
    if !exe.is_empty() && !(path.len() >= 3 && path[0].is_ascii_alphabetic() &&
        path[1] == b':' && matches!(path[2], b'\\' | b'/')) {
        bail!("Wine relaunch executable must be an absolute drive path");
    }
    stream.write_all(&[1]).context("acknowledging Wine relaunch request")?;
    Ok(Some(Relaunch { exe, args }))
}

fn nonblocking(reader: &impl AsRawFd) -> Result<()> {
    // SAFETY: fcntl acts on a live pipe descriptor and does not retain pointers.
    unsafe {
        let fd = reader.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error()).context("setting child output nonblocking");
        }
    }
    Ok(())
}

// Read bounded output without a reader thread that could remain blocked after
// a descendant inherits the pipe. Observe signals and child exits throughout.
fn read_output(child: &mut ManagedChild, line: bool, description: &str) -> Result<Vec<u8>> {
    let mut stdout = child.child.stdout.take().context("missing child stdout")?;
    nonblocking(&stdout)?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut output = Vec::new();
    let mut exited = None;
    loop {
        check_signal()?;
        let mut buffer = [0; 1024];
        match stdout.read(&mut buffer) {
            Ok(0) => {
                if line { bail!("{description} exited or closed stdout before readiness"); }
                let status = match exited { Some(status) => status, None => loop {
                    check_signal()?;
                    if let Some(status) = child.try_wait()? { break status; }
                    if Instant::now() >= deadline { bail!("timed out waiting for {description}"); }
                    std::thread::sleep(POLL);
                }};
                if !status.success() { bail!("{description} exited with {status}"); }
                return Ok(output);
            }
            Ok(n) => {
                output.extend_from_slice(&buffer[..n]);
                if output.len() > MAX_OUTPUT { bail!("{description} output exceeds limit"); }
                if line && output.contains(&b'\n') {
                    child.child.stdout = Some(stdout);
                    return Ok(output);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e).with_context(|| format!("reading {description} output")),
        }
        if exited.is_none() { exited = child.try_wait()?; }
        if line && exited.is_some() { bail!("{description} exited before readiness"); }
        if Instant::now() >= deadline { bail!("timed out waiting for {description}"); }
        std::thread::sleep(POLL);
    }
}

fn winepath_output(winepath: &Path, prefix: &Path, flag: &str, path: &OsStr) -> Result<Vec<u8>> {
    let mut command = Command::new(winepath);
    command.arg(flag).arg(path).env("WINEPREFIX", prefix).stdout(Stdio::piped());
    clean_bridge_env(&mut command);
    let mut child = ManagedChild::spawn(&mut command, "winepath")?;
    let mut output = read_output(&mut child, false, "winepath")?;
    while matches!(output.last(), Some(b'\r' | b'\n')) { output.pop(); }
    if output.is_empty() || output.iter().any(|b| matches!(b, b'\0' | b'\r' | b'\n'))
        || std::str::from_utf8(&output).is_err() {
        bail!("winepath returned an invalid path");
    }
    // The exited leader can be reaped without signaling the shared Wine server.
    child.child.try_wait().context("reaping winepath")?
        .context("winepath unexpectedly still running")?;
    child.reaped = true;
    Ok(output)
}

fn lock_save_root(path: &Path) -> Result<(PathBuf, File)> {
    let path = directory(path, "save root")?;
    let lock = File::open(&path).with_context(|| format!("opening save root {}", path.display()))?;
    // SAFETY: lock is a live descriptor. The nonblocking advisory lock is held
    // until the launcher has stopped its children and drops this file.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error()).context("locking save root (another launcher may be using it)");
    }
    Ok((path, lock))
}
fn wine_overrides(gameinput: bool) -> Result<OsString> {
    let owned = [b"d3d12_xs".as_slice(), b"xg_xs", b"xgameruntime", b"xmem", b"acphal", b"xapu"];
    let mut result = OsString::from("d3d12_xs,xg_xs,xgameruntime,xmem,acphal,xapu=n");
    if gameinput {
        result.push(";gameinput=n");
    }
    if let Some(existing) = std::env::var_os("WINEDLLOVERRIDES") {
        for entry in existing.as_bytes().split(|b| *b == b';') {
            let Some(eq) = entry.iter().position(|b| *b == b'=') else { continue };
            // Wine separates names on commas, spaces and tabs, and also
            // accepts path-specific and `*name` overrides.
            for name in entry[..eq].split(|b| matches!(*b, b',' | b' ' | b'\t')) {
                let basename = name.rsplit(|b| matches!(*b, b'/' | b'\\')).next().unwrap_or(name);
                let basename = basename.strip_prefix(b"*").unwrap_or(basename);
                let base = if basename.len() >= 4 && basename[basename.len()-4..].eq_ignore_ascii_case(b".dll") {
                    &basename[..basename.len()-4]
                } else { basename };
                if owned.iter().any(|item| base.eq_ignore_ascii_case(item))
                    || (gameinput && base.eq_ignore_ascii_case(b"gameinput")) {
                    let mode = entry[eq+1..].trim_ascii();
                    if !mode.eq_ignore_ascii_case(b"n") && !mode.eq_ignore_ascii_case(b"native") {
                        bail!("conflicting WINEDLLOVERRIDES setting for {}", String::from_utf8_lossy(name));
                    }
                }
            }
        }
        if !existing.is_empty() {
            result.push(";");
            result.push(existing);
        }
    }
    Ok(result)
}

// These are snapshot checks only: Wine performs its own later opens and also
// supports activation contexts, registry overrides and guest-directed loads.
fn reject_shadow_dlls(dir: &Path, selected: &[PathBuf]) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading DLL search directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading DLL search directory {}", dir.display()))?;
        if let Some(index) = PROVIDERS.iter().copied().chain(std::iter::once(XMEM_BACKEND))
            .position(|name| entry.file_name().as_bytes().eq_ignore_ascii_case(name.as_bytes())) {
            let resolved = fs::canonicalize(entry.path())
                .with_context(|| format!("resolving possible provider DLL conflict {}", entry.path().display()))?;
            if resolved != selected[index] {
                bail!("provider DLL conflict in Wine search directory: {}", entry.path().display());
            }
        }
    }
    Ok(())
}

fn optional_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) if path.is_dir() => Ok(true),
        Ok(_) => bail!("Wine search path is not a readable directory: {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("reading Wine search path {}", path.display())),
    }
}

// Wine resolves Windows path components case-insensitively even when the host
// volume does not. Reject ambiguous component spellings rather than guess.
fn windows_child_directory(parent: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    for entry in fs::read_dir(parent).with_context(|| format!("reading Wine directory {}", parent.display()))? {
        let entry = entry.with_context(|| format!("reading Wine directory {}", parent.display()))?;
        if entry.file_name().as_bytes().eq_ignore_ascii_case(name.as_bytes()) {
            if found.is_some() { bail!("ambiguous Wine directory component {name} in {}", parent.display()); }
            let path = entry.path();
            if !path.is_dir() { bail!("Wine search path is not a readable directory: {}", path.display()); }
            found = Some(path);
        }
    }
    Ok(found)
}

fn reject_known_shadows(exe: &Path, dll_dir: &Path, prefix: &Path) -> Result<()> {
    let selected: Vec<_> = PROVIDERS.iter().copied().chain(std::iter::once(XMEM_BACKEND))
        .map(|name| regular_file(&dll_dir.join(name), name)).collect::<Result<_>>()?;
    reject_shadow_dlls(dll_dir, &selected)?;
    reject_shadow_dlls(exe.parent().context("guest executable has no parent directory")?, &selected)?;
    reject_shadow_dlls(&std::env::current_dir().context("reading current directory")?, &selected)?;
    for root in [prefix.join("drive_c"), prefix.join("dosdevices").join("c:")] {
        if !optional_directory(&root)? { continue; }
        let Some(windows) = windows_child_directory(&root, "windows")? else { continue };
        reject_shadow_dlls(&windows, &selected)?;
        for name in ["system32", "system", "syswow64"] {
            if let Some(dir) = windows_child_directory(&windows, name)? {
                reject_shadow_dlls(&dir, &selected)?;
            }
        }
    }
    Ok(())
}

// Verify Wine's existing G: drive without creating a mount or rewriting
// title paths. The guest still uses Wine's ordinary Win32 file APIs.
fn checked_title_root(prefix: &Path, root: &Path) -> Result<PathBuf> {
    let root = directory(root, "title root")?;
    let mapping = prefix.join("dosdevices").join("g:");
    let metadata = fs::symlink_metadata(&mapping)
        .with_context(|| format!("Wine G: mapping is missing: {}", mapping.display()))?;
    if !metadata.file_type().is_symlink() {
        bail!("Wine G: mapping is not a symlink: {}", mapping.display());
    }
    let target = fs::canonicalize(&mapping).context("resolving Wine G: mapping")?;
    if target != root {
        bail!("Wine G: mapping {} does not match title root {}", target.display(), root.display());
    }
    Ok(root)
}

// Wine uses prefix drive links for ordinary Win32 opens. Never create or
// replace them: only accept a dedicated, already configured storage drive.
fn checked_storage_drive(prefix: &Path, save: &Path, letter: char) -> Result<PathBuf> {
    let letter = letter.to_ascii_uppercase();
    if !('D'..='Y').contains(&letter) || matches!(letter, 'G' | 'T') {
        bail!("storage drive must be D through Y, excluding G and T");
    }
    let storage = save.join("storage");
    let metadata = fs::symlink_metadata(&storage).context("reading save-root storage directory")?;
    if !metadata.file_type().is_dir() {
        bail!("save-root storage must be an existing directory, not a symlink");
    }
    let storage = directory(&storage, "persistent storage")?;
    let dosdevices = prefix.join("dosdevices");
    let wanted = format!("{}:", letter.to_ascii_lowercase());
    let mut found = false;
    for entry in fs::read_dir(&dosdevices).context("reading Wine drive mappings")? {
        let entry = entry.context("reading Wine drive mapping")?;
        let name = entry.file_name();
        let bytes = name.as_bytes();
        if bytes.eq_ignore_ascii_case(wanted.as_bytes()) {
            if found || bytes != wanted.as_bytes() {
                bail!("ambiguous or noncanonical Wine storage drive mapping");
            }
            found = true;
            if !fs::symlink_metadata(entry.path())?.file_type().is_symlink() {
                bail!("Wine storage drive mapping must be a symlink");
            }
            if fs::canonicalize(entry.path()).context("resolving Wine storage drive")? != storage {
                bail!("Wine storage drive does not map to save-root storage");
            }
        } else if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            if fs::canonicalize(entry.path()).ok().as_deref() == Some(storage.as_path()) {
                bail!("another Wine drive maps to the storage directory");
            }
        }
    }
    if !found { bail!("Wine storage drive mapping is missing"); }
    Ok(storage)
}

// Wine's system32 GameInput placeholder shadows WINEPATH. Provision only in
// a prefix that was empty at entry; never replace anything in an existing one.
fn gameinput_file(dir: &Path) -> Result<Option<PathBuf>> {
    let mut found = None;
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if entry.file_name().as_bytes().eq_ignore_ascii_case(b"gameinput.dll") {
            if found.is_some() { bail!("ambiguous GameInput DLLs in {}", dir.display()); }
            found = Some(entry.path());
        }
    }
    Ok(found)
}

fn gameinput_system32(prefix: &Path) -> Result<PathBuf> {
    let root = prefix.join("drive_c");
    let drive = fs::canonicalize(&root).context("resolving Wine drive_c")?;
    let mapped = fs::canonicalize(prefix.join("dosdevices/c:")).context("resolving Wine C: mapping")?;
    if drive != mapped || !drive.starts_with(prefix) {
        bail!("Wine C: must map to the selected prefix's drive_c");
    }
    let windows = windows_child_directory(&root, "windows")?.context("Wine prefix has no Windows directory")?;
    let system32 = windows_child_directory(&windows, "system32")?.context("Wine prefix has no system32 directory")?;
    let resolved = fs::canonicalize(&system32)?;
    if !resolved.starts_with(prefix) { bail!("Wine system32 escapes the selected prefix"); }
    Ok(resolved)
}

fn same_contents(left: &Path, right: &Path) -> Result<bool> {
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let size = left.metadata()?.len();
    if size != right.metadata()?.len() { return Ok(false); }
    let mut a = [0u8; 32768];
    let mut b = [0u8; 32768];
    let mut remaining = size;
    while remaining != 0 {
        let count = remaining.min(a.len() as u64) as usize;
        left.read_exact(&mut a[..count])?;
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] { return Ok(false); }
        remaining -= count as u64;
    }
    Ok(true)
}

fn wine_builtin(path: &Path) -> Result<bool> {
    if !fs::symlink_metadata(path)?.file_type().is_file() { return Ok(false); }
    let mut header = [0u8; 80];
    if File::open(path)?.read_exact(&mut header).is_err() { return Ok(false); }
    Ok(&header[..2] == b"MZ" && &header[64..80] == b"Wine builtin DLL")
}

fn reject_gameinput_shadow(dir: &Path) -> Result<()> {
    if let Some(path) = gameinput_file(dir)? {
        bail!("GameInput DLL conflict in Wine search directory: {}", path.display());
    }
    Ok(())
}

fn reject_gameinput_shadows(exe: &Path, prefix: &Path) -> Result<()> {
    reject_gameinput_shadow(exe.parent().context("guest executable has no parent directory")?)?;
    reject_gameinput_shadow(&std::env::current_dir()?)?;
    let root = prefix.join("drive_c");
    if !optional_directory(&root)? { return Ok(()); }
    let Some(windows) = windows_child_directory(&root, "windows")? else { return Ok(()) };
    reject_gameinput_shadow(&windows)?;
    if let Some(dir) = windows_child_directory(&windows, "system")? {
        reject_gameinput_shadow(&dir)?;
    }
    // A fresh Wine 11 prefix also has a 32-bit builtin in syswow64.
    if let Some(dir) = windows_child_directory(&windows, "syswow64")? {
        if let Some(path) = gameinput_file(&dir)? {
            if !wine_builtin(&path)? {
                bail!("GameInput DLL conflict in Wine search directory: {}", path.display());
            }
        }
    }
    Ok(())
}

fn checked_gameinput_installation(prefix: &Path, provider: &Path) -> Result<()> {
    let system32 = gameinput_system32(prefix)?;
    let installed = gameinput_file(&system32)?.context("existing Wine prefix has no GameInput provider")?;
    if !fs::symlink_metadata(&installed)?.file_type().is_file() || !same_contents(&installed, provider)? {
        bail!("existing Wine prefix GameInput differs from the selected provider; refusing to modify it");
    }
    Ok(())
}

fn install_gameinput(prefix: &Path, provider: &Path) -> Result<()> {
    let system32 = gameinput_system32(prefix)?;
    let installed = gameinput_file(&system32)?.context("new Wine prefix has no GameInput placeholder")?;
    if !wine_builtin(&installed)? {
        bail!("new prefix GameInput is not a Wine builtin placeholder");
    }
    let temporary = system32.join(format!(".victory-gameinput-{}.tmp", std::process::id()));
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)
        .context("creating temporary GameInput provider")?;
    let result = (|| -> Result<()> {
        io::copy(&mut File::open(provider)?, &mut output)?;
        output.sync_all()?;
        fs::rename(&temporary, &installed)?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result.context("installing GameInput in dedicated Wine prefix")
}


fn run_inner(exe: &Path, wine: &Path, winepath: &Path, metal_host: &Path, converter: &Path,
    dll_dir: &Path, wine_prefix: &Path, allow_unverified_dll_search: bool, provision_gameinput: bool, guest_dll: &[String], title_root: Option<&Path>, save_root: Option<&Path>, storage_drive: Option<char>, args: &[OsString]) -> Result<i32> {
    let exe = regular_file(exe, "guest executable")?;
    let wine = executable_file(wine, "Wine executable")?;
    let winepath = executable_file(winepath, "winepath executable")?;
    let metal_host = executable_file(metal_host, "Metal host executable")?;
    let converter = regular_file(converter, "Metal converter")?;
    let dll_dir = directory(dll_dir, "DLL directory")?;
    let wine_prefix = directory(wine_prefix, "Wine prefix")?;
    let title_root = title_root.map(|root| checked_title_root(&wine_prefix, root)).transpose()?;
    let save_root = save_root.map(lock_save_root).transpose()?;
    if let (Some(title), Some((save, _))) = (&title_root, &save_root) {
        if title.starts_with(save) || save.starts_with(title) {
            bail!("title root and save root must be separate directories");
        }
    }
    let storage_drive = storage_drive.map(|letter| letter.to_ascii_uppercase());
    let storage = if let Some(letter) = storage_drive {
        let (save, _) = save_root.as_ref().context("storage drive requires a save root")?;
        Some(checked_storage_drive(&wine_prefix, save, letter)?)
    } else { None };
    reject_known_shadows(&exe, &dll_dir, &wine_prefix)?;
    crate::wine_imports::run(&exe, &dll_dir, provision_gameinput, guest_dll)?;
    let overrides = wine_overrides(provision_gameinput)?;
    if !allow_unverified_dll_search {
        bail!("Wine cannot guarantee exact provider DLL identity; use --allow-unverified-dll-search only with a trusted, stable guest, prefix and search paths");
    }
    let (input_provider, fresh_prefix) = if provision_gameinput {
        let provider = regular_file(&dll_dir.join("gameinput.dll"), "GameInput provider")?;
        reject_gameinput_shadows(&exe, &wine_prefix)?;
        let fresh = fs::read_dir(&wine_prefix)?.next().transpose()?.is_none();
        if !fresh { checked_gameinput_installation(&wine_prefix, &provider)?; }
        (Some(provider), fresh)
    } else { (None, false) };
    eprintln!("Warning: Wine provider DLL paths are not pinned; preflight cannot exclude activation contexts, registry overrides, guest-directed loads or file replacement. Trust the guest and prefix.");
    let _signals = SignalGuard::install()?;
    let windows_path = winepath_output(&winepath, &wine_prefix, "-w", dll_dir.as_os_str())?;
    if windows_path.contains(&b';') { bail!("winepath returned an invalid DLL directory"); }
    let windows_save_root = if let Some((root, _)) = &save_root {
        let windows = winepath_output(&winepath, &wine_prefix, "-w", root.as_os_str())?;
        let host = winepath_output(&winepath, &wine_prefix, "-u", OsStr::from_bytes(&windows))?;
        let host = PathBuf::from(OsString::from_vec(host));
        if directory(&host, "Wine save root")? != *root {
            bail!("winepath save-root mapping does not round-trip to the selected directory");
        }
        Some(OsString::from_vec(windows))
    } else { None };
    if let (Some(letter), Some(storage), Some((save, _))) = (storage_drive, &storage, &save_root) {
        checked_storage_drive(&wine_prefix, save, letter)?;
        let drive = format!("{letter}:\\");
        let host = winepath_output(&winepath, &wine_prefix, "-u", OsStr::new(&drive))?;
        if directory(&PathBuf::from(OsString::from_vec(host)), "Wine storage drive")? != *storage {
            bail!("Wine storage drive does not resolve to the selected directory");
        }
    }
    if let Some(root) = &title_root { checked_title_root(&wine_prefix, root)?; }
    if let Some(provider) = &input_provider {
        if fresh_prefix { install_gameinput(&wine_prefix, provider)?; }
        checked_gameinput_installation(&wine_prefix, provider)?;
        reject_gameinput_shadows(&exe, &wine_prefix)?;
    }
    let mut wine_path = OsString::from_vec(windows_path);
    if let Some(old) = std::env::var_os("WINEPATH").filter(|value| !value.is_empty()) {
        wine_path.push(OsStr::new(";")); wine_path.push(old);
    }
    let mut random = [0u8; 16];
    File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut random)).context("generating Metal bridge token")?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut host_command = Command::new(&metal_host);
    host_command.arg("127.0.0.1:0").arg("--announce").stdout(Stdio::piped());
    clean_bridge_env(&mut host_command);
    host_command.env("VICTORY_WINE_METAL_TOKEN", &token)
        .env("VICTORY_METAL_CONVERTER", &converter);
    let mut host = ManagedChild::spawn(&mut host_command, "Metal host")?;
    let ready = read_output(&mut host, true, "Metal host")?;
    let line = ready.split(|b| *b == b'\n').next().context("missing Metal host readiness line")?;
    let address = line.strip_prefix(b"VICTORY_WINE_METAL_READY=").context("invalid Metal host readiness line")?;
    let address: SocketAddrV4 = std::str::from_utf8(address).context("invalid Metal host address")?.parse().context("invalid Metal host address")?;
    if *address.ip() != Ipv4Addr::LOCALHOST || address.port() == 0 { bail!("Metal host announced a non-loopback or zero address"); }
    if let Some(status) = host.try_wait()? { bail!("Metal host exited after readiness: {status}"); }
    if let Some(root) = &title_root {
        checked_title_root(&wine_prefix, root)?;
        eprintln!("Wine G: currently maps to {}; the mapping and guest file access are not sandboxed.", root.display());
    }
    if let (Some(letter), Some((save, _))) = (storage_drive, &save_root) {
        checked_storage_drive(&wine_prefix, save, letter)?;
    }
    let relaunch = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("binding Wine relaunch channel")?;
    relaunch.set_nonblocking(true).context("setting Wine relaunch channel nonblocking")?;
    let relaunch_address = relaunch.local_addr().context("reading Wine relaunch address")?.to_string();
    let wine_command = |exe: &OsStr, args: &[OsString]| {
        let mut command = Command::new(&wine);
        command.arg(exe).args(args).env("WINEPREFIX", &wine_prefix)
            .env("WINEPATH", &wine_path).env("WINEDLLOVERRIDES", &overrides);
        clean_bridge_env(&mut command);
        command.env("VICTORY_WINE_METAL_ADDR", address.to_string())
            .env("VICTORY_WINE_METAL_TOKEN", &token)
            .env("VICTORY_WINE_RELAUNCH_ADDR", &relaunch_address);
        if let Some(root) = &windows_save_root { command.env("VICTORY_WINE_TITLE_DATA", root); }
        if let Some(letter) = storage_drive { command.env("VICTORY_WINE_PLS_DRIVE", letter.to_string()); }
        command
    };
    let mut launch_path = exe.as_os_str().to_owned();
    let mut current_exe = exe;
    let mut guest = ManagedChild::spawn(&mut wine_command(&launch_path, args), "Wine")?;
    let mut pending = None;
    loop {
        check_signal()?;
        // Bound each drain so a noisy host cannot starve status and signal checks.
        if let Some(stdout) = host.child.stdout.as_mut() {
            let mut buffer = [0; 4096];
            for _ in 0..16 {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => break,
                    Err(e) => return Err(e).context("reading Metal host output"),
                }
            }
        }
        if pending.is_none() { pending = receive_relaunch(&relaunch, &token)?; }
        let status = guest.try_wait()?;
        if let Some(host_status) = host.try_wait()? { bail!("Metal host exited while Wine was running: {host_status}"); }
        if let Some(status) = status {
            let Some(request) = pending.take() else {
                return Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)));
            };
            if !status.success() { bail!("Wine exited with {status} after requesting relaunch"); }
            // End and reap the old process group before launching the new one.
            drop(guest);
            if !request.exe.is_empty() {
                let path = winepath_output(&winepath, &wine_prefix, "-u", OsStr::new(&request.exe))
                    .context("converting Wine relaunch executable")?;
                current_exe = regular_file(&PathBuf::from(OsString::from_vec(path)), "relaunch executable")?;
                // Validate the mapped host file, but let Wine open the exact
                // requested Windows path so GetModuleFileName preserves it.
                launch_path = OsString::from(&request.exe);
            }
            reject_known_shadows(&current_exe, &dll_dir, &wine_prefix)?;
            crate::wine_imports::run(&current_exe, &dll_dir, provision_gameinput, guest_dll)?;
            if let Some(root) = &title_root { checked_title_root(&wine_prefix, root)?; }
            if let (Some(letter), Some((save, _))) = (storage_drive, &save_root) {
                checked_storage_drive(&wine_prefix, save, letter)?;
            }
            if let Some(provider) = &input_provider {
                checked_gameinput_installation(&wine_prefix, provider)?;
                reject_gameinput_shadows(&current_exe, &wine_prefix)?;
            }
            let next_args: Vec<OsString> = request.args.split_whitespace().map(OsString::from).collect();
            guest = ManagedChild::spawn(&mut wine_command(&launch_path, &next_args), "relaunched Wine")?;
        }
        std::thread::sleep(POLL);
    }
}

pub fn run(exe: &Path, wine: &Path, winepath: &Path, metal_host: &Path, converter: &Path,
    dll_dir: &Path, wine_prefix: &Path, allow_unverified_dll_search: bool, provision_gameinput: bool, guest_dll: &[String], title_root: Option<&Path>, save_root: Option<&Path>, storage_drive: Option<char>, args: &[OsString]) -> Result<i32> {
    match run_inner(exe, wine, winepath, metal_host, converter, dll_dir, wine_prefix, allow_unverified_dll_search, provision_gameinput, guest_dll, title_root, save_root, storage_drive, args) {
        Err(error) if error.downcast_ref::<Interrupted>().is_some() => {
            Ok(128 + error.downcast_ref::<Interrupted>().expect("checked above").0)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunch_accepts_a_fragmented_request() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let token = "0123456789abcdef0123456789abcdef";
        let (ready, connected) = std::sync::mpsc::channel();
        let client = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            stream.write_all(b"VILNCH01").unwrap();
            stream.write_all(token.as_bytes()).unwrap();
            stream.write_all(&3u32.to_le_bytes()).unwrap();
            stream.write_all(&3u32.to_le_bytes()).unwrap();
            stream.write_all(b"C:\\one").unwrap();
            let mut ack = [0];
            stream.read_exact(&mut ack).unwrap();
            assert_eq!(ack, [1]);
        });
        connected.recv().unwrap();
        let request = receive_relaunch(&listener, token).unwrap().unwrap();
        assert_eq!(request.exe, "C:\\");
        assert_eq!(request.args, "one");
        client.join().unwrap();
    }
}
