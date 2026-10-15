"""`from m import x` of a name the partially initialized `m` has not bound
yet raises CPython's circular-import `ImportError`."""

print("main: start")
import config  # noqa: E402

print("main: unreachable", config.READY)
