# Porting the compiler to Claw

The goal of iteration 9: `selfhost/main.claw` compiles itself. The Rust compiler (`compiler/src`) stays the reference and the only place where the optimizer, `why`, the fuzzer, `fmt` and `fix` live.

## What is ported and what is not

Ported word for word, module by module: `lexer` (done), `diag` (done), `ast`, `parser`, `modules`, `types`, `sema`, `effects`, `cir`, `lower`, the naive branch of `regalloc`/`codegen`, `arm64`, `macho`, `sha256` (done).

Not ported: `opt`, `ssa`, `ranges`, register allocation, `interp`, `fuzz`, `why`, `ctfe`, `fmt`, the patches of `fix`. The compiler in Claw rejects computed `const` (`const x T = f()`) with `E0174`.

## The measure: byte for byte

`claw build --no-opt f.claw` in Rust and `selfhost f.claw` must produce **the same bytes**. Intermediate measures: `claw __tokens` (lexer), `claw __ast` (parser), `claw --emit-cir --no-opt` (lowering), the error codes in the headers of `tests/err` (sema). Behaviour is changed only where Rust cannot be repeated, and that is written down here.

## How to write

- The names of functions, fields and variants are the same as in Rust where the language allows.
- A variant lives in the namespace of its module: two variants with one name in one file are not allowed, add a prefix.
- There are no recursive types, so **the tree lives in an arena**: `ast.Ast { exprs []Expr, stmts []Stmt, blocks []Block, tys []TypeExpr }`; a reference is an `int` index, "no value" is `-1`, a list is a `[]int`. Sema writes `ty`, `coerce`, `res` straight into the arena element.
- There are no mutable global variables: state is passed as a `&` parameter.
- Types from another module are written with the module name: `diag.Span`, `ast.Expr`. Generics and functions from `core` are visible everywhere.
- Do not write the effects of `pub` functions by hand: `claw fix file` adds them.
- Integers: `int` is i64. There is no `i128`. Literals: `u64` and a `big` flag on the token. `int` is enough for `eval_const`; if a value does not fit, repeat the behaviour with flags.

## Traps of the language

- `{` in a string is always an expression: a literal brace is `{{`.
- `if` is a statement, not an expression. There is no `?:` and there are no bare blocks `{ }`.
- `x := a[i]`, when the type owns memory, is a view: `a` cannot be changed until the end of the block. For `mut` you need `copy`. A parameter `x T` only reads; it can be moved if it is declared `own`.
- An array `[N]T` is copied on assignment. To avoid copying a big one, pass it as a parameter.
- There are no methods, closures or overloading. `match` on numbers exists, on strings it does not; compare strings with `==`. Long chains of `if name == "..."` are fine.
- Detailed reference: `docs/CARD.md`; library signatures: `claw sig std`.
- `selfhost/rt.claw`: reading a file, writing, arguments, `text(own []u8) str` without a copy.
- `match a.exprs[i].kind` is a view too: while it lives, `&a` cannot be passed. To walk and write into the arena, use `k := copy(a.exprs[i].kind)` and then `match k`.
- A `?str` value from a parameter or a field can be read with `else` only after `copy`, otherwise it is a move.
- Inside `{...}` in a string you cannot write `"`: the string would end early. Move it into a variable.
- A long `type X = A | B` is wrapped only after `|`, not before it. A multi-line `match` arm goes in `{ }`.
- A variant without fields from another module is written `ast.Break`, as a value; `E0152` on it was a compiler bug (fixed).

## Deliberate differences from Rust

- `modules.find_std` looks for `std` next to the compiler and then in every folder above it: that is how a binary from `selfhost/` finds the repository's `std`. Rust checks only the compiler's folder and the third one above it.
- In the error message for a literal wider than 64 bits (`found number ...`) the text is the source text without `_`, not the i128 value; for hexadecimal numbers and for numbers above i128::MAX the text differs. Inside `{...}` in a string, where there is no source, it writes `big`.

## Building

`compiler/target/release/claw` builds `selfhost/*.claw`. `use x` looks for `x.claw` next to the file.

## Sema and effects (`types`, `sema`, `effects`, `wide`, `dump_sema`)

The measure: `claw __sema [--check] [--tests] file` (Rust) and `dump_sema` give **the same text** on the whole corpus (`tests/**`, `std`, `selfhost`, `benchmarks`) in all four modes. Compare like this: put `dump_sema` next to `std`, for example with a `std` symlink in the folder with the binary; the `std` paths in the output then differ only by a prefix, which must be replaced before comparing.

