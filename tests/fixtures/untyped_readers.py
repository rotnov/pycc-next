"""More untyped code `--non-strict` compiles natively (ROADMAP v0.6 accept 2),
the shapes of a binary format reader: a class whose chunk handlers CPython
code looks up by name, generator methods feeding each other, helpers with
`None` defaults, byte buffers built and iterated natively, objects compared
with sets, lists and tuples of scalars, module functions picked out of a
tuple and called, and the integer attribute a method reads first taken by
its native specialization. Run as a program it prints what each function
returns."""
import struct
import zlib


def undo_sub(fu, scanline, result):
    ai = 0
    for i in range(fu, len(result)):
        result[i] = (scanline[i] + result[ai]) & 0xff
        ai += 1


def undo_up(fu, scanline, result, previous):
    for i in range(len(result)):
        result[i] = (scanline[i] + previous[i]) & 0xff


def _cut(bs, width=None):
    if width is None:
        return bs
    return bs[:width]


def cut_both(x):
    return _cut(x), _cut(x, 2)


def planes(ct):
    colormap = bool(ct & 1)
    grey = not (ct & 2)
    return (3, 1)[grey or colormap], (0.5, 1.5)[ct & 1]


def letters(kind):
    found = set(bytearray(kind))
    return (
        found <= set(range(65, 91)) | set(range(97, 123)),
        found == {73, 72},
        [1, 2] == list(kind[:2]),
        (1, 2) < tuple(kind[:2]),
    )


def spread(bs, n):
    out = bytearray()
    for o in bs:
        out.append(o)
        out.extend([o, n & 255])
    for x in out:
        if x == 7:
            out.append(1)
    return out


def store_bytes(n):
    out = bytearray(b"ab")
    try:
        out.append(n)
    except ValueError as e:
        print("ValueError", e)
    try:
        out.extend([1, n, 2])
    except ValueError as e:
        print("ValueError", e, out)
    out.extend("cd".encode())
    out.append(True)
    return out


def gen(o):
    for x in o:
        yield x


def with_rows(o, w):
    rows = gen(o)
    return w, rows, {"w": w}


class Reader:
    """An untyped reader of a two-chunk format: `HEAD` (width and bit
    depth) and `DATA` (zlib-compressed filtered rows)."""

    def __init__(self, data):
        self.data = data
        self.pos = 0

    def chunk(self):
        length, kind = struct.unpack("!I4s", self.data[self.pos:self.pos + 8])
        body = self.data[self.pos + 8:self.pos + 8 + length]
        self.pos += 8 + length
        return kind, body

    def process(self):
        kind, body = self.chunk()
        handler = getattr(self, "_process_" + kind.decode("ascii"), None)
        if handler:
            handler(body)
        return kind

    def _process_HEAD(self, body):
        self.width, self.depth = struct.unpack("!2B", body)
        self.row_bytes = (self.width * self.depth + 7) // 8

    def _process_DATA(self, body):
        self.raw = zlib.decompress(body)

    def _packed(self, blocks):
        a = bytearray()
        recon = None
        for block in blocks:
            a.extend(block)
            while len(a) >= self.row_bytes + 1:
                kind = a[0]
                scanline = a[1:self.row_bytes + 1]
                del a[:self.row_bytes + 1]
                recon = self.undo(kind, scanline, recon)
                yield recon

    def undo(self, kind, scanline, previous):
        if kind == 0:
            return scanline
        if kind not in (1, 2):
            raise ValueError("bad filter %d" % kind)
        if not previous:
            previous = bytearray([0] * len(scanline))
        fn = (None, undo_sub, undo_up)[kind]
        if kind == 1:
            fn(1, scanline, scanline)
        else:
            fn(1, scanline, scanline, previous)
        return scanline

    def _values(self, bs, width=None):
        if self.depth == 8:
            return bytearray(bs)
        if width is None:
            width = self.width
        spb = 8 // self.depth
        out = bytearray()
        mask = 2 ** self.depth - 1
        shifts = [self.depth * i for i in reversed(list(range(spb)))]
        for o in bs:
            out.extend([mask & (o >> i) for i in shifts])
        return out[:width]

    def _rows(self, rows):
        for row in rows:
            yield self._values(row)

    def read(self):
        while self.process() != b"DATA":
            pass
        if self.depth == 1:
            def convert():
                for row in self._rows(self._packed([self.raw])):
                    yield list(row)
        else:
            def convert():
                for row in self._rows(self._packed([self.raw])):
                    yield bytes(row)
        return self.width, convert(), {"depth": self.depth}


def encode(width, depth, rows):
    raw = bytearray()
    for kind, row in rows:
        raw.append(kind)
        raw.extend(row)
    head = struct.pack("!I4s2B", 2, b"HEAD", width, depth)
    data = zlib.compress(bytes(raw))
    return head + struct.pack("!I4s", len(data), b"DATA") + data


def decode(data):
    width, rows, info = Reader(data).read()
    return width, list(rows), info


def main():
    print(cut_both(b"abc"), cut_both([1, 2, 3]))
    print([planes(c) for c in (0, 2, 3, 6)])
    print(letters(b"IHDR"), letters(b"I\x01DR"), letters(b"\x01\x02ab"))
    print(spread(bytearray(b"\x01\x07\x03"), 5), spread(bytearray(), 1))
    print(store_bytes(300), store_bytes(-1))
    w, rows, info = with_rows([1, "a", 2.5], 3)
    print(w, next(rows), list(rows), info)
    print(decode(encode(16, 1, [(0, b"\xa5\x0f"), (1, b"\x01\x01"), (2, b"\x10\x01")])))
    print(decode(encode(3, 8, [(1, b"\x01\x02\x03"), (2, b"\x05\x05\x05"), (0, b"abc")])))
    print(decode(encode(4, 2, [(0, b"\x1b"), (2, b"\x01")])))
    for bad in (encode(2, 8, [(3, b"ab")]), encode(2, 8, [(0, b"ab")])[:20]):
        try:
            print(decode(bad))
        except (ValueError, zlib.error, struct.error) as e:
            print(type(e).__name__, e)


if __name__ == "__main__":
    main()
