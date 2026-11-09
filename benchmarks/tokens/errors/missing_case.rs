enum C {
    R,
    G,
    B,
}

fn f(c: C) -> i32 {
    match c {
        C::R => 1,
        C::G => 2,
    }
}

fn main() {
    println!("{}", f(C::B));
}
