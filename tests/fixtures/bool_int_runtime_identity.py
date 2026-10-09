def forward(value: int) -> int:
    return value


def source() -> int:
    return True


annotated: int = True
print(annotated)
annotated_false: int = False
print(annotated_false)

reassigned = 7
reassigned = True
print(reassigned)

print(forward(True))
print(source())
print(forward(source()))
print(f"value={source()}")

tuple_forwarded = (source(),)
print(tuple_forwarded[0])

values = [0]
values.append(True)
print(values[1])
print(values.pop())

mapping = {"value": 0}
mapping["value"] = True
print(mapping["value"])
print(mapping.get("missing", True))

ones = {1}
ones.add(True)
zeros = {0}
zeros.add(False)
print(len(ones))
print(len(zeros))

one_candidates = [2]
one_candidates.append(True)
bool_first_ones = {value for value in one_candidates if value < 2}
bool_first_ones.add(1)
for value in bool_first_ones:
    print(value)

zero_candidates = [1]
zero_candidates.append(False)
bool_first_zeros = {value for value in zero_candidates if value < 1}
bool_first_zeros.add(0)
for value in bool_first_zeros:
    print(value)

# Function returns and annotated slots are statically int but carry bool
# identity markers. Range must consume them numerically before its phi.
for value in range(source(), 4, source()):
    print(value)

# All three positions are marker-bearing int values, but range yields ordinary
# ints rather than forwarding False as its first visible target.
for value in range(annotated_false, source(), annotated):
    print(value)

# A marker-bearing False stop must produce an empty loop, not be mistaken for
# a pointer or a bool-identity target.
for value in range(0, annotated_false, 1):
    print(value)
print(99)

range_list = [value for value in range(annotated_false, source(), annotated)]
for value in range_list:
    print(value)

range_set = {value for value in range(annotated_false, source(), annotated)}
print(len(range_set))
for value in range_set:
    print(value)

range_dict = {
    f"k{value}": value
    for value in range(annotated_false, source(), annotated)
}
print(range_dict["k0"])

print(source() + 0)

# #627: an attribute store is a checked bool-into-int boundary too. The
# `int`-declared slot holds D-141 encoded words, so an unencoded `bool`
# word landing there read back as `0` and then aborted the program.
class Slots:
    def __init__(self) -> None:
        self.n = 0
        self.flag = False


slots = Slots()
slots.n = True
print(slots.n)
slots.n = False
print(slots.n)

# The same slot after it has held a heap-allocated bigint: the store of a
# bool must release the old word rather than leak it (D-180 Consequences
# item 6), and must still print the bool identity.
slots.n = 4611686018427387904 + 1
print(slots.n)
slots.n = True
print(slots.n)

# A `bool`-declared slot must keep storing its raw word: encoding a D-141
# marker into it would truncate to a non-zero `i8` and print `True` for
# `False`.
slots.flag = True
print(slots.flag)
slots.flag = False
print(slots.flag)
