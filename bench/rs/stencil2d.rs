use std::hint::black_box;
use std::time::Instant;

fn relax(src: &[f64], dst: &mut [f64], n: usize, m: usize) {
    for i in 1..n - 1 {
        for j in 1..m - 1 {
            dst[i * m + j] = 0.25 * (src[(i - 1) * m + j] + src[(i + 1) * m + j] + src[i * m + j - 1] + src[i * m + j + 1]);
        }
    }
}

fn checksum(a: &[f64], n: usize, m: usize) -> f64 {
    let mut total = 0.0;
    for i in 0..n {
        for j in 0..m {
            total += a[i * m + j] * (i + 1) as f64 / (j + 1) as f64;
        }
    }
    total
}

fn main() {
    let n: usize = black_box(200);
    let m: usize = black_box(300);
    let steps: usize = black_box(60);
    let mut a = vec![0.0f64; n * m];
    for j in 0..m {
        a[j] = 100.0;
    }
    for i in 0..n {
        a[i * m] = 50.0;
    }
    let mut b = a.clone();
    let start = Instant::now();
    for _ in 0..steps {
        relax(&a, &mut b, n, m);
        relax(&b, &mut a, n, m);
    }
    let total = checksum(&a, n, m);
    let elapsed = start.elapsed().as_secs_f64();
    println!("{total:?}");
    eprintln!("kernel_seconds={elapsed}");
}
