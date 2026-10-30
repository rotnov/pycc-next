"""A wildcard import of a module's exception classes (`tests/v010_exceptions.rs`)."""
from .combo import Combined, combine
from .common import Coded, coded

__all__ = ["Coded", "Combined", "coded", "combine"]
