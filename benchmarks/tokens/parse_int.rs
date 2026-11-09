enum ParseErr {
    Empty,
    Bad(usize),
}

fn parse(s: &str) -> Result<i64, ParseErr> {
    if s.is_empty() {
        return Err(ParseErr::Empty);
    }
    let mut n = 0;
    for (i, c) in s.bytes().enumerate() {
        if !c.is_ascii_digit() {
            return Err(ParseErr::Bad(i));
        }
        n = n * 10 + (c - b'0') as i64;
    }
    Ok(n)
}

fn show(s: &str) {
    match parse(s) {
        Ok(v) => println!("{}", v),
        Err(ParseErr::Empty) => println!("empty"),
        Err(ParseErr::Bad(at)) => println!("bad at {}", at),
    }
}

fn main() {
    show("123");
    show("");
    show("12x");
}