How it is built:
- `sema` reads the tree as a **read-only parameter** (`l modules.Loaded`) and does not write into the arena while it works: otherwise `match l.a.exprs[e].kind` would block `&` calls and every walk would need a `copy`. The answers live in `Global.ty/coerce/res` (one element per `Expr`), and `check` writes them into the arena at the very end. A generic instance is `clone_func`: new ids, the tables grow through `new_tables`.
- `Global` holds both the state of the whole module and the state of the body being checked (`Ctx` in Rust): there is no recursion in `check_body`, the state is recreated on entry. `l` and `g` are threaded everywhere.
- `FnSig` and `Func` are not copied during lookup: `sigs` holds an index into `sig_list`.
- Rust's `HashMap` iterates in random order. Where that affected the result (`instance_of`), Rust now sorts by type id, and the port keeps a list in creation order (`insts`): it is the same order. `user_drop`, `known` and `sigs` are sorted in the dump.
- `Types` interns by itself (open addressing on `kind_hash`, comparison `kind_eq`), with no `Map` and no string keys.
- `types.name` writes into one buffer: a type `[2][2]...` can be thousands of levels deep, and `"[{n}]{inner}"` at every level would be quadratic (`tests/err/generic_infinite_fn` failed on time).
- Numbers in `const`: `wide.claw` repeats i128 (`checked_*`, shifts, `as f64`). A literal larger than `u64` is marked `big`: the digits were already lost in the lexer.

Traps met here:
- `own` is a keyword: the field is called `owned`.
- A local variable cannot be named like a function of this module (`E0102`), even if that function is called through another module. Hence `er` instead of `err`, `vname` instead of `var`: the functions `err` and `var` exist in `sema`.
- `swap(&d[i], &d[j])` on one array is forbidden (`E0158`): such a `swap` is done through `remove`/`insert`.
- A `?BinOp` field can be read from a view: `o := op else ast.Add`; a `?str` only through `copy` (`opt_text`).
- `if` gives no value, `match` is not an expression: everywhere `mut x := ...`, then an assignment in the branches.
- Inside `{...}` of a string you cannot nest `"`: `{join(xs, ", ")}` is moved into a local variable.
- A recursive descent has no big stack: `rt.ensure_stack()` raises the stack limit (`setrlimit` plus a repeated `exec`); Rust starts a 256 MB thread for this. Left nesting `1 + 1 + ...` of 6000 links (`tests/run/large_frame`) needs tens of megabytes.

Deliberate differences from Rust (only about literals larger than `u64`, which are not in the tests):
- `index ... is outside`: for such an index `big` is written instead of the value.
- Such a literal in `const x f64`: Rust computes the value, the port answers `E0115` because the digits are not there.
- `const x = 99999999999999999999 / 1000`: Rust computes it, the port sees `big` and answers `E0115` on the typed check.
- `E0151` from a `defer` with two or more variables: Rust takes the first in the random order of a `HashMap`, the port the first in order of appearance.

Computed `const`s are not evaluated here: `resolve_computed` and the checks are in place, and `E0174` comes from the next stage.

## CIR and lowering (`cir`, `lower`, `dump_cir`)

The measure: `claw __cir [--check] [--tests] [--fast] [--regs] file` (Rust, without `opt`) and `dump_cir` with the same flags give **the same bytes**. Without flags this is `claw build --no-opt`, `--fast` is `claw build --fast --no-opt`, `--check` is `claw run`, `--check --tests` is `claw test --blocks`, `--regs` compares `each`/`def_of`/`is_call`. Put the binary in `compiler/target/release/`, or `std` will not be found. There are only three exceptions: `tests/info/sig_all`, `tests/run/const_computed`, `tests/run/const_computed_uses` (computed `const`s, the port answers `E0174`). A declared but unused computed `const` merely produces a function `const x`, as in Rust.

How it is built:
- `Lower` is Rust's `FnLower` and `Env` together; `new_lower` resets everything that lives for one function, and at the end the blocks and objects leave through `swap` in `into_func`.
- `l` and `c` are passed everywhere as read-only parameters, so `match l.a.exprs[e].kind` does not get in the way of calls with `&me`. `me` itself cannot be read like that: take an `int` first, not a view.
- There are no closures: `counted_loop` became `counted_open`/`counted_close`, `on_tag` became `tag_open`/`tag_close`, `for_each` became `for_each_drop` (the body was always a drop).
- `Option<VReg>` is an `int` with the value -1; `Clean`, `Place`, `Piece` are plain variants. A function is named by `FuncRef{unit, index}`: an `ast.Func` cannot be returned as a view and cannot be put into a struct, hence the pair and two branches of the call.
- `olds` and `under_old` are keyed by the number of the expression: an instance of a generic (`clone_func`) has its own numbers, as do the addresses in Rust.
- Not ported (optimizer and interpreter only): `Phi`, `Pi`, `DataAddr`, `ConstRef`, `Line`, `Note`, `each_mut`, `removable`, `Auto`.
- The `Debug` text of a string: ASCII and C1 control characters as in Rust; other non-ASCII is written as it is. Rust also escapes combining and non-printable characters by its own tables. This does not affect CIR, only the text of the dump; there is no non-ASCII in the corpus.

Traps met here:
- `me.sret = vreg(&me)` is forbidden (`E0158`): a temporary variable is needed.
- **A compiler bug, not fixed:** the name of a generic instance is built from the names of types, and the name of a type does not contain the module. Two types with one name from different modules give one `push[P]`, and the second call fails with `E0104 expected &[]P, found &[]P`. That is why in `cir` the types are called `CirFunc` and `CirBlock`, not `Func` and `Block`. The same in Rust and in `sema.claw`.
- `sure false` in the place of `unreachable!`: it stays as a check in `claw build` too.

