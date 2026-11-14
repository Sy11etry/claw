# Language status: claw 1.0

What really works in the installed version. Updated at the end of every release. Version 1.0 is the first public release:
it is the same compiler as 0.13, which is described here.

## Done

| Feature | Example |
|---|---|
| Functions, recursion, values of any type as parameters and results | `fn add(a Point, b Point) Point { ... }` |
| All integers | `int` (= `i64`), `i8 i16 i32 u8 u16 u32 u64` |
| Floating point | `f32`, `f64`, `1.5`, `1e3` |
| `bool` and characters | `true`, `'a'` (which is a `u8`) |
| Explicit conversions | `i32(x)`, `f64(n)`, `u8(c)`, `int(flag)` |
| Variables and `mut` | `x := 5`, `y i32 = 7`, `mut z := 0` |
| Compile-time constants | `const limit = 10 * 2` (also in `[limit]int`) |
| Structs, fields, assignment to fields | `type P { x int, y int }`, `P{1, 2}`, `p.x = 5` |
| Fixed-size arrays | `[4]int`, `[1, 2, 3]`, `[0...]`, `a[i]`, `len(a)`, nested |
| Variants | `type S = Circle{r f64} \| Dot` |
| `match` with a completeness check | variants, integers, characters, `bool`, `_:` |
| A value may be absent | `?int`, `none`, `f()?`, `f() else 0`, `f() else { ret }` |
| A function may return an error | `int ! E`, `! E`, `fail X`, `f()?`, `f() else e { ... }` |
| Every form of `for`, including over an array | `for x in a`, `for x, i in a`, `for i in 0..n`, `for c`, `for` |
| `print` of any value and `{expression}` in a string | `print("p = {p}, {a * 2}")` |
| A trap with `file:line:column`, exit code 101 | signed overflow of any size, division by zero, a shift, an index outside the array, a conversion that does not fit |
| Unsigned types wrap | `u8(250) + 10` gives `4` |
| `?` in `main` | prints `error: ... at file:line:column`, exit code 1 |
| Strictness | everything from 0.1, plus a complete `match`, no ignoring an error, no changing an array during `for`, letter case of names, no types that contain themselves |
| `[]T` | `[1, 2]`, `[]`, `a[i]`, `a[i] = x`, `len`, walking, nested, bounds checks |
| `str` | literals, `"{x}"` as a value, `s[i]`, `s[a..b]`, walking, `== != < <= > >=`, immutable bytes |
| Slices | `a[1..3]`, `a[..2]`, `a[3..]` for `[]T`, `[N]T` and `str`, read-only, bounds-checked |
| Three ways to pass a value | read (the default), `&T` (change, call `f(&x)`), `own x T` (take) |
| Ownership and automatic freeing | memory is freed at the end of the scope on every path: end of block, `ret`, `break`, `continue`, `fail`, `?`; temporaries at the end of the statement |
| Moves | at the first consuming use; no use after a move; the patch is `copy(x)` |
| `copy(x)` | a deep copy of any type except types with their own `drop` |
| Your own `drop(x &T)` | for ordinary and generic types, called by the compiler |
| Protection from dangling references | no changing an array while it is walked, nor the subject of a `match`; no overlapping `&` in one call |
| Standard library written in Claw | `core` is always available (arrays, strings, `to_int`), `use fs`, `use math` |
| `claw fn` | raw pointers, no safety net for them |
| Pointers | `*T`, `&x`, `*p`, `p[i]`, `p.x` through a pointer, `p + 1`, `q - p`, comparisons, `none` as the null pointer |
| Memory | `alloc[T](n)`, `free(p)`, `sizeof[T]()`, `cast[T](x)` |
| `defer` | runs when the block is left on any path: the end, `ret`, `fail`, `?`, `break`, `continue` |
| Calls to C | `extern fn`, any function of libSystem, `*u8` from a string literal, variadic ones (`snprintf`) |
| Generic functions, structs and variants | `fn max[T](a T, b T) T`, `type Pair[A, B] {...}`, `type Tree[T] = Leaf \| Node{v T, kids []Tree[T]}` |
| A function as a value | `f fn(int) int = twice`, fields, arrays, parameters, results; `pick(x)(1)`, `table[i](x)`; effects in the type: `fn(int) int +io` |
| An array as `[]T` | `[N]T` can be passed to a `[]T` parameter that only reads |
| A view of an element | `x := a[i]`, `n := p.name`, `v := s[1..]`: no copy, read-only, `a` cannot change while `x` lives |
| Hash table | `new_map[str, int](hash_str)`, `put`, `get`, `has_key`, `del`, walking with `for e in m.items` |
| The compiler in Claw itself | `selfhost/`, 15 thousand lines; `clawc file [-o out] [--fast]`, `clawc run`, `clawc check`; it builds itself, the second and third stages are identical byte for byte, and identical to `claw build --no-opt` |
| `claw card` | the whole card of the language, up to 2500 tokens |
| `claw sig file`, `claw sig std`, `--pub` | signatures only; the effects of private functions are derived by the compiler |
| `claw ctx file name` | a function and everything needed to change it: types, constants, signatures of what it calls, who calls it |
| `claw audit file` | everything the compiler takes on trust: `claw fn` and what they do, `extern`, checks missing in `claw build` |
| Modules | `use lib.geo`, `pub`, `geo.area(p)`, a function is looked up in the module of the type of its first argument |
| `claw fmt`, `claw fmt --check` | one format with no settings, comments are kept; the file does not change if anything but whitespace would change |
| Our own back end | ARM64, Mach-O, signature, a `__DATA` segment, no `as`, `ld` or `clang` |
| Output | a buffer, one write per `print`, short writes are appended |
| `need` | checked at the call site; the error shows the line of the call and the values of the variables |
| `sure` in a signature | checked at every return; `ret`, `old(x)` |
| `sure` in a body | an assertion: `sure x == 1`; shows the values |
| Exact arithmetic in contracts | `+ - *` of integers inside a comparison are computed in 128 bits and do not overflow |
| Effects `+mem`, `+io` | `pub` must declare them, private ones are derived from the call graph, error `E0160` with a patch |
| Contracts without consequences | no `&`, moves, `?`, `else` blocks or effects |
| `test "name" { }` and `claw test` | every test in its own process, a report of failures only, a 10 s limit |
| The optimizer in `claw build` | SSA, constant folding, dead code removal, inlining, struct fields and `a.len` in registers, removal of repeated work, loop hoisting, register allocation |
| Removing proven checks | array bounds, overflow, division, `need`, `sure` and ordinary branches whose outcome the compiler proved |
| `claw build --fast` | every check removed, contracts not compiled; speed and risk as in C |
| `claw build --stats` | how many checks were removed, how many calls inlined |
| A binary of only what is needed | functions nobody calls do not get into the file |
| `claw test` with no tests at all | every function of the root file without `+io`: boundary, random and coverage-mutated inputs; a failure shrinks to a small input |
| `claw why file` | shows only the lines that led to the failure, and the values of variables; `--test name` for a `test` block |
| A ready `need` in the report | `fix  add need len(xs) != 0 to avg`, when the condition can be expressed through the parameters |
| `claw fix file` | applies the compiler's safe patches while there are any; shows the rest |
| Compile-time evaluation | `const t [4]int = table()`: numbers, `bool`, `str`, arrays and structs of plain data |
| Checked mode (`claw run`, `claw test`) | every contract; going outside a heap block, reading after `free`, double `free`, a foreign `free`, a null pointer, leaks |
| Debugging with `-g` | `claw build -g f.claw` writes `f.dSYM` (macOS, arm64 and x86-64); lldb stops at source lines and sees function names; on Linux the DWARF sections are in the ELF itself |
| A language server | `claw lsp`: errors as you type, hover hints, the outline of the file; unsaved text is laid over the disk |
| Threads without races | `use par`, `t := par.spawn(work, x)`, `par.join(t)`; the input is `own`, the result comes through `join`, no `+io`; in `claw run` the tasks run one after another |
| ARM64 Linux | `--target aarch64-linux-musl` and `aarch64-linux`: an ELF without a linker, dynamically linked with libc; 245 programs (with `-g`) gave the same output in a virtual machine with Alpine (`tools/linux_check.py`) |
| x86-64 | `--target x86_64` for `build`, `run`, `test`; signed Mach-O, run through Rosetta; the same output as arm64 on random programs (`tools/fuzz_x64.py`); threads, SIMD and `-g` too |
| Examples | `examples/*.claw`: seven short programs, which are tests as well (build, `claw run`, x86-64) |
| SIMD | `f64x2`, `f32x4`, `i32x4` in `claw fn`: `+ - *`, `/`, `vload`, `vstore`, `lane`, `fma`; NEON registers on arm64, SSE on x86-64; bounds checks in `claw run` |
| Printing floats in the shortest form | `0.1`, `3`, `1e+16`; `f64` up to 17 digits, `f32` up to 9, on both targets and in the interpreter |
| Checked slices | `a[3..1]`, `a[..99]`: `slice bounds out of range`, exit code 101 |
| Deep recursion | a 64 MB stack for the main thread and for `par` tasks; an overflow prints `stack overflow at ...`, exit code 139 |
| C structs by value | up to 16 bytes in registers, larger ones as a result through memory; by the ABI rules of arm64 and x86-64; `E0149` for what cannot be passed |
| Checking C symbols at build time | the function is in none of the loaded libraries: `E0185` instead of a dyld failure at launch |
| Headers: `sizeof`, renames, `static inline`, an unclosed `#if` (`E0184`) | `claw cdecl` shows the notes |
| Packages in packages | dependencies are read through the whole chain, a conflict of sources is rejected |
| `claw run` and orphan processes | if `claw` is killed, the program and the temporary folder are cleaned up |
| The flight recorder | `CLAW_RECORD=x.clawtrace ./app`: on a trap, a file with the arguments, the environment and the files that were read; `claw why x.clawtrace app.claw` replays the same crash in the interpreter and compares the text of the failure |
| Faster printing of floats | the search for the shortest form starts at 15 digits (`f32`: 6) instead of 1: 2 million numbers in 4.4 s instead of 9.4 s in 0.12 and 3.4 s in 0.11 |
| A 2500-token card | SIMD, C structs, `IoErr`, `for v, i`, `match` on numbers, slices, build targets and the flight recorder are back |
| Error messages of 0.12 | `E0152` shows the expression and a ready `copy(...)`; `E0111` on `T ! E` talks about `?`/`else`; `E0102` for a `const` and a function with one name; `E0144` for extra type arguments; `E0901` points at the function |
| Generic bugs of 0.9 | types with one name from different modules get different instances; late instances are completed |

