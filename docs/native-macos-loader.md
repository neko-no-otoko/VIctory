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

`victory_xg::NativeResolver` is a separate, deliberately limited resolver for
x86-64 macOS. It exposes only `xg_xs.dll!XGSuggestSwizzleMode`, through an
explicit Windows x64 ABI wrapper using the same selection logic as the
Windows HLE implementation. Every other import remains unresolved. The
mode-selection rule is upstream's older optimal-mode approximation, not
verified complete console behavior. It does not expose the Windows HLE
registry, COM objects, or other Xbox APIs.

With the LLVM-MinGW and Rust setup below, run the synthetic guest test with
`cargo test -p victory-xg --target x86_64-apple-darwin --test native_guest`.
It checks four texture decisions and null-pointer errors through a real PE
import under Rosetta, without game data.

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

## Static TLS remains unsupported

The Windows loader's existing TLS rewrite is not a macOS TLS implementation:
it expects Windows to allocate the per-thread TLS array and its generated
thunks still read `gs:[0x58]`. Its byte matcher covers one instruction form
and is not a decoder. For example, the byte sequence
`48 b8 65 48 8b 04 25 58 00 00 00 c3` decodes as a `movabs` followed by an
`add`, but the matcher sees its nine-byte GS pattern at offset 2, inside the
`movabs` immediate. Applying that rewrite to arbitrary code can corrupt it.

In isolated child-process experiments on an Apple M3 Max running macOS 27.3
under Rosetta, `gs:[0x58]` read zero, and both `rdgsbase` and `wrgsbase`
raised SIGILL; CPUID did not advertise FSGSBASE. Reading Mach's translated
`x86_THREAD_STATE64` succeeded and showed a zero GS selector, but did not
establish a supported way to install a Windows TEB. These observations do
not rule out other emulation or rewriting designs. Correct support would
require safe instruction identification, PE TLS template/index handling,
per-thread storage, and process/thread callback ordering, including threads
not created by this loader. Until those are implemented and tested together,
images containing TLS remain explicitly rejected.

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
