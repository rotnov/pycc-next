"""v0.8 accept 4: `cast(Callable[..., str], ...)` of a CPython object in a
method looking its renderer up by name (`--ext` only: a native program's
compiled instances do not answer `object.__getattribute__` for a method)."""

from typing import Callable, cast


class Renderer:
    def text(self, t):
        return "text:" + t

    def _get_method(self, name):
        try:
            return cast(Callable[..., str], object.__getattribute__(self, name))
        except AttributeError:
            raise AttributeError('No renderer "{!r}"'.format(name))


r = Renderer()
print(r._get_method("text")("hi"))
try:
    r._get_method("missing")
except AttributeError as e:
    print("AttributeError", e)
