# VIctory

An experimental macOS fork of VIctory, the Xbox Series X|S compatibility
layer for user-supplied titles. The fork's macOS paths have been exercised
with synthetic fixtures, not GTA VI.

The Windows base runs x86-64 title code in its process and supplies the
platform underneath it. See [docs/architecture.md](docs/architecture.md).

## Status

**No game boot or upstream parity is established on macOS.** The fork has
experimental PE mapping and Wine-to-native-Metal paths; its documented
end-to-end results use synthetic guests. On Apple silicon the Wine route runs
x86-64 code under Rosetta, not native ARM guest code. Neither route is a sandbox.

The current upstream benchmark at
[`ccfb02996eac831853e8324156f123eeff083e9d`](https://github.com/Marvisak/VIctory/tree/ccfb02996eac831853e8324156f123eeff083e9d)
reports menus and audio, and a visible world with streets, buildings,
vehicles and characters at 1–2 seconds per frame. It remains unplayable:
surfaces are unlit, geometry is missing and some characters are broken.
That is upstream's Windows result, not a result of this fork on macOS.

### Historical Windows base (2026-11-23)

At the fork's upstream base, the title was reported to boot, run menus at
30 fps and load the world, where rendering failed:

| Stage | State |
|-------|-------|
| Boot, first-boot screens (accessibility, language, settings, legal) | works, 30 fps |
| Intro video with sound | works |
| Start screen, profile and saves | works |
| Loading into the world (about 50 s) | works |
| The world (first mission's intro) | reached and runs with indirect draws; its title card renders, the scene after it is still dark; 0.5-1.8 s per frame |

Tested on one machine: Windows 11, AMD RX 6700 XT.

> **Warning.** Entering the world on a real GPU can currently hang it, and
> Windows then resets the display driver (every monitor goes black for a
> moment). To look further, use a diagnostics build with
> `VICTORY_D3D12_WARP=1`, which renders on the software adapter.

The following table records that historical Windows base, not macOS support:

| Subsystem | Today | Needs |
|-----------|-------|-------|
| Loader | Runs the title's executable and its DLLs in our process; crash reports name the faulting code and are kept in `crashes.log` | – |
| Runtime services | One signed-in offline user, licences, packages, saves that survive a kill at any step, text entry dialog | Speech is silent; the dialog and relaunch are untested live |
| Network | Offline by design: the title cannot reach the network | – |
| Files and streaming | Archives and streamed data read, zlib and BCPack decoded in parallel | Loading is limited by writes into title memory |
| Input | XInput pads, keyboard, keyboard stand-in pad | Trigger rumble |
| Audio | Menu, video and loading sound; XMA voices and Opus streams decode | Reverb is silent; Opus results unverified against the title |
| GPU: menus | Draws, compute and presentation on host D3D12; only changed title memory is copied | Frame pacing drifts on one screen in release builds |
| GPU: world | Mesh pipelines, shader-written buffers, descriptor lifetimes, indirect execution (arguments clamped), pipelines made by worker threads and kept on disk | Most of the scene is still missing (untranslated shaders drop draws); frames take seconds; real queries, depth bounds, HDR and scaling are missing |
| Shaders | Console GPU code recompiled to DXIL at first use, cached on disk. Menus: all. World: mesh 8,191 of 8,506, vertex/pixel 10,617 of 12,346, compute 1,643 of 2,539 | 64-bit image atomics, merged geometry stages, the ray-tracing builder's shaders; some compute programs need a GPU with 64-lane waves |
| Textures | Console tiled layouts and detiling for the modes the title uses | Render-target metadata planes |
| Series S mode | Runs through the menus to the loading screen | The world, as above |

## macOS: inspect first

For a locally supplied PE, the shortest read-only import inspection from the
repository root is:

```sh
cargo run -p victory -- inspect /path/to/title.exe --imports
```

It reads the file without executing it; it does not establish whether the
title can run. Input files must be supplied by the user; see
[the asset policy](docs/asset-policy.md).

The CLI also inspects import libraries and shader containers, disassembles
GFX10 instructions, and translates shaders to DXIL:

```sh
cargo run -p victory -- lib-exports /path/to/import.lib --list
cargo run -p victory -- shader-info /path/to/shaders --disasm
cargo run -p victory -- translate /path/to/shaders --list
```

Translation reports DXIL emission, not validated behavior. `--validate`,
`--pipelines` and `VICTORY_PAIRS` require Windows. Inputs may contain raw
DXBC containers or SGD5 records; compressed or encrypted archives are not
generally unpacked.

### Offline DXIL to Metal

With the pinned Rust toolchain and a separately supplied Apple Metal Shader
Converter dynamic library, emitted DXIL can be converted to a `.metallib`.
The library is proprietary and not bundled; use it under
[Apple's terms](https://developer.apple.com/metal/shader-converter/).
Output paths must be outside the repository.

```sh
cargo build -p victory
VICTORY_EMIT_DUMP=/path/outside/repo target/debug/victory translate /path/to/shader --list
target/debug/victory convert-metal /path/outside/repo/emitted.dxil \
  --library /path/to/libmetalirconverter.dylib --stage compute \
  --output /path/outside/repo/shader.metallib
```

Use the emitted DXIL file's actual path and stage (`vertex`, `fragment`,
`mesh` or `compute`). Conversion alone does not validate shader behavior or
resource bindings. The converter integration test and an opt-in synthetic
Metal compute test can be run without title assets:

```sh
VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory --test shader_offline
VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory --test metal_compute -- --ignored
```

The GPU test additionally requires Xcode command-line tools and a Metal
device with argument buffers tier 2. It verifies a synthetic compute dispatch
and readback; it is not a game or renderer compatibility test.

### Experimental execution paths

The fork also has restricted Metal compute, graphics and presentation paths
verified with synthetic fixtures, not a complete Xbox GPU implementation.
For the native `victory-gpu` path, these opt-in synthetic tests exercise
ordered compute and an offscreen draw with GPU readback:

```sh
VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-gpu console_compute_records_and_submits_on_metal -- --ignored
VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-gpu console_triangle_draw_preserves_target_and_copies_to_padded_buffer -- --ignored
```

The standalone `victory-metal::Renderer` also has a synthetic translated
vertex/fragment test:

```sh
cargo build -p victory
VICTORY_TEST_CLI="$(pwd)/target/debug/victory" \
VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-metal --test render -- --ignored
```

These are separate, restricted synthetic paths; their supported commands,
formats and bindings are not a claim about arbitrary title workloads.
The [native loader notes](docs/native-macos-loader.md) describe the x86-64
macOS PE mapper and its limitations; the
[Wine loader notes](docs/wine-loader-tests.md) describe normal Wine loading,
the Metal bridge, provider and DirectStorage gaps, and trusted-guest
requirements. Their x86-64 macOS results on Apple silicon use Rosetta;
they do not establish native ARM guest execution or game compatibility.

## Windows getting started

Requirements: Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with
shader model 6.6 (and mesh shaders for the world), Rust (pinned in
`rust-toolchain.toml`).

The title also needs several GiB of available Windows commit at startup.
Check **Committed** under **Task Manager → Performance → Memory**. If
`crashes.log` reports Windows error 1455, close other applications or increase
the Windows paging file size, then retry.

```powershell
cargo xtask install-hooks                # asset-policy check on every commit
copy victory.example.toml victory.toml   # then set your paths (outside the repo)
cargo test --workspace                   # needs no game data
cargo run -p victory -- config           # check your paths
```

Run the title:

```powershell
cargo build --release -p victory
target\release\victory.exe run <game dir>\<the title's exe> --execute
```

The window caption shows the framerate. Saves, settings and `crashes.log`
are kept under the data folder (default `%LOCALAPPDATA%\victory\data`), derived
data under the cache folder.

Input: XInput pads. With none connected the keyboard is a pad while the
title's window has focus:

| Keys | Pad |
|------|-----|
| W A S D / I J K L | left / right stick |
| arrows | D-pad |
| Enter, Backspace, X, Y | A, B, X, Y |
| Q / E, Z / C | shoulders, triggers |
| F / H | stick clicks |
| Esc, Tab | Menu, View |

## Commands

```
victory inspect <exe|dll> [--imports] [--exports]
victory census [paths...] [--missing] [--strict]
victory lib-exports <import.lib> [--list]
victory shader-info [paths...] [--verbose] [--disasm]
victory translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
victory run <exe> [--execute] [--unimplemented trap|zero] [--strict]
            [--console series-x|series-s] [-- <title arguments>]
victory config
```

Logging: `VICTORY_LOG=victory_loader=trace,info` (a release build logs
info and above).

## Debugging

A release build has no debugging aids. Build them in with

```powershell
cargo build --profile diag -p victory --features diagnostics   # target\diag\victory.exe
```

| Variable | Effect |
|----------|--------|
| `VICTORY_LOG=<target>=debug,...` | debug and trace output |
| `VICTORY_STACKS=20,60` | every thread's stack at those seconds |
| `VICTORY_WATCH=<rva>,...` | logs a title u32 when it changes |
| `VICTORY_KEYS=30:Enter,35:X` | presses keys at those seconds, focus or not |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | saves title memory to the cache folder on a crash |
| `VICTORY_CAPTURE=<dir>` | saves presented frames (powers of two, or `VICTORY_CAPTURE_FRAMES=<list>`) |
| `VICTORY_DUMP_TARGETS=1` | with the two above, saves every render target of those frames |
| `VICTORY_D3D12_DEBUG=1` | host debug layer |
| `VICTORY_D3D12_GBV=1` | with the above, GPU-based validation (bad descriptor use reported, not faulted; slow) |
| `VICTORY_D3D12_DRED=1` | on a GPU failure, the allocation hit and the commands in flight |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | omit draws and dispatches, including indirect ones, whose pipeline contains a listed 16-digit shader hash |
| `VICTORY_GPU_TIMING=1` | with the above, each list of a submission run and timed on its own; slow lists logged with their slowest commands |
| `VICTORY_D3D12_WARP=1` | host device on the software adapter (slow; cannot reset the driver) |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | translated shader loops stop after `n` iterations per lane instead of 2^20 (own shader cache entries); if a lower `n` makes world frames much faster, loops hitting the limit are what makes them slow |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | records what the title plays (our process only) |
| `VICTORY_XMA_DUMP=<dir>` | saves the first 32 XMA voices' context and input |

Every build appends crashes and host GPU failures to `crashes.log` in the
title's data folder.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
