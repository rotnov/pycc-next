class P:
    def __init__(self, v: int) -> None:
        self.v = v


def show(ps: list[P]) -> str:
    return ",".join([str(p.v) for p in ps])


words = ["ab", "cd"]
keep = words
words *= 3
print(keep)
words *= 0
print(keep, len(words))
words = ["x"]
words *= -3
print(words)
ps = [P(1), P(2)]
alias = ps
ps *= 3
print(show(alias))
ps *= 1
print(show(ps))
ps *= -1
print(len(alias))
empty: list[int] = []
empty *= 2**62
print(empty)
flags = [True, False]
flags *= 2
print(flags)


def attempt(n: int) -> None:
    big = [1, 2]
    try:
        big *= n
        print(len(big))
    except MemoryError as e:
        print("MemoryError", e, big)
    except OverflowError as e:
        print("OverflowError", e, big)


attempt(2**62)
attempt(2**63)
attempt(-(2**70))
attempt(2)
