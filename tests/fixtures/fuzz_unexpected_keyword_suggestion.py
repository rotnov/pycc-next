# A call with an unknown keyword that compiled code binds itself (a function
# with keyword-only parameters or defaults) says what CPython 3.13 says,
# "Did you mean" included when a parameter name is close enough (fuzz
# seed 101, cases 7 and 130).
def b0_fn(key: str, count: str, *, beta: float, width: int) -> str:
    return key


def b1_fn(width: float = 2.0, *, height: float = 3.25) -> str:
    return ''


class K:
    def m(self, by: int = 1, *, label: str = "") -> str:
        return label * by


try:
    b0_fn('', 'a', count2=1, beta=3.25, width=5)
except TypeError as e:
    print('T', e)
try:
    b1_fn(heigh=8)
except TypeError as e:
    print('T', e)
try:
    b1_fn(zz=8)
except TypeError as e:
    print('T', e)
try:
    K().m(lable="x")
except TypeError as e:
    print('T', e)
try:
    K().m(slf=2)
except TypeError as e:
    print('T', e)
