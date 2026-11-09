fn isort(a: &mut [i32]) {
    for i in 1..a.len() {
        let x = a[i];
        let mut j = i;
        while j > 0 && a[j - 1] > x {
            a[j] = a[j - 1];
            j -= 1;
        }
        a[j] = x;
    }
}

fn main() {
    let mut b = vec![5, 2, 9, 1, 7];
    isort(&mut b);
    println!("{:?}", b);
}
