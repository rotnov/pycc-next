"""Imports `registry` back while `registry` is still running its body: its
functions are not defined yet."""

import registry


def run(n: int) -> int:
    return registry.lookup(n) + 1


def probe() -> int:
    # CPython reads `registry.lookup` on the partially initialized module
    # (AttributeError); pycc raises NameError (docs/DEVIATIONS.md). Either
    # way the failure is catchable and the import goes on.
    try:
        return run(0)
    except (AttributeError, NameError):
        print("plugins: registry.lookup is not bound yet")
    return -1


print("plugins:", probe())
