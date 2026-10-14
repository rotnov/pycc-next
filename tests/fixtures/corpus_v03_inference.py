# v0.3 corpus round: inference the corpus needed in strict mode.


def ncr(n, r):
    # `r` is typed only by `a.count(m - 1)`: the result of an operator over
    # concrete operands is known before the method call is typed.
    return fact[n] // (fact[r] * fact[n - r])


fact = [1]
for i in range(1, 12):
    fact.append(fact[-1] * i)


def split_digits(x, k, s):
    # `result` is typed from `x % num`, which reads unannotated parameters.
    result = []
    for num in reversed(s):
        m = x % num
        x //= num
        result.append(m * k)
    out = ""
    while result:
        out += str(result.pop()) + " "
    return out


def size(grid, i):
    if i >= len(grid):
        return 0
    return grid[i] + size(grid, i + 1)


def never_called(a, b):
    # Dead and unannotated: nothing types it, nothing runs it.
    return [a] * b


def also_dead():
    return never_called(1, 2)


for nt in range(3):
    a = [nt, nt + 1, nt + 1, nt + 2, nt + 2, nt + 2]
    m = max(a)
    c = a.count(m - 1)
    print(ncr(len(a), c), c)

print(split_digits(1000, 2, [3, 5, 7]))
print(split_digits(0, 1, []))

# A module-level list typed by an unannotated function's return.
sizes = []
for j in range(3):
    sizes.append(size([1, 2, 3, 4], j))
print(sorted(sizes, reverse=True))
empty = []
print(empty, len(empty))
rebound = []
for j in range(2):
    rebound = []
print(len(rebound), rebound == [], not rebound)


def reset():
    xs = []
    xs = []
    print(xs)


reset()


def countdown(n):
    # The empty list's element type comes from the recursive call's.
    if n == 0:
        return []
    return [n] + countdown(n - 1)


print(countdown(3), countdown(0))


def lim(s):
    # A returned tuple display: typed from the body once `s` is.
    up, down = 0, 0
    now = 0
    for i in s:
        now += i
        up = max(up, now)
        down = min(down, now)
    return up, down


def span(a):
    # Typed from `lim`'s return, once that is.
    return a[0] - a[1] + 1


def egcd(a, b):
    if a == 0:
        return (b, 0, 1)
    g, y, x = egcd(b % a, a)
    return (g, x - (b // a) * y, y)


print(span(lim([1, -2, 3])), lim([]), egcd(30, 12), egcd(0, 5))
