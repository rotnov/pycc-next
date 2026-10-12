# Non-strict mode: `x is True` / `x is False` is identity with a `bool`
# singleton, and a native scalar is never `None`.
import json

flag = len("ab") > 1
print(flag is True, flag is not True, flag is False, False is flag)
n = 3
s = "x"
print(n is True, s is not False, n is None, s is not None, flag is None)
o = json.loads("true")
print(o is True, o is False, json.loads("1") is True)


def calls():
    print("evaluated")
    return 7


print(calls() is None)
