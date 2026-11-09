fn is_pal(s: &str) -> bool {
    let b = s.as_bytes();
    let n = b.len();
    for i in 0..n / 2 {
        if b[i] != b[n - 1 - i] {
            return false;
        }
    }
    true
}

fn main() {
    println!("{}", is_pal("level"));
    println!("{}", is_pal("claw"));
    println!("{}", is_pal(""));
}
