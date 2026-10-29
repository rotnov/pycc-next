# v0.10: `sorted` makes a list of its whole input before it calls the key,
# so an input that raises part way (`map(int, ...)` yields 0, then raises
# `ValueError`) raises before the first key call (`1 // 0`).


def by_scale(texts, scale):
    scale = scale + 0

    def key(v):
        return scale // v

    try:
        return sorted(map(int, texts), key=key)
    except ValueError:
        return "ValueError"
    except ZeroDivisionError:
        return "ZeroDivisionError"


def by_lambda(texts):
    try:
        return sorted(map(int, texts), key=lambda v: 1 // v)
    except ValueError:
        return "ValueError"
    except ZeroDivisionError:
        return "ZeroDivisionError"


print(by_scale(["0", "x"], 1))
print(by_lambda(["0", "x"]))
print(by_scale(["3", "1", "2"], 6))
print(by_lambda(["0"]))
