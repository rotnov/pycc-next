# Library compilation: compile little, compile well

Status: in progress. Implemented: PR 1 (measurement, section 3), PR 2 (`hot_api.c` deleted,
section 7 item 12), PR 3 (`pycc profile`, section 2.2; `src/profile.rs`), PR 4 (the switch,
sections 2.1-2.4; `src/select.rs`, `src/fallback/selected.rs`), PR 5 (the chunk machinery
deleted, section 7 items 1-5; `src/ext_build/module_code.rs`), PR 5a (selection by
region), PR 8's first part (observed-type specialization), PR 6b (`--select module`
deleted). Tracking issue:
[#78](https://github.com/rotnov/pycc-next/issues/78). Roadmap: [v0.12](../ROADMAP.md#v012--imports-faster-than-cpython-finished).

This is how `pycc build ... --ext --non-strict` compiles untyped library code (pydantic,
mistune, pystache). Strict mode, which compiles typed code end to end, does not change.

## 1. Problem and evidence

Today a non-strict `--ext` build compiles the whole package. Every module body, class
statement and function becomes native code, and that code calls the shim's generic
`PyObject*` helpers (`pycc_ext_obj_*`). Whatever pycc cannot compile runs as precompiled
fallback chunks. On pydantic this does not pay.

These figures were measured locally on 2026-11-06 with PBS CPython 3.15.2 on a 4-vCPU
x86_64 VM, running `bench/pydantic_workload.py`'s phases with `PYTHONHASHSEED=0`. The
harness is in [`bench/headroom/`](../../bench/headroom/) (section 10).

1. **Compiling everything runs about as many instructions as the interpreter.** Over
   import, define and schema the compiled tree runs 1175M instructions against CPython's
   1190M. The compiled code is only 8% leaner than the bytecode it replaced. Over define
   and schema alone (5 repetitions each, cachegrind) it runs more: 737.2M against 703.7M.
2. **The extra wall time comes from the instruction cache and the branch predictor.** Wall
   time is worse even where the instruction counts are close. These are medians of 80
   interleaved fresh-process rounds, ms per repetition:

   | phase | CPython | pycc | ratio |
   |---|---|---|---|
   | define | 6.82 | 8.78 | 0.78x |
   | schema | 11.68 | 15.09 | 0.77x |

   Over define and schema, the compiled tree has 6.71M I1 misses against CPython's 3.38M
   (about 2x), and 11.40M conditional-branch mispredictions against 9.20M. The extension
   is a 10.9 MB `.so`, mostly out-of-line helpers, and both it and the interpreter's own
   hot code compete for the cache.
3. **Ideal code for the hot functions beats both.** Three hot functions were written by
   hand as ideal C (`bench/headroom/hrfast.c`): `json_schema._deduplicate_schemas`,
   `json_schema._make_json_hashable` and `_config.ConfigWrapper.__getattr__`. They were
   injected into plain CPython, with every other function left as bytecode. Replaying the
   exact inputs recorded during define and schema gave these times (median ms) and
   instructions (callgrind):

   | function | calls | CPython | pycc today | ideal C |
   |---|---|---|---|---|
   | `_deduplicate_schemas` | 2,410 | 13.1 ms, 205.7M | 9.95 ms, 149.5M | 3.3 ms, 62.8M |
   | `_make_json_hashable` (top-level calls) | 2,580 | 12.0 ms, 193.9M | 8.9 ms, 137.1M | 2.9 ms, 54.1M |
   | `ConfigWrapper.__getattr__` | 16,445 | 1.75 ms, 37.0M | 2.65 ms, 44.9M | 0.34 ms, 7.4M |

   So the ideal C is 3-8x faster than both the interpreter and pycc's output. pycc's
   `__getattr__` is slower than the interpreter.

   With them injected, the phases moved as follows (same 80 rounds):

   | phase | CPython | CPython + 3 C functions | ratio |
   |---|---|---|---|
   | define | 6.82 | 6.89 | 0.99x, unchanged |
   | schema | 11.68 | 9.33 | **1.25x** |

   The define and schema phases ran 538.9M instructions against 703.7M, with 3.40M I1 misses
   against 3.38M and 7.62M mispredictions against 9.20M.
4. **The win comes from doing less work, not from cheaper dispatch.** In the C:
   - `tuple(sorted(genexpr))` fills one list, sorts it and steals its items into the
     tuple. No generator or second list is made.
   - The dict comprehension fills the dict directly.
   - `try: d[k] except KeyError:` is a miss check (`PyDict_GetItemRef` answering 0). No
     exception is made unless the handler itself raises.
   - `isinstance` tests the exact type first. A `str`, `int`, `None`, `bool` or `float`
     answers without probing `__class__`.
   - `d.items()` over an exact dict is a `PyDict_Next` loop, with no view and no item
     tuple.
   - The recursive call goes straight to the function, with no vectorcall trampoline and
     no argument binding.
5. **Code size and layout set the ceiling.** With the hot functions ideal, a fully compiled
   tree would still be slower than CPython, because its code is large and laid out badly.
   CPython plus a few good native functions beats CPython. Leaving the hot modules
   uncompiled already reached parity.
6. **The ceilings differ by phase.** Schema can drop by up to about 25% in time. Define is
   bound by the CPython runtime (`type_new`, dicts, allocation), and none of the three
   functions runs in it, so parity is its honest target.

The levers merged in v0.11 and v0.12 tuned the "compile everything" model one cost at a
time: chunk tables, dataclass texts, forward references, classes made once, cold calls.
Each was a real but small win, and together they left the define and schema phases at
0.77-0.80x. By the rule in section 6, this means the approach is wrong.

## 2. The model

A package's modules stay CPython bytecode. Only the functions a representative run shows
to be hot are compiled, and each is compiled into specialized native code.

### 2.1 What is compiled

- **Unit.** Unit: a function. Eligible functions are module-level functions, and methods of
  module-level classes, including `staticmethod`/`classmethod` functions and `property`
  accessors (qualnames without `<locals>`). Generators, coroutines and closures are not
  eligible in the first version.
- **Everything else stays bytecode.** That means module bodies, class statements,
  decorators, nested functions and every unselected function. CPython runs them exactly
  as it runs the source, so import, tracebacks, audit events and `inspect` all work as in
  CPython.
- **A selected body that does not compile stays bytecode.** It is reported (`--report-fallbacks`
  lists it with its diagnostic). There is no fallback inside a selected function: the
  function is compiled whole or not at all.

### 2.2 The profile

- **Gathering.** `pycc profile -o pycc.profile.json -- PYTHON SCRIPT [ARGS]` runs the
  workload under the target interpreter against the package's sources, with nothing
  compiled in its first two passes. It makes three:
  1. `cProfile` (deterministic). For each code object of the package it records calls
     and self time (`tottime`), and its calls to Python functions by callee (a function
     of the package, or any other), each a median over `--runs` repetitions (default 5).
     The calls a generator expression or lambda makes count as its enclosing
     function's, since compiled they are part of its body.
  2. `sys.monitoring` `PY_START`, only for functions that pass 1 would select. It
     records the exact types of the arguments in the first 1,000 calls of each, and in
     how many of those calls the function caught an exception in its own frame
     (`EXCEPTION_HANDLED`). Sections 2.3 and 2.5 use them.
  3. Verification: the candidates are built and the workload timed against them
     (section 2.3), so only measured winners are selected.
- **Storage.** One JSON file, committed with the project that builds the package. The
  file holds:
  - `{"version": 1, "python": "3.15.2", "workload": "<command line>"}`;
  - one entry per function: `module` (its import name: the dotted path below the
    directory above every package that holds it, so `outer/inner/m.py` is
    `outer.inner.m` even when `outer/inner` is the target; `select::import_name`, the
    one name `--compile`, a package build's members and the build's report use too),
    `qualname`, `firstlineno`, the SHA-256 of the
    function's source text, `calls`, `self_ns`, `callees` (`module:qualname:firstlineno`
    of a package function, or `*` for any other Python function, to its count), and
    `arg_types` (exact type name to count, per parameter), and `handled` (of those
    sampled calls, the ones that caught an exception), and for a function pass 3
    measured, `measured` (section 2.3).

  Entries are sorted by `module`, then `qualname`, so a re-profile diffs cleanly. The
  header also records `runs` and `total_ns`, the run's total self time (the median over
  the runs of everything `cProfile` saw), the denominator of section 2.3's share. Calls,
  callees and argument types are exact (`PYTHONHASHSEED=0`); self times vary by run, so two
  profiles of one workload differ only in `self_ns` and `total_ns`, and the same samples
  always serialize to the same bytes. A function's source text runs from its first
  decorator's line (its `co_firstlineno`) to its last line, as `ast` reports them.
- **Use.** Pass the profile with `pycc build PKG --ext --non-strict --profile pycc.profile.json`,
  or name it in `[tool.pycc] profile = "..."`.
- **Stale entries.** An entry whose source hash no longer matches is dropped, with a note
  naming it. A changed function is not compiled from a profile of other code. Entries
  are matched by module, qualified name and hash, so a function that only moved stays
  current. `pycc profile ... --check -- PYTHON` prints what a build would make of a
  profile: its stale entries and the selection.
- **Default with no profile.** Nothing is selected. The build says so in one note, and a
  package with nothing selected gets no extension at all, so CPython imports it unchanged.
  A single module built to `-o OUT` gets its source written as `OUT`'s `.py`, so the
  output contract (an importable `OUT`) holds. Either way an extension an earlier build left
  at `OUT` is removed, as CPython would import it ahead of the source.
  Compiling without evidence was measured as a loss (section 1, point 2).
- **Overrides.** `--select all` compiles every eligible function. It exists for tests and
  for users who measure for themselves. `[tool.pycc] compile = ["pkg.mod:qual", ...]`
  adds functions and `[tool.pycc] exclude = [...]` removes them. Both override the
  profile.

### 2.3 Selection rule

