# Claw 1.0 specification

A short reference for people is `docs/CARD.md`. This file has the exact rules.

## Lexical rules

A comment is `//` to the end of the line. Numbers: decimal, `0x`, `0b`, with `_` between digits; floating-point `1.5`, `1e3`, `2.5e-3`. The character `'a'` has type `u8`. Strings: `\n \t \r \0 \\ \"`, and `{expression}` inserts a value (`{{` and `}}` give a brace).

The end of a line ends a statement, except when the line ends with a binary operator, an assignment operator, a comma, an opening bracket, `and`, `or` or `not`, or when the line is inside `()` or `[]`. Line breaks are allowed before the `{` of a function body. `else` is written on the same line as the closing `}`.

## Precedence, weakest to strongest

`or`, `and`, `not`, comparisons (`== != < <= > >=`), `+ - | ^`, `* / % << >> &`, unary `- ~`.

## Semantics

- Operands and arguments are evaluated left to right; `and` and `or` short-circuit.
- The bounds of `for i in a..b` are evaluated once, the range is half-open; when `a >= b` the body does not run.
- `int` is `i64`. Signed types end the program on overflow of `+ - *`, of unary `-` and of `MIN / -1`.
- Unsigned types wrap modulo 2^N.
- A literal takes its type from the context; an integer literal also fits a floating-point type. There are no other implicit conversions.
- `T(x)`: to a signed type, ends the program if the value does not fit; to an unsigned type, drops the high bits. From floating point to an integer it drops the fraction; NaN and out-of-range values end the program.
- Structs, variants and arrays are values. Assignment copies; parameters are read-only.
- Layout: structs as in C, fields in declaration order. Variants, `?T` and `T ! E` have an 8-byte tag followed by the data.
- An index outside an array ends the program. A constant index outside an array is a compile error.
- An array cannot be changed inside a `for` that walks over it.
- `?` passes an absence or an error up; in `main` it prints `error: ...` and ends the program with exit code 1.
- An `else` block must leave (`ret`, `fail`, `break`, `continue`). In the header of `if/for/match`, `else` and struct literals are written in parentheses.
- Comparisons of floating-point numbers with NaN are false, except `!=`.
- `/` and `%` by 0 end the program. `MIN % -1` is 0.
- `<<` and `>>`: a shift by an amount outside `0..N-1` (N is the size of the type in bits) ends the program. For signed types, a `<<` that loses significant bits also ends the program. `>>` is arithmetic for signed types and logical for unsigned.
- A slice outside the array (`a[i..j]` with `i > j`, or `j` larger than the length) ends the program with the reason `slice bounds out of range`.
- Floating-point numbers print in the shortest form that reads back as the same number (up to 17 digits for `f64`, 9 for `f32`): `0.1`, not `0.10000000000000001`. An exponent is used only when `|x| >= 1e16` or `0 < |x| < 1e-4` (except numbers from `1e16` to `1e17`, which need all 17 digits: `%g` writes those without an exponent); `NaN`, `inf`, `-inf` as they are.
- A trap: the message `trap: <reason> at <file>:<line>:<column>` on stderr, exit code 101.
- A stack overflow (endless recursion) prints `stack overflow` on stderr and ends the program with a signal (exit code 139); a wild pointer in `claw fn` prints `bad pointer`. The main thread's stack is 64 MB, and so is the stack of every `par` task.

## Ownership

A value has one owner. The types that own memory are `[]T`, `str`, types with their own `drop`, and everything that contains them.

- A parameter is read-only by default and lives for the duration of the call. `&T` allows changing the value; the call is written `f(&x)`. `own x T` takes the value.
- These consume a value: a declaration, an assignment, `ret`, `fail`, an `own` argument, a struct field, an array element, `?`, `else`.
- Only a whole local variable or an `own` parameter can be moved. Fields, elements, slices, read-only parameters and the bindings of `for` and `match` cannot.
- After a move the variable cannot be read until it is given a new value. A move inside a loop is forbidden unless the value is restored before the next pass.
- At the end of a scope the owning values are freed in the reverse order of declaration. Temporary values live to the end of the statement. `defer` and freeing share one queue.
- On assignment the old value is freed after the new one is computed. When writing through a raw pointer the old value is not freed.
- Using `x` as the subject of a `for` or a `match` forbids changing or moving it inside the body.
- In one call the same thing cannot be passed with `&` twice, nor can a compound value be read while another argument changes it through `&`. In `claw fn` this check does not apply.
- `[]T` and `str` in memory: a pointer, a length, a capacity. Capacity 0 means the bytes are not owned: static data or a slice.

## Claw mode

