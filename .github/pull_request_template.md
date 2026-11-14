## What and why

<!-- One or two sentences. For language changes, link the issue where it was discussed. -->

## How it was checked

- [ ] `cargo build --release --manifest-path compiler/Cargo.toml` has no warnings
- [ ] `python3 tools/quicktests.py` passes (or `tools/runtests.py` for changes to the compiler, std or selfhost)
- [ ] New compiler rule: stable `E####`, a test in `tests/err`, a patch for `claw fix` if it is safe
- [ ] `docs/CARD.md` is still under 2500 tokens; `docs/STATUS.md` and `docs/DESIGN.md` updated if behaviour changed
