import itertools
from itertools import permutations, combinations, product, islice, count, chain, accumulate, groupby, pairwise
import operator


def main() -> None:
    xs = [1, 2, 3]
    print(list(itertools.permutations(xs, 2)))
    print(list(permutations([1, 2, 3])))
    print(list(permutations("abc", 2)))
    print(list(combinations(xs, 2)))
    print(list(itertools.combinations_with_replacement(xs, 2)))
    print(list(product(xs, "ab")))
    print(list(product([0, 1], repeat=3)))
    print(list(islice(count(), 5)))
    print(list(islice(count(10, 3), 2, 8, 2)))
    print(list(islice(xs, None)))
    print(list(chain(xs, [4, 5], xs)))
    print(list(chain.from_iterable([[1, 2], [3], []])))
    print(list(chain.from_iterable(["ab", "cd"])))
    print(list(accumulate(xs)))
    print(list(accumulate(xs, operator.mul)))
    print(list(accumulate([3, 1, 4, 1, 5], max)))
    print(list(accumulate(xs, lambda a, b: a * b, initial=10)))
    print(list(accumulate([1.5, 2.5])))
    print(list(accumulate(["a", "b", "c"])))
    print(list(pairwise("abcd")))
    print(list(pairwise(xs)))
    for k, g in groupby("aaabccdd"):
        print(k, list(g))
    print([(k, len(list(g))) for k, g in groupby([1, 1, 2, 3, 3, 3])])
    words = ["apple", "avocado", "banana", "blueberry", "cherry"]
    for k, g in groupby(words, key=lambda w: w[0]):
        print(k, list(g))
    total = 0
    for a, b, c in product(range(3), repeat=3):
        total += a * 9 + b * 3 + c
    print(total)
    print(sum(1 for _ in combinations(range(10), 3)))
    print(sorted(set(permutations([1, 1, 2]))))
    g = count(1)
    print(next(g), next(g), next(g))
    print(list(islice(count(0.5, 0.25), 4)))
    try:
        print(list(islice(xs, -1)))
    except ValueError as e:
        print("ValueError", e)


main()
