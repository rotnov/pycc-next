# v0.10: `sorted(xs, key=f, reverse=r())` evaluates every argument before
# calling the key: `reverse` runs first, and when it raises no key is
# called (a nested function used as the key included).


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


print(run([3, 1, 2], 1, True))
print(run([3, 1, 2], -1, False))
try:
    print(run([3, 1, 2], 1, None))
except ValueError as e:
    print("ValueError", e)
