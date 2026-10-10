import sys

input = sys.stdin.readline


def solve(n, xs):
    best = xs[0]
    for x in xs:
        if x > best:
            best = x
    return best * n


def count(words):
    d = {}
    for w in words:
        try:
            d[w] += 1
        except KeyError:
            d[w] = 1
    return d


def show(value):
    # `value` is only printed: the solver cannot pin it, so it is an object.
    print("value:", value)


pairs = []
for i in range(4):
    pairs.append([(i * 7) % 5, i])
pairs.sort()
print(pairs)

t = int(input())
for _ in range(t):
    n = int(input())
    xs = [int(w) for w in input().split()]
    print(solve(n, xs))

for k, v in count(input().split()).items():
    print(k, v)
show(3)
