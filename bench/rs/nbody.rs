#[derive(Clone, Copy)] struct B { x: f64, y: f64, z: f64, vx: f64, vy: f64, vz: f64, m: f64 }
fn energy(b: &[B]) -> f64 {
    let mut e = 0.0;
    for i in 0..b.len() { let p = b[i]; e += 0.5 * p.m * (p.vx * p.vx + p.vy * p.vy + p.vz * p.vz);
        for q in &b[i + 1..] { let (dx, dy, dz) = (p.x - q.x, p.y - q.y, p.z - q.z); e -= p.m * q.m / (dx * dx + dy * dy + dz * dz).sqrt(); } }
    e
}
fn advance(b: &mut [B], dt: f64, steps: usize) {
    let n = b.len();
    for _ in 0..steps {
        for i in 0..n { for j in i + 1..n {
            let (dx, dy, dz) = (b[i].x - b[j].x, b[i].y - b[j].y, b[i].z - b[j].z);
            let d2 = dx * dx + dy * dy + dz * dz; let mag = dt / (d2 * d2.sqrt());
            let (mi, mj) = (b[i].m, b[j].m);
            b[i].vx -= dx * mj * mag; b[i].vy -= dy * mj * mag; b[i].vz -= dz * mj * mag;
            b[j].vx += dx * mi * mag; b[j].vy += dy * mi * mag; b[j].vz += dz * mi * mag;
        } }
        for p in b.iter_mut() { p.x += dt * p.vx; p.y += dt * p.vy; p.z += dt * p.vz; }
    }
}
fn r9(x: f64) -> f64 { (x * 1e9).round() / 1e9 }
fn main() {
    let pi = 3.14159265358979323; let sm = 4.0 * pi * pi; let d = 365.24;
    let mut b = vec![
        B { x: 0.0, y: 0.0, z: 0.0, vx: 0.0, vy: 0.0, vz: 0.0, m: sm },
        B { x: 4.84143144246472090e+00, y: -1.16032004402742839e+00, z: -1.03622044471123109e-01, vx: 1.66007664274403694e-03 * d, vy: 7.69901118419740425e-03 * d, vz: -6.90460016972063023e-05 * d, m: 9.54791938424326609e-04 * sm },
        B { x: 8.34336671824457987e+00, y: 4.12479856412430479e+00, z: -4.03523417114321381e-01, vx: -2.76742510726862411e-03 * d, vy: 4.99852801234917238e-03 * d, vz: 2.30417297573763929e-05 * d, m: 2.85885980666130812e-04 * sm },
        B { x: 1.28943695621391310e+01, y: -1.51111514016986312e+01, z: -2.23307578892655734e-01, vx: 2.96460137564761618e-03 * d, vy: 2.37847173959480950e-03 * d, vz: -2.96589568540237556e-05 * d, m: 4.36624404335156298e-05 * sm },
        B { x: 1.53796971148509165e+01, y: -2.59193146099879641e+01, z: 1.79258772950371181e-01, vx: 2.68067772490389322e-03 * d, vy: 1.62824170038242295e-03 * d, vz: -9.51592254519715870e-05 * d, m: 5.15138902046611451e-05 * sm },
    ];
    let (mut px, mut py, mut pz) = (0.0, 0.0, 0.0);
    for p in &b { px += p.vx * p.m; py += p.vy * p.m; pz += p.vz * p.m; }
    b[0].vx = -px / sm; b[0].vy = -py / sm; b[0].vz = -pz / sm;
    println!("{}", r9(energy(&b)));
    advance(&mut b, 0.01, std::hint::black_box(500_000));
    println!("{}", r9(energy(&b)));
}
