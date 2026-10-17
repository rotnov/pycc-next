"""Untyped loops over byte buffers. Built `--ext --non-strict`, each function
gets a native specialization for `bytearray` arguments (an item of each is
used as a number), which reads and writes the caller's buffer in place."""


def undo_sub(fu, scanline, result):
    ai = 0
    for i in range(fu, len(result)):
        x = scanline[i]
        a = result[ai]
        result[i] = (x + a) & 0xff
        ai += 1


def paeth(fu, scanline, previous, result):
    ai = -fu
    for i in range(len(result)):
        x = scanline[i]
        if ai < 0:
            a = c = 0
        else:
            a = result[ai]
            c = previous[ai]
        b = previous[i]
        p = a + b - c
        pa = abs(p - a)
        pb = abs(p - b)
        pc = abs(p - c)
        if pa <= pb and pa <= pc:
            pr = a
        elif pb <= pc:
            pr = b
        else:
            pr = c
        result[i] = (x + pr) & 0xff
        ai += 1
    return result


def checksum(data):
    s = 0
    for i in range(len(data)):
        s = (s * 31 + data[i]) & 0xFFFF
    return s


def poke(buf, i, v):
    for _ in range(1):
        buf[i] = v + 1
    return buf


def peek(buf, i):
    total = 0
    for _ in range(2):
        total += buf[i] - 1
    return total


def grow(buf, n):
    for i in range(n):
        buf.append(buf[i] ^ 0x55)
    return len(buf)


def shrink(buf):
    n = 0
    while len(buf) > 0:
        n += buf[len(buf) - 1] - 0
        del buf[0]
    return n


def unpack(bs, depth, k):
    # Sub-byte samples, as pypng unpacks a row: each `extend` of a list
    # comprehension reuses one list per site (`extend_scratch`). A negative
    # `depth` makes a shift raise part way through filling it, a large `k`
    # a byte out of range, an index past `bs` an `IndexError`.
    out = bytearray()
    d = depth * 1
    shifts = [d * i for i in reversed(list(range(8 // (d * d))))]
    mask = 2 ** (d * d) - 1
    for o in bs:
        out.extend([mask & (o >> i) for i in shifts])
        out.extend([(o + k * 1) for _ in range(2)])
        out.extend([bs[(o + i) % 5] for i in range(3) if i != 1])
    return out
