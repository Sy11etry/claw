#!/usr/bin/env python3
"""Runs every test of every iteration against the freshly built compiler.

tests/run/*.claw  compiled and executed. Headers:
    // expect: <stdout line>     (repeatable)
    // exit: <code>              (default 0)
    // stderr: <substring>       (repeatable)
tests/err/*.claw  must be rejected. Header:
    // error: E####              (repeatable; all must appear, nothing else)
tests/err/<dir>/  a project (claw.pkg, main.claw) rejected after one of its packages changed: PROJECT_CHANGES says how.
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
examples/*.claw  the showcase programs: built, run checked, and run as x86-64.
tests/simd/*.claw  SIMD vectors: checked run, three build modes and the interpreter agree; err_*.claw must be rejected.
tests/stack/*.claw  deep recursion works everywhere; a full stack or a wild pointer is reported, not a silent crash.
tests/trace/*.claw  the flight recorder: a trap leaves a trace when asked, `claw why x.clawtrace` replays it.
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
    for flags in ([], ["--no-opt"], ["--target", "x86_64"], ["--target", "x86_64", "--no-opt"]):
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
        plain = "--no-opt" in flags
        wanted = headers(text, "frame") if plain else [f"{path.name}:{line}"]
        for want in wanted:
            where = want.split(" at ")[-1]
            if not re.search(rf"at {re.escape(where)}", out):
                problems.append(f"{' '.join(flags) or 'optimized'}: nothing at `{want}` in:\n{out}")
        if plain:
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
    # The x86-64 target (Rosetta) starts its threads through a small shim: it must work too.
    for mode in ([], ["--fast"], ["--no-opt"], ["--target", "x86_64"], ["--target", "x86_64", "--no-opt"]):
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
    # The x86-64 back end (SSE, Rosetta here) must print the same.
    for mode in ([], ["--no-opt"]):
        exe = tmp / ("simd_x86_" + path.stem + "".join(mode))
        b = child([CLAW, "build", "--target", "x86_64", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"x86-64 compile {mode} failed:\n{b.stderr}"
        r = child([exe], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
        if r.returncode != 0 or r.stdout.splitlines() != want:
            return f"x86-64 {' '.join(mode)}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    return None


def stack_test(path, tmp):
    """The stack: deep recursion works in every mode, and running out of stack (or a wild pointer in `claw fn`)
    says so on stderr after what was printed, instead of a silent crash. `// crash: yes` means the program must
    end by the signal (SIGSEGV or SIGBUS), `// build-only` leaves out `claw run`, where the checker reports first."""
    text = path.read_text()
    want = headers(text, "expect")
    crash = bool(headers(text, "crash"))
    runs = [] if headers(text, "build-only") else [([CLAW, "run", path], "claw run")]
    for mode in ([], ["--no-opt"], ["--fast"], ["--target", "x86_64"], ["--target", "x86_64", "--no-opt"]):
        exe = tmp / ("stack_" + path.stem + "".join(mode).replace("-", ""))
        b = child([CLAW, "build", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"compile {mode} failed:\n{b.stderr}"
        runs.append(([exe], f"build {' '.join(mode)}".strip()))
    for cmd, what in runs:
        r = child(cmd, capture_output=True, text=True, env={"PATH": ""}, timeout=60)
        ended = r.returncode in (-10, -11, 138, 139) if crash else r.returncode == 0
        if not ended or r.stdout.splitlines() != want:
            return f"{what}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
        for s in headers(text, "stderr"):
            if s not in r.stderr:
                return f"{what}: stderr lacks {s!r}: {r.stderr!r}"
    return None


def trace_test(path, tmp):
    for mode in ([], ["--target", "x86_64"]):
        msg = trace_one(path, tmp, mode)
        if msg:
            return f"{' '.join(mode) or 'arm64'}: {msg}"
    return None


def trace_one(path, tmp, mode):
    """The flight recorder: a program that traps leaves a trace only when `CLAW_RECORD` names a file, and
    `claw why x.clawtrace source.claw` plays the same crash again after the world has changed (the input file is
    deleted, the environment variable is gone). Headers: `// input:` text of the file `{file}` in `// args:`,
    `// env: NAME=value`, `// why:` text the replay must print, `// clean: yes` a program that ends well."""
    text = path.read_text()
    exe = tmp / ("trace_" + path.stem + "".join(mode).replace("-", ""))
    b = child([CLAW, "build", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
    if b.returncode != 0:
        return f"compile failed:\n{b.stderr}"
    data = tmp / (path.stem + ".input")
    inputs = headers(text, "input")
    if inputs:
        data.write_text("\n".join(inputs) + "\n")
    args = [a.replace("{file}", str(data)) for a in " ".join(headers(text, "args")).split()]
    env = {"PATH": ""}
    for e in headers(text, "env"):
        k, v = e.split("=", 1)
        env[k] = v
    trace = tmp / (path.stem + ".clawtrace")
    trace.unlink(missing_ok=True)
    clean = bool(headers(text, "clean"))
    want = 0 if clean else 101
    r = child([exe, *args], capture_output=True, text=True, env=env)
    if r.returncode != want or trace.exists():
        return f"without CLAW_RECORD: exit {r.returncode} (wanted {want}), trace written: {trace.exists()}"
    r = child([exe, *args], capture_output=True, text=True, env={**env, "CLAW_RECORD": str(trace)})
    if r.returncode != want:
        return f"recording: exit {r.returncode}, wanted {want}\n{r.stderr}"
    if clean:
        return None if not trace.exists() else "a program that ended well left a trace"
    if not trace.exists():
        return "the program trapped but no trace file was written"
    # The world changes: the input file is gone, the variables are gone.
    data.unlink(missing_ok=True)
    w = child([CLAW, "why", trace, path], capture_output=True, text=True, env={"PATH": ""})
    if w.returncode != 1:
        return f"why: exit {w.returncode}\n{w.stdout}{w.stderr}"
    for s in headers(text, "why"):
        if s not in w.stdout:
            return f"why lacks {s!r}:\n{w.stdout}"
    # Another program is not the same crash, and it says so.
    other = tmp / (path.stem + "_other.claw")
    other.write_text("fn main() {\n    print(1)\n}\n")
    o = child([CLAW, "why", trace, other], capture_output=True, text=True, env={"PATH": ""})
    if o.returncode != 1 or "not reproduced" not in o.stdout:
        return f"another source: exit {o.returncode}\n{o.stdout}{o.stderr}"
    return None


def cinterop_test(path, tmp):
    """C libraries and headers: `use c`, `link`, callbacks. A program that needs the test library gets it built
    here with clang (a test fixture, never part of building Claw), once for each architecture."""
    text = path.read_text()
    want = headers(text, "expect")
    # `// build-error: E####`: the program checks, but building it must fail (a C symbol no library has).
    for code in headers(text, "build-error"):
        b = child([CLAW, "build", path, "-o", tmp / ("ci_err_" + path.stem)], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode == 0 or code not in b.stderr:
            return f"build should fail with {code}: exit {b.returncode}\n{b.stderr}"
        return None
    needs_lib = 'link "mylib"' in text
    modes = [[], ["--fast"], ["--no-opt"], ["--target", "x86_64"], ["--target", "x86_64", "--no-opt"]]
    for mode in modes:
        x86 = "x86_64" in mode
        libdir = tmp / ("cilib_x86" if x86 else "cilib_arm")
        extra = []
        if needs_lib:
            if not libdir.exists():
                libdir.mkdir()
                src = ROOT / "tests/cinterop/mylib.c"
                arch = ["-arch", "x86_64"] if x86 else []
                r = child(["clang", *arch, "-dynamiclib", "-o", libdir / "libmylib.dylib", src], capture_output=True, text=True)
                if r.returncode != 0:
                    return f"clang could not build the test library: {r.stderr}"
            extra = ["-L", libdir]
        exe = tmp / ("ci_" + path.stem + "".join(mode).replace("-", ""))
        b = child([CLAW, "build", *mode, *extra, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"build {mode}: {b.stderr}"
        r = child([exe], capture_output=True, text=True, timeout=30)
        if r.returncode != 0 or r.stdout.splitlines() != want:
            return f"{mode}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    if not needs_lib:
        r = child([CLAW, "run", path], capture_output=True, text=True, env={"PATH": ""}, timeout=60)
        if r.returncode != 0 or r.stdout.splitlines() != want:
            return f"claw run: exit {r.returncode}, got {r.stdout.splitlines()}\n{r.stderr}"
    return None


def script_test(path, tmp):
    """`claw script.claw args...` (a `#!` line is allowed), and the same program built for both architectures."""
    text = path.read_text()
    args = (headers(text, "args") or [""])[0].split()
    want = headers(text, "expect")
    want_exit = int((headers(text, "exit") or ["0"])[0])
    r = child([CLAW, path, *args], capture_output=True, text=True, env={"PATH": "", "HOME": os.environ.get("HOME", "")})
    if r.returncode != want_exit or r.stdout.splitlines() != want:
        return f"script: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    for mode in ([], ["--fast"], ["--target", "x86_64"]):
        exe = tmp / ("sc_" + path.stem + "".join(mode).replace("-", ""))
        b = child([CLAW, "build", *mode, path, "-o", exe], capture_output=True, text=True, env={"PATH": ""})
        if b.returncode != 0:
            return f"build {mode}: {b.stderr}"
        r = child([exe, *args], capture_output=True, text=True, env={"PATH": "", "HOME": os.environ.get("HOME", "")})
        if r.returncode != want_exit or r.stdout.splitlines() != want:
            return f"{mode}: exit {r.returncode}, got {r.stdout.splitlines()}, wanted {want}\n{r.stderr}"
    return None


def pkg_test(tmp):
    """Packages: a folder dependency and a git dependency, pinned by claw.lock; a changed copy is refused."""
    root = tmp / "pkgtest"
    shutil.copytree(ROOT / "tests/pkg", root)
    app = root / "app"
    want = headers((app / "main.claw").read_text(), "expect")
    env = {"PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"}
    r = child([CLAW, "get"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode != 0 or "fetched" not in r.stdout:
        return f"claw get: {r.stdout}{r.stderr}"
    r = child([CLAW, "run", "main.claw"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode != 0 or r.stdout.splitlines() != want:
        return f"run with a folder package: {r.stdout.splitlines()} {r.stderr}"
    if "up to date" not in child([CLAW, "get"], cwd=app, capture_output=True, text=True, env=env).stdout:
        return "a second `claw get` should find the pinned copy"
    # The same package from a git repository.
    repo = root / "greet_repo"
    shutil.copytree(root / "greet", repo)
    git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]
    for cmd in (["init", "-q", "-b", "main"], ["add", "."], ["commit", "-q", "-m", "x"]):
        if child([*git, *cmd], cwd=repo, capture_output=True, text=True, env=env).returncode != 0:
            return "git is needed for this test"
    app2 = root / "app2"
    shutil.copytree(app, app2, ignore=shutil.ignore_patterns(".claw", "claw.lock"))
    (app2 / "claw.pkg").write_text(f"greet file://{repo}#main\n")
    r = child([CLAW, "get"], cwd=app2, capture_output=True, text=True, env=env)
    if r.returncode != 0:
        return f"claw get from git: {r.stdout}{r.stderr}"
    r = child([CLAW, "run", "main.claw"], cwd=app2, capture_output=True, text=True, env=env)
    if r.returncode != 0 or r.stdout.splitlines() != want:
        return f"run with a git package: {r.stdout.splitlines()} {r.stderr}"
    # A copy edited by hand no longer matches the lock.
    with open(app2 / ".claw/pkg/greet/greet.claw", "a") as f:
        f.write("// edited\n")
    r = child([CLAW, "get"], cwd=app2, capture_output=True, text=True, env=env)
    if r.returncode == 0 or "changed" not in r.stderr:
        return f"an edited package must be refused: {r.stdout}{r.stderr}"
    return None


def pkg_lock_test(tmp):
    """claw.pkg and claw.lock: `claw get` refuses a source that changed (until `--update` takes it), run and build
    refuse a copy edited by hand, and `claw get --update` puts either one back."""
    root = tmp / "pkglock"
    shutil.copytree(ROOT / "tests/pkg", root)
    app = root / "app"
    want = headers((app / "main.claw").read_text(), "expect")
    env = {"PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"}

    def in_app(*args):
        return child([CLAW, *args], cwd=app, capture_output=True, text=True, env=env)

    if in_app("get").returncode != 0:
        return "claw get failed"
    for line in (app / "claw.lock").read_text().splitlines():
        if len(line.split()) != 3:
            return f"claw.lock should write `name source hash`, got: {line}"
    # The source changes in claw.pkg: `claw get` refuses without --update, and run refuses too.
    shutil.copytree(root / "greet", root / "greet_b")
    (app / "claw.pkg").write_text("greet ../greet_b\n")
    r = in_app("get")
    if r.returncode == 0 or "claw.pkg says" not in r.stderr or "--update" not in r.stderr:
        return f"claw get must refuse a changed source: {r.stdout}{r.stderr}"
    r = in_app("run", "main.claw")
    if r.returncode == 0 or "E0183" not in r.stderr or r.stdout:
        return f"claw run must refuse a changed source: {r.stdout}{r.stderr}"
    if in_app("get", "--update").returncode != 0:
        return "claw get --update must take the new source"
    r = in_app("run", "main.claw")
    if r.returncode != 0 or r.stdout.splitlines() != want:
        return f"run after --update: {r.stdout.splitlines()} {r.stderr}"
    # A copy edited by hand: run and build refuse it, and `claw get --update` puts the source back.
    with open(app / ".claw/pkg/greet/greet.claw", "a") as f:
        f.write("// edited\n")
    r = in_app("run", "main.claw")
    if r.returncode == 0 or "E0183" not in r.stderr or "changed" not in r.stderr:
        return f"claw run must refuse an edited copy: {r.stdout}{r.stderr}"
    r = in_app("build", "main.claw", "-o", str(tmp / "pkglock_app"))
    if r.returncode == 0 or "E0183" not in r.stderr:
        return f"claw build must refuse an edited copy: {r.stdout}{r.stderr}"
    if in_app("get", "--update").returncode != 0:
        return "claw get --update must restore the copy"
    r = in_app("run", "main.claw")
    if r.returncode != 0 or r.stdout.splitlines() != want:
        return f"run after restoring: {r.stdout.splitlines()} {r.stderr}"
    return None


def pkg_nested_test(tmp):
    """A package with a claw.pkg of its own: `claw get` fetches what it needs too and pins it in the same lock;
    a folder with no .claw files is refused."""
    root = tmp / "pkgnested"
    shutil.copytree(ROOT / "tests/pkg", root)
    app = root / "nested_app"
    want = headers((app / "main.claw").read_text(), "expect")
    env = {"PATH": "/usr/bin:/bin"}
    r = child([CLAW, "get"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode != 0 or "inner: fetched" not in r.stdout:
        return f"claw get must fetch the package's own dependency: {r.stdout}{r.stderr}"
    names = [l.split()[0] for l in (app / "claw.lock").read_text().splitlines()]
    if sorted(names) != ["inner", "outer"]:
        return f"claw.lock should pin both packages: {names}"
    r = child([CLAW, "run", "main.claw"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode != 0 or r.stdout.splitlines() != want:
        return f"run with a nested package: {r.stdout.splitlines()} {r.stderr}"
    with open(app / ".claw/pkg/inner/inner.claw", "a") as f:
        f.write("// edited\n")
    r = child([CLAW, "run", "main.claw"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode == 0 or "E0183" not in r.stderr:
        return f"an edited nested package must be refused: {r.stdout}{r.stderr}"
    (app / "claw.pkg").write_text("hole ../empty\n")
    r = child([CLAW, "get", "--update"], cwd=app, capture_output=True, text=True, env=env)
    if r.returncode == 0 or "no .claw files" not in r.stderr:
        return f"an empty package must be refused: {r.stdout}{r.stderr}"
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


def pin_by_source(app):
    """The package's line in claw.lock is rewritten as an older claw wrote it: name and hash only."""
    name, _, digest = (app / "claw.lock").read_text().split()
    (app / "claw.lock").write_text(f"{name} {digest}\n")


