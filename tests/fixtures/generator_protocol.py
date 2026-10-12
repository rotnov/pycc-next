from typing import Generator, Iterator


def echo() -> Generator[int, int, str]:
    total = 0
    try:
        while True:
            x = yield total
            total += x
            if total > 100:
                return "big"
    finally:
        print("echo finally", total)


def counter(n: int) -> Iterator[int]:
    i = 0
    try:
        while i < n:
            yield i
            i += 1
    finally:
        print("counter done", i)


def averager() -> Generator[float, float, int]:
    count = 0
    total = 0.0
    average = 0.0
    while True:
        value = yield average
        if value < 0:
            return count
        count += 1
        total += value
        average = total / count


def delegator() -> Generator[float, float, None]:
    while True:
        n = yield from averager()
        print("averaged", n, "values")


def stubborn() -> Iterator[int]:
    while True:
        try:
            yield 1
        except GeneratorExit:
            print("ignoring exit")


def catcher() -> Generator[str, None, None]:
    while True:
        try:
            yield "ready"
        except ValueError as e:
            print("caught", e)


def inner(n: int) -> Generator[int, int, str]:
    total = 0
    try:
        for i in range(n):
            got = yield i
            total += got
    except ValueError as e:
        print("inner caught", e)
        yield -1
    finally:
        print("inner finally", total)
    return "sum=" + str(total)


def outer() -> Generator[int, int, str]:
    result = yield from inner(3)
    print("inner returned", result)
    again = yield from inner(2)
    return result + "|" + again


def leaf() -> Iterator[int]:
    try:
        yield 1
        yield 2
    finally:
        print("leaf closed")


def middle() -> Iterator[int]:
    try:
        yield from leaf()
    finally:
        print("middle closed")


def raises_stop() -> Iterator[int]:
    yield 1
    raise StopIteration("x")


def ret_only() -> Generator[int, None, int]:
    if False:
        yield 0
    return 42


def words() -> Iterator[str]:
    ws = ["a", "b"]
    yield from ws
    yield from ["c"]


def running_sum() -> Generator[int, int, None]:
    total = 0
    while True:
        total += yield total


def doubler() -> Generator[str, str, None]:
    s = ""
    while True:
        s = (yield s + s) or "empty"


def main() -> None:
    g = echo()
    print(next(g))
    print(g.send(5))
    print(g.send(10))
    try:
        g.send(90)
    except StopIteration as e:
        print("stop", e.value)
    try:
        g.send(1)
    except StopIteration as e:
        print("again stop")

    c = counter(5)
    print(next(c), next(c))
    c.close()
    c.close()
    try:
        next(c)
    except StopIteration:
        print("closed counter stops")

    d = delegator()
    print(next(d))
    print(d.send(10.0))
    print(d.send(20.0))
    print(d.send(-1.0))
    print(d.send(4.0))

    s = stubborn()
    next(s)
    try:
        s.close()
    except RuntimeError as e:
        print("RuntimeError:", e)

    k = catcher()
    print(next(k))
    print(k.throw(ValueError("boom")))
    try:
        k.throw(KeyError("k"))
    except KeyError as e:
        print("propagated", e)
    try:
        next(k)
    except StopIteration:
        print("k is done")

    fresh = echo()
    try:
        fresh.send(3)
    except TypeError as e:
        print("TypeError:", e)

    t = counter(3)
    try:
        t.throw(ValueError("early"))
    except ValueError as e:
        print("early throw:", e)

    o = outer()
    print(next(o))
    print(o.send(10))
    print(o.send(20))
    print(o.send(30))
    print(o.send(1))
    try:
        o.send(2)
    except StopIteration as e:
        print("outer done:", e.value)

    o2 = outer()
    next(o2)
    print(o2.throw(ValueError("x")))
    o2.close()

    m = middle()
    print(next(m))
    m.close()
    print("after close")

    m2 = middle()
    print(next(m2))
    del m2
    print("after del")

    b = raises_stop()
    print(next(b))
    try:
        next(b)
    except RuntimeError as e:
        print("RuntimeError:", e)

    r = ret_only()
    try:
        next(r)
    except StopIteration as e:
        print("value", e.value)

    print(list(words()))
    for w in words():
        print(w)

    rs = running_sum()
    next(rs)
    for v in [1, 2, 3]:
        print(rs.send(v))
    rs.close()

    dbl = doubler()
    print("[" + next(dbl) + "]")
    print(dbl.send("ab"))
    print(dbl.send(""))


main()
module_gen = counter(4)
print(next(module_gen))
deleted = counter(4)
print(next(deleted))
del deleted
print("after module del")
left_open = leaf()
print(next(left_open))
print("end")