The cost of a compiled function sits at its boundary with interpreted code, not inside
its body. A prototype of PR 4 measured this on pydantic's schema phase: patching a function into
CPython's function object loses the specialized inline call CPython had for it
(`LOAD_ATTR_PROPERTY`, `CALL_PY_EXACT_ARGS`), and each call the compiled body makes to a
Python function goes through the C API. A function that mostly calls interpreted code
pays both and gains little: `GenerateJsonSchema._config` (one Python call per call)
cost 0.4 ms of schema and `ConfigWrapper.__getattr__` 0.2 ms, while
`_make_json_hashable` (0.22 Python calls per call) saved 0.55 ms.

So selection is by call-graph region, not by single function. The profile records, per
function, its calls to Python functions by callee (section 2.2), and the rule
(`src/profile/select.rs`, `select`) decides each function by the first of these steps it fails:

1. **The function itself**: eligible, its profile entry current, not measured slower by
   pass 3, sampled by pass 2 once pass 2 has run, and catching an exception in less than
   half of its sampled calls (`handled`). Exceptions as control flow run slower compiled
   than in CPython until PR 9 (section 2.7): PR 4 measured `ConfigWrapper.__getattr__`,
   which catches a `KeyError` in 989 of its first 1,000 calls, at +0.2 ms of schema.
2. **Its region**: the functions that pass step 1, connected by calls among them, whatever
   their own heat. The region is judged as a whole: its functions together have at least
   0.5% of the run's total self time, and one of them was called at least 50 times. A
   cold callee joins its hot caller's region, so diff-match-patch's `diff_bisectSplit`
   (0.03% of the time) comes with `diff_bisect` (98%).
3. **Its crossings**: each call it makes to a Python function outside the selection
   (itself and its own nested code are inside) costs about 36 ns more than the
   interpreter's own call (`CROSSING_NS`: a compiled loop calling an identity function,
   47.7 ns a call, against the interpreter's 11.7 ns for the call itself, CPython 3.15.2
   x86-64). A function whose crossings would cost its whole self time cannot win and is
   not selected. Through PR 5 this step counted the share of calls that leave, not
   their cost: it rejected `diff_bisect`, whose one call out per call is nothing beside
   its loop, and pydantic's glue alike.
4. **The size**: at most 64 functions and 4,000 source lines, taken by self time. This,
   not heat, caps how far regions grow: each selected line is compiled code in the `.so`.
5. **Pass 3's evidence**, once pass 3 has run: the set pass 3 measured as the winner is
   the selection, member for member. Steps 2 to 4 chose the candidates pass 3 measured
   and are not applied to its result again, so a build compiles exactly the set that was
   timed. The profile records the set's key (`winner`): each member with the source hash
   it was measured with, the `--exclude` list and the interpreter's version. The set is
   valid only while all of them hold, checked against the whole package's sources
   whichever module a build compiles: a build of one module compiles that module's share
   of it, so the builds of every module together compile exactly the measured set. A
   member that changed, is gone or is now excluded, a different `--exclude` list, or
   another interpreter version makes the set invalid: it selects nothing (an exclusion
   never yields to an old verdict), `--check` reports it, and the next profile measures
   from scratch.

Steps 2 to 4 repeat until no function changes. A rejection names the first step the
function failed: a function its region or its crossings reject says so in a verified
profile too, never "not measured by pass 3".

Thin glue (properties, accessors, functions that mostly call Python code) therefore
stays bytecode unless the functions it calls are selected with it. Inside a region the
calls become native direct calls in PR 7 (section 2.8). The rule uses only the
profile's numbers, never a function's name.

The rule's picks are candidates, not the selection: only measured winners are
selected. Pass 3 of `pycc profile` (`src/profile/verify.rs`) builds the candidates into
a copy of the project and times the training workload with them compiled against the
unchanged sources, interleaved in fresh processes after one discarded warm-up run of each
(`--verify-rounds`, default 7, at most 15; a check stops after 5 rounds once the two interquartile ranges are apart). The
verdict is on wall time: the compiled median must be under CPython's first quartile. A
median inside CPython's interquartile range is noise, and a function that wins only by
noise adds size and risk for no gain, so it is not kept (PR 5 measured pydantic's 5
training candidates kept in one run and rejected in another under the earlier "no
slower than CPython's third quartile").
A region pulls its cold functions in only when that pays. Measured on diff-match-patch's
diff_main (best of 3), CPython took 4.89 s, `diff_bisect` alone compiled 1.75 s (2.8x),
its whole region compiled 1.88 s (2.6x) and `--select all` 2.11 s (2.3x): a generic lift
of a cold function costs more than its bytecode. So pass 3 checks all the candidates
together and their hot functions (each at least 0.5% and 50 calls on its own) alone, and
If neither is faster, each region (the candidates connected by calls among them, most
self time first) is checked the same way alone, then the regions that won are checked
together once more; at most 8 builds, one of them reserved for that last check before any
region is measured. The selection is always exactly one set that was measured as a whole
and won, never a union of sets measured apart: the smallest winner, unless a larger one is
faster beyond the noise of both, its compiled third quartile under the smaller one's first
quartile (sets of one size: the one measured first, more self time, unless beaten the same
way). A lower median alone does not order two winners: with overlapping interquartile
ranges the pick flipped from run to run, and on diff-match-patch CI selected a 30-function
region (a build of about 9 s, near `bench/package_check.py`'s `--max-build-seconds 10`) in
one run and `diff_bisect` alone (1.4 s) in the next. Each function more costs build time,
size and risk, so a larger set must earn its place by more than noise. The quartiles
compared are each check's own compiled runs: every check interleaves its rounds with
CPython's, so drift inside a check is controlled, and normalizing by each check's CPython
median would add that median's noise to the comparison. When the regions together cannot be built within the budget, or lose,
the best single winner is kept (a region, a core, or nothing). A function the winner
leaves out of a set it was measured in is rejected ("faster without it",
`measured.without`). Pass 3 enforces this structurally:
- **Every build is checked against its request.** The functions a check's build compiled
  (its `compiled:` report) must equal the set it was asked for, compared as sets. A
  missing member is refused and the rest built again. An extra (for example from a copied
  `pyproject.toml`'s `[tool.pycc] compile`, which its builds also ignore with the internal
  `--no-project-select`) refuses the measurement: it is never timed.
- **The selection is one entry of an append-only list of measurements, whole, or nothing.**
  An entry is selectable only if it was timed, won, and no member of it was refused at any
  point of the run; no subset of an entry is ever derived or restored.
- **A carried winning set is the first entry.** A later profile carries an earlier winning
  set that is still valid (step 5) as that entry, inside every set it measures. A refusal
  of any of its members invalidates it whole.

A seeded property test drives these decisions through thousands of random runs (wins,
losses, refusals, extras, carried sets, budgets, a changed key) and checks that the
selection is always one valid winning entry and that the build compiles exactly it. The copy is the project root: the nearest directory at or above the
package's import root with a `pyproject.toml`, `setup.cfg` or `setup.py`, searched up to
the first `site-packages` or `dist-packages`, so the workload's tests and data are found
at the same paths (the workload reads the copy's import root from `PYCC_PROFILE_ROOT`).
With no marker, or for an installed package, it is the package alone, plus the paths
`--verify-include` names. An unreadable entry outside the package is left out with a
note, and one inside it is an error. A module a build keeps in Python (`--exclude`) is
excluded from the profile's candidates by the same `--exclude` (it is still profiled, so
the calls into it count as crossings), so the build compiles exactly the set pass 3
measured; `bench/package_check.py --select profile` checks that it does. Every candidate gets its
last check's outcome in the profile (`measured`: `compiled`, `with`, `cpython_ns`,
`compiled_ns` as first quartile, median and third quartile, and with
`--verify-instructions` the instruction delta under callgrind, as evidence only). A
function measured not faster is rejected ("not faster when compiled") by builds and `--check`
alike. A later `pycc profile` keeps every verdict, a win as well as a rejection, while
the function's source hash is unchanged, and measures only the candidates without one,
so a re-run never flips a recorded verdict and builds do not flap (deleting the profile measures every function
again, as a new pycc whose code got faster should); `--no-verify` skips the pass.
Each check's build compiles exactly its set (`--select none --compile MODULE:QUALNAME`)
and reports what it compiled (`--report-fallbacks`); a candidate the build leaves
bytecode is never timed: it is rejected with the build's reason ("not compiled: ...",
`measured.refused`, not carried to the next profile, whose pycc may compile it) and the
check is built again without it. The timed runs keep the original module's import root
on `sys.path` after the copy's, so a standalone module's siblings are found. A
function is selected only with evidence from every pass that ran: once pass 2 has run
the profile is `sampled` and only the functions it sampled (`sampled` on the entry)
are selectable ("not sampled by pass 2"), and once pass 3 has run it is `verified` and
only its measured winners are. A rejection frees a slot under `--select-max` but
admits no function a pass did not examine.

Pass 3 exists because of what PR 4 measured on mistune. Its 11 candidates (escape,
`HTMLRenderer.text`, `InlineState.append_token`, the emphasis parser and its helpers)
made mistune's benchmark 0.966x (91.86 ms against 88.70 ms, the IQRs apart) and 2.6%
more instructions; built one at a time and eleven times leaving one out, under
callgrind, not one was a winner: most were neutral (within 0.03M instructions a pass),
`append_token` cost 0.30M and `escape` 0.10M. An untyped lift does not beat 3.15's
specializing interpreter at its boundary: a call into a patched function, or out of
one, is a vectorcall where CPython had an inlined frame, and the body itself, still
generic object code, saves little. Only whole regions with direct calls inside them
(PR 7) and type-specialized bodies (PR 8) can pay; pydantic's recursive functions, whose
calls stay inside their region, already save 5% of schema's instructions, but that is
0.25% of the training workload, under its noise, so pass 3 does not keep them either:
after PR 4 none of the three packages selects a function. If PR 7 does
not make mistune's emphasis region a measured winner, that is the point to stop and
report.

The defaults are build options (`--select-share`, `--select-calls`, `--select-max`,
`--select-lines`, `--select-crossing-ns`, `--select-handled`). They stay until the measurements in section 8 justify other
values. The report lists every selected function with its share, and every function it
rejected with the reason.

### 2.4 Installation: the module runs as bytecode, then selected functions are patched

