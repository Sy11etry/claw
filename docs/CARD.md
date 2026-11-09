# Claw 0.8 card

One statement per line. No `;`. A line ending in an operator, comma or open bracket continues.

```claw
type Point { x int, y int }              // struct
type Shape = Circle{r f64} | Dot         // variants
type ParseErr = Empty | Bad{at int}

const limit = 100
const big int = twice(limit)             // typed: may call functions; runs while compiling

fn add(a Point, b Point) Point {         // parameters are read-only
    ret Point{a.x + b.x, a.y + b.y}      // struct fields are positional
}

fn area(s Shape) f64 {
    match s {                            // must cover every case, or `_:`
        Circle{r}: ret 3.14 * r * r
        Dot: ret 0
    }
}

fn digit(c u8) int ! ParseErr {
    if c < '0' or c > '9' { fail Bad{0} }
    ret int(c - '0')
}

fn find(a []int, x int) ?int {
    for v, i in a {
        if v == x { ret i }
    }
    ret none
}

fn main() {
    x := 5                               // immutable, type inferred
    y i32 = 7
    mut s := 0
    for i in 0..10 { s += i }
    for s > 40 { s -= 1 }                // while
    for { break }
    if s == 40 {
        print("s = {s}, sum = {s + x}")
    } else {
        print(y)
    }
    mut a [4]int = [0...]                // all 0
    a[1] = len(a)
    b := [1, 2, 3]
    d := digit('7')?                     // pass the failure up
    e := digit('x') else 0               // or a default
    f := find(b, 2) else {               // or a block that exits
        ret
    }
    g := digit('x') else err {           // the error value
        print("{err}")
        ret
    }
    print(a[1] + d + e + f + g)
}
```

Types: `int` (= `i64`), `i8 i16 i32 u8 u16 u32 u64 f32 f64 bool str`, `[N]T`, `[]T`, `?T`, `T ! E`, `! E`. No implicit conversions:
write `i32(x)`, `f64(n)`, `int(c)`. A number literal takes the type around it. `'a'` is a `u8`.

Operators as in C, but `and or not`; no `++` or `?:`.
Struct and array values copy on assignment. `==` works on numbers and bool; use `match` for variants.
Names: types and variants start with a capital, everything else with a small letter.

`print` takes one value of any type, or a string with `{expr}` (`{{` is a literal brace), and adds a newline.

Errors, not warnings: unused variable or parameter (name it `_`), unused call result (`_ = f()`), reusing a name in
any enclosing scope, `mut` that never changes, unreachable code, a function that can end without `ret`, ignoring a `T ! E`.

Exit code 101 with `file:line:col`: signed overflow, division by zero, bad shift, index out of range,
a conversion that does not fit a signed type. Unsigned types wrap.

## Memory is automatic

```claw
fn names() []str {
    mut a []str = []                     // growable
    push(&a, "cat")
    ret a
}

fn total(a []int) int { ... }            // parameter: read-only view; takes a `[N]int` too
fn clear_it(a &[]int) { a = [] }         // `&`: may change it; call clear_it(&x)
fn keep(own s str) Box { ... }           // `own`: takes it over; call keep(s)

fn demo() {
    a := names()
    b := a                               // `a` moved
    c := copy(b)
    s := "n = {len(c)}"                  // a new str
    v := b[1..]                          // also `[..2]`, and on str
    print("{s} {v}")
}                                        // the rest is freed here
```

Only a whole local variable can move. `x := a[i]` (or a field, or a slice) is a read-only view: `a` cannot change while
`x` lives. To own one: `mut x := copy(a[i])`, `remove(&a, i)`, `swap(&a, &b)`.
A collection cannot change while `for` walks it. One call cannot pass the same thing with `&` and without.
`s[i]` is a `u8`; a `str` cannot change in place. A type may define `drop(x &T)`.

Always available: `push pop insert remove clear reserve swap sort has index_of`,
`find contains starts_with ends_with trim split lines join upper lower to_int`. Also `use fs`, `use math`.
`claw sig std` lists their signatures.

Map: `mut m := new_map[str, int](hash_str)` (or `hash_int`), `put(&m, k, v)`, `get(m, k) else 0`, `has_key(m, k)`,
`del(&m, k)`, `for e in m.items { e.key, e.val }`, `len(m.items)`.

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
    print(max(1, 2))
    p := Pair[int, f64]{1, 2.5}
    t Tree[int] = Node{1, []}            // type from the context, or `Node[int]{...}`
    f fn(int) int = twice                // a named function as a value
    print(geo.make(3))
}
```

No methods. `+mem +io` may follow a function type. A template, an `extern` or a function with `need` cannot be a value.

## Claw mode: `claw fn` is C

```claw
extern fn strlen(s *u8) u64              // any libSystem function

claw fn sum() int {
    p := alloc[int](3)                   // 3 items on the heap
    defer free(p)                        // when this block ends
    p[0] = 1
    q := p + 1
    *q = 2
    mut x := 5
    r := &x                              // `&` needs a `mut`
    *r += 1
    print(strlen("hi"))
    ret p[0] + p[1] + x
}
```

`none` is null. Pointers, `alloc`, `free`, `cast[T](x)`, `defer`, `extern` calls: only in `claw fn`. `p.x` reads through a pointer.
`sizeof[T]()` is in bytes. `cast` converts between pointers, `int` and `u64`. A string passed as `*u8` is NUL-terminated.
Arithmetic and array bounds stay checked; raw pointers do not.

## Contracts, effects, tests

```claw
pub fn div(a int, b int) int +io
    need b != 0                          // checked at every call
    sure ret * b <= a                    // checked at every return; `old(x)` is x on entry
{
    ret a / b
}

test "div" {
    sure div(6, 2) == 3                  // works in any body
}
```

`+mem` allocates, `+io` touches the world. `pub` must declare them; private functions have them worked out.
A contract only looks: no `&`, moves or effects.

`claw run` and `claw test` check everything, also heap misuse and leaks. `claw build` leaves out contracts that call
functions; `--fast` removes every check.
`claw test` also fuzzes functions without `+io`. `claw why f.claw` explains a failure. `claw fix f.claw` applies patches.
`claw sig f.claw`: signatures. `claw ctx f.claw name`: one function's context. `claw audit f.claw`: trusted code.
