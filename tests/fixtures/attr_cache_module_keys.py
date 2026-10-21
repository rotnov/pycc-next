# A module's own globals read as attributes of the module object: the
# compiled stores put keys into its dict that are equal to, not identical
# with, the interned names the reads look up.
import sys

LIMIT = "strict"
COUNT = 5


def read_many(n):
    me = sys.modules[__name__]
    total = 0
    for _ in range(n):
        total += len(me.LIMIT) + me.COUNT
    return total


print(read_many(200))
sys.modules[__name__].LIMIT = "xx"
print(read_many(3))
del sys.modules[__name__].COUNT
try:
    read_many(1)
except AttributeError as e:
    print("AttributeError", e.name)
