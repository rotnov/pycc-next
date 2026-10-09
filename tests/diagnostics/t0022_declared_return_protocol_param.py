from typing import Protocol

# #949: `f` has a written return annotation, so its body returning an
# incompatible value is a declared/actual mismatch -- not a private helper's
# conflicting *inferred* return type. Neither the protocol nor `f` being
# public is the discriminator; the annotation is. The `1:1` span is D-043's
# placeholder for every `pycc_types` diagnostic, tracked by #877, and is not
# specific to this fixture.


class P(Protocol):
    def foo(self) -> int: ...


def f(p: P) -> int:
    return p
