"""Uses `m`'s exception classes from Python (`tests/fixtures/v010_exceptions.py`)
as CPython code would; the output must match CPython running the source."""
# The pickled data is this test's own round trip.
import copy
import pickle
import sys
import traceback

import m


def mro(cls):
    """The MRO without the layout templates (`docs/DEVIATIONS.md`)."""
    return [c.__name__ for c in cls.__mro__ if c.__module__ != "pycc"]


e = m.Owned(3, "here")
print(type(e).__name__, e.args, e.code, e.where, str(e), repr(e), e.describe())
print(isinstance(e, Exception), isinstance(e, m.Owned), issubclass(m.Deeper, m.Owned))
print(mro(m.Owned), mro(m.Deeper), mro(m.Tag))
for n in (1, 5, 15):
    print(m.run(n))
print(m.churn(200))
try:
    m.fail(7)
except m.Owned as caught:
    print("caught", type(caught).__name__, caught.code, caught.where, caught.args)
    print("".join(traceback.format_exception_only(caught)), end="")
try:
    m.fail(17)
except m.Owned as caught:
    print("caught", type(caught).__name__, caught.describe(), caught.total(), caught.depth)
print(e.bump(4), e.code, e.describe())
e.code = 11
print(e.describe())
e.extra = "x"
print("extra" in e.__dict__, e.__dict__["extra"], e.extra)
del e.code
try:
    e.code
except AttributeError as err:
    print("AttributeError", err)
try:
    e.describe()
except AttributeError as err:
    print("AttributeError", err)

s = m.Silent(2, "quiet")
print(s.args, repr(s), str(s), s.level, s.why)
c = m.Coded(9)
print(c.args, str(c), repr(c), c.code)
for obj in (c, m.Coded(1)):
    for proto in (0, 2, pickle.HIGHEST_PROTOCOL):
        back = pickle.loads(pickle.dumps(obj, proto))
        print(proto, type(back).__name__, back.args, back.code, str(back))
    print(copy.copy(obj).code, copy.deepcopy(obj).code)
try:
    pickle.dumps(m.Owned(1, "w"))
    pickle.loads(pickle.dumps(m.Owned(1, "w")))
except TypeError as err:
    print("TypeError", err)
d = m.Deeper(4, "d", 2)
print(d.describe(), d.total(), d.args, d.code, d.depth, str(d))
t = m.Tag(5, "t")
print(t.describe(), t.args, type(t).__name__)


class Sub(m.Owned):
    def __init__(self, code, where, more):
        super().__init__(code, where)
        self.more = more

    def describe(self):
        return "sub " + super().describe() + " " + self.more


class Pick(m.Coded):
    def __init__(self, code):
        super().__init__(code)
        self.note = "n"


sub = Sub(1, "s", "x")
print(sub.describe(), sub.more, sub.args, sub.code, mro(Sub))
try:
    raise sub
except m.Owned as caught:
    print("caught", caught is sub, caught.describe())
p = Pick(6)
back = pickle.loads(pickle.dumps(p))
print(type(back).__name__, back.code, back.note, back.args, str(back))
print(copy.copy(p).note, copy.deepcopy(p).code)
try:
    raise m.Coded(3) from ValueError("why")
except m.Coded as caught:
    print(type(caught.__cause__).__name__, caught.__cause__, caught.code)
group = ExceptionGroup("g", [m.Coded(1), m.Owned(2, "g")])
try:
    raise group
except* m.Owned as eg:
    print("group", [x.describe() for x in eg.exceptions])
except* m.Coded as eg:
    print("group", [x.code for x in eg.exceptions])
for n in (0, 2):
    print(m.probe(n))
ch = m.Checked(3)
print(ch.show(), ch.origin, ch.code, ch.args, mro(m.Validated))
try:
    m.Checked(-1)
except ValueError as err:
    print("ValueError", err)
v = m.Validated(4, "v")
print(v.show(), v.origin, v.label, v.args, str(v))
nf = m.NoFile(2, "/p")
print(str(nf), nf.errno, nf.strerror, nf.args, nf.path, nf.show())
nf.errno = 5
print(str(nf), nf.show())
back = pickle.loads(pickle.dumps(nf))
print(type(back).__name__, str(back), back.errno, back.path)
print(sys.getrefcount(e) > 0)
