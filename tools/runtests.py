#!/usr/bin/env python3
"""Runs every test of every iteration against the freshly built compiler.

tests/run/*.claw  compiled and executed. Headers:
    // expect: <stdout line>     (repeatable)
    // exit: <code>              (default 0)
    // stderr: <substring>       (repeatable)
tests/err/*.claw  must be rejected. Header:
    // error: E####              (repeatable; all must appear, nothing else)
tests/own/*.claw  like run, built with --alloc-stats: allocations must equal frees.
tests/check/*.claw  like run, but through `claw run`: every contract and the memory checker.
tests/unit/*.claw  through `claw test --blocks`: the headers describe its report.
tests/run and tests/own again, in the interpreter: it must agree with machine code.
tests/auto/*.claw  through `claw test`: generated inputs, shrunk failures and their causes.
tests/why/*.claw  through `claw why`: the report names the lines that matter.
tests/fix/x.claw  `claw fix` must turn it into tests/fix/x.out, or leave it and report.
tests/run again, in check mode: the checker must not complain about correct programs.
x86-64: tests/run and tests/own built with --target x86_64 and run under Rosetta (optimized build, then
    --alloc-stats for tests/own), tests/run and tests/check through `claw run --target x86_64`,
    the encoder against llvm-mc, and random programs that must behave the same on both targets.
tests/fast/*.claw  built with --fast; tests/stats/*.claw  the optimizer proves what it should.
tools/fuzz_opt.py  random programs: optimized and unoptimized builds must agree.
tests/fmt/x.claw  must format to tests/fmt/x.out.
tests/par/*.claw  tasks: checked run and real threads must print the same; err_*.claw must be rejected.
tests/debug/*.claw  lldb stops at the right line.
tests/simd/*.claw  SIMD vectors: checked run, three build modes and the interpreter agree; err_*.claw must be rejected.
Linux target: the ELF is checked, and with qemu and an Alpine image the programs run there too.

The compiler runs with an empty PATH, proving it needs no as, ld or clang.
"""
import os
import pathlib
import re
import glob
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))


def child(cmd, timeout=120, **kw):
    """Every child has a time limit: a compiler that hangs is a failed test, not a stuck run."""
    try:
        return subprocess.run(cmd, timeout=timeout, **kw)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, -9, "", f"timed out after {timeout} s")


def headers(text, key):
    return re.findall(rf"^// {key}: ?(.*)$", text, re.M)


def run_test(path, tmp, extra=()):
    text = path.read_text()
    exe = tmp / path.stem
    p = child(
        [CLAW, "build", *extra, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""}
    )
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if r.returncode != want_exit:
        return f"exit code {r.returncode}, wanted {want_exit}\nstderr: {r.stderr}"
    want_out = headers(text, "expect")
    got_out = r.stdout.splitlines()
    if got_out != want_out:
        return f"stdout mismatch\n  got:  {got_out}\n  want: {want_out}"
    for s in headers(text, "stderr"):
        if s not in p.stderr + r.stderr:
            return f"stderr lacks {s!r}: {p.stderr + r.stderr!r}"
    return None


