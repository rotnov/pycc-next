"""v0.8 accept 4: string annotations naming `TYPE_CHECKING`-only imports
(`"State"`, `'''State'''`) in a module whose annotations CPython never
evaluates."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .state import State


def advance(state: "State", n: int) -> int:
    if state.pos + n < 0:
        raise ValueError("before the start")
    state.pos += n
    return state.pos


def describe(state: '''State''', marker: str) -> "str":
    return marker + state.src[: state.pos] + marker
