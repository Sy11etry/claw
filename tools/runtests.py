#!/usr/bin/env python3
"""Runs every test of every iteration against the freshly built compiler.

tests/run/*.claw  compiled and executed. Headers:
    // expect: <stdout line>     (repeatable)
    // exit: <code>              (default 0)
    // stderr: <substring>       (repeatable)
tests/err/*.claw  must be rejected. Header:
    // error: E####              (repeatable; all must appear, nothing else)

The compiler runs with an empty PATH, proving it needs no as, ld or clang.
"""
import os
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))


def headers(text, key):
    return re.findall(rf"^// {key}: ?(.*)$", text, re.M)


def run_test(path, tmp):
    text = path.read_text()
    exe = tmp / path.stem
    p = subprocess.run(
        [CLAW, "build", path, "-o", exe], capture_output=True, text=True, env={"PATH": ""}
    )
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    try:
        r = subprocess.run([exe], capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired:
        return "timed out"
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if r.returncode != want_exit:
        return f"exit code {r.returncode}, wanted {want_exit}\nstderr: {r.stderr}"
    want_out = headers(text, "expect")
    got_out = r.stdout.splitlines()
    if got_out != want_out:
        return f"stdout mismatch\n  got:  {got_out}\n  want: {want_out}"
    for s in headers(text, "stderr"):
        if s not in r.stderr:
            return f"stderr lacks {s!r}: {r.stderr!r}"
    return None


def err_test(path):
    text = path.read_text()
    want = sorted(c.strip() for c in headers(text, "error"))
    if not want:
        return "test has no // error: header"
    p = subprocess.run([CLAW, "check", path], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 1:
        return f"expected rejection, exit code {p.returncode}\n{p.stderr}"
    got = sorted(re.findall(r"\bE\d{4}\b", " ".join(l.split(" ")[1] for l in p.stderr.splitlines())))
    if got != want:
        return f"errors {got}, wanted {want}\n{p.stderr}"
    return None


def main():
    failures = 0
    total = 0
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        for path in sorted((ROOT / "tests/run").glob("*.claw")):
            total += 1
            msg = run_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL run/{path.name}: {msg}")
    for path in sorted((ROOT / "tests/err").glob("*.claw")):
        total += 1
        msg = err_test(path)
        if msg:
            failures += 1
            print(f"FAIL err/{path.name}: {msg}")
    print(f"{total - failures}/{total} passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
