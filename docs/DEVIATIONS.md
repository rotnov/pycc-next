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
| Unsupported but valid Python (e.g. `sorted`, `lambda`, generators) | strict: compile error `C0001` naming the construct; non-strict: the enclosing top-level function, class or statement runs in CPython (`--report-fallbacks`) | runs | each is a gap, never silent miscompilation |
| Non-strict fallback: compiled code reads a module global that fallback code binds, before it is bound | `KeyError: 'name'` (the read is a lookup in the module dict) | `NameError: name 'name' is not defined` | to be fixed |
| Non-strict fallback: a type error in non-strict code (`T00xx`) | the statement falls back and runs in CPython (which raises or not at run time) | runs | by design: non-strict code has CPython's semantics where pycc cannot prove types |
| Non-strict fallback in a module imported by the entry module | the diagnostic is reported; only the entry module falls back | runs | not implemented yet |
| Non-strict fallback: a compiled class used by fallback code in a native (embedded) build | the class falls back too (an embedded `__main__` publishes compiled functions, not classes) | n/a | not implemented yet |
| An `--ext` module mixing compiled `print` with Python-side output (fallback code, a foreign call) when the host's stdout is a pipe or file | compiled output is written at once, Python's is block-buffered, so lines can come out of order (an embedded executable runs CPython unbuffered and keeps the order) | program order | to be fixed: route compiled `print` through `sys.stdout` in `--ext` |
| Compiled code (not fallback code) raising an uncaught exception | prints only the last traceback line | prints the full traceback | not implemented yet |
| Non-strict mode: a CPython object passed to a typed `int`/`float`/`bool`/`str` parameter (or returned as one) | checked at the boundary: `TypeError: f() argument 1 must be int, not str`; an `int` for a `float` slot becomes a `float` | the body runs with the object and fails (or not) later | by design: typed code keeps native representations |
| Non-strict mode: a name holding a native container (`xs: list[int]`, or a list built natively) passed into an `object` slot | compile error `T0021` (a display such as `f([1, 2])` and a local that only ever flows into objects are built as CPython containers instead) | the same list object is shared | a copy would break aliasing; refused rather than copied |
