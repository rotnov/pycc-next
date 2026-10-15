# Type aliases for annotations only: `_core` imports them under
# `if TYPE_CHECKING:`, so CPython never runs this module for `_core`.
from typing import Any, Callable, Tuple

Pos = int
Key = Tuple[str, ...]
ParseFloat = Callable[[str], Any]
