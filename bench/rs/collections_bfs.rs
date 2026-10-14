use std::collections::{HashMap, VecDeque};
fn bfs_counts(n: usize) -> i64 {
    let mut dist = vec![-1i64; n];
    dist[0] = 0;
    let mut q = VecDeque::from([0usize]);
    while let Some(u) = q.pop_front() {
        for v in [(2 * u) % n, (u + 1) % n] {
            if dist[v] < 0 { dist[v] = dist[u] + 1; q.push_back(v); }
        }
    }
    let mut c: HashMap<i64, i64> = HashMap::new();
    for d in &dist { *c.entry(*d).or_insert(0) += 1; }
    *c.values().max().unwrap() + c.len() as i64
}
fn main() { let mut t = 0; for _ in 0..10 { t += bfs_counts(std::hint::black_box(300_000)); } println!("{t}"); }
