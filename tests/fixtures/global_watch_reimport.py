# A global another module binds while this module's body runs, read once by
# compiled code in each exec of the module (the driver imports it twice;
# `hook.py` is a CPython module).
import hook

RUN: object = None
hook.bind()


def run() -> object:
    return RUN


print("run", run())
