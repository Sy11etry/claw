use std::collections::BTreeMap;

fn main() {
    let text = "the cat and the dog and the bird";
    let mut m: BTreeMap<&str, i32> = BTreeMap::new();
    for w in text.split(' ') {
        *m.entry(w).or_insert(0) += 1;
    }
    for (w, n) in &m {
        println!("{} {}", w, n);
    }
}
