#!/usr/bin/env python3
"""Second pass over the error codes: corrected programs, extra cases, and `claw fix` checks."""
import json
import pathlib
import shutil
import subprocess

import gen_e as g

HERE = pathlib.Path(__file__).resolve().parent
CLAW = g.CLAW
P = dict(g.P)
P["E0102"] = ("builtin_name", 'fn len(a int) int {\n    ret a\n}\nfn main() {\n    print(1)\n}\n', "run")
P["E0125"] = ("discard_error", 'type E = A\nfn g() int ! E {\n    fail A\n}\nfn main() {\n    _ = g()\n    print(1)\n}\n', "run")
P["E0126"] = ("zero_array", 'fn main() {\n    x [0]int = [0...]\n    print(len(x))\n}\n', "run")
P["E0133"] = ("unclosed_brace", 'fn main() {\n    print("a { b")\n}\n', "run")
P["E0144"] = ("too_many_type_args", 'fn pair[A](a A) A {\n    ret a\n}\nfn main() {\n    print(pair[int, int](1))\n}\n', "run")
P["E0153"] = ("owner_repeat_typed", 'fn main() {\n    mut v []int = [1]\n    push(&v, 2)\n    a [2][]int = [v...]\n    print(len(a))\n}\n', "run")
P["E0163"] = ("old_of_array", 'fn f(a []int) int\n    sure ret == old(a)\n{\n    ret 1\n}\nfn main() {\n    print(f([1]))\n}\n', "run")
P["E0202"] = ("keyword_param", 'fn f(in int) int {\n    ret in\n}\nfn main() {\n    print(f(1))\n}\n', "run")
P["E0901"] = ("function_too_big", "fn big(x int) int {\n    mut a int = x\n" + "    a += 1\n" * 130000 + "    ret a\n}\nfn main() {\n    print(big(1))\n}\n", "run")

def expected_found(res, code):
    return code in res["codes"]

def main():
    results = {}
    for code, (name, text, cmd) in P.items():
        r = g.run_one(code, name, text, cmd)
        r["expected"] = code
        results[code] = r
    # Known-missing extra case: a struct with one type parameter given two.
    extra = g.run_one("E0144", "struct_extra_type_args",
                      'type Box[T] { v T }\nfn main() {\n    b := Box[int, int]{1}\n    print(b.v)\n}\n', "run")
    extra["expected"] = "E0144"
    results["E0144 (struct case)"] = extra
    # claw fix on every error whose message carries a fix suggestion.
    fixdir = HERE / "fixed"
    fixdir.mkdir(exist_ok=True)
    for code, r in results.items():
        r["fix"] = None
        if r.get("found") and "fix" in r["line"]:
            src = HERE / r["file"]
            dst = fixdir / r["file"]
            shutil.copy(src, dst)
            fx = subprocess.run([CLAW, "fix", dst.name], cwd=fixdir, capture_output=True, text=True)
            after = subprocess.run([CLAW, "run", dst.name], cwd=fixdir, capture_output=True, text=True)
            out = after.stdout + after.stderr
            still = [c for c in CODE_RE.findall(out)]
            r["fix"] = {"fix_says": (fx.stdout + fx.stderr).strip().splitlines()[0][:80] if (fx.stdout + fx.stderr).strip() else "",
                        "still_errors": still[:3], "compiles": after.returncode == 0}
    (HERE / "errors_results2.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    hit = sum(1 for k, r in results.items() if r["found"])
    for code, r in results.items():
        mark = "OK  " if r["found"] else "MISS"
        extra_fix = ""
        if r.get("fix") is not None:
            extra_fix = f" | claw fix: {'compiles' if r['fix']['compiles'] else 'still ' + ','.join(r['fix']['still_errors'])}"
        print(f"{mark} {code} {r['line'][:150] if r['found'] else '| got ' + ','.join(r['codes'])}{extra_fix}")
    print(f"{hit}/{len(results)} triggered")

import re
CODE_RE = re.compile(r"\bE\d{4}\b")

if __name__ == "__main__":
    main()
