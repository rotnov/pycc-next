# Deliberately omits `from typing import Final`: pycc has no
# `Stmt::Import` support (any `import` is rejected with C0001), and CPython
# 3.14 defers annotation evaluation by default (PEP 649/749), so the bare
# `Final` name here is never evaluated by the pinned 3.14.7 oracle
# either. pycc recognizes `Final` as a bare name and unwraps `Final[X]`
# to `X` at HIR-lowering time, tracking the binding as non-reassignable
# (T0045 on reassignment). Variable-level annotations only.
MAX_CONNECTIONS: Final[int] = 100
APP_NAME: Final[str] = "pycc"

def compute() -> int:
    return MAX_CONNECTIONS + 1

print(MAX_CONNECTIONS)
print(APP_NAME)
print(compute())
