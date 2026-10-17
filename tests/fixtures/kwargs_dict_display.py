def opts(name: str, **kwargs: int) -> str:
    return name + ":" + ",".join(f"{k}={v}" for k, v in sorted(kwargs.items()))


def kw(a: int, b: int = 3, *, c: str = "c") -> str:
    return f"{a}-{b}-{c}"


trace: list[str] = []


def note(tag: str, value: int) -> int:
    trace.append(tag)
    return value


print(opts(**{"name": "c", "z": 3}))
print(opts("d", **{}), opts("e", **{"y": 2, "x": 1}))
print(kw(**{"a": 1}), kw(4, **{"c": "q"}), kw(**{"b": 7, "a": 8}, c="r"))
print(opts("f", **{"p": note("p", 5), "q": note("q", 6)}, r=note("r", 7)))
print(trace)
try:
    print(kw(1, **{"a": 2}))
except TypeError as e:
    print("TypeError", e)
