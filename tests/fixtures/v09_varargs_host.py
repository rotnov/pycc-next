# v0.9 accept 1: Python code calling `v09_varargs`'s compiled methods with
# every binding shape (`tests/v09_native_state.rs` imports the fixture as
# `m`, compiled or not).
from m import Pair, Stack, Triple, _Registry, gather

print(repr(Stack.create(1, 2)), Stack(*range(3))._stack, Stack().kind)
print(Stack.create(x=[1]).top(), Stack().create(4).top(), Stack.__dict__["create"].__class__.__name__)
print(Pair(*"ab").describe(), Pair("a", "b", *"cd").describe())
print(_Registry.build(), _Registry.build(*"pq", mode="r"))
print(gather(*range(2), **{"z": 1, "y": 2}))
print(Pair(second=2, first=1).describe(), Pair(1, second=2).describe(), Triple(1, 2, 3, 4).total)
bad_calls = [
    lambda: Pair(),
    lambda: Pair(1),
    lambda: Stack(k=1),
    lambda: Pair(1, 2, x=3),
    lambda: Pair(1, 2, 3, first=0),
    lambda: Triple(),
    lambda: Triple(b=1),
    lambda: Triple(1, c=2, d=3),
]
for bad in bad_calls:
    try:
        bad()
    except TypeError as e:
        print("TypeError", e)
try:
    Stack.create().top()
except IndexError as e:
    print("IndexError", e)
