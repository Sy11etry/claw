#!/usr/bin/env python3
"""A quick, portable slice of the test suite: macOS arm64 only, nothing but the compiler itself.

`tools/runtests.py` is the full suite. It also needs Rosetta (x86-64), lldb, llvm-mc and, for the Linux target, qemu.
This script runs what needs none of those, so it works on a plain Apple Silicon machine and in CI:

    tests/run, tests/own   built (optimized) and run; stdout and exit code must match the headers
    tests/check            through `claw run`, with every contract and the memory checker
    tests/err              must be rejected with exactly the listed error codes
    examples               built and run
    docs/CARD.md           `claw card` prints it, and it fits the token budget
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import runtests as rt  # noqa: E402


def main():
    root = rt.ROOT
    runs = sorted((root / "tests/run").glob("*.claw"))
    owns = sorted((root / "tests/own").glob("*.claw"))
    checks = sorted((root / "tests/check").glob("*.claw"))
    errs = sorted((root / "tests/err").glob("*.claw"))
    examples = sorted((root / "examples").glob("*.claw"))
    jobs = [(f"{p.parent.name}/{p.name}", lambda p=p: rt.in_temp(rt.run_test, p, ())) for p in examples + runs]
    jobs += [(f"check/{p.name}", lambda p=p: rt.check_test(p)) for p in checks]
    jobs += [(f"err/{p.name}", lambda p=p: rt.err_test(p)) for p in errs]
    jobs += [("card", rt.card_size), ("card command", rt.card_command)]
    results = rt.parallel(jobs)
    # Programs that count allocations write to a fixed file, so they run in a second batch.
    results += rt.parallel([(f"own/{p.name}", lambda p=p: rt.in_temp(rt.own_test, p, ())) for p in owns])
    failed = [(label, msg) for label, msg in results if msg]
    for label, msg in failed:
        print(f"FAIL {label}: {msg}")
    print(f"{len(results) - len(failed)}/{len(results)} passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
