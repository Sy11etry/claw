struct Node {
    v: i32,
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

fn insert(t: &mut Option<Box<Node>>, x: i32) {
    match t {
        None => *t = Some(Box::new(Node { v: x, left: None, right: None })),
        Some(n) => {
            if x < n.v {
                insert(&mut n.left, x)
            } else {
                insert(&mut n.right, x)
            }
        }
    }
}

fn walk(t: &Option<Box<Node>>, out: &mut Vec<i32>) {
    if let Some(n) = t {
        walk(&n.left, out);
        out.push(n.v);
        walk(&n.right, out);
    }
}

fn main() {
    let mut t = None;
    for x in [5, 2, 8, 1, 9] {
        insert(&mut t, x);
    }
    let mut out = Vec::new();
    walk(&t, &mut out);
    println!("{:?}", out);
}
