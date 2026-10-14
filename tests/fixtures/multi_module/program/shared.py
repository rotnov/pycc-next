"""Imported by two modules: its body runs once."""

print("shared: body")
_calls: list[int] = [0]


def suffix() -> str:
    _calls[0] += 1
    return "!"


def calls() -> int:
    return _calls[0]
