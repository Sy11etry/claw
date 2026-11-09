#!/usr/bin/env python3
"""Runs every test of every iteration against the freshly built compiler.

tests/run/*.claw  compiled and executed. Headers:
    // expect: <stdout line>     (repeatable)
    // exit: <code>              (default 0)
    // stderr: <substring>       (repeatable)
tests/err/*.claw  must be rejected. Header:
    // error: E####              (repeatable; all must appear, nothing else)
tests/own/*.claw  like run, built with --alloc-stats: allocations must equal frees.
tests/check/*.claw  like run, but through `claw run`: every contract and the memory checker.
tests/unit/*.claw  through `claw test --blocks`: the headers describe its report.
tests/run and tests/own again, in the interpreter: it must agree with machine code.
tests/auto/*.claw  through `claw test`: generated inputs, shrunk failures and their causes.
tests/why/*.claw  through `claw why`: the report names the lines that matter.
tests/fix/x.claw  `claw fix` must turn it into tests/fix/x.out, or leave it and report.
tests/run again, in check mode: the checker must not complain about correct programs.
tests/fast/*.claw  built with --fast; tests/stats/*.claw  the optimizer proves what it should.
tools/fuzz_opt.py  random programs: optimized and unoptimized builds must agree.
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


def child(cmd, timeout=120, **kw):
    """Every child has a time limit: a compiler that hangs is a failed test, not a stuck run."""
    try:
        return subprocess.run(cmd, timeout=timeout, **kw)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, -9, "", f"timed out after {timeout} s")


def headers(text, key):
    return re.findall(rf"^// {key}: ?(.*)$", text, re.M)


def run_test(path, tmp):
    text = path.read_text()
    exe = tmp / path.stem
    p = child(
        [CLAW, "build", path, "-o", exe], capture_output=True, text=True, env={"PATH": ""}
    )
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
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


def check_test(path):
    """`claw run`: the mode with every contract and the memory checker."""
    text = path.read_text()
    p = child([CLAW, "run", path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout mismatch\n  got:  {p.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    for s in headers(text, "stderr"):
        if s not in p.stderr:
            return f"stderr lacks {s!r}: {p.stderr!r}"
    return None


def run_again_checked(path):
    """A correct program behaves the same with the checker switched on."""
    text = path.read_text()
    if headers(text, "build-only"):
        return None
    p = child([CLAW, "run", path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "checked-exit") or headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout differs: {p.stdout.splitlines()}"
    return None


def interp_test(path):
    """The interpreter must agree with real machine code, or `why` and the fuzzer mean nothing."""
    text = path.read_text()
    if headers(text, "build-only"):
        return None
    p = child([CLAW, "__interp", path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    if p.returncode == 91:
        return None  # a C function the interpreter does not carry
    want_exit = int((headers(text, "checked-exit") or headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout differs: {p.stdout.splitlines()}"
    for s in headers(text, "stderr"):
        if s.startswith("trap: ") and "out of memory" not in s and s not in p.stderr:
            return f"stderr lacks {s!r}: {p.stderr!r}"
    return None


def unit_test(path):
    text = path.read_text()
    p = child([CLAW, "test", "--blocks", path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"report differs\n  got:  {p.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    return None


def auto_test(path):
    """`claw test` with generated inputs: the whole report is compared."""
    text = path.read_text()
    p = child([CLAW, "test", path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return "report differs\n  got:  " + "\n        ".join(p.stdout.splitlines()) + "\n  want: " + "\n        ".join(headers(text, "expect"))
    return None


def why_test(path):
    """`claw why`: the lines of cause are compared exactly."""
    text = path.read_text()
    cmd = [CLAW, "why", path]
    for name in headers(text, "test"):
        cmd += ["--test", name]
    p = child(cmd, capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "exit") or ["1"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return "report differs\n  got:  " + "\n        ".join(p.stdout.splitlines()) + "\n  want: " + "\n        ".join(headers(text, "expect"))
    return None


def fix_test(path, tmp):
    """`claw fix` must produce tests/fix/x.out; applying it again must change nothing."""
    text = path.read_text()
    copy = tmp / path.name
    copy.write_text(text)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    p = child([CLAW, "fix", copy], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\n{p.stdout}{p.stderr}"
    got = copy.read_text()
    want = path.with_suffix(".out").read_text()
    if got != want:
        return f"result differs\n--- got\n{got}--- want\n{want}"
    child([CLAW, "fix", copy], capture_output=True, env={"PATH": ""})
    return None if copy.read_text() == got else "a second `claw fix` changed the file"


def fast_test(path, tmp):
    """`claw build --fast`: still correct, but nothing is checked."""
    text = path.read_text()
    exe = tmp / ("fast_" + path.stem)
    p = child([CLAW, "build", path, "-o", exe, "--fast"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        return f"exit code {r.returncode}\nstderr: {r.stderr}"
    if r.stdout.splitlines() != headers(text, "expect"):
        return f"stdout mismatch\n  got:  {r.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    return None


def stats_test(path, tmp):
    """The optimizer must really remove the checks it claims to be able to prove."""
    text = path.read_text()
    exe = tmp / ("stats_" + path.stem)
    p = child([CLAW, "build", path, "-o", exe, "--stats"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    m = re.search(r"checks: (\d+) of (\d+) removed", p.stdout)
    if not m:
        return f"no statistics: {p.stdout!r}"
    least = headers(text, "removes-at-least")
    if least and int(m[1]) < int(least[0]):
        return f"removed only {m[1]} of {m[2]}"
    if re.search(r"^// removes-all-bounds", text, re.M):
        dump = child([CLAW, "build", path, "-o", exe, "--emit-cir"], capture_output=True, text=True, env={"PATH": ""})
        if "BoundsDyn" in dump.stdout or "Bounds {" in dump.stdout:
            return "a bounds check survived"
    return None


def fuzz(count):
    """Random programs: the optimized build must behave exactly like the unoptimized one."""
    p = child([sys.executable, ROOT / "tools/fuzz_opt.py", str(count), "1"], capture_output=True, text=True, timeout=3600)
    return None if p.returncode == 0 else p.stdout[-800:]


def own_test(path, tmp):
    """Like a run test, but every allocation must also have been freed."""
    text = path.read_text()
    exe = tmp / ("own_" + path.stem)
    p = child([CLAW, "build", path, "-o", exe, "--alloc-stats"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        return f"exit code {r.returncode}\nstderr: {r.stderr}"
    want_out = headers(text, "expect")
    if r.stdout.splitlines() != want_out:
        return f"stdout mismatch\n  got:  {r.stdout.splitlines()}\n  want: {want_out}"
    m = re.search(r"allocs=(\d+) frees=(\d+)", r.stderr)
    if not m:
        return f"no allocation report: {r.stderr!r}"
    if m[1] != m[2]:
        return f"leak or double free: {m[0]}"
    least = headers(text, "allocs-at-least")
    if least and int(m[1]) < int(least[0]):
        return f"expected at least {least[0]} allocations, saw {m[1]}"
    return None


def err_test(path):
    text = path.read_text()
    want = sorted(c.strip() for c in headers(text, "error"))
    if not want:
        return "test has no // error: header"
    p = child([CLAW, "check", path], capture_output=True, text=True, env={"PATH": ""})
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
    p = child([CLAW, "fmt", copy], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"fmt failed: {p.stderr}"
    got = copy.read_text()
    want = path.with_suffix(".out").read_text()
    return None if got == want else f"format differs\n--- got\n{got}--- want\n{want}"


def fmt_stable(path, tmp):
    """Formatting twice equals formatting once, for every program we have."""
    copy = tmp / ("fmt_" + path.name)
    copy.write_text(path.read_text())
    if child([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
        return "first format failed"
    once = copy.read_text()
    if child([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
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
        for path in sorted((ROOT / "tests/check").glob("*.claw")):
            total += 1
            msg = check_test(path)
            if msg:
                failures += 1
                print(f"FAIL check/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/unit").glob("*.claw")):
            total += 1
            msg = unit_test(path)
            if msg:
                failures += 1
                print(f"FAIL unit/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw"))):
            total += 1
            msg = run_again_checked(path)
            if msg:
                failures += 1
                print(f"FAIL checked/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw"))):
            total += 1
            msg = interp_test(path)
            if msg:
                failures += 1
                print(f"FAIL interp/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/own").glob("*.claw")):
            total += 1
            msg = own_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL own/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/auto").glob("*.claw")):
            total += 1
            msg = auto_test(path)
            if msg:
                failures += 1
                print(f"FAIL auto/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/why").glob("*.claw")):
            total += 1
            msg = why_test(path)
            if msg:
                failures += 1
                print(f"FAIL why/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/fix").glob("*.claw")):
            total += 1
            msg = fix_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fix/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/fast").glob("*.claw")):
            total += 1
            msg = fast_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fast/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/stats").glob("*.claw")):
            total += 1
            msg = stats_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL stats/{path.name}: {msg}")
        total += 1
        msg = fuzz(100)
        if msg:
            failures += 1
            print(f"FAIL fuzz: {msg}")
        for path in sorted((ROOT / "tests/fmt").glob("*.claw")):
            total += 1
            msg = fmt_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fmt/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw")) + list((ROOT / "tests/check").glob("*.claw")) + list((ROOT / "tests/unit").glob("*.claw")) + list((ROOT / "tests/auto").glob("*.claw")) + list((ROOT / "tests/why").glob("*.claw")) + list((ROOT / "tests/fast").glob("*.claw")) + list((ROOT / "tests/stats").glob("*.claw")) + list((ROOT / "std").glob("*.claw")) + list((ROOT / "benchmarks").glob("*.claw"))):
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
