# v0.10: `sorted(xs, key=f, reverse=r())` evaluates every argument before
# calling the key: `reverse` runs first, and when it raises no key is
# called (a nested function used as the key included); and `sorted` keys a
# copy of its input, which a key changing the list does not change.


def rev(flag):
    print("reverse", flag)
    if flag is None:
        raise ValueError("no reverse")
    return flag


def run(xs, base, flag):
    base = base + 0

    def key(x):
        print("key", x)
        return x * base

    return sorted(xs, key=key, reverse=rev(flag))


def mutating(xs):
    def key(v):
        xs[1] = 0
        return v

    return sorted(xs, key=key), xs


def mutating_reversed(xs):
    def key(v):
        xs.append(v)
        return -v

    return sorted(xs, key=key, reverse=True), xs


print(mutating([2, 1]))
print(mutating_reversed([1, 3, 2]))
print(run([3, 1, 2], 1, True))
print(run([3, 1, 2], -1, False))
try:
    print(run([3, 1, 2], 1, None))
except ValueError as e:
    print("ValueError", e)
