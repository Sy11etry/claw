#!/usr/bin/env python3
"""Checks C-interop programs: builds the claw program, runs it, and compares its output with a C reference.

Directives (comments in the first lines of a program):
  // expect: <line>         expected stdout lines of the claw program (optional)
  // exit: <n>              expected exit code of the claw program (default 0)
  // c: <file.c>            C reference program, built with clang; its stdout must match claw's
  // cflags: <flags>        extra flags for clang (e.g. -lz)
  // clw: <flags>           extra flags for `claw build` (e.g. -l sqlite3)
  // clw-x64: <flags>       extra flags for the x86-64 build (a library built for x86-64)
  // x64: yes               also build with --target x86_64 and compare (Rosetta runs it)
  // expect-compile-error   the program must not compile; its E#### code is recorded
Runs use this folder as the working directory, so files the programs write land here.
"""
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "out"
CLAW = shutil.which("claw") or str(pathlib.Path.home() / ".claw/bin/claw")
ERR = re.compile(r"\bE\d{4}\b")


def first_line(text):
    for line in text.splitlines():
        if line.strip():
            return line.strip()[:200]
    return ""


def directives(text):
    d = {"expect": [], "exit": "0", "c": None, "cflags": [], "clw": [], "clw_x64": None, "x64": False, "compile_error": False}
    for raw in text.splitlines():
        line = raw.strip()
        if not line.startswith("//"):
            continue
        body = line[2:].strip()
        if body.startswith("expect:"):
            d["expect"].append(body[len("expect:"):].strip())
        elif body.startswith("exit:"):
            d["exit"] = body[len("exit:"):].strip()
        elif body.startswith("c:"):
            d["c"] = body[len("c:"):].strip()
        elif body.startswith("cflags:"):
            d["cflags"] = body[len("cflags:"):].split()
        elif body.startswith("clw-x64:"):
            d["clw_x64"] = body[len("clw-x64:"):].split()
        elif body.startswith("clw:"):
            d["clw"] = body[len("clw:"):].split()
        elif body.startswith("x64:"):
            d["x64"] = body[len("x64:"):].strip() == "yes"
        elif body == "expect-compile-error":
            d["compile_error"] = True
    return d


def run(cmd, cwd, env=None):
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=120, env=env)
    except subprocess.TimeoutExpired:
        return 124, "", "timeout after 120 s"
    return p.returncode, p.stdout, p.stderr


def check(path):
    d = directives(path.read_text())
    OUT.mkdir(exist_ok=True)
    stem = path.stem
    env = {**os.environ, "DYLD_LIBRARY_PATH": str(HERE / "lib")}
    problems = []
    # 1. The claw program: build it.
    exe = OUT / stem
    rc, so, se = run([CLAW, "build", str(path), "-o", str(exe), *d["clw"]], HERE, env)
    if d["compile_error"]:
        codes = ERR.findall(so + se)
        ok = rc != 0 and bool(codes)
        return {"ok": ok, "code": codes[0] if codes else None, "detail": "" if ok else "compiled, but an error was expected"}
    if rc != 0:
        return {"ok": False, "detail": "claw build: " + first_line(se or so)}
    rc, claw_out, se = run([str(exe)], HERE, env)
    if rc != int(d["exit"]):
        problems.append(f"claw exit {rc}, expected {d['exit']}: {first_line(se)}")
    claw_lines = claw_out.splitlines()
    if d["expect"] and claw_lines != d["expect"]:
        problems.append("claw output differs from expect")
    # 2. The C reference, built with clang.
    if d["c"]:
        cexe = OUT / (stem + "_c")
        rc, so, se = run(["clang", "-O0", "-o", str(cexe), str(path.parent / d["c"]), *d["cflags"]], HERE, env)
        if rc != 0:
            problems.append("clang: " + first_line(se))
        else:
            rc, c_out, _ = run([str(cexe)], HERE, env)
            if c_out.splitlines() != claw_lines:
                problems.append("claw differs from the C reference")
    # 3. The x86-64 build, run under Rosetta.
    if d["x64"]:
        xexe = OUT / (stem + "_x64")
        rc, so, se = run([CLAW, "build", "--target", "x86_64", str(path), "-o", str(xexe), *(d["clw_x64"] or d["clw"])], HERE, env)
        if rc != 0:
            problems.append("x86-64 build: " + first_line(se or so))
        else:
            rc, x_out, se = run([str(xexe)], HERE, env)
            if x_out != claw_out:
                problems.append("x86-64 output differs from arm64")
    return {"ok": not problems, "detail": "; ".join(problems), "got": claw_lines[:8]}


def main():
    programs = sorted(HERE.glob("b[0-9][0-9]_*.claw"))
    if len(sys.argv) > 1:
        programs = [p for p in programs if any(a in p.name for a in sys.argv[1:])]
    results = {}
    for path in programs:
        results[path.name] = check(path)
    passed = 0
    for name, r in results.items():
        passed += r["ok"]
        status = "PASS" if r["ok"] else "FAIL"
        code = f" {r['code']}" if r.get("code") else ""
        print(f"{status} {name}{code} {r.get('detail', '')}".rstrip())
    print(f"{passed}/{len(results)} passed")
    (HERE / "cinterop_results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
