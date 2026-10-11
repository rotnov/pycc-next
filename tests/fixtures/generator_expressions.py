from typing import Iterator


def count(n: int) -> Iterator[int]:
    i = 0
    while i < n:
        print("produce", i)
        yield i
        i += 1


def loud(x: int) -> bool:
    print("check", x)
    return x > 2


def main(xs: list[int], t: int) -> None:
    # `any`/`all` stop at the first deciding element.
    print(any(loud(x) for x in xs))
    print(all(not loud(x) for x in xs))
    print(any(x > t for x in count(10)))
    print(sum(x * y for x in xs for y in xs if x != y))
    print(next(x for x in xs if x > t))
    print(next((x for x in xs if x > 100), -1))
    print(max(len(w) for w in "a bb ccc".split()))
    print(any(i == j for i, j in enumerate(xs)))
    d = {"a": 1, "b": 2}
    print(sorted(k for k, v in d.items() if v > 1))
    # A generator expression kept and consumed later.
    g = (x * t for x in xs)
    print(next(g), next(g))
    for v in g:
        print("rest", v)
    r = (i * i for i in range(t, t + 3))
    print(list(r), list(r))
    print(",".join(str(a) + str(b) for a in xs for b in range(2)))
    print(sum(sum(y for y in range(x)) for x in xs))


main([3, 1, 4, 1, 5], 2)
total = 0
for k in (x * x for x in range(5)):
    total += k
print(total)
print(any(c.isdigit() for c in "ab3"), all(c.isalpha() for c in "ab3"))
lines = ["a 1", "b 22", "c 333"]
print(max(len(line.split()[1]) for line in lines))
