//! Explicit acyclic DLL sets, mapped and initialized dependency-first.
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use anyhow::{bail, ensure, Context, Result};
use victory_pe::{ImageInfo, ImportSymbol};
use crate::{ImportResolver, Provenance, Resolution};
use super::Image;

/// Owns every module in dependency order; all exports become invalid on drop.
pub struct ModuleSet {
    modules: Vec<(String, Image)>,
    attached: usize,
}

struct SetResolver<'a> {
    modules: &'a [(String, Image)],
    names: &'a [String],
    external: &'a dyn ImportResolver,
    error: RefCell<Option<String>>,
}
impl ImportResolver for SetResolver<'_> {
    fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
        let Some((_, image)) = self.modules.iter().find(|(name, _)| name.eq_ignore_ascii_case(dll)) else {
            return if self.names.iter().any(|name| name.eq_ignore_ascii_case(dll)) {
                Resolution::Missing
            } else { self.external.resolve(dll, symbol) };
        };
        let export = image.info.exports.iter().find(|e| match symbol {
            ImportSymbol::Name { name, .. } => e.name.as_deref() == Some(name),
            ImportSymbol::Ordinal(n) => e.ordinal == *n,
        });
        let Some(export) = export else { return Resolution::Missing };
        if export.forward.is_some() {
            *self.error.borrow_mut() = Some(format!("forwarded export {dll}!{symbol} is unsupported"));
            return Resolution::Missing;
        }
        let Some(rva) = export.rva else { return Resolution::Missing };
        if rva >= image.info.size_of_image { return Resolution::Missing; }
        // SAFETY: rva is bounded within the owned image mapping.
        let address = unsafe { image.base.as_ptr().add(rva as usize) } as usize;
        Resolution::Found { address, provenance: Provenance::Title }
    }
}

fn visit(i: usize, info: &[ImageInfo], names: &[String], states: &mut [u8], order: &mut Vec<usize>) -> Result<()> {
    if states[i] == 1 { bail!("cyclic DLL dependency involving {}", names[i]); }
    if states[i] == 2 { return Ok(()); }
    states[i] = 1;
    for import in &info[i].imports {
        if let Some(dep) = names.iter().position(|n| n.eq_ignore_ascii_case(&import.dll)) {
            visit(dep, info, names, states, order)?;
        }
    }
    states[i] = 2;
    order.push(i);
    Ok(())
}

impl ModuleSet {
    /// Map explicitly supplied DLLs in dependency order; no code is executed.
    pub fn load(inputs: &[(&str, &[u8])], external: &dyn ImportResolver) -> Result<Self> {
        ensure!(!inputs.is_empty(), "module set must not be empty");
        let mut names = Vec::with_capacity(inputs.len());
        let mut info = Vec::with_capacity(inputs.len());
        for (name, bytes) in inputs {
            ensure!(!name.is_empty() && !names.iter().any(|n: &String| n.eq_ignore_ascii_case(name)), "empty or duplicate DLL name: {name}");
            names.push((*name).to_owned());
            info.push(ImageInfo::parse(bytes)?);
        }
        let mut order = Vec::new();
        let mut states = vec![0; inputs.len()];
        for i in 0..inputs.len() { visit(i, &info, &names, &mut states, &mut order)?; }
        let mut modules = Vec::with_capacity(inputs.len());
        for i in order {
            let resolver = SetResolver { modules: &modules, names: &names, external, error: RefCell::new(None) };
            let result = Image::load(inputs[i].1, &resolver);
            if let Some(error) = resolver.error.into_inner() { bail!(error); }
            let image = result.with_context(|| format!("loading {}", names[i]))?;
            modules.push((names[i].clone(), image));
        }
        Ok(Self { modules, attached: 0 })
    }

