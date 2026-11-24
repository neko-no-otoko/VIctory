//! Minimal x86-64 macOS PE32+ mapping for synthetic modules.
//!
//! This is not a Windows process or an Xbox runtime. TLS, exception tables,
//! load configuration and delay imports are unsupported. DLL entry points
//! may be invoked only through the explicit unsafe lifecycle methods.
//! Export calls and resolver-provided functions use the Windows x64 ABI.

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{Context, Result, bail, ensure};
use victory_pe::{ImageInfo, layout};

use crate::{ImportResolver, Resolution};
mod graph;
pub use graph::{Executable, ModuleSet};

const READ: u32 = 0x4000_0000;
const WRITE: u32 = 0x8000_0000;
const EXECUTE: u32 = 0x2000_0000;
const RELOC_DIR64: u16 = 10;

/// An owned, rebased image. Dropping it unmaps it without calling DLL detach.
pub struct Image {
    base: NonNull<u8>,
    len: usize,
    info: ImageInfo,
    attached: bool,
}

impl Drop for Image {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns the mapping returned by mmap.
        unsafe { libc::munmap(self.base.as_ptr().cast(), self.len) };
    }
}

impl Image {
    /// Maps an image without executing it, resolving its ordinary imports.
    ///
    /// Windows exception handling, delay loads, TLS and load configuration
    /// are rejected rather than partly initialized. This does not call DllMain.
    pub fn load(bytes: &[u8], resolver: &dyn ImportResolver) -> Result<Self> {
        Self::map(bytes, resolver, true)
    }

