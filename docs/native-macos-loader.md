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
x86-64 macOS. It exposes only `xg_xs.dll!XGSuggestSwizzleMode` and
`XGCreateTextureComputer`, through Windows x64 ABI wrappers using the same
implementations as Windows HLE. The returned texture computer has wrappers
for all 13 upstream slots; slots upstream does not implement still return
its existing `E_NOTIMPL` behavior. Every other import remains unresolved.
The mode-selection rule is upstream's older optimal-mode approximation, not
verified complete console behavior. The general Windows HLE registry and
other Xbox APIs are not exposed.

With the LLVM-MinGW and Rust setup below, run the synthetic guest test with
`cargo test -p victory-xg --target x86_64-apple-darwin --test native_guest`.
It checks texture decisions, null-pointer errors, a texture-computer layout,
tile/detile round trip, an unsupported slot, and reference-counted release
through PE imports under Rosetta, without game data.

Separately, `victory_runtime::NativeResolver` exposes only
`xgameruntime.dll!QueryApiImpl` for the two upstream XSystem interface IDs.
The returned Windows-ABI object supports the upstream offline console ID,
sandbox ID and device ID string methods. Other provider/interface pairs and
all other imports remain unsupported. Its strings and buffer-error behavior
are shared with the Windows implementation; this is not the full runtime.
Run its synthetic PE test with
`cargo test -p victory-runtime --target x86_64-apple-darwin --test native_guest`.

`victory_native::NativeResolver` combines these providers and a restricted
`d3d12_xs.dll!D3D12XboxCreateDevice` bridge for the native loader. It leaves
unimplemented imports missing. This is not the Windows HLE registry.
The synthetic executable integration test imports and calls both providers,
checks that an unimplemented import fails to load, and runs without game data:
`cargo test -p victory-native --target x86_64-apple-darwin --test native_executable`.

The D3D12.X bridge uses typed Windows-x64 entry points for limited compute and
offscreen graphics device, descriptor heap, resource, pipeline, allocator,
list, queue and fence paths backed by the existing Metal implementation.
Unsupported vtable slots abort the process rather than silently succeed or call
host-ABI pointers.
Guest DLL allocation hooks and the Windows runtime page mapper are not
installed by this native bridge; it uses the GPU implementation's own
allocation path. This is not complete D3D12.X, Windows startup, or game support.
With a trusted Apple Metal Shader Converter dylib, LLVM-MinGW and a Metal GPU:

```sh
export VICTORY_METAL_CONVERTER=/path/to/libmetalirconverter.dylib
ulimit -c 0
cargo test -p victory --target x86_64-apple-darwin --test native_compute -- --ignored
```

The synthetic test invokes the PE through the CLI, dispatches its own console
shader through VIctory's translation and Metal backend, verifies readback 42,
and checks an unsupported device slot terminates the child process.

The native graphics fixture exercises a restricted vertex/pixel pipeline,
render-target clear, two triangle draws with different root constants, and a
texture-to-buffer copy with padded rows through the same CLI. It checks the
readback pixels and that an unsupported list slot aborts safely.
A second synthetic PE samples two RGBA8 textures using shader-visible SRV and
sampler heaps and graphics root descriptor tables. It switches point and
linear filtering and textures between draws, releases caller references before
submission, changes texture contents after recording, and checks the sampled
pixels and row padding on readback. It also checks an unsupported device
method terminates the child. This covers the existing restricted sampled
texture path, not general descriptor or sampler support.
The indexed fixture also draws 16- and 32-bit triangle-list indices with
nonzero view offsets, start indices and a negative base vertex. It checks
pixels after releasing the index resource, rejects invalid formats, ranges,
alignment and instancing at list close, and rejects out-of-range effective
indices at submission. Only nonempty, non-instanced triangle lists using a
float4 vertex view are supported; other formats and topologies are rejected.
With the same prerequisites, run:

```sh
cargo test -p victory --target x86_64-apple-darwin --test native_graphics -- --ignored
cargo test -p victory --target x86_64-apple-darwin --test native_indexed -- --ignored
```

On Apple M3 Max, macOS 27.3 and Rosetta, this focused test passed with the
Apple Metal Shader Converter and LLVM-MinGW. These synthetic shaders are not
game shaders, and this result does not establish game compatibility.

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

## Trusted fixture CLI

An x86-64 macOS build of `victory` exposes `native-fixture`. On Apple Silicon,
install Rosetta, the Rust `x86_64-apple-darwin` target and Apple command-line
tools, then build or run that target. For a self-authored PE executable:

```sh
cargo run -p victory --target x86_64-apple-darwin -- native-fixture /path/to/fixture.exe
cargo run -p victory --target x86_64-apple-darwin -- native-fixture /path/to/fixture.exe --execute
cargo run -p victory --target x86_64-apple-darwin -- native-fixture /path/to/fixture.exe --dll /path/to/consumer.dll --dll /path/to/provider.dll --execute
```

Without `--execute`, the command maps and binds but calls neither the
executable entry nor DLL attach functions. Repeat `--dll` to supply each
trusted dependency explicitly. The UTF-8 basename is its import name;
matching and duplicate-name rejection use ASCII case-insensitivity. Two
paths with the same basename are rejected even if they are in different
directories. The loader attaches DLLs in dependency order and detaches in
reverse order; cycles, forwarded exports, unsupported images and missing
imports are rejected. DLLs are not searched for or loaded dynamically.

With `--execute`, executable and DLL code runs **unsandboxed in the CLI
process** and can crash, hang or corrupt it. Only pass trusted images with
a normally returning `extern "win64" fn() -> u32` executable entry, and
ensure every DLL, callback and thread lifetime satisfies the safety
requirements above. The returned integer is printed, not used as the CLI
exit status. Only the limited native services above are exposed. This does
not supply a Windows environment, start Windows processes, or support games.

With LLVM-MinGW on `PATH` as in the test setup below, the following builds
and runs self-authored synthetic executables and DLLs, checks dependency
attachment/detachment order and opt-in, and exercises failure paths:

```sh
cargo test -p victory --target x86_64-apple-darwin --test native_fixture
```

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
establish a supported way to install a Windows TEB. A zero value at
`gs:[0x58]` is a zero *slot value*, not evidence that GS base is zero.

The unresolved issue is how to expose the Windows TLS array at that address
without corrupting macOS thread-specific data or depending on an undocumented
host ABI. A [Wine developer's macOS investigation](https://list.winehq.org/hyperkitty/list/wine-gitlab@list.winehq.org/message/NROQNDTC4ZB2MNFQGQPC3BEQHTINZPVT/)
describes both a compatibility-slot approach and a private GS-base switching
approach, including the need to switch around host calls and signal handlers.
Neither establishes a public, stable contract for this loader; Apple's
[TSD implementation](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/os/tsd.h)
itself warns that its implementation details are not ABI. In particular,
writing a zero-valued slot is not proof that the slot is available.

A viable design needs either a supported host interface for that slot, or a
controlled execution engine that correctly translates all guest GS accesses
without scanning arbitrary instruction bytes. It must also copy the PE TLS
template and zero-fill, assign module indices, initialize an array for each
participating thread, run callbacks and DLL notifications in the correct
order, and keep mappings and blocks alive through teardown. The PE parser
exposes and validates the template RVA range, zero-fill count, index RVA and
callbacks; parsing does not allocate or execute TLS. The native loader has no
managed thread lifecycle, and the existing controlled-entry API cannot enforce
setup for threads created outside it. Until an execution model and lifecycle
satisfy these constraints and are tested together, images containing TLS
remain explicitly rejected.

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
