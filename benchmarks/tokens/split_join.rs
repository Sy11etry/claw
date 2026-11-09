fn main() {
    let parts: Vec<&str> = "a,b,c".split(',').collect();
    println!("{}", parts.len());
    println!("{}", parts.join("-"));
    println!("{}", "  claw ".trim().to_uppercase());
}
