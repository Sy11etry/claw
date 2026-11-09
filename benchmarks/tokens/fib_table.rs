fn main() {
    let mut a = [0i64; 20];
    a[1] = 1;
    for i in 2..20 {
        a[i] = a[i - 1] + a[i - 2];
    }
    let sum: i64 = a.iter().sum();
    println!("{} {}", a[19], sum);
}
