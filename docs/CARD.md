# Claw 1.0 card

One statement per line, no `;`. A line ending in an operator, comma or open bracket continues.

```claw
type Point { x int, y int }             // Point{1, 2}: fields are positional
type Shape = Circle{r f64} | Dot
type ParseErr = Empty | Bad{at int}

const limit = 100                        // it may call functions, run while compiling

fn area(s Shape) f64 {
    match s {                            // must cover every case, or `_:`
        Circle{r}: ret 3 * r * r
        Dot: ret 0
    }
}

fn digit(c u8) int ! ParseErr {
    if c < '0' { fail Bad{0} }
    ret int(c - '0')
}

fn first(a []int) ?int {
    if len(a) == 0 { ret none }
    ret a[0]
}

fn main() {
    x := 5                               // immutable
    mut s := 0
    for i in 0..10 { s += i }            // `for v, i in a` walks values with their index
    for s > 40 { s -= 1 }                // while; `for { }` loops forever
    if s == 40 { print("s = {s + x}") }
    mut a [4]int = [0...]                // all 0
    a[1] = 2
    b := [1, 2]
    d := digit('7')?                     // pass the failure up
    e := digit('x') else 0               // or a default
    f := first(b) else { ret }           // or a block that exits
    g := digit('x') else err { ret }     // `err` is the failure
    print(a[1] + d + e + f + g)
}
```

Types: `int` (= `i64`), `i8 i16 i32 u8 u16 u32 u64 f32 f64 bool str`, `[N]T`, `[]T`, `?T`, `T ! E`, `! E`. No implicit
conversions: `i32(x)`, `f64(n)`. A literal takes the type around it. `'a'` is a `u8`.

Operators as in C, but `and or not`; no `++` or `?:`.
Structs and arrays copy on assignment. `==` works on numbers and bool; `match` variants.
Types and variants start with a capital, the rest with a small letter.

`print` takes one value or a string with `{expr}` (`{{` is a brace) and adds a newline. A float prints as the shortest
text that reads back exactly (`0.1`, `3`, `1e+16`). `match` also takes ints, chars and `bool`:
`0: ret "zero"`, `_: ret "many"`; a failure is matched by its variants: `Failed{code}: ...`.

Errors, not warnings: an unused variable, parameter (name it `_`) or result (`_ = f()`), a reused name,
`mut` never changed, unreachable code, a missing `ret`, an ignored `T ! E`.

Exit code 101 with `file:line:col`: signed overflow, division by zero, bad shift, index, slice (`a[3..1]`) or conversion.
Unsigned types and integer conversions to them wrap; `f64` to an int out of range traps. Endless recursion prints
`stack overflow` and ends with signal 139.

## Memory is automatic

```claw
fn names() []str {
    mut a []str = []                     // growable
    push(&a, "cat")
    ret a
}

fn total(a []int) int { ... }            // read-only view; takes a `[N]int` too
fn clear_it(a &[]int) { a = [] }         // `&`: may change it; call clear_it(&x)
fn keep(own s str) Box { ... }           // `own`: takes it; call keep(s)

fn demo() {
    a := names()
    b := a                               // `a` moved
    c := copy(b)
    print("{len(c)} {b[1..]}")           // slices: `[..2]` too
}                                        // the rest is freed here
```

Only a whole local variable or `own` parameter moves. A parameter, `a[i]`, a field or a slice only reads, so return
a copy: `ret copy(p)`. While `x := a[i]` lives, `a` cannot change. To own one: `copy(a[i])`, `remove(&a, i)`.
A collection cannot change while `for` walks it; one call cannot pass a thing with `&` and without.
`s[i]` is a `u8`; a `str` is immutable. A type may define `drop(x &T)`.

Always available: `push pop insert remove clear reserve swap sort has index_of find contains starts_with ends_with
trim split lines join upper lower to_int`; `claw sig std` lists them.
Modules: `fs.read(path)` and `fs.write(path, s)` fail with `IoErr` (`NotFound | Denied | Failed{code}`), `math`,
`os.args()` (`[0]` is the program), `os.env(name)` (`?str`), `os.exit(n)`, `par`.
Threads share nothing: `t := par.spawn(work, x)`, `par.join(t)`, `work` is `fn(own T) R +mem`.
`par.map(xs, f)`, `par.build(n, f)` run `f` on all cores, results in order.

