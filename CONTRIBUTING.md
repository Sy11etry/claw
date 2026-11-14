# Contributing to Claw

Thank you for looking. Claw is written by an AI model and answered for by a person, and it welcomes contributions from
both kinds of author. The rules below are the same for everyone.

## Build and test

You need macOS on Apple Silicon and a Rust toolchain. Nothing else is downloaded: the compiler has no external crates.

```bash
cargo build --release --manifest-path compiler/Cargo.toml   # must build without warnings
python3 tools/quicktests.py                                 # about half a minute, needs only the compiler
python3 tools/runtests.py                                   # the full suite (see below)
```

`tools/runtests.py` is the full suite (about 2500 checks). Besides the compiler it needs Rosetta (for the x86-64 target),
`lldb`, `llvm-mc` from Homebrew's LLVM (only to check our own encoder against it) and, for the Linux target, `qemu`.
Please run it before you open a pull request that touches `compiler/`, `std/`, `selfhost/` or `tests/`.
If you cannot, say so in the pull request and we will run it.

## What a change must do

These come from the eight principles in the [README](README.md); a change that breaks one is probably the wrong change.

1. **The card stays small.** `docs/CARD.md` is the whole language and must stay under 2500 tokens
   (`tools/runtests.py` checks it). A new feature has to pay for its place on the card, or it waits.
2. **One way to do a thing, rarely two.** No new punctuation. Keywords are ordinary short English words.
3. **Every new compiler rule has a stable error code** `E####`, a test in `tests/err/` and, where it can be done safely,
   a ready patch for `claw fix`. Constructs that are reserved but not built give `E0900`, not a parse error.
4. **Tests come with the change.** `tests/run` (a program and its expected output in `// expect:` lines),
   `tests/err` (`// error: E####`), `tests/own` (allocations must equal frees). Old tests are never deleted.
5. **The interpreter must agree with machine code.** `claw why`, `claw test` and compile-time evaluation run on the
   interpreter; `tools/runtests.py` and `tools/fuzz_opt.py` compare it with real binaries.
6. **Keep the self-hosted compiler in step.** `selfhost/` must still build itself to a fixed point and produce the same
   bytes as `claw build --no-opt`. Every change to arm64 code generation in `compiler/src/codegen.rs` has a twin in
   `selfhost/codegen.claw`; see `selfhost/PORTING.md`.
7. **Decisions leave a trace.** A choice that someone could reasonably have made the other way goes into
   `docs/DESIGN.md` with its reason. `docs/STATUS.md` says what really works, including what does not.
8. **Do not promise speed you did not measure.** A forecast is called a forecast. Numbers go to `benchmarks/RESULTS.md`
   with the command that produced them.

## Code style

- Rust: no external crates, no `unsafe` unless the line says why, `cargo build` without warnings.
- Claw: `claw fmt` is the only style (`claw fmt --check` runs in the tests). Comments are in English.
- Python tools: the standard library only.

## Pull requests

Small and focused is better than large and clever. Describe what you changed, why, and how you checked it; link the
issue if there is one. For anything that changes the language, open an issue first: the card is small on purpose,
and it is cheaper to talk before the code than after.

## Reporting a bug

Use the issue form. The most useful report is the smallest program that shows it, the command you ran, what you expected
and what you got. If a program trapped, run it as `CLAW_RECORD=crash.clawtrace ./program` and attach the file:
`claw why crash.clawtrace program.claw` will replay it on our side.
