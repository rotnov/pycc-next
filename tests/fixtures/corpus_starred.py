n, *rest = map(int, input().split())
print(n, rest)
first, *mid, last = [1, 2, 3, 4, 5]
print(first, mid, last)
*init, tail = "abc"
print(init, tail)
a, b, *c = [7, 8]
print(a, b, c)
head, *others = [9]
print(head, others)


def split(xs: list[int]) -> int:
    x, *ys = xs
    return x + len(ys)


print(split([4, 5, 6]))
try:
    p, q, *r = [1]
except ValueError as e:
    print("ValueError", e)
u, *v, w = [0][:0]
