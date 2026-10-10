def f() -> int:
    return n * 2


def g() -> str:
    return s


try:
    print(f())
except NameError as e:
    print("NameError", e)
for i in range(2):
    n = i + 5
    print(f())
for i in range(0):
    s = "x"


def cal(x: int) -> int:
    return lcp[x] + k


for _ in range(2):
    k = int(input())
    lcp = [k] * (k + 1)
    print(cal(1))
if f() > 50:
    import copy


def name() -> str:
    return str(copy.__name__)


try:
    print(name())
except NameError as e:
    print("NameError", e)
import copy
print(name())
print(g())
