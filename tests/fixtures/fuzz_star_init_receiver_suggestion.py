# A compiled constructor with `*args` whose receiver is not spelled `self`,
# called with an unknown keyword: CPython 3.13 offers the receiver's own
# spelling as the "Did you mean" (`ths` -> `this`), never `self` (`slf`).
class C:
    def __init__(this, alpha: int, *rest) -> None:
        this.alpha = alpha
        this.rest = list(rest)


def attempt(kind: int) -> None:
    try:
        if kind == 0:
            c = C(1, 2, 3)
        elif kind == 1:
            c = C(alpha=1, ths=2)
        elif kind == 2:
            c = C(alpha=1, slf=2)
        elif kind == 3:
            c = C(alpa=1)
        elif kind == 4:
            c = C(1, alpha=2)
        else:
            c = C()
        print(c.alpha, c.rest)
    except TypeError as e:
        print("TypeError", e)


for kind in range(6):
    attempt(kind)
