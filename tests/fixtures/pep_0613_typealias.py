# Deliberately omits `from typing import TypeAlias`: pycc has no
# `Stmt::Import` support (any `import` is rejected with C0001), and CPython
# 3.14 defers annotation evaluation by default (PEP 649/749), so the bare
# `TypeAlias` name here is never evaluated by the pinned 3.14.7 oracle
# either. See tests/conformance.rs's pep_0613_typealias test for detail.
IntAlias: TypeAlias = int


def double(x: IntAlias) -> IntAlias:
    return x * 2


print(double(21))