- Pointers, `&`, `*`, `alloc`, `free`, `cast`, `defer` and `extern` calls exist only inside `claw fn`.
- `&` needs a mutable variable. Writing through a pointer does not need `mut` on the pointer itself.
- `p + n` and `p - n` move by whole elements, `q - p` gives a number of elements. `p[i]` does not check bounds.
- Arithmetic and array indices are checked in `claw fn` too.
- `alloc[T](n)` ends the program on a bad size or when memory runs out.
- `defer` runs when its block is left, in the reverse order of declaration. The value of `ret` is computed before `defer`. Inside `defer` there can be no `ret`, `fail` or `?`. On a trap `defer` does not run; on `?` in `main` it does.

## Function as a value

The type is `fn(T, &U, own V) R ! E +mem +io`. Only a named function becomes a value, and only where the type is known: `f fn(int) int = g`, an argument, a field, an element, `ret`. A call is `f(x)`, `s.f(x)`, `a[i](x)`, `g()(x)`. The effects of a call are the effects in the type. Values cannot be compared.

## Views

`x := e`, where `e` is a field, an element or a slice of a local variable and the type owns memory, neither copies nor moves: `x` is a read-only view, and the root cannot be changed or moved until the end of the block where `x` is declared. `[N]T` is passed to a `[]T` parameter as a view.

## Generics

Variants can be generic too: `type O[T] = S{v T} | N`. The type comes from the context or is written out: `S[int]{1}`. A variant without fields needs a known type.

Type parameters are written with a capital letter. A separate copy is made for every set of types that is used, and that copy is what gets checked. Parameters are inferred from the values of the arguments; a literal counts only if nothing else determines the parameter.

## Modules

One file is one module. `use a.b` looks for `a/b.claw` next to the file, then in `std`, then in the packages from `claw.pkg` (see "Packages"). The name of the module in code is its last segment. Only `pub` is visible from outside. A function without a prefix is looked up in the current module, then among the `pub` functions of the module that declared the type of the first argument. An unused `use` is an error.

## Calling C

`extern fn` takes and returns numbers, `bool`, pointers, and structs made of such fields (and arrays and structs of them); `str`, `[]T`, `own`, variants and `?T` cannot be given to C (`E0149`). A string literal where `*u8` is expected is passed as the address of its bytes with a terminating zero. `...` at the end allows variadic arguments; `f32` becomes `f64` there.

### C headers and libraries

`use c "zlib.h"` reads the header and turns its declarations into `extern fn`, `type` and `const` (constant names in lower case: `Z_OK` becomes `z_ok`; the C `int` is `i32`, `long` is `int`, a pointer to a struct is a pointer to that type). A mini preprocessor understands `#define`, `#if/#ifdef` and `#include`, and skips what it does not understand. The header is looked up next to the file, in the folders given with `-I dir` and in the system folders (the macOS SDK, `/usr/include`); if there is none, `E0181`. `claw cdecl zlib.h` shows the result.

A library is linked with `link "z"` in the source or `-l z` on the command line (`-L dir` adds a search folder). In Mach-O this is `LC_LOAD_DYLIB` (system libraries are taken by name, `/usr/lib/libNAME.dylib`: they have no file on disk, they live in the dyld cache); in ELF it is `DT_NEEDED`.

A callback: `cast[*u8](f)` hands C the function `f`, an ordinary `fn` or a `claw fn` (up to 6 parameters, numbers, `bool` and pointers, no `own`, not generic; otherwise `E0180`). A struct of up to 16 bytes is passed to C and returned from C by value according to the ABI of the target (arm64: up to four identical floating-point fields go in `v` registers, the rest in one or two `x` registers; x86-64: every 8 bytes in a register `rdi..` or `xmm`). A struct larger than 16 bytes can only be returned from C (the caller supplies the memory); as an argument by value it is not allowed (`E0149`: pass a pointer). If after laying structs out in registers there are more arguments than C has registers, that is `E0149` as well. `claw why` cannot replay such a call (the interpreter does not run them).

The header is read with `sizeof` in array sizes and in `#define`. C names that Claw had to rename (a Claw keyword, letter case), `static inline` functions and macros that Claw does not have are shown by `cdecl` as notes, and on an `E0101` error it names the reason. An unclosed `#if` is `E0184`. Before the file is written, the compiler checks that every C function that is called is in one of the loaded libraries; if not, that is `E0185` (a `static inline` in a header is not a symbol of the library).

A struct that C lays out differently from Claw (`packed`, `aligned`, `#pragma pack`, a vector type, or a struct that contains such a struct by value) is not translated. If the program names it, that is `E0182`; a pointer to it in a prototype becomes `*u8`, and its fields are read through `*u8` at offsets.

## Packages

