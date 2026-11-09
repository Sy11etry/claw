enum Shape {
    Square(i64),
    Rect(i64, i64),
    Empty,
}

fn area(x: &Shape) -> i64 {
    match x {
        Shape::Square(s) => s * s,
        Shape::Rect(w, h) => w * h,
        Shape::Empty => 0,
    }
}

fn main() {
    let a = [Shape::Square(3), Shape::Rect(2, 5), Shape::Empty];
    let t: i64 = a.iter().map(area).sum();
    println!("{}", t);
}
