# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added

- Compile speed (v0.5 accept 4): building diff-match-patch takes 5.2 s on the CI Linux
  runner (v0.4: ~25 s), tomli 2.8 s and pypng 1.0 s, and every `bench/py` program
  builds in under 1 s; CI fails a package over 10 s or a benchmark build over 2 s
  (`bench/package_check.py` prints each module's build time, `bench/run.py
  --build-only` each program's). The non-strict specialization loop runs independent
  attempts on threads (the program without specializations beside the first attempt,
  every suspect of a span-less failure alone at once) and a failing check runs once,
  not twice; the type checker hashes with FxHash and the compiler allocates with
  mimalloc; a large optimized module is emitted as several objects by LLVM's backend
  on several threads; the `--ext` shim and an embedded build's launcher and sidecar
  are compiled and assembled while LLVM runs. `PYCC_TIME=1` prints per-phase timings.

- A third real package (v0.5 accept 2): `bench/package_check.py` adds
  [pypng](https://pypi.org/project/pypng/) 0.20220715.0 (MIT, a pure-Python PNG codec,
  sdist pinned by sha256), a single module (the table now takes one as well as a
  package). `png` builds unchanged with `--ext --non-strict`, its own 103 tests pass
  against it (CI) and reading a 256x256 RGB image that uses all five PNG filters runs
  10x faster than in CPython (`python3 bench/package_check.py pypng --time --runs 5`:
  0.015 s vs 0.157 s); 15 of its 85 functions compile, its `Reader`/`Writer` classes
  still run in CPython.
- Untyped byte loops run native: a guarded specialization also guesses `bytearray` for a
  parameter whose items are used as numbers (`(x + a) & 0xff` with `x = s[i]`, an index
  local computed from an `int` parameter counts as its use too) and takes the caller's
  `bytearray` as it is: items are read and stored in its own buffer, its size and buffer
  pointer held in registers until something may run CPython code (then re-read), with
  `bytearray`'s own `IndexError`/`ValueError`s (the stored byte is checked before the
  index, as CPython does). pypng's `undo_filter_*` loops run 25x-50x faster per byte.
- `abs()` of a small `int` is computed inline instead of calling the runtime.
- Native variadic tuples (v0.5 accept 1, partly): `tuple[T, ...]` of `str`, class
  instances or containers is a native tuple -- indexing, slicing, `+`, `*`, `in`,
  comparison, hashing, iteration, unpacking, dict keys and set elements without the C
  API -- and a fixed tuple of matching elements is one with no conversion. It crosses
  into CPython as a copy made once and remembered, so `f(t) is t` holds for a tuple that
  came from CPython; a strict `--ext` `tuple[str, ...]` parameter takes a `tuple` of
  `str` (anything else raises `TypeError`), and a variadic tuple of mutable elements is
  refused at the boundary (`C0003`) rather than copied. `tuple[int, ...]` and the other
  scalar ones are `T0053` in strict mode (not implemented yet).
- Native tuples of objects: under `--non-strict` and at the `--ext` boundary,
  `tuple[int, Any]` keeps its position native and its value the CPython object itself,
  and a native tuple returned where a tuple with object slots is declared is converted
  element by element (`return pos, True` from `-> tuple[int, Any]`).

### Fixed

- Compiled callers see every rebinding of a module-level function (#1): a
  module-scope binding of a name a `def` also binds (`f = g`, a loop or `with`
  target, ...) is refused with `C0001` (in non-strict mode it runs in CPython, with the
  `def`s of the name, so compiled code reads the name from the module's namespace); a
  `def` or lambda that runs in CPython fallback and rebinds a compiled function's name
  (in a block, through `global`) makes its `def`s run there too; a value of a
  function defined more than once (`g = f`, then another `def f`) keeps the
  definition of that moment. Before, compiled callers kept calling the old function or
  raised `NameError`.
- A guarded specialization calls a builtin its original reaches through CPython (`abs`
  of an object) natively when its typed arguments allow (and as the original does when
  they do not), instead of always through CPython.
- A function was not specialized when its original failed to compile in a way reported
  without a source span in the specialized attempt (a module with no imports and an
  `abs()` of an object): such a failure is the original's, not the copy's.
- A string index or slice that raised (`s[0]` of `""`, `s[::0]`) leaked its 72-byte
  placeholder result; a list or variadic tuple sliced with a zero step leaked an empty
  list.

### Performance

- tomli's keys are native `tuple[str, ...]` values. `loads()` stays at 1.4x-1.7x
  CPython (2.70M against 4.56M instructions per call under callgrind; v0.4: 2.60M): the
  object reference traffic halved, and native tuple allocation took its place. The v0.5
  accept 1 target (>= 3x) is not met; see `docs/ROADMAP.md`.

## [0.4.0] - 2026-10-15

Fast untyped code, no leaks. `--non-strict` keeps untyped code native: strings index in
O(1), names widen per function, and public untyped `--ext` functions get guarded native
copies, so diff-match-patch's own speed test runs 11.5x faster than CPython (v0.3:
1.7x). A second real package, tomli (the parser behind `tomllib`), builds unchanged and
passes its own suite, 1.67x faster than CPython. Exceptions are reference counted and
freed. Strict mode compiles 171/200 real-world programs unchanged (v0.3: 161), and
`--non-strict` runs all but 8 of their 1191 top-level statements natively (v0.3: 23).
Circular imports, `IntEnum` as `int`, sorting class instances and `dataclasses.fields`
/ `asdict` work as in CPython.

Benchmarks (`python3 bench/run.py`, best of 7, shared 4-core box): fib 18.9x faster than
CPython / 2.2x of Rust, mandelbrot 35.2x / 0.98x, nbody 32.8x / 2.6x, sieve 21.3x /
2.5x, spectral_norm 47.5x / 1.8x, collections_bfs 6.5x / 2.7x, numpy kernels stencil2d
459x / 2.8x and prefix_sum_np 95x / 2.1x.

### Added

- Untyped code runs native (v0.4 accept 1): an `--ext --non-strict` module's unannotated
  function or method that loops gets a guarded native specialization -- a copy whose
  parameters are typed `str` or `int` from how its body uses them, called when the
  arguments are exactly those types (any other call runs on the CPython objects as
  before; a copy the checker refuses is dropped, never fallen back). diff-match-patch's
  own speed test now runs 12x faster than CPython (1.03 s vs 12.3 s) (v0.3: 1.7x), its suite passing
  (`python3 bench/package_check.py --time`). `--report-fallbacks` lists `specialized:`
  and `not specialized:` functions.
- A second real package (v0.4 accept 4): `bench/package_check.py` is a table of pinned
  packages and adds [tomli](https://pypi.org/project/tomli/) 2.5.0 (MIT, the TOML parser
  `tomllib` comes from; GitHub tag archive pinned by its sha256 and that of the files used). Its
  four modules build unchanged with `--ext --non-strict --foreign-relative-imports`, its
  own test suite passes against them (CI) and 36 of 40 functions compile; `loads()` runs
  1.6x faster than in CPython (`python3 bench/package_check.py tomli --time --runs 15`,
  best of 15 runs of 100 calls on one machine: 0.036 s vs 0.059 s, 1.52x-1.75x over seven
  runs; was 0.40x). What typed packages written for mypyc needed:
  - `from __future__ import annotations`: an annotation pycc cannot lower is erased
    instead of making its statement fall back, and fallback code is compiled with the
    flag; names imported under `if TYPE_CHECKING:` from a sibling module that are
    aliases (`Pos = int`, `Key = tuple[str, ...]`) are read through to their definition.
  - A function whose signature the `--ext` boundary cannot carry (`tuple[int, str]`
    result, `frozenset` parameter) is compiled for the module's callers and runs as its
    CPython *twin* when Python calls it (`twin:` in `--report-fallbacks`).
  - Non-strict signature wrappers: defaults of unannotated or object parameters, and a
    Python caller passing a value of another type than a scalar parameter's runs the
    function's CPython twin, so its own error handling answers (`loads(b"")` raises
    tomli's `TypeError`, not the boundary's).
  - A module function with an `Iterable[T]` parameter gets a `frozenset[T]`
    specialization that the module's own calls with a native frozenset run.
  - Native `frozenset(str)`, `frozenset(chr(i) for i in range(n))` (a `chr` whose
    argument is provably not a surrogate is native in non-strict mode too), set
    displays in membership tests, `isdisjoint`/`issuperset`/`issubset` with a `str`,
    `str.startswith(p, start)`, `find`/`index(sub, start)`.
  - Non-strict: `isinstance(o, (A, B))` of an object, `repr(o)`, `raise o from cause`,
    `type(native)`, an object rebinding an `int`/`float`/`bool` parameter (converted),
    an object passed for a `frozenset[str]`/`frozenset[int]` parameter (copied), a
    tuple display returned as `tuple[...]`, an object in a native frozenset, a `try`
    and its handler binding a name with two types, `Callable[...]`, `Iterable[...]`
    and `tuple[X, ...]` annotations as objects, `self.x = {}` as an object attribute.
- Faster code around CPython objects and tuples (v0.4 accept 4):
  - Multi-value returns: a function whose every `return` is a tuple display of one
    arity, or forwards another such function, gets a compiled twin that returns the
    values without a tuple, and a caller that unpacks the call (`pos, value = f(...)`)
    calls it -- no CPython tuple (nor pycc tuple, for a native `tuple[int, str]`) is
    built, unpacked and freed. Result types are what the returns agree on, an object
    where they differ; the function itself serves every other use, and a call whose
    name may be rebound runs it (`tests/multi_return.rs`).
  - `a, b = x, y` with object values binds the targets without building two tuples
    (`tests/object_swap_unpack.rs`).
  - Non-strict: a `str | None` local bound only to native `str` values and `None`
    (`char: str | None = src[pos]` under postponed annotations) stays a native `str`,
    null standing for `None`; its literal comparisons, `frozenset[str]` membership and
    `is None` tests are native and any other use sees the object it stands for
    (`tests/optional_str_local.rs`).
  - `while s[i] in chars: i += 1` (or `not in`) over a set of `str` is one scan of the
    text, the `IndexError` of running off either end raised as the loop raises it
    (`tests/str_scan_loop.rs`).
  - In a condition, an object compared with a `str` literal, or tested against a native
    frozenset of `str`, is compared by its UTF-8 bytes when it is an exact `str`, with
    no `str` made for the literal and no CPython copy of the set; anything else takes
    CPython's own operation (`tests/object_text_compare.rs`).
  - Non-strict object operations take shorter paths: `o[i]` and `o[a:b]` with native
    `int` keys read an exact tuple or list directly, a tuple display of objects is
    built as a tuple, `o.m(args)` with literal or parameter arguments is one
    `PyObject_VectorcallMethod`, a rich comparison in an `if`/`while` answers a native
    `bool` (two exact `str` operands are compared without a result object), and
    `isinstance` of a plain built-in value skips the `__class__` lookup
    (`tests/object_fast_paths.rs`).
  - A string literal reads its site's cached, immortal `str` with no call; freeing a
    list or an instance no longer allocates; error paths are laid out of line; the
    runtime builds as one codegen unit.
- Crossing to CPython is cheaper: a native `str` or frozenset converted to a CPython
  object keeps that object and shares it on its next crossing (a `str` a Python caller
  passed in shares the caller's); `str` literals are built once per site; attribute
  names are interned once; the C shim is compiled with `-O2`; the pending-exception test
  after each runtime call no longer reads thread-local storage while no thread has an
  exception pending; `c in frozenset_of_str` for one ASCII character is a bit test.
- Circular imports between project modules run as in CPython (v0.4 accept 5): a module
  imported back sees the partially initialized one -- the names it bound before the
  import -- and `from m import x` of a name `m` has not bound yet raises CPython's
  `ImportError` with its message. Only the entry module imported back is still refused
  (`E0108`, now "import cycle through the entry module").
- Language gaps from v0.3 (v0.4 accept 5): `IntEnum` members are usable where
  an `int` is needed (arithmetic, comparisons, `range()`, indexing, `int` parameters);
  `sorted()`, `min()`, `max()` and `list.sort()` of class instances order by `__lt__`
  (or `__gt__`, or `@dataclass(order=True)`), also with `key=` returning instances and
  `reverse=`; a list changed while it is sorted raises `ValueError: list modified during
  sort` as in CPython; `x in s` for a set of instances uses `__hash__`/`__eq__`;
  `dataclasses.fields()`, `asdict()`, `astuple()` and `replace()` compile natively
  (`asdict()` of mixed field types needs `--non-strict`); printing a list or tuple of
  instances uses their `__repr__`; a strict `--ext` module publishes its native
  module-level variables (numbers, strings, tuples of them) as attributes, kept current
  when a function rebinds them with `global`.
- Format specs and printf-style formatting compile natively: `f"{x:>10,.2f}"`,
  `format(x, spec)` and `"%5d|%-8.3f|%s" % (...)` over `int`, `bool`, `float` and `str`
  follow CPython's format-spec mini-language and `%` conversions exactly, errors
  included (`pycc_rt::format`; `tests/format_spec.rs`). A conversion CPython refuses
  for its argument's type (`'%x' % 1.5`) is a compile-time `T0021` in strict mode; a
  spec with a nested field (`{x:{w}}`) is still `C0001`.
- Strict mode admits a read of a name bound on some paths only: it is checked at run
  time and raises CPython's `UnboundLocalError`/`NameError` where CPython does
  (`tests/maybe_unbound_runtime.rs`). `T0041` stays for a name CPython would read
  something else for when unbound (a builtin, `__name__`), a `del`/`except ... as`
  name, and a buffer.
- Non-strict: a comprehension over a CPython object whose elements are native lists,
  sets, dicts or tuples collects a native list; `set(o)` of an object and an empty
  `set()`/`dict()` the empty-container pass cannot type are CPython's own; a name
  rebound from one container kind to another (`t = [..]`, then `t = {..}` on some
  path) becomes an object; `t |= {..}` on such a name and a `for` over a maybe-bound
  object run through CPython (`tests/non_strict_object_containers.rs`).
- `d.get(k)` with no default compiles (as `d[k] if k in d else None`, for a key that is a
  name, a literal or a subscript of those): an `Optional` of the value type. An
  `Optional[int|float|bool]` compares with `== None`/`!= None` (narrowing like
  `is None`) and renders with `print`, `str()` and f-strings
  (`tests/dict_get_optional.rs`).
- An unannotated `self.d = {}` is typed from the first `self.d[k] = v` in the class's
  own methods (it was `C0001`; with no such store it is now `T0003`), and an empty
  attribute container whose producer reads an unannotated method parameter is typed
  once the solver has typed the parameter (`tests/attribute_dict_producer.rs`).
- An empty `set()`/`dict()`/`list()` or attribute container whose producer reads an
  unannotated function's result (`s.add(dsu.find(i))`) is typed once the solver has
  typed the function; `s.update(it)`/`xs.extend(it)` type an empty container from any
  iterable; `sorted((a, b))` (and `min`, `max`, `sum`) of a tuple display compiles
  (`tests/solver_typed_containers.rs`).
- Non-strict: a name holding a native container that meets a CPython object (a call
  argument, an item store, a comparison) becomes an object when every binding of it is a
  fresh container -- a display, a comprehension, an empty constructor, or the result of a
  function that only returns lists nothing else holds -- so aliasing is kept; a list
  display nested in an object list, a list holding a big `int` literal, a
  `collections.defaultdict` the native dict cannot hold (built by CPython from its
  factory) and `sys.stdin.buffer` read as bytes are CPython's own; a chained comparison
  over objects whose middle operands are names or literals is the conjunction of its
  links (`tests/non_strict_widened_containers.rs`). The corpus meets v0.4 accept 3:
  strict 170/200, non-strict 200/200 with 8/1191 statements in CPython fallback.
- A module-level `pass` statement compiles; a bare `zip()` statement is dropped.
- The empty-container pass reads `d.get(k, 0)` when the call is also readable as a
  user or CPython method (non-strict), so `d = {}` filled that way stays native.

### Changed

- Native `str` indexing, slicing and `len()` are O(1) in the string's length: a string
  caches its code-point count and, when not ASCII, the byte offset of every 32nd code
  point; one-character ASCII strings are shared. `in` on `str` uses a substring search.
- Non-strict widening is per function: a name one function rebinds with another type
  becomes a CPython object in that function only, not in every function of the module.
- Non-strict: an object tested for membership in a native `str` (`c in "aeiou"`) is
  CPython's own test (was a fallback).

### Fixed

- An empty `frozenset()` was always `frozenset[int]`, so `frozenset("") == frozenset()`
  answered `False`: it is now typed by its uses, like `set()` (an equality, a set
  operator or a membership test with a typed set, or `frozenset(x)`'s argument)
  (`tests/empty_frozenset.rs`).
- Implicit concatenation of plain string literals and f-strings
  (`"allowed" f" {n} levels"`) dropped the plain parts.
- A method call on an imported module (`m.set(...)`, `builtins.set()`) was bound to the
  signature of a method of the same name a class of the module defines, filling in its
  defaults.
- A call of a module-level function before its `def` statement ran (from a function
  called earlier) raises CPython's catchable `NameError: name 'f' is not defined`, before
  any argument is evaluated, in native strict, `--non-strict` and `--ext` builds and
  across a circular import; it aborted the process (exit 134).
- The peak-RSS leak tests no longer fail at random when the suite runs in parallel. On
  Linux a child's `ru_maxrss` also counts the resident set of the test harness that
  spawned it (`exec` folds the replaced address space's high-water mark into it), so
  two probes compared how much the harness grew in between. Every probe now reads its
  own peak (`VmHWM`, at its exit) through one helper, `tests/memory_support`. The growth
  limit is 2 MB (was 1 MB): a probe embedding CPython grows by up to one 1 MiB obmalloc
  arena or not depending on the loop count; one leaked object per iteration still adds
  3-6 MB.
- A `NameError` for a global of an imported project module read before it is bound
  names it as written (`name 'LATE' is not defined`), not by the compiler's qualified
  spelling.
- Exception objects are freed (v0.4 accept 2): a raised or caught exception, its
  message, `__cause__`, group members and a `StopIteration`'s value are reference counted
  and released with the last reference -- the end of an `except` handler (its `as`
  binding is unbound then, as in CPython), a `return`/`break`/`continue` out of it, a
  `finally` clause that replaces it, or whatever list, attribute or local kept it. A
  loop catching `ZeroDivisionError` no longer grows by ~220 bytes per iteration, nor one
  catching a failing `json.loads` by ~1.9 kB, in strict, `--non-strict` and `--ext`
  builds (`tests/memory_leaks_exceptions.rs`, `tests/memory_leaks_ext.rs`). A CPython
  exception bridged into compiled code now carries its original object, which is
  released with it, instead of a per-call table of originals.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.

## [0.3.0] - 2026-10-14

Packages, stdlib, everyday code. Programs of several modules and packages compile as
one unit; dataclasses, enums, `functools.cache`, `deque`/`Counter`/`defaultdict` and
`itertools` compile natively; a real PyPI package (diff-match-patch) builds unchanged
as `--ext` modules and passes its own test suite in CI; strict mode compiles 161/200
real-world programs unchanged (was 124) and `--non-strict` runs 98.1% of their
top-level statements natively (was 95.1%). Prebuilt archives for Linux x86_64, Linux
aarch64 and macOS 26+ on Apple silicon.

Benchmarks (`python3 bench/run.py`, best of 7, shared 4-core box): fib 19.9x faster
than CPython / 2.0x of Rust, mandelbrot 43.9x / 0.96x, nbody 29.4x / 2.6x, sieve 25.0x
/ 1.9x, spectral_norm 46.1x / 1.7x, collections_bfs 6.8x / 2.9x, numpy kernels
stencil2d 677x / 2.3x and prefix_sum_np 78x / 1.7x.

### Added

- A real PyPI package, unchanged: `bench/package_check.py` (CI-gated on Linux x86_64)
  builds every module of diff-match-patch 20241021 with `--ext --non-strict` and runs the
  package's own test suite with CPython against the compiled modules; every function
  and method compiles (v0.3 accept 4).
- `--ext`: a package's `__init__.py` builds to `pkg/__init__.abi3.so` (`-o pkg/__init__`),
  imported as the package; under `--foreign-relative-imports` a module's `__name__` is
  its dotted name (`pkg.mod`); the module docstring is `__doc__`; a class's literal
  constants (`INSERT = 1`) are attributes of its published type.
- Non-strict `--ext`: module-level variables and imports are module attributes
  (`mod.X`, `from mod import X`, a package `__init__`'s re-exports).
- Non-strict: `o[a:b] = v` on a CPython object; a class attribute initialized with any
  expression (`PATTERN = re.compile(...)`), evaluated once where the class runs; a
  method that can run off its end returns `None`; an in-module call fills an `object`
  parameter's `None` or scalar default; a builtin exception built from an object message
  (`ValueError("bad: " + obj)`).
- Non-strict typing: an instance attribute nothing types is an object; an attribute
  stored values of two types becomes an object; a local first bound to `None` and later
  to something else is an object; a helper parameter or return inferred both as an
  object and as a scalar is an object; a helper returning a tuple or `None` returns an
  object; a container held by a name that flows into an object tuple becomes an object.
- Linux aarch64: a `pycc-vX.Y.Z-aarch64-linux.tar.gz` archive (glibc 2.39+), built,
  tested and smoke-tested on GitHub's `ubuntu-24.04-arm` runner next to the other two;
  CI runs the full test suite and the corpus gate on Linux aarch64.
- `itertools` natively in strict mode (and in `--non-strict` and `--ext` code), with no
  embedded CPython: `count`, `islice`, `chain`, `chain.from_iterable`, `accumulate`
  (`operator` functions, `max`/`min`, two-argument lambdas of those, `initial=`),
  `pairwise`, `groupby` (with a `key`), `product` (with `repeat=`), `permutations`,
  `combinations` and `combinations_with_replacement`. Each returns a lazy iterator
  implemented in the runtime, consumed by `for`, `list()`, `sum()`, `next()`,
  comprehensions and every other generator consumer; inputs are lists, `str`s, `range`s
  and generators, pulled only as far as CPython pulls them.
- `collections.deque` natively: `deque()`, `deque(iterable[, maxlen])`, `append`,
  `appendleft`, `pop`, `popleft`, `extend`, `extendleft`, `rotate`, indexing, `len`,
  `in`, `index`, `count`, `remove`, iteration, `copy`, `+`/`*`, equality and `repr`
  (`deque([...], maxlen=n)`); `deque[T]` annotations. Both ends are O(1) amortized, and a
  BFS over a deque runs ~10x faster than CPython.
- `collections.Counter` and `collections.defaultdict` natively: `Counter()`,
  `Counter(list|str|dict)`, `c[k]` reading `0` for a missing key, `most_common([n])`,
  `update`, `subtract`, `elements`, `total`, `+`/`-`/`|`/`&`, equality and CPython's
  `repr` order; `defaultdict(int|float|str|list|set|dict|lambda|function)` with the
  factory run on a missing-key read, and nested defaultdicts. `Counter[K]` and
  `defaultdict[K, V]` annotations, and `defaultdict(list)` typed from its uses.
  `bench/py/collections_bfs.py` (a deque BFS plus a `Counter`) runs ~9x faster than
  CPython.
- Dict `|` (union) between two dicts.
- A list, set or dict comprehension over a generator (`[x for x in gen()]`).
- An unannotated generator helper whose yields the solver cannot type from their terms
  alone (a yielded tuple display, a value read from a local container) is typed from its
  body once its parameters are known; this includes every lifted generator expression.
- Programs of several modules and packages compile as one unit (natively in strict and
  `--non-strict` mode, and as an `--ext` module): `import m`, `import pkg.sub [as s]`,
  `from pkg.sub import f [as g]`, relative imports and `__init__.py`; `m.f(...)`,
  `m.CONST` and `m.Class(...)` are direct native calls and reads. Each module has its own
  namespace and `__name__`, and its body runs once at its first import, in CPython's
  import order. An unresolved module or name, a module used as a value and an import
  cycle are compile errors (`T0021`, `C0001`, `E0108`).
- `dataclasses.dataclass`, natively in strict mode: `@dataclass` and
  `@dataclass(...)` with `init`/`repr`/`eq`/`order`/`frozen`/`unsafe_hash`, field
  defaults, `field(default=..., default_factory=..., init=, repr=, compare=)`,
  `__post_init__`, inherited fields, generated `__init__`/`__repr__`/`__eq__`/
  ordering/`__hash__`, and `dataclasses.FrozenInstanceError` on assignment to a frozen
  field. An `--ext` dataclass takes keyword arguments and prints as in CPython.
- `enum`: `IntEnum` beside `Enum`/`StrEnum`; members print, `str()`, `repr()` and
  format exactly as CPython's (`Color.RED`, `<Color.RED: 1>`, `2` for an `IntEnum`),
  `Color(value)` looks a member up (`ValueError: 4 is not a valid Color`), `case
  Color.RED:` patterns, members as set elements and dict keys, `IntEnum` ordering.
- `functools.cache` / `functools.lru_cache` (bare, `()`, `maxsize=None`, `maxsize=N`
  with least-recently-used eviction, `maxsize=0`) on module-level functions with
  `int`/`float`/`bool`/`str` parameters, and `f.cache_clear()`. Any other decorator
  still runs in CPython under `--non-strict`.
- Classes: `print`/`str()`/f-strings call a class's `__str__` or `__repr__`, the
  `repr()` builtin and f-string `!r`/`!s` work, `==`/`!=`/`<`/`<=`/`>`/`>=` between
  instances call the class's dunders, `==` without `__eq__` and `is` compare identity,
  and a call of a constructor with defaults may omit them. `--ext` classes install
  `__repr__`/`__str__` as `tp_repr`/`tp_str`.
- `hash()` of a `str`, a `float` and a tuple holding them.
- Strict mode infers more of real-world untyped code (the corpus, v0.3 accept 3):
  - a return the solver cannot type from its terms (a returned tuple display such as
    `return up, down`, a value computed from a local it did not type) is typed from the
    function's body once its parameters are, and inference runs again with it, so its
    callers and the parameters they feed are typed too;
  - an empty list whose producer reads an unannotated function's parameter or return
    (`result.append(x % num)`) is typed once those are inferred;
  - an empty list bound to a name whose element nothing reads, produces or compares
    (`L = []` only rebound, measured or printed) is a `list[int]`: no operation can
    observe it;
  - an operator over two operands already typed is typed at once, so `a.count(m - 1)`
    types the parameter it is passed to.
- The builtins `exit()`/`quit()` and `from sys import exit` are `sys.exit` (run through
  the embedded CPython, so `finally` blocks run and the exit status and message are
  CPython's).
- A module-level `X = sys.stdin` / `X = sys.stdout` alias: `X.readline()`,
  `X.write(s)` and the other stream operations are the native ones.
- More template idioms of competitive-programming code compile natively: a module-level
  `if` on a flag bound once to a literal (`DEBUG = 0`) or on `sys.gettrace()` is the
  branch that runs; `next(sys.stdin)` (a line, `StopIteration` at EOF); a parameter
  whose default is a builtin conversion no call passes (`def rl(t=int)`); `x in c` of a
  constant tuple; `a, b = x, y` typing both targets for inference;
  `g[i].sort(reverse=True)`; a `def` returning `sys.stdin.buffer.readline()` read only
  as numbers.
- Non-strict mode: the inference solver runs again with what it settled written in, and
  a return it leaves open is typed from the body when every `return` agrees, so less
  code becomes a CPython object.
- A tuple whose elements share one type: `t[k]` with a computed index (int, bool and
  float elements; `IndexError: tuple index out of range` outside `[-len, len)`), and
  `for x in t` / a comprehension over it (any element a list can hold).
- A generator expression reading a module-level loop variable (`all(q % p for p in
  primes)` inside `for q in ...`) is typed; `graph[u].append(v)` types the empty list
  `graph`'s items start as (`{i: [] for i in range(n)}`); non-strict mode runs
  `any`/`all` of a generator of CPython objects natively.
- Item access on an instance of a module class defining `__getitem__`/`__setitem__`/
  `__delitem__` (through `self`, or a name bound only to calls of the class) calls those
  methods, and their unannotated parameters are inferred from those calls.
- `list()` and `dict()` with no argument are `[]` and `{}`; `set.update(g)` and
  `list.extend(g)` take a generator; a local rebound from a scalar (`temp = ord(c)`) to an
  empty list is a separate variable; non-strict mode passes a generator to a CPython
  object's method as an iterator over it.
- The real-world corpus (v0.3 accept 3): strict mode passes 161/200, non-strict mode runs
  98.1% of top-level statements natively.

### Changed

- Non-strict mode: `chr()` runs in CPython, whose `str` holds a lone surrogate
  (`chr(0xD800)`) that pycc's UTF-8 `str` cannot.
- `d[k] += n` on a dict of `int` values (a `Counter`'s `c[x] += 1`) is one lookup, and a
  `str` caches its hash as CPython does: a word-counting loop runs ~4x faster than
  CPython.
- Strict mode, like non-strict mode, does not compile an unannotated definition nothing
  in a native program's entry module names: it can never run and no call site could type
  it (previously `T0001`). An `--ext` module keeps every definition.
- A dependency module's top-level names no longer share the entry module's namespace:
  two modules may define the same name, and an `--ext` artifact no longer exports the
  public functions of the project modules it imports (only the entry module's own).

### Fixed
- `print()`, `str()` or an f-string of a value that may be `None` (`int | None`) passed
  `pycc check` and crashed the backend; it is refused with `C0001`.
- A `__str__` whose signature is not `(self) -> str` was called for `print()`/`str()` and
  its result used as a `str`; the conversion is now refused (`C0001`), as CPython raises
  `TypeError`.
- A tuple boxed inside a loop (`list(...)` of a generator of tuples, a tuple stored in a
  container) grew the stack on every iteration and crashed after ~10^5 iterations.
- A comprehension over a fresh list (a call's result, a `str` expression) leaked that
  list.
- A fresh native value passed to CPython leaked once per call: a `str` (a literal, `str(i)`,
  a concatenation, an f-string), a list, a bigint `int` or a class instance as a positional
  or keyword argument of a call on a CPython function or method, or a `str` subscript key
  of a CPython object (`json.dumps(str(i))` grew ~47 bytes per iteration; it now runs in
  constant memory, strict and non-strict).
- Non-strict: a function returning a tuple with a non-scalar element (`return text[:n],
  len(text) - n`) crashed the backend in a build with fallback code (and `--ext`), whose
  export thunk flattened the tuple as if it were a struct of scalars; such a return is now
  typed from the body like any other instead of falling back to a CPython object.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.

## [0.2.0] - 2026-10-12

Rust speed, numpy, more Python. Every benchmark is within 3x of hand-written Rust,
typed numpy arrays compile to native loads and stores, strict mode compiles 124/200
real-world programs unchanged (was 93), and `--non-strict` runs 95% of their top-level
statements natively (was 84%). Prebuilt archives for Linux x86_64 and macOS 26+ on
Apple silicon.

### Added

- Typed numpy arrays: `numpy.ndarray` / `NDArray[float64|int64|bool]` of 1 and 2
  dimensions are native in strict and `--ext` code: element reads and writes, `.shape`,
  `len`, iteration, `numpy.zeros/ones/empty/array`, with no CPython call in the loop
  body. Benchmarks `stencil2d` (436x faster than CPython, 2.5x of Rust) and
  `prefix_sum_np` (80x, 1.8x).
- macOS arm64: a `pycc-vX.Y.Z-aarch64-macos.tar.gz` archive (macOS 26.0+, Apple
  silicon), built and smoke-tested next to the Linux one; CI runs the full test suite
  and the corpus gate on macos-26.
- Strict mode infers the signature of an unannotated public function or method that
  only the module itself calls; conflicting call sites, `--ext` modules and exported
  functions still need annotations (T0001).
- `--non-strict` compiles more everyday code natively: CPython objects as indexes,
  slice bounds and `range()` arguments (`__index__`), mixed-type containers promoted to
  CPython containers, `str % args`, `str` methods and `sys.stdout.write` with object
  arguments, keyed `sort`/`sorted`/`min`/`max` and `heapq`/`bisect` on objects,
  `x is True`/`x is False`, names bound on only some paths (unbound reads raise
  `UnboundLocalError`/`NameError` as in CPython), `open(0)` as standard input,
  `sys.stdin.buffer` as bytes.
- `10 ** -10` is a float, as in CPython (was a compile error).

- The generator protocol, natively in strict mode: `send()`, `throw()`,
  `close()` (`GeneratorExit`, "generator ignored GeneratorExit"), `yield` as
  an expression, `return value` as `StopIteration.value`, `yield from`
  delegating all three and returning the subgenerator's value, PEP 479, and
  generators in module globals closed by `del` and at exit.
- Full call signatures: `*args`, keyword-only parameters, `**kwargs`,
  positional-only parameters and defaults of every kind, for functions,
  methods, constructors and nested functions, called with keywords, `*seq`
  and `**mapping`, with CPython's `TypeError` messages.
- Function values (module-level functions and closures) stored in lists,
  dicts and instance attributes and called from there (`fs[0](x)`,
  `table[k](x)`, `self.f(x)`); `print`, `str()` and f-strings render a
  function or a generator as CPython does (`<function f at 0x...>`,
  `<generator object g at 0x...>`).
- `--ext`: a public function or method with defaults, positional-only or
  keyword-only parameters, `*args` or `**kwargs` is published to Python
  callers with its declared signature (#1194); CPython binds the call and
  raises its own `TypeError`s.

### Fixed

- Debug builds (no `--release`) stored a `None`-typed variable -- `x = None`,
  or `got = yield v` in a generator whose send type is `None` -- as two bytes
  into its one-byte slot, overrunning the stack frame: depending on the
  layout a generator lost a yielded value or the program crashed. Codegen
  now refuses, as an internal error in both profiles, any store wider than
  the stack slot or global it writes.
- A nested function's plain assignment to a `nonlocal` variable (`k = v`, not
  only `k += v`) is seen by the enclosing function; it used to rebind a copy.
- `sys.stdin.buffer` read as `str` printed and compared unlike CPython's bytes.
- Security review: an object used as an index could leak a heap int; a
  `StopIteration.value` of another type is refused instead of reinterpreted;
  `gen.send(None)` is type-checked; numpy array headers are re-read after any call
  that may run Python.

### Performance
- Every benchmark is within 3x of its Rust twin (fib 1.8x, mandelbrot 1.1x, nbody 2.4x,
  sieve 2.1x, spectral_norm 1.8x; v0.1: up to 4.2x). Whole-program proofs drop checks that
  cannot fire: an attribute every constructor assigns before the instance escapes skips the
  "assigned" test; a function that cannot raise is called without the exception check; a
  `def` that precedes all module code skips the call-before-definition check. A tree of
  `int` operators tests its leaves' tags once; `x += y` releases the old value only on the
  bigint path; range loops with one level of nested loops count with a machine integer;
  `x ** 0.5` proves `pow == sqrt` with exact integer arithmetic.
- `bench/run.py`: best of 7 by default; Rust twins build for the default target CPU and
  black-box every input.
- `PYCC_DUMP_LLVM=<path>` writes the LLVM IR before and after optimization.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.

## [0.1.0] - 2026-10-11

First release of pycc-next: an ahead-of-time compiler for typed Python that turns
type mismatches into compile errors and runs 11-34x faster than CPython on the
benchmark set, interoperating with CPython through extension modules and an
embedded interpreter. Everything below in 0.1.0-rc.1 and 0.1.0-rc.2 is included.

Highlights:
- Strict mode (default): Rust-style diagnostics at the offending code; native
  representations; generators, closures, classes, exceptions, containers, bigints.
- `--non-strict`: untyped code compiles; unprovable parts run through CPython,
  per statement if needed. 200/200 real-world corpus programs match CPython.
- `--ext`: abi3 CPython extension modules (CPython >= 3.13).
- Reference-counted memory with no per-iteration leaks; security review fixes
  (keyed str hashing, int(str) digit limit, no host aborts on big ints).
- Benchmarks vs CPython 3.13 / hand-written Rust: fib 10.8x / 3.8x slower,
  mandelbrot 33.7x / 1.1x, nbody 23.3x / 4.2x, sieve 16.6x / 3.1x,
  spectral_norm 29.2x / 3.5x (see README).
- Self-contained Linux x86_64 release archive (needs `cc`; CPython 3.13+ with headers
  for `--ext` and `--non-strict`).

Known differences from CPython: docs/DEVIATIONS.md.

## [0.1.0-rc.2] - 2026-10-11

The release archive is now self-contained (rc.1 could not build programs). Changes
since rc.1:

### Added
- `global` statements: a module global a function rebinds lives in a one-element
  cell, so every function and the module see its writes (native in strict mode).
- `def`s inside module-level blocks (`for _ in range(t): def solve(): ...`) compile
  natively when used only after their definition in that block.
- Comprehensions with several `for`/`if` clauses, and starred call arguments
  (`f(*xs)`, `f(a, *t)`) with CPython's `TypeError` for a wrong count.
- Instance attributes initialised in `__init__` from any expression
  (`self.t = [0] * (n + 1)`): the slot takes the value's inferred type.
- `len()` of a tuple bound to a name.
- Generators, native in strict mode: `def`s with `yield` (annotated `Iterator[T]`,
  `Iterable[T]` or `Generator[T, None, None]`, or inferred for a private helper),
  `yield from` statements, `return` in a generator, `for`/`next()`/`list()`/`sum()`/...
  over them, and generator expressions, lazy (so `any()`/`all()` stop at the first
  deciding element). An unfinished generator is closed when its last reference goes,
  running its `finally` clauses. An `--ext` module's public generator returns a
  CPython iterator.
- Function values (`Callable[[A, B], R]`): module-level functions used as values and
  nested functions returned or passed as closures (capturing their enclosing variables,
  `nonlocal` counters included), called through a variable or a call's result
  (`make_adder(1)(2)`). A closure binds a variable when it is made, so one that reads
  a variable the enclosing function may still change stays unsupported.

### Fixed
- A heap `int` (beyond 63 bits) held by a function's parameters or locals is released
  when the function returns, and one passed to a builtin (`str(x * 2)`,
  `max(x, x + 1)`) after the call: big-int loops through functions run in constant
  memory (#625).
- Function values: a call keeps the called closure alive while its arguments run
  (an argument rebinding the variable that held it no longer frees it under the
  call); a closure capturing a variable rebound by `except ... as`, a `match`
  capture, a `type` alias or a comprehension walrus, or used before its `def`, is
  refused (strict) or runs in CPython (non-strict) instead of seeing a stale or
  unassigned value; the compiler's internal names (`__pycc_closure__`, `_pycc_*`)
  are reserved (`E0109`). A `--non-strict` fallback function annotated with a
  compiled `typing` import (`Callable`) no longer fails with `NameError`.
- The release archive is self-contained and relocatable (#5): it ships
  `bin/pycc` + `lib/pycc/libpycc_rt.a`, and `pycc` finds its runtime relative to
  its own executable instead of the checkout it was built in (`v0.1.0-rc.1` failed
  with "no pycc_rt build found (expected /home/runner/work/...)"). The release
  workflow smoke-tests the unpacked tarball (native, `--ext`, `--non-strict`)
  before publishing. A missing C toolchain is reported with what to install.
- `pycc version` reports the LLVM it was built against (from `llvm-config`)
  instead of a hard-coded `22.1.1`.

### Changed
- Type-checker diagnostics (`T0xxx`, and the `C0001`/`I0404` the checker raises) point
  at the offending expression or statement instead of `1:1`, with a short caret label
  (`expected `int`, found `str``) instead of the headline repeated; `--error-format
  json` carries the same line, column and length (#10).
- `--non-strict` native builds infer unannotated function signatures from their call
  sites, keep imports and `sys` stream aliases compiled when fallback code reads
  them, call CPython's builtin for a builtin call they do not compile, and keep
  scalar globals that fallback code only reads native: 83% of the corpus's top-level
  statements now compile natively (was 51%) (#11).
- `--non-strict`: fallback code calling a compiled function with a parameter default
  left out raised `TypeError`; such a function now runs in CPython.

## [0.1.0-rc.1] - 2026-10-10

First release candidate of the pycc restart: compile typed Python to native code,
catch type errors at compile time, interoperate with CPython.

### Added
- Compiler pipeline imported from pycc@aab8cc61: parser, HIR, strict type checker,
  MIR, LLVM 22 codegen, native runtime, Rust-style diagnostics.
- Build modes: native executables, CPython extension modules (`--ext`, abi3,
  CPython >= 3.13), and native executables that embed the installed CPython (>= 3.13)
  when the program uses CPython objects.
- `--non-strict` mode (or `[tool.pycc] strict = false`): untyped code compiles; values
  that can't be proven typed run through the CPython C API; constructs the compiler
  can't lower yet fall back per function / top-level block to CPython
  (`--report-fallbacks`).
- `break`, `continue`, `while ... else`, `for ... else` with path-precise
  definite-assignment analysis.
- Generic `list[T]` (int, float, bool, str, class instances, nested lists), `[x] * n`,
  slicing, `round()`, `input()`, common `str` methods and builtins.
- Arbitrary-precision `int` arithmetic with inline small-int fast paths.
- Benchmarks against CPython and hand-written Rust (`bench/run.py`) and a
  real-world corpus metric (`bench/corpus.py`).

- Reference counting for `str`, lists, dicts, sets, class instances and CPython
  objects; no per-iteration leaks; cycles are not collected (see DEVIATIONS).
- Empty-container inference, closures (`nonlocal` via cells), `lambda`, `key=` /
  `reverse=` for `sorted`/`min`/`max`, set operators, `math` integer functions.
- Shared stdin/stdout with CPython when it is embedded or hosting an `--ext` module.

### Security
- `str` hashing is CPython's keyed SipHash-1-3 (`PYTHONHASHSEED` honoured).
- `int(str)` enforces CPython's 4300-digit limit.
- Big ints passed into `--ext` modules raise instead of aborting the host.
- Fixed use-after-free / double-free classes found by review (operand pinning,
  set in-place ops, deep frees without recursion).

### Fixed

- A nested function's plain assignment to a `nonlocal` variable (`k = v`, not
  only `k += v`) is seen by the enclosing function; it used to rebind a copy.

### Performance
- Inline small-int arithmetic, direct calls, inline exception checks, fixed-layout
  class instances, inline float `/` and `**`, one-byte `list[bool]`.
- vs CPython 3.13 (bench/run.py): mandelbrot ~40x, spectral_norm ~15x, fib ~12x,
  sieve ~6x, nbody ~5-25x faster; mandelbrot runs at hand-written Rust speed.

### Real-world corpus (200 competitive-programming solutions)
- `--non-strict`: 200/200 match CPython (CI-gated, 0 wrong outputs).
- strict: 93/200 compile and match CPython unchanged (most of the rest only lack
  annotations on public functions, which strict mode requires by design).
- `--non-strict`: 51% of top-level statements compile natively, the rest fall back
  to CPython per statement.

### Known gaps
- See `docs/DEVIATIONS.md` for every known difference from CPython 3.13.
- Linux x86_64 only for prebuilt binaries.
