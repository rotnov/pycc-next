print("pkg.sub.deep: body")
from ..util import triple
from .. import geometry

TAG = "deep"
NAME = __name__


def depth() -> int:
    return triple(2) + geometry.Point(1, 0).norm2()
