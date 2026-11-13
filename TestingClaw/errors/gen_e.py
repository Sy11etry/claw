#!/usr/bin/env python3
"""One small program per error code in docs/SPEC.md. Each program should trigger its code.

Run: python3 gen_e.py   -> writes eNNNN_<name>.claw files and errors_results.json.
"""
import json
import pathlib
import re
import shutil
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
CLAW = shutil.which("claw") or str(pathlib.Path.home() / ".claw/bin/claw")
CODE = re.compile(r"\bE\d{4}\b")

# code: (name, program, command)
P = {
 "E0001": ("unexpected_char", 'fn main() {\n    x := 1 @ 2\n    print(x)\n}\n', "run"),
 "E0002": ("string_not_closed", 'fn main() {\n    s := "abc\n    print(s)\n}\n', "run"),
 "E0003": ("number_runs_into_name", 'fn main() {\n    x := 12abc\n    print(x)\n}\n', "run"),
 "E0004": ("unknown_escape", 'fn main() {\n    s := "a\\q"\n    print(s)\n}\n', "run"),
 "E0005": ("char_two_chars", "fn main() {\n    c := 'ab'\n    print(c)\n}\n", "run"),
 "E0101": ("unknown_name", 'fn main() {\n    print(zz)\n}\n', "run"),
 "E0102": ("name_taken", 'const limit = 1\nfn limit() int {\n    ret 2\n}\nfn main() {\n    print(limit)\n}\n', "run"),
 "E0103": ("assign_immutable", 'fn main() {\n    x := 1\n    x = 2\n    print(x)\n}\n', "run"),
 "E0104": ("type_mismatch", 'fn main() {\n    x i32 = "s"\n    print(x)\n}\n', "run"),
 "E0105": ("never_read", 'fn main() {\n    x := 1\n}\n', "run"),
 "E0106": ("wrong_arg_count", 'fn f(a int) int {\n    ret a\n}\nfn main() {\n    print(f(1, 2))\n}\n', "run"),
 "E0107": ("missing_ret", 'fn f(a int) int {\n    if a > 0 {\n        ret 1\n    }\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0108": ("break_outside_loop", 'fn main() {\n    break\n}\n', "run"),
 "E0109": ("main_with_args", 'fn main(a int) {\n    print(a)\n}\n', "run"),
 "E0110": ("fn_twice", 'fn f() {\n}\nfn f() {\n}\nfn main() {\n    f()\n}\n', "run"),
 "E0111": ("result_unused", 'fn g() int {\n    ret 1\n}\nfn main() {\n    g()\n}\n', "run"),
 "E0112": ("unknown_type", 'fn main() {\n    x Foo = 1\n    print(x)\n}\n', "run"),
 "E0113": ("unreachable", 'fn main() {\n    ret\n    print(1)\n}\n', "run"),
 "E0114": ("no_effect_expr", 'fn main() {\n    1 + 2\n}\n', "run"),
 "E0115": ("int_too_big", 'fn main() {\n    x := 9223372036854775808\n    print(x)\n}\n', "run"),
 "E0116": ("mut_unchanged", 'fn main() {\n    mut x := 1\n    print(x)\n}\n', "run"),
 "E0117": ("else_block_no_exit", 'type E = A\nfn g() int ! E {\n    fail A\n}\nfn main() {\n    v := g() else err {\n        print("x")\n    }\n    print(v)\n}\n', "run"),
 "E0118": ("pattern_wrong_type", 'type S = A{r int} | B\nfn main() {\n    x := 1\n    match x {\n        A{r}: print(r)\n        B: print(0)\n    }\n}\n', "run"),
 "E0119": ("pattern_wrong_names", 'type S = A{r int} | B\nfn f(s S) int {\n    match s {\n        A{r, q}: ret r\n        B: ret 0\n    }\n}\nfn main() {\n    print(f(B))\n}\n', "run"),
 "E0120": ("match_not_exhaustive", 'type S = A | B\nfn f(s S) int {\n    match s {\n        A: ret 1\n    }\n}\nfn main() {\n    print(f(A))\n}\n', "run"),
 "E0121": ("case_twice", 'type S = A | B\nfn f(s S) int {\n    match s {\n        A: ret 1\n        A: ret 2\n        B: ret 0\n    }\n}\nfn main() {\n    print(f(A))\n}\n', "run"),
 "E0122": ("array_changes_in_for", 'fn main() {\n    mut a []int = [1, 2]\n    for x in a {\n        push(&a, x)\n    }\n    print(len(a))\n}\n', "run"),
 "E0123": ("type_unknown_from_context", 'fn main() {\n    x := none\n    print(x)\n}\n', "run"),
 "E0124": ("fail_in_plain_fn", 'fn f() int {\n    fail 1\n}\nfn main() {\n    print(f())\n}\n', "run"),
 "E0125": ("error_ignored", 'type E = A\nfn g() int ! E {\n    fail A\n}\nfn main() {\n    g()\n}\n', "run"),
 "E0126": ("array_size_zero", 'fn main() {\n    x [0]int\n    print(len(x))\n}\n', "run"),
 "E0127": ("type_contains_itself", 'type T {\n    t T\n    v int\n}\nfn main() {\n    print(1)\n}\n', "run"),
 "E0128": ("type_name_lowercase", 'type point {\n    x int\n}\nfn main() {\n    print(1)\n}\n', "run"),
 "E0129": ("no_such_field", 'type P { x int }\nfn main() {\n    p := P{1}\n    print(p.zz)\n}\n', "run"),
 "E0130": ("const_index_out_of_range", 'fn main() {\n    a := [1, 2]\n    print(a[5])\n}\n', "run"),
 "E0131": ("cannot_assign_here", 'fn f() int {\n    ret 1\n}\nfn main() {\n    f() = 3\n}\n', "run"),
 "E0132": ("unknown_effect", 'fn f() int +wat {\n    ret 1\n}\nfn main() {\n    print(f())\n}\n', "run"),
 "E0133": ("bad_braces_in_string", 'fn main() {\n    print("x {1 +}")\n}\n', "run"),
 "E0134": ("optional_optional", 'fn main() {\n    x ??int = none\n    print(1)\n}\n', "run"),
 "E0135": ("range_two_names", 'fn main() {\n    for a, b in 0..3 {\n        print(a)\n    }\n}\n', "run"),
 "E0136": ("repeat_not_alone", 'fn main() {\n    a := [1, 0...]\n    print(len(a))\n}\n', "run"),
 "E0137": ("field_twice", 'type P {\n    x int,\n    x int\n}\nfn main() {\n    print(1)\n}\n', "run"),
 "E0138": ("optional_has_no_error", 'fn f() ?int {\n    ret none\n}\nfn main() {\n    v := f() else err {\n        ret\n    }\n    print(v)\n}\n', "run"),
 "E0139": ("const_call_needs_type", 'fn twice(x int) int {\n    ret x * 2\n}\nconst big = twice(3)\nfn main() {\n    print(big)\n}\n', "run"),
 "E0140": ("pointer_outside_claw", 'fn main() {\n    p := alloc[int](1)\n    print(1)\n}\n', "run"),
 "E0141": ("extern_from_plain_fn", 'extern fn abs(x i32) i32\nfn main() {\n    print(abs(-1))\n}\n', "run"),
 "E0142": ("defer_outside_claw", 'fn main() {\n    defer print("x")\n}\n', "run"),
 "E0143": ("defer_exits_function", 'claw fn f() int {\n    defer ret 3\n    ret 1\n}\nfn main() {\n    print(f())\n}\n', "run"),
 "E0144": ("template_params_unknown", 'type Box[T] { v T }\nfn main() {\n    b := Box{1}\n    print(1)\n}\n', "run"),
 "E0145": ("template_expands_forever", 'fn f[T](x T) int {\n    ret f([x, x])\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0146": ("module_missing", 'use nothere\nfn main() {\n    print(1)\n}\n', "run"),
 "E0149": ("type_to_c", 'extern fn tell(s str) i32\nclaw fn t() int {\n    ret int(tell("x"))\n}\nfn main() {\n    print(t())\n}\n', "run"),
 "E0150": ("no_methods", 'fn main() {\n    a := [1, 2]\n    print(a.len())\n}\n', "run"),
 "E0151": ("value_moved", 'fn main() {\n    mut a []int = []\n    b := a\n    print(len(a))\n    print(len(b))\n}\n', "run"),
 "E0153": ("repeat_owning", 'fn main() {\n    s := ["x"...]\n    print(len(s))\n}\n', "run"),
 "E0154": ("copy_type_with_drop", 'type H { v int }\nfn drop(x &H) {\n}\nfn main() {\n    h := H{1}\n    g := copy(h)\n    print(g.v)\n}\n', "run"),
 "E0156": ("drop_called_directly", 'type H { v int }\nfn drop(x &H) {\n}\nfn main() {\n    h := H{1}\n    drop(&h)\n    print(1)\n}\n', "run"),
 "E0157": ("ampersand_not_argument", 'fn main() {\n    mut x := 1\n    y := &x\n    print(y)\n}\n', "run"),
 "E0158": ("overlapping_ampersand", 'fn two(a &int, b &int) {\n    a = b\n}\nfn main() {\n    mut x := 1\n    two(&x, &x)\n}\n', "run"),
 "E0160": ("pub_effect_missing", 'pub fn f() []int {\n    mut a []int = []\n    push(&a, 1)\n    ret a\n}\nfn main() {\n    print(len(f()))\n}\n', "run"),
 "E0161": ("contract_with_effect", 'fn g() int +io {\n    print(1)\n    ret 1\n}\nfn f(a int) int\n    need g() == 1\n{\n    ret a\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0162": ("ret_in_unit_contract", 'fn f(a int)\n    sure ret == 1\n{\n    print(a)\n}\nfn main() {\n    f(1)\n}\n', "run"),
 "E0163": ("bad_old", 'fn f(a int) int\n    sure ret == old(a + 1)\n{\n    ret a\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0164": ("contract_changes_state", 'fn bump(x &int) bool {\n    ret true\n}\nfn f(a int) int\n    need bump(&a)\n{\n    ret a\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0165": ("sure_reads_own_param", 'fn f(own s str) int\n    sure len(s) > 0\n{\n    ret 1\n}\nfn main() {\n    print(f("x"))\n}\n', "run"),
 "E0166": ("contract_on_main", 'fn main()\n    need true\n{\n    print(1)\n}\n', "run"),
 "E0167": ("two_tests_same_name", 'test "a" {\n    sure 1 == 1\n}\ntest "a" {\n    sure 2 == 2\n}\nfn main() {\n    print(1)\n}\n', "test"),
 "E0168": ("need_in_body", 'fn f(a int) int {\n    need a > 0\n    ret a\n}\nfn main() {\n    print(f(1))\n}\n', "run"),
 "E0171": ("const_call_io", 'fn printer() int +io {\n    print(1)\n    ret 1\n}\nconst x int = printer()\nfn main() {\n    print(x)\n}\n', "run"),
 "E0173": ("const_type_not_simple", 'fn mk() []int {\n    ret [1]\n}\nconst v []int = mk()\nfn main() {\n    print(len(v))\n}\n', "run"),
 "E0174": ("const_fails", 'fn bad(x int) int {\n    ret 10 / x\n}\nconst c int = bad(0)\nfn main() {\n    print(c)\n}\n', "run"),
 "E0175": ("const_cycle", 'fn fa() int {\n    ret b\n}\nconst a int = fa()\nconst b int = a + 1\nfn main() {\n    print(a)\n}\n', "run"),
 "E0176": ("template_as_value", 'fn twice[T](x T) T {\n    ret x\n}\nfn main() {\n    f := twice\n    print(1)\n}\n', "run"),
 "E0177": ("need_fn_as_value", 'fn needy(x int) int\n    need x > 0\n{\n    ret x\n}\nfn main() {\n    f := needy\n    print(1)\n}\n', "run"),
 "E0178": ("io_not_in_type", 'fn work(x int) int +io {\n    print(x)\n    ret x\n}\nfn main() {\n    f fn(int) int = work\n    print(1)\n}\n', "run"),
 "E0179": ("print_recursive_value", 'type T = A{kids []T} | B\nfn main() {\n    t := A{[B]}\n    print(t)\n}\n', "run"),
 "E0201": ("expected_token", 'fn main() {\n    print(1\n}\n', "run"),
 "E0202": ("keyword_as_name", 'fn main() {\n    ret := 1\n    print(ret)\n}\n', "run"),
 "E0203": ("else_on_new_line", 'fn main() {\n    if true {\n        print(1)\n    }\n    else {\n        print(2)\n    }\n}\n', "run"),
 "E0204": ("block_not_closed", 'fn main() {\n    print(1)\n', "run"),
 "E0205": ("two_statements_one_line", 'fn main() {\n    x := 1 y := 2\n    print(x)\n}\n', "run"),
 "E0905": ("vector_outside_claw", 'fn main() {\n    v := f64x2(1.0)\n    print(1)\n}\n', "run"),
 "E0906": ("vector_in_field", 'type V { v f64x2 }\nfn main() {\n    print(1)\n}\n', "run"),
 "E0908": ("vload_bad_argument", 'claw fn g() f64 {\n    ret lane(vload(1), 0)\n}\nfn main() {\n    print(1)\n}\n', "run"),
 "E0909": ("int_vector_divide", 'claw fn g() int {\n    a := i32x4(1)\n    b := a / a\n    ret 1\n}\nfn main() {\n    print(g())\n}\n', "run"),
 # E0900 candidates: find which string form is refused outside print.
 "E0900": ("string_form_as_value", 'fn main() {\n    x := ["a{1}", "b"]\n    print(len(x))\n}\n', "run"),
}

def run_one(code, name, text, cmd):
    path = HERE / f"{code.lower()}_{name}.claw"
    path.write_text(f"// code: {code}\n" + text)
    p = subprocess.run([CLAW, cmd, path.name], cwd=HERE, capture_output=True, text=True, timeout=120)
    out = p.stdout + p.stderr
    codes = CODE.findall(out)
    line = next((l.strip() for l in out.splitlines() if code in l), out.strip().splitlines()[0] if out.strip() else "(no output)")
    return {"file": path.name, "found": code in codes, "codes": sorted(set(codes)), "exit": p.returncode, "line": line[:260]}

if __name__ == "__main__":
    results = {}
    for code, (name, text, cmd) in P.items():
        results[code] = run_one(code, name, text, cmd)
    (HERE / "errors_results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    hit = sum(r["found"] for r in results.values())
    for code, r in results.items():
        mark = "OK  " if r["found"] else "MISS"
        print(f"{mark} {code} {r['line'] if r['found'] else '| got ' + ','.join(r['codes'])}")
    print(f"{hit}/{len(results)} codes triggered")
