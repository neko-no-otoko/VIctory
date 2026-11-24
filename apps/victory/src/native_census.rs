//! Offline import and metadata report for the native macOS resolver.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use victory_loader::{ImportResolver, Resolution};
use victory_native::{FileScope, NativeResolver};
use victory_pe::ImageInfo;

pub fn run(path: &Path, file_root: Option<&Path>, save_root: Option<&Path>) -> Result<()> {
    ensure!(save_root.is_none() || file_root.is_some(), "--save-root requires --file-root");
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let info = ImageInfo::parse(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let _scope = file_root
        .map(|root| FileScope::enter_with_save(root, save_root))
        .transpose()
        .context("opening native file scope")?;
    let resolver = NativeResolver;

    println!("Native import census: {}", path.display());
    println!("Offline report only: no image is mapped or executed; game compatibility is not assessed.");
    println!("Results are from NativeResolver only; no DLL providers are inferred.");
    println!("File scope: {}", if file_root.is_some() { "enabled" } else { "disabled" });
    println!("Save scope: {}", if save_root.is_some() { "enabled" } else { "disabled" });
    println!("Resolved imports indicate symbol availability only; individual calls may be unsupported.");

    let mut ordinary_found = 0usize;
    let mut ordinary_missing = 0usize;
    let mut delay_found = 0usize;
    let mut delay_missing = 0usize;
    for (kind, modules) in [("ordinary", &info.imports), ("delay", &info.delay_imports)] {
        for module in modules {
            for entry in &module.entries {
                let resolution = resolver.resolve(&module.dll, &entry.symbol);
                let (found, status) = match resolution {
                    Resolution::Found { address, .. } if address != 0 => (true, "resolved"),
                    _ => (false, "missing"),
                };
                let (found_count, missing_count) = if kind == "ordinary" {
                    (&mut ordinary_found, &mut ordinary_missing)
                } else {
                    (&mut delay_found, &mut delay_missing)
                };
                if found { *found_count += 1; } else { *missing_count += 1; }
                println!("{kind} {}!{}: {status}", module.dll, entry.symbol);
                if kind == "delay" {
                    println!("  native loader: unsupported");
                }
            }
        }
    }
    println!(
        "imports: {} ordinary, {} delay; {} resolved, {} missing",
        ordinary_found + ordinary_missing,
        delay_found + delay_missing,
        ordinary_found + delay_found,
        ordinary_missing + delay_missing,
    );
    println!("Native loader metadata checks (absence passes; presence is rejected):");
    for (name, present) in [
        ("TLS", info.tls.is_some()),
        ("exception", info.has_exception_table),
        ("delay imports", !info.delay_imports.is_empty()),
        ("load config", info.has_load_config),
    ] {
        println!("  {name}: {}", if present { "rejected" } else { "absent" });
    }
    Ok(())
}