- **Package entry.** A package keeps `PKG/__init__.abi3.so` as its entry. Its finder
  (`src/ext/package/finder.c`) serves only the modules that hold selected functions.
  Every other module is left to CPython's path finder, so it is CPython's module, file
  and spec, with nothing pycc-specific.
- **Single module.** A single-module build (`m.abi3.so`) is the same case with one module.
- **Running the module.** A served module runs its own code object. It is marshalled at
  build time, one object per module, not per chunk, and checked against the interpreter's
  magic number and `-O`. When that check fails, the module runs `compile()` of its
  embedded source. The module's `__file__` and `__spec__.origin` name its `.py` file, as
  CPython's own would. Its loader has `get_code` and `get_source`.

  As of PR 5, the build compiles the module's source once with the target interpreter
  and embeds the code object, marshalled with the interpreter's magic number
  (`src/ext_build/module_code.rs`); the extension runs it in the module's namespace
  (`module_code.c`), or `compile()`s the embedded source when the running interpreter's
  magic number differs or it runs with `-O`. `__file__` and `__spec__.origin` name the
  `.py` file, which ships beside the extension, so `inspect` and tracebacks read it. The
  loader does not have `get_code` and `get_source` yet.
- **Patching.** After the body has run, each selected function is patched in place, with
  the frameless forwarder mechanism v0.8 built (`src/ext/module/forward.c`; the hook is `patch.c`):
  - its `vectorcall` becomes a trampoline into the native body;
  - its version is cleared, so no call site specializes past it;
  - CPython's own binding stays the fallback for any argument shape the trampoline does
    not bind.

  The function object, its `__code__`, `__defaults__`, `__wrapped__` chains and its
  identity stay CPython's. A decorator that wrapped the function keeps calling the
  patched object. A function whose `__code__` is later replaced runs as bytecode.
- **Where patching is unavailable.** The trampoline writes CPython 3.13's and 3.15's
  function layouts. On another CPython (3.14) the selected functions run as bytecode, and
  the import says so with a `RuntimeWarning`; under `PYCC_REQUIRE_PATCH` the import fails
  instead, which is how pass 3 refuses to time bytecode against bytecode
  (`PYCC_NO_PATCH` simulates such an interpreter). The same holds for any selected
  function the patch does not find where its class statement stored it (private names
  are looked up mangled, as CPython stores them): it is counted and named in the warning,
  and under `PYCC_REQUIRE_PATCH` it fails the import, so a function is never reported
  compiled while it runs as bytecode.
- **Before the patch.** A function called while the module body runs (before the patch)
  runs as bytecode. That is correct, only slower.

### 2.5 Call-site specialized code

Each operation in a selected body is lowered for the types it is expected to meet:

