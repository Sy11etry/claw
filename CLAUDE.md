# Claw

A low-level compiled language written entirely by Claude. The question it answers:
what systems language do you need when an AI writes the code and a human answers for it?

What really works: `docs/STATUS.md`. The exact rules: `docs/SPEC.md`. Why it is shaped this way: `docs/DESIGN.md`.
Since 1.0 the project is public (MIT): `README.md`, `CONTRIBUTING.md` and `CHANGELOG.md` are written for strangers.
Version 1.0 is the same compiler as 0.13, prepared for publication.

## Principles (check every feature against them)

1. The language fits in a prompt: the core `docs/CARD.md` is at most 2500 tokens (it was 2000 before 0.13). A feature pays for its place.
2. Paw and Claw: `fn` is safe and has no manual memory, `claw fn` is plain C. No runtime, no GC.
3. Contracts `need/sure`: prove or check, never stay silent.
4. The compiler writes the tests: fuzzing of functions without effects.
5. `claw why`: the cause of a failure, not an address.
6. The signature tells the truth: effects `+mem +io`.
7. Count the tokens of the whole loop: write, read, fix.
8. From source to binary there is only our code: no LLVM, `as`, `ld`.

Syntax rule: no strange punctuation, freshness lives in the meanings. One or two ways to do one thing.
Keywords: ordinary short English words, one token each.

## Commands

```bash
cargo build --release --manifest-path compiler/Cargo.toml   # build, no warnings
python3 tools/runtests.py                                   # every test of every iteration; tests/own compare alloc and free
python3 tools/quicktests.py                                 # a quick portable slice (no Rosetta, lldb, llvm-mc); CI runs it too
python3 tools/check_encoder.py                              # the ARM64 encoder against llvm-mc
python3 benchmarks/run.py                                   # measurements against clang
python3 tools/tokens.py                                     # token estimate: Claw against C and Rust, writes benchmarks/TOKENS.md
python3 tools/exam.py <folder> [--tasks=tasks2.json]        # grade the answers of the exam on the card
python3 tools/install.py                                    # install a version into ~/.claw (--no-path leaves your shell alone)
```

The compiler in Claw itself: `selfhost/` (porting rules in `selfhost/PORTING.md`). The measure: the same bytes as `claw build --no-opt`; `tools/runtests.py` checks that it builds itself to a fixed point.

## Layout

`compiler/src`: `lexer → parser → ast → sema → effects → lower → cir → codegen (arm64) → macho + sha256`, errors in `diag`.
Modes: `claw build` without contracts that call functions; `claw run` and `claw test` with every check and `std/check_rt.claw`.
The interpreter `interp` executes CIR (lowering mode `Opts.interp`): `ctfe` (evaluating `const`), `fuzz` (automatic tests) and `why` (the cause of a failure) stand on it. It must agree with the machine code: `tools/runtests.py` and `tools/fuzz_opt.py` check this.
Stage 0 is in Rust with no external crates. The target is macOS ARM64 first; x86-64 and ARM64 Linux are secondary.

Check the Mach-O format and signatures against real `clang` output (`otool -l`, `codesign -dvvv`, `dyld_info`), not from memory.

## Checklist at the end of a release

1. `cargo build --release` without warnings.
2. `tools/runtests.py`: all tests, including the tests of earlier iterations. Never delete old tests.
3. `tools/install.py`, then `claw-X.Y --version` from a new shell. **Never delete old versions in `~/.claw`.**
4. Update `docs/STATUS.md`, `CHANGELOG.md` and `benchmarks/RESULTS.md`.
5. git commit and a tag `vX.Y`.
6. Report to the user in plain language: what of the syntax works, what does not.

## How to work

- The user knows C, C++ and Python: explain through them, simply. The user's language is Russian; the repository is English only.
- No more than one subagent at a time unless the user asked otherwise.
- Do not promise speed without measurements: a forecast is called a forecast.
- Write significant decisions into `docs/DESIGN.md`.
- Every new compiler rule: a stable error code `E####`, a test in `tests/err`, a ready patch where possible.
- Reserved but not yet implemented constructs give `E0900`, not a strange parse error.
