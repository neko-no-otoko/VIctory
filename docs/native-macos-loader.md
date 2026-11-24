# Experimental x86-64 macOS PE mapping

`victory_loader::macos::Image` is a small PE32+ mapping foundation for an
**x86-64 macOS process**, including one running under Rosetta on Apple
silicon. Rosetta translates the whole process; this is not a native arm64 PE
loader. It does not run GTA VI or implement an Xbox or Windows environment.

The mapper copies headers and sections, applies `DIR64` base relocations,
resolves ordinary imports through an `ImportResolver`, and sets page
permissions from section flags. It rejects writable/executable pages,
unresolved imports, unsupported relocations, DLL entry points, TLS, Windows
unwind tables, load configuration and delay imports. It does not run
initializers or manage Windows threads, exceptions, the TEB, GS-based TLS,
other DLL loading or Windows APIs. Export addresses remain valid only while
the image is alive; callers and imported callbacks must use the correct
Windows x64 ABI and satisfy their own safety and lifetime requirements.
Only load trusted code: calling an export executes arbitrary machine code.

## Synthetic test under Rosetta

The test builds the repository's own fixture with LLVM-MinGW. It needs no game
files. Install Rust's `x86_64-apple-darwin` target, Apple's command line tools,
Rosetta, and LLVM-MinGW (the tools `x86_64-w64-mingw32-clang`, `llvm-dlltool`,
and `lld-link` must be on `PATH`). From the repository root:

```sh
export LLVM_MINGW=/path/to/llvm-mingw
export PATH="$LLVM_MINGW/bin:$PATH"
rustup target add x86_64-apple-darwin
export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/usr/bin/clang
export CC_x86_64_apple_darwin=/usr/bin/clang
ulimit -c 0
cargo test -p victory-loader --test macos_probe --target x86_64-apple-darwin
cargo test -p victory-loader --lib --target x86_64-apple-darwin macos::tests
```

The integration test creates an actual PE with an import and a `DIR64`
relocation, maps it, calls its export using the Windows x64 ABI, checks the
host callback and rebased pointer, and checks that writes to read-only data
and executable code fault in child processes. It also checks missing imports,
invalid images and rejected entry points. Passing these tests is not evidence
of title execution, D3D12 translation, or Metal support. See
[rosetta-avx-probe.md](rosetta-avx-probe.md) for the separate instruction
experiment; CPUID reporting and execution can differ under Rosetta.
