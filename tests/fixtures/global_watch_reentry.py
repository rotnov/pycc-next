# A compiled read run by the read before it: the value a read replaces is
# freed there, and its `__del__` rebinds the global and reads it again
# (`hook.py`, a CPython module, does both, and rebinds it first: no
# compiled code reads such a rebinding back).
import hook

X: object = hook.Last()


def get() -> object:
    return X


print(type(get()).__name__)
hook.rebind("X", "rebound")
print(get())
