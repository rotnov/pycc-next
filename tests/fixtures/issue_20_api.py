# Issue #20, `--ext`: the module's fallback code reaches its compiled
# globals by a string (`eval`, `exec`, `globals()`), and `issue_20_driver.py`
# calls it and reads the globals back as module attributes.
class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def norm1(self) -> int:
        return abs(self.x) + abs(self.y)


p = Point(3, -4)
count = 5
label = "hi"
_hidden = [1, 2]


def _scaled(a: int) -> int:
    return a * 10


def read_count() -> int:
    return count


def read_point() -> int:
    return p.norm1()


def read_hidden() -> int:
    return len(_hidden)


def reads():
    print(eval("p").x, eval("p.norm1()"), eval("count + 1"), eval("label"), eval("_hidden"))
    print(eval("_scaled(2)"), eval("Point(1, 2).norm1()"), globals()["count"])
    exec("print('exec sees', p.y, count, _hidden)")


def writes():
    globals()["count"] = 7
    exec("global label\nlabel = 'rebound'")
    globals()["p"] = Point(10, 20)
    exec("global _hidden\n_hidden = [4, 5, 6]")
    return read_count(), read_point(), read_hidden()


reads()
