print("init sub", __name__, __package__)
from . import circle


def __getattr__(name):
    if name == "lazy":
        return "lazy value"
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