A file `claw.pkg` next to the main file: lines `name source`, where the source is a folder or a git address (`url#tag`). `claw get` downloads git packages into `.claw/pkg`, computes the SHA-256 of the whole tree and writes `claw.lock`: one line `name source hash` per package. After that, `use name` or `use name.module`. Before using a package, `claw run`, `claw build` and `claw check` check it against `claw.pkg` and the lock: if the files changed, the source changed, or the package is not in the lock, that is `E0183` with the hint `claw get --update` (or `claw get`). `claw get` without `--update` does not accept a new source from `claw.pkg`. A package can itself depend on others: its `claw.pkg` is read as well (dependencies go into the lock under their full path), and two different sources for one name are rejected. An empty package (no `.claw` files) is an error. `claw fix` does not touch the copies in `.claw/pkg`.

## The flight recorder: `CLAW_RECORD` and `claw why x.clawtrace`

A program started with the environment variable `CLAW_RECORD=path` remembers what came in from outside: its arguments (`os.args()` and `argv[0]`), the environment variables it asked for (`os.env`), and the files it read whole (`fs.read`, failures included, with their `errno`). If the program ends in a trap (exit code 101), it writes all of this to the file `path` on the way out. Without the variable (and if the program ended well) nothing is recorded and nothing is written. The recorded inputs take at most 4 MB; beyond that the file is marked `truncated`.

`claw why x.clawtrace [file.claw]` runs the `main` of the source in the interpreter, giving it the recorded arguments, environment and files instead of the real ones, and prints the usual `why` report. One line comes before it: `replayed the recorded run: the same trap happens here` if the failure matches the recorded text (file, line, column), otherwise `not the same crash` or `not reproduced`: most likely this is not the right source. Without a named source, it is looked up from `argv[0]` plus `.claw`. Crashes by signal (stack overflow, wild pointer) and input from C functions are not recorded. The format of the file is described in `compiler/src/trace.rs`.

## Scripts

`claw file.claw arg1 arg2` builds and runs the file at once; a first line `#!/usr/bin/env claw` is allowed. Arguments and environment: `use os`, `os.args()`, `os.env(name)`, `os.exit(n)`.

## Vectors (SIMD)

Only in `claw fn` (`E0905`). The types `f64x2`, `f32x4`, `i32x4` are 128 bits, one register. A vector can be a local variable, a temporary, a parameter (among the first eight) or the result of a `claw fn`; it cannot be a field, an element, `?T`, the target of a pointer or part of a function type (`E0906`). A vector is passed and returned in registers `v0-v7`, `v0` (on x86-64: in two slots, the result in `rax`/`rdx`); there are none in an ordinary `fn` or in an `extern fn` (`E0905`). The constructor `f64x2(x)` copies a number into all lanes.

`+ - *` and unary `-` work lane by lane, `/` only on floating-point vectors (`E0909`). The operands must be the same vector type; a vector and a number cannot be mixed (`E0104`). The integer lanes of `i32x4` wrap. `print` does not print a vector (`E0909`).

| Built-in | Meaning |
|---|---|
| `vload(p)` | read 16 bytes from a pointer to `f64`, `f32` or `i32` (unaligned) |
| `vstore(p, v)` | write a vector |
| `lane(v, i)` | lane `i`, a number literal (`E0908`) |
| `fma(a, b, c)` | `a * b + c` for `f64x2` and `f32x4` |

In checked mode `vload` and `vstore` check all 16 bytes. Vectors work on both macOS targets (NEON on arm64, SSE on x86-64; `i32x4 * i32x4` on x86-64 needs SSE4.1, `fma` needs FMA3). For Linux, vectors are built as on arm64.

## Threads

Threads in Claw have no shared memory: there are no mutexes, atomics or shared variables, so there are no data races by construction rather than by discipline. There are three forms, all in `use par`.

`par.cores()` is the number of cores. `par.map(xs, f)`: `f` is called on every element (read-only), and the results are in the same order. `par.build(n, f)`: `f(i)` for `i` from 0 to n, the results in order. The work is divided into chunks by cores and every chunk writes into its own separate part of the result, so the answer is always the same as that of a plain loop.

`t := par.spawn(work, x)` and `par.join(t)` are for one separate task. `work` is a named `fn(own T) R +mem`; no `+io`, no closures and no global variables. The input goes into the task through `own` and the result comes back through `join`, so there are no races. `claw run` and `claw test` run the tasks one after another, `claw build` starts them with `pthread_create`.

## Build targets

`--target arm64` (the default, macOS), `x86_64` (macOS, run through Rosetta), `aarch64-linux` and `aarch64-linux-musl` (ELF, dynamically linked with libc; `build` only). `-g` writes DWARF: into `<out>.dSYM` for lldb on macOS (both architectures), and on Linux the `.debug_*` sections are embedded in the ELF file itself.

## Error codes

