use std::hint::black_box;
use std::time::Instant;

fn fill(xs: &mut [i64], seed: i64) {
    let mut x = seed;
    for i in 0..xs.len() {
        x = (x * 1103515245 + 12345).rem_euclid(2147483648);
        xs[i] = x.rem_euclid(2001) - 1000;
    }
}

fn prefix_sums(xs: &[i64], out: &mut [i64]) {
    let mut acc = 0i64;
    for i in 0..xs.len() {
        acc += xs[i];
        out[i] = acc;
    }
}

fn running_max(xs: &[i64], out: &mut [i64]) {
    let mut best = xs[0];
    for i in 0..xs.len() {
        if xs[i] > best {
            best = xs[i];
        }
        out[i] = best;
    }
}

fn main() {
    let n: usize = black_box(400000);
    let rounds: usize = black_box(25);
    let mut xs = vec![0i64; n];
    let mut sums = vec![0i64; n];
    let mut peaks = vec![0i64; n];
    fill(&mut xs, 42);
    let start = Instant::now();
    let mut check = 0i64;
    for r in 0..rounds {
        xs[r] += 1;
        prefix_sums(&xs, &mut sums);
        running_max(&sums, &mut peaks);
        check += sums[n - 1] + peaks[n - 1] + peaks[n / 3];
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!("{check}");
    eprintln!("kernel_seconds={elapsed}");
}
