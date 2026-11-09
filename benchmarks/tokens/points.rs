#[derive(Clone, Copy)]
struct Point {
    x: i64,
    y: i64,
}

fn add(a: Point, b: Point) -> Point {
    Point { x: a.x + b.x, y: a.y + b.y }
}

fn dist2(a: Point, b: Point) -> i64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let q = Point { x: 4, y: 6 };
    let r = add(p, q);
    println!("{} {}", r.x, r.y);
    println!("{}", dist2(p, q));
}
