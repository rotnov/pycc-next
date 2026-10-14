"""A small untyped package, every module of which is built as its own
--ext --non-strict extension (tests/package_ext.rs)."""

from .version import __version__
from .core import __doc__, Differ, Patch, PREFIX

__packager__ = "textkit"
