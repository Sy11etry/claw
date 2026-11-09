struct Pair<A, B> {
    a: A,
    b: B,
}

fn max<T: PartialOrd>(x: T, y: T) -> T {
    if x > y {
        x
    } else {
        y
    }
}

fn main() {
    println!("{}", max(3, 9));
    println!("{}", max(2.5, 1.5));
    let p = Pair { a: 1, b: "one" };
    println!("{} {}", p.a, p.b);
}
