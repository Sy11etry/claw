# Claw 0.2 card

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

Types: `int` (= `i64`), `i8 i16 i32 u8 u16 u32 u64 f32 f64 bool`, `[N]T`, `?T`, `T ! E`, `! E`. No implicit conversions:
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

Not yet (`E0900`): strings as values, `[]T`, pointers, modules, generics.
