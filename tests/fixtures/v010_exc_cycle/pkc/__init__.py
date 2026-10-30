"""A layout class whose subclass in another module is subclassed back in the
first, beside `OSError` (`tests/v010_exceptions.rs`)."""
from .a import A, C, make
from .b import B

__all__ = ["A", "B", "C", "make"]
