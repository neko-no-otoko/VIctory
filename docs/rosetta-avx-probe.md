# Synthetic Rosetta AVX probe

Apple's [Rosetta documentation](https://developer.apple.com/documentation/apple-silicon/about-the-rosetta-translation-environment) says Rosetta translates AVX and AVX2 instructions, but not AVX-512. Rosetta translates an entire x86-64 process; it cannot mix arm64 and x86-64 code in one process.

This diagnostic reports CPUID bits and then executes one AVX and one AVX2
instruction in separate child processes, checking the results. Each child
uses the default SIGILL handler so an unsupported instruction does not kill
the parent. It uses no game data. On Apple silicon with Rosetta and macOS
Command Line Tools installed, from the repository root:

```sh
ulimit -c 0
/usr/bin/clang -arch x86_64 -O2 -Wall -Wextra tools/rosetta-avx-probe.c \
  -o /tmp/rosetta-avx-probe
/tmp/rosetta-avx-probe
```

On one Apple M3 Max running macOS 27.3, the process reported `translated=1`
and CPUID bits `AVX=0 OSXSAVE=0 AVX2=0`, while both sampled instructions
executed and their results validated. CPUID flags and instruction execution
are distinct observations; this does not establish support for the entire
instruction sets, a particular executable, or game compatibility. The probe
returns success when it completes and reports SIGILL as an observed outcome;
it does not use its exit status as an instruction-availability verdict.
