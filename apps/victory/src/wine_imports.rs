//! Offline inspection of the executable and guest DLLs beside it.
//!
//! This is not a Wine loader: its search order, API-set mapping, dynamically
//! loaded modules, and forwarder resolution cannot be established here.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use victory_pe::{ImageInfo, ImportSymbol, layout};

const PROVIDERS: [&str; 6] = ["d3d12_xs.dll", "xg_xs.dll", "xgameruntime.dll", "xmem.dll", "acphal.dll", "xapu.dll"];
const BACKEND: &str = "victory_xmem_backend.dll";
const BACKEND_EXPORTS: [&str; 14] = [
    "XMemAlloc", "XMemFree", "XMemAllocDefault", "XMemFreeDefault",
    "XMemIsToolsMemAvailable", "XMemAllocatePhysicalPages", "XMemFreePhysicalPages",
    "XMemMapPhysicalPages", "XMemMapPhysicalPagesScatter", "XMemVirtualAlloc",
    "XMemVirtualQuery", "XMemGetWorkingSetStatistics", "XMemTransferMemory",
    "VIctoryXMemMapAlias",
];
// These console-only providers appear in victory-hle's module registry but
// have no corresponding Wine HLE DLL in this build (modules/dsp.rs and
// modules/dstorage.rs respectively). Do not infer providers from a prefix.
const UNSUPPORTED: [&str; 2] = ["chad.dll", "xcrdapi.dll"];

fn image(path: &Path, dll: bool) -> Result<(ImageInfo, usize)> {
    if !fs::metadata(path).with_context(|| format!("examining PE {}", path.display()))?.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    let data = fs::read(path).with_context(|| format!("reading PE {}", path.display()))?;
    let info = ImageInfo::parse(&data).with_context(|| format!("parsing AMD64 PE {}", path.display()))?;
    if info.characteristics & 0x0002 == 0 {
        bail!("{} is not an executable PE image", path.display());
    }
    if (info.characteristics & layout::file_flags::DLL != 0) != dll {
        bail!("{} is not a PE {}", path.display(), if dll { "DLL" } else { "executable" });
    }
    Ok((info, data.len()))
}

fn export_error(info: &ImageInfo, len: usize, name: &str) -> Option<String> {
    let Some(export) = info.exports.iter().find(|e| e.name.as_deref() == Some(name)) else {
        return Some(format!("does not export {name}"));
    };
    let Some(rva) = export.rva.filter(|rva| *rva != 0 && export.forward.is_none()) else {
        return Some(format!("export {name} is forwarded or unbacked"));
    };
    if !info.sections.iter().any(|section| {
        rva.checked_sub(section.rva).is_some_and(|offset| {
            offset < section.file_size && u64::from(section.file_offset) + u64::from(offset) < len as u64
        })
    }) {
        return Some(format!("export {name} is not file-backed"));
    }
    None
}

// Windows path syntax is deliberately checked independently of the host OS.
fn safe_basename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    !reserved
        && !name.is_empty()
        && name.is_ascii()
        && name.to_ascii_lowercase().ends_with(".dll")
        && name.len() > 4
        && !name.starts_with('.')
        && !name.ends_with(' ')
        && !name.bytes().any(|b| b < 0x20 || b == 0x7f || b"/\\:<>\"|?*".contains(&b))
}

