__all__ = ("loads", "load_pairs", "Table", "ParseError")
__version__ = "1.0"

from ._core import Table, load_pairs, loads
from ._errors import ParseError
