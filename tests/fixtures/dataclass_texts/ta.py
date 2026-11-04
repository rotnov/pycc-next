from dataclasses import dataclass

import sync


@dataclass(kw_only=False)
class A:
    x: int = 1
    sync.hold("a")