fn scan_dir(dir: &Path) -> Result<BTreeMap<String, Vec<PathBuf>>> {
    let mut files = BTreeMap::<String, Vec<PathBuf>>::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading guest directory {}", dir.display()))? {
        let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if safe_basename(name) {
            files.entry(name.to_ascii_lowercase()).or_default().push(entry.path());
        }
    }
    for paths in files.values_mut() { paths.sort(); }
    Ok(files)
}
fn ambiguity_error(name: &str, paths: &[PathBuf]) -> String {
    format!("ambiguous guest DLL {name}: {}", paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
}


fn load_provider(dir: &Path, name: &str, errors: &mut Vec<String>) -> Option<(ImageInfo, usize)> {
    match image(&dir.join(name), true) {
        Ok(value) => Some(value),
        Err(error) => { errors.push(format!("provider {name}: {error:#}")); None }
    }
}

/// Inspect only explicit PE imports, without executing Wine or assuming host DLLs.
pub(crate) fn run(exe: &Path, dll_dir: &Path, gameinput: bool, required_guest: &[String]) -> Result<()> {
    let exe_path = fs::canonicalize(exe).with_context(|| format!("resolving executable {}", exe.display()))?;
    let exe = exe_path.as_path();
    let mut errors = Vec::new();
    let mut report = Vec::new();
    let mut external = Vec::new();
    let guest_dir = exe.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let candidates = scan_dir(guest_dir)?;
    let mut providers = BTreeMap::new();
    for name in PROVIDERS {
        if let Some(provider) = load_provider(dll_dir, name, &mut errors) {
            providers.insert(name, provider);
        }
    }
    if let Some((backend, len)) = load_provider(dll_dir, BACKEND, &mut errors) {
        for name in BACKEND_EXPORTS {
            if let Some(error) = export_error(&backend, len, name) {
                errors.push(format!("provider {BACKEND}: {error}"));
            }
        }
    }
    if gameinput {
        if let Some((provider, len)) = load_provider(dll_dir, "gameinput.dll", &mut errors) {
            if let Some(error) = export_error(&provider, len, "GameInputCreate") {
                errors.push(format!("provider gameinput.dll: {error}"));
            }
            if let Some(export) = provider.exports.iter().find(|e| e.name.as_deref() == Some("GameInputCreate")) {
                if export.ordinal != 1 {
                    errors.push("provider gameinput.dll: GameInputCreate must have ordinal 1".to_owned());
                }
            }
            providers.insert("gameinput.dll", (provider, len));
        }
    }

    let mut queue = VecDeque::new();
    let mut scheduled = BTreeSet::new();
    let mut graph = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
    let ambiguous: BTreeSet<_> = candidates.iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, _)| name.clone())
        .collect();
    let mut reported_ambiguities = BTreeSet::new();
    for name in required_guest {
        if !safe_basename(name) {
            errors.push(format!("invalid --guest-dll basename {name:?}; expected a safe DLL basename"));
            continue;
        }
        let key = name.to_ascii_lowercase();
        match candidates.get(&key) {
            None => errors.push(format!("required guest DLL {name} not found in {}", guest_dir.display())),
            Some(paths) if paths.len() == 1 => {
                if scheduled.insert(key) { queue.push_back(paths[0].clone()); }
            }
            Some(paths) => {
                if reported_ambiguities.insert(key.clone()) {
                    errors.push(ambiguity_error(&key, paths));
                }
            }
        }
    }
    // The executable is distinct from the DLL worklist, even if its basename
    // happens to coincide with a declared guest DLL.
    let mut pending = VecDeque::from([(exe.to_path_buf(), false)]);
    while !pending.is_empty() || !queue.is_empty() {
        if pending.is_empty() {
            if let Some(path) = queue.pop_front() { pending.push_back((path, true)); }
        }
        let Some((path, is_dll)) = pending.pop_front() else { break };
        let (info, _) = match image(&path, is_dll) {
            Ok(value) => value,
            Err(error) => { errors.push(format!("guest {}: {error:#}", path.display())); continue; }
        };
        report.push(format!("inspected {}", path.display()));
        for (kind, modules) in [("ordinary", &info.imports), ("delay", &info.delay_imports)] {
            for module in modules {
                let key = module.dll.to_ascii_lowercase();
                let provider = PROVIDERS.contains(&key.as_str()) || key == "gameinput.dll";
                let unsupported = UNSUPPORTED.contains(&key.as_str());
                let safe = safe_basename(&module.dll);
                if safe && ambiguous.contains(&key) && reported_ambiguities.insert(key.clone()) {
                    errors.push(ambiguity_error(&key, &candidates[&key]));
                }
                if safe && !provider && !ambiguous.contains(&key) {
                    if let Some(paths) = candidates.get(&key) {
                        if paths.len() == 1 {
                            graph.entry(path.clone()).or_default().insert(paths[0].clone());
                            if scheduled.insert(key.clone()) {
                                queue.push_back(paths[0].clone());
                            }
                        }
                    }
                }
                // Preserve module-level diagnostics even for an empty import table.
                let symbols: Vec<String> = if module.entries.is_empty() {
                    vec!["<no symbols>".to_owned()]
                } else {
                    module.entries.iter().map(|entry| entry.symbol.to_string()).collect()
                };
                for (index, symbol) in symbols.iter().enumerate() {
                    let context = format!("{}: {kind} import {}!{symbol}", path.display(), module.dll);
                    if !safe {
                        external.push(format!("{context}: path-bearing or non-basename import; Wine resolution unclassified"));
                    } else if unsupported && !candidates.contains_key(&key) {
                        external.push(format!("{context}: no VIctory Wine provider for {}; Wine resolution unverified", module.dll));
                    } else if unsupported {
                        report.push(format!("{context}: no VIctory Wine provider; guest candidate selection and ABI are unverified"));
                    } else if provider {
                        if key == "gameinput.dll" && !gameinput {
                            errors.push(format!("{context}: GameInput requires --provision-gameinput"));
                        } else if let Some(entry) = module.entries.get(index) {
                            match &entry.symbol {
                                ImportSymbol::Ordinal(1) if key == "gameinput.dll" => {}
                                ImportSymbol::Ordinal(ordinal) if key == "gameinput.dll" => errors.push(format!("{context}: unsupported GameInput import #{ordinal}")),
                                ImportSymbol::Ordinal(ordinal) => errors.push(format!("{context}: unsupported ordinal #{ordinal}; provider ordinals are not a stable ABI")),
                                ImportSymbol::Name { name, .. } if key == "gameinput.dll" && name != "GameInputCreate" => {
                                    errors.push(format!("{context}: unsupported GameInput import"));
                                }
                                ImportSymbol::Name { name, .. } => {
                                    if let Some((info, len)) = providers.get(key.as_str()) {
                                        if let Some(error) = export_error(info, *len, name) {
                                            errors.push(format!("{context}: {error}"));
                                        }
                                    }
                                }
                            }
                        }
                        report.push(format!("{context}: HLE provider import; Wine selection is not verified"));
                    } else if ambiguous.contains(&key) {
                        report.push(format!("{context}: ambiguous guest candidates; not inspected"));
                    } else if candidates.contains_key(&key) {
                        report.push(format!("{context}: guest candidate beside executable; Wine selection and exports are not verified"));
                    } else {
                        external.push(format!("{context}: unresolved external (Wine resolution unclassified)"));
                    }
                }
            }
        }
    }
    // A DFS distinguishes real cycles from shared dependencies. The worklist
    // above already prevents repeatedly parsing either kind of edge.
    let mut state = BTreeMap::<PathBuf, u8>::new();
    for root in graph.keys() {
        if state.contains_key(root) { continue; }
        let mut stack = vec![(root.clone(), false)];
        while let Some((node, leaving)) = stack.pop() {
            if leaving {
                state.insert(node, 2);
                continue;
            }
            if state.contains_key(&node) { continue; }
            state.insert(node.clone(), 1);
            stack.push((node.clone(), true));
            if let Some(children) = graph.get(&node) {
                for child in children.iter().rev() {
                    match state.get(child) {
                        Some(1) => report.push(format!("guest import cycle: {} -> {}", node.display(), child.display())),
                        Some(2) => {}
                        _ => stack.push((child.clone(), false)),
                    }
                }
            }
        }
    }
    report.sort();
    external.sort();
    errors.sort();
    println!("Offline Wine import diagnostic (not a compatibility or DLL selection guarantee)");
    println!("Only the executable directory is searched for guest DLL candidates; API sets, forwarders, dynamic loads and Wine search/overrides are not resolved.");
    println!("Inspected images and classified imports:");
    for line in report { println!("  {line}"); }
    println!("Unclassified Wine resolution:");
    for line in external { println!("  {line}"); }
    if !errors.is_empty() {
        eprintln!("Confirmed problems:");
        for error in &errors { eprintln!("  {error}"); }
        bail!("offline Wine import diagnostic found {} problem(s)", errors.len());
    }
    println!("No confirmed problems in the inspected scope.");
    Ok(())
}
