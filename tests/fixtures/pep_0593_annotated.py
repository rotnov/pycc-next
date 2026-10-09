# Deliberately omits `from typing import Annotated`: pycc has no
# `Stmt::Import` support (any `import` is rejected with C0001), and CPython
# 3.14 defers annotation evaluation by default (PEP 649/749), so the bare
# `Annotated` name here is never evaluated by the pinned 3.14.7 oracle
# either. pycc recognizes `Annotated` as a bare name and unwraps
# `Annotated[X, ...]` to `X` at HIR-lowering time, discarding metadata.
MAX_SIZE: Annotated[int, "max buffer size"] = 1024
GREETING: Annotated[str, "greeting message"] = "hello"

def process(x: Annotated[int, "input value"]) -> int:
    return x + 1

print(MAX_SIZE)
print(GREETING)
print(process(41))
