# v0.10: where the program binds `list` (here a local), the `sorted` rewrite
# that keys a copy through `list()` is not made: the call runs as written,
# keying a copy of its input as CPython does.


def shadowing(xs):
    list = 0

    def key(v):
        xs[1] = list
        return v

    return sorted(xs, key=key)


c = [2, 1]
print(shadowing(c), c)
