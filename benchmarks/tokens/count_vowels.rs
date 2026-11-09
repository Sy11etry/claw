fn vowels(s: &str) -> usize {
    s.chars().filter(|c| "aeiou".contains(*c)).count()
}

fn main() {
    println!("{}", vowels("the quick brown fox"));
}
