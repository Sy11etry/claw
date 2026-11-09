#!/usr/bin/env python3
"""Tokens for the whole cycle: write, read, fix. Claw against C and Rust.

No real tokenizer is available here, so these are ESTIMATES, made the same way for
all three languages. Two estimates are given: `pessimistic` is the one the card's
budget uses (every punctuation mark is a token), `words` is closer to a language
model (common operators are one token, long names are several).

Every program must print the same thing in all three languages, or the comparison
would be meaningless, and the run fails.
"""
import pathlib
import re
import statistics
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = ROOT / "compiler/target/release/claw"
HERE = ROOT / "benchmarks/tokens"
OPS = ["<<=", ">>=", "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", ":=", "..", "<<", ">>", "{}", "()", "[]", "\\n"]


def pessimistic(text):
    return len(re.findall(r"[A-Za-z_]+|\d+|[^\sA-Za-z_\d]", text))


def words(text):
    pattern = "|".join(re.escape(o) for o in OPS)
    n = 0
    for m in re.finditer(rf"[A-Za-z_][A-Za-z_0-9]*|\d+|{pattern}|[^\s]", text):
        t = m.group()
        if re.match(r"[A-Za-z_]", t):
            pieces = [p for p in re.split(r"_|(?<=[a-z])(?=[A-Z])", t) if p]
            n += max(1, sum(1 + len(p) // 9 for p in pieces))
        elif t.isdigit():
            n += 1 + len(t) // 4
        else:
            n += 1
    return n


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60)


def outputs(name, tmp):
    src = HERE / name
    out = {}
    for lang, build in (
        ("claw", [CLAW, "build", src.with_suffix(".claw"), "-o", tmp / f"{name}_claw"]),
        ("c", ["clang", "-w", "-O0", src.with_suffix(".c"), "-o", tmp / f"{name}_c"]),
        ("rs", ["rustc", "-A", "warnings", "-o", tmp / f"{name}_rs", src.with_suffix(".rs")]),
    ):
        b = run(build)
        if b.returncode != 0:
            sys.exit(f"{name}.{lang} does not build:\n{b.stderr[:800]}")
        r = run([tmp / f"{name}_{lang}"])
        if r.returncode != 0:
            sys.exit(f"{name}.{lang} exited with {r.returncode}: {r.stderr[:400]}")
        out[lang] = r.stdout
    return out


def write_table():
    names = sorted(p.stem for p in HERE.glob("*.claw"))
    rows = []
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        for name in names:
            out = outputs(name, tmp)
            if len(set(out.values())) != 1:
                sys.exit(f"{name}: the three programs print different things:\n{out}")
            texts = {lang: (HERE / f"{name}.{lang}").read_text() for lang in ("claw", "c", "rs")}
            rows.append((name, {k: pessimistic(v) for k, v in texts.items()}, {k: words(v) for k, v in texts.items()}))
    return rows


def error_table():
    rows = []
    for src in sorted((HERE / "errors").glob("*.claw")):
        base = src.with_suffix("")
        claw = run([CLAW, "check", src])
        c = run(["clang", "-Wall", "-Wextra", "-fsyntax-only", base.with_suffix(".c")])
        rs = run(["rustc", "--edition", "2021", "--emit=metadata", "-o", "/dev/null", base.with_suffix(".rs")])
        rows.append((src.stem, words(claw.stderr), words(c.stderr), words(rs.stderr), claw.returncode, rs.returncode))
    return rows


def sig_ratio():
    src, sig = 0, 0
    for p in HERE.glob("*.claw"):
        r = run([CLAW, "sig", p])
        if r.returncode == 0:
            src += words(p.read_text())
            sig += words(r.stdout)
    return src, sig


def main():
    rows = write_table()
    out = ["# Token cycle", "", "**These are estimates**, not a real tokenizer: two counting rules, applied the same way to all three languages. All three programs of each task print the same output (checked on every run).", ""]
    out += ["| Task | Claw | C | Rust | Claw vs C | Claw vs Rust |", "|---|---|---|---|---|---|"]
    for name, _, w in rows:
        out.append(f"| {name} | {w['claw']} | {w['c']} | {w['rs']} | {100 * (w['claw'] / w['c'] - 1):+.0f}% | {100 * (w['claw'] / w['rs'] - 1):+.0f}% |")
    def total(i, lang):
        return sum(r[i][lang] for r in rows)
    def med(lang):
        return statistics.median(r[2]["claw"] / r[2][lang] - 1 for r in rows) * 100
    t = {k: (total(1, k), total(2, k)) for k in ("claw", "c", "rs")}
    out += ["", "| Total over all tasks | Claw | C | Rust | Claw vs C | Claw vs Rust |", "|---|---|---|---|---|---|"]
    out.append(f"| `words` estimate | {t['claw'][1]} | {t['c'][1]} | {t['rs'][1]} | {100 * (t['claw'][1] / t['c'][1] - 1):+.1f}% | {100 * (t['claw'][1] / t['rs'][1] - 1):+.1f}% |")
    out.append(f"| `pessimistic` estimate | {t['claw'][0]} | {t['c'][0]} | {t['rs'][0]} | {100 * (t['claw'][0] / t['c'][0] - 1):+.1f}% | {100 * (t['claw'][0] / t['rs'][0] - 1):+.1f}% |")
    out.append(f"| Median task (`words`) | | | | {med('c'):+.1f}% | {med('rs'):+.1f}% |")
    errs = error_table()
    out += ["", "## Fixing: tokens in the compiler's message for the same mistake", "", "`clang -Wall -Wextra` and `rustc`, default output. `-` means the compiler says nothing at all.", "", "| Mistake | Claw | clang | rustc |", "|---|---|---|---|"]
    for n, a, b, c, rc_claw, rc_rs in errs:
        out.append(f"| {n} | {a} | {b or '-'} | {c or '-'} |")
    out.append(f"| **Total** | **{sum(e[1] for e in errs)}** | **{sum(e[2] for e in errs)}** | **{sum(e[3] for e in errs)}** |")
    src, sig = sig_ratio()
    out += ["", "## Reading", "", f"`claw sig` over these programs: {sig} tokens instead of {src} for the full source ({100 * sig / src:.0f}%).", ""]
    text = "\n".join(out)
    (ROOT / "benchmarks/TOKENS.md").write_text(text)
    print(text)


if __name__ == "__main__":
    main()
