//! Explicitly opt-in execution of controlled PE fixtures in this process.

use std::path::Path;

use anyhow::{Context, Result};
use victory_loader::macos::Executable;

pub fn run(path: &Path, execute: bool) -> Result<()> {
    eprintln!("WARNING: trusted controlled fixtures only; this is not Windows process startup or game support. --execute runs unsandboxed code in this process; it may crash, hang, or corrupt the process.");
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut executable = Executable::load(&bytes, &[], &victory_native::NativeResolver)
        .context("mapping native fixture")?;
    if !execute {
        println!("fixture mapped and imports resolved; entry not executed (pass --execute to opt in)");
        return Ok(());
    }
    // SAFETY: --execute explicitly makes the user responsible for trusting
    // this image, its callbacks, and its normally-returning win64 fn() -> u32
    // entry; no Windows process environment, TLS or exception support exists.
    let result = unsafe { executable.start() }.context("starting native fixture")?;
    println!("fixture entry returned {result}");
    Ok(())
}
