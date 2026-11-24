# VIctory

Runs Grand Theft Auto VI (Xbox Series X|S) natively on Windows, from files
the user supplies.

The title's x86-64 code runs unchanged inside our process; VIctory supplies
the platform underneath it and converts the title's data, ahead of time where
it can and on first use where it must (shaders). See
[docs/architecture.md](docs/architecture.md).

## Status (2026-11-23)

**Not playable yet.** The title boots, runs its menus at its own 30 fps and
loads the world, where rendering fails.

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

What each part does today and what it still needs:

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

## Offline inspection on macOS

The native macOS CLI supports read-only PE32+ x86-64 and COFF inspection,
shader-container analysis and GFX10 disassembly, and offline translation
to DXIL. It does not load or execute titles, provide Xbox runtime services,
or implement a Metal renderer. These commands also work on Windows.
Use the pinned Rust toolchain and build only the CLI package:

```sh
cargo build -p victory
target/debug/victory inspect /path/to/title.exe --imports --exports
target/debug/victory lib-exports /path/to/import.lib --list
target/debug/victory shader-info /path/to/shaders --disasm
target/debug/victory translate /path/to/shaders --list
cargo test -p victory -p victory-pe
```

Translation reports whether DXIL was emitted; this does not validate the
output or establish that it can run on macOS. `--validate`, `--pipelines`,
and `VICTORY_PAIRS` require Windows. Shader inputs may be raw DXBC containers
or SGD5 packed records, including those found inside otherwise uncompressed
files. Compressed or encrypted archives are not generally unpacked.

On macOS, an independently obtained Apple Metal Shader Converter dynamic
library can convert an emitted DXIL file to a `.metallib`:

```sh
VICTORY_EMIT_DUMP=/path/outside/repo target/debug/victory translate /path/to/shader --list
target/debug/victory convert-metal /path/outside/repo/emitted.dxil \
  --library /path/to/libmetalirconverter.dylib --stage compute \
  --output /path/outside/repo/shader.metallib
```

