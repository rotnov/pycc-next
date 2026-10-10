class Body:
    def __init__(self, x: float, y: float, z: float, vx: float, vy: float, vz: float, m: float) -> None:
        self.x = x
        self.y = y
        self.z = z
        self.vx = vx
        self.vy = vy
        self.vz = vz
        self.m = m


def advance(bodies: list[Body], dt: float, steps: int) -> None:
    n = len(bodies)
    for _ in range(steps):
        for i in range(n):
            b = bodies[i]
            for j in range(i + 1, n):
                c = bodies[j]
                dx = b.x - c.x
                dy = b.y - c.y
                dz = b.z - c.z
                d2 = dx * dx + dy * dy + dz * dz
                mag = dt / (d2 * d2 ** 0.5)
                b.vx -= dx * c.m * mag
                b.vy -= dy * c.m * mag
                b.vz -= dz * c.m * mag
                c.vx += dx * b.m * mag
                c.vy += dy * b.m * mag
                c.vz += dz * b.m * mag
        for b in bodies:
            b.x += dt * b.vx
            b.y += dt * b.vy
            b.z += dt * b.vz


def energy(bodies: list[Body]) -> float:
    e = 0.0
    n = len(bodies)
    for i in range(n):
        b = bodies[i]
        e += 0.5 * b.m * (b.vx * b.vx + b.vy * b.vy + b.vz * b.vz)
        for j in range(i + 1, n):
            c = bodies[j]
            dx = b.x - c.x
            dy = b.y - c.y
            dz = b.z - c.z
            e -= b.m * c.m / (dx * dx + dy * dy + dz * dz) ** 0.5
    return e


PI = 3.14159265358979323
SM = 4.0 * PI * PI
DPY = 365.24
bodies = [
    Body(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, SM),
    Body(4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01,
         1.66007664274403694e-03 * DPY, 7.69901118419740425e-03 * DPY, -6.90460016972063023e-05 * DPY,
         9.54791938424326609e-04 * SM),
    Body(8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01,
         -2.76742510726862411e-03 * DPY, 4.99852801234917238e-03 * DPY, 2.30417297573763929e-05 * DPY,
         2.85885980666130812e-04 * SM),
    Body(1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
         2.96460137564761618e-03 * DPY, 2.37847173959480950e-03 * DPY, -2.96589568540237556e-05 * DPY,
         4.36624404335156298e-05 * SM),
    Body(1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
         2.68067772490389322e-03 * DPY, 1.62824170038242295e-03 * DPY, -9.51592254519715870e-05 * DPY,
         5.15138902046611451e-05 * SM),
]
px = 0.0
py = 0.0
pz = 0.0
for b in bodies:
    px += b.vx * b.m
    py += b.vy * b.m
    pz += b.vz * b.m
bodies[0].vx = -px / SM
bodies[0].vy = -py / SM
bodies[0].vz = -pz / SM
print(round(energy(bodies), 9))
advance(bodies, 0.01, 500_000)
print(round(energy(bodies), 9))
