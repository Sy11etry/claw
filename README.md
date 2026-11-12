# Claw

**A low-level compiled language written entirely by Claude, built around one question:
what systems language do you need when an AI writes the code and a human answers for it?**

Claw compiles straight to a signed native executable. No LLVM, no assembler, no linker: the compiler
encodes the machine code, writes the Mach-O / ELF file and signs it itself. No runtime, no garbage collector.

```claw
fn avg(xs []int) int
    need len(xs) > 0                 // what the function asks for
{
    mut total := 0
    for x in xs { total += x }
    ret total / len(xs)
}

fn main() {
    print(avg([4, 8, 15]))
    print(avg([]))                   // breaks the contract
}
```

```
$ claw run avg.claw
trap: need failed: `len(xs) > 0` in avg at avg.claw:11:11
  left = 0
  xs = []
$ claw why avg.claw
FAIL need failed: `len(xs) > 0` in avg at avg.claw:11:11
why    avg.claw:2  need len(xs) > 0
       avg.claw:11  print(avg([]))    left = 0, xs = []
```

## Eight ideas

1. **The whole language fits in a prompt.** `docs/CARD.md` is the complete reference and stays under 2000 tokens
   (`claw card` prints it). Every feature has to pay for its place on the card.
2. **Paw and Claw.** `fn` is safe: no pointers, no `free`, memory is released by the compiler at the end of the scope.
   `claw fn` is plain C: raw pointers, `alloc`/`free`, `defer`. One language, two gears, `claw audit` lists every spot where
   the compiler has to trust you.
3. **Contracts.** `need` / `sure` are documentation, check, test and optimizer hint in one. The compiler proves what it can
   and removes the check; it never stays silent about the rest.
4. **The compiler writes the tests.** `claw test` fuzzes every function without side effects: boundary values, random inputs,
   inputs picked by code coverage, and shrinks a failure to a small example.
5. **`claw why`.** Not an address and a stack, but the lines that led to the failure and the values on them.
6. **The signature tells the truth.** `+mem +io` mark allocation and I/O; no plus means a guarantee. A reader (or a model)
   does not have to read the body.
7. **We count tokens of the whole loop:** write, read, fix. Short words, no `;`, compiler messages of about 30-45 tokens, most with a ready patch.
8. **Source to binary is our code only.** The compiler is a Rust program with zero dependencies, and it is also
   written in Claw (`selfhost/`) and compiles itself to a fixed point.

## Install and build

You need macOS on Apple Silicon and a Rust toolchain.

```bash
cargo build --release --manifest-path compiler/Cargo.toml
python3 tools/install.py          # puts claw into ~/.claw/bin (add it to PATH)
claw run examples/tour.claw
```

| Command | What it does |
|---|---|
| `claw run f.claw` | compile with every check and run |
| `claw build f.claw -o out` | optimized executable; `--fast` removes every check, `-g` adds debug info |
| `claw test f.claw` | your `test` blocks, then automatic tests for every function without `+io` |
| `claw why f.claw` | which lines led to the failure |
| `claw fix f.claw` | apply the compiler's safe patches |
| `claw sig` / `ctx` / `audit` / `card` | signatures only / one function's context / trusted code / the language on a page |
| `claw fmt f.claw` | the one standard format |
| `claw lsp` | language server for editors |
| `claw f.claw args` | build and run a script at once (`#!/usr/bin/env claw` works) |
| `claw get` | fetch the packages listed in `claw.pkg` and write `claw.lock` |
| `claw cdecl x.h` | show what `use c "x.h"` turns a C header into |

C libraries: `use c "zlib.h"` and `link "z"` (or `-l z`) call a C library with no hand-written declarations; tried on libc, zlib,
sqlite3 and a library built from our own C. Threads: `par.map(xs, f)` and `par.build(n, f)` spread work over all cores with no
shared memory, so no data races, and the answer is always the one a plain loop would give.

Targets: macOS arm64 (default), macOS x86-64 (`--target x86_64`, runs under Rosetta), ARM64 Linux
(`--target aarch64-linux-musl` or `aarch64-linux`, builds only).

## Numbers (measured on one Apple Silicon Mac, see `benchmarks/RESULTS.md`)

- Against `clang -O2`, with the safety checks the compiler could not prove away still on: `primes` 1.0x, `sieve` 1.1x, `mandel` 1.4x, `collatz` 1.4x,
  `fib` 1.9x, `matmul` 2.8x; hand-written SIMD `matmul_simd` 1.1x.
- Compile speed: about 320,000 lines per second, to a signed binary.
- Tokens, Claw against C and Rust over 21 small tasks (an estimate, not a real tokenizer):
  37.9% fewer than C and 17.8% fewer than Rust. Compiler messages for the same mistake: 288 tokens in total
  against 651 for clang and 1343 for rustc (`benchmarks/TOKENS.md`).
- An exam: Claude Haiku wrote Claw from the card alone, one try per program: 19 of 24 on the first round and 11 of 12 on a
  new set after the card was fixed (`benchmarks/EXAM.md`; small sample, written by the language's author).

## Honest limits

Claw is a young language. The full list is in `docs/STATUS.md`; the main points:

- Tested on macOS arm64 only in earnest. The x86-64 target is checked under Rosetta, not on a real Intel chip. The Linux
  target runs in a virtual machine with musl; a glibc build is produced but has not been run.
- No automatic vectorization (SIMD is written by hand in `claw fn`). Floating-point values on x86-64 live on the stack.
- `clawc`, the compiler written in Claw, is a subset: it builds everything in `tests/run` byte for byte but does not know
  threads, SIMD, C headers, packages, Linux, `-g`, `test`, `why` or `fix`. The Rust compiler `claw` is the main one.
- Packages are folders or git URLs with a lock file: no registry, no version ranges. The standard library is what `std/` shows (`claw sig std`).
- Not done: Windows, WebAssembly, Linux on x86-64, automatic vectorization, channels between threads.

## Repository

| Path | What |
|---|---|
| `compiler/` | the Rust compiler (stage 0), no external crates |
| `selfhost/` | the compiler written in Claw |
| `std/` | the standard library, in Claw |
| `examples/` | short showcase programs (they are tests too) |
| `tests/` | about 2500 tests: run, error codes, contracts, ownership, fuzzing, debug info, SIMD, threads |
| `benchmarks/` | Claw and C versions of each benchmark, measurement and token scripts |
| `tools/` | test runner, encoder checks, fuzzers, installer |
| `docs/` | `CARD.md` (the language), `SPEC.md`, `STATUS.md` (what works), `DECISIONS.md` (why), `PLAN.md` |

The design documents are written in Russian; the card, the examples, the compiler messages and the code comments are in English.

Run everything:

```bash
python3 tools/runtests.py
```
