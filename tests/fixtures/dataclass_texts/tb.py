from dataclasses import dataclass

import sync


@dataclass(kw_only=False)
class B:
    y: str = "b"
    sync.hold("b")
