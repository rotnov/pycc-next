# #1395: the conditional expression `body if test else orelse`
# (docs/TYPE_SYSTEM.md, "Conditional expressions"). The condition is
# evaluated once, for its truth, then exactly one branch; the node yields
# that branch's value, typed by the branch join. Optional-typed results are
# observed only inside an `is not None` branch: printing an Optional
# directly is not supported yet.


class Box:
    def __init__(self, n: int) -> None:
        self.n = n


def say_int(label: str, n: int) -> int:
    print("eval", label)
    return n


def say_str(label: str, s: str) -> str:
    print("eval", label)
    return s


def say_bool(label: str, b: bool) -> bool:
    print("eval", label)
    return b


def maybe(n: int) -> int | None:
    r: int | None = None
    if n >= 0:
        r = n
    return r


def pick(flag: bool, n: int) -> int | None:
    return n if flag else None


def sign(n: int) -> str:
    return "negative" if n < 0 else "zero" if n == 0 else "positive"


def scalars(flag: bool, n: int, x: float, s: str) -> None:
    print(n if flag else -n, x if flag else -x, s if flag else s + s)
    print(flag if n else not flag, 1 if s else 0, "x" if x else "no x")
    print(sign(-3), sign(0), sign(8))


def one_branch_runs(flag: bool) -> None:
    print(say_int("body", 1) if say_bool("test", flag) else say_int("orelse", 2))
    print(say_str("body", "b") if not flag else say_str("orelse", "o"))


def containers_and_instances(flag: bool) -> None:
    xs = [1, 2, 3] if flag else [4]
    print(len(xs), xs[0])
    pair = (1, 1.5) if flag else (2, 2.5)
    print(pair[0], pair[1])
    b = Box(10) if flag else Box(20)
    print(b.n, (Box(1) if flag else Box(2)).n)


def optional_join(flag: bool, n: int) -> None:
    v = pick(flag, n)
    if v is not None:
        print("present", v)
    else:
        print("absent")
    w = maybe(n) if flag else 7
    if w is not None:
        print("w", w)
    u = None if flag else n
    print(u is None)


def conditions(n: int, s: str, o: int | None, x: float) -> None:
    print("n" if n else "no n", "s" if s else "no s", "o" if o else "no o", "x" if x else "no x")
    if (1 if (k := n) else 0):
        print("walrus", k)


def bigints(rounds: int) -> None:
    big = 4611686018427387904
    total = 0
    for i in range(rounds):
        x = big + i if i % 2 == 0 else big
        total = total + (x if i % 3 == 0 else 1)
    print(total)


def run(flag: bool) -> None:
    scalars(flag, 3, 1.5, "s")
    one_branch_runs(flag)
    containers_and_instances(flag)
    optional_join(flag, 4)
    optional_join(flag, -1)


run(True)
run(False)
conditions(0, "", maybe(-1), 0.0)
conditions(5, "s", maybe(0), 2.5)
conditions(-1, "t", maybe(3), -0.5)
bigints(50)
