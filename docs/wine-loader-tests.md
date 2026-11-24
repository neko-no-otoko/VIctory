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
