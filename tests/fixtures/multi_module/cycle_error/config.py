"""Imports `loader` before binding `READY`."""

DEBUG = True
import loader  # noqa: E402

READY = loader.LEVEL > 0
