//! `victory`: the command-line front end.
//!
//! Inspection commands read the user's own files. `wine-run` executes trusted
//! guest code without a sandbox and Wine may write to its selected prefix.

#[cfg(windows)]
mod census;
mod inspect;
#[cfg(target_os = "macos")]
mod metal;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod native_census;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod native_fixture;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod wine_launcher;
#[cfg(windows)]
mod run;
mod shader;
mod translate;

use std::path::PathBuf;

use anyhow::Result;
#[cfg(windows)]
use clap::ValueEnum;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "victory", version)]
#[cfg_attr(
    windows,
    command(about = "Run an Xbox Series title natively on Windows")
)]
#[cfg_attr(
    not(windows),
    command(about = "Inspect PE images and analyze shaders offline")
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarise one PE image: headers, sections, imports, TLS, build id.
    Inspect {
        file: PathBuf,
        /// List every imported function.
        #[arg(long)]
        imports: bool,
        /// List every exported function.
        #[arg(long)]
        exports: bool,
    },
    #[cfg(windows)]
    /// Aggregate the imports of every PE image under the given paths and
    /// show which ones our runtime covers. Defaults to the configured
    /// game and test-title directories.
    Census {
        paths: Vec<PathBuf>,
        /// Do not count host pass-through as covered.
        #[arg(long)]
        strict: bool,
        /// List every missing `dll!function`, most used first.
        #[arg(long)]
        missing: bool,
    },
    /// List the `dll!export` pairs an import library describes.
    LibExports {
        lib: PathBuf,
        /// List every export, not just per-DLL counts.
        #[arg(long)]
        list: bool,
    },
    /// Find shader containers in files or directories (also inside engine
    /// archives, if uncompressed) and report their layout.
    ShaderInfo {
        paths: Vec<PathBuf>,
        /// Print one line per container found.
        #[arg(long)]
        verbose: bool,
        /// Also list the decoded native instructions (implies --verbose).
        #[arg(long)]
        disasm: bool,
    },
    /// Run every shader found (containers, SGD5 files) through the
    /// translator and report what it cannot handle yet.
    Translate {
        /// Search configured game and test-title directories when omitted.
        paths: Vec<PathBuf>,
        /// Check emitted DXIL with dxil.dll's validator (`VICTORY_DXIL`,
        /// else the newest Windows SDK's).
        #[arg(long)]
        validate: bool,
        /// Also create a host D3D12 pipeline for each emitted shader.
        #[arg(long)]
        pipelines: bool,
        /// Print one line per program and outcome.
        #[arg(long)]
        list: bool,
        /// Worker threads (default: every logical core).
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Convert a DXIL file to a Metal library using Apple's converter.
    #[cfg(target_os = "macos")]
    ConvertMetal {
        /// Input DXIL container.
        input: PathBuf,
        /// Output Metal library (outside the source tree).
        #[arg(long)]
        output: PathBuf,
        /// Path to Apple's libmetalirconverter.dylib.
        #[arg(long)]
        library: PathBuf,
        /// Shader stage of the input.
        #[arg(long, value_enum)]
        stage: metal::Stage,
    },
    /// Report native resolver coverage without mapping or executing a PE.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    NativeCensus {
        /// PE32+ image to inspect.
        file: PathBuf,
        /// Enable the scoped file provider for resolution reporting.
        #[arg(long)]
        file_root: Option<PathBuf>,
        /// Configure the separate writable save root.
        #[arg(long, requires = "file_root")]
        save_root: Option<PathBuf>,
    },
    /// Map a trusted, controlled PE fixture; never Windows process startup.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    #[command(long_about = "Map a trusted, controlled PE fixture and explicitly supplied DLLs. With --execute, call its normally returning extern win64 fn() -> u32 entry in this process, without isolation. This is not Windows process startup or game support; Windows process state, TLS, exception handling and CRT startup are absent.")]
    NativeFixture {
        /// Self-authored PE executable with a controlled Windows x64 entry.
        exe: PathBuf,
        /// Trusted dependent DLL path; repeat for each dependency.
        #[arg(long)]
        dll: Vec<PathBuf>,
        /// Allow the restricted read-only file API under this directory.
        #[arg(long)]
        file_root: Option<PathBuf>,
        /// Bind the writable T: save drive to this separate directory.
        #[arg(long, requires = "file_root")]
        save_root: Option<PathBuf>,
        /// Execute trusted code in this process (unsafe; no isolation).
        #[arg(long)]
        execute: bool,
    },
    /// Run a trusted PE as a Wine process with the native Metal bridge.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    #[command(long_about = "Run a trusted PE as a normal Wine process using explicit HLE DLLs and a native Metal bridge. Wine's DLL search order still applies. This is not a sandbox or a claim of game compatibility.")]
    WineRun {
        /// Trusted Windows x86-64 executable.
        exe: PathBuf,
        /// Wine executable.
        #[arg(long)]
        wine: PathBuf,
        /// Wine's winepath executable.
        #[arg(long)]
        winepath: PathBuf,
        /// Native wine_metal_compute_host executable.
        #[arg(long)]
        metal_host: PathBuf,
        /// Apple Metal Shader Converter dylib.
        #[arg(long)]
        converter: PathBuf,
        /// Directory containing the VIctory HLE DLLs and private XMem backend.
        #[arg(long)]
        dll_dir: PathBuf,
        /// Existing, explicitly selected Wine prefix directory.
        #[arg(long)]
        wine_prefix: PathBuf,
        /// Accept that Wine's remaining DLL search rules and races cannot be pinned by this launcher.
        #[arg(long)]
        allow_unverified_dll_search: bool,
        /// Provision GameInput only in a new empty prefix, or reuse an exact existing installation.
        #[arg(long)]
        provision_gameinput: bool,
        /// Require the existing Wine G: drive to point at this title directory.
        #[arg(long)]
        title_root: Option<PathBuf>,
        /// Enable offline XGameSave under this existing, dedicated directory.
        #[arg(long)]
        save_root: Option<PathBuf>,
        /// Use an existing Wine drive mapped to the save root's storage directory.
        #[arg(long, requires = "save_root")]
        storage_drive: Option<char>,
        /// Arguments passed to the guest after `--`.
        #[arg(last = true)]
        guest_args: Vec<std::ffi::OsString>,
    },
    #[cfg(windows)]
    /// Prepare, map and bind a title executable; with --execute, run it.
    Run {
        exe: PathBuf,
        /// What unimplemented imports do when called.
        #[arg(long, value_enum, default_value_t = Unimplemented::Trap)]
        unimplemented: Unimplemented,
        /// Disable host pass-through (everything must be implemented by us).
        #[arg(long)]
        strict: bool,
        /// Actually start the title. Without it, stop after binding.
        #[arg(long)]
        execute: bool,
        /// The console model the title sees (device type, memory).
        #[arg(long, value_enum, default_value_t = Console::SeriesX)]
        console: Console,
        /// Arguments for the title, after `--` (it reads the whole command
        /// line; a title that relaunches itself passes some).
        #[arg(last = true)]
        title_args: Vec<String>,
    },
    #[cfg(windows)]
    /// Show the resolved configuration and check it.
    Config,
}

