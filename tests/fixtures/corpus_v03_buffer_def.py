# A `def` returning `sys.stdin.buffer.readline()` whose every call is
# converted to numbers reads standard input as text.
import sys


def _r(): return sys.stdin.buffer.readline()
def rs(): return _r().decode('ascii').strip()
def rn(): return int(_r())
def rnt(): return map(int, _r().split())


for _ in range(rn()):
    x, y = rnt()
    print(x + y, x * y)
