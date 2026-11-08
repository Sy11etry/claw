#!/usr/bin/env python3
"""Measures Claw against clang -O0 and -O2. Median of 5 runs; writes RESULTS.md."""
import pathlib
import statistics
import subprocess
import time

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "out"
CLAW = HERE.parent / "compiler/target/release/claw"
BENCHES = ["fib", "primes", "collatz"]
RUNS = 5


def timed(cmd, **kw):
    t = time.perf_counter()
    subprocess.run(cmd, check=True, stdout=subprocess.PIPE, **kw)
    return time.perf_counter() - t


def median_run(exe):
    return statistics.median(timed([exe]) for _ in range(RUNS))


def measure_compile_speed():
    """A synthetic program of many small functions, to time the compiler itself."""
    n = 4000
    parts = []
    for i in range(n):
        parts.append(
            f"fn f{i}(a int, b int) int {{\n    mut s := a\n"
            f"    for j in 0..b {{\n        if j % 2 == 0 {{\n            s += j\n        }}\n    }}\n"
            f"    ret s + {i}\n}}\n"
        )
    parts.append("fn main() {\n    print(f0(1, 2))\n}\n")
    src = "\n".join(parts)
    path = OUT / "big.claw"
    path.write_text(src)
    lines = src.count("\n")
    elapsed = statistics.median(
        timed([CLAW, "build", path, "-o", OUT / "big"]) for _ in range(RUNS)
    )
    return lines, elapsed


def main():
    OUT.mkdir(exist_ok=True)
    rows = []
    for b in BENCHES:
        subprocess.run([CLAW, "build", HERE / f"{b}.claw", "-o", OUT / f"{b}_claw"], check=True)
        for opt in ["O0", "O2"]:
            subprocess.run(["clang", f"-{opt}", HERE / f"{b}.c", "-o", OUT / f"{b}_{opt}"], check=True)
        claw = median_run(OUT / f"{b}_claw")
        o0 = median_run(OUT / f"{b}_O0")
        o2 = median_run(OUT / f"{b}_O2")
        rows.append((b, o2, o0, claw))
        print(f"{b:10} clang -O2 {o2:7.3f}s  clang -O0 {o0:7.3f}s  claw {claw:7.3f}s")
    lines, elapsed = measure_compile_speed()
    speed = lines / elapsed
    print(f"compile: {lines} lines in {elapsed * 1000:.1f} ms = {speed:,.0f} lines/s")

    version = subprocess.run([CLAW, "--version"], capture_output=True, text=True).stdout.strip()
    out = [
        f"# Benchmarks: {version}",
        "",
        "Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.",
        "",
        "| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |",
        "|---|---|---|---|---|---|",
    ]
    for b, o2, o0, claw in rows:
        out.append(f"| {b} | {o2:.3f} s | {o0:.3f} s | {claw:.3f} s | {claw / o2:.1f}x | {claw / o0:.1f}x |")
    out += ["", f"Compiler speed: {lines} lines in {elapsed * 1000:.1f} ms, about {speed:,.0f} lines/s (full build to a signed binary).", ""]
    (HERE / "RESULTS.md").write_text("\n".join(out))


if __name__ == "__main__":
    main()
