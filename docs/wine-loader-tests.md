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
VIctory's Metal paths and synchronously copies results back. No
guest pointers or COM objects cross the process boundary. Wine and the native
host both execute under Rosetta on Apple Silicon; this is not ARM-native guest
execution. This is a trusted-workload experiment, not a process sandbox.

The restricted compute path supports one committed default raw UAV buffer, a
one-entry shader-visible heap, root constants in slot 0, descriptor table in
slot 1, and synchronous dispatch/readback. Limits are 4 MiB per shader,
16 MiB per buffer and 64 constant dwords. A separate restricted offscreen
graphics path is described below. Presentation, root CBVs, general resource
states and arbitrary D3D12.X methods remain unsupported; some unsupported void
methods abort rather than pretend to succeed. The loopback bearer token is not
a boundary against other processes with access to it.

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
resulting bytes and resource states back after the host completes. All proxy
queue submissions are serialized and synchronous; this is not asynchronous
fence or cross-process shared-memory support.

The supported subset includes multiple full color/depth clears, buffer copies,
full texture uploads and readbacks, resource transitions, and multiple
nonindexed or 16-/32-bit indexed triangle-list draws with viewport/scissor
and pipeline changes. Committed default-heap resources are limited to
single-mip RGBA8_UNORM color/sampled textures, D32_FLOAT depth textures, and
buffers. Graphics supports POSITION float4, optionally followed by TEXCOORD0
float2; one color target and optional depth target per draw; the native depth
comparison/write subset; source-alpha blending and color write masks; and a
single sampled texture and point or linear clamp sampler. Shader constants
must be contiguous and use one root slot. Texture copy row pitch is
256-byte aligned and buffer offset is 512-byte aligned.

This remains restricted: no presentation, asynchronous fences, partial clears,
multi-mip or tiled textures, BGRA/sRGB/BC formats through this proxy, stencil,
arbitrary sampler/address modes, or general D3D12 resource or pipeline states.
Unsupported combinations fail or trap rather than being ignored. Limits are
4 MiB per shader, 16 MiB per resource, 64 constant dwords, 4096 batch items,
and 64 MiB per request. Shader translation and drawing use the existing
native Metal path, not MetalVulkan.

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
