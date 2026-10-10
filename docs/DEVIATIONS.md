# Deviations from CPython

Behavior that differs from CPython 3.13 on purpose or because it is not implemented yet.
Each entry: what differs, why, and whether it is planned to go away.

| Behavior | pycc-next | CPython | Status |
|---|---|---|---|
| An `int` beyond 63 bits stored in a container (`list[int]`/`set[int]`/`dict` element) or used as a slice bound | raises `OverflowError` (element) or `IndexError` (list slice bound) at run time; a literal in those positions is compile error `T0051` | stores it / clamps the bound | not implemented yet; arithmetic, comparison, conversion, printing and the `--ext` boundary are arbitrary precision, and an index or repeat count raises CPython's own `IndexError`/`OverflowError` |
| `int ** negative int` | raises `RuntimeError` (`**` is typed `int -> int`) | returns a `float` | to be fixed in the type checker |
| `**` or `<<` producing more than 2**36 bits | raises `OverflowError: too many digits in integer` | raises `MemoryError` (or runs out of memory) | on purpose: pycc has no `MemoryError` |
| `str()`/`print` of an `int` with more than 4300 digits | always raises `ValueError` (the default limit) | limit adjustable with `sys.set_int_max_str_digits()` | `sys` is not compiled |
| An `--ext` module's attribute rebound from Python (`mod.f = g`) | compiled callers inside the module still call the original `f` | module-level lookups see `g` | on purpose: calls between compiled functions bind at compile time (a function defined once and never assigned is called directly) |
| `xs += ys` / `xs *= n` on a list, and the equivalent `xs = xs + ys` | compile error `C0001` | extends/repeats the list in place | the in-place form is not implemented yet; both spellings are refused because they lower to one shape |
| `print`/`str()`/f-string of a list whose elements are class instances | compile error `C0001` | calls each element's `__repr__` | not implemented yet |
| A list element type other than `int`/`float`/`bool`/`str`/class instance/list of those (e.g. `list[set[int]]`) | compile error `T0034` | runs | not implemented yet |
| Unsupported but valid Python (e.g. `sorted`, `lambda`, generators) | compile error `C0001` naming the construct | runs | each is a gap, never silent miscompilation |
