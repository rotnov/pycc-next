import itertools
from itertools import count, islice, chain, product, permutations, combinations, accumulate, groupby, pairwise


class P:
    def __init__(self, name: str) -> None:
        self.name = name


def noisy(n: int):
    for i in range(n):
        print("produce", i)
        yield i


def main() -> None:
    e: list[int] = []
    # laziness: islice pulls only what it needs
    g = noisy(10)
    for x in islice(g, 1, 4):
        print("got", x)
    print("next after islice:", next(g))
    # chain of a generator and a list, lazily
    for x in chain(noisy(2), [7, 8]):
        print("chain", x)
    # infinite count with break
    for i in count(5, -2):
        if i < 0:
            break
        print("count", i)
    # bigints
    big = 2 ** 70
    print(list(islice(count(big, big), 3)))
    for v in islice(accumulate(count(big)), 3):
        print(v)
    # tuples of mixed kinds
    pairs = [(1, "a"), (2, "b")]
    print(list(product(pairs, [True, False])))
    print(list(permutations([1.5, 2.5])))
    print(list(combinations([(1, 2), (3, 4), (5, 6)], 2)))
    print(list(pairwise([True, False, True])))
    print(list(chain([[1], [2, 3]], [[4]])))
    # instances
    ps = [P("x"), P("y"), P("z")]
    for a, b in permutations(ps, 2):
        print(a.name + b.name, end=" ")
    print()
    # early exit closes the iterator
    for p in permutations(range(8)):
        if p[0] == 1:
            print("first with 1:", p)
            break
    # empty and degenerate cases
    print(list(permutations([1, 2], 3)))
    print(list(combinations([1, 2, 3], 3)))
    print(list(itertools.combinations_with_replacement(e, 2)))
    print(list(product(e, [1, 2])))
    print(list(groupby(e)))
    print(list(pairwise([1])))
    print(list(accumulate(e)))
    print(list(islice([1, 2, 3], 5, 10)))
    print(list(islice([1, 2, 3, 4, 5, 6], 0, None, 2)))
    # groupby with a key and a named function
    data = [("a", 1), ("a", 2), ("b", 3), ("a", 4)]
    print([(k, [v for _, v in grp]) for k, grp in groupby(data, key=lambda t: t[0])])
    print([(k, list(grp)) for k, grp in groupby([1, -1, 2, -3, -4], key=abs_sign)])
    # accumulate variants
    print(list(accumulate([5, 3, 8, 1], min)))
    print(list(accumulate([1, 2, 3], initial=100)))
    print(list(accumulate(["x", "y"], initial="w")))
    print(list(accumulate([6, 3, 5], lambda a, b: a ^ b)))
    # consumers
    print(sum(x * y for x, y in product(range(4), range(4))))
    print(max(combinations([3, 1, 4, 1, 5], 2)))
    print(sorted(set(itertools.permutations("aab"))))
    print(any(c[0] + c[1] == 9 for c in combinations(range(6), 2)))
    print(sorted({a + b for a, b in itertools.product("ab", "cd")}))
    d = {k: len(list(v)) for k, v in groupby(sorted("mississippi"))}
    print(d)
    # repr of a native iterator names its type (not compared: address)
    print(str(permutations([1])).startswith("<itertools.permutations object at 0x"))
    # errors
    try:
        islice([1], -2)
    except ValueError as err:
        print("ValueError:", err)
    try:
        islice([1], 0, 3, 0)
    except ValueError as err:
        print("ValueError:", err)
    it = count(0)
    print(next(it), next(it))


def abs_sign(x: int) -> bool:
    return x > 0


main()