def move_source(app):
    """claw.pkg now names another folder than the one the lock pins."""
    (app / "claw.pkg").write_text("geo ./geo_v2\n")


def edit_copy(app):
    """A copy of the package in .claw/pkg is changed by hand after `claw get`."""
    (app / ".claw/pkg/geo/geo.claw").write_text("pub fn square(x int) int {\n    ret x * x + 1\n}\n")


# What happens to each project in tests/err/<dir>/ after `claw get`, before `claw check` must reject it.
PROJECT_CHANGES = {"pkg_edited": edit_copy, "pkg_moved_source": move_source, "pkg_old_lock": pin_by_source}


def err_project_test(path):
    """tests/err/<dir>/: a project (claw.pkg, main.claw) whose packages change after they were pinned. The
    `// error:` header of its main.claw lists the codes that `claw check` must give."""
    text = (path / "main.claw").read_text()
    want = sorted(c.strip() for c in headers(text, "error"))
    change = PROJECT_CHANGES.get(path.name)
    if not want or change is None:
        return "test has no // error: header, or no change is defined for it in PROJECT_CHANGES"
    with tempfile.TemporaryDirectory() as d:
        app = pathlib.Path(d) / path.name
        shutil.copytree(path, app)
        env = {"PATH": ""}
        r = child([CLAW, "get"], cwd=app, capture_output=True, text=True, env=env)
        if r.returncode != 0:
            return f"claw get: {r.stderr}"
        change(app)
        p = child([CLAW, "check", app / "main.claw"], capture_output=True, text=True, env=env)
        if p.returncode != 1:
            return f"expected rejection, exit code {p.returncode}\n{p.stderr}"
        got = sorted(re.findall(r"^\S+:\d+:\d+ (E\d{4}) ", p.stderr, re.M))
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
    print(f"card: about {tokens} tokens (budget 2500)")
    return None if tokens <= 2500 else f"docs/CARD.md is about {tokens} tokens"


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
        for path in sorted((ROOT / "tests/cinterop").glob("*.claw")):
            total += 1
            msg = err_test(path) if path.name.startswith("err_") else cinterop_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL cinterop/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/stack").glob("*.claw")):
            total += 1
            msg = stack_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL stack/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/trace").glob("*.claw")):
            total += 1
            msg = trace_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL trace/{path.name}: {msg}")
        for path in sorted((ROOT / "tests/script").glob("*.claw")):
            total += 1
            msg = script_test(path, tmp)
            if msg:
                failures += 1
                print(f"FAIL script/{path.name}: {msg}")
        total += 1
        msg = pkg_test(tmp)
        if msg:
            failures += 1
            print(f"FAIL pkg: {msg}")
        total += 1
        msg = pkg_lock_test(tmp)
        if msg:
            failures += 1
            print(f"FAIL pkg lock: {msg}")
        total += 1
        msg = pkg_nested_test(tmp)
        if msg:
            failures += 1
            print(f"FAIL pkg nested: {msg}")
        for path in sorted((ROOT / "examples").glob("*.claw")):
            total += 1
            msg = run_test(path, tmp) or check_test(path)
            if msg:
                failures += 1
                print(f"FAIL examples/{path.name}: {msg}")
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
        for path in sorted(list((ROOT / "tests/run").glob("*.claw")) + list((ROOT / "tests/own").glob("*.claw")) + list((ROOT / "tests/check").glob("*.claw")) + list((ROOT / "tests/unit").glob("*.claw")) + list((ROOT / "tests/auto").glob("*.claw")) + list((ROOT / "tests/why").glob("*.claw")) + list((ROOT / "tests/fast").glob("*.claw")) + list((ROOT / "tests/stats").glob("*.claw")) + list((ROOT / "std").glob("*.claw")) + list((ROOT / "benchmarks").glob("*.claw")) + list((ROOT / "examples").glob("*.claw"))):
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
    examples = sorted((ROOT / "examples").glob("*.claw"))
    jobs = [("x86-64 encoder", encoder_x64)]
    jobs += [(f"x86-64 examples/{p.name}", lambda p=p: in_temp(run_test, p, X64)) for p in examples]
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
    for path in sorted(p for p in (ROOT / "tests/err").iterdir() if p.is_dir() and (p / "main.claw").is_file()):
        total += 1
        msg = err_project_test(path)
        if msg:
            failures += 1
            print(f"FAIL err/{path.name}/: {msg}")
    total += 1
    msg = card_size()
    if msg:
        failures += 1
        print(f"FAIL card: {msg}")
    print(f"{total - failures}/{total} passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
