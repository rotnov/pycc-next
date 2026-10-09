# #1244 (Part 1 of #1216): `del name`. A deleted name is unbound until it is
# bound again; pycc proves statically that no read reaches it in between
# (docs/TYPE_SYSTEM.md, "`del` statement"). Every module-level name deleted
# below is spelled differently from every name a function or class here
# mentions: a module-level `del` of a name one of them mentions is refused.
# The generic-function case lives in tests/issue_1244_del_name.rs, since a
# generic function beside an enum loop in a function fails to build (#1252).
from enum import Enum


class Color(Enum):
    RED = 1
    GREEN = 2


def rebind_local(n: int) -> int:
    y = n + 1
    del y
    y = n * 10
    return y


def drop_parameter(n: int) -> int:
    del n
    n = 4
    return n


def count_after_del() -> int:
    items = []
    scratch = 1
    del scratch
    items.append(2)
    items.append(3)
    return len(items)


def loops(limit: int) -> int:
    i = 0
    total = 0
    while i < limit:
        i += 1
        step = i
        total += step
        del step
    for j in range(3):
        k = j
        del k
    values = [1, 2]
    for v in values:
        del v
    for shade in Color:
        tag = shade.value
        total += tag
        del tag
    return total


class Tools:
    @staticmethod
    def untouched(self: int) -> int:
        del self
        return 7


print(rebind_local(3))
print(drop_parameter(1))
print(count_after_del())
print(loops(3))
print(Tools.untouched(1))

a = 1
print(a)
del a
a = 2
print(a)

b1 = 1
b2 = 2
del b1, b2
b1 = 3
b2 = 4
print(b1 + b2)

c1 = 1
c2 = 2
c3 = 3
c4 = 4
del (c1, c2)
del [c3, [c4]]
del ()
del []
c1 = c2 = c3 = c4 = 5
print(c1 + c2 + c3 + c4)

flag = len([1]) > 0
if flag:
    del flag
    flag = False
else:
    del flag
    flag = True
print(flag)

for member in Color:
    label = member.value
    del label
    print(member.value)

names = [1, 2, 3]
for each in names:
    del each

count = 0
while count < 2:
    count += 1
    other = count
    del other
print(count)

held = 1
try:
    del held
    held = 9
except ValueError:
    held = 0
print(held)

grouped = 1
try:
    raise ValueError("e")
except* ValueError:
    del grouped
    grouped = 5
print(grouped)
