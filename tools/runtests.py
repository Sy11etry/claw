#!/usr/bin/env python3
"""Runs every test of every iteration against the freshly built compiler.

tests/run/*.claw  compiled and executed. Headers:
    // expect: <stdout line>     (repeatable)
    // exit: <code>              (default 0)
    // stderr: <substring>       (repeatable)
tests/err/*.claw  must be rejected. Header:
    // error: E####              (repeatable; all must appear, nothing else)
tests/fmt/x.claw  must format to tests/fmt/x.out.

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
        if s not in p.stderr + r.stderr:
            return f"stderr lacks {s!r}: {p.stderr + r.stderr!r}"
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


def fmt_test(path, tmp):
    """tests/fmt/x.claw must format to tests/fmt/x.out."""
    copy = tmp / path.name
    copy.write_text(path.read_text())
    p = subprocess.run([CLAW, "fmt", copy], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"fmt failed: {p.stderr}"
    got = copy.read_text()
    want = path.with_suffix(".out").read_text()
    return None if got == want else f"format differs\n--- got\n{got}--- want\n{want}"


def fmt_stable(path, tmp):
    """Formatting twice equals formatting once, for every program we have."""
    copy = tmp / ("fmt_" + path.name)
    copy.write_text(path.read_text())
    if subprocess.run([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
        return "first format failed"
    once = copy.read_text()
    if subprocess.run([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
        return "second format failed"
    return None if copy.read_text() == once else "formatting is not stable"


def card_size():
    """The language must fit in a prompt. A rough count, deliberately pessimistic."""
    text = (ROOT / "docs/CARD.md").read_text()
    tokens = len(re.findall(r"[A-Za-z_]+|\d+|[^\sA-Za-z_\d]", text))
    print(f"card: about {tokens} tokens (budget 2000)")
    return None if tokens <= 2000 else f"docs/CARD.md is about {tokens} tokens"


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
        for path in sorted((ROOT / "tests/fmt").glob("*.claw")):
            total += 1
            msg = fmt_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fmt/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "benchmarks").glob("*.claw"))):
            total += 1
            msg = fmt_stable(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fmt-stable/{path.name}: {msg}")
    for path in sorted((ROOT / "tests/err").glob("*.claw")):
        total += 1
        msg = err_test(path)
        if msg:
            failures += 1
            print(f"FAIL err/{path.name}: {msg}")
    total += 1
    msg = card_size()
    if msg:
        failures += 1
        print(f"FAIL card: {msg}")
    print(f"{total - failures}/{total} passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
