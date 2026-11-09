struct Stack {
    items: Vec<i32>,
}

impl Stack {
    fn push(&mut self, x: i32) {
        self.items.push(x);
    }

    fn pop(&mut self) -> Option<i32> {
        self.items.pop()
    }
}

fn main() {
    let mut s = Stack { items: Vec::new() };
    s.push(1);
    s.push(2);
    println!("{}", s.pop().unwrap_or(-1));
    println!("{}", s.pop().unwrap_or(-1));
    println!("{}", s.pop().unwrap_or(-1));
}
