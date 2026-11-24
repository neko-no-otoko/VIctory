# Synthetic loader tests under Wine on macOS

These tests use our synthetic fixture, not game data. Wine runs a Windows
x86-64 build through a compatibility layer; this is **not native macOS
execution or a Metal backend**, and passing these tests does not establish
that a game runs or renders.

Prerequisites: Wine, the pinned Rust toolchain, and an LLVM-MinGW UCRT
cross-toolchain for macOS. The GNU-target delay-load flag requires LLVM's
linker; a different MinGW linker may not support it. Install the Rust target
`x86_64-pc-windows-gnullvm`. The commands below assume the toolchain is
extracted to `$LLVM_MINGW`, and run from the repository root:

```sh
export LLVM_MINGW=/path/to/llvm-mingw
export PATH="$LLVM_MINGW/bin:$PATH"
rustup target add x86_64-pc-windows-gnullvm
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUNNER=wine
# Wine requires the prefix's parent to be owned by the current user.
mkdir -p /tmp/victory-wine
export WINEPREFIX=/tmp/victory-wine/prefix
export WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0
# LLVM-MinGW's runtime DLL must be visible to the Windows process.
export WINEPATH="$(winepath -w "$LLVM_MINGW/x86_64-w64-mingw32/bin")"
export CARGO_TARGET_DIR=/tmp/victory-wine-target
cargo build --manifest-path tests/fixtures/hle-probe/Cargo.toml \
  --target x86_64-pc-windows-gnullvm
export VICTORY_HLE_PROBE_DLL="$(winepath -w "$CARGO_TARGET_DIR/x86_64-pc-windows-gnullvm/debug/hle_probe.dll")"
cargo test -p victory-loader --test load_probe \
  --target x86_64-pc-windows-gnullvm
```

The environment variable supplies the actual prebuilt fixture to the test
process, avoiding a nested Windows-side Cargo build. Without it the existing
native Windows test builds the fixture as before. The test still verifies the
fixture's imports and delay imports and exercises the loader's real mapping,
binding, TLS and thread behavior.

## Synthetic D3D12 capability probe

The optional diagnostic creates D3D12 devices and queries capabilities; it
does not submit rendering work or use game data. With the Wine and
LLVM-MinGW setup above, run from the repository root:

```sh
x86_64-w64-mingw32-clang -O2 -Wall -Wextra tools/d3d12-probe.c \
  -o /tmp/victory-wine/d3d12-probe.exe -ld3d12 -ldxgi -ldxguid -lole32
wine /tmp/victory-wine/d3d12-probe.exe
```

Exit status zero means the default D3D12 device was created; individual
capability-query failures are shown as HRESULTs. A successful device creation
does not prove rendering, game execution, or Metal support. Adapter names and
IDs may be supplied by Wine rather than describe the physical GPU.

In one test on an Apple M3 Max with Wine 11.0 and MoltenVK 1.4.1, the
default device was created, but the shader-model query returned `0x60`
(6.0) and the mesh-shader tier was 0. These are observations of that Wine
configuration, **not limits of macOS or the hardware**. VIctory's Windows
requirements include shader model 6.6 and mesh shaders for the world.

## Isolated MetalVulkan experiment