Map: `mut m := new_map[str, int](hash_str)` (or `hash_int`), `put(&m, copy(k), v)`, `get(m, k) else 0`, `has_key`,
`del`, `for e in m.items { e.key, e.val }`.

## Templates and modules

```claw
use lib.geo                              // lib/geo.claw, known as `geo`

type Pair[A, B] { a A, b B }
type Tree[T] = Leaf | Node{v T, kids []Tree[T]}

pub fn max[T](a T, b T) T {
    if a > b { ret a }
    ret b
}

fn demo() {
    t Tree[int] = Node{1, []}            // type from the context, or `Node[int]{...}`
    f fn(int) int = twice                // a named function as a value
    print(max(1, 2) + geo.make(3))
}
```

No methods. `+mem +io` may follow a function type. Templates, `extern` and `need` functions are not values.

## Claw mode: `claw fn` is C

```claw
extern fn strlen(s *u8) u64              // any libSystem function

claw fn sum() int {
    p := alloc[int](3)
    defer free(p)                        // when this block ends
    *(p + 1) = 2
    mut x := 5
    r := &x                              // `&` needs a `mut`
    *r += strlen("hi")
    ret p[1] + x
}
```

`none` is null. Pointers, `alloc`, `free`, `cast[T](x)`, `defer`, `extern` calls: only in `claw fn`; `p.x` works on pointers.
`sizeof[T]()` is in bytes. `cast` converts between pointers, `int`, `u64`. A string literal as `*u8` is NUL-terminated;
`s.ptr` is the bytes of a `str`. Raw pointers are unchecked.
C: `use c "zlib.h"` brings `extern fn`, `type`, `const` from a header (`Z_OK` is `z_ok`, C `int` is `i32`);
`link "z"` or `-l z` loads the library; `cast[*u8](f)` passes `f` to C as a callback (up to 6 numbers or pointers).
A C struct of numbers, pointers and arrays goes by value up to 16 bytes, and comes back by value of any size;
`str`, `[]T` and `own` never go to C.

Vectors (`f64x2 f32x4 i32x4`) live in `claw fn` only: `+ - *`, `/` on floats, `f64x2(x)` fills every lane.

```claw
claw fn dot2(a *f64, b *f64, out *f64) f64 {
    v := vload(a) * vload(b)             // 16 bytes
    vstore(out, fma(v, f64x2(2.0), v))   // a * b + c
    ret lane(v, 0) + lane(v, 1)          // a literal
}
```

## Contracts, effects, tests

```claw
pub fn div(a int, b int) int +io
    need b != 0                          // checked at every call
    sure ret * b <= a                    // checked at every return; `old(x)` is x on entry
{
    ret a / b
}

test "div" {
    sure div(6, 2) == 3
}
```

`+mem` allocates, `+io` touches the world. `pub` declares them; private ones are inferred. A contract has no `&`, moves or effects.

`claw run` and `claw test` check everything, heap misuse and leaks too; `claw build` skips contracts that call
functions, `--fast` every check. `claw test` also fuzzes functions without `+io`. `claw why f.claw` explains a failure,
`fix` applies patches, `sig` lists signatures, `ctx f.claw name` gives one function's context, `audit` shows trusted code.
`claw f.claw args` runs a script (`#!` works). Packages: `claw.pkg` lines `name source`, `claw get`, `use name`.
`claw fmt` formats; `claw cdecl h.h` shows what a header becomes.
`claw build --target x86_64` (or `aarch64-linux`) picks the machine; `-g` writes debug info for lldb.

Crash of a program that runs elsewhere: start it as `CLAW_RECORD=crash.clawtrace ./app`. If it traps, it writes what it
was given (arguments, environment, files it read) to that file. Then `claw why crash.clawtrace app.claw` plays the same
crash again here and names the lines that caused it. Without the variable nothing is recorded or written.
