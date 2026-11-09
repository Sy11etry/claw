fn search(a: &[i64], x: i64) -> Option<usize> {
    let mut lo = 0;
    let mut hi = a.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if a[mid] == x {
            return Some(mid);
        }
        if a[mid] < x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

fn main() {
    let a = [1, 3, 5, 7, 9, 11];
    println!("{}", search(&a, 7).map_or(-1, |i| i as i64));
    println!("{}", search(&a, 4).map_or(-1, |i| i as i64));
}
