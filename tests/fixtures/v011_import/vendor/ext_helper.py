"""A module outside the package whose attributes come from `__getattr__`, which
calls compiled code running a `from json import ...` statement of its own."""


def __getattr__(name):
    if name in ("a", "b"):
        from pkimp import imports
        return name + imports.again()
    raise AttributeError(name)
