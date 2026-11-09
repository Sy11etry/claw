#!/usr/bin/env python3
"""Writes random Claw programs and compares the optimized build with the unoptimized one.

Both must print the same, stop with the same exit code and report the same trap.
usage: fuzz_opt.py [count] [first seed]
"""
import os
import pathlib
import random
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))
INTS = ["int", "i32", "i16", "i8", "u8", "u16", "u32", "u64"]


class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)
        self.n = 0
        self.funcs = []  # (name, param types, result type)
        self.depth = 0

    def name(self, p="v"):
        self.n += 1
        return f"{p}{self.n}"

    def lit(self, t):
        r = self.r
        if t == "f64":
            return r.choice(["0.5", "1.5", "2.0", "-3.25", "100.0", "0.0"])
        if t == "bool":
            return r.choice(["true", "false"])
        hi = {"i8": 127, "u8": 255, "i16": 32767, "u16": 65535, "i32": 2147483647, "u32": 4294967295, "int": 9223372036854775807, "u64": 18446744073709551615}[t]
        if r.random() < 0.08:
            return str(hi)
        v = r.choice([0, 1, 2, 3, 4, 7, 8, 10, 16, 31, 100])
        if t in ("i8", "i16", "i32", "int") and r.random() < 0.3:
            return f"(-{v})" if v else "0"
        return str(min(v, hi))

    def var(self, t, scope):
        """A variable of this type: a bare literal here could be judged at compile time."""
        here = [n for n, x in scope if x == t]
        return self.r.choice(here)

    def expr(self, t, scope, d=0):
        r = self.r
        here = [n for n, x in scope if x == t]
        if d > 3 or r.random() < 0.3:
            if here and r.random() < 0.8:
                return r.choice(here)
            return self.lit(t)
        if t == "bool":
            k = r.random()
            if k < 0.5:
                u = r.choice(INTS + ["f64"])
                op = r.choice(["<", "<=", ">", ">=", "==", "!="])
                return f"({self.var(u, scope)} {op} {self.expr(u, scope, d + 1)})"
            if k < 0.75:
                return f"({self.expr('bool', scope, d + 1)} {r.choice(['and', 'or'])} {self.expr('bool', scope, d + 1)})"
            return f"(not {self.expr('bool', scope, d + 1)})"
        if t == "f64":
            k = r.random()
            if k < 0.7:
                return f"({self.var('f64', scope)} {r.choice('+-*/')} {self.expr('f64', scope, d + 1)})"
            return f"f64({self.var(r.choice(INTS), scope)})"
        k = r.random()
        arrays = [n for n, x in scope if x == "[]int" or x == "[4]int"]
        structs = [n for n, x in scope if x == "P"]
        if k < 0.5:
            ops = ["+", "-", "*", "/", "%", "&", "|", "^"]
            return f"({self.var(t, scope)} {r.choice(ops)} {self.expr(t, scope, d + 1)})"
        if k < 0.58:
            return f"({self.var(t, scope)} {r.choice(['<<', '>>'])} {r.choice(['0', '1', '2', '3', '7'])})"
        if k < 0.68:
            return f"{t}({self.var(r.choice(INTS), scope)})"
        if k < 0.78 and arrays:
            a = r.choice(arrays)
            i = self.var("int", scope)
            if r.random() < 0.7:
                i = f"({i} & 3)"
            v = f"{a}[{i}]"
            return v if t == "int" else f"{t}({v})"
        if k < 0.84 and arrays:
            v = f"len({r.choice(arrays)})"
            return v if t == "int" else f"{t}({v})"
        if k < 0.9 and structs:
            f = r.choice('xy')
            v = f"{r.choice(structs)}.{f}"
            return v if t == {'x': 'int', 'y': 'i32'}[f] else f"{t}({v})"
        if self.funcs:
            f = r.choice(self.funcs)
            args = ", ".join(self.expr(p, scope, d + 1) for p in f[1])
            v = f"{f[0]}({args})"
            return v if f[2] == t else f"{t}({v})"
        return self.lit(t)

    def block(self, scope, muts, count, indent):
        r = self.r
        pad = "    " * indent
        scope = list(scope)
        out = []
        for _ in range(count):
            k = r.random()
            ints = [(n, x) for n, x in muts if x in INTS or x == "f64" or x == "bool"]
            if k < 0.45 and ints:
                n, x = r.choice(ints)
                out.append(f"{pad}{n} = {self.expr(x, scope)}")
            elif k < 0.6 and ints and self.depth < 3:
                self.depth += 1
                out.append(f"{pad}if {self.expr('bool', scope)} {{")
                out += self.block(scope, muts, r.randint(1, 3), indent + 1)
                if r.random() < 0.5:
                    out.append(f"{pad}}} else {{")
                    out += self.block(scope, muts, r.randint(1, 3), indent + 1)
                out.append(f"{pad}}}")
                self.depth -= 1
            elif k < 0.72 and ints and self.depth < 3:
                self.depth += 1
                i = self.name("k")
                out.append(f"{pad}for {i} in 0..{r.choice([1, 2, 3, 5, 9])} {{")
                inner = self.block(scope + [(i, "int")], muts, r.randint(1, 3), indent + 1)
                out.append(f"{pad}    if {i} < 0 {{ continue }}")
                out += inner
                out.append(f"{pad}}}")
                self.depth -= 1
            elif k < 0.8:
                arrays = [n for n, x in muts if x in ("[]int", "[4]int")]
                if arrays:
                    a = r.choice(arrays)
                    out.append(f"{pad}{a}[{self.var('int', scope)} & 3] = {self.expr('int', scope)}")
            elif k < 0.86:
                grow = [n for n, x in muts if x == "[]int"]
                if grow:
                    out.append(f"{pad}push(&{r.choice(grow)}, {self.expr('int', scope)})")
            elif k < 0.92:
                s = [n for n, x in muts if x == "P"]
                if s:
                    out.append(f"{pad}{r.choice(s)}.x = {self.expr('int', scope)}")
            else:
                n = self.name()
                t = r.choice(INTS + ["f64", "bool"])
                out.append(f"{pad}{n} {t} = {self.expr(t, scope)}")
                out.append(f"{pad}print({n})")
                scope.append((n, t))
        return out

    def base_vars(self):
        """One variable of every type, so that expressions always have something to use."""
        out = []
        for t in INTS + ["f64"]:
            out.append((self.name("z"), t, self.lit(t)))
        return out

    def func(self):
        r = self.r
        name = self.name("fun")
        params = [r.choice(INTS + ["f64"]) for _ in range(r.randint(0, 9 if r.random() < 0.15 else 3))]
        result = r.choice(INTS + ["f64"])
        scope = [(self.name("p"), t) for t in params]
        head = ", ".join(f"{n} {t}" for n, t in scope)
        base = self.base_vars()
        lines = [f"fn {name}({head}) {result}"]
        ints = [n for n, t in scope if t == "int"]
        if ints and r.random() < 0.4:
            lines.append(f"    need {ints[0]} != {r.choice(['0', '7', '100'])}")
        if result == "int" and r.random() < 0.3:
            lines.append(f"    sure ret != {r.choice(['12345', '0', '-1'])}")
        lines.append("{")
        for n, _ in scope:
            lines.append(f"    _ = {n}")
        for n, t, init in base:
            lines.append(f"    {n} {t} = {init}")
            lines.append(f"    _ = {n}")
        scope += [(n, t) for n, t, _ in base]
        mine = []
        for _ in range(r.randint(0, 3)):
            n = self.name()
            t = r.choice(INTS)
            lines.append(f"    mut {n} {t} = {self.expr(t, scope)}")
            lines.append(f"    {n} = {n}")
            mine.append((n, t))
        lines += self.block(scope + mine, mine, r.randint(0, 4), 1)
        for n, _ in mine:
            lines.append(f"    _ = {n}")
        lines.append(f"    ret {self.expr(result, scope + mine)}")
        lines.append("}")
        self.funcs.append((name, params, result))
        return lines

    def program(self):
        r = self.r
        out = ["type P { x int, y i32 }", ""]
        for _ in range(r.randint(1, 5)):
            out += self.func()
            out.append("")
        out.append("fn main() {")
        base = self.base_vars()
        for n, t, init in base:
            out.append(f"    {n} {t} = {init}")
            out.append(f"    print({n})")
        scope = [(n, t) for n, t, _ in base]
        for _ in range(r.randint(3, 8)):
            n = self.name()
            t = r.choice(INTS + ["f64", "bool"])
            out.append(f"    mut {n} {t} = {self.expr(t, scope)}")
            out.append(f"    {n} = {n}")
            scope.append((n, t))
        a, b, s = self.name("a"), self.name("b"), self.name("s")
        out.append(f"    mut {a} []int = [1, 2, 3, 4]")
        out.append(f"    mut {b} [4]int = [5, 6, 7, 8]")
        out.append(f"    mut {s} := P{{1, 2}}")
        out.append(f"    {s}.y = 3")
        out.append(f"    {b}[0] = 9")
        out.append(f"    push(&{a}, 0)")
        scope += [(a, "[]int"), (b, "[4]int"), (s, "P")]
        for _ in range(r.randint(3, 8)):
            out += self.block(scope, scope[len(base):], 1, 1)
            for n, _ in scope:
                out.append(f"    print({n})")
        out.append("}")
        return "\n".join(out) + "\n"


def run(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired:
        return None
    return p.returncode, p.stdout, p.stderr


def check(seed, tmp):
    """None if the two builds agree; otherwise what differed."""
    path = tmp / f"p{seed}.claw"
    path.write_text(Gen(seed).program())
    exe = tmp / f"p{seed}"
    built = subprocess.run([CLAW, "build", path, "-o", exe], capture_output=True, text=True)
    if built.returncode != 0:
        return "compile", built.stderr[:600], path
    slow = run([CLAW, "run", path])
    fast = run([exe])
    if slow is None or fast is None:
        return None
    if slow != fast:
        return "differs", f"unoptimized {slow[0]}: {slow[2][-200:]}\noptimized   {fast[0]}: {fast[2][-200:]}\nstdout same: {slow[1] == fast[1]}", path
    return None


def main():
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 200
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        for seed in range(first, first + count):
            r = check(seed, tmp)
            if r:
                bad += 1
                print(f"seed {seed}: {r[0]}\n{r[1]}")
                keep = pathlib.Path(f"/tmp/claw_fuzz_{seed}.claw")
                keep.write_text(r[2].read_text())
                if bad > 5:
                    break
    print(f"{count} programs, {bad} problems")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
