# A set of str iterates in its hash table's order, which CPython keys with
# PYTHONHASHSEED: under the same seed pycc prints the same order.


def main() -> None:
    words: set[str] = set()
    for i in range(12):
        words.add("w" + str(i * 7))
    print(words)
    print(frozenset({"x" + str(i) for i in range(6)}))
    print({("k" + str(i), i) for i in range(5)})
    print({"été" + str(i) for i in range(3)}, {"中" + str(i) for i in range(3)})


main()