## The back end (`arm64`, `codegen`, `macho`, `main`, `dump_golden`)

The measure: `claw build --no-opt f -o x` and `main f -o x` give **the same bytes** (`cmp`), including LC_UUID (the first 16 bytes of sha256 of the code and rodata) and the signature (the ident is the name of the output file, so files with one name in different folders must be compared). The same for `--fast` and `--alloc-stats`. The corpus: `tests/{run,own,check,unit,fast,stats,why,auto,fix,info,fmt}/**`, `benchmarks/`, `benchmarks/tokens/`, the `selfhost` programs with `main`: 440 files, in every mode 392 identical, 45 identical errors (the same stderr and exit code), 3 exceptions (computed `const`s: `tests/run/const_computed`, `tests/run/const_computed_uses`, `tests/info/sig_all`). `main check` on `tests/err` agrees with `claw check`, except five `const_computed_*` (there Rust computes and finds its own error, the port answers `E0174`). The port does not know C headers (`use c`) or `claw.pkg` packages, so it rejects `tests/err/packed_c_struct.claw` (`use c`) at the syntax level (`E0201`), and cannot find the module in `tests/err/pkg_*/main.claw` (a `use` from a package, `E0146`): this is not a difference in parsing, it is the limit of the port. The encoder: `dump_golden` prints the same table as `claw __encoder-golden` (126 lines), and `llvm-mc` agrees with it.

How it is built:
- `codegen` repeats only the `optimized == false` path: every CIR register is a slot `spill_base + v * 8`, there are no callee-saved registers. `Place`, `allocate`, fusing a comparison with a branch, `indexed_access` and the `uses` count are not ported: without the optimizer they never fire. There are no dead functions without the optimizer, but the `dead` check is kept.
- `mov_imm` takes a `u64`: the bit pattern, Rust's `i64` is not needed. The bounds of `float_range_check` are assembled straight from bits (`2^k` is `(1023 + k) << 52`), without `powi`.
- `Option` is `-1` everywhere; `m.check` is read through `else cir.CheckRt{-1, -1, -1}`.
- Variants of different enums in one module: `Target` became `TgUser`, `TgTrap`, ...; `Slot` became `SlotX/SlotD/SlotStack`. In a `match` the variants are written without the module (`Eq:`), on creation with the module (`arm64.Eq`).
- `and`, `ret`, `str` are taken words: the encoders are `and_x`, `ret_x`, `str_x`; `B`/`BL` became `b_op`/`bl_op`. The registers in `codegen` are declared as constants of their own (`x9 u32 = 9`), so as not to write `arm64.x9` hundreds of times.
- Errors are printed like Rust's `render_text`: the first 20 and a line `N more errors not shown`. `diag.render` does not do this (`dump_sema` is compared without the limit), so the cut is in `main.report`.
- `main run`: `posix_spawn` + `waitpid` (in `rt`), the exit code or `128 + signal`, the folder `$TMPDIR/claw-run-<pid>` is removed after the run. The temporary file while writing is `path.tmp` (`rt.write_exec`), in Rust `path.tmp<pid>`; the bytes are not affected.

Traps met here:
- `const x = 9` has type `int` and is not substituted into a `u32`: you need `const x u32 = 9`. A numeric constant with a type and a literal does not count as computed.
- There are no multi-line strings; a `const` string with line breaks cannot be assembled either, so the help text became a function.
- `else { ... }` must leave, even if there is `rt.exit` inside: there is no "does not return" type. Hence `compile_to_cir ... ! Halted` and `else { ret }` at the caller.
- `--no-opt --fast` in Rust keeps the overflow and bounds checks: the optimizer removes them. So `tests/fast/no_bounds` and `tests/fast/wraps` fail with 101 in this mode in both Rust and the port (the bytes are the same).

## What 0.12 added to `clawc`

The `--no-opt` bytes still match after 0.12, because the same was done in `selfhost/codegen.claw`: shortest printing of floating point (the import `_strtod`), the `Slice` trap, the stack handler (`_sigaltstack`, `_sigaction`, imports always, even for Linux), a 64 MB `LC_MAIN` stack size. The texts of the errors `E0102`, `E0105` (local variable), `E0111`, `E0118`, `E0121`, `E0127`, `E0144`, `E0152`, `E0134` were ported to `sema.claw`. Not ported: the `E0149` checks for C structs by value, `E0184`/`E0185`, packages in packages (`clawc` has no `use c` and no packages).

## What 0.13 added to `clawc`

The same code generator changes, so that the `--no-opt` bytes match: `rt_trap` writes the text of the failure into words 9 and 10 of the state (for the flight recorder), and the printing of floating point starts the search for the shortest form at 15 digits (`f32`: 6), and at one digit for numbers below `1e-37`. `std/trace.claw` is not needed in `clawc`: it is needed only by programs that use `os` or `fs`.
