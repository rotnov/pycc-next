fn a(i: usize, j: usize) -> f64 { 1.0 / (((i + j) * (i + j + 1) / 2 + i + 1) as f64) }
fn mul_av(v: &[f64]) -> Vec<f64> { (0..v.len()).map(|i| (0..v.len()).map(|j| a(i, j) * v[j]).sum()).collect() }
fn mul_atv(v: &[f64]) -> Vec<f64> { (0..v.len()).map(|i| (0..v.len()).map(|j| a(j, i) * v[j]).sum()).collect() }
fn main() {
    let n = std::hint::black_box(400);
    let mut u = vec![1.0; n]; let mut v = vec![0.0; n];
    for _ in 0..10 { v = mul_atv(&mul_av(&u)); u = mul_atv(&mul_av(&v)); }
    let vbv: f64 = (0..n).map(|i| u[i] * v[i]).sum(); let vv: f64 = (0..n).map(|i| v[i] * v[i]).sum();
    let r = (vbv / vv).sqrt();
    println!("{}", (r * 1e9).round() / 1e9);
}