## Partly

| Feature | What there is | What there is not |
|---|---|---|
| Proving contracts | what follows from number ranges and comparisons on the path to the check is removed | reasoning about the contents of arrays, loop invariants more complex than simple counters |
| Vectorization | hand-written SIMD in `claw fn`: a vector is a local variable, a parameter (among the first eight) or the result of a `claw fn`; not a field, not an element | automatic vectorization of loops |
| Contracts with calls | work in `claw run` and `claw test`; `claw build` says how many were skipped | checks in `claw build` |
| Effects in `claw fn` | visible `alloc`, `print` and calls are checked | `extern` is taken on trust; `claw audit` lists the places |
| Memory checking | the heap | the stack and static data; what a function does through `extern` |
| Formatting | the `claw fmt` command | running automatically at build |
| Strings `{x}` as a value | work | longer than 64 KB end with an error; quotes cannot be written inside `{...}` |
| Recursive types | through `[]T`: `type Tree { kids []Tree }`, and through pointers in `claw fn` | the implicit box for `?T` promised in the plan |
| Moving | only whole local variables | taking a value apart without copying (except `remove`, `swap`) |
| `claw fix` | applies patches: add or remove `mut`, add `&`, `copy(x)`, effects, `_ = e` for an unused local; prints every patch with its position | `_ = f()`, renaming a parameter to `_` and `claw fn` deliberately stay hints |
| Automatic tests | functions of the root file with value parameters | generics, functions with pointer parameters, functions with `+io`, imported modules |
| Overflow in automatic tests | a note with an example input, exit code 0 | a failure on overflow |
| `claw why` | runs the program again in the interpreter, input and output are repeated | replaying exactly the run that failed; C functions outside the built-in list (`snprintf` and others) |
| The patch in `why` | division, bounds, shift, conversion, `alloc`, the `need` of the called function | when the value depends on a `mut` or a loop counter; for overflow |
| Compile-time evaluation | any function without `+io`, every contract on | the size of an array; values with heap and pointers, except `str` |
| Debugging | `-g`: lines and function names on macOS (arm64, x86-64) and in Linux files | types and values of variables in lldb |
| Generics | functions, structs, variants; the body is checked for every set of types used | an unused generic is not checked at all; a generic cannot be passed as a value |
| A function as a value | named functions | closures; functions with `need`, `extern` and generics as values; `print` shows `<fn>` inside a struct |
| Printing | any values | a recursive type as a whole (`E0179`): write your own walking function |
| Hash table | keys: numbers, `bool`, `str` | struct keys: the generic compares keys with `==` |
| A view of an element | only an immutable `x := ...` of a local variable | `mut x := a[i]` needs `copy`, a view cannot be returned |
| C headers and libraries | `use c "zlib.h"`, `link "z"` or `-l z`; `sizeof` in sizes and constants; structs of up to 16 bytes by value; tried on libc, zlib, sqlite3 and our own `.dylib`, on arm64 and x86-64; a symbol missing from the libraries is `E0185` at build time | reading `.a` files; bit fields; function-like macros; structs with `packed`, `aligned`, `#pragma pack` or a vector type: their name gives `E0182` |
| Callbacks into C | `cast[*u8](f)` for `fn` and `claw fn`, up to 6 parameters | closures |
| Threads in our style | `par.map`, `par.build`, `par.spawn/join`: no shared memory, the answer is always that of the loop | channels and shared structures: nobody needed them |
| Scripts and `os` | `claw f.claw args`, `#!`, `os.args/env/exit` | |
| Packages | `claw.pkg`, `claw get`, `claw.lock` (name, source, SHA-256 of the tree), the source is a folder or git; dependencies of packages on packages (through the chain, in the lock by full path); `run`, `build` and `check` check every package that is used against the lock (`E0183`) | a registry, version ranges |
| Calls to C | registers, variadic arguments, narrow results are tested | non-variadic arguments beyond the eighth go on the stack by Apple's rules, but there is no suitable function in libSystem to test them |

