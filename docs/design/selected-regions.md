# Native calls inside a selected region

Status: R1 done, R4 measured and parked, R2, R3, R5 and R6 planned (section 5.1). Part of [compile-little.md](compile-little.md) (section 2.8, PR 7's second
part). Roadmap: [v0.13](../ROADMAP.md#v013--compile-little-compile-well), accept 2.

This plan lets the selected functions of one region call each other with native arguments and
native results. The type guard runs once, where the region is entered.

## 1. Problem

Since v0.12 builds compile only the functions pass 3 measures as winners, tomli's `loads()`
has fallen from 1.70x / 1.49x CPython (`bench.yml` run 41750538464, x86_64 / aarch64) to
1.42x / 1.31x at v0.12.0. v0.13 accept 2 asks for the old ratios again. The measurements below were taken locally on CPython 3.15.0b4, x86-64, with
`PYTHONHASHSEED=0`. Instructions are counted with callgrind as `(I(21) - I(1)) / 20`: the
steady-state cost of one `loads()` of tomli's `benchmark/data.toml` (3,893 characters), after
the first call has imported `tomli._re`. Wall times are medians of 15 interleaved
fresh-process rounds, each the best of 3 runs of 100 `loads()`.

| build | instructions per `loads()` | ratio | wall, 100 `loads()` |
|---|---|---|---|
| CPython | 4.082M | 1.00x | 39.3 ms |
| whole-module build from before the switch (every function compiled, types inferred from call sites) | 2.427M | 1.68x | 26.4 ms (1.49x) |
| profile build on main (pass 3's 19 functions, generic bodies) | 2.808M | 1.45x | 31.1 ms (1.26x) |

An earlier measurement, counted another way, put the same three builds at 6.84M, 5.20M and
5.64M per call. Their differences match these within 0.06M: the old
build is 1.65M (here 1.66M) under CPython, and the profile build 0.44M (here 0.38M) over the
old build.

An earlier attempt typed the selected lifts from their source annotations. tomli became
slower: 1.28x became 1.06x on the same machine. Only the small scanners `skip_chars` and
`skip_until` got native copies. Together they are called about 647 times per `loads()` and
run about 2.6 iterations a call. Across 20 `loads()` the attempt cost:

- about 2.10M in the generic wrapper around each call: the frame, the guard, the call and
  reboxing the result;
- 2.4M converting `src` to pycc's native `str`, and 1.5M in `pycc_ext_obj_coerce`;
- 4.6M in membership tests. `chars` stayed a CPython `frozenset`, so every native character
  was boxed again and looked up through `PySet_Contains`.

Every other parser function does `pos = skip_chars(...)`. The result of another selected
function is a CPython object, so `pos` could not be proven an `int` in any caller.

## 2. What exists, and what is missing

Checked against main at 40c5657.

**What exists:**

- **Generic lifts.** A selected function compiles as a lift
  (`src/fallback/selected.rs`). Every parameter and the result are `object`, and every module
  global is read from the namespace (`__pycc_fallback_ns['skip_chars']`).
- **Direct calls.** Generic bodies call each other directly through each lift's cell
  (`crates/pycc_codegen/src/direct_call.rs`; [call-site-specialization.md](call-site-specialization.md),
  PR 7's first part). The cell holds the function object, its code object and an argument-count
  mask. Arguments and results stay objects.
- **Observed specialization.** A parameter that is not rebound gets a native copy behind
  an exact-type guard when pass 2 observed one dominant type for it (95%) and a loop uses its
  native value (`specialize_observed`, `crates/pycc_ast/src/desugar/specialize.rs`). The
  guard sits at the top of the generic lift's body, after the trampoline, the argument binding
  and the generic frame. The copy returns `object`.
- **Tuple results without the tuple.** Multi-return twins return `(pos, value)` through
  out-pointers (`crates/pycc_mir/src/multi_return_object.rs`). The items are objects.
- **Fast paths in generic bodies.** Tagged locals hold `pos` as a word and characters as
  words. `src[a:b]` and `src.startswith(t, pos)` take word operands (`str_words.rs`). A
  one-character `str` tested against a `frozenset` reads one bit of a cached 128-bit ASCII
  mask (`pycc_ext_ascii_mask_fill`).
- **Native-call redirection in MIR.** A call `f(args)` of a compile-unit function goes to
  `_pycc_native_f` when every argument already has the copy's parameter type
  (`native_specialization`, `crates/pycc_mir/src/expr/call_lower.rs`). This is how
  whole-module builds called the `frozenset[T]` copies of `Iterable[T]` functions
  (`plan_iterable`). A selected unit never takes this path: a lift calls another lift through
  the namespace, so the call is an object call.
- **Return values in pass 2.** Pass 2 already registers a `PY_RETURN` callback
  (`src/profile/driver.py`), but it records argument types only.

**What is missing:**

1. A region of selected functions as a compile-time unit, with signatures.
2. Native results. Every copy returns `object`.
3. Native calls from one copy to another. Calls go through the namespace as object calls.
4. A rebound parameter that stays native. `pos` is excluded from every copy because the body
   rebinds it.
5. A guard outside the generic frame.
6. Native `int` operands in the `str` word paths. `str_words::word_leaf` accepts only `int`
   literals and `object` locals.
7. A character-class test that makes no character object.

## 3. The experiment and its bound

A selected unit is plain Python text, so the region can be written by hand and compiled by
the real compiler. For this experiment a pycc built from 40c5657 got a 6-line hook in
`render_selected`: `PYCC_UNIT_FILE=<file>` replaces the lifts' text with the file's, and the
trailer that patches the functions is unchanged. The hook was never committed.

The hand-written unit adds `__r_<name>` copies of 14 parser functions: `loads`'s loop,
`skip_chars`, `skip_until`, `skip_comment`, `skip_comments_and_array_ws`, `parse_key`,
`parse_key_part`, `parse_key_value_pair`, `key_value_rule`, `parse_value`, `parse_array`,
`parse_inline_table`, `parse_basic_str` and `create_dict_rule`. In each copy:

- `pos`, `nest_lvl` and the results that hold positions are typed `int`. Results are
  `int`, `tuple[int, object]` or `tuple[int, object, object]`.
- `src`, the frozensets and the values stay `object`.
- Calls between copies name the copies, so they compile as native calls.
- Calls to functions outside the group (`parse_literal_str`, `create_list_rule`, the regular
  expressions) stay object calls, and their positions are converted.
- The generic `loads` lift enters the copies once, behind `type(src) is str`.

Its result is identical to CPython's.

| variant (same 19 functions selected) | instructions per `loads()` |
|---|---|
| main, generic bodies (`m0`) | 2.808M |
| every eligible function generic, `--select all` (37 functions) | 2.766M |
| `skip_chars` inlined by hand into its callers, generic | 2.692M |
| **hand-written native region (`u1`)** | **2.657M** (-151K, -5.4%; wall 28.8 ms, 1.36x) |
| `u1` with `skip_chars` inlined into its callers | 2.670M |
| `u1` with `c == " " or c == "\t"` in place of `in TOML_WS` | 2.965M |

Two costs in `u1` come from gaps in the lowering, not from the region:

- **Native `int` operands miss the `str` word paths** (missing item 6). With `pos` native,
  `src[start:pos]` builds a `slice` (`PySlice_New`, `pycc_ext_obj_slice_of`,
  `unicode_subscript`, `slice_dealloc`), and `src.startswith(t, pos)` is a method call. Against
  `m0` these helpers add about 57K. `startswith` adds 8K, and boxing and coercing positions
  for them 15K: about **80K** together. The last variant shows the same gap for comparisons:
  `c = src[pos]` with a native `pos` makes an object character, and its comparisons go
  through `PyObject_RichCompare`.
- **`src[pos] in chars` makes a character object** (missing item 7). In `u1`, `__r_skip_chars`
  costs 206K per `loads()`: 615 calls (12,914 over 21 calls of `loads()`) and about 2,330
  tests. Each test reads the cached one-character `str`, increments and releases its count,
  checks its type, kind and length, looks up the mask cache, and tests the bit: about 55
  instructions in the generated IR. A test that loads the byte and tests the bit of a mask
  resolved once per call costs about 10 instructions. That would cut `__r_skip_chars` to
  about 35K, **about 170K** less, an estimate from the IR.
  Inlining `skip_chars` (the `u2` variant) does not help: the loop costs the same, and the
  frame of a native call is not where the time goes.

**The bound.** The region alone, measured, gives 2.657M per `loads()`. That is 9.5% above
the old build's 2.427M, and 1.54x CPython against the old 1.68x. Closing the two gaps it
exposes gives about 2.41M (estimated). Compiling the region's cold callees too gives about
40K more (`--select all` saved 42K over `m0`): about 2.37M, or 1.72x CPython in instructions,
at or just past the old build. In the earlier measurement's units, `u1` is about 5.43M against the old
build's 5.20M, and the projection is about 5.14-5.18M. Accept 2 is therefore reachable only
with all three parts (the region, native `int` word operands and character-class tests), and
only by a few percent.

Every copy had a native body, so `.text` grew from 833 KB to 967 KB (+16%) for 14 copies.

## 4. Design

### 4.1 Region formation

The call graph comes from the selected unit itself. A call in a selected body is a
**region edge** when:

- its callee is a module global that names another selected function, or a `self.m(...)`
  method lift as in the direct-call cell's rule;
- the global is not rebound in the module's source (section 2.8's condition: no other
  assignment, no `global` declaration, no `del`).

A **region** is a connected component of region edges among the selected functions.

A region is extended by its **position-flow callees**: an unselected function the region
calls whose result flows into a local the region types natively. For tomli these are
`parse_literal_str`, `parse_one_line_basic_str`, `create_list_rule`,
`parse_basic_str_escape` and `parse_multiline_str`. Pass 3 measures the region with these
callees included. The rule is static: a callee joins only when its source is in the module, it
is eligible (section 2.1), and every result that flows into the native local comes from a
`return` the compiler can type.

Members called from outside the region (bytecode, Python callers, other regions) are the
region's **entries**. Pass 2's `callees` counts tell which calls into a member come from
outside. Static edges alone cannot tell this, because a module's bytecode can call any member.
So every member stays an entry: each keeps its generic lift. The counts are approximate (a
member's total and each caller's count are medians taken separately across pass 2's runs), so
the report's entries are a guide, not a guarantee: the entry guard (R5) must not rely on a
member being no entry, nor on a region reporting `entries: none`.

### 4.2 Types: annotations, observations and proofs

Annotations and observations propose types. Only the compiler proves them, and only a
proven type is used.

- **Entry parameters** are guarded as today: the exact type pass 2 observed in 95% of the
  sampled calls (a `bool` is not an `int` there). A source annotation never types an entry
  parameter on its own: Python does not enforce it, and an unsampled function has no
  evidence of what its callers pass.
- **Inside the region** a parameter is native when every region call site passes a value the
  compiler proves of that type, and every entry passes a guarded one. A rebound parameter
  (`pos = skip_chars(...)`) stays native when every binding is proven of its type. This is
  item 4: `pos` becomes native because `skip_chars`'s copy returns a native `int`, not because
  of its annotation.
- **Results** are native when every `return` of the copy is proven of one type: `int`, or a
  tuple whose items are `int` or `object` (`tuple[int, object]`, which multi-return twins
  already lower). A source annotation (`-> Pos`, resolved through the module's simple
  aliases, including names imported under `TYPE_CHECKING` from a package module's
  `Pos = int`) is the hint that orders the attempts. A result the compiler cannot prove keeps
  `object`, whatever the annotation says.
- **`str` and `frozenset` stay CPython objects.** The guard proves them exact, and their
  operations take the inline paths of generic bodies. Converting `src` to pycc's native
  `str` cost the earlier attempt 2.4M, and every slice of it that leaves the region (keys,
  values) would be boxed again. A native copy of a frozenset would need its own
  invalidation, and a native character would still have to be hashed. The ASCII mask
  (section 4.5) answers membership without either.
- **`float` stays an object in the first version.** A NaN compares and hashes by identity, so
  passing one natively and boxing it again changes `x in [x]`. tomli and pypng do not need
  native `float`s. A later PR may add them under the escape rules
  (`specialize/escapes.rs`) with a measurement.

Return-type observations are not needed. A result is native only where the compiler proves
it, and a callee whose result would need checking joins the region instead (4.1). Pass 2's
`PY_RETURN` hook is not extended.

### 4.3 Calling convention

Each member gets a native copy `__pycc_rnative_<lift>`:

- native parameters in their native representation (a tagged `int` word), the others as
  `object`, in source order;
- the original objects of guarded entry parameters, appended as observed copies already do
  (the argument stays the argument);
- the result native, or `object`, or a tuple through the twin's out-pointers.

A region edge compiles as a direct call of the callee's copy, checked first against the
callee's cell, as section 2.8's direct calls are (one load and compare of the function
object and its code object per call). `Py_EnterRecursiveCall` is asked once per invocation of
the calling copy, as for direct calls today, so `RecursionError` is raised where CPython raises
it. A copy keeps its compiled frame, named as the original function. The `u2` variant shows
the frame is not the cost.

Generic lifts are unchanged for every caller outside the region.

### 4.4 Guards and deoptimization

- **Entry.** An entry checks its observed parameter types exactly (`type(x) is int`, so a
  `bool` or a subclass runs the generic lift), then calls the copy. Until the guard moves
  into the trampoline, this check is the one `add_guard` puts at the top of the generic
  lift (PR R5 below moves it).
- **Bindings at entry.** An entry also checks that the region is still intact: every
  member's module global still holds the compiled function the region was built from. The
  check is one load and compare of a per-region `intact` flag. A `PyDict_Watch` watcher on
  the module's dict clears the flag when any member's key is assigned or deleted (or the
  dict is cleared), and nothing sets it again. A `PyFunction_AddWatcher` watcher on each
  member's function object clears it too, on `PyFunction_EVENT_MODIFY_CODE` (a replaced
  `__code__`, as `tests/fixtures/direct_calls.py` exercises), `MODIFY_DEFAULTS`,
  `MODIFY_KWDEFAULTS` and `DESTROY`, so every mutation the per-call cell check detects also
  closes the region before the next entry. A region whose flag is clear is entered
  through the generic lifts only, so a member patched *before* a call (for example by
  `mock.patch` in a test's setup) behaves exactly as in CPython: no native copy runs and no
  conversion can fail. Only a rebinding made *while* a region call is running reaches the
  `TypeError` path below, which is what section 6 and the DEVIATIONS row promise. If
  either watcher cannot be installed (no free watcher id), the region is not formed and its
  members stay generic lifts; `--report-fallbacks` says so.
- **Inside.** No argument check is needed: a native argument is proven. An `object` argument
  passed to an `object` parameter needs none either.
- **A rebound callee.** If the cell check fails (`mock.patch`, a replaced `__code__`, a
  global rebound by code the region called), the call goes to whatever object the name holds.
  It goes through the object protocol, with native arguments boxed: CPython calls the new
  object too. Its result is converted where the caller needs a native value. When it is not
  of that exact type, the call raises `TypeError` at the call site (`skip_chars() returned
  str, the compiled caller needs int`). DEVIATIONS already has this rule for a compiled
  method that a Python subclass overrides. Here it applies to a module function rebound
  while a region call that reaches it is running.
  Mid-function deoptimization into the generic body is not attempted: it would need the
  generic body to resume at any call site with the copy's locals, for a case no measured
  workload has.
- **An entry with other types** runs the generic lift. The generic lift calls the region's
  members through their generic lifts, which may enter the region again. Nothing is shared
  between the two paths.

### 4.5 Membership and characters

`s[i] in C`, with `s` an exact `str` (guarded or proven), `i` an `int` word and `C` an exact
`frozenset` whose ASCII mask is usable, loads the byte and tests the bit. No character object
is made. The mask entry is looked up once where the loop starts, since `C` is not rebound in
the loop and a `frozenset` cannot change. Anything else takes today's path: a non-ASCII
`str`, an index out of range (the `IndexError` is raised as now), a set with more than 256
members or with a member that is not an exact `str`, or a `set`.

The same holds for `s[i] == "x"`, `s[i] != "x"` and `s[i] in "xyz"` with a one-character
or short literal. This also applies in generic bodies whose index is a tagged word, so it
helps `m0` as well.

### 4.6 Semantics against CPython

- **Results and exceptions** are CPython's. The oracle tests (section 5) cover:
  - the entry guard's misses (`bool`, `int` subclasses, a big `int`);
  - `IndexError` at the end of `src`;
  - a member rebound before a call (the region is not entered: CPython's result, no
    `TypeError`) and during one (section 4.4's deviation);
  - the recursion limit;
  - tracebacks through copies.
- **Identity of `int`s.** An `int` that passes between copies natively, and then reaches
  Python unchanged (returned, stored, passed to bytecode or to an exception), is a new object
  equal to the old one, where CPython would pass the argument's own object. Only `is` and `id()`
  on `int`s outside `-5..256` can see it. This needs a new DEVIATIONS row, next to the one
  for observed-type specializations.
  `int`s an operation makes are new objects in CPython as well, so they are not affected.
- **A rebound member's mismatched result** raises `TypeError` (4.4). This needs a DEVIATIONS
  row.
- **Floats** are not passed natively (4.2), so NaN identity is unchanged.

### 4.7 Pass 3 measures a region as a unit

A typed region's members do not compile alone the way they do together: removing one changes
its neighbors' signatures. So:

- a check's set is a union of whole regions and single functions: the regions of its own
  selected unit, since each check builds exactly its set (a hot core does not take along
  the cold members of the regions the larger set formed: 5.2);
- pruning (`src/profile/verify/prune.rs`) removes a whole region of the winner at a time,
  never one member;
- a region's position-flow callees (4.1) are part of it and are never pruned alone;
- the winner's key stays its members, so builds compile exactly the measured set.

### 4.8 Generators and closures

Generators and coroutines are not eligible (section 2.1), so a region never contains one.
A generator calls a member through the member's patched function, which is an entry.

A local (or parameter) that any nested scope captures, whether a lambda, a nested `def`, a
generator expression or a comprehension, is never native in a copy: it stays a CPython object
in the shared cell, exactly as in the generic lift. Boxing it only when the closure is created
would break late binding: in `read = lambda: pos; pos += 1; return read()` CPython returns the
incremented position. Excluding captured names costs nothing measured, since no tomli scanner
captures `pos`. R2's tests include this late-binding case, after the existing checks in
`tests/closure_soundness.rs`.

### 4.9 Not in this plan

- Native `float` and native `str` in regions.
- Return-type observation.
- Regions across modules. A `from .m import f` edge is checked against the binding module's
  dict, as in section 2.8; it may follow once a single module's region is measured.

## 5. Pull requests, in order

Each PR is mergeable on its own and reports section 3's numbers before and after (`m0`
selection, callgrind per `loads()` and wall), plus pydantic's, mistune's, pystache's and
pypng's checks with no workload slower. Under compile-little section 6, a PR that misses its
number stops the sequence for a measurement and a revision of this plan.

| PR | change | tests | expected effect |
|---|---|---|---|
| R1 | Regions: formation from the selected unit (4.1, without the position-flow callees), `--report-fallbacks` lines `region: m: a, b, c (entries: ...)`, and pass 3 measuring and pruning regions as units (4.7) | unit tests of formation on a fixture module: an edge through a rebound global, a method edge, two components, a self-recursive function; prune tests with a two-member region; `package_check` reports tomli's region | no change in instructions (no code is generated differently); the five packages' selections unchanged or each change explained |
| R2 | Native `int` calls inside a region (4.2-4.4, 4.6): native copies, proven parameters and results, rebound parameters proven native, region edges as checked direct calls, `TypeError` on a rebound member's mismatched result; position-flow callees join the region; two DEVIATIONS rows | `tests/select_regions.rs` against CPython: tomli-shaped scanners and tuple-returning parsers; entry with `bool`, an `int` subclass and a big `int`; `mock.patch` of a member before a call (the generic path runs, result matches CPython for both result types) and from inside one (the `TypeError` row); the `intact` flag cleared by assignment, deletion, `dict.clear()` and a replaced `__code__` or `__defaults__` before a call (CPython's result, no `TypeError`); `RecursionError` depth; a rebound `pos` captured by a lambda or generator expression (late binding, 4.8); traceback function names; `is` of a returned position, matching the DEVIATIONS row | tomli `loads()` <= 2.66M per call locally (`u1`: 2.657M), wall >= 1.33x; `.text` growth reported (`u1`: +16%) |
| R3 | Native `int` operands in the `str` word paths: `s[a:b]`, `s.startswith(t, a)`, and the literal comparison of `s[i]` (item 6) | `tests/tagged_words.rs` extended with native `int` operands, fast paths on and off, a `str` subclass receiver | about -80K: tomli <= 2.59M; `diff_bisect` (native `int` locals) not slower |
| R4 | Character-class tests without a character object (4.5, item 7), in copies and generic bodies | membership against CPython: ASCII and non-ASCII `src`, the end of `src`, a set of more than 256 members, a set with a non-`str` member, a `set` and a `frozenset` subclass, a mask evicted in the loop | about -170K estimated (tomli <= 2.45M); a prototype on `m0` measured **-85K**, under the 120K of section 6: parked until R2 lands (5.1) |
| R5 | The entry guard in the patched trampoline (`forward.c`), ahead of the generic frame and binding: exact types checked on the vectorcall arguments, the copy called directly, its result boxed (item 5) | entry with each guarded type and each miss; keyword calls and defaults take the generic path; `PYCC_NO_FAST_OPS=1` | per call from bytecode into a typed member, at least 100 instructions fewer (the earlier attempt's wrapper: about 160 per call); tomli unchanged (one entry per `loads()`); pypng `read()` measured |
| R6 | CI confirmation in `bench.yml` on both runners | | accept 2's tomli numbers (1.70x / 1.49x) met, or the gap measured and this plan revised |

R3 and R4 do not depend on R1 and R2 and may land first if a measurement says so. Their
acceptance numbers then apply to `m0`, measured again.

### 5.1 Status

- **R4, measured first.** The ASCII-mask test of `s[i] in frozenset` (4.5) was prototyped
  on generic bodies (`m0`, never merged) and saved **85K instructions per `loads()`**, half
  of the 170K section 3 estimated from the IR, and under the 120K section 6 sets as the
  point to stop. The test itself got cheap; what remains is each `skip_chars` call's fixed
  cost, about 155 instructions a call (with `skip_until`, about 647 calls per `loads()`), which the
  scanners' 2-3 iterations a call cannot amortize. That cost is the generic wrapper,
  argument binding and boxed results that R2's native calls inside a region remove. The
  saving stacks with R2, since the per-test cost stays inside a region, so R4 is parked
  until R2 lands and is then measured again on top of it.
- **R1, done.** Regions are formed from the selected unit (4.1, without position-flow
  callees: `src/fallback/regions.rs`), a build's `--report-fallbacks` lists them, and
  pass 3 prunes them as units (4.7, `src/profile/verify/units.rs`). No code is generated
  differently. Measured below (5.2).

### 5.2 R1's measurements

Taken locally on CPython 3.15.0b4, x86-64, as section 3's.

- **No code generated differently.** tomli built with section 3's `m0` selection by main
  and by R1: `_parser`'s `.text` is byte for byte the same, `.rodata` differs only in the
  build directory's name, and both run 2,821,441 instructions per `loads()`.
- **Selections.** `bench/package_check.py` (tomli, pystache, pypng, mistune) and
  `bench/pydantic_check.py`, each with its training profile, by main and by R1:

  | package | regions of the winner (`--report-fallbacks`) | selection |
  |---|---|---|
  | tomli | `loads`, `skip_chars`, `skip_until`, `skip_comment`, `skip_comments_and_array_ws`, `create_dict_rule`, `key_value_rule`, `parse_key_value_pair`, `parse_key`, `parse_key_part`, `parse_array`, `parse_inline_table`, `parse_value` (entries: `loads`, `skip_chars`, `skip_until`, `parse_key`) | unchanged, 19 functions; `loads()` 2.80M instructions per call either way |
  | pypng | `Reader.undo_filter`, `Reader._deinterlace`, `Reader._bytes_to_values` (entries: all three) | unchanged, 8 functions; `read()` 13.75M either way |
  | pystache, mistune | none (pass 3 keeps no set) | unchanged, none |
  | pydantic | `_DefinitionsRemapping.remap_json_schema`; `GenerateJsonSchema._sort_recursive` (self-recursive); `_deduplicate_schemas`, `_make_json_hashable` | unchanged, 5 functions, -7.26% of the training workload's instructions either way |

  One of three pydantic runs by R1 kept 33 functions instead: the 9-function set both
  main and R1 build lost pass 3's wall-time veto in that run (274.3 ms against
  CPython's 249.5 ms, at -6.36% instructions), so pass 3 took the next set. Two further
  runs by main and one by R1 passed the same set and pruned it to the same 5 functions;
  the veto is noise main is exposed to as much.
  The entries are the members pass 2 counted called from outside the region: tomli's
  other parser functions (`parse_literal_str`, `parse_basic_str`, ...) call `skip_chars`,
  `skip_until` and `parse_key` directly, so those are entries as well as `loads`.
- **The rule as first built.** A first version formed the regions over all of pass 3's
  candidates and made every set it measured whole under them: a hot core took its
  regions' cold members along. tomli's selection grew from 19 functions to 29 (the whole
  22-member region of the candidates, -48K instructions per `loads()` with generic bodies,
  +14% `.text`) and pypng's from 8 to 19 (+20K per `read()`), as pruning could no longer
  remove one costly member of a region. (Its pydantic run also kept 33 functions, but
  because its 12-function core lost the wall-time veto, as above.) Since each check builds exactly its set,
  its own selected unit's regions are the ones that matter (4.7), and pruning removes
  those of the winner.

## 6. Risks

- **The margin is thin.** The projection beats the old build by about 2%, and part of it is an
  estimate (R4's 170K, from the IR). If R2 measures above 2.66M, or R4 saves less than 120K,
  accept 2 is not reachable by this plan, and the next step is a measurement, not more tuning.
- **Code size.** Each member is compiled twice (lift and copy): +16% `.text` for tomli. That
  costs I1 misses, which section 2.9 found to dominate generic bodies. Wall time stays the
  verdict in pass 3.
- **Over-wide regions.** A region joins everything connected by calls, so in pydantic's
  glue a large component could pull in cold code. Pass 3 prunes whole regions, so such a
  region is kept only if it wins as a whole. Copies are made only for members with a native
  parameter or result.
- **Rebinding during a call.** The `TypeError` path (4.4) is a deviation. Tests must show it
  is reached only by a rebinding made while a region call is running.
- **Annotation aliases.** Resolving `Pos` through `TYPE_CHECKING` imports reads another
  module's source. A wrong resolution only orders the attempts, since types are proven, but
  it costs build time.

## 7. Reproducing section 3

The scripts were kept outside the repository. To repeat the experiment:

1. Build pycc from 40c5657 with the `PYCC_UNIT_FILE` hook (6 lines at the end of
   `Plan::render_selected` that replace the lifts' text before the trailer).
2. Dump the 19-function unit of `tomli/_parser.py`:
   `PYCC_FALLBACK_DUMP=1 pycc build tomli/_parser.py -o tomli/_parser --ext --non-strict --release --foreign-relative-imports --select none --compile tomli._parser:...`.
3. Add the `__r_` copies described in section 3, and make the `loads` lift call
   `__r_loads(src, parse_float)` behind `type(src) is str`.
4. Rebuild with `PYCC_UNIT_FILE`, and count
   `python3.15 -c 'import tomli; [tomli.loads(doc) for _ in range(N)]'` under callgrind for
   N = 1 and N = 21.
