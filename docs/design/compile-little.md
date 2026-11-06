# Library compilation: compile little, compile well

Status: in progress. Implemented: PR 1 (measurement, section 3), PR 2 (`hot_api.c` deleted,
section 7 item 12), PR 3 (`pycc profile`, section 2.2; `src/profile.rs`), PR 4 (the switch,
sections 2.1-2.4; `src/select.rs`, `src/fallback/selected.rs`). Tracking issue:
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
  for users who measure for themselves. Until PR 6 (section 8), a transitional `--select module`
  keeps today's whole-module compilation for the existing tests. `[tool.pycc] compile = ["pkg.mod:qual", ...]`
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
function, its calls to Python functions by callee (section 2.2). A function is selected
when it is eligible, its profile entry is current, and:

- its self time is at least 0.5% of the run's total;
- it was called at least 50 times;
- it ranks in the top 64 functions by self time;
- it catches an exception in less than half of its sampled calls (`handled`).
  Exceptions as control flow run slower compiled than in CPython until PR 9 (section
  2.7): PR 4 measured `ConfigWrapper.__getattr__`, which catches a `KeyError` in 989 of
  its first 1,000 calls, at +0.2 ms of schema;
- most of its calls to Python functions stay inside the region: less than half leave it. Calls to itself, to
  its own nested functions and to the other selected functions stay inside. Rejecting a
  function can push its callers out, so this condition is applied until no function
  changes. A function that calls no Python function passes.

Thin glue (properties, accessors, functions that mostly call Python code) therefore
stays bytecode unless the functions it calls are selected with it. Inside a region the
calls become native direct calls in PR 7 (section 2.8). The rule uses only the
profile's numbers, never a function's name.

The rule's picks are candidates, not the selection: only measured winners are
selected. Pass 3 of `pycc profile` (`src/profile/verify.rs`) builds the candidates into
a copy of the package and times the training workload with them compiled against the
unchanged sources, interleaved in fresh processes after one discarded warm-up run of each
(`--verify-rounds`, default 7, at most 15; a check stops after 5 rounds once the two interquartile ranges are apart). The
verdict is on wall time: the compiled median must not exceed CPython's third quartile.
If all the candidates together are slower, each region (the candidates connected by
calls among them, most self time first) is checked alone, then the regions that were no
slower alone are checked together once more; at most 6 builds. Every candidate gets its
last check's outcome in the profile (`measured`: `compiled`, `with`, `cpython_ns`,
`compiled_ns` as first quartile, median and third quartile, and with
`--verify-instructions` the instruction delta under callgrind, as evidence only). A
function measured slower is rejected ("slower when compiled") by builds and `--check`
alike, and a later `pycc profile` keeps the rejection while the function's source hash
is unchanged, so builds do not flap (deleting the profile measures every function
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
`--select-outside`, `--select-handled`). They stay until the measurements in section 8 justify other
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

  As of PR 4, the whole source is one precompiled chunk of the existing chunk machinery,
  run in the module's namespace. `__file__` and `__spec__.origin` name the `.py` file,
  which ships beside the extension, so `inspect` and tracebacks read it. The loader
  does not have `get_code` and `get_source` yet: PR 5 replaces the chunk machinery.
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

### 2.7 Exceptions as control flow

A `try` whose body is one operation with a cheap miss test, and whose handler catches only
that miss, is lowered to the test:

| operation | handler | miss test |
|---|---|---|
| `d[k]` on an exact `dict` | `except KeyError` (or a tuple holding it) | `PyDict_GetItemRef` answers 0 |
| `o.name` | `except AttributeError` | `PyObject_GetOptionalAttr` answers 0 |
| `l[i]` on an exact `list` / `tuple` with an `int` index | `except IndexError` | bounds check |

When the handler can see the exception, the exception is made just before that point, so
CPython code observes the same exception:

- the handler binds it (`as e`) or re-raises it (`raise`);
- the handler calls anything that is not itself lowered this way, since that code could
  call `sys.exc_info()`;
- the handler raises, which needs the exception as `__context__`. A `from None` keeps
  `__suppress_context__` as CPython sets it.

`ConfigWrapper.__getattr__` (a dict miss, then a second dict miss, then `raise ... from None`)
is the case to match: no exception exists on its hit and first-miss paths.

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

### 2.9 Size and layout

- **Hot code.** Selected bodies are placed together in profile order.
- **Helpers.** The shim's generic helpers are cold and linked only when referenced
  (`--gc-sections`, as now).
- **Size check.** The size of the `.so` is part of every report.
- **Target.** pydantic's extension drops from 10.9 MB to about 1 MB or less, from
  measurements, not a fixed rule.

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
| 6 | Delete non-strict module-body compilation (items 6, 7, 8 slot sync, 9, 20) and `--select module` | as PR 5; non-strict test suite converted to `--select all` passes; LOC removed reported |
| 7 | Direct calls (section 2.8) inside the selected region (section 2.3), replacing non-strict sibling interfaces (item 21) | schema >= 1.05x (if missed, stop and report before PR 8); `_make_json_hashable` replay <= 7.0 ms (8.9 today), `_deduplicate_schemas` <= 8.0 ms (9.95) |
| 8 | Call-site specialization (section 2.5): exact-type guards inline, helpers cold, per-site attribute records (item 13), profile-observed types (item 19, then delete `specialize.rs`), `isinstance` exact path | schema I1 misses <= 1.1x CPython's; `ConfigWrapper.__getattr__` replay <= 1.75 ms (no slower than CPython); schema >= 1.15x |
| 9 | Exceptions as control flow (section 2.7) | `ConfigWrapper.__getattr__` replay <= 0.8 ms (C: 0.34) |
| 10 | Idiom lowering (section 2.6) | `_make_json_hashable` <= 4.5 ms (C: 2.9), `_deduplicate_schemas` <= 5.0 ms (C: 3.3); schema >= 1.2x locally |
| 11 | Size and layout (section 2.9); item 10's decision; item 18's re-measure; item 23 dead-helper sweep | pydantic `.so` <= 1.5 MB; import >= 1.0x with no regression of any phase |
| 12 | CI confirmation: `bench.yml` on both runners, ROADMAP accept | every section 4 target on both runners, cited by run id |

PRs 7-10 each touch a separate code seam (call lowering, guards, `try` lowering,
comprehension and builtin lowering). They can be reordered if a measurement says so; the
acceptance numbers stay.

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
- **Test churn.** The non-strict `--ext` tests compile whole modules today. They run on the
  transitional `--select module` from PR 4 to PR 6, and PR 6 converts
  them to `--select all`. Tests of the deleted mechanisms go with them.
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
