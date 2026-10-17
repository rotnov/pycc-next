"""v0.8 accept 2: a nested `def` or `lambda` that rebinds a name a sibling
closure captures stores into the same cell: conditionally rebinding a
parameter (mistune's `add_toc_hook`), unconditionally rebinding a local, a
`lambda` assigned to the name, and a rebinding after the closure is made
(late binding)."""

from typing import Any, Callable, Optional


def make(heading_id: Optional[Callable[[int], str]] = None) -> Any:
    if heading_id is None:
        auto = True

        def heading_id(index: int) -> str:
            return "toc_" + str(index + 1)

    else:
        auto = False

    def hook(n: int) -> str:
        return heading_id(n) + ("!" if auto else "")

    return hook


def unconditional(tag: str) -> Any:
    label = None

    def label(n: int) -> str:
        return tag + str(n)

    def show(n: int) -> str:
        return label(n)

    return show


def with_lambda(scale: Optional[int]) -> Any:
    if scale is None:
        scale = lambda v: v * 10

    def apply(v: int) -> int:
        return scale(v) if callable(scale) else v * scale

    return apply


def late() -> Any:
    def read() -> str:
        return step()

    def step() -> str:
        return "first"

    first = read()

    def step() -> str:
        return "second"

    return first, read()


print(make()(1), make(lambda i: "x" + str(i))(2))
print(unconditional("t")(3))
print(with_lambda(None)(4), with_lambda(3)(4))
print(late())
print(make(None)(0).upper(), make()(-2))
make(5)(1)