    /// Attach in dependency order, rolling back successful attaches on failure.
    ///
    /// # Safety
    /// Every image must be trusted and satisfy `Image::attach`'s requirements.
    pub unsafe fn attach(&mut self) -> Result<()> {
        ensure!(self.attached == 0, "module set already attached");
        for i in 0..self.modules.len() {
            // SAFETY: the caller guarantees every module's lifecycle requirements.
            if let Err(error) = unsafe { self.modules[i].1.attach() } {
                while self.attached > 0 {
                    self.attached -= 1;
                    // SAFETY: previously attached modules are quiescent during rollback.
                    unsafe { self.modules[self.attached].1.detach() }?;
                }
                return Err(error.context(format!("attaching {}", self.modules[i].0)));
            }
            self.attached += 1;
        }
        Ok(())
    }

    /// Detach in reverse dependency order. Drop itself never invokes DLL code.
    ///
    /// # Safety
    /// All users must be quiescent and `Image::detach` requirements must hold.
    pub unsafe fn detach(&mut self) -> Result<()> {
        ensure!(self.attached == self.modules.len() && self.attached != 0, "module set is not attached");
        while self.attached > 0 {
            self.attached -= 1;
            // SAFETY: the caller guarantees lifecycle requirements.
            unsafe { self.modules[self.attached].1.detach() }?;
        }
        Ok(())
    }

    /// Returns an export address tied to this set's lifetime.
    pub fn export(&self, dll: &str, name: &str) -> Option<NonNull<c_void>> {
        self.modules.iter().find(|(n, _)| n.eq_ignore_ascii_case(dll))?.1.export(name)
    }
}

impl Drop for ModuleSet {
    fn drop(&mut self) {
        while self.modules.pop().is_some() {}
    }
}

/// A mapped executable and its owned, explicitly supplied DLL dependencies.
///
/// This does not create a Windows process, thread, TEB or startup environment.
pub struct Executable {
    image: Image,
    dependencies: ModuleSet,
    started: bool,
}

impl Executable {
    /// Map a PE executable and bind its imports without executing any code.
    pub fn load(bytes: &[u8], dlls: &[(&str, &[u8])], external: &dyn ImportResolver) -> Result<Self> {
        let dependencies = if dlls.is_empty() {
            ModuleSet { modules: Vec::new(), attached: 0 }
        } else {
            ModuleSet::load(dlls, external)?
        };
        let names: Vec<String> = dlls.iter().map(|(name, _)| (*name).to_owned()).collect();
        let resolver = SetResolver {
            modules: &dependencies.modules,
            names: &names,
            external,
            error: RefCell::new(None),
        };
        let result = Image::map(bytes, &resolver, false);
        if let Some(error) = resolver.error.into_inner() { bail!(error); }
        let image = result.context("loading executable")?;
        Ok(Self { image, dependencies, started: false })
    }

    /// Attach DLLs, call the executable entry point, then detach DLLs.
    ///
    /// This is only for controlled entry points known to use the signature
    /// `extern "win64" fn() -> u32` and return normally. It is not Windows
    /// process startup. An executable may be started at most once.
    ///
    /// # Safety
    /// The caller must trust every image and callback, guarantee the stated
    /// entry-point signature, and satisfy all `Image::attach` and `detach`
    /// requirements. There is no Windows process environment, loader lock,
    /// thread setup, TLS, exception handling, CRT startup, or exit handling.
    pub unsafe fn start(&mut self) -> Result<u32> {
        ensure!(!self.started, "executable has already started");
        if !self.dependencies.modules.is_empty() {
            // SAFETY: the caller guarantees DLL lifecycle requirements.
            unsafe { self.dependencies.attach() }?;
        }
        self.started = true;
        // SAFETY: load validated the entry is in executable image memory;
        // the caller guarantees its signature and that it returns normally.
        let entry: unsafe extern "win64" fn() -> u32 = unsafe {
            std::mem::transmute(self.image.base.as_ptr().add(self.image.info.entry_rva as usize))
        };
        // SAFETY: the caller guarantees the entry point is safe to execute.
        let result = unsafe { entry() };
        if !self.dependencies.modules.is_empty() {
            // SAFETY: the entry returned and the caller guarantees quiescence.
            unsafe { self.dependencies.detach() }?;
        }
        Ok(result)
    }
}
