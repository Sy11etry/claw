fn double(x: i32) -> i32 {
    x * 2
}

fn square(x: i32) -> i32 {
    x * x
}

fn apply(a: &[i32], f: fn(i32) -> i32) -> Vec<i32> {
    a.iter().map(|x| f(*x)).collect()
}

fn main() {
    println!("{:?}", apply(&[1, 2, 3], double));
    println!("{:?}", apply(&[1, 2, 3], square));
}
