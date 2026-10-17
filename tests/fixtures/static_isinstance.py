def mat_pow(mat, init, k, mod):
    def mat_dot(a, b, mod):
        # A vector is a one-row matrix; a column vector is made of one-element rows.
        if not isinstance(a[0], list) and not isinstance(a[0], tuple):
            a = [a]
        if not isinstance(b[0], list):
            b = [[x] for x in b]
        n1 = len(a)
        n2 = len(a[0])
        m2 = len(b[0])
        res = [[0] * m2 for _ in range(n1)]
        for i in range(n1):
            for j in range(m2):
                for t in range(n2):
                    res[i][j] = (res[i][j] + a[i][t] * b[t][j]) % mod
        return res

    def power(m, e, mod):
        n = len(m)
        res = [[int(i == j) for j in range(n)] for i in range(n)]
        while e > 0:
            if e & 1:
                res = mat_dot(res, m, mod)
            m = mat_dot(m, m, mod)
            e >>= 1
        return res

    res = power(mat, k, mod)
    res = mat_dot(res, init, mod)
    return [row[0] for row in res]


def describe():
    def kind(v):
        if isinstance(v[0], (str, tuple)) or isinstance(v[0], int):
            return "scalar rows"
        return "nested rows"

    return kind([1, 2]) + "/" + kind([[1], [2]]) + "/" + kind(["a"]) + "/" + kind([(1, 2)])


def sizes():
    def size(v):
        return len(v)

    return size([1, 2, 3]) + size(["a"]) + size([[1], [2]])


def first(xs):
    def head(v):
        if isinstance(v[0], list):
            return len(v[0])
        return -1

    return head(xs)


def shout(n):
    n = str(n)
    return n + "!"


def total(values, scale):
    scale = float(scale)
    return sum(values) * scale


fib = [[1, 1], [1, 0]]
print(mat_pow(fib, [1, 0], 10, 10 ** 9 + 7))
print(mat_pow(fib, [1, 0], 90, 10 ** 9 + 7))
print(mat_pow([[2, 0], [0, 3]], [1, 1], 5, 1000))
print(describe())
print(sizes(), first([[4, 5, 6]]), shout(42), shout(-1), total([1, 2, 3], 2))
try:
    print(first([]))
except IndexError as e:
    print("IndexError", e)
