import sys
from decimal import Decimal

import pkg
from pkg import _core


def show(label, f, *args, **kwargs):
    try:
        print(label, repr(f(*args, **kwargs)))
    except Exception as e:
        cause = type(e.__cause__).__name__ if e.__cause__ is not None else None
        print(label, type(e).__name__, str(e), cause)


for name in ("_core", "_errors", "_types"):
    __import__("pkg." + name)
print(sorted(k for k in dir(_core) if k.isupper()))
print(sorted(_core.KEY_CHARS)[:5], len(_core.CTRL), type(_core.CTRL).__name__)
print(_core.MAX_PARTS, sorted(_core.WS))

docs = [
    'a = 1\nb.c = "x y"\nb.d = 2.5\n',
    "# comment\nflag = true\nother = false\n",
    'n = -12\n; another comment\ns = ""\n',
    "a.b.c.d = 1\n",
    "a.b.c.d.e = 1\n",
    'a = "unterminated\n',
    'a = "bad\x01char"\n',
    "a = 1.2.3\n",
    "= 1\n",
    "a 1\n",
    "a = \n",
    "x = 1\r\ny = 2\r\n",
]
for i, doc in enumerate(docs):
    show(f"loads[{i}]", pkg.loads, doc)
show("decimal", pkg.loads, "x = 0.1\ny = 2\n", parse_float=Decimal)
show("kw-only", pkg.loads, "x = 1\n", float)
show("unknown kw", pkg.loads, "x = 1\n", parse_floats=float)
show("pairs", pkg.load_pairs, ["a = 1", "b=2", "c"])
show("pairs sep", pkg.load_pairs, ["a: 1", "b:2"], sep=":")
show("skip_until", _core.skip_until, "abc#", 0, "#", error_on=frozenset("\x00"), error_on_eof=True)
show("skip_until eof", _core.skip_until, "abc", 0, "#", error_on=frozenset(), error_on_eof=True)
show("skip_until bad", _core.skip_until, "a\x01c#", 0, "#", error_on=frozenset("\x01"), error_on_eof=False)
show("skip_chars", _core.skip_chars, "   x", 0, " ")
show("skip_chars list", _core.skip_chars, "abba!", 0, ["a", "b"])
show("parse_key", _core.parse_key, "a.b.c = 1", 0)
show("describe", _core.describe, "text")
t = _core.Table()
t.put(("a", "b"), 1)
t.put(("a", "c"), 2)
print(t.data, t.count)
sys.stdout.flush()
