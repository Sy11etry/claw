# Claw 0.1 card

One statement per line. No `;`. A line ending in an operator, comma or open bracket continues.

```claw
fn add(a int, b int) int {
    ret a + b
}

fn main() {                  // required, no params, no result
    x := 5                   // immutable, type inferred
    y int = 7                // explicit type
    mut s := 0               // mutable
    for i in 0..10 { s += i }
    for s > 40 { s -= 1 }    // while
    for { break }            // forever
    if s == 40 {
        print("yes")
    } else if x > y {
        print(x)
    } else {
        print(false)
    }
    _ = add(x, y)            // discard a result
}
```

Types: `int` (64-bit signed), `bool`. No implicit conversions.
Operators: `+ - * / % & | ^ ~ << >>`, `== != < <= > >=`, `and or not`.
`print` takes one int, bool or string literal and adds a newline.

Errors, not warnings: unused variable or parameter (name it `_`), unused call result, reusing a name in any
enclosing scope, `mut` that never changes, unreachable code, a value function that can end without `ret`.

Signed overflow, division by zero and bad shifts stop the program with exit code 101 and `file:line:col`.

Not in 0.1 (`E0900`): other number types, floats, strings as values, structs, arrays, pointers, modules.
