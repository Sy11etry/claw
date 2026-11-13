#!/usr/bin/env python3
"""Writes three large Claw programs (about 1k, 10k and 50k lines) for compile-speed and memory checks.

Each has: many functions, many struct types, a 500-arm `match`, a 20-level nested `if`, and a
large array literal. main calls every function, so nothing is dead code.
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent


def make(target_lines, name):
    nf = max(10, target_lines // 8)
    ntypes = max(2, target_lines // 400)
    out = []
    for t in range(ntypes):
        out.append(f"type P{t} {{ a int, b int }}")
    arms = 500
    out.append("type Big = " + " | ".join(f"V{i}" for i in range(arms)))
    out.append("fn code(v Big) int {")
    out.append("    match v {")
    out += [f"        V{i}: ret {i}" for i in range(arms)]
    out.append("    }")
    out.append("}")
    # One huge literal in one function trips the 1 MB code limit (E0901), so split it into chunks.
    lit_n = target_lines * 2
    chunk = 20000
    parts = []
    for c in range(0, lit_n, chunk):
        out.append(f"fn table{c // chunk}() int {{")
        out.append("    ret len([" + ", ".join(str(i % 97) for i in range(c, min(c + chunk, lit_n))) + "])")
        out.append("}")
        parts.append(f"table{c // chunk}()")
    out.append("fn table() int {")
    out.append("    ret " + " + ".join(parts))
    out.append("}")
    out.append("fn deep(x int) int {")
    out.append("    mut y := 0")
    for i in range(20):
        out.append("    " * (i + 1) + f"if x > {i} {{")
    out.append("    " * 21 + "y += 1")
    for i in reversed(range(20)):
        out.append("    " * (i + 1) + "}")
    out.append("    ret y")
    out.append("}")
    for f in range(nf):
        out.append(f"fn g{f}(x int) int {{")
        out.append("    mut y := x")
        out.append(f"    y += {f}")
        out.append("    ret y")
        out.append("}")
    out.append("fn main() {")
    out.append("    mut total := 0")
    out.append("    total += code(V7)")
    out.append("    total += deep(12)")
    out.append("    total += table()")
    for f in range(nf):
        out.append(f"    total += g{f}(total % 7)")
    out.append("    print(total)")
    out.append("}")
    path = HERE / name
    path.write_text("\n".join(out) + "\n")
    return path, len(out)


if __name__ == "__main__":
    for lines, name in [(1000, "scale_1k.claw"), (10000, "scale_10k.claw"), (50000, "scale_50k.claw")]:
        p, n = make(lines, name)
        print(p.name, n, "lines", p.stat().st_size, "bytes")