#[cfg(windows)]
#[derive(Clone, Copy, ValueEnum)]
enum Unimplemented {
    Trap,
    Zero,
}

#[cfg(windows)]
#[derive(Clone, Copy, ValueEnum)]
enum Console {
    SeriesX,
    SeriesS,
}

fn main() -> Result<()> {
    #[cfg(windows)]
    victory_core::logging::init("info");
    // The audio recorder `run` starts for `VICTORY_AUDIO_CAPTURE`.
    #[cfg(all(windows, feature = "diagnostics"))]
    if victory_hle::modules::audio::capture::child() {
        return Ok(());
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect {
            file,
            imports,
            exports,
        } => inspect::run(&file, imports, exports),
        #[cfg(windows)]
        Command::Census {
            paths,
            strict,
            missing,
        } => census::run(paths, strict, missing),
        Command::LibExports { lib, list } => inspect::lib_exports(&lib, list),
        Command::ShaderInfo {
            paths,
            verbose,
            disasm,
        } => shader::run(paths, verbose || disasm, disasm),
        Command::Translate {
            paths,
            validate,
            pipelines,
            list,
            jobs,
        } => translate::run(paths, validate, pipelines, list, jobs),
        #[cfg(target_os = "macos")]
        Command::ConvertMetal {
            input,
            output,
            library,
            stage,
        } => metal::convert(&input, &output, &library, stage),
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        Command::NativeFixture { exe, dll, execute, file_root, save_root } => native_fixture::run(&exe, &dll, execute, file_root.as_deref(), save_root.as_deref()),
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        Command::NativeCensus { file, file_root, save_root } => native_census::run(&file, file_root.as_deref(), save_root.as_deref()),
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        Command::WineRun { exe, wine, winepath, metal_host, converter, dll_dir, wine_prefix, allow_unverified_dll_search, provision_gameinput, title_root, save_root, storage_drive, guest_args } => {
            let code = wine_launcher::run(&exe, &wine, &winepath, &metal_host, &converter, &dll_dir, &wine_prefix, allow_unverified_dll_search, provision_gameinput, title_root.as_deref(), save_root.as_deref(), storage_drive, &guest_args)?;
            if code != 0 { std::process::exit(code); }
            Ok(())
        }
        #[cfg(windows)]
        Command::Run {
            exe,
            unimplemented,
            strict,
            execute,
            console,
            title_args: _,
        } => run::run(&exe, unimplemented, strict, execute, console),
        #[cfg(windows)]
        Command::Config => show_config(),
    }
}

#[cfg(windows)]
fn show_config() -> Result<()> {
    let cfg = victory_core::Config::load()?;
    let status = |p: &Option<PathBuf>| match p {
        Some(p) if p.exists() => format!("{}", p.display()),
        Some(p) => format!("{}  (does not exist)", p.display()),
        None => "(not set)".to_owned(),
    };
    println!(
        "config file : {}",
        cfg.source
            .as_ref()
            .map_or("(none)".into(), |p| p.display().to_string())
    );
    println!("game_dir    : {}", status(&cfg.game_dir));
    println!("cache_dir   : {}", cfg.cache_dir.display());
    for t in &cfg.test_titles {
        println!("test_title  : {}", status(&Some(t.clone())));
    }
    for p in cfg
        .game_dir
        .iter()
        .chain(&cfg.test_titles)
        .chain([&cfg.cache_dir])
    {
        if let Err(e) = victory_core::ensure_outside_repo(p) {
            println!("POLICY VIOLATION: {e}");
        }
    }
    Ok(())
}

/// Paths given on the command line, or the configured game + test titles.
pub(crate) fn default_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if !paths.is_empty() {
        return Ok(paths);
    }
    let cfg = victory_core::Config::load()?;
    let all: Vec<PathBuf> = cfg.game_dir.into_iter().chain(cfg.test_titles).collect();
    anyhow::ensure!(
        !all.is_empty(),
        "no paths given and none configured (see victory.example.toml)"
    );
    Ok(all)
}
