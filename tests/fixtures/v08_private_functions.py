"""v0.8 accept 4: private module-level functions are attributes of the
module, whatever immutable values they take."""


def _wrap(text: str, width: int) -> str:
    return "<" * width + text + ">" * width


def _repeat(s: str, n: int) -> str:
    out = ""
    for _ in range(n):
        out += s
    return out


def public(s: str) -> str:
    return _wrap(_repeat(s, 2), 1)


print(public("z"))
