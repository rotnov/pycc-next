# Stays Python: `_core` raises this class as a CPython object.


class ParseError(ValueError):
    def __init__(self, msg: str, doc: str, pos: int) -> None:
        super().__init__(f"{msg} (at {pos})")
        self.doc = doc
        self.pos = pos