## Not yet

| Release | Features |
|---|---|
| after 1.0 | **Linux x86-64** (needs an image to check against; downloading was not agreed), **Windows (PE)**, **WASM** and an in-browser playground, automatic vectorization, floating point in registers on x86-64 (speed only), checking x86-64 on a real Intel chip, running the glibc build, channels, moving threads, SIMD, the C interface and packages into `clawc` |

## Known limits of 1.0

- The flight recorder: it writes only on a failure through `trap`, and only with `CLAW_RECORD`; a crash by signal (stack, wild pointer) is not recorded. The calls of `os.args`, `os.env` and `fs.read` are recorded; input from your own C functions is not. The source is not embedded in the binary: `why` compares the text of the failure, not a hash. A regression `test` is not generated from a recording.
- Printing floats: numbers from `1e16` to `1e17` with 17 digits are written without an exponent (Python writes them with one).

- The C interface: loading libraries on Linux (`DT_NEEDED`) is built but has not been run on Linux. C structs by value (up to 16 bytes; larger ones only as a result) and bit fields: bit fields are not translated. `claw run` executes a call with a struct by value, but `claw why` cannot replay such a call (the interpreter cannot do them). The header is read by a mini preprocessor, not a real one: a complex `#if` with arithmetic may be read wrongly. The layouts of structs by value are checked on macOS (arm64 and x86-64 through Rosetta) and have not been run on Linux.
- `par.map` and `par.build` need `f` to be a named `fn` without `+io`. Results on the heap (`str`, `[]T`) are tested, but not on millions of elements.
- `claw run` and `claw test` run `par` one task after another, as before.
- Output from several threads is built under a short lock of the buffer (before 0.11 lines from different threads could be mixed); the lock is turned on only in programs that use `par`.
- Packages: there is no registry; `claw get` for git needs `git` installed (the only external tool, and only when downloading). A `claw.lock` written before it had a source (a line `name hash`) is not accepted by `run` and `build`: one `claw get` is needed, which checks the package again.
- C structs: `packed`, `aligned`, `#pragma pack` and vector typedefs are not translated, their names give `E0182`. The layout in Claw does not change: the fields of a C struct stay padded in Claw as if unpacked.
- No Windows, WASM or Linux x86-64: see "Not yet".

