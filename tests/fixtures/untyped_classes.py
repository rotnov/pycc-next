"""Untyped code `--non-strict` compiles natively (ROADMAP v0.6 accept 2):
`bytes` literals, calls unpacking `*args`/`**kwargs` into a CPython callee,
`except` clauses naming a CPython object, an object tested against a tuple
of scalars, a standard-library function called with an object, a builtin
exception class as a value and builtin calls inside generators. Run as a
program it prints what each function returns."""
import itertools
import math
import struct
import warnings
import zlib

SIGNATURE = b"\x89PNG\r\n\x1a\n"


def tag(t):
    if t == b"IEND":
        return "end"
    if t in (b"IDAT", b"PLTE"):
        return "data"
    return t + b"\x00\xff"


def literals():
    return [b"", b"a\x00b", b"\xe9\xff", len(SIGNATURE), SIGNATURE[1:4], b"ab" * 2, b"x" + b"y"]


def group(s, n):
    return list(zip(*[iter(s)] * n))


def chain(blocks):
    return bytearray(itertools.chain(*blocks))


def pack(fmt, values, last):
    return struct.pack(fmt, 7, *values, last)


def forward(f, *args, **kwargs):
    return f(*args, **kwargs)


def call_mixed(f, a, rest, d):
    return f(a, *rest, 9, key=1, **d)


def unpack_word(data):
    try:
        return struct.unpack("!L", data)[0]
    except struct.error:
        return -1


def classify(data):
    try:
        return struct.unpack("!L", data)[0] // len(data)
    except ValueError:
        return "value"
    except struct.error as e:
        return "struct: " + str(e)
    except ZeroDivisionError:
        return "zero"


def unmatched(data):
    try:
        return struct.unpack("!H", data)[0]
    except KeyError:
        return "key"


def not_a_class(x):
    try:
        return 1 // x
    except len:
        return "never"


def filter_kind(kind):
    if kind not in (1, 2, 3, 4):
        return "bad"
    return kind in [2, 4]


def row_count(width, step):
    return int(math.ceil((width - 1) / float(step)))


def warn_if(x):
    if x:
        warnings.warn("careful", RuntimeWarning)
    return x


def decompress(blocks):
    d = zlib.decompressobj()
    for data in blocks:
        yield bytearray(d.decompress(data))
    yield bytearray(d.flush())


def target(*a, **k):
    return a, sorted(k.items())


def add(a, b):
    return a + b


def sub(a, b):
    return a - b


def pick(k, x, y):
    fn = (None, add, sub)[k]
    return fn(x, y)


def apply(f, x):
    return f(x, 1)


class Decoder:
    """An untyped class: attributes created outside `__init__`, keyword
    calls of its own methods, generator methods, nested functions defined
    under an `if`, and a method used as a value."""

    def __init__(self, data=None, wide=False):
        if data is not None:
            self.data = data
        self.wide = wide
        self.header = None

    def parse(self, strict=True):
        self.width, self.height = struct.unpack("!2H", self.data[:4])
        self.planes = 3 if self.wide else 1
        if strict and self.width == 0:
            raise ValueError("empty image")
        return self.width * self.height

    def chunk(self, lenient=False, size=2):
        return self.data[4:4 + size], lenient

    def chunks(self):
        yield self.chunk(lenient=True)
        yield self.chunk(size=3, lenient=False)

    def _rows(self, values):
        for i in range(0, len(values), self.width):
            yield values[i:i + self.width]

    def rows(self, values, flat=False):
        if flat:
            def it():
                for v in values:
                    yield [v]
            rows = it()
        else:
            rows = self._rows(values)
        return list(rows)

    def wide_rows(self, rows):
        if self.wide:
            def newarray():
                return [0]
        else:
            def newarray():
                return bytearray([0])

        def expand():
            for row in rows:
                a = newarray() * 3 * len(row)
                for i in range(3):
                    a[i::3] = bytearray(row) if not self.wide else list(row)
                yield a
        return list(expand())

    def _scaled(self, get, k):
        w, h = get()
        return w * k, h * k

    def size(self):
        return self.width, self.height

    def scaled(self, k):
        return self._scaled(self.size, k)


def main():
    print(tag(b"IEND"), tag(b"PLTE"), tag(b"zTXt"), literals())
    print(group([1, 2, 3, 4, 5, 6], 2), group("abcdef", 3), chain([b"ab", b"cd"]))
    print(pack("!4B", (1, 2), 9), pack("!2B", [], 4))
    print(forward(target, 1, 2, x=3), call_mixed(target, 0, (1, 2), {"z": 3}))
    for bad in [(1, ()), ([], 5), ([], {"key": 2}), ([], {1: 2})]:
        try:
            print(call_mixed(target, 0, *bad))
        except TypeError as e:
            print("TypeError", e)
    for f, args in [(struct.pack, ("!B",)), (pack, ("!B", (1,), 2))]:
        try:
            print(f(*args))
        except struct.error as e:
            print("struct.error", e)
    print(unpack_word(b"abcd"), unpack_word(b"ab"), classify(b"abcd"), classify(b"ab"))
    try:
        unmatched(b"x")
    except struct.error as e:
        print("escaped", type(e).__name__, e)
    try:
        not_a_class(0)
    except TypeError as e:
        print("TypeError", e)
    print(not_a_class(1))
    print([filter_kind(k) for k in (1, 2, 4, 9, 4.0, True, "a", None)])
    print(row_count(10, 3), row_count(7.5, 2), row_count(2 ** 70, 1) > 0)
    print(warn_if(0))
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        try:
            warn_if(1)
        except RuntimeWarning as e:
            print("RuntimeWarning", e)
    data = zlib.compress(b"hello world" * 3)
    print(list(decompress([data[:5], data[5:]])))
    print(pick(1, 2, 3), pick(2, 2, 3), apply(sub, 5), apply(add, 1.5))
    try:
        pick(0, 1, 2)
    except TypeError as e:
        print("TypeError", e)
    d = Decoder(b"\x00\x02\x00\x03abcdef")
    print(d.parse(), d.planes, d.chunk(), list(d.chunks()), d.scaled(2))
    print(d.rows([1, 2, 3, 4, 5, 6]), d.rows([7, 8], flat=True))
    print(d.wide_rows([[1, 2], [3]]), Decoder(b"\x00\x01\x00\x01", True).wide_rows([[4]]))
    try:
        Decoder(b"\x00\x00\x00\x01").parse()
    except ValueError as e:
        print("ValueError", e)
    try:
        Decoder().parse()
    except AttributeError as e:
        print("AttributeError", e)


if __name__ == "__main__":
    main()
