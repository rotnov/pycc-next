# Binds globals of the module `m` from outside it while its body runs.
import sys

runs = 0


def bind():
    global runs
    sys.modules["m"].RUN = runs
    runs += 1


def rebind(name, value):
    setattr(sys.modules["m"], name, value)


class Last:
    def __del__(self):
        m = sys.modules["m"]
        m.X = "set by __del__"
        print("in __del__:", m.get())
