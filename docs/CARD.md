# Claw 0.4 card

One statement per line. No `;`. A line ending in an operator, comma or open bracket continues.

```claw
type Point { x int, y int }              // struct, laid out like C
type Shape = Circle{r f64} | Dot         // variants
type ParseErr = Empty | Bad{at int}

const limit = 100                        // compile-time value

fn add(a Point, b Point) Point {         // parameters are read-only
    ret Point{a.x + b.x, a.y + b.y}      // struct fields are positional
}

fn area(s Shape) f64 {
    match s {                            // must cover every case; `_:` is the default
        Circle{r}: ret 3.14 * r * r
        Dot: ret 0
    }
}

fn digit(c u8) int ! ParseErr {          // may fail
    if c < '0' or c > '9' { fail Bad{0} }
    ret int(c - '0')
}

fn find(a [3]int, x int) ?int {          // may be absent
    for v, i in a {
        if v == x { ret i }
    }
    ret none
}

fn main() {                              // required, no params, no result
    x := 5                               // immutable, type inferred
    y i32 = 7                            // explicit type
    mut s := 0                           // mutable
    for i in 0..10 { s += i }
    for s > 40 { s -= 1 }                // while
    for { break }                        // forever
    if s == 40 {
        print("s = {s}, x = {x}, sum = {s + x}")
    } else {
        print(y)
    }
    mut a [4]int = [0...]                // array filled with 0
    a[1] = len(a)
    b := [1, 2, 3]
    d := digit('7')?                     // pass the failure up
    e := digit('x') else 0               // or use a default
    f := find(b, 2) else {               // or leave; the block must exit
        ret
    }
    g := digit('x') else err {           // `err` is the error value
        print("{err}")
        ret
    }
    print(a[1] + d + e + f + g)
}
```

Types: `int` (= `i64`), `i8 i16 i32 u8 u16 u32 u64 f32 f64 bool str`, `[N]T`, `[]T`, `?T`, `T ! E`, `! E`. No implicit conversions:
write `i32(x)`, `f64(n)`, `int(c)`. A number literal takes the type around it. `'a'` is a `u8`.

Operators: `+ - * / %`, `& | ^ ~ << >>`, `== != < <= > >=`, `and or not`.
Struct and array values copy on assignment. `==` works on numbers and bool; use `match` for variants.
Names: types and variants start with a capital, everything else with a small letter.

`print` takes one value of any type, or a string with `{expr}` (`{{` is a literal brace), and adds a newline.

Errors, not warnings: unused variable or parameter (name it `_`), unused call result (`_ = f()`), reusing a name in
any enclosing scope, `mut` that never changes, unreachable code, a value function that can end without `ret`,
changing an array while `for` walks it, ignoring a `T ! E`.

Stops the program with exit code 101 and `file:line:col`: signed overflow, division by zero, bad shift, array index
out of range, a conversion that does not fit a signed type (unsigned types wrap).

## Memory is automatic

```claw
fn names() []str {
    mut a []str = []                     // growable; `[1, 2]` works too
    push(&a, "cat")
    ret a                                // moves out, no copy
}

fn total(a []int) int { ... }            // parameter: read-only view
fn clear_it(a &[]int) { a = [] }         // `&`: may change it; call with clear_it(&x)
fn keep(own s str) Box { ... }           // `own`: takes it over; call with keep(s)

fn demo() {
    a := names()
    b := a                               // `a` has moved: using it now is an error
    c := copy(b)                         // copies are always written out
    s := "n = {len(c)}"                  // `{expr}` builds a str
    v := b[1..]                          // read-only view; also `[..2]`, works on str
    print(v)
    print(s[0..1] == "n")
}                                        // everything still owned is freed here
```

Only a whole local variable can move. For a field or an element: `copy(x)`, `remove(&a, i)`, `swap(&a, &b)`.
A collection cannot change while `for` walks it. One call cannot pass the same thing with `&` and without.
`s[i]` is a `u8`; a `str` cannot be changed in place. A type may define `drop(x &T)`, run when it goes away.

Always available: `push pop insert remove clear reserve swap sort has index_of`,
`find contains starts_with ends_with trim split lines join upper lower to_int`. Also `use fs`, `use math`.

## Templates and modules

```claw
use lib.geo                              // the file lib/geo.claw, known here as `geo`

type Pair[A, B] { a A, b B }

pub fn max[T](a T, b T) T {              // `pub` is visible from other modules
    if a > b { ret a }
    ret b
}

fn demo() {
    print(max(1, 2))                     // types are worked out from the values
    print(max[i32](1, 2))                // or written out
    p := Pair[int, f64]{1, 2.5}
    q := geo.make(3)                     // `module.name`
    print(area(q))                       // or by the module of the first argument's type
}
```

There are no methods: write `area(q)`, not `q.area()`.

## Claw mode: `claw fn` is C

```claw
extern fn strlen(s *u8) u64              // any libSystem function; callable from `claw fn`

type Node { next *Node, v int }

claw fn sum() int {
    p := alloc[int](3)                   // n items on the heap; check-free pointer
    defer free(p)                        // runs when this block ends, however it ends
    p[0] = 1
    q := p + 1                           // moves by whole items
    *q = 2
    mut x := 5
    r := &x                              // `&` needs a `mut` variable
    *r += 1
    n *Node = none                       // the null pointer
    if n == none { print(strlen("hi")) }
    ret p[0] + p[1] + x
}
```

`*T` values, `&`, `*`, `alloc`, `free`, `cast[T](x)`, `defer`, `extern` calls: only in `claw fn`. `p.x` reads through
a pointer. `sizeof[T]()` is the size in bytes. `cast` converts between pointers, `int` and `u64`. A string passed
where `*u8` is expected is its NUL-terminated bytes. Arithmetic and array bounds stay checked in `claw fn`;
raw pointers do not.

In `claw fn`, `a.ptr`, `a.len`, `a.cap` of `[]T` and `str` are open to change.

Not yet (`E0900`): generic variants, `test`.