- In `claw build`, an access through a null or dangling pointer crashes as in C (exit code 139) or corrupts memory. `claw run` catches this for the heap and for the zero address.
- `/`, `%` and shifts in contracts are computed as usual and can trap. A multiplication of values wider than 64 bits in a contract ends with a message instead of giving a wrong answer.
- While a contract with calls runs, contracts with calls inside it are switched off, otherwise `sure ret == sum(a)` would never finish.
- `old(x)` remembers only numbers and `bool`; for an array write `old(len(a))`.
- Dynamic libraries are linked (`link`, `-l`); static `.a` files and your own `.o` files are not yet.
- Deep recursion: the stack of the main thread and of `par` tasks is 64 MB (hundreds of thousands of calls); beyond that comes the message `stack overflow` and exit code 139. An access through a bad pointer prints `bad pointer` and also gives 139 (138 under Rosetta). On Linux there is no message, only the signal.
- A function: at most about 1 MB of machine code and 7 MB of local data, otherwise `E0901`. Put big data into a `[]T`.
- `claw build` compiles into the file only what is called; but types and bodies are still fully checked.
- The literal `[x...]` is filled with a plain loop.
- An error inside `{...}` in a string points at the start of the whole string.
- `claw run` and `claw test` do not optimize: checks and a fast build matter more there.
- The interpreter is 30-40 times slower than `claw build` (measured on `sieve` and `mandel`). `claw why` and `__interp` stop after 3 billion steps, about 6 seconds: a program that runs natively for longer than about 0.2 s may not reach its end (`primes` does not). The history loop keeps the last 8 million events; earlier causes are marked in the report.
- `claw test` budgets steps: 4000 inputs and 20 million steps per function, 300 million per file. Functions over the budget count as skipped.
- `clawc` (the compiler in Claw) does not optimize, does not evaluate a `const` with a call (`E0174`), and cannot do `test`, `why`, `fix`, `fmt`, `sig`, `ctx`, `audit`: `claw` in Rust still does those.
- The token figures in `benchmarks/TOKENS.md` are an estimate by two counting rules; there is no real tokenizer here.
- An input that made a function loop forever or recurse too deeply is thrown away, not counted as an error.
- Bit test and multiplication by a constant: `x & 2^k` with a branch is `tbz/tbnz` (in functions of up to 100 instructions, otherwise the usual `cmp`/`b.ne` pair); `x * 2^n` and `x * 3` without `smulh`.
- Threads: `spawn` takes only an `own` input and returns one value; there is no shared memory between tasks. A crash inside a thread in `claw build` ends the whole program, and `claw run` runs the tasks one after another, so a race, if one were possible, would not be reproduced.
- Linux: `claw run` and `claw test` do not work for a foreign target (`--target` with Linux only builds a file). The glibc build is checked at the level of the format but has not been run on glibc (the virtual machine has musl).
- x86-64: the back end keeps integers in callee-saved registers, and floating-point and all other values in stack slots; it is checked through Rosetta, not on a real Intel chip (`0.0 / 0.0` on Intel may print as `-nan`). Vectors (SSE) work; `i32x4 * i32x4` needs SSE4.1, `fma` needs FMA3 (Rosetta has it, and so does every Intel chip since 2013).
- SIMD: a vector cannot be put into a struct or an array, passed to an `extern fn` or a generic. `i32x4` wraps without a check. The index in `lane` is only a literal.
- `clawc` (the compiler in Claw) does not know `par`, SIMD or Linux: these parts live outside `tests/run` so that the bytes of `--no-opt` stay equal.
