def main() -> None:
    pi = 3.14159265358979323
    solar_mass = 4.0 * pi * pi
    days_per_year = 365.24

    # Position (x, y, z), velocity (vx, vy, vz), mass -- one variable each,
    # per body, since v0.1/early-v0.2 has no list/tuple to group them yet
    # (PR-10/11's job). Values are pyperformance's bm_nbody constants
    # verbatim (jupiter/saturn/uranus/neptune; sun's own velocity is fixed
    # up after the fact to offset momentum, exactly like the original).
    #
    # Deviation from the plan brief: negative literals (e.g. `-1.16...`)
    # and the `-1.5` exponent are written here as `0.0 - <value>`. Unary
    # operators (`Expr::UnaryOp` / `USub`/`UAdd`/`Not`/`Invert`) are not
    # lowered by the implemented HIR subset (verified empirically: `x =
    # -1.5` produces a spanned C0001 "expression kind not supported yet"
    # diagnostic), matching the same rewrite already applied to the
    # mandelbrot fixture in tests/slice1_codegen_depth.rs. `0.0 - x` is
    # bit-for-bit identical to `-x` in IEEE-754 (subtracting an exact
    # value from 0.0 only flips the sign bit), so this is exactly
    # semantically equivalent to the original, not an approximation. At
    # several sites (e.g. `jupiter_vz`, `sun_vx`/`vy`/`vz`) the value being
    # negated is itself a product or a sum of products rather than a bare
    # literal; that is still exact, because IEEE-754 multiplication's sign
    # is the XOR of its operands' signs, so negating one factor negates the
    # product with no rounding difference (`(-a) * b == -(a * b)` exactly),
    # and negating a sum is the sum of the negated terms.
    #
    # Also deviation from the plan brief: every local variable below
    # dropped its `: float`/`: int` annotation. Verified empirically that
    # `pycc_hir` handles `Stmt::Assign` but has no `Stmt::AnnAssign` case
    # (`x: float = 1.0` also produces a spanned C0001), matching the PR-9
    # scope note in
    # `docs/decisions/D-088-correct-v0-2-s-acceptance-criteria-before-any-v0.md`.
    # Local annotations are inert under CPython for function-local variables
    # (no `__annotations__` entry is created), so dropping them changes
    # nothing observable.
    sun_x = 0.0
    sun_y = 0.0
    sun_z = 0.0
    sun_vx = 0.0
    sun_vy = 0.0
    sun_vz = 0.0
    sun_mass = solar_mass

    jupiter_x = 4.84143144246472090
    jupiter_y = 0.0 - 1.16032004402742839
    jupiter_z = 0.0 - 1.03622044471123109e-01
    jupiter_vx = 1.66007664274403694e-03 * days_per_year
    jupiter_vy = 7.69901118419740425e-03 * days_per_year
    jupiter_vz = 0.0 - 6.90460016972063023e-05 * days_per_year
    jupiter_mass = 9.54791938424326609e-04 * solar_mass

    saturn_x = 8.34336671824457987
    saturn_y = 4.12479856412430479
    saturn_z = 0.0 - 4.03523417114321381e-01
    saturn_vx = 0.0 - 2.76742510726862411e-03 * days_per_year
    saturn_vy = 4.99852801234917238e-03 * days_per_year
    saturn_vz = 2.30417297573763929e-05 * days_per_year
    saturn_mass = 2.85885980666130812e-04 * solar_mass

    uranus_x = 1.28943695621391310e01
    uranus_y = 0.0 - 1.51111514016986312e01
    uranus_z = 0.0 - 2.23307578892655734e-01
    uranus_vx = 2.96460137564761618e-03 * days_per_year
    uranus_vy = 2.37847173959480950e-03 * days_per_year
    uranus_vz = 0.0 - 2.96589568540237556e-05 * days_per_year
    uranus_mass = 4.36624404335156298e-05 * solar_mass

    neptune_x = 1.53796971148509165e01
    neptune_y = 0.0 - 2.59193146099879641e01
    neptune_z = 1.79258772950371181e-01
    neptune_vx = 2.68067772490389322e-03 * days_per_year
    neptune_vy = 1.62824170038242295e-03 * days_per_year
    neptune_vz = 0.0 - 9.51592254519715870e-05 * days_per_year
    neptune_mass = 5.15138902046611451e-05 * solar_mass

    # offset_momentum: sun's velocity absorbs the system's total momentum,
    # exactly like pyperformance's own offset_momentum(SYSTEM[0], *SYSTEM[1:]).
    sun_vx = 0.0 - (
        jupiter_vx * jupiter_mass
        + saturn_vx * saturn_mass
        + uranus_vx * uranus_mass
        + neptune_vx * neptune_mass
    ) / solar_mass
    sun_vy = 0.0 - (
        jupiter_vy * jupiter_mass
        + saturn_vy * saturn_mass
        + uranus_vy * uranus_mass
        + neptune_vy * neptune_mass
    ) / solar_mass
    sun_vz = 0.0 - (
        jupiter_vz * jupiter_mass
        + saturn_vz * saturn_mass
        + uranus_vz * uranus_mass
        + neptune_vz * neptune_mass
    ) / solar_mass

    dt = 0.01
    # D-093: raised from upstream pyperformance's own `DEFAULT_ITERATIONS =
    # 20000` to amortize the same-machine paired benchmark's fixed
    # per-process overhead (`tests/nbody_bench.rs`) -- at 20000, that fixed
    # cost was a large enough fraction of each side's own tiny total runtime
    # to mechanically compress the measured wall-clock ratio well below the
    # actual compute-only speedup. Physics, constants, and update order are
    # completely unaffected by this loop's own trip count -- see D-093 for
    # the full measurement.
    iterations = 525000
    step = 0
    while step < iterations:
        # Pairwise gravitational update -- 10 pairs for 5 bodies, unrolled
        # (no list/enumerate/itertools.combinations available yet).
        # Pair: sun/jupiter
        dx = sun_x - jupiter_x
        dy = sun_y - jupiter_y
        dz = sun_z - jupiter_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        sun_vx = sun_vx - dx * jupiter_mass * mag
        sun_vy = sun_vy - dy * jupiter_mass * mag
        sun_vz = sun_vz - dz * jupiter_mass * mag
        jupiter_vx = jupiter_vx + dx * sun_mass * mag
        jupiter_vy = jupiter_vy + dy * sun_mass * mag
        jupiter_vz = jupiter_vz + dz * sun_mass * mag

        # Pair: sun/saturn
        dx = sun_x - saturn_x
        dy = sun_y - saturn_y
        dz = sun_z - saturn_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        sun_vx = sun_vx - dx * saturn_mass * mag
        sun_vy = sun_vy - dy * saturn_mass * mag
        sun_vz = sun_vz - dz * saturn_mass * mag
        saturn_vx = saturn_vx + dx * sun_mass * mag
        saturn_vy = saturn_vy + dy * sun_mass * mag
        saturn_vz = saturn_vz + dz * sun_mass * mag

        # Pair: sun/uranus
        dx = sun_x - uranus_x
        dy = sun_y - uranus_y
        dz = sun_z - uranus_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        sun_vx = sun_vx - dx * uranus_mass * mag
        sun_vy = sun_vy - dy * uranus_mass * mag
        sun_vz = sun_vz - dz * uranus_mass * mag
        uranus_vx = uranus_vx + dx * sun_mass * mag
        uranus_vy = uranus_vy + dy * sun_mass * mag
        uranus_vz = uranus_vz + dz * sun_mass * mag

        # Pair: sun/neptune
        dx = sun_x - neptune_x
        dy = sun_y - neptune_y
        dz = sun_z - neptune_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        sun_vx = sun_vx - dx * neptune_mass * mag
        sun_vy = sun_vy - dy * neptune_mass * mag
        sun_vz = sun_vz - dz * neptune_mass * mag
        neptune_vx = neptune_vx + dx * sun_mass * mag
        neptune_vy = neptune_vy + dy * sun_mass * mag
        neptune_vz = neptune_vz + dz * sun_mass * mag

        # Pair: jupiter/saturn
        dx = jupiter_x - saturn_x
        dy = jupiter_y - saturn_y
        dz = jupiter_z - saturn_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        jupiter_vx = jupiter_vx - dx * saturn_mass * mag
        jupiter_vy = jupiter_vy - dy * saturn_mass * mag
        jupiter_vz = jupiter_vz - dz * saturn_mass * mag
        saturn_vx = saturn_vx + dx * jupiter_mass * mag
        saturn_vy = saturn_vy + dy * jupiter_mass * mag
        saturn_vz = saturn_vz + dz * jupiter_mass * mag

        # Pair: jupiter/uranus
        dx = jupiter_x - uranus_x
        dy = jupiter_y - uranus_y
        dz = jupiter_z - uranus_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        jupiter_vx = jupiter_vx - dx * uranus_mass * mag
        jupiter_vy = jupiter_vy - dy * uranus_mass * mag
        jupiter_vz = jupiter_vz - dz * uranus_mass * mag
        uranus_vx = uranus_vx + dx * jupiter_mass * mag
        uranus_vy = uranus_vy + dy * jupiter_mass * mag
        uranus_vz = uranus_vz + dz * jupiter_mass * mag

        # Pair: jupiter/neptune
        dx = jupiter_x - neptune_x
        dy = jupiter_y - neptune_y
        dz = jupiter_z - neptune_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        jupiter_vx = jupiter_vx - dx * neptune_mass * mag
        jupiter_vy = jupiter_vy - dy * neptune_mass * mag
        jupiter_vz = jupiter_vz - dz * neptune_mass * mag
        neptune_vx = neptune_vx + dx * jupiter_mass * mag
        neptune_vy = neptune_vy + dy * jupiter_mass * mag
        neptune_vz = neptune_vz + dz * jupiter_mass * mag

        # Pair: saturn/uranus
        dx = saturn_x - uranus_x
        dy = saturn_y - uranus_y
        dz = saturn_z - uranus_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        saturn_vx = saturn_vx - dx * uranus_mass * mag
        saturn_vy = saturn_vy - dy * uranus_mass * mag
        saturn_vz = saturn_vz - dz * uranus_mass * mag
        uranus_vx = uranus_vx + dx * saturn_mass * mag
        uranus_vy = uranus_vy + dy * saturn_mass * mag
        uranus_vz = uranus_vz + dz * saturn_mass * mag

        # Pair: saturn/neptune
        dx = saturn_x - neptune_x
        dy = saturn_y - neptune_y
        dz = saturn_z - neptune_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        saturn_vx = saturn_vx - dx * neptune_mass * mag
        saturn_vy = saturn_vy - dy * neptune_mass * mag
        saturn_vz = saturn_vz - dz * neptune_mass * mag
        neptune_vx = neptune_vx + dx * saturn_mass * mag
        neptune_vy = neptune_vy + dy * saturn_mass * mag
        neptune_vz = neptune_vz + dz * saturn_mass * mag

        # Pair: uranus/neptune
        dx = uranus_x - neptune_x
        dy = uranus_y - neptune_y
        dz = uranus_z - neptune_z
        d2 = dx * dx + dy * dy + dz * dz
        mag = dt * (d2 ** (0.0 - 1.5))
        uranus_vx = uranus_vx - dx * neptune_mass * mag
        uranus_vy = uranus_vy - dy * neptune_mass * mag
        uranus_vz = uranus_vz - dz * neptune_mass * mag
        neptune_vx = neptune_vx + dx * uranus_mass * mag
        neptune_vy = neptune_vy + dy * uranus_mass * mag
        neptune_vz = neptune_vz + dz * uranus_mass * mag

        # Position update for all 5 bodies.
        sun_x = sun_x + dt * sun_vx
        sun_y = sun_y + dt * sun_vy
        sun_z = sun_z + dt * sun_vz
        jupiter_x = jupiter_x + dt * jupiter_vx
        jupiter_y = jupiter_y + dt * jupiter_vy
        jupiter_z = jupiter_z + dt * jupiter_vz
        saturn_x = saturn_x + dt * saturn_vx
        saturn_y = saturn_y + dt * saturn_vy
        saturn_z = saturn_z + dt * saturn_vz
        uranus_x = uranus_x + dt * uranus_vx
        uranus_y = uranus_y + dt * uranus_vy
        uranus_z = uranus_z + dt * uranus_vz
        neptune_x = neptune_x + dt * neptune_vx
        neptune_y = neptune_y + dt * neptune_vy
        neptune_z = neptune_z + dt * neptune_vz

        step = step + 1

    print(sun_x, sun_y, sun_z)

main()