def check_test(path, extra=()):
    """`claw run`: the mode with every contract and the memory checker."""
    text = path.read_text()
    p = child([CLAW, "run", *extra, path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout mismatch\n  got:  {p.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    for s in headers(text, "stderr"):
        if s not in p.stderr:
            return f"stderr lacks {s!r}: {p.stderr!r}"
    return None


def run_again_checked(path, extra=()):
    """A correct program behaves the same with the checker switched on."""
    text = path.read_text()
    if headers(text, "build-only"):
        return None
    p = child([CLAW, "run", *extra, path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "checked-exit") or headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout differs: {p.stdout.splitlines()}"
    return None


def interp_test(path):
    """The interpreter must agree with real machine code, or `why` and the fuzzer mean nothing."""
    text = path.read_text()
    if headers(text, "build-only"):
        return None
    p = child([CLAW, "__interp", path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    if p.returncode == 91:
        return None  # a C function the interpreter does not carry
    want_exit = int((headers(text, "checked-exit") or headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"stdout differs: {p.stdout.splitlines()}"
    for s in headers(text, "stderr"):
        if s.startswith("trap: ") and "out of memory" not in s and s not in p.stderr:
            return f"stderr lacks {s!r}: {p.stderr!r}"
    return None


def unit_test(path):
    text = path.read_text()
    p = child([CLAW, "test", "--blocks", path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return f"report differs\n  got:  {p.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    return None


def auto_test(path):
    """`claw test` with generated inputs: the whole report is compared."""
    text = path.read_text()
    p = child([CLAW, "test", path], capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return "report differs\n  got:  " + "\n        ".join(p.stdout.splitlines()) + "\n  want: " + "\n        ".join(headers(text, "expect"))
    return None


def why_test(path):
    """`claw why`: the lines of cause are compared exactly."""
    text = path.read_text()
    cmd = [CLAW, "why", path]
    for name in headers(text, "test"):
        cmd += ["--test", name]
    p = child(cmd, capture_output=True, text=True, env={"PATH": ""}, timeout=120)
    want_exit = int((headers(text, "exit") or ["1"])[0])
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\nstdout: {p.stdout}\nstderr: {p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return "report differs\n  got:  " + "\n        ".join(p.stdout.splitlines()) + "\n  want: " + "\n        ".join(headers(text, "expect"))
    return None


def info_test(path):
    """`claw sig`, `ctx`, `audit`: the whole output is compared."""
    text = path.read_text()
    word, *rest = headers(text, "cmd")[0].split()
    tail = [path, *rest] if word == "ctx" else [*rest, path]
    p = child([CLAW, word, *tail], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"exit code {p.returncode}\n{p.stderr}"
    if p.stdout.splitlines() != headers(text, "expect"):
        return "output differs\n  got:  " + "\n        ".join(p.stdout.splitlines()) + "\n  want: " + "\n        ".join(headers(text, "expect"))
    return None


def bootstrap():
    """The compiler written in Claw builds itself, twice, to the very same bytes as Rust's `--no-opt`."""
    out = ROOT / "selfhost/build"
    for d in ("s0", "s1", "s2", "s3"):
        (out / d).mkdir(parents=True, exist_ok=True)
    main = ROOT / "selfhost/main.claw"
    steps = [
        [CLAW, "build", main, "-o", out / "s1/clawc"],
        [CLAW, "build", "--no-opt", main, "-o", out / "s0/clawc"],
        [out / "s1/clawc", main, "-o", out / "s2/clawc"],
        [out / "s2/clawc", main, "-o", out / "s3/clawc"],
    ]
    for cmd in steps:
        p = child(cmd, capture_output=True, text=True, timeout=300)
        if p.returncode != 0:
            return f"{cmd[0]} failed: {p.stderr[-600:]}"
    a, b, c = ((out / d / "clawc").read_bytes() for d in ("s0", "s2", "s3"))
    if b != c:
        return "stage 2 and stage 3 differ: the compiler does not reproduce itself"
    if a != b:
        return "the self-built compiler differs from Rust's --no-opt build"
    return None


def selfhost_corpus(tmp):
    """Every run test, built by the compiler written in Claw: the same bytes as Rust's `--no-opt`."""
    clawc = ROOT / "selfhost/build/s3/clawc"
    bad = []
    for path in sorted((ROOT / "tests/run").glob("*.claw")):
        if "const_computed" in path.name:
            continue
        for flags in ([], ["--fast"]):
            a, b = tmp / "x" / path.stem, tmp / "y" / path.stem
            a.parent.mkdir(exist_ok=True)
            b.parent.mkdir(exist_ok=True)
            p = child([CLAW, "build", "--no-opt", *flags, path, "-o", a], capture_output=True)
            q = child([clawc, *flags, path, "-o", b], capture_output=True)
            if p.returncode != q.returncode or (p.returncode == 0 and a.read_bytes() != b.read_bytes()):
                bad.append(f"{path.name} {' '.join(flags)}")
    return None if not bad else f"{len(bad)} differ: {', '.join(bad[:5])}"


def debug_test(path, tmp):
    """`claw build -g`: the .dSYM passes dwarfdump, and lldb stops on the line with the right backtrace."""
    import shutil

    if not (shutil.which("lldb") and shutil.which("dwarfdump")):
        return None
    text = path.read_text()
    problems = []
    for flags in ([], ["--no-opt"]):
        exe = tmp / ("dbg" + "".join(f.strip("-") for f in flags) + "_" + path.stem)
        p = child([CLAW, "build", "-g", path, "-o", exe, *flags], capture_output=True, text=True, env={"PATH": ""})
        if p.returncode != 0:
            return f"compile failed:\n{p.stderr}"
        v = child(["dwarfdump", "--verify", f"{exe}.dSYM"], capture_output=True, text=True)
        if v.returncode != 0 or "No errors" not in v.stdout + v.stderr:
            return f"dwarfdump --verify {' '.join(flags)}: {v.stdout}{v.stderr}"
        line = headers(text, "break")[0]
        cmds = ["-o", f"breakpoint set -f {path.name} -l {line}", "-o", "run", "-o", "bt", "-o", "kill"]
        r = child(["lldb", "-b", *cmds, str(exe)], capture_output=True, text=True, timeout=60)
        out = r.stdout + r.stderr
        # Optimized code may have inlined a callee, so only the plain build promises the call chain.
        wanted = headers(text, "frame") if flags else [f"{path.name}:{line}"]
        for want in wanted:
            where = want.split(" at ")[-1]
            if not re.search(rf"at {re.escape(where)}", out):
                problems.append(f"{' '.join(flags) or 'optimized'}: nothing at `{want}` in:\n{out}")
        if flags:
            for want in headers(text, "frame"):
                fn, where = want.split(" at ")
                if not re.search(rf"`{re.escape(fn)} at {re.escape(where)}", out):
                    problems.append(f"{' '.join(flags)}: no frame `{want}` in:\n{out}")
    return "\n".join(problems) or None


def lsp_test():
    """`claw lsp` answers an editor: diagnostics on unsaved text, quick fixes, hover, outline, formatting."""
    p = child([sys.executable, ROOT / "tools/lsp_check.py"], capture_output=True, text=True, env={**os.environ, "CLAW": str(CLAW)}, timeout=120)
    return None if p.returncode == 0 else p.stdout + p.stderr


def card_command():
    """`claw card` is exactly the card the size budget is checked on."""
    p = child([CLAW, "card"], capture_output=True, text=True)
    return None if p.stdout == (ROOT / "docs/CARD.md").read_text() else "`claw card` differs from docs/CARD.md"


def fix_test(path, tmp):
    """`claw fix` must produce tests/fix/x.out; applying it again must change nothing."""
    text = path.read_text()
    copy = tmp / path.name
    copy.write_text(text)
    want_exit = int((headers(text, "exit") or ["0"])[0])
    p = child([CLAW, "fix", copy], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != want_exit:
        return f"exit code {p.returncode}, wanted {want_exit}\n{p.stdout}{p.stderr}"
    got = copy.read_text()
    want = path.with_suffix(".out").read_text()
    if got != want:
        return f"result differs\n--- got\n{got}--- want\n{want}"
    child([CLAW, "fix", copy], capture_output=True, env={"PATH": ""})
    return None if copy.read_text() == got else "a second `claw fix` changed the file"


def fast_test(path, tmp):
    """`claw build --fast`: still correct, but nothing is checked."""
    text = path.read_text()
    exe = tmp / ("fast_" + path.stem)
    p = child([CLAW, "build", path, "-o", exe, "--fast"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        return f"exit code {r.returncode}\nstderr: {r.stderr}"
    if r.stdout.splitlines() != headers(text, "expect"):
        return f"stdout mismatch\n  got:  {r.stdout.splitlines()}\n  want: {headers(text, 'expect')}"
    return None


def stats_test(path, tmp):
    """The optimizer must really remove the checks it claims to be able to prove."""
    text = path.read_text()
    exe = tmp / ("stats_" + path.stem)
    p = child([CLAW, "build", path, "-o", exe, "--stats"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    m = re.search(r"checks: (\d+) of (\d+) removed", p.stdout)
    if not m:
        return f"no statistics: {p.stdout!r}"
    least = headers(text, "removes-at-least")
    if least and int(m[1]) < int(least[0]):
        return f"removed only {m[1]} of {m[2]}"
    if re.search(r"^// removes-all-bounds", text, re.M):
        dump = child([CLAW, "build", path, "-o", exe, "--emit-cir"], capture_output=True, text=True, env={"PATH": ""})
        if "BoundsDyn" in dump.stdout or "Bounds {" in dump.stdout:
            return "a bounds check survived"
    return None


def fuzz(count):
    """Random programs: the optimized build must behave exactly like the unoptimized one."""
    p = child([sys.executable, ROOT / "tools/fuzz_opt.py", str(count), "1"], capture_output=True, text=True, timeout=3600)
    return None if p.returncode == 0 else p.stdout[-800:]


def in_temp(test, path, extra):
    """Runs `test(path, dir, extra)` in a directory of its own."""
    with tempfile.TemporaryDirectory() as d:
        return test(path, pathlib.Path(d), extra)


def parallel(jobs, workers=8):
    """Runs (label, function) jobs side by side; gives back (label, result) in the original order."""
    import concurrent.futures as cf

    with cf.ThreadPoolExecutor(workers) as ex:
        results = list(ex.map(lambda j: j[1](), jobs))
    return [(j[0], r) for j, r in zip(jobs, results)]


def encoder_x64():
    """Our x86-64 encoder against llvm-mc."""
    p = child([sys.executable, ROOT / "tools/check_encoder_x64.py"], capture_output=True, text=True)
    return None if p.returncode == 0 else (p.stdout + p.stderr)[-800:]


def fuzz_x64(count):
    """Random programs: arm64 and x86-64 must print the same, trap the same, exit the same."""
    p = child([sys.executable, ROOT / "tools/fuzz_x64.py", str(count), "1"], capture_output=True, text=True, timeout=3600)
    return None if p.returncode == 0 else p.stdout[-800:]


def own_test(path, tmp, extra=()):
    """Like a run test, but every allocation must also have been freed."""
    text = path.read_text()
    exe = tmp / ("own_" + path.stem)
    p = child([CLAW, "build", *extra, path, "-o", exe, "--alloc-stats"], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"compile failed:\n{p.stderr}"
    r = child([exe], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        return f"exit code {r.returncode}\nstderr: {r.stderr}"
    want_out = headers(text, "expect")
    if r.stdout.splitlines() != want_out:
        return f"stdout mismatch\n  got:  {r.stdout.splitlines()}\n  want: {want_out}"
    m = re.search(r"allocs=(\d+) frees=(\d+)", r.stderr)
    if not m:
        return f"no allocation report: {r.stderr!r}"
    if m[1] != m[2]:
        return f"leak or double free: {m[0]}"
    least = headers(text, "allocs-at-least")
    if least and int(m[1]) < int(least[0]):
        return f"expected at least {least[0]} allocations, saw {m[1]}"
    return None


def par_test(path, tmp):
    """Tasks: the checked run is sequential, the built program really uses threads.
    Both must print the same, many times over, in every build mode."""
    text = path.read_text()
    want = headers(text, "expect")
    p = child([CLAW, "run", path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
    if p.returncode != 0 or p.stdout.splitlines() != want:
        return f"claw run: exit {p.returncode}, got {p.stdout.splitlines()}, wanted {want}\n{p.stderr}"
    for mode in ([], ["--fast"], ["--no-opt"]):
        exe = tmp / ("par_" + path.stem + "".join(mode))
        b = child([CLAW, "build", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"compile {mode} failed:\n{b.stderr}"
        for _ in range(25):
            r = child([exe], capture_output=True, text=True, timeout=30)
            if r.returncode != 0 or r.stdout.splitlines() != want:
                return f"threads {mode}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    return None


def linux_test(tmp):
    """The Linux target: the file must look right, and where qemu and an Alpine image exist it must also run.
    Returns (message, ran) where ran says whether the programs really executed on Linux."""
    src = ROOT / "tests/run/arith.claw"
    for target, interp in (("aarch64-linux", b"/lib/ld-linux-aarch64.so.1"), ("aarch64-linux-musl", b"/lib/ld-musl-aarch64.so.1")):
        exe = tmp / ("linux_" + target)
        p = child([CLAW, "build", "--target", target, src, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if p.returncode != 0:
            return f"{target}: compile failed:\n{p.stderr}", False
        data = exe.read_bytes()
        if data[:4] != b"\x7fELF" or int.from_bytes(data[18:20], "little") != 183 or interp not in data[:512]:
            return f"{target}: not an aarch64 ELF with the right loader", False
    if not shutil.which("qemu-system-aarch64") or not glob.glob(os.path.expanduser("~/Downloads/alpine-virt-*-aarch64.iso")):
        return None, False
    r = child([sys.executable, str(ROOT / "tools/linux_check.py")], capture_output=True, text=True, env={**os.environ, "CLAW": str(CLAW)}, timeout=1500)
    if r.returncode != 0:
        return f"on Linux:\n{r.stdout[-3000:]}{r.stderr[-1000:]}", True
    print("linux: " + r.stdout.strip().splitlines()[-1])
    return None, True


def simd_test(path, tmp):
    """SIMD vectors: the checked run, all three build modes and the interpreter must print the same."""
    text = path.read_text()
    want = headers(text, "expect")
    runs = [([CLAW, "run", path], "claw run"), ([CLAW, "__interp", path], "interpreter")]
    for mode in ([], ["--no-opt"], ["--fast"]):
        exe = tmp / ("simd_" + path.stem + "".join(mode))
        b = child([CLAW, "build", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"compile {mode} failed:\n{b.stderr}"
        runs.append(([exe], f"build {' '.join(mode)}".strip()))
    for cmd, what in runs:
        r = child(cmd, capture_output=True, text=True, env={"PATH": ""}, timeout=60)
        if r.returncode != 0 or r.stdout.splitlines() != want:
            return f"{what}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    # The x86-64 back end has no vector code yet: it must say so, not miscompile.
    x = child([CLAW, "build", "--target", "x86_64", path, "-o", tmp / "simd_x86"], capture_output=True, text=True, env={"PATH": ""})
    if x.returncode == 0 or "E0907" not in x.stderr:
        return f"--target x86_64 should refuse SIMD with E0907, got exit {x.returncode}: {x.stderr}"
    return None


def err_test(path):
    text = path.read_text()
    want = sorted(c.strip() for c in headers(text, "error"))
    if not want:
        return "test has no // error: header"
    p = child([CLAW, "check", path], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 1:
        return f"expected rejection, exit code {p.returncode}\n{p.stderr}"
    got = sorted(re.findall(r"\bE\d{4}\b", " ".join(l.split(" ")[1] for l in p.stderr.splitlines())))
    if got != want:
        return f"errors {got}, wanted {want}\n{p.stderr}"
    return None


def fmt_test(path, tmp):
    """tests/fmt/x.claw must format to tests/fmt/x.out."""
    copy = tmp / path.name
    copy.write_text(path.read_text())
    p = child([CLAW, "fmt", copy], capture_output=True, text=True, env={"PATH": ""})
    if p.returncode != 0:
        return f"fmt failed: {p.stderr}"
    got = copy.read_text()
    want = path.with_suffix(".out").read_text()
    return None if got == want else f"format differs\n--- got\n{got}--- want\n{want}"


def fmt_stable(path, tmp):
    """Formatting twice equals formatting once, for every program we have."""
    copy = tmp / ("fmt_" + path.name)
    copy.write_text(path.read_text())
    if child([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
        return "first format failed"
    once = copy.read_text()
    if child([CLAW, "fmt", copy], capture_output=True, env={"PATH": ""}).returncode != 0:
        return "second format failed"
    return None if copy.read_text() == once else "formatting is not stable"


def card_size():
    """The language must fit in a prompt. A rough count, deliberately pessimistic."""
    text = (ROOT / "docs/CARD.md").read_text()
    tokens = len(re.findall(r"[A-Za-z_]+|\d+|[^\sA-Za-z_\d]", text))
    print(f"card: about {tokens} tokens (budget 2000)")
    return None if tokens <= 2000 else f"docs/CARD.md is about {tokens} tokens"


def main():
    failures = 0
    total = 0
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        for path in sorted((ROOT / "tests/run").glob("*.claw")):
            total += 1
            msg = run_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL run/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/check").glob("*.claw")):
            total += 1
            msg = check_test(path)
            if msg:
                failures += 1
                print(f"FAIL check/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/unit").glob("*.claw")):
            total += 1
            msg = unit_test(path)
            if msg:
                failures += 1
                print(f"FAIL unit/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw"))):
            total += 1
            msg = run_again_checked(path)
            if msg:
                failures += 1
                print(f"FAIL checked/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw"))):
            total += 1
            msg = interp_test(path)
            if msg:
                failures += 1
                print(f"FAIL interp/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/own").glob("*.claw")):
            total += 1
            msg = own_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL own/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/auto").glob("*.claw")):
            total += 1
            msg = auto_test(path)
            if msg:
                failures += 1
                print(f"FAIL auto/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/why").glob("*.claw")):
            total += 1
            msg = why_test(path)
            if msg:
                failures += 1
                print(f"FAIL why/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/info").glob("*.claw")):
            total += 1
            msg = info_test(path)
            if msg:
                failures += 1
                print(f"FAIL info/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/debug").glob("*.claw")):
            total += 1
            msg = debug_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL debug/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/par").glob("*.claw")):
            total += 1
            msg = err_test(path) if path.name.startswith("err_") else par_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL par/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/simd").glob("*.claw")):
            total += 1
            msg = err_test(path) if path.name.startswith("err_") else simd_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL simd/{path.name}: {msg}")
        total += 1
        msg, ran = linux_test(tmp)
        if msg:
            failures += 1
            print(f"FAIL linux: {msg}")
        elif not ran:
            print("linux: ELF checked; programs not run (no qemu or Alpine image)")
        total += 1
        msg = bootstrap()
        if msg:
            failures += 1
            print(f"FAIL bootstrap: {msg}")
        total += 1
        msg = msg or selfhost_corpus(tmp)
        if msg:
            failures += 1
            print(f"FAIL selfhost corpus: {msg}")
        total += 1
        msg = lsp_test()
        if msg:
            failures += 1
            print(f"FAIL lsp: {msg}")
        total += 1
        msg = card_command()
        if msg:
            failures += 1
            print(f"FAIL card command: {msg}")
        for path in sorted((ROOT / "tests/fix").glob("*.claw")):
            total += 1
            msg = fix_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fix/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/fast").glob("*.claw")):
            total += 1
            msg = fast_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fast/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/stats").glob("*.claw")):
            total += 1
            msg = stats_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL stats/{path.name}: {msg}")
        total += 1
        msg = fuzz(100)
        if msg:
            failures += 1
            print(f"FAIL fuzz: {msg}")
        for path in sorted((ROOT / "tests/fmt").glob("*.claw")):
            total += 1
            msg = fmt_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fmt/{path.name}: {msg}")
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw")) + list((ROOT / "tests/check").glob("*.claw")) + list((ROOT / "tests/unit").glob("*.claw")) + list((ROOT / "tests/auto").glob("*.claw")) + list((ROOT / "tests/why").glob("*.claw")) + list((ROOT / "tests/fast").glob("*.claw")) + list((ROOT / "tests/stats").glob("*.claw")) + list((ROOT / "std").glob("*.claw")) + list((ROOT / "benchmarks").glob("*.claw"))):
            total += 1
            msg = fmt_stable(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL fmt-stable/{path.name}: {msg}")
    # The second target: the same programs as x86-64 executables (Rosetta runs them).
    # Every job has its own directory, and they run side by side: Rosetta translates each new binary on first run.
    X64 = ("--target", "x86_64")
    runs = sorted((ROOT / "tests/run").glob("*.claw"))
    owns = sorted((ROOT / "tests/own").glob("*.claw"))
    checks = sorted((ROOT / "tests/check").glob("*.claw"))
    jobs = [("x86-64 encoder", encoder_x64)]
    jobs += [(f"x86-64 {p.parent.name}/{p.name}", lambda p=p: in_temp(run_test, p, X64)) for p in runs + owns]
    jobs += [(f"x86-64 check/{p.name}", lambda p=p: check_test(p, X64)) for p in checks]
    jobs += [(f"x86-64 checked/{p.name}", lambda p=p: run_again_checked(p, X64)) for p in runs]
    jobs += [("x86-64 fuzz", lambda: fuzz_x64(60))]
    # Second batch: a few programs write to a fixed file, so no program runs twice at once.
    jobs2 = [(f"x86-64 own/{p.name}", lambda p=p: in_temp(own_test, p, X64)) for p in owns]
    for label, msg in parallel(jobs) + parallel(jobs2):
        total += 1
        if msg:
            failures += 1
            print(f"FAIL {label}: {msg}")
    for path in sorted((ROOT / "tests/err").glob("*.claw")):
        total += 1
        msg = err_test(path)
        if msg:
            failures += 1
            print(f"FAIL err/{path.name}: {msg}")
    total += 1
    msg = card_size()
    if msg:
        failures += 1
        print(f"FAIL card: {msg}")
    print(f"{total - failures}/{total} passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
