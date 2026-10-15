# A read of a name bound on some paths only compiles in strict mode and is
# checked at run time: `UnboundLocalError` in a function, `NameError` at
# module scope, exactly where CPython raises them.


def first_divisor(n: int) -> int:
    for d in range(2, n):
        if n % d == 0:
            break
    return d


def pick(flag: bool) -> str:
    if flag:
        label = "ready"
    return label


def root_of(edges: list[int], n: int) -> int:
    for i in range(1, n + 1):
        if i not in edges:
            root = i
            break
    total = 0
    for i in range(1, n + 1):
        if i == root:
            continue
        total += i
    return total * 100 + root


def last_seen(xs: list[str]) -> str:
    for x in xs:
        pass
    return x


def while_bound(flag: bool) -> int:
    while flag:
        x = 1
        flag = False
    return x


def handler_passes(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        pass
    return x


def finally_reads(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = -1
    finally:
        print("finally", x)
    return x


print(while_bound(True), handler_passes(3), finally_reads(0), finally_reads(5))
for call in range(2):
    try:
        if call == 0:
            print(while_bound(False))
        else:
            print(handler_passes(0))
    except NameError as e:
        print("NameError", e)
print(first_divisor(91), first_divisor(15))
print(pick(True))
print(root_of([1, 2], 4))
print(last_seen(["a", "b"]))
for call in range(4):
    try:
        if call == 0:
            print(first_divisor(2))
        elif call == 1:
            print(pick(False))
        elif call == 2:
            print(root_of([1, 2, 3], 3))
        else:
            print(last_seen([]))
    except NameError as e:
        print("NameError", e)

total = 0
for k in range(3):
    if k == 1:
        found = k
for k in range(5):
    total += k
print(found, k, total)
try:
    for j in range(0):
        pass
    print(j)
except NameError as e:
    print("NameError", e)
for t in range(3):
    if t == 2:
        late = "late"
print(late)
