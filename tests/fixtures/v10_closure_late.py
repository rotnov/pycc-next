# v0.10: a nested function used as a value whose capture is rebound after
# its `def` reads the value the variable holds when it runs (non-strict: the
# function stays a CPython closure over a cell).


def late(items):
    # `suffix` is rebound after the `def`: the closure must read the value
    # it holds when it runs.
    suffix = "a"

    def tag(item: str) -> str:
        return item + suffix

    first = list(map(tag, items))
    suffix = "b"
    return first + list(map(tag, items))


print(late(["x", "y"]))
print(late([]))
