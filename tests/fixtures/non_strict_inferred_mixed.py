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
    xs = list(map(int, input().split()))
    print(solve(n, xs))

print(sorted(count(input().split()).items()))
show(3)
show("three")
empty = []
print(solve(2, empty))
