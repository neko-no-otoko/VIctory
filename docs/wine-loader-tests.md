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

In that same prefix, the synthetic loader integration tests passed 2/2. A
separate GPU compute experiment compiled self-authored HLSL with Microsoft
DXC v1.9.2609 and used an adapted MetalVulkan compute probe: after dispatch,
UAV readback was `42, 7, 7, 7` from a buffer initialized to 7. The adapted
probe is not included in this repository. VIctory-generated synthetic DXIL
was also tried but failed pipeline creation with `0x80070057` and a
`dxil-spirv` conversion error; the HLSL result does not validate that DXIL.
Neither experiment establishes game compatibility or a native Metal backend.
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
