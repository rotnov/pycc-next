# v0.9: `html.escape` and `re.escape` calls by a name `from html import
# escape` binds (in a `try`, as libraries write it) or as `html.escape`:
# CPython's results, the argument itself when `html.escape` has nothing to
# escape, CPython's own call for any other callee or argument.
import html
import html as markup
import re
from re import escape as re_escape

try:
    from html import escape
except ImportError:
    escape = None


# A module-level lambda alias: published as the function compiled code makes
# of it, so Python code calls it compiled.
ESC = lambda u: escape(u, quote=True)  # noqa: E731


# A `str` subclass (made by CPython): `html.escape` returns a `str`.
Tagged = type("Tagged", (str,), {})


def esc_all(items):
    out = []
    for s in items:
        out.append(escape(s))
        out.append(escape(s, False))
        out.append(escape(s, quote=False))
        out.append(html.escape(s, quote=True))
        out.append(markup.escape(s, 0))
    return out


def rebound(s):
    escape = str.upper
    return escape(s)


def identity(s):
    return escape(s) is s


def bad(value):
    try:
        return escape(value)
    except AttributeError as e:
        return "AttributeError " + str(e)


def re_all(items):
    out = [re.escape(s) for s in items]
    out.extend(re_escape(s) for s in items)
    return out, [re.escape(s) is s for s in items]


samples = ["plain", "a<b>&c", "\"q\" 'x'", "", "café <中>", "<<&&>>"]
print(ascii(esc_all(samples)))  # ascii: a native build on Windows prints cp1252
print(rebound("abc"), identity("plain"), identity("a<b"), bad(3))
print(ascii(re_all(samples + ["a.b*c", "(x|y)\t\n#~ &", "a", "\u00e9t\u00e9"])))
print(re.escape(b"a.b"), re.escape(Tagged("1+1")))
print(re.compile(re.escape("1+1=2")).match("1+1=2") is not None)
t = Tagged("x<y")
print(type(escape(t)).__name__, escape(t), type(escape(Tagged("xy"))).__name__)
escape = lambda s, quote=True: "custom:" + s
print(esc_all(["<"])[0])
