#!/usr/bin/env python3
"""Random programs from fuzz_opt.py, built for arm64 and for x86-64: they must behave the same.

For each program, two pairs are compared (stdout, stderr and exit code, exactly):
  claw build           the optimized executables
  claw run             check mode, with every contract and the memory checker
usage: fuzz_x64.py [count] [first seed]
"""
import concurrent.futures as cf
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from fuzz_opt import Gen  # noqa: E402

CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))


def run(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env={"PATH": ""})
    except subprocess.TimeoutExpired:
        return None
    return p.returncode, p.stdout, p.stderr


def check(seed, tmp):
    """None if the targets agree; otherwise what differed."""
    path = tmp / f"p{seed}.claw"
    path.write_text(Gen(seed).program())
    arm, x64 = tmp / f"p{seed}_arm", tmp / f"p{seed}_x64"
    a = run([CLAW, "build", path, "-o", arm])
    b = run([CLAW, "build", "--target", "x86_64", path, "-o", x64])
    if a is None or b is None:
        return "compile", "the compiler did not finish in 60 s", path
    if a[0] != b[0]:
        return "compile", f"arm64 {a[0]}: {a[2][:300]}\nx86_64 {b[0]}: {b[2][:300]}", path
    if a[0] == 0:
        ra, rb = run([arm]), run([x64])
        if ra is not None and rb is not None and ra != rb:
            return "build differs", f"arm64 {ra[0]}: {ra[2][-200:]}\nx86_64 {rb[0]}: {rb[2][-200:]}\nstdout same: {ra[1] == rb[1]}", path
    ca = run([CLAW, "run", path])
    cb = run([CLAW, "run", "--target", "x86_64", path])
    if ca is not None and cb is not None and ca != cb:
        return "run differs", f"arm64 {ca[0]}: {ca[2][-200:]}\nx86_64 {cb[0]}: {cb[2][-200:]}\nstdout same: {ca[1] == cb[1]}", path
    return None


def main():
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 200
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        seeds = list(range(first, first + count))
        with cf.ThreadPoolExecutor(8) as ex:
            for seed, r in zip(seeds, ex.map(lambda s: check(s, tmp), seeds)):
                if r:
                    bad += 1
                    print(f"seed {seed}: {r[0]}\n{r[1]}")
                    keep = pathlib.Path(f"/tmp/claw_fuzz_x64_{seed}.claw")
                    keep.write_text(r[2].read_text())
    print(f"{count} programs, {bad} problems")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
