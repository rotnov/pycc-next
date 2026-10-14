"""A program of several modules and packages (v0.3 accept 1): the entry."""

print("main: start")
import helper
import pkg.geometry
import pkg.sub.deep as deep
from pkg import util
from pkg.geometry import Point, origin_distance
from helper import double as dbl, COUNT, LIMITS
from pkg.sub import deep as deep2

print("main: imports done")


def main() -> None:
    print(helper.double(21), dbl(2), util.triple(3))
    p = pkg.geometry.Point(3, 4)
    print(p.norm2(), origin_distance(Point(1, 1)), p.shifted(1).x)
    print(helper.COUNT, COUNT)
    helper.bump()
    helper.bump()
    print(helper.COUNT, COUNT)
    print(helper.greet("pycc"), LIMITS[1], len(helper.LIMITS))
    print(deep.depth(), deep2.depth(), deep.TAG)
    print(helper.main(), helper.shared_calls(), pkg.geometry.shared_calls())
    try:
        helper.fail(5)
    except helper.HelperError:
        print("caught HelperError")
    try:
        helper.fail(-1)
    except ValueError:
        print("caught ValueError")
    total = 0
    for i in range(4):
        total += helper.double(i)
    print("total", total)


main()
print(__name__, helper.NAME, pkg.NAME, deep.NAME)
