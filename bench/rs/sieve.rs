fn count_primes(limit: usize) -> i64 {
    let mut p = vec![true; limit + 1];
    p[0] = false; p[1] = false;
    let mut i = 2;
    while i * i <= limit { if p[i] { let mut j = i * i; while j <= limit { p[j] = false; j += i; } } i += 1; }
    p.iter().filter(|&&f| f).count() as i64
}
fn main() { let mut t = 0; for _ in 0..5 { t += count_primes(std::hint::black_box(2_000_000)); } println!("{t}"); }
