#!/usr/bin/env python3
"""Runs each program in every build mode and checks that stdout and the exit code agree.

Modes: claw run, claw build, claw build --no-opt, claw build --fast, claw build --target x86_64 (run under Rosetta).
Usage: check_modes.py <file.claw> ...   (a `// args:` line gives arguments; `// expect-compile-error` files are skipped)
"""
import os
import pathlib
import re
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "out"
CLAW = shutil.which("claw") or str(pathlib.Path.home() / ".claw/bin/claw")
MODES = [
    ("run", None),
    ("build", []),
    ("noopt", ["--no-opt"]),
    ("fast", ["--fast"]),
    ("x64", ["--target", "x86_64"]),
]


def run(cmd, cwd):
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=180)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"


def first(text):
    for line in text.splitlines():
        if line.strip():
            return line.strip()[:120]
    return ""


def check(path, args):
    OUT.mkdir(exist_ok=True)
    results = {}
    for name, flags in MODES:
        if flags is None:
            rc, so, se = run([CLAW, "run", path.name, *args], path.parent)
        else:
            exe = OUT / f"{path.stem}_{name}"
            rc, so, se = run([CLAW, "build", *flags, path.name, "-o", str(exe)], path.parent)
            if rc != 0:
                results[name] = ("BUILD", first(se or so))
                continue
            rc, so, se = run([str(exe), *args], path.parent)
        results[name] = (rc, so, first(se))
    values = {(r[0], r[1]) if len(r) == 3 else r for r in results.values()}
    same = len(values) == 1
    return results, same


def main():
    files = [pathlib.Path(a).resolve() for a in sys.argv[1:]]
    differ = 0
    for path in files:
        text = path.read_text()
        if "expect-compile-error" in text:
            continue
        args = []
        m = re.search(r"^//\s*args:\s*(.*)$", text, re.M)
        if m:
            args = m.group(1).split()
        results, same = check(path, args)
        if not same:
            differ += 1
        summary = "same" if same else "DIFFERS"
        codes = " ".join(f"{k}={v[0] if isinstance(v[0], int) else v[0]}" for k, v in results.items())
        first_out = next((v[1].splitlines()[0] for v in results.values() if len(v) == 3 and v[1].strip()), "")
        print(f"{summary:8} {path.parent.name}/{path.name} exit[{codes}] out: {first_out[:60]!r}")
        if not same:
            for k, v in results.items():
                print(f"           {k}: {v if len(v) == 2 else (v[0], v[1][:80], v[2])}")
    print(f"{len(files) - differ}/{len(files)} agree across modes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
