# Found by the differential fuzzer (fuzz/): an `--ext --non-strict` module
# published a compiled exception class to Python without its own members, so
# `SubErr(n)` built by Python code skipped `SubErr.__init__` (no `code`
# attribute, the wrong message) and a caught `E` had no `hello` method.
class Err(Exception):
    pass


class SubErr(Err):
    def __init__(self, code: int) -> None:
        super().__init__(f"code {code}")
        self.code = code


class E(Exception):
    level: int = 3

    def hello(self) -> str:
        return "hi"


def risky(n: int) -> int:
    if n < 0:
        raise SubErr(n)
    return n * 2


def boom(n: int) -> int:
    if n > 0:
        raise E("n=" + str(n))
    return n


try:
    print(risky(-5))
except SubErr as e:
    print("sub", e.code, e, e.args, repr(e))
try:
    risky(-3)
except (SubErr, ValueError) as e:
    print("tuple", str(e))
