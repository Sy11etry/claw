fn first_neg(a: &[i32]) -> Option<i32> {
    a.iter().copied().find(|x| *x < 0)
}

fn main() {
    println!("{}", first_neg(&[3, -4, 5]).unwrap_or(0));
    println!("{}", first_neg(&[3, 4]).unwrap_or(0));
}
