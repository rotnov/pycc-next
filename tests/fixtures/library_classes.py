# Classes from libraries (v0.7 accept 2): classes inheriting CPython
# classes, metaclasses, descriptors, class attributes of any type,
# decorated methods, library annotations, unions, `with` and starred
# expressions. Oracle: CPython runs this file unchanged.
import argparse
import collections
import contextlib
import functools
import re
import threading


class Worker(threading.Thread):
    def __init__(self, n: int) -> None:
        super().__init__()
        self.n = n
        self.result = 0

    def run(self) -> None:
        s = 0
        for i in range(self.n):
            s += i * i
        self.result = s


class Tally(collections.Counter):
    def top(self, k: int):
        return [key for key, _ in self.most_common(k)]


class AppError(ValueError):
    def __init__(self, code: int, detail: str) -> None:
        super().__init__(detail)
        self.code = code

    def __str__(self) -> str:
        return "E" + str(self.code) + ": " + self.args[0]


registry = []


class Registered(type):
    def __new__(mcs, name, bases, namespace):
        cls = super().__new__(mcs, name, bases, namespace)
        registry.append(name)
        return cls


class Plugin(metaclass=Registered):
    kinds = {"a": 1, "b": 2}
    pattern = re.compile(r"[a-z]+")

    def __init_subclass__(cls, **kwargs):
        super().__init_subclass__(**kwargs)
        cls.tag = cls.__name__.lower()

    def words(self, text: str) -> list:
        return self.pattern.findall(text)


class Upper(Plugin):
    def words(self, text: str) -> list:
        return [w.upper() for w in super().words(text)]


class Positive:
    def __set_name__(self, owner, name):
        self.name = "_" + name

    def __get__(self, obj, objtype=None):
        if obj is None:
            return self
        return getattr(obj, self.name)

    def __set__(self, obj, value):
        if value < 0:
            raise ValueError("negative " + self.name[1:])
        setattr(obj, self.name, value)


def traced(f):
    @functools.wraps(f)
    def wrapper(*args, **kwargs):
        return "<" + str(f(*args, **kwargs)) + ">"
    return wrapper


class Account:
    balance = Positive()
    count: int = 0

    def __init__(self, owner: str, balance: int) -> None:
        self.owner = owner
        self.balance = balance
        Account.count += 1

    @property
    def label(self) -> str:
        return self.owner + "#" + str(self.balance)

    @label.setter
    def label(self, value: str) -> None:
        self.owner = value.split("#")[0]

    @functools.cached_property
    def digits(self) -> int:
        return len(str(self.balance))

    @traced
    def deposit(self, amount: int) -> int:
        self.balance += amount
        return self.balance

    @staticmethod
    def fee(amount: int | float) -> int | float:
        return amount / 100 if isinstance(amount, float) else amount // 100

    @classmethod
    def opened(cls) -> int:
        return cls.count

    def merge(self, other: "Account | None") -> int:
        if other is None:
            return self.balance
        return self.balance + other.balance


class Collect(argparse.Action):
    def __call__(self, parser, namespace, values, option_string=None):
        items = getattr(namespace, self.dest) or []
        items.append(values.upper())
        setattr(namespace, self.dest, items)


class Session:
    def __init__(self, log: list) -> None:
        self.log = log

    def __enter__(self):
        self.log.append("enter")
        return self

    def __exit__(self, kind, value, tb):
        self.log.append("exit " + (kind.__name__ if kind else "ok"))
        return kind is ZeroDivisionError


@contextlib.contextmanager
def scoped(log: list, name: str):
    log.append("open " + name)
    try:
        yield name.upper()
    finally:
        log.append("close " + name)


def use_with(log: list, n: int) -> str:
    with Session(log) as s, scoped(log, "x") as tag:
        s.log.append(tag)
        if n == 0:
            return str(1 // n)
    with Session(log):
        log.append("zero " + str(1 // n if n else 1 // n))
    return "done"


def starred(*args: int, **kwargs: int):
    first, *rest = args
    head, *middle, tail = [*args, *rest]
    merged = {**kwargs, "n": len(args)}
    pairs = [(k, v) for k, v in sorted(merged.items())]
    for a, *b in [(1, 2, 3), (4, 5)]:
        print(a, b, end=" ")
    print(*rest, sep="-")
    return first, middle, tail, pairs, max(*args), {*rest, *args}


def library_annotation(m: re.Match, d: collections.OrderedDict | None = None) -> str | None:
    if d is None:
        return m.group(0)
    return None


w = Worker(50)
w.start()
w.join()
print(w.result, isinstance(w, threading.Thread))
t = Tally("abracadabra")
print(t.top(2), t["a"])
try:
    raise AppError(7, "broken")
except ValueError as e:
    print(str(e), e.code, isinstance(e, AppError))
print(registry, Upper.tag, Upper().words("ab cd1ef"), sorted(Plugin.kinds))
acct = Account("ann", 120)
other = Account("bob", 5)
print(acct.label, acct.digits, acct.deposit(30), acct.balance, Account.opened())
acct.label = "carl#1"
print(acct.owner, acct.merge(other), acct.merge(None), Account.fee(250), Account.fee(2.5))
try:
    acct.balance = -1
except ValueError as e:
    print("refused", e)
parser = argparse.ArgumentParser()
parser.add_argument("--name", action=Collect)
print(parser.parse_args(["--name", "a", "--name", "b"]).name)
log = []
print(use_with(log, 2), log)
log = []
print(use_with(log, 0), log)
print(starred(3, 1, 4, 1, 5, x=9))
print(library_annotation(re.match(r"\w+", "hi there")), library_annotation(re.match("a", "a"), collections.OrderedDict()))


@functools.cache
def defaults():
    return {"retries": 3}


def describe_error(n: int) -> str:
    try:
        if n:
            raise ValueError("missing " + str(n))
        return "none"
    except ValueError as e:
        return str(e)


MISSING = [lambda: 0][0]


def lookup(table: dict, key: str, default=MISSING):
    found = table.get(key, default)
    return "missing" if found is MISSING else found


print(lookup({"a": 1}, "a"), lookup({}, "b"))
defaults()["retries"] += 1
print(defaults() is defaults(), defaults()["retries"], describe_error(2), describe_error(0))


class Point(collections.namedtuple("Point", "x y")):
    def norm1(self) -> int:
        return abs(self.x) + abs(self.y)


print(Point(3, -4).norm1(), Point(1, 2))
if __name__ == "__main__":
    acct.balance = -5
