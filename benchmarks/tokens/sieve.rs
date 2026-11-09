fn main() {
    let mut composite = [false; 100];
    let mut count = 0;
    for i in 2..100 {
        if composite[i] {
            continue;
        }
        count += 1;
        let mut j = i * i;
        while j < 100 {
            composite[j] = true;
            j += i;
        }
    }
    println!("{}", count);
}
