fn mandel(w: i64, h: i64, max_iter: i64) -> i64 {
    let mut inside = 0;
    for py in 0..h { for px in 0..w {
        let cr = 3.0 * px as f64 / w as f64 - 2.0; let ci = 2.0 * py as f64 / h as f64 - 1.0;
        let (mut zr, mut zi, mut n) = (0.0f64, 0.0f64, 0);
        while n < max_iter && zr * zr + zi * zi <= 4.0 { let t = zr * zr - zi * zi + cr; zi = 2.0 * zr * zi + ci; zr = t; n += 1; }
        if n == max_iter { inside += 1; }
    } }
    inside
}
fn main() { println!("{}", mandel(std::hint::black_box(600), std::hint::black_box(400), std::hint::black_box(200))); }