Use the input's actual stage (`vertex`, `fragment`, `mesh`, or `compute`).
The converter is proprietary and not bundled; obtain it and comply with
[Apple's terms](https://developer.apple.com/metal/shader-converter/).
Conversion alone is not validation of shader behavior or resource bindings,
and this command does not create a Metal renderer. To exercise the converter
integration test, set `VICTORY_TEST_METAL_CONVERTER` to the dylib path and run
`cargo test -p victory --test shader_offline`.

An opt-in GPU test requires a Metal device with argument buffers tier 2,
Xcode command-line tools and the converter library:

```sh
VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory --test metal_compute -- --ignored
```

It emits DXIL from a synthetic one-thread GFX10 shader, converts it, checks
the converter's resource reflection, dispatches it on Metal and verifies
`41 + 1` is written to a buffer without modifying its other words. This is
one synthetic compute case, not a game compatibility or renderer test.

The macOS `victory-gpu` command objects have an experimental synchronous
compute path: a recorded console dispatch translates the native shader,
loads it through Apple's converter, and executes it on Metal when its
command list is submitted. Compute supports a single shader-visible view
heap, a whole-buffer UAV at descriptor index zero, root constants, recorded
buffer-to-buffer region copies, unsplit UAV barriers, and whole-buffer
transitions among `COMMON`, `COPY_SOURCE`, `COPY_DEST` and
`UNORDERED_ACCESS`. Only committed default-heap buffers on matching direct or
compute queues and lists are supported; UAV access requires the resource's
unordered-access flag. The path validates command use and transition
before-states in list order and again at submission. Buffers can be promoted
from `COMMON` on use; a transition from `COMMON` may also name a supported
promotable before-state. Buffers decay to `COMMON` after the entire
`ExecuteCommandLists` batch, not between lists, following the
[D3D12 promotion and decay rules](https://learn.microsoft.com/en-us/windows/win32/direct3d12/using-resource-barriers-to-synchronize-resource-states-in-direct3d-12#implicit-state-transitions).

Copies use host memory at submission; dispatches finish before the next command,
so GPU writes are visible to later copies and barriers. Copy ranges must be in
bounds and non-overlapping, and source and destination must be distinct.
Commands retain their resources until the list is reset or released. Placed or
aliased resources, other heap types and state combinations, individual
subresources, split or aliasing barriers, other shader bindings, descriptor
changes after recording, root-signature commands and waits are not implemented.
Unsupported list commands reject the list; submission
failures latch the queue and prevent later fence signals.
The caller must not access mapped resources concurrently with submission.
This experimental path is not wired into a macOS title loader and is not
an Xbox GPU backend.

An opt-in native macOS `PresentX` path sends a single RGBA8/BGRA8 color
resource to a separate authenticated helper with a real AppKit window and
`CAMetalLayer`. It waits for Metal completion and preserves the source
resource. The Wine proxy supports RGBA8 presentation; both paths reject
unsupported planes and parameters. This is an experimental synthetic-fixture
path, not a general swap chain or game compatibility. See
[the native loader notes](docs/native-macos-loader.md) for setup and limits.

To run the synthetic device/list/queue test on a Metal GPU (no title assets):

```sh
VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-gpu console_compute_records_and_submits_on_metal -- --ignored
```

It records copy → transition → compute → transition → copy and checks unchanged
pre-submit data, the post-submit value 42, surrounding bytes, resource lifetimes,
batch-wide state tracking and decay, invalid transitions and queue failure.

The same queue can execute a narrowly constrained offscreen draw: a validated
graphics pipeline stream with a resource-free translated vertex shader and a
translated pixel shader using at most one graphics root-constants parameter,
one SRV table and one sampler table. Used root dwords are snapshotted per draw
and uploaded as a real fragment constant buffer at `b0, space0`; Apple's
converter reflection must confirm the expected automatic layout. This is not
support for arbitrary console root CBVs or constant-buffer descriptors.
Draws use one float4 vertex buffer,
triangle-list topology, full-target viewport and scissor, and one committed
linear single-sample RGBA8 render target. Whole-target readback to a buffer
uses a 256-byte row-pitched footprint after an explicit `RENDER_TARGET` to
`COPY_SOURCE` transition. Drawing preserves pixels outside the triangles; GPU
work and host-memory copies complete in command order. Other graphics states,
formats, depth, blending, instancing and presentation are rejected.

`ClearRenderTargetView` records ordered full-target or clipped rectangle clears
for the same RGBA8 targets, independent of the bound draw target. It snapshots
the color and rectangles, retains the target, requires `RENDER_TARGET` state,
and clears the linear host backing synchronously between GPU draws and copies.
FLOAT channels convert to UNORM by clamping to [0, 1] and rounding to nearest
with halfway values up; NaN becomes zero. Null rectangle arrays with a nonzero
count and inverted rectangles are rejected.

```sh
VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-gpu console_triangle_draw_preserves_target_and_copies_to_padded_buffer -- --ignored
```

The test uses a synthetic 5×4 target and two draws with different root values
to check clear/draw/clear ordering, clipping, color conversion, rejected states,
converter reflection, mapped row pitch, readback padding and resource lifetime.

For texturing, the SRV must be a default view of a committed default-heap,
linear, single-mip, single-sample RGBA8 2D texture. The shader's statically
traced SRV and sampler tables must each address descriptor index zero in
separate owned shader-visible heaps. Supported samplers are point or linear
filtering with clamp-to-edge, zero LOD range and bias, and no comparison.
Changing heaps invalidates descriptor-table bindings; rebind them before a draw.
Recorded draws retain their resources and heaps, snapshot descriptor identity,
and reject descriptor changes before execution; texture contents are read at
submission. The converter is configured to use its texture-array compatibility
ABI. Other texture layouts, dynamic indices and sampler modes are rejected.

```sh
VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-gpu console_samples_recorded_texture_and_sampler_tables -- --ignored
```

This synthetic test checks nearest and linear GPU samples, switching to an
updated second texture, descriptor/resource lifetime and padded readback.

The macOS-only `victory-metal::Renderer` builds a persistent offscreen Metal
pipeline from converted vertex and fragment metallibs. It draws triangle
lists from a float4 vertex buffer into RGBA8 and reads the pixels back;
device, pipeline, target and buffers are reused across synchronous draws.
Its fixed input layout supports one per-vertex float4, optional fragment
`b0, space0` constants, and one fragment texture and sampler at heap index 0.
Specify the constant buffer size in dwords (zero or a multiple of four) and
supply that exact number per draw. If texture binding is enabled, upload a
tightly packed RGBA8 image with `set_fragment_texture` before drawing; nearest
or linear filtering uses clamp-to-edge with one mip level.

The caller must verify from converter reflection that b0 uses the automatic
linear layout at offset zero and that the constant buffer is large enough, and
from translated shader metadata that texture and sampler accesses use heap
index zero. The fragment metallib must expect a 2D-array texture view (the
converter's older default or array compatibility mode). Other layouts and
resource types are not supported; the renderer does not infer or validate
them from metallibs. Console GPU semantics and presentation are not
implemented. To exercise synthetic translated GFX10 vertex and fragment
shaders, including changing constants and sampled texels:

```sh
cargo build -p victory
VICTORY_TEST_CLI="$(pwd)/target/debug/victory" \
VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib \
  cargo test -p victory-metal --test render -- --ignored
```

Input files are supplied by the user; see [the asset policy](docs/asset-policy.md).

## Getting started

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
