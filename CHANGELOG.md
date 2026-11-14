# Changelog

Every release is tagged `vX.Y` in git. Before 1.0 each version was one iteration of the plan the language was built by;
the reasons behind the choices are in `docs/DESIGN.md`, and what works today is in `docs/STATUS.md`.

## 1.0: first public release

- The version number moves from 0.13 to 1.0 and the repository is prepared for publication: MIT license, contributing and
  security notes, CI, issue and pull-request templates, a portable
  quick test suite (`tools/quicktests.py`, 686 checks in about 20 seconds), `tools/install.py --no-path`.
- The specification, the status page and the design notes (`docs/DESIGN.md`) are in English, and so is everything else in
  the repository.
- No change to the language or the compiler since 0.13. The flight recorder costs about 0.5% on a program that reads
  15000 files in a loop (measured once, on one machine).

## 0.13: the flight recorder

- `CLAW_RECORD=crash.clawtrace ./program` leaves a small file when the program ends in a trap: the arguments, the
  environment variables and the files it read. `claw why crash.clawtrace program.claw` replays that crash in the
  interpreter, on any machine, and compares the failure with the recorded one.
- The card is allowed 2500 tokens (was 2000) and now covers SIMD, C structs by value, `IoErr`, `for v, i`, slices and the
  recorder.
- Printing floating-point numbers is about twice as fast, with identical output; `5e-324` and other denormals print
  correctly again.
- `claw fix` gives the right advice for an unused local variable: write `_ = f()`.

## 0.12: fixes from the review

- Claude Haiku wrote 290 programs against 0.11 and recorded 45 findings; this release closes
  all of them except the ones that need a new target (Linux x86-64, Windows).
- C structs by value in calls to C libraries; a stack overflow is reported instead of a silent crash; floating-point
  numbers print in their shortest form that reads back exactly.

## 0.11: C libraries, threads, packages

- `use c "zlib.h"` and `link "z"`: call a C library with no hand-written declarations.
- `par.map` and `par.build`: spread work over all cores with no shared memory, so no data races.
- Scripts (`#!/usr/bin/env claw`), packages (`claw.pkg`, `claw get`, `claw.lock`).

## 0.10: more targets and SIMD

- SIMD vectors (`f64x2`, `f32x4`, `i32x4`) in `claw fn`; debug information (`-g`) for `lldb`; a language server
  (`claw lsp`).
- The x86-64 target (runs under Rosetta) and the ARM64 Linux target (ELF).
- Threads without data races (`use par`).

## 0.9: by itself

- `selfhost/`: the compiler written in Claw. It compiles itself to a fixed point, and its output is byte for byte
  what the Rust compiler writes with `claw build --no-opt`.

## 0.8: thrift

- `claw card`, `claw sig`, `claw ctx`, `claw audit`; the token benchmark (`benchmarks/TOKENS.md`) and the exam for a weak
  model (`benchmarks/EXAM.md`).

## 0.7: the claw's trail

- An interpreter for the compiler's own intermediate form. On it stand compile-time evaluation, automatic tests
  (`claw test`), and `claw why`, which names the lines that led to a failure. `claw fix` applies the compiler's patches.

## 0.6: speed

- An optimizer: SSA, inlining, register allocation, removal of checks the compiler can prove. See
  `benchmarks/RESULTS.md` for measurements against `clang`.

## 0.5: honest signatures

- Contracts (`need`, `sure`), effects (`+mem`, `+io`), `test` blocks, a checked mode with a memory checker for `claw fn`.

## 0.4: paw mode

- Ordinary `fn` with automatic memory: `[]T`, `str`, ownership (`own`, `&`), `drop`, a standard library written in Claw.

## 0.3: claw mode

- `claw fn`: raw pointers, `alloc` / `free`, `defer`, `extern`, generics, modules, calls into C.

## 0.2: the skeleton

- All number types, floats, structs, arrays, variants, `match`, `?T`, `T ! E`, strings, the formatter, the error format.

## 0.1: first scratch

- Functions, integers, `if`, `for`, `ret`, `print`, overflow trapping. Our own ARM64 encoder, Mach-O writer and
  code signature: `claw run fib.claw` works without `clang` or `ld`.
