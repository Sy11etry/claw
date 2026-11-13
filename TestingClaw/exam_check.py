#!/usr/bin/env python3
"""Runs each exam program and checks it against the directives in its first lines.

Directives, written as comments:
  // expect: <line>          one expected stdout line, in order
  // exit: <n>               expected exit code; `nz` means any non-zero code (default 0)
  // args: <a b>             arguments passed to the program
  // cmd: test               run `claw test` instead of `claw run`
  // expect-compile-error    the file must fail to compile; its E#### code is recorded
Results go to exam_results.json next to this script.
"""
import json
import pathlib
import re
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
CLAW = shutil.which("claw") or str(pathlib.Path.home() / ".claw/bin/claw")
ERR = re.compile(r"\bE\d{4}\b")


def first_line(text):
    for line in text.splitlines():
        if line.strip():
            return line.strip()[:160]
    return ""


def directives(text):
    d = {"expect": [], "exit": None, "args": [], "cmd": "run", "compile_error": False}
    for raw in text.splitlines():
        line = raw.strip()
        if not line.startswith("//"):
            continue
        body = line[2:].strip()
        if body.startswith("expect:"):
            d["expect"].append(body[len("expect:"):].strip())
        elif body.startswith("exit:"):
            d["exit"] = body[len("exit:"):].strip()
        elif body.startswith("args:"):
            d["args"] = body[len("args:"):].split()
        elif body.startswith("cmd:"):
            d["cmd"] = body[len("cmd:"):].strip()
        elif body == "expect-compile-error":
            d["compile_error"] = True
    return d


def check(path):
    d = directives(path.read_text())
    cmd = [CLAW, d["cmd"], path.name, *d["args"]]
    try:
        p = subprocess.run(cmd, cwd=path.parent, capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return {"ok": False, "detail": "timeout after 120 s"}
    out = p.stdout.splitlines()
    both = p.stdout + p.stderr
    codes = ERR.findall(both)
    if d["compile_error"]:
        ok = p.returncode != 0 and bool(codes)
        return {"ok": ok, "code": codes[0] if codes else None, "exit": p.returncode,
                "detail": "" if ok else "compiled, but an error was expected: " + first_line(both)}
    problems = []
    if d["expect"] and out != d["expect"]:
        problems.append("stdout differs")
    want = d["exit"] or "0"
    if want == "nz":
        if p.returncode == 0:
            problems.append("exit 0, expected non-zero")
    elif p.returncode != int(want):
        problems.append(f"exit {p.returncode}, expected {want}")
    return {"ok": not problems, "exit": p.returncode, "code": codes[0] if codes else None,
            "detail": "; ".join(problems) or first_line(p.stderr), "got": out[:6]}


def main():
    dirs = sys.argv[1:] or ["exam"]
    programs = []
    for d in dirs:
        # Skip helper modules that live in a `lib` folder (lib/geo.claw is imported, not run).
        programs += sorted(p for p in (HERE / d).rglob("*.claw") if p.parent.name != "lib")
    results = {}
    for path in programs:
        results[str(path.relative_to(HERE))] = check(path)
    passed = 0
    for name, r in results.items():
        status = "PASS" if r["ok"] else "FAIL"
        passed += r["ok"]
        code = f" {r['code']}" if r.get("code") else ""
        print(f"{status} {name}{code} {r.get('detail', '')}".rstrip())
    print(f"{passed}/{len(results)} passed")
    (HERE / "exam_results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
