"""A function-local import spelled like a module-level one (`tests/v010_exceptions.rs`)."""
from .a import A, make
from .b import B, other

__all__ = ["A", "B", "make", "other"]
