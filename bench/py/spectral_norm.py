def a(i: int, j: int) -> float:
    return 1.0 / ((i + j) * (i + j + 1) // 2 + i + 1)


def mul_av(v: list[float], n: int) -> list[float]:
    out: list[float] = []
    for i in range(n):
        s = 0.0
        for j in range(n):
            s += a(i, j) * v[j]
        out.append(s)
    return out


def mul_atv(v: list[float], n: int) -> list[float]:
    out: list[float] = []
    for i in range(n):
        s = 0.0
        for j in range(n):
            s += a(j, i) * v[j]
        out.append(s)
    return out


def spectral(n: int) -> float:
    u = [1.0] * n
    v = [0.0] * n
    for _ in range(10):
        v = mul_atv(mul_av(u, n), n)
        u = mul_atv(mul_av(v, n), n)
    vbv = 0.0
    vv = 0.0
    for i in range(n):
        vbv += u[i] * v[i]
        vv += v[i] * v[i]
    return (vbv / vv) ** 0.5


print(round(spectral(400), 9))