- **Expected types.** They come from the profile's `arg_types`, and from local inference
  (a literal, a display, a builtin's result).
- **Hot path inline.** The exact-type path is inline: `Py_IS_TYPE(o, &PyDict_Type)`, then
  the CPython call that type needs (`PyDict_GetItemRef`, `PyList_GetItemRef`,
  `PyDict_Next`).
- **Slow path out of line.** The generic `pycc_ext_obj_*` helper is the fallback when the
  guard fails. It is called out of line, from a block marked cold.
- **Error paths cold.** Error paths (building an exception, a traceback record) are cold
  too. They are emitted into `.text.unlikely`, away from the hot body.
- **Layout reads.** Inline reads of a non-stable layout (a list's items, a tuple's
  items) use the probe-checked offsets the shim already uses for 3.13 and 3.15 (#77). On
  any other layout the site takes the stable-ABI call.

This replaces the guessed specializations of `src/specialize.rs` (section 7).

**PR 8, first part: bodies specialized for observed types.** A selected function gets
the parameter types pass 2 observed. Only parameters that held one exact type in every
sampled call count, and only for `int`, `float`, `str` and `bytearray`
(`Entry::observed_types`). Those types travel with the selection:
- `Selection.types`;
- the member `--select-functions` file, as ` param:type` after the qualified name;
- for pass 3's builds, `--types-profile`, so pass 3 times the same typed bodies a build
  compiles.

The selection marks each such function's lift explicitly
(`crate::specialize::set_lifts`). `pycc_ast::specialize_observed` then gives the lift a
native copy with those parameters typed, and an exact-type guard in front that calls it
(`type(s) is str`). Any other argument, a subclass included, runs the generic lift.

**PR 8, second part: guarded inline paths in generic bodies.** A body none of whose
values has a native type still meets exact `int`s and `str`s at most of its operations.
Each of these operations now checks for them inline, as CPython's specializing
interpreter does, and calls its helper only when the check fails
(`crates/pycc_codegen/src/fast_ops.rs`):

| operation | inline path | CPython's |
|---|---|---|
| `a + b`, `a - b`, `a & b`, `a \| b`, `a ^ b`, `a += 1` | two compact `int`s (a native operand a small word): the result, one `PyLong_FromLongLong` | `BINARY_OP_ADD_INT` |
| `a < b` ... in a condition | two compact `int`s: compared by value; `==`/`!=` of two exact ASCII `str`s: answered when they are one object, or their lengths or computed hashes differ | `COMPARE_OP_INT`, `COMPARE_OP_STR` |
| `s == "lit"` | an exact compact ASCII `str`: its length, then its bytes | `COMPARE_OP_STR` |
| `s[i]` | an exact compact ASCII `str` and a compact `int` in range: the cached one-character `str`; an exact `bytes` or `bytearray`: its byte | `BINARY_SUBSCR_STR_INT` |
| `b[i] = v` | an exact `bytearray`, a compact `int` index in range and a value in `range(256)`: the byte stored | `STORE_SUBSCR` |
| `c in s` | a one-character ASCII `str` in an exact `frozenset`: one bit of the set's ASCII mask (below); any other exact `str` in an exact `set` or `frozenset`: `PySet_Contains`, the item unpacked | `CONTAINS_OP_SET` |
| `isinstance(o, dict)` (a builtin class) | `o` exactly of that class: `True`; exactly of another of the 12 builtins the shim publishes: `False` | |
| a module global | a per-site cache of the namespace dict, its version and the value | `LOAD_GLOBAL_MODULE` |

The layouts these read are checked at module exec (`src/ext/module/fast_ops.c`), which
publishes the type pointers the checks compare with only when every check passes; on any
other build, and with `PYCC_NO_FAST_OPS=1`, every check fails. The loads of an object's
type and of an `int`'s or `str`'s value are `!invariant.load`, so a check on a value a
loop does not rebind is loop-invariant.

`c in s` against a `frozenset` reads a 128-bit mask of the set's one-character ASCII
members from a two-way cache of 64 sets keyed by the set's address
(`pycc_ext_ascii_mask_fill`). A frozenset cannot change, and the cache holds a reference
to each set whose mask it keeps, so an address cannot be reused while its entry lives. A
set with more than 256 members, or with any member that is not an exact `str`, gets an
entry marked unusable, without a reference, and takes `PySet_Contains`: a member of
another type could compare equal to a one-character string. Releasing an evicted set can run
a weakref callback that refills the cache, so a miss answers from the entry it has just
computed, not from the cache. tomli's `c in
BARE_KEY_CHARS`-style loops made the lookup its largest helper; the mask took tomli's
`loads()` from 31.6M to 29.8M instructions.

A release stays one call into the module's own out-of-line copy (`object_rc.rs`):
inlining its common case (a mortal object that stays alive) took 1.5% of tomli's
instructions and 0.5% of mistune's, for 23% more `.text` and 17% more build time on
mistune.

**PR 7, first part: direct calls between generic bodies (section 2.8).** A call from a compiled body to another
lifted body went through that body's forwarder: the Python function object, vectorcall,
the forwarder's argument binding, a recursion check, then the body. For pydantic's
`_sort_recursive` these out-of-line layers, not cache misses, were the gap to CPython:
callgrind's cache and branch simulation put its I1, D1 and mispredict counts within a few
percent of the interpreter's, while it made about 67,000 calls per top-level call against
CPython's 46,000. Now each lift has a cell (`fnptr_<lift>__pycc_dcell`: the function
object, its code object, and a mask of the argument counts it binds in order), which the
shim fills when it patches the function (`pycc_fwd_publish_direct` in
`src/ext/module/forward.c`). The cell stays empty for a function with `*args`, `**kw`, a
`__class__` cell, 64 or more parameters, or with `PYCC_NO_FAST_OPS=1`.

A call site whose callee is a module global or a method named like exactly one lift, with
the keywords naming that lift's parameters in order, checks inline that the callee is the
cell's function, still runs the cell's code object, and binds this many arguments in
order. It then enters CPython's recursion check (`Py_EnterRecursiveCall`) and calls the
body with the packed arguments (`crates/pycc_codegen/src/direct_call.rs`). Any failed
check, a rebound global, a replaced `__code__`, an override in a subclass, a default
left out, takes the call as before, and so does a recursion limit reached, so the
`RecursionError` is raised where CPython raises it.

Two of the inline paths above changed with it:
- `isinstance` answers `False` too: the exact class of the object is one of the 12
  builtins the shim publishes (`NoneType` among them), so the answer is whether it is the
  class asked for (or `bool` for `int`).
- Packing an object argument is an inline reference increment, with the call kept for a
  missing value.

Local callgrind on CPython 3.15.2, `--select all`, before (PR 8) → after:
- `_sort_recursive`: 2.38M → 2.04M instructions per call (CPython 2.53M);
- tomli `loads()`: 38.0M → 31.9M;
- mistune: 61.8M → 59.8M;
- pystache: 0.47M → 0.45M.

Wall time of `_sort_recursive`, best of 9: 184.8 us against CPython's 189.3 us (PR 8:
230.9 us). mistune's `.text` grows from 1.79 MB to 1.87 MB (+4.5%), and its build from 6.0 s
to 6.4 s.

**Unpacking a call without the tuple.** tomli's parsers return `(pos, value)` and their
callers unpack it at once (`pos, key = parse_key(src, pos)`): each such call built a
tuple, unpacked it through the generic protocol and freed it, about 450 instructions
per call. A lifted body whose `return` displays all have `n` items, or whose other
returns forward a call of such a body (`return parse_basic_str(src, pos,
multiline=False)`), now gets a twin (`0gen_multi<n>_<lift>`,
`crates/pycc_mir/src/multi_return_object.rs`) when an unpacking assignment into `n`
targets calls it. In the twin, `return a, b` stores `b` through an out-pointer and
returns `a`; every other `return` is unchanged and its value is the whole result; a
forward passes the twin's own out-pointers to the forwarded body's twin. The call
site (`MirStmt::AssignMulti`, `crates/pycc_codegen/src/multi_out.rs`) clears its
out-pointers and calls the twin on the direct path (section 2.8). A stored first
out-pointer means the items are there. A null one means the result is whole (the
forwarder ran, or the body returned something else), and the site unpacks it as
before, so a list, a string, a wrong length or a non-iterable is unpacked or raises at
the call site as in CPython, after the callee's frame is gone. A twin has its body's
name in tracebacks and `sys._getframe`. A body with a `finally` (which could raise
after the items were stored), a generator statement or a closure gets no twin.
`tests/multi_returns.rs` checks these paths, with the direct calls on and off, against
CPython.

Local callgrind on CPython 3.15.2, `--select all`, per call: tomli `loads()` of a 38.6 KB
`Cargo.lock` 26.76M → 24.72M (CPython 45.9M); a loop of `i, x = g(s, i)` 963 → 475
instructions per iteration.

**Rebinding an `int` in place.** In a scanning loop (`pos += 1`, `i -= 1`), every step
made a new `int` and freed the old one: `PyLong_FromLongLong`, then the release and
`long_dealloc`. CPython pays the same, through its free list. A statement `x = x op y`
(or `x op= y`) on an object local whose slot owns its reference, where `op` is `+`,
`-`, `&`, `|` or `^`, now checks one more thing on its compact-`int` fast path: if the
slot holds the only reference to `x`'s `int` (its reference count reads 1) and the result
is compact and outside the small-`int` cache (`-5..=256`), the result is written into
that `int` (its tag and digit), and nothing is stored, made or freed
(`foreign_arith::emit_rebinding_binop`, `Fast::int_in_place`). No other reference can
see the change, since the count proves there is none. An `int`'s value is therefore no
longer read with `!invariant.load`; its type still is. `fast_ops.c` checks at module
exec that a new `int`'s count reads 1 where the generated code reads it.

The alternative was a second, unboxed representation for such locals, boxed where the
value escapes. It would touch every reader of a local slot (frames, cells, releases,
exception cleanup) for the same saving in these loops. tomli `loads()`: 31.8M → 28.8M
instructions; the others move by less than 1% (pypng +0.6%, from the plain loads).

**Tagged locals.** That alternative, narrowed, is now what a scanner's position and
current character get (`crates/pycc_codegen/src/tagged_locals.rs`). An `object` local or
parameter of a non-generator body whose every mention is a read, an assignment or a
release, that some statement rebinds as `x = x ± y` or `c = s[i]`, and that some
expression reads as a word, holds a word: a small `int` `v` as `v << 1 | 1`, an ASCII
character as `c << 3 | 2`, or a reference. The producers make words with no object made
or changed; an index into an exact `list`, `tuple`, `str`, `bytes` or `bytearray` (any
other `__getitem__` may keep or return its key, so it gets the local's object), a
comparison with a one-character literal, a comparison with an `int` literal in a
condition, `c in frozenset` and the `str` operations below read them as words, each
after the same unbound-local check an object read makes; every other read boxes the word (CPython's cached object for a small `int` or
a character) and stores the box back into the slot, so later reads of the same value see
the same object, as a CPython local would. A local no expression reads as a word is left
an object: pypng's `undo_filter_paeth` rebinds `a = result[ai]` from a `bytearray`, and
tagging `a` only moved the box to its arithmetic (+1.1% on `read()`). A read-only object
parameter is also declared non-null to LLVM.

A direct call (section 2.8) on CPython 3.15 asks `Py_EnterRecursiveCall` once per
invocation of the calling body, not once per call: there the check compares the C stack
pointer with the thread's limit and `Py_LeaveRecursiveCall` does nothing, so its answer
holds for every call the body makes from its frame (`pycc_ext_direct_enter` answers 2;
a generator body, which a resume may run at another depth, asks every time). Returning a
tagged local's word from a body to a direct caller was tried and dropped: the box moved
to the caller's next boundary, and `loads()` did not get faster.

**`str` operations with word operands.** Tagging alone left tomli's `int` boxes where
they were: about 9,500 per `loads()` of `Cargo.lock` on main and after, each made where
the position leaves a local, as an argument, a returned tuple item, or an operand of a
`str` method or slice. Counting the calls into CPython showed where the cost was instead:
2,274 `src[a:b]` per `loads()`, each boxing its bounds, building a `slice`, dispatching
through `PyObject_GetItem` and freeing all three (about 540 instructions with the
substring), and 1,615 `src.startswith(t, pos)` through a bound method call. Where the
receiver is a local holding an exact `str` and each `int` operand is a small-`int`
literal, a local holding a small `int` (tagged or a compact `int` object) or the sum or
difference of two of these (`crates/pycc_codegen/src/str_words.rs`):
- `s[a:b]` calls `pycc_ext_str_slice`, which clamps the bounds as
  `PySlice_AdjustIndices` does and returns what `unicode_subscript` returns: the empty
  string, `s` itself sliced whole, else `PyUnicode_Substring` (a cached one-character
  string for one Latin-1 character);
- the truth of `s.startswith(t)` or `s.startswith(t, a)` in an `if`, a `while` or a
  `not`, `t` a literal of one to eight ASCII characters and `s` compact ASCII, compares
  the bytes inline after `ADJUST_INDICES`' clamping.

Any other receiver or operand (a `str` subclass, a big `int`, a `bool`, a list, an object
whose `__getitem__` sees the `slice`) takes CPython's own operation from the same
receiver; the operands are names and literals, so evaluating them again there has no
visible effect. A receiver read that `operand_pin` pins (in `src[pos : pos + 1]` the
`pos + 1` evaluated after `src` may run code) is a new reference, held on the pending
stack until the operation is done. `tests/tagged_words.rs` checks both paths, with the
fast paths on and off, against CPython, and that the pinned receiver's count is
unchanged.

Local callgrind on CPython 3.15.0b4, `--select all`, `PYTHONHASHSEED=0`, per `loads()`
of the 38.6 KB `Cargo.lock`: main 22.92M → tagged locals 21.82M → recursion check per
invocation 21.45M → `str` word operands, and only word-read locals tagged, 19.91M (no
`slice` object or `startswith` call left in `loads()`) → the indexed object's class checked
before a word index, and the unbound-local check before each word read, 20.16M.

`specialize_untyped` still skips `__` names and keeps explicit annotations, for its own
reasons:
- a dunder runs through a slot;
- a class-private `__x` is name-mangled in its class body;
- an annotation is the user's type.

None of those apply to a lift: it is module-level, generated, and annotated `object`
only as a placeholder. So lifts are told apart by the selection's flag, never by their
names or annotations.

A native parameter is a converted copy of its argument, so two rules keep the copy the
function it copies (`specialize/escapes.rs`):
- **The argument stays the argument.** The copy also takes each specialized `int`,
  `float` or `str` parameter's original object. Every use reads that object unless it
  provably keeps the value inside builtin code; the rule is an allowlist, not a list of
  escapes. Allowed: arithmetic, unary operations and comparisons (not `is`) whose
  operands are all provably native (specialized parameters, literals, items of native
  sequences, or such operations); items, slices, indexes and slice bounds of a native
  `str` or `bytearray`, and the `str` an `in` searches; tests and loops over native
  sequences; `len`, `ord` and `range` of native values when no module binding, formal
  parameter or body binding shadows them; a native `str`'s methods, with native
  arguments except to `format`. Everything else reads the object: a return, a store,
  a call or container argument, `is`, `hash` (a NaN hashes by identity), and any
  operation with an operand of unknown type, which may reach that operand's reflected
  method (`__radd__`). An allowed operation whose result CPython may give as the operand
  itself (`+x`, `s[:]`, `s + ''`, `s * 1`, a `str` method with nothing to change, such as
  `s.strip()`, or `s.split()`'s one item) keeps its operands native only while its own
  result is consumed. Where that result leaves, the operands read their objects too.
  Operations whose result is always a new value (comparisons, `-x`, `len`, an item,
  `s.find(t)`) consume their operands. A `bytearray`'s native form is the object itself.
- **A parameter the body may bind again is not specialized.** The observed type is the
  argument's, not that of later values (`for item in xs: n = item`). Any assignment,
  augmented assignment, `for`/`with`/comprehension target, walrus, `del`, `except ... as`
  or `case` capture, import, `global`/`nonlocal`, or nested scope binding the name counts.
  One scan answers this and the builtin-shadowing question above, and the per-function
  fallback uses the same per-scope part of it (`pycc_ast::namespace::bound_anywhere` and
  `bound_names`);
  `--report-fallbacks` says `not specialized: ... may be bound again in the body`.

`tests/select_observed_types.rs` checks both paths against CPython: the observed `str`s;
a list, a tuple, an `int` or `None`; `str` subclasses that change indexing or `len`; a
NaN, a large `int` and a `str` that leave unchanged; `hash` of a NaN, a reflected
`__radd__`, and a `len` bound by a parameter, a `case` capture or an `except ... as`; `+x`,
`s.strip()`, `s[:]`, `s + ''` and `s.split()` against CPython's identities; and a rebound
parameter.

The guessed specializations of `specialize_untyped` (the `--select module` path) follow
the same rules, through the same `keep_objects` (object parameters come after a method's
attribute snapshots), with one difference: their parameters may be rebound
(`text1 = text1[x:]`). Such a parameter has no original object left to read, so it stays
native only if every binding is `p = e` or `p op= e` in the function's own scope with `e`
provably of its type, and no use of it, or of a value derived from it by an operation that
may return its operand, may let the value out (`escapes::escaping_rebound`); otherwise it
is not specialized. `tests/untyped_identity.rs` checks a large `int` and a `str` returned
unchanged, through `s[0:]` or `+k`, stored in a list, rebound with the value leaving or
consumed, and a method's object parameter. diff-match-patch `diff_main` (medians of 5, measured locally on CPython 3.15):
- **Module mode:** 6.47x and 6.55x after (compiled 1.100 s and 1.071 s), 6.77x before
  (1.054 s). `diff_commonOverlap` and `patch_apply` are no longer specialized: they rebind
  `text1`/`text2`/`text` and let a slice of them leave (`pattern = text1[-length:]`,
  which CPython may answer with `text1` itself).
- **Profile mode:** unchanged, 6.80x after against 6.78x before (`diff_bisect`
  specialized, compiled 1.058 s against 1.040 s).

Numbers that a function computes do not need the original object back: CPython 3.15's
`int` and `float` arithmetic never returns an operand itself, so arithmetic on numbers
consumes its operands, while `+x` and `str` concatenation, repetition and `%` pass the
escape of their result on.

Measured locally on CPython 3.15:
- **diff-match-patch `diff_main` in profile mode: 6.88x** (CPython 6.162 s, compiled
  0.896 s, medians of 5). Pass 3 keeps `diff_bisect` alone, specialized for
  `text1: str, text2: str` (2,763 ms against 2,814 ms for its 30-function region). Before:
  2.75x. **The checkpoint's diff_main bar (>= 6x) is met.** With the two rules below
  (the allowlist of native uses, no rebound parameters) it measures 6.73x (CPython
  7.116 s, compiled 1.058 s, medians of 5). The runs during these fixes ranged from 6.33x
  to 6.98x; the machine's own speed varied between them (CPython 5.43 to 7.12 s).
  `diff_bisect` keeps its specialization, since its loop only indexes `text1` and
  `text2` and takes their `len`. Pass 3 keeps it alone or the 30-function region, whose
  timings are within noise of each other; three functions of the region rebind an
  observed parameter and are not specialized.
- **pydantic `model_json_schema()`: 0.99x** (14.41 ms against 14.32 ms, 31 rounds). Pass
  3 keeps none of 55 candidates. **The checkpoint's schema bar (>= 1.05x) is missed.**

Observed types do not help polymorphic library code. In pydantic's training profile,
the hot functions' parameters each hold many exact types:
- `_make_json_hashable(value)`: `str` 536, `dict` 308, `list` 88, `int` 42, `NoneType`
  12, `float` 8, `bool` 6 (of 1,000 sampled calls);
- `_DefinitionsRemapping.remap_json_schema(schema)`: `str` 512, `dict` 327, `list` 85,
  `int` 49, `NoneType` 13, `bool` 10, `float` 4;
- `GenerateJsonSchema._sort_recursive(value, parent_key)`: `value` `str` 529, `dict`
  318, `list` 80, `int` 47, `NoneType` 13, `bool` 10, `float` 3; `parent_key` always
  `str`.

The only parameter with one type among the top functions is `parent_key`, plus
`ConfigWrapper.__getattr__`'s `name`, which is not specialized because that body has no
loop. Whole-body specialization therefore has nothing to specialize. pydantic needs
this section's per-operation specialization: an inline exact-type path at each
`isinstance` and dict/list operation, with the generic helper cold.

Generic bodies are slower than the interpreter. A generic lift costs 2-7% on whole
pydantic phases (section 1's whole-module numbers and PR 4's candidates). Per top-level
call it can cost much more. The table below replays the top-level calls each function
received during define and schema (5 repetitions), on fresh deep copies, giving ns per
call (medians of 5 processes of 15 passes each):

| function | calls | CPython | generic lift | typed (observed) | ideal C |
|---|---|---|---|---|---|
| `_make_json_hashable` | 2,580 | 5,961 | 5,499 (1.08x) | n/a: no single-type parameter | 1,601 (3.7x) |
| `_DefinitionsRemapping.remap_json_schema` | 120 | 25,834 | 29,054 (0.89x) | n/a: no single-type parameter | not written |
| `GenerateJsonSchema._sort_recursive` | 170 | 6,661 | 10,306 (0.65x) | 12,864 (0.52x; `parent_key: str`) | not written |

The ideal C is `bench/headroom/hrfast.c` (section 1). Typing the one `str` parameter of
`_sort_recursive` makes it slower: its recursive calls still go through the object
protocol, and the guard adds a test per call. Pass 3 rejects such a set, since it loses.
The fix is either of:
- compile a body only when it is typed where it matters;
- prove per function that the generic lift pays, as pass 3 already does per set.

### 2.6 Idiom lowering

Patterns that make intermediate objects are compiled to code that builds the result
directly. The semantics stay CPython's:

| idiom | lowering | guard |
|---|---|---|
| `tuple(sorted(genexpr))`, `list(sorted(...))`, `sorted(listcomp)` | one list filled by the loop, `PyList_Sort`, its items moved into the tuple | `tuple`/`sorted`/`list` are the builtins (name not bound in the module, builtins dict unchanged) |
| dict, list and set comprehensions | the container filled in the loop, no per-item temporaries beyond the item | none (PEP 709 already inlines them in CPython; no frame changes) |
| `for k, v in d.items()` / `d.values()` / `d.keys()` over an exact `dict` | `PyDict_Next` with a size check (`RuntimeError: dictionary changed size during iteration`, as CPython) | `Py_IS_TYPE(d, &PyDict_Type)`; otherwise the method call |
| `for x in l` over an exact `list` / `tuple` | indexed loop, re-reading the size each step (a shrinking list stops early, as the iterator does) | exact type |
| `list(d.values())`, `len(x)` of exact containers, `x in d` | `PyDict_Values`, the size field, `PyDict_Contains` | exact type |
| a generator expression consumed at once by `tuple`, `list`, `sorted`, `any`, `all`, `sum` | the consuming loop inlined | builtins as above; an exception raised inside adds the `<genexpr>` frame record, so the traceback is CPython's |

Other idioms join this table only with a measured win on a profiled function.

**Implemented so far** (`tests/idiom_lowerings.rs`; `PYCC_NO_FAST_OPS=1` turns each off):

- `tuple(a)`, `list(a)` and `sorted(a)` run `PySequence_Tuple`, `PySequence_List` and
  `PyList_Sort` directly when the builtin read on their first call is CPython's own, and
  call the builtin otherwise. Each still makes the builtin's copy. Reusing a list only the
  call holds (sorting it in place) was built and withdrawn: while `sorted` sorts its copy
  the argument stays whole, which code run by a comparison can see (`gc.get_referrers`),
  and which a failed sort leaves in its own order for its elements' finalizers. Restricted
  to lists whose sort runs no Python code, it saved 2M of `_make_json_hashable`'s 118M
  instructions, too little for the extra case.
- A comprehension that unpacks its item (`for k, v in d.items()`) over a dict's own
  `items()` releases each item before it advances, kept or filtered out, so the iterator
  hands the same pair tuple out again, as it does in CPython, where the pair is gone once
  unpacked. Only that iterator: advancing it runs no Python code of its own. An item bound
  whole (`for pair in d.items()`), and every other iterable's item, lives until the next
  one is bound. The desugaring that rewrites the tuple target marks it
  (`pycc_ast::is_unpack_target`) and the comprehension carries the mark (`unpacks`); a
  comprehension assigned by a statement of its own (`xs = [... for k, v in ...]`) does
  not carry it yet and keeps each item.
  Whatever the iterable, the exhausted iterator is released before the last item, and an
  exception out of the element or the condition releases the iterator, the result and
  then the item, as CPython's unwinding does (`tests/fixtures/comprehension_unwinding.py`).

The `PyDict_Next` loop, the items moved into the tuple, the indexed list loop and the
inlined generator consumers are not implemented.

### 2.7 Exceptions as control flow

A subscript directly in a `try` body, whose first handler catches a plain `KeyError` and
neither binds nor re-raises it, takes a missing key of an exact `dict` straight into that
handler: `PyDict_GetItemRef` answers 0, and no `KeyError`, bridged exception or traceback
record is made (`crates/pycc_codegen/src/exception/lazy_miss.rs`,
`pycc_ext_obj_getitem_lazy`). The handler keeps the key until it is left, as the
`KeyError` would have, so a key that is a temporary or that the handler drops is
finalized where CPython finalizes it. An exact dict raises nothing else on a miss (it has no
`__missing__`). Every other mapping, and every other failure of the lookup (an unhashable
key, a `__hash__` or `__eq__` that raises), raises and is dispatched as before.

A handler reads the exception itself in two ways, and neither is lowered:

- it binds it (`as e`);
- it re-raises it with a bare `raise` of its own (one inside a nested handler re-raises
  that handler's exception, and does not count).

Only the first handler is considered. An earlier handler of another class may catch a
`KeyError` too (a base class, a class another module defines), which only the run-time
dispatch knows.

Everything else sees the exception through CPython: `sys.exception()` in code the handler
calls, the `__context__` of what it raises. A compiled `except` clause publishes the
exception it handles, as a CPython handler does, but not at its entry: the first time
something could see it (`crates/pycc_codegen/src/exception/handled.rs`). Once a handler's
body is emitted, a call of `pycc_ext_handler_publish` is put before every call in it that
can run other code or raise. A retain, the pending-exception test, a bare `raise` and the
handler's own way out get none; a release publishes only when it frees its object, whose
finalizer could look (`pycc_ext_handler_release`). So the path without an exception pays
nothing, and a handler that only returns a default never makes a CPython exception or a
traceback entry.

A handler a lazy miss entered has no exception until it publishes: the `KeyError` the
lookup would have raised is made then, from the key the handler holds and the position of
the subscript that missed, with the context and the traceback entry it would have had. A
subscript of this lowering inside a handler's body (the second lookup of
`ConfigWrapper.__getattr__`) publishes the enclosing handler's exception first unless the
lookup provably runs no code and raises nothing: an exact `str` key in an exact dict whose
keys are all `str`s, which no key's `__eq__` is called for
(`pycc_ext_obj_getitem_lazy_in`). `PyDictKeysObject.dk_kind`, which says so, is not part of
the stable ABI: it is read at the offset CPython 3.13 to 3.15 share after a probe has
checked it against a dict of `str` keys and one with an `int` key; on another version, or a
failed probe, every such lookup publishes first.

An exception leaving a handler's body that compiled code raised itself (no CPython object
exists for it yet) is made one in the handler's cleanup, while the handler's exception is
still the handled one, so it gets its `__context__` where CPython's `raise` sets it
(`pycc_ext_handler_unwind`). `raise X from None` marks the context suppressed and
`raise X from Y` sets the cause on that object (`pycc_ext_raised_links`).

`ConfigWrapper.__getattr__` (a dict miss, then a second dict miss, then `raise ... from None`)
is the case matched: no exception exists on its hit and first-miss paths, and on the
second miss the `AttributeError` carries both `KeyError`s as its context chain, suppressed,
as in CPython.

Not implemented: `o.name` with `except AttributeError` (`PyObject_GetOptionalAttr`
answering 0), and `l[i]` with `except IndexError` on an exact `list` or `tuple`.

### 2.8 Direct calls to module globals

A call from a selected function to a module global that names another selected function
goes straight to the callee's native body, with native argument passing and no vectorcall
trampoline. Calls to itself are included. The global must not be rebound anywhere in its
module's source: there is no assignment to it apart from its `def`, no `global`
declaration of it, and no `del`.

The call is guarded at run time by the module dict's version. It reuses the dict watcher
of `src/ext/module/global_sync.c`, one compare per call. When the dict changed since the
last check (for example after `mock.patch`), the global is read again. If it no longer
names the function, the call is an ordinary vectorcall. Across modules of one package, a
`from .mod import f` binding is checked the same way, against the binding module's dict.

Between generic bodies, where every argument is already an object, the call is made
without the module dict's version: the callee is read as before and compared with the
function the callee's direct-call cell names (section 2.5, PR 7's first part). The same
check serves methods, so `self.m(...)` reaches `m`'s body too unless a subclass or an
instance overrides it. The replay targets of PR 7 in section 8 are not measured yet.

### 2.9 Size and layout

- **Hot code.** Selected bodies are placed together in profile order.
- **Helpers.** The shim's generic helpers are cold and linked only when referenced
  (`--gc-sections`, as now).
- **Size check.** The size of the `.so` is part of every report.
- **Target.** pydantic's extension drops from 10.9 MB to about 1 MB or less, from
  measurements, not a fixed rule.

**Measured: generic bodies lose to the interpreter on I1 misses, and layout cannot win it
back.** On main 40b65df, each workload built with `--select all` (every function a
generic body), per call, under callgrind's default I1 (32 KB, 8-way, 64-byte lines):

| workload | instructions, pycc / CPython | I1 misses, pycc / CPython | `.text` |
|---|---|---|---|
| pystache benchmark example | 0.452M / 0.369M | 16.0K / 5.6K | 0.67 MB |
| `mistune.html` of the benchmark cases | 59.8M / 54.3M | 1.27M / 0.59M | 1.89 MB |
| pydantic define (5 reps) | 178.9M / 157.9M | 4.07M / 2.78M | 4.14 MB |
| pydantic schema (5 reps) | 524.0M / 547.1M | 8.72M / 5.04M | 4.14 MB |

The extra misses are capacity misses, not conflicts. Raising the simulated I1 removes
them: pystache's misses per iteration are 6.5K (CPython) and 16.7K (compiled) at 32 KB,
2.5K and 7.5K at 64 KB, 0 and 374 at 256 KB. The compiled code a pystache iteration
executes covers about 110 KB of cache lines: about 56 KB of generic bodies (84% of each
line's bytes executed), 25 KB of shim helpers (56%), and 20 KB of forwarders and wrappers.
One generic body is several KB of machine code (`parse` is 28 KB). The interpreter's own
loop stays resident; compiled bodies add their code on top of it. For pydantic's schema,
the `.so` takes 3.13M of the 3.68M extra misses: bodies 1.10M, shim helpers 1.01M, the
runtime 0.32M, wrappers 0.27M, the rest 0.43M.

In LLVM instructions, a generic body of pydantic's `json_schema` is 17.6% inline fast
paths, 15% inline reference counts, 12.5% frame entry and exit, 10.3% pending-exception
tests after object operations, and 9.5% failure and traceback blocks. The tests after
object operations are dead: each operation routes its own failure
(`crates/pycc_codegen/src/foreign_fail.rs`). Removing them (#102) cuts
`.text` by 4% to 7.5% and I1 misses by 2% to 4%. Nothing else measured gave more:

| tried | I1 misses | instructions | verdict |
|---|---|---|---|
| frame entry and exit as calls, not inline | -0.5% (define) to -8% (pystache) | +0.3% to +1.8% | not adopted: more instructions, about 5% smaller `.text` |
| no inline fast paths | higher on every workload | +8.6% mistune | not adopted |
| shim helpers built with `-Os` | no change | higher | not adopted |
| LLVM `hotcoldsplit` | -3.6% schema, about 0 elsewhere | about 0 | not adopted |

Shrinking or reordering compiled code moves I1 misses by 8% at most, while the gap to
CPython is 1.5x to 2.9x. A cycle estimate from instructions, I1 misses and
mispredictions also predicts less of the measured wall penalty than there is (schema:
0.97x estimated, about 0.8x measured), so wall time stays the selection measure. Two
decisions follow:

- **Default.** A build compiles pass 3's measured winners (profile mode) and nothing
  else. On pystache (0 of 64 candidates kept), mistune (0 of 63) and pydantic, that set is
  empty today and the build equals CPython. `--select all` stays for tests and the
  instruction gate.
- **Gate.** Pass 3 keeps a set only when its median wall time is below CPython's first
  quartile, on the training workload. On a 4-vCPU VM, identical builds measured 0.87x to
  0.98x of each other over 11 rounds, so a weaker rule would select noise.

Schema >= 1.2x therefore needs generic bodies that execute fewer instructions, not a
different layout: exceptions as control flow (section 2.7) and idiom lowering (section
2.6) remove whole operations; a smaller layout does not.

### 2.10 What does not change

- Strict mode and every strict `--ext` mechanism stay as they are: compiled module bodies,
  published classes and carriers, `ext_globals`, signatures, keywords, `richcompare`,
  `getset`, `ndarray`, buffers, and compiled frames.
- Non-strict native executables (`src/embed/`, without `--ext`) keep their fallback loop.
  This plan only covers `--ext`.

## 3. Measurement rule

Every pull request under this plan reports, before and after, on the same VM:

- **Wall time.** Interleaved fresh-process rounds: at least 61 for import and 21 for the
  repeated phases, nothing else running. Report median and IQR in ms, and the ratio of
  CPython time to pycc time. Phases: import, define, schema, round-trips and the whole
  process for pydantic; import and the package benchmark for mistune and pystache.
- **Instructions.** `valgrind --tool=callgrind`, per phase, with `PYTHONHASHSEED=0`.
- **I1 misses and branch mispredictions.** The same runs with `--cache-sim=yes
  --branch-sim=yes`.
- **Size.** The extension's `.so` size and its `.text` size.

Instructions alone never decide a change. Section 1 shows the instruction count close
while wall time is 0.77x. A change that cuts instructions but adds I1 misses or wall time
is not a win.

Published numbers still come from `bench.yml`, cited by run id (`bench` skill).

## 4. Targets

All targets are ratios of CPython time to pycc time, by median, on both CI runners:

| workload | target | today (local, section 1 / ROADMAP) |
|---|---|---|
| pydantic `model_json_schema()` | **>= 1.2x** | 0.77x |
| pydantic model definition | **>= 1.0x** | 0.78x |
| pydantic whole process | **>= 1.0x** | 0.88x |
| pydantic import | **stays >= 1.0x** | 1.05x |
| `import mistune`, `mistune.html`, full parser | no regression from current main | 1.00x; 0.89x / 0.86x (CI) |
| `import pystache`, pystache's benchmark | no regression from current main | 0.94-0.96x; 0.65x (CI) |

The suites must pass as before: pydantic's, mistune's and pystache's package checks fail
no test that passes against their sources.

To keep from overfitting, the profile behind the CI numbers comes from a training input
(`bench/pydantic_workload.py --train`: other models than the timed ones), never from the
timed run itself.

## 5. Behavior

The model removes deviations rather than adding them. The pull request that ships each
deletion removes its row from `docs/DEVIATIONS.md`:

- set-display order in fallback code;
- the import's audit events;
- the dataclass texts and the `TypedDict`/`NamedTuple` forward references;
- the whole-package build's missing `__file__` and its `get_source`;
- non-strict published classes.

A selected function keeps the deviations every compiled function has, for example no
trace or monitoring events. Section 7 deletes `--exact-cpython`, whose only job was to
turn off fallback-chunk fast paths.

## 6. Process rule

If a target is not met after the direct fixes, the approach is wrong. Stop iterating on
it, measure where the time goes, and rethink the concept before the next pull request.
Never ship a stream of small wins that leave the target unmet (`AGENTS.md`).

Under this plan:

- Each pull request in section 8 has an acceptance number. A pull request that misses it
  is not followed by more tuning of the same mechanism. The next step is a measurement of
  where the time goes, recorded here, and a revision of this design.
- The v0.11 and v0.12 import levers are the example this rule exists to prevent: more than
  ten pull requests each moved the import a little, and the define and schema phases did
  not move.

## 7. Cleanup inventory

What exists only to make "compile everything" survive goes. What strict compilation needs
stays. LOC are line counts at a92ee70. "Partial" means only the non-strict part of the
file goes.

| # | item | files (LOC) | why it existed | under the new model | verdict |
|---|---|---|---|---|---|
| 1 | Fallback chunk tables and their import hook | `src/ext_build/precompiled.rs` (634), `src/ext_build/precompiled_hook.c` (853), `src/ext_build/precompile.py` (312); tests `precompiled_fallback.rs` (204), `build_time_code_rewrites.rs` (138) | fallback statements ran with `compile()` on every import (v0.8); the table marshals them at build time | no fallback chunks: a served module runs one marshalled code object | **replace** with one marshalled code object per served module (about 150 LOC, keeping `pycc_ext_marshal_loads`); delete the chunk table, `CodeRewrite` (v0.11 lever 10) and the frozenset `ORDERS_FIX` (v0.12 lever 1) |
| 2 | Dataclass texts compiled at build time (v0.12 lever 11) | `precompile.py`'s `dcs`, the `exec` hook in `precompiled_hook.c` (part of 287 added by #66); `tests/dataclass_texts.rs` (165); a DEVIATIONS row | compiling `dataclasses`' generated texts was a third of the compiled tree's import | the module runs as CPython runs it; CPython compiles those texts too, so parity holds without it. Measurement that decides: pydantic import after the switch, with and without it | **delete** after the switch (PR 5): it is a documented deviation, and with bytecode modules the import is at parity without it |
| 3 | `TypedDict`/`NamedTuple` forward references compiled at build time (v0.12 lever 12) | `precompile.py`'s `fwds`, the `compile` hook (part of 387 changed by #71); `tests/forward_codes.rs` (174); a DEVIATIONS row | a sixth of the import left after item 2 | as item 2 | **delete** after the switch (PR 5) |
| 4 | `def` chunks made directly (v0.11 lever 14) and signature wrappers without factories (v0.12 #62) | `precompile.py`'s `defs`, the `__pycc_run__` paths of `precompiled_hook.c`; `tests/direct_defs.rs` (196) | a `def` chunk ran in a frame of its own | no chunks; CPython makes every function | **delete** with item 1 |
| 5 | `--exact-cpython` | `src/mode.rs`, `src/cli.rs`, `src/main.rs` (about 40) | turned off items 2 and 3 | nothing left to turn off | **delete** with items 2 and 3 |
| 6 | Mirrored imports (v0.11 lever 12) | `src/fallback/render.rs`'s `mirror_text` and `mirror_spans` (part of 585), the mirror half of `src/ext/module/import_io.c` (part of 795); `tests/mirrored_imports.rs` (198) | a compiled import whose names fallback code read ran twice | module bodies are bytecode | **delete**; `import_io.c`'s import helper stays (strict `import numpy`) |
| 7 | Non-strict module-body compilation and the `--ext` fallback loop | the `--ext` branches of `src/fallback/driver.rs` (321) and `cascade.rs` (379); `attach.rs` (256), `package_bases.rs` (442 + 43), `exceptions.rs` (51); the non-strict branches of `src/build_pipeline/ext_plan.rs` | made a module pycc could only partly compile run as one extension | only selected function bodies compile; a body that fails is not selected | **delete** the `--ext`-only parts. `lift.rs` (body extraction, the `__class__` cell) and `names.rs` (dict globals) **stay**: they build the selected bodies' compile unit. The embedded native path keeps the rest |
| 8 | Global slots and their sync (v0.8), the watch from the end of the body (v0.12 lever 10) | `src/ext/module/global_sync.c` (402), codegen's per-global cache | compiled module bodies kept globals in native slots that the dict mirrored | globals live in the module dict, which CPython's bytecode owns | **replace**: keep the dict-version watcher as the guard for section 2.8; delete the slot sync and its conversions |
| 9 | Published non-strict classes: templates evaluated from source (`Generic[T]`), non-strict carriers, sibling classes across modules | the non-strict paths of `src/ext/module/class_maker.c` (795), `src/package_build/sibling_classes.rs` (1004), the non-strict half of `src/ext_build/publication.rs` (689); DEVIATIONS rows 43-44's non-strict cases | compiled classes of untyped modules | classes are CPython's | **delete** the non-strict paths; the strict publication, carriers and `class_maker.c`'s template path **stay** |
| 10 | Classes made once at their statement (v0.12 lever 8) | `src/ext/module/class_once.c` (206), `src/ext_build/deferred_classes.rs` (121), part of `module_init.c`; `tests/v012_class_once.rs` (155) | pystache's import (-0.83M) | non-strict classes are gone (item 9); only strict `--ext` classes would use it. Decided by: import of the strict `--ext` corpus with and without it | **delete** unless the strict corpus import is at least 2% faster with it |
| 11 | `class_body_attrs.rs` (247) | `__firstlineno__`, `__doc__`, `__static_attributes__` of a published class | exactness of strict published classes | correctness, not speed | **keep** |
| 12 | Cold calls through CPython's hot text (v0.12) | `src/ext/module/hot_api.c` (170), `src/ext_build_tests/hot_api.rs` (41) | fewer cold windows of a BOLT-built interpreter at import | measured -0.41M against 0.3M noise: no measured win in either model | **deleted** (PR 2); the inline reference counts it had taken from `bridge.c` are back there |
| 13 | Shared attribute inline caches in `.bss`, dense fill (v0.8, v0.12 lever 7) | `src/ext/module/attr_cache.c` (890) | attribute reads and method calls on CPython objects; dense fill cut import page faults | a few hundred compiled sites at most; the per-site record (`src/ext/module/call_site.c`, 137, #75) needs no shared table. Decided by: schema wall and I1 misses, and minor faults, with per-site records | **replace** with per-site records (PR 8) |
| 14 | Keyword-name tuple caches per call site | `src/ext_build/keywords.rs` (370), the `kwnames` caches in `forward.c` | keyword calls without building the tuple each time | cheap and per site | **keep** |
| 15 | Layout probes | `frames.c` (863), `forward.c`'s function and code offsets, the tuple/list item offsets (#77) | non-stable-ABI fields read on 3.13 and 3.15 | the forwarder patch (section 2.4) and the inline paths (section 2.5) depend on them | **keep**; extend when the oracle moves |
| 16 | Frameless forwarders | `src/ext/module/forward.c` (983), forwarder owner paths (v0.12 lever 3) | methods of classes CPython created called compiled bodies | the installation mechanism of section 2.4 | **keep**, retargeted from lifts to selected functions |
| 17 | Compiled frames | `src/ext/module/frames.c` (863), `src/traceback_blocks.rs` (284) | exact tracebacks through compiled code | selected functions must show as CPython frames | **keep** |
| 18 | `str` literal objects (v0.11 lever 5, v0.12 lever 2) | `crates/pycc_codegen/src/py_literal.rs` (350), `crates/pycc_rt/src/str_core.rs` (537) | each literal made once and shared | selected bodies still use literals; the per-module dedup table was an import cost | **keep**; re-measure the table's import cost after the switch and drop it if it no longer pays |
| 19 | Guessed specializations of untyped functions | `src/specialize.rs` (395), `crates/pycc_ast/src/desugar/specialize.rs` (550 + 258 tests) | native code for unannotated functions by guessed `int`/`str`/... types, with a retry loop when the guess fails | the profile records the exact types | **replace** with profile-observed guards (section 2.5, PR 8); delete once schema is at least as fast without it |
| 20 | Signature wrappers of non-strict public `def`s | the non-strict uses of `src/ext_build/signatures.rs` (395) and `crates/pycc_ast/src/desugar/ext_signatures.rs` (943) | a compiled public function showed its full signature to Python | a selected function is CPython's own function object | **delete** the non-strict uses; strict **keeps** them |
| 21 | Cross-module native calls of a package built whole | `src/package_build/interface.rs` (804), `siblings.rs` (573), `graph.rs` (196) | a member calling another member's compiled functions natively | replaced by section 2.8's guarded direct calls across modules | **replace** for non-strict (PR 7). Whether strict whole-package builds read interfaces is checked in that PR: keep for strict if they do |
| 22 | Package finder | `src/ext/package/finder.c` (481) | served every compiled module of the package | serves only modules with selected functions, with CPython's spec and `__file__` | **replace** (PR 4) |
| 23 | Generic object helpers | `src/ext/module/object_*.c`, `src/ext/pycc_ext_object_ops.c` (5,081 together) | every object operation of non-strict code | the slow path behind each guard | **keep**, called from cold blocks (PR 8); delete any helper no corpus build references after PR 8 (checked with `nm` over the corpus builds) |
| 24 | Non-strict whole-module typing reached only through module bodies and classes | parts of `crates/pycc_hir` and `crates/pycc_types/src/non_strict.rs` (928) | typing compiled module bodies and native classes of untyped code | only function bodies are typed | **delete** what a coverage run of the non-strict tests (`cargo llvm-cov`) shows unreached after PR 6; it cannot be sized before the switch |

Not in the inventory, because strict compilation needs them or this plan does not touch
them: `ext_globals.rs`, carriers and pickling, `richcompare.rs`, `getset.rs`, buffers,
`ndarray`, `module_hooks.rs`, `link.rs` (#73's symbolizer removal stays), the shim split,
and `src/embed/`.

## 8. Pull requests, in order

Each pull request is small and can be merged on its own. Each reports the measurements in
section 3 and meets its acceptance number, or triggers section 6. Deletions go first
where they are safe. A deletion that would regress a tracked number waits for the pull
request that makes it unnecessary.

| PR | change | acceptance |
|---|---|---|
| 1 | Measurement: move `docs/design/headroom/` to `bench/headroom/`; `bench/pydantic_check.py --cachegrind` reports instructions, I1 misses and mispredictions per phase; `bench.yml` publishes them | the section 1 numbers reproduce within 5%; no compiler change |
| 2 | Delete `hot_api.c` (item 12) | pydantic and pystache import wall within the IQR of main (81 rounds); import instructions within 0.3M |
| 3 | `pycc profile` (section 2.2): the two passes, the JSON file, staleness | profile of `bench/pydantic_workload.py --train` lists the three section 1 functions in its top 10 by self time; byte-identical output for the same runs; tests for stale and missing entries |
| 4 | The switch: a non-strict `--ext` build runs module bytecode and patches the selected functions (sections 2.1, 2.4); one marshalled code object per served module (item 1's replacement); `--profile`, `--select all\|none`, `[tool.pycc] compile/exclude`; default none; the finder serves only modules with selected functions (item 22); `bench.yml` builds with the training profile. Today's whole-module compilation stays reachable only as a transitional `--select module`, which the existing non-strict tests use until PR 6 | no phase of pydantic, pystache or mistune slower than CPython (within the IQR), with the training profiles; all three suites pass, plus pydantic's `__file__` test that fails today. Only measured winners are selected. (The first plan asked schema >= 1.05x here; section 2.3 gives the measurement that moved it to PR 7) |
| 5 | Delete the chunk machinery (items 1-5): chunk tables, dataclass texts, forward references, direct defs, `--exact-cpython`, their tests and DEVIATIONS rows. The transitional `--select module` runs its fallback statements with `exec(compile(...))`, as the native path does | the PR 4 numbers hold within the IQR; pydantic `.so` at least 1.3 MB smaller (the chunk tables were 1.32 MB of `.rodata`); build time reported |
| 5a | Selection by region (section 2.3): a region judged on its combined self time, crossings by cost, a size cap in lines; pass 3 checks a region against its hot functions alone; a rejection names the first step that failed; `package_check --select profile` trains on the test suite plus the timed workloads | diff-match-patch's diff_main with the profile about 2.8x (`diff_bisect` selected; hand-selected it measured 2.8x); no phase of pydantic, mistune or pystache slower than CPython within the IQR |
| 6 | Delete non-strict module-body compilation (items 6, 7, 8 slot sync, 9, 20), which since 6b no non-strict `--ext` build reaches. The `--ext` oracle tests whose fixture runs its program at import run it before the patch, as bytecode (section 2.4), so since 6b they check the build and the embedded code but not the compiled bodies; each is converted to call the program after the import, as `tests/attr_cache.rs` does since 6b | as PR 5; LOC removed reported |
| 8 | Call-site specialization (section 2.5): exact-type guards inline, helpers cold, per-site attribute records (item 13), profile-observed types (item 19, then delete `specialize.rs`), `isinstance` exact path. **The concept checkpoint** (moved ahead of PR 7) | profile mode on diff-match-patch's diff_main >= 6x (`--select module`: 7.1x); pydantic schema >= 1.05x; schema I1 misses <= 1.1x CPython's; `ConfigWrapper.__getattr__` replay <= 1.75 ms. If either of the first two is missed, stop and report |
| 6b | Delete `--select module`; CI's `--instructions` gate and `package_check` move to profile mode | as PR 8; the non-strict test suite converted to `--select all` passes. **Done.** The `--instructions` gate builds with `--select all`, not the profile: pass 3 picks winners by wall time on the runner, so a profile build's set, and its counts, may change from run to run, which a gate on counts cannot allow. The `--ext` branches of the fallback loop (item 7) went with the flag; the runtime and codegen parts of items 6-9 and 20, now reached by no non-strict build, are PR 6. No build makes item 19's guessed specializations any more; their tests check that the functions compile and match CPython, and the code is deleted with item 19. With the whole-module bodies gone, the `--select all` counts measure generic bodies, which execute more instructions than CPython on the sources for tomli (`loads()`), pypng (`read()`) and pystache: every indexing, addition and comparison is a C API call where CPython's specializing interpreter has an inline path. Pass 3 keeps such sets out of a profile build; making generic bodies beat the interpreter is section 2.5's work |
| 7 | Direct calls (section 2.8) inside the selected region (section 2.3), replacing non-strict sibling interfaces (item 21), only if a measurement shows they pay | `_make_json_hashable` replay <= 7.0 ms (8.9 today), `_deduplicate_schemas` <= 8.0 ms (9.95) |
| 9 | Exceptions as control flow (section 2.7) | `ConfigWrapper.__getattr__` replay <= 0.8 ms (C: 0.34). **Partly done, target missed**: the dict-miss lowering alone; replay 0.9-1.3 ms and 14.2M instructions (before: 3.8 ms, 46.3M; CPython 2.1 ms, 37.5M; C: 8.5M) |
| 10 | Idiom lowering (section 2.6) | `_make_json_hashable` <= 4.5 ms (C: 2.9), `_deduplicate_schemas` <= 5.0 ms (C: 3.3); schema >= 1.2x locally. **Partly done, targets missed**: the direct `tuple`/`list`/`sorted` and the early item release alone; `_make_json_hashable` 9.3 ms and 120M instructions (before: 11.0 ms, 133M), `_deduplicate_schemas` 9.2 ms and 131M (before: 11.8 ms, 144M); schema about 1.12x with the three section 1 functions compiled (see below) |
| 11 | Size and layout (section 2.9); item 10's decision; item 18's re-measure; item 23 dead-helper sweep | pydantic `.so` <= 1.5 MB; import >= 1.0x with no regression of any phase. The I1 measurement in section 2.9 limits what this PR can win: the dead pending-exception tests are gone, and the other layout changes tried moved I1 misses by 8% at most |
| 12 | CI confirmation: `bench.yml` on both runners, ROADMAP accept | every section 4 target on both runners, cited by run id |

PRs 7-10 each touch a separate code seam (call lowering, guards, `try` lowering,
comprehension and builtin lowering). They can be reordered if a measurement says so; the
acceptance numbers stay.

**After the first lowerings of PRs 9 and 10** (measured locally on 3.15.0b4, 31 interleaved
rounds of 20 repetitions). With only the three section 1 functions compiled
(`--compile`), schema runs about 1.12x CPython (458.5M instructions against 557.9M, 1.22x;
4.12M I1 misses against 5.22M), define 1.00-1.03x, process 1.02-1.04x, import within noise
of 1.0x. The 1.2x schema target is not met. Two measurements bound what is left
(both taken with a list reuse in `sorted` that was since withdrawn, section 2.6, worth 7M
instructions and about 0.02x; with it schema ran 1.14-1.18x):

- with the live frame of every compiled body removed as an experiment (no frame link,
  no position stores), schema measured 1.19x: frames cost 6M instructions;
- a `PyDict_Next` loop for `for k, v in d.items()` would save an estimated 15-20M
  instructions, which puts schema at about 1.2x, within the noise of the measurement.

Section 1's hand-written C is the ceiling at 1.25x. Instruction savings convert to wall
time poorly here: compiling `remap_json_schema`, `remap_json_ref` and `_sort_recursive` as
well removed 20M more instructions and no wall time. `pycc profile` still selects nothing
for pydantic (pass 3 kept 0 of 55 candidates: its whole-workload wall-time gate is noisier
than the wins it has to see), so a default profile build of pydantic is CPython's speed.

**Order revised after PR 5.** With `--select module`, diff-match-patch's diff_main runs
7.1x CPython. With its training profile, it ran 1.02x: nothing was selected. Two causes:
- step 3 of section 2.3 then counted calls, not their cost, and rejected `diff_bisect` (98%
  of the self time);
- the report masked that as "not measured".

Selected by hand, `diff_bisect` alone is 2.8x and its region 2.6x. The gap to 7.1x is the
typed body that whole-module compilation infers from call sites, which a generic lift lacks.
So typed bodies (PR 8) are the lever and direct calls (PR 7) add little there:
- PR 5a fixes selection;
- PR 8 follows as the checkpoint;
- `--select module` stays until PR 8 shows profile mode can replace it (6b);
- PR 7 comes after that, only if measured to pay.

## 9. Open risks

- **Profile fit.** A profile that matches the bench workload more closely than real use
  would flatter the numbers. That is why section 4 requires the training input. PR 4 also
  reports schema built with a profile of pydantic's own test suite.
- **Patching relies on layouts.** It depends on the 3.13 and 3.15 `PyFunctionObject` layout
  (probed today). A new CPython needs the probe updated before the build supports it. An
  unknown layout must patch nothing, and the module must stay bytecode.
- **Import parity under load.** Loading a small `.so` costs about 0.13 ms (an empty `.so`)
  to 0.45 ms (pystache's), about 0.5% of pydantic's import. That is inside the IQR but
  could tip the median. A package with nothing selected gets no extension at all.
- **Test churn.** The non-strict `--ext` tests compiled whole modules until PR 6b, which
  converted them to `--select all`. Tests of the deleted mechanisms go with them.
- **Generator-expression tracebacks.** Inlining a generator expression needs the error path
  to add the `<genexpr>` frame record (section 2.6). The oracle tests must cover an
  exception raised inside one.
- **Define stays at parity.** Define cannot beat parity by compiling pydantic. 1.0x is the
  target, not a stepping stone.
- **Open issues.** #68 and #69, non-strict codegen of function values, are likely moot
  after PR 6 (module bodies are bytecode). Each is re-checked there and closed with
  evidence or kept.

## 10. The headroom harness

[`bench/headroom/`](../../bench/headroom/) holds the code behind section 1 (moved there
from this directory by PR 1). `PYTHONHASHSEED=0` throughout:

- `hrfast.c` is the hand-written ideal C for the three functions. It returns the same
  results and raises the same exceptions as the Python source for the inputs that occur,
  checked by `edge.py`.
- `inject.py` swaps the three functions in, in every module and class namespace that
  holds them.
- `drive.py TREE REPEAT INJECT MARK` runs the workload's define and schema phases.
  `hrmark.c` marks the phases for `--toggle-collect=hr_mark_region`.
- `timing.py N PYTHON PLAIN_TREE PYCC_TREE` runs interleaved fresh-process rounds of
  CPython, CPython with the C injected, and the pycc tree.
- `record.py` records the three functions' inputs during define and schema.
  `replay.py TREE IMPL FUNC MODE` replays them through one implementation: the tree's own
  function, the C, or a no-op baseline. It checks each result against the tree's own.

Build both C files against the interpreter's headers as ordinary extension modules (for
example `cc -O2 -shared -fPIC $(python3-config --includes) hrfast.c -o
hrfast$(python3-config --extension-suffix)`).

`bench/pydantic_check.py --cachegrind [PYTHON]` measures section 3's counts for every
pull request: each phase of `bench/pydantic_workload.py` alone, `define+schema` in one
process (as point 2 of section 1 counted them) and the whole process, each under
callgrind with `hrmark`'s region toggled, plus the extension's `.so` and `.text` size.
Rerun on section 1's own trees (built 2026-11-05, before #75 and #77), it gives
736.8M / 6.69M / 11.31M (instructions, I1 misses, conditional mispredictions) for the
compiled tree and 710.7M / 3.39M / 9.27M for CPython over define and schema, within 1%
of section 1. On main at 178cc87 it gives 722.4M / 6.74M / 10.80M against 704.5M /
3.39M / 9.18M.