    fn map(bytes: &[u8], resolver: &dyn ImportResolver, dll: bool) -> Result<Self> {
        let info = ImageInfo::parse(bytes)?;
        ensure!(info.is_dll() == dll, "expected {} image", if dll { "DLL" } else { "executable" });
        ensure!(dll || info.entry_rva != 0, "executable has no entry point");
        ensure!(info.tls.is_none(), "TLS is not supported");
        ensure!(!info.has_exception_table, "Windows unwind tables are not supported");
        ensure!(!info.has_load_config, "load configuration is not supported");
        ensure!(info.delay_imports.is_empty(), "delay imports are not supported");
        let len = info.size_of_image as usize;
        ensure!(len != 0, "empty image");
        let optional = layout::optional_header_offset(bytes).context("missing optional header")?;
        let header_size = layout::read_u32(bytes, optional + 60).context("missing header size")? as usize;
        ensure!(header_size <= len && header_size <= bytes.len(), "invalid header size");
        // SAFETY: sysconf has no pointer arguments.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        ensure!(page > 0, "cannot determine page size");
        let page = page as usize;
        let pages = len.checked_add(page - 1).context("image too large")? / page;
        let map_len = pages.checked_mul(page).context("image too large")?;
        let mut protections = vec![0; pages];
        for slot in protections.iter_mut().take(header_size.div_ceil(page)) {
            *slot |= libc::PROT_READ;
        }
        let mut ranges = vec![0..header_size];
        for section in &info.sections {
            let start = section.rva as usize;
            let size = (section.virtual_size as usize).max(section.file_size as usize);
            let end = start.checked_add(size).context("section size overflow")?;
            ensure!(end <= len, "section outside image");
            let file_start = section.file_offset as usize;
            let file_end = file_start.checked_add(section.file_size as usize).context("section file range overflow")?;
            ensure!(file_end <= bytes.len(), "section outside file");
            if size == 0 { continue; }
            ensure!(ranges.iter().all(|r| end <= r.start || start >= r.end), "overlapping sections or headers");
            ranges.push(start..end);
            let flags = section.characteristics;
            let prot = if flags & READ != 0 { libc::PROT_READ } else { 0 }
                | if flags & WRITE != 0 { libc::PROT_WRITE } else { 0 }
                | if flags & EXECUTE != 0 { libc::PROT_EXEC } else { 0 };
            for slot in &mut protections[start / page..end.div_ceil(page)] {
                *slot |= prot;
                ensure!(*slot & (libc::PROT_WRITE | libc::PROT_EXEC) != (libc::PROT_WRITE | libc::PROT_EXEC), "writable executable page is unsupported");
            }
        }
        if info.entry_rva != 0 {
            let entry = info.entry_rva as usize;
            ensure!(info.sections.iter().any(|s| {
                let start = s.rva as usize;
                let size = (s.virtual_size as usize).max(s.file_size as usize);
                s.characteristics & EXECUTE != 0 && entry >= start && entry - start < size
            }), "entry point is not in an executable section");
        }
        // SAFETY: anonymous private mapping; map_len is nonzero.
        let raw = unsafe { libc::mmap(std::ptr::null_mut(), map_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        ensure!(raw != libc::MAP_FAILED, "mmap failed: {}", std::io::Error::last_os_error());
        let base = NonNull::new(raw.cast::<u8>()).context("mmap returned null")?;
        let image = Self { base, len: map_len, info, attached: false };
        // SAFETY: the newly allocated mapping is writable and map_len bytes long.
        let mapped = unsafe { std::slice::from_raw_parts_mut(image.base.as_ptr(), len) };
        mapped[..header_size].copy_from_slice(&bytes[..header_size]);
        for section in &image.info.sections {
            let start = section.rva as usize;
            let file_start = section.file_offset as usize;
            let size = section.file_size as usize;
            mapped[start..start + size].copy_from_slice(&bytes[file_start..file_start + size]);
        }
        let delta = (image.base.as_ptr() as u64).wrapping_sub(image.info.image_base);
        if delta != 0 {
            ensure!(image.info.has_relocations, "image cannot be rebased");
            let at = layout::data_directory_offset(bytes, layout::dir::BASERELOC).context("missing relocation directory")?;
            let start = layout::read_u32(bytes, at).context("missing relocation RVA")? as usize;
            let size = layout::read_u32(bytes, at + 4).context("missing relocation size")? as usize;
            relocate(mapped, start, size, delta)?;
        }
        for module in &image.info.imports {
            for entry in &module.entries {
                let address = match resolver.resolve(&module.dll, &entry.symbol) {
                    Resolution::Found { address, .. } if address != 0 => address,
                    _ => bail!("unresolved import {}!{}", module.dll, entry.symbol),
                };
                let at = entry.iat_slot_rva as usize;
                let slot = mapped.get_mut(at..at.checked_add(8).context("IAT overflow")?).context("IAT outside image")?;
                slot.copy_from_slice(&(address as u64).to_le_bytes());
            }
        }
        for (i, &prot) in protections.iter().enumerate() {
            // SAFETY: each page is within the mapping and page aligned.
            let page_addr = unsafe { image.base.as_ptr().add(i * page) };
            // SAFETY: page_addr identifies a whole page of the owned mapping.
            ensure!(unsafe { libc::mprotect(page_addr.cast(), page, prot) } == 0,
                "mprotect failed: {}", std::io::Error::last_os_error());
        }
        Ok(image)
    }

    /// Calls DLL_PROCESS_ATTACH once, or marks an entrypoint-free DLL attached.
    /// An entry point returning FALSE leaves the image unattached.
    ///
    /// # Safety
    /// The caller must trust the DLL and ensure its imports, ABI, dependencies,
    /// thread environment and any required serialization are valid. Windows
    /// loader-lock and thread notifications are not provided.
    pub unsafe fn attach(&mut self) -> Result<()> {
        ensure!(!self.attached, "image is already attached");
        if self.info.entry_rva != 0 {
            // SAFETY: load validated the entry lies in executable image memory;
            // the caller is responsible for the DLL's ABI and requirements.
            let entry: unsafe extern "win64" fn(*mut c_void, u32, *mut c_void) -> i32 =
                unsafe { std::mem::transmute(self.base.as_ptr().add(self.info.entry_rva as usize)) };
            // SAFETY: the caller guarantees the entry point can run here.
            ensure!(unsafe { entry(self.base.as_ptr().cast(), crate::DLL_PROCESS_ATTACH, std::ptr::null_mut()) } != 0,
                "DLL_PROCESS_ATTACH returned FALSE");
        }
        self.attached = true;
        Ok(())
    }

    /// Calls DLL_PROCESS_DETACH once for a successfully attached image.
    ///
    /// # Safety
    /// The requirements of `attach` still apply. All users of the image must
    /// be quiescent. Call this before dropping an attached image; Drop cannot
    /// safely invoke arbitrary DLL code and only unmaps the image.
    pub unsafe fn detach(&mut self) -> Result<()> {
        ensure!(self.attached, "image is not attached");
        if self.info.entry_rva != 0 {
            // SAFETY: the entry point was validated when the image was mapped.
            let entry: unsafe extern "win64" fn(*mut c_void, u32, *mut c_void) -> i32 =
                unsafe { std::mem::transmute(self.base.as_ptr().add(self.info.entry_rva as usize)) };
            // SAFETY: the caller guarantees the entry point can run here.
            unsafe { entry(self.base.as_ptr().cast(), crate::DLL_PROCESS_DETACH, std::ptr::null_mut()) };
        }
        self.attached = false;
        Ok(())
    }

    /// Address of a named, non-forwarded export. Valid only while `self` lives.
    /// Calling it requires the correct Windows x64 ABI and function signature.
    pub fn export(&self, name: &str) -> Option<NonNull<c_void>> {
        let export = self.info.exports.iter().find(|e| e.name.as_deref() == Some(name))?;
        let rva = export.rva? as usize;
        if rva >= self.info.size_of_image as usize { return None; }
        // SAFETY: rva is within the mapping; this does not dereference it.
        NonNull::new(unsafe { self.base.as_ptr().add(rva) }.cast())
    }
}

fn relocate(image: &mut [u8], start: usize, size: usize, delta: u64) -> Result<()> {
    let end = start.checked_add(size).context("relocation range overflow")?;
    ensure!(end <= image.len(), "relocation directory outside image");
    let mut at = start;
    while at < end {
        ensure!(end - at >= 8, "truncated relocation block");
        let page = u32::from_le_bytes(image[at..at + 4].try_into()?) as usize;
        let block = u32::from_le_bytes(image[at + 4..at + 8].try_into()?) as usize;
        ensure!(block >= 8 && block % 2 == 0 && block <= end - at, "invalid relocation block");
        for entry_at in (at + 8..at + block).step_by(2) {
            let entry = u16::from_le_bytes(image[entry_at..entry_at + 2].try_into()?);
            match entry >> 12 {
                0 => continue,
                RELOC_DIR64 => {
                    let target = page.checked_add((entry & 0xfff) as usize).context("relocation target overflow")?;
                    let slot = image.get_mut(target..target.checked_add(8).context("relocation overflow")?).context("relocation target outside image")?;
                    let value = u64::from_le_bytes(slot.try_into()?);
                    slot.copy_from_slice(&value.wrapping_add(delta).to_le_bytes());
                }
                kind => bail!("unsupported relocation type {kind}"),
            }
        }
        at += block;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::relocate;

    #[test]
    fn relocations_are_bounded_and_require_dir64() {
        let mut bytes = [0u8; 32];
        bytes[4..8].copy_from_slice(&10u32.to_le_bytes());
        bytes[8..10].copy_from_slice(&0xa010u16.to_le_bytes());
        bytes[16..24].copy_from_slice(&100u64.to_le_bytes());
        relocate(&mut bytes, 0, 10, 5).unwrap();
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 105);
        bytes[8..10].copy_from_slice(&0x3010u16.to_le_bytes());
        assert!(relocate(&mut bytes, 0, 10, 5).is_err());
        assert!(relocate(&mut bytes, 0, 9, 5).is_err());
        assert!(relocate(&mut bytes, usize::MAX, 10, 5).is_err());
        bytes[8..10].copy_from_slice(&0xafffu16.to_le_bytes());
        assert!(relocate(&mut bytes, 0, 10, 5).is_err());
    }
}
