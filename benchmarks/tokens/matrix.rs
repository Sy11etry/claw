#[derive(Clone, Copy)]
struct Mat {
    a: [[i64; 2]; 2],
}

fn mul(x: Mat, y: Mat) -> Mat {
    let mut r = [[0; 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                r[i][j] += x.a[i][k] * y.a[k][j];
            }
        }
    }
    Mat { a: r }
}

fn main() {
    let m = Mat { a: [[1, 1], [1, 0]] };
    let mut p = m;
    for _ in 0..9 {
        p = mul(p, m);
    }
    println!("{}", p.a[0][1]);
}
