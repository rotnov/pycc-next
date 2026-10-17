import decimal
import fractions


class Raising:
    def __hash__(self):
        raise ValueError("no hash for you")


def of(o: object):
    return hash(o)


def keyed(o, n: int, s: str):
    return hash((o, n, s))


def nested(o):
    return hash(((o, 1), "x"))


def same(o) -> bool:
    return hash(o) == hash(o)


def unhashable():
    out = []
    out.append([1])
    out.append({})
    out.append({1})
    return out


def main():
    d = decimal.Decimal("2.5")
    f = fractions.Fraction(5, 2)
    print(of(d) == of(f), of(d) == hash(2.5), of(7), of(-1), of(2**70) == hash(2**70))
    print(of("abc") == of("ab" + "c"), of(None) == hash(None), of((1, "a")) == of((1, "a")))
    print(keyed(d, 3, "x") == hash((d, 3, "x")), nested(f) == hash(((f, 1), "x")), same(object()))
    for bad in unhashable():
        try:
            of(bad)
        except TypeError as e:
            print("TypeError:", e)
    try:
        keyed([], 1, "s")
    except TypeError as e:
        print("TypeError:", e)
    of(Raising())


main()
