#!/usr/bin/env python3
"""The exam: a weak model writes Claw from the card alone. This grades the answers.

    tools/exam.py <dir with answers> [--tasks=tasks2.json] [--json]

Each answer is <task>.claw. It must build and print exactly what the task expects.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from tokens import words  # noqa: E402

CLAW = ROOT / "compiler/target/release/claw"


def grade(path, exe, expect):
    if not path.exists():
        return "missing", "no answer"
    b = subprocess.run([CLAW, "build", path, "-o", exe], capture_output=True, text=True, timeout=60)
    if b.returncode != 0:
        return "rejected", b.stderr
    try:
        r = subprocess.run([exe], capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        return "wrong", "did not finish in 10 s"
    if r.returncode != 0:
        return "wrong", f"exit code {r.returncode}\n{r.stderr}"
    if r.stdout.splitlines() != expect:
        return "wrong", f"printed {r.stdout.splitlines()}, wanted {expect}"
    return "pass", ""


def main():
    answers = pathlib.Path(sys.argv[1])
    which = next((a.split("=")[1] for a in sys.argv if a.startswith("--tasks=")), "tasks.json")
    tasks = json.loads((ROOT / "benchmarks/exam" / which).read_text())
    results = []
    for t in tasks:
        path = answers / f"{t['name']}.claw"
        verdict, detail = grade(path, answers / (t["name"] + ".bin"), t["expect"])
        results.append({"name": t["name"], "verdict": verdict, "detail": detail, "tokens": words(path.read_text()) if path.exists() else 0, "error_tokens": words(detail)})
    if "--json" in sys.argv:
        print(json.dumps(results, ensure_ascii=False, indent=1))
        return
    for r in results:
        print(f"{r['verdict']:9}{r['name']:18}{r['detail'].splitlines()[0][:130] if r['detail'] else ''}")
    n = sum(r["verdict"] == "pass" for r in results)
    print(f"{n}/{len(results)} pass; code {sum(r['tokens'] for r in results)} tokens, errors {sum(r['error_tokens'] for r in results)} tokens")


if __name__ == "__main__":
    main()
