# Experimental x86-64 macOS PE mapping

`victory_loader::macos::Image` is a small PE32+ mapping foundation for an
**x86-64 macOS process**, including one running under Rosetta on Apple
silicon. Rosetta translates the whole process; this is not a native arm64 PE
loader. It does not run GTA VI or implement an Xbox or Windows environment.

The mapper copies headers and sections, applies `DIR64` base relocations,
resolves ordinary imports through an `ImportResolver`, and sets page
permissions from section flags. It rejects writable/executable pages,
unresolved imports, unsupported relocations, TLS, Windows unwind tables,
load configuration and delay imports. Loading does not invoke code. An
explicit unsafe `Image::attach` calls the DLL entry point with
`DLL_PROCESS_ATTACH`; after successful attachment, unsafe `Image::detach`
calls `DLL_PROCESS_DETACH` with a null reserved pointer. A failed attach is
not followed by detach. Duplicate attach or detach is rejected. An image
without an entry point still has explicit attach/detach state.

`ModuleSet::load` maps an explicit, nonempty set of named DLLs in dependency
order, resolving imports from members before using the external resolver.
Cycles, duplicate names and imported forwarded exports are rejected. Unsafe
`ModuleSet::attach` initializes dependencies first and rolls back prior
attaches if an initializer fails; `detach` runs in reverse order. The set
owns its mappings until drop; callers must explicitly detach before dropping
an attached set, and must not use its export pointers afterward.

`Executable::load` also maps a PE executable with explicitly supplied DLLs,
without executing code. Unsafe `Executable::start` attaches the DLLs, calls
the executable's actual entry RVA, and detaches the DLLs if that call returns.
It is limited to trusted, controlled entry points with the exact signature
`extern "win64" fn() -> u32` that return normally; it can be called only once.
The caller must ensure DLL and callback safety and that no threads or other
users remain when the entry returns. A crash, process exit or nonreturning
entry cannot be handled by this API.

This is **not Windows process startup**. It does not construct the Windows
process parameters, PEB/TEB, startup stack, CRT, TLS, exception handling or
Windows APIs. An ordinary Windows or Xbox executable cannot be assumed to
meet the controlled entry contract, and this is not title execution support.
The same unsupported-image checks apply to executables and DLLs.

The caller must ensure the DLL and all callbacks are trusted, ABI-correct,
properly initialized and safely serialized, and that users are quiescent
before detach. Windows loader-lock semantics, thread notifications, the TEB,
GS-based TLS, exception handling, dynamic DLL loading and Windows APIs are not
provided. The caller must detach before dropping an attached image: `Drop`
only unmaps memory and never invokes arbitrary code. Export addresses remain
valid only while the image is alive. Calling an export executes arbitrary
machine code.

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
invalid entry points, attach failure, module and reserved arguments, and
exactly-once detach. A two-DLL synthetic test checks cross-module imports,
dependency initialization, reverse teardown, failure rollback, cycles and
forwarded-export rejection. A separate test maps and calls a self-authored PE
executable entry after binding and attaching its synthetic DLL dependency.
Passing these tests is not evidence of title execution, D3D12 translation, or
Metal support. See
[rosetta-avx-probe.md](rosetta-avx-probe.md) for the separate instruction
experiment; CPUID reporting and execution can differ under Rosetta.
