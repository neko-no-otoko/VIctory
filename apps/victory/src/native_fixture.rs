//! Explicitly opt-in execution of controlled PE fixtures in this process.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use victory_loader::macos::Executable;

pub fn run(path: &Path, dlls: &[PathBuf], execute: bool) -> Result<()> {
    eprintln!("WARNING: trusted controlled fixtures only; this is not Windows process startup or game support. --execute runs unsandboxed code in this process; it may crash, hang, or corrupt the process.");
    let mut names: Vec<&str> = Vec::with_capacity(dlls.len());
    for dll in dlls {
        let name = dll.file_name().and_then(|name| name.to_str()).filter(|name| !name.is_empty())
            .with_context(|| format!("DLL path has no UTF-8 filename: {}", dll.display()))?;
        if names.iter().any(|other| other.eq_ignore_ascii_case(name)) {
            bail!("duplicate DLL filename (ASCII case-insensitive): {name}");
        }
        names.push(name);
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let dll_bytes = dlls.iter().map(|dll| {
        std::fs::read(dll).with_context(|| format!("reading DLL {}", dll.display()))
    }).collect::<Result<Vec<_>>>()?;
    let inputs: Vec<_> = names.into_iter().zip(dll_bytes.iter().map(Vec::as_slice)).collect();
    let mut executable = Executable::load(&bytes, &inputs, &victory_native::NativeResolver)
        .context("mapping native fixture")?;
    if !execute {
        println!("fixture mapped and imports resolved; entry not executed (pass --execute to opt in)");
        return Ok(());
    }
    // SAFETY: --execute makes the user responsible for trusting the executable,
    // every DLL and callback, lifecycle and thread quiescence, and a normally
    // returning win64 fn() -> u32 entry. No Windows process environment exists.
    let result = unsafe { executable.start() }.context("starting native fixture")?;
    println!("fixture entry returned {result}");
    Ok(())
}
