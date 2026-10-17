def fib_mod(k: int, mod: int) -> int:
    def dot(a, b, mod):
        if not isinstance(b[0], list):
            b = [[x] for x in b]
        out = []
        for row in a:
            line = []
            for j in range(len(b[0])):
                total = 0
                for t in range(len(b)):
                    total = (total + row[t] * b[t][j]) % mod
                line.append(total)
            out.append(line)
        return out

    m = [[1, 1], [1, 0]]
    r = [[1, 0], [0, 1]]
    while k > 0:
        if k & 1:
            r = dot(r, m, mod)
        m = dot(m, m, mod)
        k >>= 1
    return dot(r, [1, 0], mod)[1][0]


def shapes(n: int) -> str:
    def kind(v):
        if isinstance(v[0], (list, tuple)):
            return "rows from " + str(v[0][0])
        return "flat"

    return kind([n]) + ", " + kind([[n, n]]) + ", " + kind(["s"]) + ", " + kind([(n, n, n)])


def head(n: int) -> int:
    def first(v):
        if isinstance(v[0], int):
            return v[0]
        return -1

    return first(list(range(n)))


print(fib_mod(10, 1000), fib_mod(100, 10 ** 9 + 7), shapes(3), head(4))
try:
    print(head(0))
except IndexError as e:
    print("IndexError", e)
