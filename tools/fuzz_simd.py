#!/usr/bin/env python3
"""Random SIMD programs: the checked run, the interpreter, arm64 (three modes) and x86-64 must print the same.

Values are multiples of a quarter, so float sums, products and fused multiply-adds are exact and any
disagreement is a bug, not rounding. Integer lanes wrap on purpose.
usage: fuzz_simd.py [count] [first seed]
"""
import concurrent.futures as cf
import os
import pathlib
import random
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))

KINDS = {
    "f64x2": dict(ptr="f64", n=2, scalar="f64", lit=lambda r: f"{r.randint(-12, 12) / 4}"),
    "f32x4": dict(ptr="f32", n=4, scalar="f32", lit=lambda r: f"{r.randint(-12, 12) / 4}"),
    "i32x4": dict(ptr="i32", n=4, scalar="i32", lit=lambda r: f"{r.choice([0, 1, -1, 7, 100000, 2147483647, -2147483648, r.randint(-50, 50)])}"),
}


def program(seed):
    r = random.Random(seed)
    out = []
    out.append("claw fn mixf(a f64x2, b f64x2, c f64x2) f64x2 {\n    ret fma(a, b, c)\n}\n")
    out.append("claw fn addv(a i32x4, b i32x4) i32x4 {\n    ret a * b + a\n}\n")
    out.append("claw fn main_simd() {")
    vars_ = {k: [] for k in KINDS}
    used = set()
    n = 0
    for k, info in KINDS.items():
        cnt = r.randint(2, 3)
        out.append(f"    {k}_mem := alloc[{info['ptr']}]({info['n'] * cnt + 4})")
        out.append(f"    defer free({k}_mem)")
        for j in range(info["n"] * cnt + 4):
            lit = info["lit"](r)
            out.append(f"    {k}_mem[{j}] = {info['scalar']}({lit})" if k == "i32x4" else f"    {k}_mem[{j}] = {lit}")
        for j in range(cnt):
            name = f"v{n}"
            n += 1
            out.append(f"    {name} := vload({k}_mem + {j * info['n']})")
            vars_[k].append(name)
    steps = r.randint(6, 14)
    for _ in range(steps):
        k = r.choice(list(KINDS))
        info = KINDS[k]
        a = r.choice(vars_[k])
        b = r.choice(vars_[k])
        name = f"v{n}"
        n += 1
        form = r.randint(0, 6 if k != "i32x4" else 5)
        if form == 0:
            e = f"{a} + {b}"
        elif form == 1:
            e = f"{a} - {b}"
        elif form == 2:
            e = f"{a} * {b}"
        elif form == 3:
            e = f"-{a}"
        elif form == 4:
            e = f"{k}({info['lit'](r)}) + {a}"
        elif form == 5:
            e = f"{a} * {k}({info['lit'](r)})" if k != "i32x4" else f"addv({a}, {b})"
        else:
            if k == "f64x2":
                c = r.choice(vars_[k])
                e = f"mixf({a}, {b}, {c})" if r.random() < 0.5 else f"fma({a}, {b}, {c})"
            elif k == "f32x4":
                c = r.choice(vars_[k])
                e = f"fma({a}, {b}, {c})"
            else:
                e = f"{a} / {k}(2.0)"
        out.append(f"    {name} := {e}")
        for old in vars_[k]:
            if old in e.replace("(", " ").replace(")", " ").replace(",", " ").split():
                used.add(old)
        vars_[k].append(name)
        if r.random() < 0.3:
            # a vector kept across a call, and stored back
            out.append(f"    vstore({k}_mem, {name})")
    for k, info in KINDS.items():
        v = vars_[k][-1]
        used.add(v)
        for i in range(info["n"]):
            if k == "i32x4":
                out.append(f"    print(int(lane({v}, {i})))")
            else:
                out.append(f"    print(f64(lane({v}, {i})))")
    for k, info in KINDS.items():
        for v in vars_[k]:
            if v not in used:
                out.append(f"    print(int(lane({v}, 0)))" if k == "i32x4" else f"    print(f64(lane({v}, 0)))")
    out.append("}\n")
    out.append("fn main() {\n    main_simd()\n}\n")
    return "\n".join(out)


def run(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env={"PATH": ""})
    except subprocess.TimeoutExpired:
        return None
    return p.returncode, p.stdout, p.stderr


def one(seed, tmp):
    src = pathlib.Path(tmp) / f"s{seed}.claw"
    src.write_text(program(seed))
    results = {}
    results["run"] = run([CLAW, "run", src])
    results["interp"] = run([CLAW, "__interp", src])
    for name, flags in (("opt", []), ("noopt", ["--no-opt"]), ("fast", ["--fast"]), ("x86", ["--target", "x86_64"]), ("x86-noopt", ["--target", "x86_64", "--no-opt"])):
        exe = pathlib.Path(tmp) / f"s{seed}_{name}"
        b = run([CLAW, "build", *flags, src, "-o", exe])
        if b is None or b[0] != 0:
            results[name] = ("build failed", b)
            continue
        results[name] = run([exe])
    ref = results["run"]
    bad = [k for k, v in results.items() if v != ref and not (isinstance(v, tuple) and v[:2] == ref[:2] and False)]
    # stderr of a failed check run is not comparable with a build; here every program must succeed.
    if ref is None or ref[0] != 0:
        return seed, ["reference run failed: " + repr(ref)], src.read_text()
    bad = [k for k, v in results.items() if v is None or v[0] != 0 or v[1] != ref[1]]
    return seed, bad, src.read_text() if bad else ""


def main():
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 100
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    problems = 0
    with tempfile.TemporaryDirectory() as tmp, cf.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        for seed, bad, text in pool.map(lambda s: one(s, tmp), range(first, first + count)):
            if bad:
                problems += 1
                print(f"seed {seed}: differs in {bad}\n{text}")
    print(f"{count} programs, {problems} problems")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