| Code | Meaning |
|---|---|
| E0001-E0005 | lexical: a character, a string, a number, an escape, a character literal |
| E0101 | unknown name |
| E0102 | the name is already taken (by a `const` or a function too) |
| E0103 | changing something immutable |
| E0104 | type mismatch |
| E0105 | the value is never read |
| E0106 | wrong number of arguments |
| E0107 | a function can end without `ret` |
| E0108 | `break`/`continue` outside a loop |
| E0109 | a problem with `main` |
| E0110 | a function defined twice |
| E0111 | the result of a call is not used |
| E0112 | unknown type |
| E0113 | unreachable code |
| E0114 | the expression does nothing |
| E0115 | the number does not fit in `int` |
| E0116 | `mut` without a change |
| E0117 | an `else` block does not leave |
| E0118 | the pattern does not fit the type |
| E0119 | wrong number of names in a pattern |
| E0120 | `match` does not cover everything |
| E0121 | the case is already handled |
| E0122 | an array is changed while it is walked |
| E0123 | the type cannot be determined from the context |
| E0124 | `?` or `fail` in a function that cannot take it |
| E0125 | an error is ignored |
| E0126 | wrong array size |
| E0127 | a type contains itself |
| E0128 | wrong letter case of a name |
| E0129 | no such field, or it cannot be indexed |
| E0130 | a constant index outside the array |
| E0131 | cannot assign here |
| E0132 | unknown effect |
| E0133 | an error inside `{...}` in a string |
| E0134 | `??T` is forbidden |
| E0135 | a range with two variables |
| E0136 | `x...` must be the only element |
| E0137 | a field declared twice |
| E0138 | `?T` has no error value |
| E0139 | a `const` cannot be computed at compile time; with a function call it needs an explicit type |
| E0140 | a pointer or a memory operation outside `claw fn` |
| E0141 | `extern` called from outside `claw fn` |
| E0142 | `defer` outside `claw fn` |
| E0143 | `defer` tries to leave the function |
| E0144 | the generic parameters cannot be determined, or their number is wrong |
| E0145 | a generic expands forever |
| E0146 | module not found, not imported, or part of an import cycle |
| E0147 | the name is not declared `pub` |
| E0148 | an import is not used |
| E0149 | the type cannot be passed to C |
| E0150 | Claw has no methods |
| E0151 | the value has already been moved |
| E0152 | this cannot be moved |
| E0153 | `[x...]` for an owning type |
| E0154 | a type with its own `drop` cannot be copied |
| E0156 | a wrong `drop`, or a direct call of it |
| E0157 | wrong use of `&` |
| E0158 | overlapping `&` |
| E0160 | a function does something that its `+mem` / `+io` does not say |
| E0161 | a contract or `sure` with effects |
| E0162 | `ret` in a contract where there is no result |
| E0163 | wrong `old(...)` |
| E0164 | a contract changes something: `&`, a move, `?`, an `else` block |
| E0165 | `sure` reads an `own` parameter without `old` |
| E0166 | a contract on `main` or `drop` |
| E0167 | two tests with the same name |
| E0168 | `need` in the body of a function |
| E0171 | a `const` with a call uses `+io` |
| E0173 | a computed `const` cannot have this type: it needs plain data or `str` |
| E0174 | a `const` could not be computed: a trap, a broken contract, too long, a C function is needed |
| E0175 | computed `const`s depend on each other in a circle |
| E0201-E0205 | syntax |
| E0176 | a generic or an `extern fn` cannot be used as a value |
| E0177 | a function with `need` cannot be used as a value |
| E0178 | the function has effects that the type of the value does not have |
| E0179 | a value that contains itself cannot be printed whole |
| E0180 | the function cannot be given to C as a callback: a generic, an `extern`, a `need`, more than 6 parameters, not a number or a pointer |
| E0181 | the C header was not found: add its folder with `-I` |
| E0182 | the C struct is laid out differently than in Claw (`packed`, `aligned`, `#pragma pack`, a vector type): its name cannot be used, read the fields through `*u8` |
| E0183 | the package does not match `claw.pkg` and `claw.lock`: files or source changed, or the package is not in the lock; `claw get --update` (or `claw get`) |
| E0184 | the C header is broken (for example, an `#if` is not closed) |
| E0185 | the C function that is called is in no loaded library, or it is a `static inline` from the header |
| E0900 | not used (formerly: a reserved feature) |
| E0901 | the function is too large for this version |
| E0902 | the formatter refused to change the file |
| E0905 | a vector outside `claw fn` |
| E0906 | a vector where it cannot be: a field, an element, a pointer, `?T`, a generic argument, a parameter after the eighth |
| E0907 | not used (formerly: a vector on x86-64) |
| E0908 | wrong use of `vload`, `vstore`, `lane`, `fma` |
| E0909 | an operation the vector does not have: `/` for integers, `print` |