[MetalVulkan v7.0](https://github.com/metalsharp/MetalVulkan/releases/tag/v7.0)
provides a paired VKD3D-Proton and modified MoltenVK stack for x86-64 Wine.
The commands below install it into a **new dedicated prefix**: the installer
replaces DLLs and sets overrides within that prefix. They require Wine,
`zstd`, `curl`, and the LLVM-MinGW setup above. Run from the repository root:

```sh
mkdir -p /tmp/victory-metalvulkan
cd /tmp/victory-metalvulkan
for f in moltenvk-v7.0-universal.tar.zst vkd3d-proton-v7.0-x86_64.tar.zst; do
  curl -fLO "https://github.com/metalsharp/MetalVulkan/releases/download/v7.0/$f"
  tar --zstd -xf "$f"
done
WINEPREFIX="$PWD/prefix" WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0 wineboot -u
./vkd3d-proton-v7.0-x86_64/install-x86_64-wine.sh \
  --prefix "$PWD/prefix" --wine "$(command -v wine)" --packages "$PWD"
WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0 \
  "$PWD/prefix/vkd3d-proton-macos/wine" /tmp/victory-wine/d3d12-probe.exe
```

Build `/tmp/victory-wine/d3d12-probe.exe` using the preceding section first.
On an M3 Max with macOS 27.3 and Wine 11.0, this configuration reported
shader model `0x66` (the probe requests 6.6) and mesh tier `10` (tier 1);
the original Wine configuration reported 6.0 and tier 0. These are capability
queries, not proof that every feature works.

To run the tracked synthetic loader tests in this prefix, return to the
repository root after the earlier Rust and LLVM-MinGW setup and run:

```sh
export WINEPREFIX=/tmp/victory-metalvulkan/prefix
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUNNER="$WINEPREFIX/vkd3d-proton-macos/wine"
export WINEPATH="$(winepath -w "$LLVM_MINGW/x86_64-w64-mingw32/bin")"
cargo build --manifest-path tests/fixtures/hle-probe/Cargo.toml \
  --target x86_64-pc-windows-gnullvm
export VICTORY_HLE_PROBE_DLL="$(winepath -w "$CARGO_TARGET_DIR/x86_64-pc-windows-gnullvm/debug/hle_probe.dll")"
cargo test -p victory-loader --test load_probe \
  --target x86_64-pc-windows-gnullvm
```

### Synthetic CLI execution

This builds our own PE for an ignored Windows integration test, with no game
data. After the setup above, from the repository root:

```sh
mkdir -p /tmp/victory-cli-fixture
printf 'LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle\nWriteFile\n' > /tmp/victory-cli-fixture/kernel32.def
printf 'LIBRARY XGameRuntime.dll\nEXPORTS\nXGameRuntimeInitialize\n' > /tmp/victory-cli-fixture/xgameruntime.def
llvm-dlltool -m i386:x86-64 -d /tmp/victory-cli-fixture/kernel32.def \
  -l /tmp/victory-cli-fixture/kernel32.lib
llvm-dlltool -m i386:x86-64 -d /tmp/victory-cli-fixture/xgameruntime.def \
  -l /tmp/victory-cli-fixture/xgameruntime.lib
x86_64-w64-mingw32-clang -O2 -c tests/fixtures/windows_cli_probe.c \
  -o /tmp/victory-cli-fixture/probe.obj
lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib \
  /out:/tmp/victory-cli-fixture/probe.exe /tmp/victory-cli-fixture/probe.obj \
  /tmp/victory-cli-fixture/kernel32.lib /tmp/victory-cli-fixture/xgameruntime.lib
export VICTORY_TEST_CLI_PE="$(winepath -w /tmp/victory-cli-fixture/probe.exe)"
cargo test -p victory --test windows_run --target x86_64-pc-windows-gnullvm \
  executes_synthetic_pe_through_cli -- --ignored --nocapture
```

Verified with Rust 1.98.1, LLVM-MinGW 20261117 UCRT, Wine 11.0 and
MetalVulkan v7.0 on an M3 Max. The test expects one HLE import and no missing
imports; the guest checks that `XGameRuntimeInitialize` returned `S_OK`,
prints `VIctory synthetic entry executed`, and returns 42 to the CLI.
It uses temporary cache and data folders.
This validates only a small synthetic PE; it does not run a title or exercise
the GPU, and Wine is neither native macOS execution nor a native Metal backend.

### Synthetic D3D12.X object path

After the setup and `kernel32.lib` creation above, build the separate fixture
and run its ignored test:

```sh
printf 'LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n' > /tmp/victory-cli-fixture/d3d12_xs.def
llvm-dlltool -m i386:x86-64 -d /tmp/victory-cli-fixture/d3d12_xs.def \
  -l /tmp/victory-cli-fixture/d3d12_xs.lib
x86_64-w64-mingw32-clang -O2 -c tests/fixtures/windows_d3d12x_probe.c \
  -o /tmp/victory-cli-fixture/d3d12x.obj
lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib \
  /out:/tmp/victory-cli-fixture/d3d12x.exe /tmp/victory-cli-fixture/d3d12x.obj \
  /tmp/victory-cli-fixture/kernel32.lib /tmp/victory-cli-fixture/d3d12_xs.lib
export VICTORY_TEST_D3D12X_PE="$(winepath -w /tmp/victory-cli-fixture/d3d12x.exe)"
cargo test -p victory --test windows_run --target x86_64-pc-windows-gnullvm \
  executes_synthetic_d3d12x_through_cli -- --ignored --nocapture
```

The guest calls VIctory's `D3D12XboxCreateDevice` HLE, creates a queue,
allocator, command list and fence, submits an empty list, and checks a
fence value. The test requires the host D3D12 device to initialize. The
fence operation is implemented in HLE; the empty list performs no rendering,
compute, or GPU readback and does not establish title compatibility.

### Synthetic D3D12.X compute and readback

With the import libraries created above, compile the separate self-authored
compute fixture and run its ignored test:

```sh
x86_64-w64-mingw32-clang -O2 -c tests/fixtures/windows_d3d12x_compute.c \
  -o /tmp/victory-cli-fixture/compute.obj
lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib \
  /out:/tmp/victory-cli-fixture/compute.exe /tmp/victory-cli-fixture/compute.obj \
  /tmp/victory-cli-fixture/kernel32.lib /tmp/victory-cli-fixture/d3d12_xs.lib
export VICTORY_TEST_D3D12X_COMPUTE_PE="$(winepath -w /tmp/victory-cli-fixture/compute.exe)"
cargo test -p victory --test windows_run --target x86_64-pc-windows-gnullvm \
  executes_synthetic_d3d12x_compute_through_cli -- --ignored --nocapture
```

The guest creates a buffer and an explicitly described raw UAV, binds a
self-authored console shader, and dispatches through VIctory's D3D12.X HLE.
The test requires shader translation and a host pipeline, then checks that
the GPU-written value is copied back into guest memory as 42 (from initial
value 7). On the M3 Max/Wine/MetalVulkan configuration above it passed.
This is one synthetic compute workload, not game or native Metal support.

In that same prefix, the synthetic loader integration tests passed 2/2. A
separate GPU compute experiment compiled self-authored HLSL with Microsoft
DXC v1.9.2609 and used an adapted MetalVulkan compute probe: after dispatch,
UAV readback was `42, 7, 7, 7` from a buffer initialized to 7. The adapted
probe is not included in this repository. A VIctory-emitted synthetic
compute DXIL shader also produced `42, 7, 7, 7` after its root signature was
corrected to use eight `b0` constants and direct resource-heap indexing.
Both shaders passed Microsoft's `dxv`. An earlier attempt at the VIctory
shader failed pipeline creation with `0x80070057` using a root signature
with only five constants and no direct-heap flag; it did not establish an
emitter defect. These experiments do not establish game compatibility or a
native Metal backend.
The adapted source and HLSL remain on the test workstation at
`/tmp/victory-metalvulkan/compute_matrix.c` and `compute.hlsl`; its original
source is [MetalVulkan's compute probe](https://github.com/metalsharp/MetalVulkan/blob/v7.0/scripts/probes/core10/compute_matrix.c).

The [MetalVulkan license inventory](https://github.com/metalsharp/MetalVulkan#license)
lists AGPL-3.0 for its repository, LGPL-2.1 for VKD3D-Proton, Apache-2.0 for
MoltenVK, and a separately licensed proprietary Apple Metal Shader Converter
library in the release. Review its packaged agreement before use or
redistribution. Microsoft DXC is distributed under its
[license](https://github.com/microsoft/DirectXShaderCompiler/blob/main/LICENSE.TXT).
No third-party binaries are included in VIctory.

## Experimental Wine-to-native-Metal pipe

An isolated prototype runs a normal x86-64 PE process under Wine and connects
its test DLL over private inherited standard-input/output pipes to the
`wine_metal_bridge` macOS example. The DLL sends bounded, synchronous RGBA8
clear requests; the host uses `victory-metal` and returns the actual Metal
readback. The channel is reserved for this protocol and cannot carry guest
console output. An invalid frame or Metal failure terminates the bridge.
This is a trusted-fixture experiment, not a sandbox or an Xbox graphics API.

The ignored integration test builds the self-authored PE and DLL with
LLVM-MinGW, uses an isolated Wine prefix, and verifies two Windows threads'
compiler-generated static TLS, TLS callbacks, and independently colored Metal
readbacks. On the M3 Max with Wine 11 under Rosetta it passed:

```sh
WINE=/opt/homebrew/bin/wine \
cargo test -p victory-metal --target x86_64-apple-darwin \
  --test wine_bridge wine_guest_tls_and_metal_readback -- --ignored --nocapture
```

The LLVM-MinGW tools must be on `PATH`; a working macOS Metal device is
required. Wine is responsible for the PE process, TLS and Windows thread
environment; its own macOS/Rosetta transitions are runtime implementation
details, not a stable macOS GS API. This proof does **not** use VIctory's
Xbox HLE, the VIctory Windows loader, game shaders or presentation, and it
does not establish game compatibility or ARM-native PE execution.

## Experimental Wine D3D12.X-to-Metal proxy

The opt-in proxy keeps D3D12.X HLE objects and vtables inside the Wine PE
process. It copies bounded, ordered resource and command data over
authenticated loopback TCP to a separate x86-64 macOS host, which uses
VIctory's Metal paths and returns each completed request's results. No
guest pointers or COM objects cross the process boundary. Wine and the native
host both execute under Rosetta on Apple Silicon; this is not ARM-native guest
execution. This is a trusted-workload experiment, not a process sandbox.

The restricted compute path supports one whole-buffer raw UAV with an
optional root CBV, or a bounded table of raw SRV/UAV buffers. The latter
accepts up to 16 populated entries, a nonzero table base and one table root
slot 1–63; root constants use slot 0. The optional CBV uses slot 2–63 and
is not combined with the multi-view path.
A pending queue wait can defer the work as described below. Limits are 4 MiB
per shader, 16 MiB per buffer and 64 constant dwords. Restricted graphics and
opt-in presentation paths are described below. Root SRV/UAV descriptors,
general resource states and arbitrary D3D12.X methods remain unsupported;
some unsupported void methods abort rather than pretend to succeed. The
loopback bearer token is not a boundary against other processes with access
to it.

The compute CBV must address a live committed default-heap buffer in COMMON
or VERTEX_AND_CONSTANT_BUFFER state, at a 16-byte-aligned offset; the view
extends to the allocation end or 64 KiB, whichever is smaller. It cannot
alias the dispatch UAV. The translated shader's CBV slot must match the bound
slot, and only the native path's validated unbounded console descriptor and
static-offset scalar loads are supported. A recorded dispatch retains the
buffer and offset and snapshots current bytes at execution, including after
a deferred queue wait; no guest pointer crosses the transport. Missing,
misaligned, undersized, aliased, state-invalid and mismatched bindings fail.
Protocol opcode 6 carries the CBV; opcode 1 remains the no-CBV path.

On an M3 Max with Wine 11 under Rosetta and the Apple converter, a normally
launched TLS guest used offset CBVs in two translated Metal dispatches,
released the bound resource before submission and checked changed output
after late writes. Invalid slots, ranges, resource states and root SRV/UAV
methods were rejected. With the launcher environment below, run `cargo test
--locked -p victory --target x86_64-apple-darwin --test wine_cbv -- --ignored
--nocapture`. This is a restricted compute binding path, not general D3D12.X
or game compatibility.

The raw compute table accepts SRV and UAV entries in any order, each backed
by a distinct committed default-heap buffer. Explicit R32_TYPELESS RAW views
retain checked `FirstElement` and `NumElements` ranges; UAV resources must
allow unordered access. SRVs require COMMON or NON_PIXEL_SHADER_RESOURCE
state, UAVs COMMON or UNORDERED_ACCESS. Shader resource kinds, static table
offsets and root slot are checked against the pipeline metadata and the
bound table. Descriptors and buffers are retained, descriptor mutation before
execution is rejected, and writable resources are returned after Metal
completes. Opcode 8 carries general tables; the prior two-entry opcode 7
remains supported.

Raw dword loads and stores, including dynamic scalar and per-lane byte
offsets, are guarded in translated compute shaders using the selected view's
reported byte length. Each out-of-view dword reads as zero or is discarded
on write; arithmetic overflow is treated as out of range. This is necessary:
on the Apple converter and M3 Max, an unguarded load outside an SRV view but
inside its backing allocation returned the backing byte. Do not rely on
Metal descriptor length alone to enforce D3D out-of-bounds semantics.
Indexed/structured addressing, dynamic descriptor selection and atomics
remain unsupported in this compute path.
Aliased views, structured or typed buffers, counters and mixing these
tables with root CBVs are rejected. This is pipeline-metadata binding, not
general D3D12 root-signature support. Metal uses the buffer descriptor
encoding documented by [Apple](https://github.com/apple/game-porting-toolkit/blob/main/docs/metal-shader-converter/binding-model.md).

The `wine_compute_bounds` and `wine_dynamic_bounds` fixtures exercise legacy,
paired-view and multi-view accesses using translated shaders, including
dynamic offsets, partial vectors, overflow and adjacent resource bytes.

The `wine_group_barrier` fixture exercises 64 threads in a group: each lane
loads through guarded raw views, exchanges data through group-shared memory
across a barrier, and stores through a bounded UAV. It checks divergent valid,
out-of-view and overflow offsets plus untouched adjacent bytes. This passed
on the M3 Max under Wine 11 with the Apple converter; the XOR-32 exchange
crosses SIMD groups on 32-wide hardware, but not necessarily on 64-wide
hardware. It does not establish general synchronization behavior.

On an M3 Max under Rosetta with Wine 11 and Apple's converter, a normally
launched TLS guest sampled a nonzero-offset raw SRV and wrote a nonzero-offset
raw UAV through translated Metal compute. It checked changed input after a
deferred fence wait, output and untouched bytes, retained resources, and
invalid state, range, format, counter, slot and alias cases. Run `cargo test
--locked -p victory --target x86_64-apple-darwin --test wine_raw_views --
--ignored --nocapture` with the launcher environment below.

The `wine_multi_views` test uses a normally Wine-launched TLS guest with
four independently accessed SRV/UAV entries, a nonzero table base, root
slot 7 and nonzero view offsets. On M3 Max with Wine 11 and Apple's converter,
it verifies two output buffers, retained resources, changed input after a
deferred fence wait, invalid descriptor ranges and rejection of an access
outside its view. Run `cargo test --locked -p victory --target
x86_64-apple-darwin --test wine_multi_views -- --ignored --nocapture` with
the launcher environment below.

The fixture below uses compiler-generated static TLS, two Windows threads and
TLS callbacks before dispatching the existing synthetic D3D12.X shader and
checking GPU readback `42`. The positive case and wrong-token rejection
passed together on an M3 Max with Wine 11 under Rosetta. This goes through
VIctory's existing Windows EXE-as-DLL loader, which currently redirects a
byte-matched TLS instruction sequence; this single fixture does not prove
arbitrary TLS, host signals, exceptions, or game startup are correct. The
native macOS loader's TLS and unwind rejections remain unchanged.

With the LLVM-MinGW and Wine setup above, build the fixture:

```sh
mkdir -p /tmp/victory-wine-hle-tls
printf 'LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle\nWriteFile\nCreateEventW\nCreateThread\nGetTickCount64\nSleep\nSetEvent\nWaitForSingleObject\nWaitForMultipleObjects\nGetExitCodeThread\nCloseHandle\n' > /tmp/victory-wine-hle-tls/kernel32.def
printf 'LIBRARY d3d12_xs.dll\nEXPORTS\nD3D12XboxCreateDevice\n' > /tmp/victory-wine-hle-tls/d3d12_xs.def
llvm-dlltool -m i386:x86-64 -d /tmp/victory-wine-hle-tls/kernel32.def -l /tmp/victory-wine-hle-tls/kernel32.lib
llvm-dlltool -m i386:x86-64 -d /tmp/victory-wine-hle-tls/d3d12_xs.def -l /tmp/victory-wine-hle-tls/d3d12_xs.lib
x86_64-w64-mingw32-clang -O2 -fno-emulated-tls -c tests/fixtures/windows_d3d12x_tls_compute.c -o /tmp/victory-wine-hle-tls/tls_compute.obj
lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /include:_tls_used \
  /out:/tmp/victory-wine-hle-tls/tls_compute.exe /tmp/victory-wine-hle-tls/tls_compute.obj \
  /tmp/victory-wine-hle-tls/kernel32.lib /tmp/victory-wine-hle-tls/d3d12_xs.lib \
  "$LLVM_MINGW/x86_64-w64-mingw32/lib/libmingw32.a"
```

Build the macOS service in a separate target directory, choose an unused
loopback port and a fresh secret 32-hex-digit token, then run the service in
one terminal with the Apple Metal Shader Converter dylib configured:

```sh
export VICTORY_WINE_METAL_ADDR=127.0.0.1:39471
export VICTORY_WINE_METAL_TOKEN="$(openssl rand -hex 16)"
export VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib
CARGO_TARGET_DIR=/tmp/victory-metal-host-target cargo build -p victory-gpu \
  --example wine_metal_compute_host --target x86_64-apple-darwin
/tmp/victory-metal-host-target/x86_64-apple-darwin/debug/examples/wine_metal_compute_host "$VICTORY_WINE_METAL_ADDR"
```

For a launcher that needs a race-free ephemeral port, pass
`127.0.0.1:0 --announce` instead of the fixed address. After binding and,
when enabled, initializing AppKit, the helper prints and flushes exactly one
`VICTORY_WINE_METAL_READY=127.0.0.1:<port>` line to stdout before serving.
The launcher must monitor the helper process after that line; the line alone
does not guarantee it remains alive.

In another terminal with the same address and token, run the Windows target
under Wine with its isolated prefix and the existing Windows runner setup:

```sh
export VICTORY_TEST_D3D12X_TLS_COMPUTE_PE="$(winepath -w /tmp/victory-wine-hle-tls/tls_compute.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test windows_run --target x86_64-pc-windows-gnullvm \
  executes_tls_d3d12x_compute_through_wine_metal_proxy -- --ignored --nocapture
```

Stop the service after the test. No compatible game files were used or
configured; these results are not evidence of game compatibility.

### Restricted offscreen graphics

Graphics submissions carry ID-keyed resources and pipelines and an ordered
command stream. The proxy retains guest objects referenced by recorded
commands, snapshots their data when the queue executes, and writes the
resulting bytes and resource states back after the host completes. Host GPU
work remains serialized and synchronous; an unsatisfied queue fence wait can
defer subsequent operations without blocking the calling thread.

The supported subset includes multiple full color/depth clears, buffer copies,
full texture uploads and readbacks, resource transitions, and multiple
nonindexed or 16-/32-bit indexed triangle-list draws with viewport/scissor
and pipeline changes. Committed default-heap resources are limited to
RGBA8_UNORM and typed BC1/BC3 linear or sRGB sampled textures (including
multiple mips), single-mip RGBA8_UNORM color targets, single-mip D32_FLOAT or
D32_FLOAT_S8X24_UINT depth/stencil textures, and buffers.
Graphics supports one color target and optional depth target per draw; the
native depth comparison/write subset; restricted single-target D3D blending
and color write masks; and a single sampled texture and point or linear clamp
sampler.
Vertex input layouts support up to 16 elements and 16 slots, with converter
reflection mapping semantic names and indices to Metal attributes. Supported
formats are R32G32B32A32_FLOAT, R32G32_FLOAT, R16G16_FLOAT, and R16G16_UNORM.
Offsets may be explicit or D3D12_APPEND_ALIGNED_ELEMENT; each slot has a single
stride and per-vertex or per-instance classification, and per-instance step
rates must be nonzero. All declared slots must have bounded vertex buffer views.
Unsupported formats, system-value vertex inputs, invalid layouts, and
out-of-range fetches are rejected. This is not general D3D12 vertex input
support.
Shader constants must be contiguous and use one root slot. Texture copy row pitch is
256-byte aligned and buffer offset is 512-byte aligned.

This remains restricted: no partial clears, cross-process shared fences,
other tiled layouts, BGRA or uncompressed sRGB formats through this proxy, BC2
and other compressed formats, arbitrary sampler/address modes, or
general D3D12 resource or pipeline states.
Unsupported combinations fail or trap rather than being ignored. Limits are
4 MiB per shader, 16 MiB per resource, 64 constant dwords, 4096 batch items,
and 64 MiB per request. Shader translation and drawing use the existing
native Metal path, not MetalVulkan.

The Wine depth/stencil path uses D32_FLOAT_S8X24_UINT with Metal
Depth32Float_Stencil8, not D24_UNORM_S8_UINT. The latter is rejected:
the tested M3 Max reports that its Metal device does not support D24S8.
The path supports full depth/stencil clears, 8-bit stencil reference and
read/write masks, and separate front/back D3D stencil comparisons and
operations. Plane readback uses D3D12's R32_FLOAT depth and R8_UINT stencil
footprints. Only whole-resource transitions, single-mip 2D textures, null DSV
descriptors, and clockwise front faces are supported; partial clears,
per-plane barriers, stencil SRVs and copyable-footprint queries for this
format remain unsupported. The launcher fixture tests both faces, stencil
and depth pass/fail, masks, plane readback, and invalid formats and states.

Blending supports `ADD`, `SUBTRACT`, `REV_SUBTRACT`, `MIN`, and `MAX`
independently for RGB and alpha. RGB factors are zero, one, source or
destination color or alpha and their inverses, plus source-alpha-saturate
as a source factor. Alpha supports zero, one, source or destination alpha and
their inverses. MIN/MAX compare unweighted values. Logic operations,
dual-source and dynamic blend-constant factors are rejected. When blending is
disabled its factors and operations are ignored; the write mask still applies.
The launcher-driven `wine_mips` fixture covers separate RGB/alpha operations,
factor combinations, disabled blending with a mask, and invalid states.
The packed transport retains its three legacy encodings; new states use a
tagged encoding that older hosts reject rather than misinterpret.

Linear sampled multi-mip textures use 2D `TEXTURE_LAYOUT_UNKNOWN`, one array layer,
one sample, explicit nonzero mip count and no render/depth flags. Null SRVs
cover all levels; explicit matching-format Texture2D SRVs support a validated
first mip and count, including `UINT_MAX` for the remaining levels, with default
component mapping, plane zero and zero resource LOD clamp. The existing point
or linear clamp sampler has min/max LOD zero, so sampling selects level zero
relative to the SRV's first mip; arbitrary sampler LOD is not supported.
CopyTextureRegion supports full selected mips with 512-byte buffer offsets
and 256-byte row pitches; partial regions and per-subresource barriers are
rejected. Packed mip bytes in the internal transport are not console tiling.
This incompatible mip-aware batch uses opcode 5; old opcode 3 is rejected
rather than misdecoded.

RGBA8_UNORM sampled textures additionally accept explicit console swizzles
`SW_4KB_S` (`0x105`) and `SW_64KB_D` (`0x10a`), with one array slice,
one sample, nonzero mips and no render/depth flags. Map subresource zero to
access the complete tiled allocation, including the shared mip tail; mapping
other subresources fails. The retained bytes are detiled per mip before
transport to Metal. Tiled copy operations and tiled render targets are not
supported; neither BC nor XOR layouts are accepted in this tiled path.
For compatibility with the existing Wine proxy, layout zero remains linear
and does not select the native loader's default tiled layout.

On an M3 Max with Wine 11 under Rosetta and the Apple converter, a normally
launched guest populated independent tiled texel offsets, sampled both
layouts and their mip tails via translated shaders, and checked Metal pixel
readback, including odd edge dimensions. Invalid layouts and subresource maps
were rejected. With the launcher test environment below, run `cargo test
--locked -p victory --target x86_64-apple-darwin --test wine_tiled -- --ignored
--nocapture`. This does not establish support for other console swizzles.

BC sampling supports `BC1_UNORM`, `BC1_UNORM_SRGB`, `BC3_UNORM` and
`BC3_UNORM_SRGB`, with top-level width and height multiples of four. Typed
resources and explicit SRVs must have identical formats; typeless resources
and reinterpretation are rejected. Metal performs the sRGB decode on RGB;
alpha remains linear. The host checks Metal's BC texture compression support
at draw time and fails if the device lacks it; there is no software fallback.
Only untiled full-mip copies are supported. Each BC mip uses
`ceil(width / 4)` blocks per row and `ceil(height / 4)` block rows, with
8-byte BC1 or 16-byte BC3 blocks. Copy footprints report dimensions rounded
up to whole blocks and retain the 256-byte pitch and 512-byte offset rules,
including for mips smaller than a block. The proxy exposes
`GetCopyableFootprints` for its supported buffer and 2D texture subset.

On an M3 Max using Wine 11 under Rosetta and the Apple converter, a normally
launched guest uploaded and read back independently constructed BC1/BC3
linear and sRGB blocks at 8x8, 4x4, 2x2 and 1x1 mips with padded rows and
distinct offsets. Translated Metal draws sampled each mip and verified
readback colors and alpha; recorded resources and descriptor heaps were
released before submission. Invalid formats, layouts, SRV ranges, mips,
offsets and pitches were rejected. Run the focused test with the launcher
environment below using `cargo test --locked -p victory --target
x86_64-apple-darwin --test wine_bc -- --ignored --nocapture`. This verifies
the listed subset, not console tiling or game compatibility.

On Wine 11 under Rosetta, a normally launched TLS guest uploaded odd-sized
7x5, 3x2 and 1x1 mips with independent offsets/pitches, sampled each through
an explicit SRV in translated Metal draws, and verified pixels and mip
readback including preserved row padding. Invalid ranges, mip counts, formats,
layouts, subresources, offsets and pitches were rejected. With the launcher
test environment below, run `cargo test --locked -p victory --target
x86_64-apple-darwin --test wine_mips -- --ignored --nocapture`. No game binary
or console tiled layout was tested.

On an M3 Max with Wine 11 and the Apple converter, the compiled TLS-bearing
guest uploaded two textures, sampled them in multiple draws across two
submissions, and verified changed and preserved pixels, row padding, and
recorded-resource lifetime. The indexed, depth and blending fixtures passed
their pixel/readback checks and rejected invalid cases. A separate guest
verified perspective-interpolated UVs and linear filtering of a nonuniform
texture. The original Wine graphics and compute tests and native graphics
regression also passed. These synthetic results do not establish arbitrary
TLS or game compatibility.

Using the import libraries built above, build both the valid and invalid-state
TLS fixtures:

```sh
for variant in valid invalid; do
  flags=
  [ "$variant" = invalid ] && flags=-DINVALID_GRAPHICS_TOPOLOGY
  x86_64-w64-mingw32-clang -O2 -fno-emulated-tls $flags \
    -c tests/fixtures/windows_d3d12x_tls_graphics.c \
    -o "/tmp/victory-wine-hle-tls/graphics-$variant.obj"
  lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /include:_tls_used \
    "/out:/tmp/victory-wine-hle-tls/graphics-$variant.exe" \
    "/tmp/victory-wine-hle-tls/graphics-$variant.obj" \
    /tmp/victory-wine-hle-tls/kernel32.lib /tmp/victory-wine-hle-tls/d3d12_xs.lib \
    "$LLVM_MINGW/x86_64-w64-mingw32/lib/libmingw32.a"
done
export VICTORY_TEST_D3D12X_TLS_GRAPHICS_PE="$(winepath -w /tmp/victory-wine-hle-tls/graphics-valid.exe)"
export VICTORY_TEST_D3D12X_TLS_GRAPHICS_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/graphics-invalid.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test wine_graphics --target x86_64-pc-windows-gnullvm -- --ignored --nocapture
```

Build and run the ordered TLS guest and its invalid-topology variant:

```sh
for variant in valid invalid; do
  flags=
  [ "$variant" = invalid ] && flags=-DINVALID_GRAPHICS_TOPOLOGY
  x86_64-w64-mingw32-clang -O2 -fno-emulated-tls $flags \
    -c tests/fixtures/windows_d3d12x_ordered_graphics.c \
    -o "/tmp/victory-wine-hle-tls/ordered-$variant.obj"
  lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /include:_tls_used \
    "/out:/tmp/victory-wine-hle-tls/ordered-$variant.exe" \
    "/tmp/victory-wine-hle-tls/ordered-$variant.obj" \
    /tmp/victory-wine-hle-tls/kernel32.lib /tmp/victory-wine-hle-tls/d3d12_xs.lib \
    "$LLVM_MINGW/x86_64-w64-mingw32/lib/libmingw32.a"
done
export VICTORY_TEST_D3D12X_ORDERED_GRAPHICS_PE="$(winepath -w /tmp/victory-wine-hle-tls/ordered-valid.exe)"
export VICTORY_TEST_D3D12X_ORDERED_GRAPHICS_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/ordered-invalid.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test wine_ordered_graphics --target x86_64-pc-windows-gnullvm -- --ignored --nocapture
```

The separate indexed, depth and blending fixtures use the same D3D12.X import
library and service:

```sh
for feature in indexed depth blend; do
  case "$feature" in
    indexed) invalid=INVALID_GRAPHICS_TOPOLOGY ;;
    depth) invalid=INVALID_DEPTH_CLEAR ;;
    blend) invalid=INVALID_BLEND_STATE ;;
  esac
  for variant in valid invalid; do
    flags=
    [ "$variant" = invalid ] && flags="-D$invalid"
    x86_64-w64-mingw32-clang -O2 $flags \
      -c "tests/fixtures/windows_d3d12x_${feature}_graphics.c" \
      -o "/tmp/victory-wine-hle-tls/$feature-$variant.obj"
    lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /dynamicbase \
      "/out:/tmp/victory-wine-hle-tls/$feature-$variant.exe" \
      "/tmp/victory-wine-hle-tls/$feature-$variant.obj" \
      /tmp/victory-wine-hle-tls/d3d12_xs.lib
  done
done
export VICTORY_TEST_D3D12X_INDEXED_PE="$(winepath -w /tmp/victory-wine-hle-tls/indexed-valid.exe)"
export VICTORY_TEST_D3D12X_INDEXED_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/indexed-invalid.exe)"
export VICTORY_TEST_D3D12X_DEPTH_PE="$(winepath -w /tmp/victory-wine-hle-tls/depth-valid.exe)"
export VICTORY_TEST_D3D12X_DEPTH_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/depth-invalid.exe)"
export VICTORY_TEST_D3D12X_BLEND_PE="$(winepath -w /tmp/victory-wine-hle-tls/blend-valid.exe)"
export VICTORY_TEST_D3D12X_BLEND_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/blend-invalid.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test wine_extended_graphics --target x86_64-pc-windows-gnullvm -- --ignored --nocapture
```

For the independent UV/linear-filter fixture:

```sh
for variant in valid invalid; do
  flags=
  [ "$variant" = invalid ] && flags=-DINVALID_UV_TOPOLOGY
  x86_64-w64-mingw32-clang -O2 $flags \
    -c tests/fixtures/windows_d3d12x_uv_graphics.c \
    -o "/tmp/victory-wine-hle-tls/uv-$variant.obj"
  lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /dynamicbase \
    "/out:/tmp/victory-wine-hle-tls/uv-$variant.exe" \
    "/tmp/victory-wine-hle-tls/uv-$variant.obj" /tmp/victory-wine-hle-tls/d3d12_xs.lib
done
export VICTORY_TEST_D3D12X_UV_PE="$(winepath -w /tmp/victory-wine-hle-tls/uv-valid.exe)"
export VICTORY_TEST_D3D12X_UV_INVALID_PE="$(winepath -w /tmp/victory-wine-hle-tls/uv-invalid.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test wine_uv_graphics --target x86_64-pc-windows-gnullvm -- --ignored --nocapture
```

### Opt-in window presentation

To enable presentation, set `VICTORY_WINE_METAL_PRESENT=1` in the helper's
environment before starting it. Without that opt-in, presentation requests
fail and the helper remains headless. The Wine queue accepts one plane with
one live RGBA8_UNORM resource and null present parameters. The plane token is
accepted but does not pace presentation, matching the upstream shortcut.
Presentation is synchronous and serialized; it returns `E_NOTIMPL` rather
than bypassing a pending queue wait. Source pixels and resource state are
preserved. The caller must not modify mapped resources concurrently. Other
formats, multiple planes, placement, non-null present parameters, HDR and
general swap-chain behavior are unsupported.

The helper owns a visible AppKit window and `CAMetalLayer` on the macOS main
thread. It stretches each source to the current view backing size, presents
the drawable, and waits for Metal completion. Resizing changes the drawable;
after the window is closed, subsequent presents fail. The helper requires an
Aqua session and a Metal device. Native macOS PE fixtures can use the same
helper; see [the native loader notes](native-macos-loader.md).

With the import libraries from above and the presentation-enabled helper
running, build and run the TLS-bearing Wine fixture:

```sh
x86_64-w64-mingw32-clang -O2 -fno-emulated-tls \
  -c tests/fixtures/windows_d3d12x_present_graphics.c \
  -o /tmp/victory-wine-hle-tls/present.obj
lld-link /entry:mainCRTStartup /subsystem:console /nodefaultlib /include:_tls_used \
  /out:/tmp/victory-wine-hle-tls/present.exe /tmp/victory-wine-hle-tls/present.obj \
  /tmp/victory-wine-hle-tls/kernel32.lib /tmp/victory-wine-hle-tls/d3d12_xs.lib \
  "$LLVM_MINGW/x86_64-w64-mingw32/lib/libmingw32.a"
export VICTORY_TEST_D3D12X_PRESENT_PE="$(winepath -w /tmp/victory-wine-hle-tls/present.exe)"
CARGO_TARGET_DIR=/tmp/victory-wine-bridge-target cargo test -p victory \
  --test wine_present_graphics --target x86_64-pc-windows-gnullvm -- --ignored --nocapture
```

On an M3 Max, the Wine and native fixtures passed, as did GPU drawable
readback, window-resize and close-rejection probes. CoreGraphics reported the
window on-screen. Screen-capture permission was unavailable and a window-only
capture failed, so these results do not visually verify display pixels.

## Normal Wine process with D3D12.X HLE DLL

The `victory-d3d12x-dll` Windows-target crate builds a real `d3d12_xs.dll`.
Its named exports delegate to the existing VIctory D3D12.X HLE registry,
including the normal device allocator setup; the IID export contains the
actual GUID data. Wine loads the original executable and DLLs normally, so
Wine owns their static TLS, thread callbacks and process lifecycle. This route
does not use VIctory's EXE-as-DLL loader or its instruction-byte rewrite.
Other Xbox HLE DLLs and VIctory's manual-loader import overrides are **not**
automatically provided by this DLL. Unsupported names are not exported; Wine
handles unresolved imports according to its own loader behavior.
Linker-assigned export ordinals are not a supported console ordinal mapping;
use named imports only. Wine may resolve an ordinal by coincidence, so ordinal
imports must not be relied upon.

The manual VIctory loader first looks up an HLE export by DLL and name, or
`#n` for an ordinal, then consults its host allowlist, and reports an unresolved
import if neither resolves it. For example, its GameInput registry maps
ordinal 1 to `GameInputCreate`. Normal Wine instead resolves the PE import
table through the DLLs actually supplied to Wine; it does not invoke this
manual resolver or inherit its API-set overrides. The separate Wine GameInput
DLL described below explicitly provides the confirmed ordinal 1; the D3D12.X
DLL does not provide guessed console ordinal mappings.

When the Wine Metal proxy is enabled, `D3D12XboxCreateDevice` supports the base
device IID and the newer Device11 IID documented in the upstream device
implementation. The newer device creates the corresponding queue layout:
`CreateCommandQueueX` at `0x180`, `CreateCommandListX` at `0x248`,
`PresentX` at `0x100`, and `ExecuteCommandListsX` at `0x118`. Supported X
queue descriptions are direct engine 0 or compute pipe 1, queue 0, with zero
flags; list flags, node mask and barrier-batch size must be zero. Submission
flags must be zero; the frame token is accepted as pacing metadata but does
not pace execution. The supported newer list IIDs retain the same COM identity
as the base list; unimplemented methods still trap. Resource creation and
presentation remain limited to the proxy subsets described above; placed or
aliased resources and advanced queue semantics are not added by these IIDs.
This is not complete Device11, console queue, or title compatibility.

### Wine Metal queue fences

The proxy supports non-shared `CreateFence` (zero flags), completed values,
CPU `Signal` and `SetEventOnCompletion`, plus queue `Signal` and `Wait` on the
same device for both documented queue layouts. Event registrations duplicate
the supplied Windows event handle; a null event blocks the caller until the
value is reached. Shared/cross-adapter fence flags, `UINT64_MAX` values and
cross-device fence operations are rejected. Valid Windows event handles are
required.

Idle queues execute and read back GPU work synchronously. A queue waiting for
an unreached value instead returns to the caller and retains subsequent work
and signals in order on a per-queue worker. Another queue or CPU signal can
release the wait. Queue and fence state and recorded resources stay alive for
those queued operations, even if their guest COM references are released.
An unsignaled dependency can remain pending indefinitely; the proxy never
fabricates completion. Worker submission failures terminate rather than signal
a false success. Fence state is process-local, not a shared Metal event or a
cross-process Windows fence; advanced queue methods remain unsupported.

With the launcher test environment below, run
`cargo test --locked -p victory --target x86_64-apple-darwin --test wine_launcher launcher_runs_real_metal_fence_ordering -- --ignored --nocapture`.
On Wine 11 under Rosetta the compiled TLS guest passed an unsignaled timeout,
real translated compute/readback before signal, cross-queue wait and signal,
event and object lifetimes, and old/new queue ABI checks. It does not establish
game compatibility.

With the presentation-enabled Metal helper running and the DLL environment
below configured, the following test launches its compiled TLS-bearing PE
directly under Wine. It checks the actual loaded DLL path, Device11 and
console List6 identity, descriptor rejection, a translated graphics draw,
pixel readback and token-bearing submit/present. A separate guest verifies
that unsupported submission flags terminate rather than being ignored:

```sh
CARGO_TARGET_DIR=/tmp/victory-newer-host-target cargo test -p victory \
  --target x86_64-apple-darwin --test wine_newer_graphics -- --ignored --nocapture
```

For an explicitly supplied, trusted PE and isolated Wine prefix, build and
select the DLL as follows. LLVM-MinGW, Rust's Windows GNU-LLVM target, Wine,
and the runner setup at the start of this document are required:

```sh
CARGO_TARGET_DIR=/tmp/victory-wine-exe-target cargo build \
  -p victory-d3d12x-dll --target x86_64-pc-windows-gnullvm
export VICTORY_TEST_D3D12X_DLL=/tmp/victory-wine-exe-target/x86_64-pc-windows-gnullvm/debug/d3d12_xs.dll
export WINEPATH="$(winepath -w "$(dirname "$VICTORY_TEST_D3D12X_DLL")");$(winepath -w "$LLVM_MINGW/x86_64-w64-mingw32/bin")"
export WINEDLLOVERRIDES='d3d12_xs=n'
wine /path/to/trusted-guest.exe
```

This deliberately uses Wine's normal DLL search order, not a private loader
or injection. `WINEPATH` does **not** override a same-named DLL in a directory
Wine searches earlier, including the executable's directory. Inspect the
actual loaded module path and trust the executable, its directories, prefix,
and DLLs; these settings do not sandbox code or guarantee DLL identity in
untrusted directories. The synthetic tests check the loaded module's exact
path and exercise missing-DLL and incorrect-path failures.

With the native Metal compute proxy running as described above and the same
address and token exported, the two isolated Wine integration tests build
distinct self-authored PE executables and a separate TLS-bearing DLL:

```sh
CARGO_TARGET_DIR=/tmp/victory-wine-exe-target cargo test -p victory \
  --target x86_64-apple-darwin --test wine_process -- --ignored --nocapture
```

Both verify compiler-generated static TLS and callbacks across two PE images,
two worker threads and a Wine-to-guest locale callback. They also observe the
TLS-bearing DLL's process-detach callback with its state intact. An attempted
EXE process-detach callback marker was not observed under this Wine build and
is not claimed by this test. The second creates a
device through the imported HLE DLL, dispatches the restricted synthetic
D3D12.X compute workload through native Metal and checks GPU readback `42`.
These tests passed with Wine 11 under Rosetta on an M3 Max. Wine's macOS
transition internals are Wine implementation details; this is not ARM-native
PE execution, a full console runtime, or evidence of game compatibility.

## Normal Wine process with XG and XGameRuntime HLE DLLs

The Windows-target crates `victory-xg-dll` and
`victory-xgameruntime-dll` provide normal Wine-loadable `xg_xs.dll` and
`xgameruntime.dll` files. They call the existing shared VIctory
implementations; they do not map the executable or rewrite TLS. XG exports
`XGCreateTextureComputer` and `XGSuggestSwizzleMode`. XGameRuntime exports
`XGameRuntimeInitialize`, `XGameRuntimeUninitialize`, `InitializeApiImpl`,
`InitializeApiImplEx`, `InitializeApiImplEx2`, `UninitializeApiImpl`,
`QueryApiImpl`, and `XErrorReport`.

XSystem and XUser are provider interfaces obtained from `QueryApiImpl`, not
separate DLLs or direct `XUser*` exports. The shared HLE implements a fixed
offline local identity and existing offline/error behavior; this does not
connect to Xbox services. `QueryApiImpl` also exposes the other interfaces
already registered by that HLE, with their existing limitations; this test
does not validate all of them. Missing names are not exported, unknown
provider/interface pairs return the upstream unsupported error, and
linker-assigned PE ordinals are not Xbox ordinal mappings. Do not rely on
ordinal imports. Unimplemented upstream vtable slots retain their existing
behavior and are not a guarantee of ABI coverage.

Using the Wine and LLVM-MinGW setup above, build all three independently
loadable DLLs into the same explicit directory:

```sh
CARGO_TARGET_DIR=/tmp/victory-providers-target cargo build \
  -p victory-xg-dll -p victory-xgameruntime-dll -p victory-d3d12x-dll \
  --target x86_64-pc-windows-gnullvm
export VICTORY_TEST_HLE_DLL_DIR=/tmp/victory-providers-target/x86_64-pc-windows-gnullvm/debug
export WINEPATH="$(winepath -w "$VICTORY_TEST_HLE_DLL_DIR");$(winepath -w "$LLVM_MINGW/x86_64-w64-mingw32/bin")"
export WINEDLLOVERRIDES='xg_xs,xgameruntime,d3d12_xs=n'
wine /path/to/trusted-guest.exe
```

Wine's usual DLL search precedence still applies: an earlier same-named DLL
can shadow the selected directory. These DLLs and paths are not a sandbox;
trust the executable, DLLs and Wine prefix. The separate Windows DLLs each
link their own HLE state and do not promise shared mutable configuration across
modules. Other Xbox DLLs, ordinals, runtime interfaces and game compatibility
remain outside the verified scope.

With `VICTORY_TEST_HLE_DLL_DIR` and `LLVM_MINGW` set, run:

```sh
CARGO_TARGET_DIR=/tmp/victory-providers-target cargo test -p victory \
  --target x86_64-apple-darwin --test wine_providers -- --ignored --nocapture
```

On the M3 Max with Wine 11 under Rosetta, the synthetic normally launched PE
passed static TLS and two-thread callback checks, exact DLL-path and missing
name/DLL checks, XG texture-computer layout and reference lifetime, and
XSystem/XUser identity, handle, gamertag and error checks. The existing
normally launched D3D12.X-to-Metal compute regression also passed with real
GPU readback. No game files were used; these results do not establish game
compatibility or ARM-native PE execution.

## Normal Wine GameInput HLE DLL

`victory-gameinput-dll` builds `gameinput.dll` with one explicit PE export:
`GameInputCreate`, also at ordinal 1. Both forms enter the existing GameInput
HLE implementation and return its same process-wide singleton; no other
ordinal mapping is claimed. A valid create pins the DLL for the process
lifetime before the HLE starts its polling worker, so unloading a loader
reference cannot invalidate its objects or thread. A null output returns the
provider's `E_INVALIDARG`; a failure to pin returns `E_FAIL`.

Build for Wine using the LLVM-MinGW setup above:

```sh
export CARGO_TARGET_DIR=/tmp/victory-gameinput-win-target
cargo build --locked -p victory-gameinput-dll --target x86_64-pc-windows-gnullvm
```

For direct Wine use, a provider beside the trusted guest EXE can take
precedence, but do not overwrite a title DLL. On the tested Wine 11 prefix,
`gameinput=n` with the provider only in `WINEPATH` failed because Wine's
system32 GameInput placeholder was found first; adding `WINEDLLPATH` did not
fix it. Allowing `gameinput=n,b` loaded Wine's builtin instead, not this
provider. Wine DLL search is not a sandbox.

The integrated `wine-run --provision-gameinput` route keeps the original guest
path and files. Put `gameinput.dll` alongside the other selected HLE DLLs and
choose a **new, empty, dedicated Wine prefix**. The launcher initializes it and
installs the native provider into that prefix's system32, with a native Wine
override. It accepts an existing prefix only if the installed provider already
byte-matches the selected DLL; it does not replace files in existing prefixes.
This separate prefix has its own Wine registry and saves and is not equivalent
to an existing game prefix. Do not use it concurrently or with untrusted
processes. An interrupted first initialization may leave a nonempty prefix
requiring a fresh dedicated prefix. The usual
`--allow-unverified-dll-search` acknowledgement remains required: activation
contexts, guest-directed loading and search-path races are not pinned.

This exposes the upstream provider's existing subset, not a new complete input
stack: it polls XInput controllers and focused-window keyboard state under Wine,
provides its keyboard/gamepad fallback, readings and callbacks, and retains
its singleton/device reference behavior. Dispatcher and aggregate-device
creation return `E_NOTIMPL`; platform-device lookup and other interfaces remain
limited by the provider. Input availability depends on Wine and host focus.
Physical device operation and title compatibility are not established.

With the Wine and LLVM-MinGW tools on `PATH`, run the isolated Wine fixture:

```sh
export VICTORY_TEST_HLE_DLL_DIR=/tmp/victory-gameinput-win-target/x86_64-pc-windows-gnullvm/debug
export CARGO_TARGET_DIR=/tmp/victory-gameinput-host-target
cargo test --locked -p victory --target x86_64-apple-darwin \
  --test wine_gameinput -- --ignored --nocapture
```

On Wine 11 under Rosetta, the normally launched synthetic PE passed actual
named and ordinal import-table checks, matching export addresses, singleton,
reading, callback, error and reference-lifetime checks. A separate dynamically
loaded guest verified the module remains pinned after its sole loader reference
is released. The seven existing GameInput HLE unit tests also passed as Windows
executables under Wine. No game binary or physical controller was tested.

With the launcher test prerequisites below and `VICTORY_TEST_GAMEINPUT_DLL`
set to the built `gameinput.dll`, the ignored
`launcher_provisions_gameinput_in_fresh_prefix_without_relocating_guest` test
also exercises the integrated route. On Wine 11 under Rosetta it passed named
and ordinal imports from the selected system32 provider, the original PE
path and adjacent resource, TLS/provider checks and a real native Metal
compute readback. It checks prefix reuse, conflicting DLLs, unsupported
ordinals and existing-prefix preservation; no game binary was used.

## Integrated Wine launcher

On x86-64 macOS (including Rosetta on Apple Silicon), `victory wine-run`
launches a trusted PE as a normal Wine process and starts a separate native
Metal host. Supply the Wine executable, `winepath`, Metal host, Apple
converter, Wine prefix and HLE DLL directory explicitly. The directory must
contain `d3d12_xs.dll`, `xg_xs.dll`, `xgameruntime.dll`, `xmem.dll`,
`acphal.dll`, `xapu.dll` and the private `victory_xmem_backend.dll`. For example:

```sh
export CARGO_TARGET_DIR=/tmp/victory-launcher-target
cargo build --locked -p victory-xg-dll -p victory-xgameruntime-dll -p victory-d3d12x-dll \
  -p victory-xmem-dll -p victory-xmem-backend -p victory-acphal-dll -p victory-xapu-dll \
  --target x86_64-pc-windows-gnullvm
cargo build --locked -p victory-gpu --example wine_metal_compute_host --target x86_64-apple-darwin
cargo build --locked -p victory --target x86_64-apple-darwin
export WINEPATH="$(/opt/homebrew/bin/winepath -w "$LLVM_MINGW/x86_64-w64-mingw32/bin")"
mkdir -p /tmp/victory-launch-prefix
"$CARGO_TARGET_DIR/x86_64-apple-darwin/debug/victory" wine-run /path/to/trusted-guest.exe \
  --wine /opt/homebrew/bin/wine --winepath /opt/homebrew/bin/winepath \
  --metal-host "$CARGO_TARGET_DIR/x86_64-apple-darwin/debug/examples/wine_metal_compute_host" \
  --converter /path/to/libmetalirconverter.dylib \
  --dll-dir "$CARGO_TARGET_DIR/x86_64-pc-windows-gnullvm/debug" \
  --wine-prefix /tmp/victory-launch-prefix --allow-unverified-dll-search -- --guest-argument
```

The launcher generates a 128-bit token from `/dev/urandom`; the host binds an
OS-assigned loopback port and announces readiness. The token is passed in the
child environments, not printed. Other same-user processes may be able to
read those environments; the token is not a sandbox or a boundary against
such processes. The launcher preserves unrelated environment settings,
prepends the selected DLL directory to `WINEPATH`, and requires native loading
for those six providers; conflicting existing provider overrides are
rejected. `VICTORY_WINE_METAL_PRESENT=1` remains an inherited opt-in for the
existing presentation path.

By default the launcher fails before starting Wine or the Metal host: it
cannot bind Wine's provider basenames to exact files. The option above
explicitly accepts that limitation for a trusted, stable guest, prefix,
providers and search paths. In that opt-in mode it rejects conflicting
case-insensitive provider names in the provider directory, executable
directory, current directory, and the prefix's `drive_c` and mapped `c:`
Windows, System, System32 and SysWOW64 directories. Unreadable directories,
ambiguous Windows directory components and dangling provider links fail the
check. These are snapshots, not pinned opens.

The same offline import diagnostic runs before Wine or the Metal host starts,
and can be run separately without either:

```sh
"$CARGO_TARGET_DIR/x86_64-apple-darwin/debug/victory" wine-imports \
  /path/to/trusted-guest.exe \
  --dll-dir "$CARGO_TARGET_DIR/x86_64-pc-windows-gnullvm/debug" \
  --guest-dll required-title-library.dll
```

It parses the executable and reachable guest DLL candidates beside it,
following ordinary and delay imports with ASCII case-insensitive DLL names and
handling cycles. It checks imports of the six selected providers against their
named exports; unsupported symbols and ordinals fail before launch.
`chad.dll` and `xcrdapi.dll` are reported as having no VIctory Wine provider,
without assuming Wine cannot resolve them. Use `--provision-gameinput` when
the separate GameInput provider will be used. The optional repeatable
`--guest-dll` declares a DLL that must exist beside the executable; a declared
DLL that is missing or case-ambiguous is an error. The same options are
available on `wine-run`. The report lists other imports not found in that
directory as unclassified external dependencies, **not** as missing: they
could be supplied by Wine, Windows DLLs or another search directory.

This is a snapshot of candidates, not Wine's actual DLL resolution. It cannot
establish that Wine selects an inspected DLL, resolve its system DLLs, follow
runtime `LoadLibrary`/`GetProcAddress`, inspect guest directories not searched
by this diagnostic, or account for activation contexts, Known DLLs, loaded
modules, path-specific registry overrides, guest-controlled search paths and
changes after preflight. `WINEPATH` adds a search directory; it does not prove
which file Wine loads. Check loaded module paths when possible, but a
post-load check does not undo initialization. The launcher does not modify or
copy guest files; the unsandboxed guest can modify files and Wine may write to
the prefix. No report establishes title compatibility.

The focused synthetic diagnostics tests need LLVM-MinGW but not Wine or Metal:

```sh
LLVM_MINGW=/path/to/llvm-mingw cargo test --locked -p victory \
  --target x86_64-apple-darwin --test wine_imports -- --ignored --nocapture
```

On this workstation both tests passed; the case-collision subcase was checked
on a temporary case-sensitive APFS volume. The existing normal-Wine TLS and
Metal readback launcher regression also passed. No game files were configured.

For a trusted title that expects its files on `G:`, an optional
`--title-root /absolute/path/to/title` checks that the existing
`<wine-prefix>/dosdevices/g:` is a symlink resolving to that directory.
Configure the drive in Wine first (for example, with that prefix's `winecfg`);
the launcher does not create or change it. The launcher checks before running
Wine, again after `winepath`, and immediately before starting the guest; an
absent, dangling, non-symlink or different mapping fails the launch. With no
option, drive behavior is unchanged. The guest uses Wine's normal file APIs,
not VIctory's native-fixture file provider.

These are checks at instants in time, not an enforced file-root sandbox:
another process or the guest can change a mapping after a check, and Wine's
normal permissions, other drives, symlinks and guest write access still apply.
Use only trusted guests and stable prefixes and files. No title files are
modified by this option; Wine itself can modify its prefix and the guest can
modify files it can access.

The Windows custom loader also mounts `T:` over a per-title temporary
directory and translates `T:` and `T:name` relative to that root. The Wine
launcher does not implement this interception or configure `T:`; an existing
Wine mapping can serve ordinary Wine file calls, but its drive-relative
current-directory behavior is not the custom loader's root-relative rule.
An unsandboxed guest can access other Wine drives and host paths.

With the launcher test environment above, run:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_title_root -- --ignored --nocapture --test-threads=1
```

On Wine 11 under Rosetta, two synthetic tests passed. The normal Wine PE read
a UTF-16 title path with case variations, exercised Windows sharing and
missing-file errors, and used compiler-generated TLS in two worker threads.
The test checked that the guest executable, title data and preconfigured drive
link were unchanged. A separate test rejected seven invalid root or mapping
cases before starting the Metal host.

Wine's [loader](https://github.com/wine-mirror/wine/blob/wine-11.0/dlls/ntdll/loader.c#L3205-L3259)
handles activation contexts and previously loaded modules before ordinary
directory lookup. Its native/builtin
[overrides](https://github.com/wine-mirror/wine/blob/wine-11.0/dlls/ntdll/unix/loadorder.c#L316-L423)
select loading behavior, not an exact provider file. The launcher has no
enforcement mechanism for those decisions. GameInput provisioning is a
separate opt-in described above.

The host readiness signal confirms binding, not converter ABI or shader
support; converter loading and shader errors can occur on first use. Wine and
Metal operation coverage remains limited as described above. The launcher
monitors both children, returns the guest's exit status, and handles INT, TERM
and HUP with bounded child-group cleanup. It does not terminate a shared Wine
server or track descendants that detach into another session. This is not
ARM-native PE execution or evidence of game compatibility.

The XGame title provider's `XLaunchNewGame` sends a bounded authenticated
loopback request to `wine-run` after the existing HLE suspend routine. The
launcher waits for the old Wine child to exit, tears down its process group,
checks the requested executable and its imports again, and launches it with the
same prefix, provider configuration, Metal host and save-root lock. An empty
executable requests the current guest; a nonempty executable must be an
absolute Windows drive path, not a relative or UNC path. The executable and
argument strings must be UTF-8 and at most 32,767 bytes each. A missing target,
invalid request or unavailable launcher channel fails explicitly instead of
silently restarting the wrong image. The guest and prefix remain trusted; the
inherited token is not a security boundary against same-user processes.

As in the existing Windows custom-loader path, the argument string is split on
whitespace; quoted Windows command-line arguments are **not** preserved. Wine
system APIs are not intercepted to populate HLE suspend registrations, so
normal Wine lifecycle callbacks are not claimed. Wine server processes and
descendants that leave the managed process group are not supervised.

With the launcher test environment below, run:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_relaunch -- --ignored --nocapture --test-threads=1
```

The synthetic tests cover a second guest checking its image and arguments and
confirming the first process had exited, absent and relative targets failing,
and host cleanup.

For the focused launcher tests, set the following paths (using the same
target directory and LLVM-MinGW setup as above):

```sh
export VICTORY_TEST_WINE=/opt/homebrew/bin/wine
export VICTORY_TEST_WINEPATH=/opt/homebrew/bin/winepath
export VICTORY_TEST_WINESERVER=/opt/homebrew/bin/wineserver
export VICTORY_TEST_METAL_HOST="$CARGO_TARGET_DIR/x86_64-apple-darwin/debug/examples/wine_metal_compute_host"
export VICTORY_TEST_METAL_CONVERTER=/path/to/libmetalirconverter.dylib
export VICTORY_TEST_DLL_DIR="$CARGO_TARGET_DIR/x86_64-pc-windows-gnullvm/debug"
export VICTORY_TEST_GAMEINPUT_DLL=/path/to/gameinput.dll
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_launcher -- --ignored --nocapture
```

On Wine 11 under Rosetta on an M3 Max, all eight tests passed: a real
TLS-bearing Wine PE called the provider DLLs and verified Metal readback 42;
unacknowledged search and known shadow DLLs were rejected; host exit and
signal paths cleaned up the host; and two simultaneous launches used distinct
loopback ports and tokens. This uses synthetic files, not a configured game.

## Offline Wine XGameSave

`wine-run --save-root /absolute/path/to/dedicated-data` enables the existing
offline XGameSave HLE for the normal Wine process. The selected directory must
already exist and must be dedicated to one title. The launcher converts it to
a Wine path and checks that converting back resolves to the same directory;
if `--title-root` is also supplied, the two directories must not overlap.
It takes a nonblocking advisory lock on the directory for the launcher's
lifetime, rejecting a second cooperating launcher using the same directory.

The runtime DLL reads `MicrosoftGame.config` beside the actual executable
using the upstream manifest parser, even without a save root, so title and
package identity do not depend on enabling storage. A missing manifest
retains upstream's default title ID and quota. Configured saves live under
`<save-root>/saves` in the upstream per-user and service-configuration
layout; choose different save roots for different titles even if IDs match.
With no option the launcher removes any inherited
`VICTORY_WINE_TITLE_DATA`, so XGameSave remains unconfigured and reports
`NO_ACCESS` rather than writing in the working directory.

This is the existing local, offline XGameSave implementation, not Xbox cloud
sync. It implements its supported asynchronous operations and versioned
container updates; blocking slots and other unimplemented methods remain
unsupported. Without an explicit storage-drive option, persistent-local-storage
requests also return unsupported rather than exposing VIctory's custom-loader
virtual drive, which Wine's ordinary file imports cannot use.
The runtime DLL pins itself when returning provider objects, so their method
tables remain available if the guest releases its explicit DLL reference.

The save root, Wine mappings and contents are trusted, not sandboxed or pinned.
Symlinks and other processes can redirect or modify paths. The advisory lock
coordinates only cooperating launchers that use the same directory; it does
not prevent a guest, another program, a nested save root, or a detached child
from accessing the data. Do not use the same save data concurrently outside
that coordination. Wine and the guest can write elsewhere with the user's
permissions.

### Wine persistent local storage

For PLS, first create a real `storage` directory inside the dedicated save
root and configure an existing Wine prefix drive link to that directory (for
example, configure `P:` in `winecfg`). Then add
`--save-root /absolute/path/to/dedicated-data --storage-drive P` to the
launcher command. The launcher does not create directories or alter drive
links. It accepts letters D–Y except G and T, rejects duplicate mappings to
the storage directory, requires the lowercase `dosdevices/p:` link and
checks its target and Wine's path conversion. A missing or mismatched link,
symlinked storage directory, or conflicting drive fails before guest startup.
These checks are repeated around Wine initialization; mappings can still
change afterward.

The PLS API returns the real Wine drive root, and ordinary Win32 file APIs
access its mapped directory. Space reporting and prompts use the existing
manifest quota and local disk accounting; they do not enforce a filesystem
quota. Game saves remain in the separate `saves` directory. The preexisting
save-root advisory lock covers cooperating launchers only; Wine, the guest,
other programs, and symlinks are not sandboxed. This is offline storage, not
cloud synchronization.

With the launcher test environment above:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_pls -- --ignored --nocapture --test-threads=1
```

On Wine 11 under Rosetta, two synthetic tests passed. A normally launched
TLS-bearing PE wrote and reopened a file through the returned drive across
launches, checked quota reporting and prompts, and used two isolated roots.
Other cases covered unsupported configuration, invalid or duplicate drive
mappings, invalid letters, and advisory lock contention.

### Wine XGameSave regression

With the launcher test environment above:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_saves -- --ignored --nocapture --test-threads=1
```

On Wine 11 under Rosetta, two synthetic tests passed. A compiler-TLS PE wrote
and reopened a binary save in separate launches, checked a second root was
isolated, and exercised an explicitly loaded runtime object after
`FreeLibrary`. It also verified title and package identity with and without
a save root, the missing-manifest default, unsupported slots and storage
paths, unconfigured saves, invalid or locked roots, and overlapping
title/save roots. A focused TLS/Metal launcher regression passed separately.

## Normal Wine XMem HLE DLL

`xmem.dll` exports the 13 named functions in the existing Windows XMem HLE
registry. Their linker-assigned ordinals are not console ordinal mappings;
unknown names are absent and the launcher rejects XMem ordinal imports.
Calls forward to the existing implementation, including title executable
`__XMemAllocationHooks__`, allocator alignment, virtual queries, budget
transfers and physical-page state. `victory_xmem_backend.dll` is a private
companion, loaded beside the provider and pinned for the process lifetime.
The D3D12.X DLL uses the same backend for its allocator and page-mapper
callbacks; the backend's `VIctoryXMemMapAlias` is a VIctory-private interface,
not an Xbox export.

`wine-run --console series-x|series-s` selects the model; the default is
`series-x`. The launcher supplies its selection to each newly launched Wine
process, including title-provider relaunches, overriding an inherited
`VICTORY_WINE_CONSOLE`. Independently linked XGameRuntime, XMem backend and
D3D12.X copies initialize from that setting. XSystem analytics reports the
selected Xbox name; the local D3D12.X HLE and Wine Metal proxy's DXGI adapter
description report the same memory split. XMem reports Series X's 10 GiB GPU-optimal plus 3.5 GiB
standard budget or Series S's 7,776 MiB standard budget; transfers reduce the
standard portion in 2 MiB increments and reset for a new process.

This does not intercept Wine registry calls: a title reading `ConsoleMode`
directly through Wine does not receive VIctory's native-loader registry
override. The environment and guest are trusted; providers used without
the launcher default to Series X if the setting is absent, and reject an
invalid setting (XGameRuntime and D3D12.X return an error; XMem aborts).

This carries forward the upstream HLE's limitations: memory budgets use its
current console setting, virtual queries report committed memory as
graphics-accessible, and not every console page/commit behavior is modelled.
The Wine Metal proxy has its own restricted GPU API coverage; XMem page
aliases do not establish GPU tiled-resource or DirectStorage support. A
matching companion is required even for the D3D12.X DLL, and none of these
DLLs or the Wine prefix are sandboxed.

With the launcher test environment above, run the XMem fixtures with:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_xmem -- --ignored --nocapture
```

On Wine 11 under Rosetta, both tests passed. The normal TLS-bearing synthetic
PE exercised all named imports, per-thread allocations and title hooks,
virtual memory, physical-page aliasing and the local D3D12.X queue's page
mapping callback against the same backend. It rejected an unknown import and
the launcher rejected XMem and companion collisions. The queue check selects
the local HLE path; the separate Metal launcher regression performs actual GPU
compute readback, not a Metal tiled-page-mapping test.

With a clean committed-HEAD snapshot plus the console changes, Wine 11 under
Rosetta passed both XMem tests, all three `wine_relaunch` tests and the real
Metal launcher compute regression. The XMem guest checks default, explicit X
and S analytics and local DXGI budgets, transfer boundaries and invalid CLI
selection. The relaunch guest checks S analytics and a fresh S XMem budget in
both processes. The DXGI check uses the local HLE path, not the Metal proxy.

The real Metal launcher regression also queries all three upstream DXGI device
IIDs on the proxy for Series X and S, checks the adapter's model-specific
memory description and COM identity/lifetime, then executes Metal compute
and reads back its result. The proxy retains the device while its DXGI
interface is referenced; the adapter uses the existing upstream static object.
This does not extend the proxy's other DXGI or graphics capabilities.

## Normal Wine AcpHal HLE DLL

`acphal.dll` forwards the nine named exports in the existing AcpHal registry
to its actual HLE implementation. The exports share one heap, SHAPE-context
state and XMA decoder worker in the DLL. The DLL pins itself on first use so
returned pointers, object vtables and the worker cannot outlive its code.
Pinning failure terminates the process rather than returning unsafe objects.
Other names are absent; linker-assigned ordinals are not claimed as console
ordinal mappings.

The upstream implementation is not complete audio hardware: XMA decoding,
heap address translation and the ACP message/command subset are implemented,
but SHAPE rate conversion, EQ, filtering and mixing are not. Some unknown
commands are acknowledged without being executed, and configured heap sizes
are not enforced as limits. This is not a claim that a title's audio works.

With the launcher test environment above:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_acphal -- --ignored --nocapture
```

On Wine 11 under Rosetta, three synthetic tests passed: a compiler-TLS PE
used all named imports, concurrent heap operations, contexts, ACP message
queues and a synthetic XMA stream; a separate guest verified module pinning
after `FreeLibrary`; and unsupported imports, ordinal imports and a
same-named DLL collision failed before host startup. The upstream HLE audio
regression also passed five Windows tests under Wine.

## Normal Wine XApu HLE DLL

`xapu.dll` forwards the five named exports in the upstream XApu registry to
the existing HLE. Each connection retains its own decoder, command/result
queue, memory and optional event. The DLL pins itself on first use so a
connection cannot outlive its code; pinning failure terminates the process.
Other names are absent and linker-assigned ordinals are not console mappings.
These XApu connections are independent of the AcpHal XMA heap and worker.

Only the upstream plain Opus decoding path is implemented; rate conversion
and decode-convert return `E_NOTIMPL`. Results are produced synchronously as
commands are submitted. This does not establish a complete audio pipeline or
title compatibility.

`xcrdapi.dll` (DirectStorage) remains a separate gap: its upstream HLE calls
the mapped-title file layer and XMem physical-page state. Linking an
independent copy into another Wine DLL would split that state; it is not
provided by the launcher. A `G:` mapping does not implement DirectStorage.

Microsoft's public [Xbox OpenFile documentation](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/dstorage/interfaces/idstoragefactoryx/methods/idstoragefactoryx_openfile)
requires a file interface IID from `dstorage_xs.h`; public method lists do not
establish all interface IIDs or vtable order. The central directory of the
public `Microsoft.GDK.Windows` NuGet package 2610.0.9661 contains no
`dstorage` or `xcrdapi`-named member. Microsoft's
[GDK repository](https://github.com/microsoft/GDK) describes Xbox Extensions
as licensed partner-only, and its [access guidance](https://learn.microsoft.com/en-us/gaming/gdk/docs/gdk-dev/development-downloads/access-resources)
describes the partner agreement and secure-download requirements. Availability
of a header to a licensed partner does not itself establish permission to
publish or reuse its contents. Desktop interfaces and guessed slots are not
substitutes for the applicable console ABI.

With the launcher test environment above:

```sh
cargo test --locked -p victory --target x86_64-apple-darwin --test wine_xapu -- --ignored --nocapture
```

On Wine 11 under Rosetta, three synthetic tests passed. A normal PE with
compiler-generated TLS exercised concurrent independent connections, events,
command and result queues, and decoded a 20 ms Opus silence packet into 960
float PCM frames. Another guest verified module pinning after `FreeLibrary`;
unsupported names and ordinals, provider collisions and override conflicts
were rejected. The upstream Windows XApu decoding regression also passed.
