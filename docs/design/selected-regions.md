# Native calls inside a selected region

Status: R1, R2 (with its code-size follow-ups), R3, R4 and R5 done (sections 5.2 and
5.4-5.7), R5b and R6 planned (section 5.1). Part of [compile-little.md](compile-little.md) (section 2.8, PR 7's second
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
calls whose result flows into a local the region types natively. For tomli these are seven:
`parse_literal_str`, `parse_one_line_basic_str`, `create_list_rule`,
`parse_basic_str_escape`, `parse_basic_str_escape_multiline`, `parse_multiline_str` and
`parse_hex_char` (the last one through the escape parsers). Pass 3 measures the region with
these callees included. The rule is static and runs to a fixpoint:

- a callee is a candidate when a member calls it through a region edge's binding rule (a
  module global bound once by its own undecorated `def`), or through a local bound only to
  such globals (4.2, B2), and the call's result, or the first item of a tuple result, is
  bound to a local the member types natively (`pos = f(...)`, `pos, x = f(...)`,
  `return f(...)` from a member whose result is native);
- it joins when its source is in the module and it is eligible (section 2.1), and every
  `return` that feeds that result is proven of the native type once the callee is typed as
  a member (4.2). A callee whose returns cannot all be proven stays outside, and the
  member's local stays `object`;
- a callee that joins is a member: its own calls are examined the same way, so
  `parse_hex_char` joins through `parse_basic_str_escape`.

A position-flow callee is never an entry in pass 2's sense (it was not selected), but it is
still called from bytecode it does not see, so it keeps its generic lift like every other
member.

Members called from outside the region (bytecode, Python callers, other regions) are the
region's **entries**. Pass 2's `callees` counts tell which calls into a member come from
outside. Static edges alone cannot tell this, because a module's bytecode can call any member.
So every member stays an entry: each keeps its generic lift. The counts are approximate (a
member's total and each caller's count are medians taken separately across pass 2's runs), so
the report's entries are a guide, not a guarantee: the entry guard (R5b) must not rely on a
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
- **What the prover knows** (added after R2's first attempt, 5.3). With only the rules above,
  every `pos` chain in tomli starts unproven, and one unproven binding keeps the local
  `object` everywhere it flows. Three facts start the chains without a checked conversion:
  - **B1, exact `str` along region edges.** A parameter guarded exact `str` at entry is still
    the same object in a callee reached through region edges when every edge passes the
    caller's own unrebound parameter (or another value proven exact `str`). It stays a CPython
    object (below), but the compiler knows its type exactly, so `s.index(t, i)`,
    `s.find(t, i)`, `len(s)` and the other `str` operations whose result type CPython fixes
    give a proven `int`. tomli's `skip_until` (`new_pos = src.index(expect, pos)`) needs this.
  - **B2, a local bound only to members.** A local whose every binding reads a module global
    that a region edge could call (each bound once by its own undecorated `def`, every one a
    member) is called as a multi-way edge: the call compares the local with each member's
    function object and calls that member's copy; anything else is a generic call. tomli's
    `parse_basic_str` (`parse_escapes = parse_basic_str_escape` or
    `parse_basic_str_escape_multiline`, then `pos, parsed_escape = parse_escapes(src, pos)`)
    needs this. Only the intact flag (4.4) makes "anything else" unreachable while the
    region runs, so the generic arm converts its result as a rebound callee's (4.4).
  - **B3, `re.Match` positions.** `m.end()`, `m.start()` and `m.span()` are proven `int`
    (a `tuple[int, int]`) when `m` is the result of `P.match(...)`, `P.fullmatch(...)` or
    `P.search(...)` tested for truth, and `P` is a module global bound once (by an
    assignment or an import) to an object that is an exact `re.Pattern` when the region is
    formed at run time. The global's key is watched with the members' keys (4.4), so a
    replaced pattern clears the intact flag and the region is not entered again; otherwise
    the result is `object`. tomli's `parse_value` (`return datetime_match.end(), ...`)
    needs this.

  None of the three adds a check that can fail: each proof holds whenever the region is
  intact, and an entry runs a copy only then. A conversion is emitted only on the generic arm
  of a call whose callee was rebound while a region call is running (4.4).
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
- the result native, or `object`, or a tuple through the twin's out-pointers.

Unlike an observed-type copy, a region copy is not passed the original objects of its native
parameters: an `int` it returns unchanged is boxed again, a new object equal to the argument
(4.6, and the DEVIATIONS row).

A region edge compiles as a direct call of the callee's copy, checked first against the
callee's cell, as section 2.8's direct calls are (one load and compare of the function
object and its code object per call), or, equivalently, against the region's intact flag,
which the same mutations clear (4.4). `Py_EnterRecursiveCall` is asked once per invocation of
the calling copy, as for direct calls today, so unbounded recursion through copies raises
`RecursionError` instead of overflowing the stack. The depth at which it is raised is not
CPython's: on 3.12 and later `Py_EnterRecursiveCall` checks the C stack, not
`sys.getrecursionlimit()`, which bounds only Python frames (the generic path behaves the
same). Only a copy that can close a cycle of region edges (a member that reaches itself, as
`parse_value` does through `parse_array`) pays it; a copy no cycle passes through cannot
recurse without leaving the region. A copy keeps its compiled frame, named as the original
function. The `u2` variant shows the frame is not the cost.

**Method edges stay generic in R2.** The intact flag sees module globals and function
objects, not a subclass's override of a method or an instance attribute that shadows one, so
a native `self.m(...)` edge would need a per-call check of the receiver's type and the
method's cell. tomli's region has no method edge; pypng's (three `Reader` methods) is
built as today. A later PR may add the per-call check with a measurement.

**Watchers.** The dict watcher and the function watcher (4.4) are each registered once per
process and shared by every pycc artifact loaded in it, through a capsule, as
`src/ext/module/global_sync.c` shares its dict watcher. The function watcher's callback
returns at once on `PyFunction_EVENT_CREATE`, which runs for every function CPython creates.

Generic lifts are unchanged for every caller outside the region.

### 4.4 Guards and deoptimization

- **Entry.** An entry checks its observed parameter types exactly (`type(x) is int`, so a
  `bool` or a subclass runs the generic lift), then calls the copy. Until the guard moves
  into the trampoline, this check is the one `add_guard` puts at the top of the generic
  lift (PR R5b below moves it).
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
  object too. Its result is converted where the caller needs a native value: an `int` (or a
  subclass, converted to a plain `int`), or a tuple of the copy's shape. Anything else raises
  `TypeError` at the call site, naming the callee as the source spells it (`skip_chars()
  returned str, the compiled caller needs int`; for an item of a tuple result,
  `parse_value() returned a tuple whose item 0 is str, the compiled caller needs int`).
  A pattern global (B3) replaced while a call runs reaches the same conversion in the callers
  of the member that used it. A copy that reads the replaced pattern itself, after the
  replacement, raises the compiled conversion's own `TypeError` where it uses the `end()`
  result (`return value must be int, not str`). DEVIATIONS already has this rule for a compiled
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
  - unbounded recursion through copies (`RecursionError`; the depth is not compared, 4.3);
  - tracebacks through copies and out of an entry (an entry guard's call of its copy records
    no frame of its own, so each Python call has one).
- **Identity of `int`s.** An `int` that passes between copies natively, and then reaches
  Python unchanged (returned, stored, passed to bytecode or to an exception), is a new object
  equal to the old one, where CPython would pass the argument's own object. Only `is` and `id()`
  on `int`s outside CPython's small-int cache (`-5..1024` on 3.14 and later, `-5..256` on
  3.13) can see it. This needs a new DEVIATIONS row, next to the one
  for observed-type specializations.
  `int`s an operation makes are new objects in CPython as well, so they are not affected.
- **A rebound member's mismatched result** raises `TypeError` (4.4). This needs a DEVIATIONS
  row. It is the only place R2 converts an object to a native type; B1-B3 (4.2) prove their
  values instead, so an intact region never reaches it.
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
| R2 | Native `int` calls inside a region (4.2-4.4, 4.6): native copies, proven parameters and results, rebound parameters proven native; the prover's facts B1 (exact `str` along edges), B2 (a local bound only to members, dispatched on identity) and B3 (`re.Match` positions from a watched exact `re.Pattern` global); region edges as direct calls checked against the intact flag, which a shared `PyDict_Watch` watcher on the module dict and a shared `PyFunction_AddWatcher` watcher on the members clear; the recursion check on copies that close a cycle; method edges generic; captured locals never native; position-flow callees join the region (4.1); no checked conversion outside a rebound callee's generic arm; the two DEVIATIONS rows of 4.6 only if a test reaches them | `tests/select_regions.rs` against CPython: native edge results; a member global rebound after a first call, and `__code__` or `__defaults__` replaced, then called (CPython's result, no `TypeError`); the `RecursionError` depth; a member called from outside the region; B2 dispatching to each target and to a non-member; B3 with the pattern global replaced; a captured `pos` (late binding, 4.8); entry with `bool`, an `int` subclass and a big `int` | tomli `loads()` <= 2.66M per call locally (`u1`: 2.658M, `u1x`: 2.630M), wall >= 1.33x; pypng and pydantic not slower; `.text` growth reported |
| R3 | Native `int` operands in the `str` word paths: `s[a:b]`, `s.startswith(t, a)`, and the literal comparison of `s[i]` (item 6). Refined against R2's tree (5.5): R2 already hands native slice bounds and indexes to `pycc_ext_obj_getslice_int` and `pycc_ext_obj_getitem_int`, so no `slice` object is built for them; what is left is that call, `startswith` as a method call with a boxed position, a slice whose other bound is an `object`, and a character loaded from a native index, which stays an object | `tests/region_words.rs`: a region whose copies slice, test `startswith` and load characters at native positions, against CPython with fast paths on and off: a `str` subclass receiver, non-ASCII text, big `int` positions, indexes past the end; the copies' IR takes the word paths | about -75K: tomli <= 2.47M after R2's code size; `.text` at most about 800K; pypng's `read()`, pydantic's workload and `diff_bisect` (native `int` locals) not slower |
| R4 | Character-class tests without a character object (4.5, item 7), in copies and generic bodies | membership against CPython: ASCII and non-ASCII `src`, the end of `src`, a set of more than 256 members, a set with a non-`str` member, a `set` and a `frozenset` subclass, a mask evicted in the loop | about -170K estimated (tomli <= 2.45M); a prototype on `m0` measured **-85K**, under the 120K of section 6, and was parked until R2 landed (5.1); measured again on top of R3, the per-test prototype saves 47K, and the loop prepared once (5.6) 96K: tomli <= 2.42M, `.text` at most +5% |
| R5 | The fixed cost of each call into and inside a region (refined against R4's tree, 5.7): a copy's frame linked without the wrapper checks and unlinked from words of its own; global reads without count traffic on the namespace; a compiled method's forwarder calling its body directly when the call leaves the later parameters to their defaults | `tests/forwarder_defaulted_calls.rs` against CPython: defaults left out positionally and by keyword, `__kwdefaults__` changed in place or dropped, `__defaults__` replaced, a keyword-only parameter left without a default, with the caches on and off; the region tests' tracebacks and `RecursionError` (`tests/select_regions.rs`) | tomli <= 2.30M (about -75K), `_parser` `.text` at most +5%; pypng's PngSuite decode, pydantic's workload and `diff_main` not slower |
| R5b | The entry guard in the patched trampoline (`forward.c`), ahead of the generic frame and binding: exact types checked on the vectorcall arguments, the copy called directly, its result boxed (item 5) | entry with each guarded type and each miss; keyword calls and defaults take the generic path; `PYCC_NO_FAST_OPS=1` | per call from bytecode into a typed member, at least 100 instructions fewer (the earlier attempt's wrapper: about 160 per call); tomli unchanged (one entry per `loads()`); pypng `read()` measured |
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
- **R2, done.** Measured below (5.4): tomli's `loads()` at 2,591,280 instructions, under the
  2.66M target, at +77% `.text` for `_parser`.
- **R2's code size, done.** Measured below (5.4): `_parser`'s `.text` +27% over main before
  R2 (the plan allowed +30%), at 2,543,978 instructions per `loads()`. (a) An entry guard
  unpacks its copy's tuple result, so the call takes the copy's multi-value twin, and the
  guard builds the tuple; a compiler-generated function nothing emitted calls (a copy only
  entries called, a twin no unpack calls, a conversion of a dropped arm) is not emitted
  (`pycc_mir::unreferenced`). An unpack's fallback counts as emitted, except a native
  twin's call of its original, which codegen makes only when the original is kept. (b) The generic arm of each region call is a *slow call*
  shared by every site of its shape (the kinds of its arguments, their keywords and the
  result's conversion): `cold`, `noinline` and without a frame of its own
  (`regions/slow.rs`). (c) The intact flag is a byte compiled code loads and compares where
  it reads it (`pycc_ext_region_flags`). And (e), found by the breakdown below: the lift of
  a member with a copy takes no object multi-value twin: while its region is intact its
  callers run the copy, and the lift runs only when a guard sends a call to the generic
  path (`pycc_mir::multi_return_object`).
  (d) `loads`' copy, the only one with neither a native parameter nor a native result,
  stays: its `pos = 0` starts every position the prover proves, and without it no member
  has anything native (2.82M instructions, `m0`'s). The fixture's `parse` and `tails`
  start theirs the same way (`pos = 0`, `skip(s, 0)`).
- **R3, done.** Measured below (5.5): tomli's `loads()` at 2,469,841 instructions, under the
  2.47M target, at 770,951 B of `.text` for `_parser`. In a copy, a native `int` local is a
  word operand of the `str` word paths (`pycc_codegen::str_words`): `s[a:b]` with native
  bounds, or `leaf + leaf` / `leaf - leaf` of native locals and literals, takes
  `pycc_ext_str_slice` directly; `s.startswith(t, a)` compares the bytes inline; and
  `c = s[pos]` with a native `pos` makes `c` a tagged character (`tagged_locals`), whose
  comparisons and membership tests read the word. A small native `int` is read by its tag;
  a big one or a `str` subclass takes CPython's operation as before, and so does non-ASCII
  text for `startswith` and the character load.
- **R4, done.** Measured below (5.6): tomli's `loads()` at 2,374,567 instructions, under the
  2.42M target set from the prototype measured again on top of R3, at 774,151 B of `.text`
  for `_parser` (+0.4%). The prototype's per-test path alone saved 47K; the rest comes from
  preparing the loop: a `while s[i] in chars:` whose `s` and `chars` are parameters the loop
  only reads checks both once, before the loop, and copies the set's mask, so each test is
  a bounds check, a byte load and a bit test (`pycc_codegen::char_class`).
- **R5, done.** Measured below (5.7): tomli's `loads()` at 2,293,301 instructions, under the
  2.30M target, at 753,527 B of `.text` for `_parser` (-2.7%). The row was refined against
  R4's tree: tomli enters its region once per `loads()`, so the trampoline's entry guard
  (now R5b) cannot move it, while the 1,800 or so copy frames, 1,364 global reads and 88
  forwarder calls with defaults left out per `loads()` each paid a fixed cost.
- **Next: R5b**, the entry guard in the patched trampoline, then R6.
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

### 5.3 R2's first attempt

A first attempt at R2 stopped before any code was written, because the proof rule of 4.2 as
first written ("a result the compiler cannot prove keeps `object`") leaves every `pos`
chain in tomli unproven. Each chain starts at one of three bindings:

1. `skip_until`: `new_pos = src.index(expect, pos)`. `src` is an `object` parameter, so its
   method's result is an `object`.
2. `parse_basic_str`: `pos, parsed_escape = parse_escapes(src, pos)`. The callee is a local
   bound to one of two module functions, not a region edge.
3. `parse_value`: `return datetime_match.end(), ...` (and the two other `re.Match` results).
   The match comes from a module-global compiled pattern.

With one type per local, solved to a fixpoint over the region, one unproven binding makes
`pos` an `object` in its function, then in every caller that binds a result of it, until no
position is native anywhere. The fix keeps the rule and teaches the prover the three facts
B1-B3 (4.2). Writing checked conversions at the three sites instead (`int(x)` raising
`TypeError` on any other type) was rejected: it adds a deviation where CPython would go on
with whatever object the call returned.

Measured locally as section 3's (CPython 3.15.0b4, x86-64, `PYTHONHASHSEED=0`, callgrind,
`(I(21) - I(1)) / 20`), with the hand-written unit of section 3 rebuilt by main at 9129c742
and section 7's hook:

| variant | instructions per `loads()` |
|---|---|
| `m0`: main, the 19-function selection, generic bodies | 2,806,989 |
| `m1`: `m0` plus the seven position-flow callees (4.1), generic | 2,734,044 |
| `u1`: the hand-written region, checked `int` conversions at the three sites | 2,657,918 |
| `u1x`: `u1` plus the seven position-flow callees as copies | 2,629,605 |

- The checked conversions stand where B1-B3 will prove the values, so `u1` and `u1x` bound
  what R2 can reach with the 19 functions and with the position-flow callees.
- The copies are called about 1,414 times per `loads()`, 615 of them `skip_chars`.
- About 50K of `u1`'s instructions are R3's: with `pos` native, `src[a:b]` builds a `slice`.

The target stays 2.66M per `loads()`, and the stop rule of section 5 stands: if R2 as built
measures above it, the implementation pull request is not opened, and the next step is a
per-function breakdown and a revision of this plan.

### 5.4 R2's measurements

Taken locally as section 3's (CPython 3.15.0b4, x86-64, `PYTHONHASHSEED=0`, callgrind,
`(I(21) - I(1)) / 20`), tomli with the 19-function selection:

| build | instructions per `loads()` | wall vs CPython | `_parser` `.text` |
|---|---|---|---|
| main (R1) | 2,821,369 | 1.30x | 598,119 B |
| R2 | 2,591,280 (-8.2%) | 1.51x | 1,058,631 B (+77%) |
| R2 after its review fixes (named conversion errors) | 2,592,008 (-8.1%) | | 1,075,175 B (+80%) |

- The target, 2.66M, is met. Wall times are noisy on this machine; `bench.yml` (R6) is the
  verdict.
- `.text` grew 77%, against the 16% section 3 estimated for 14 hand-written copies. On the
  fixture of `tests/select_regions.rs` the code splits into generic lifts 70.5K, the lifts'
  multi-value twins 42.6K, copies 57.0K, the copies' twins 41.6K and the conversions 5.1K:
  every member is compiled four times over (lift, copy and a twin of each), and every region
  call carries its generic arm inline. The follow-ups in 5.1, measured below, cut this.
- pypng's `read()` is unchanged (its region's edges are method edges, generic in R2);
  pydantic's regions get no copies.

R2's code size (5.1), each step on top of the one before, measured the same way (the
module's bytecode cache written before either count, so both runs import the same):

| build | instructions per `loads()` | `_parser` `.text` |
|---|---|---|
| R2 | 2,592,013 | 1,075,175 B (+80%) |
| (c) the intact flag loaded inline | 2,570,885 | 1,069,767 B |
| (a) guards take the copies' twins; unreferenced functions dropped | 2,563,533 | 925,799 B |
| (b) slow calls for the generic arms | 2,544,682 | 907,751 B (+52%) |
| (e) no multi-value twins for the lifts of members with copies | 2,543,978 (-1.8%) | 760,807 B (+27%) |

- After (a)-(c), `.text` stood at +52%. By symbol, the lifts' multi-value twins were
  143K of it: they double the generic bodies for a path an intact region never takes.
  (e) dropped them; instructions stayed level (2,544,682 before, 2,543,978 after).
- Where `_parser`'s `.text` goes now, by symbol, against main before R2 (591,875 B of
  symbols there, 753,937 B here): the 24 generic lifts 245.5K (+66.2K: the seven
  position-flow callees' lifts 52.6K, the guards in the members' lifts about 13.6K), the
  lifts' twins 0 (-92.5K), the 6 copies that return no tuple 42.3K, the 15 copies' twins
  118.2K, the slow calls 10.8K and their 2 conversions 4.2K, the runtime and generated
  C 332.6K (+12.7K).
- On the fixture of `tests/select_regions.rs` the compiled functions went from 216.8K to
  149.9K: generic lifts 83.1K (70.5K before), no lifts' twins,
  copies 13.9K, the copies' twins 34.9K, slow calls 11.2K and conversions 6.9K.

### 5.5 R3's measurements

The plan's estimate of R3 (about 80K, section 3) was taken against the hand-written `u1`,
in which a native bound built a `slice`. R2 as built already passes native bounds and
indexes as words: `s[a:b]` calls `pycc_ext_obj_getslice_int` and `s[i]` reads an exact
`str`'s character inline. A breakdown of main after R2's code size (2,543,791 instructions
per `loads()`) found what was left of item 6, per `loads()`:

- `pycc_ext_obj_getslice_int`, 62.7K inclusive, about 186 calls on a `str` from the copies
  (26.1K of it `pycc_ext_str_slice`, the slice itself) and 178 on tuples from generic lifts;
- `src.startswith(t, pos)`, about 92 calls, each a method call with its position boxed
  (`unicode_startswith` alone 10.1K);
- `skip_until`'s `src[pos:new_pos]`, whose `new_pos` (from `src.index`) is an `object`, 23
  calls through `pycc_ext_obj_getslice` and a `slice` object, 13.4K;
- `c = src[pos]` with a native `pos`: an object character, retained and released, its
  comparisons made on the object.

Measured as 5.4 (the bytecode cache written before the counts):

| build | instructions per `loads()` | `_parser` `.text` |
|---|---|---|
| main (R2's code size) | 2,543,791 | 760,807 B |
| R3 | 2,469,841 (-2.9%) | 770,951 B (+1.3%; +28.9% over main before R2) |

The target, 2.47M, is met by 159 instructions; two counts of the same build differ by
about 200.

- By symbol, the saving is about 35K from `startswith` (the method call, its site cache,
  the boxed position and its release), about 22K from the slices (`pycc_ext_obj_getslice_int`'s
  dispatch and `skip_until`'s `slice` object, less 2.4K of inline code in `parse_key_part`)
  and about 10K from characters loaded as words (`skip_comment`, `loads`, `parse_value`,
  `parse_basic_str`).
- pypng (the 8-function selection, built the same way by main and R3): `read()` of the
  256x256 image 143,010,463 and 143,010,461 instructions per call, the PngSuite decode
  123,529,259 and 123,527,458. pydantic (the 5-function selection): 240,819,502 and
  240,779,856 per repeat of the default workload. diff-match-patch (`bench/package_check.py
  --instructions`, both builds selecting `diff_bisect` alone): `diff_main` 101.55M per call
  either way.
- The word paths take any native `int` local, not only a region copy's position: a
  `for i in range(...)` variable of a compiled body also gets them (`s[i:i+2]`,
  `s.startswith("ab", i)`, `c = s[i]`). A leaf or a sum outside the word range (2**62)
  takes CPython's operation; `tests/region_words.rs` (`edges`) pins that edge.
- What is left in `_parser` beside R4's membership tests is not in the `str` word paths:
  positions boxed for the regular expressions' `match(src, pos)` and `src.index(expect,
  pos)` (about 150 per `loads()`), the tuple slices of the generic `Flags.is_` and
  `key_value_rule`, and the fixed cost of each `skip_chars` call.

### 5.6 R4's measurements

R4 was parked (5.1) after a prototype on `m0` saved 85K. Measured again on top of R3, the
same prototype (`src[pos] in chars` fused into one test: the byte read from an exact compact
ASCII `str` and tested against the set's cached mask, with no character object) saved
46,762 instructions per `loads()`, all in `skip_chars`, whose loop still ran about 37
instructions a test: the `str`'s type and state, the index, the set's type and the cache's
two ways were checked again at each of the 2,358 tests per `loads()`, since LLVM does not
unswitch a loop whose slow path calls out. The target was then set from that measurement:
at least 50K, tomli at most 2.42M, `.text` at most about +5%.

R4 as built adds what section 4.5 planned: the mask is looked up once where the loop
starts. A `while` loop whose test is `s[i] in c` or `s[i] not in c`, with `s` and `c`
parameters (always bound, never tagged) that the loop only reads and no `yield` in it,
reads both once before the loop: `s` an exact compact ASCII `str`, `c` an exact `frozenset`
whose cached mask is usable (`pycc_ext_ascii_mask_entry` computes it on a miss), and the
mask's two words copied. Anything else leaves a length of 0, which no index passes. Each
test is then the index's small-`int` check, one unsigned compare with the length (a
negative index, too, takes the per-test path), the byte and the bit: 14 instructions in
the compiled loop.
The copied words stay the set's answer whatever later happens to the cache, since a
`frozenset` cannot change and the loop does not rebind the name. A parameter rebound only
before the loop is prepared as it is bound at the loop; one a nested scope captures (and
could rebind through `nonlocal`) is a cell and is not prepared (`tests/char_class_loops.rs`,
`closure`). The `yield` guard is defensive: generators are not compiled today.

Measured as 5.4 (the bytecode cache written before the counts):

| build | instructions per `loads()` | `_parser` `.text` |
|---|---|---|
| main (R3) | 2,470,292 | 770,951 B |
| prototype, per test | 2,423,530 (-46,762) | 771,943 B |
| prepared, with a flag tested at each test | 2,405,210 (-65,082) | 774,631 B |
| **R4** (no flag: a length of 0) | **2,374,567 (-95,725, -3.9%)** | **774,151 B (+0.4%)** |

- By symbol, all of the saving is in `skip_chars`' copy: 227,058 -> 131,412 instructions per
  `loads()` (646 calls, 2,358 tests). A test is now 14 instructions in the compiled loop,
  and `pos += 1` 6 more. What is left is mostly the per-call cost, about 135 instructions
  a call: the copy's frame (entered and left), the two parameter references it releases,
  and the loop's preparation.
- tomli's `loads()` is now below the old whole-module build's 2.427M (section 1): 1.72x
  CPython's 4.082M in instructions, against the old 1.68x. Wall time, 100 `loads()`, median
  of 15 interleaved rounds: CPython 39.2 ms, main 26.5 ms (1.48x), R4 25.7 ms (1.53x), on a machine whose rounds varied by up to 1.6x.
- pypng (the 8-function selection): no compiled function has such a loop, and the compiled
  functions' machine code is identical to main's (the shim gains `pycc_ext_ascii_mask_entry`
  only). The PngSuite decode counts 123,548,861 and 123,560,813 per call. `read()` of the
  256x256 image varied between 121.7M and 161.7M per call across repeated counts of either
  build on this machine (main 139.6M, 134.5M, 161.7M, 144.0M; R4 113.9M, 126.2M, 151.2M,
  161.7M), with identical code, so no difference is attributable to R4. pydantic (the
  5-function selection): 240,902,069 and 240,877,776 per repeat. diff-match-patch
  (`bench/package_check.py --instructions`): `diff_main` 101.55M on main, 101.55M with R4.
- `tests/char_class_loops.rs` checks the loops in copies (a native position) and in their
  generic lifts (a `bool`, a big `int` or a `str` subclass sends a call there) against
  CPython, with the fast paths on and off.

### 5.7 R5's measurements

R4 left each call into and inside the region paying a fixed cost (5.6: about 135
instructions of `skip_chars`' 203 a call). Per `loads()` on R4's tree, the copies linked
and unlinked about 1,800 frames, each checking whether the frame on top was a signature
wrapper or a forwarder, and each unlink reading the frame's version-dependent fields
again; the 1,364 global reads (`__pycc_fallback_ns["name"]`) took and dropped a reference
to the namespace dict around a cache hit that runs no code; and `get_or_create_nest(key)`,
whose keyword-only `access_lists` is left to its default, went through the forwarder's
full binding (`pycc_fwd_bind`) 88 times.

- **Copies' frames.** Only compiled code calls a copy or its multi-value twin, with the
  caller's frame on top, so its frame is linked onto that top unchecked
  (`frames::called_from_compiled`). The link also writes the thread state and the top it
  replaced into two words of the frame's own, past the twelve CPython reads (null when
  `pycc_ext_frame_enter` linked it, the first call), and the unlink reads them there:
  no layout select at the return, and no values held through the body. Held as SSA values
  instead, they saved 5K on tomli but took registers in pypng's `undo_filter_paeth` loop
  (+7 instructions an iteration, the PngSuite decode +0.1%).
- **Global reads.** A namespace read is not pinned (`operand_pin::is_namespace_read`):
  its site's cache answers without running code, and on a miss
  `pycc_ext_ns_getitem_cached` holds the namespace itself for the lookup.
- **Defaults left out.** `pycc_fwd_defaulted` (`src/ext/module/forward_call.c`, split out
  of `forward.c`, which had reached 984 lines) calls the compiled body directly when a
  call passes the leading parameters in order and leaves every later one to its default,
  reading the defaults the function holds now (`__defaults__`, then `__kwdefaults__`);
  any other shape, or a parameter without a default, is bound as before. A slot keeps its
  parameter names' item array where the tuple layout holds it, and the in-order test is
  skipped for a slot with keyword-only parameters or a `__class__` cell, which it never
  passes.

Measured as 5.4 (the bytecode cache written before the counts):

| build | instructions per `loads()` | `_parser` `.text` |
|---|---|---|
| main (R4) | 2,374,136 | 774,151 B |
| copies' frames unchecked, global reads unpinned | 2,324,841 (-49,295) | 752,743 B |
| + defaults left out called directly | 2,306,893 (-17,948) | 753,639 B |
| **R5** (+ the unlink from the frame's own words, the names' item array) | **2,293,301 (-80,835, -3.4%)** | **753,527 B (-2.7%)** |

- What is left of the call path per `loads()`: the out-of-line part of each release of a
  mortal object (`pycc_ext_obj_release_count`, about 21.7K), the method-call sites'
  calls (`pycc_ext_site_call`, 11K), `Py_EnterRecursiveCall` (about 560 calls, 11K),
  `PyThreadState_Get` at each frame link (7.6K), and the `__kwdefaults__` lookup of the
  88 calls with defaults left out (6.5K).
- pypng (the 8-function selection): the PngSuite decode 123,437,800 to 123,465,610 per
  call (main 123,519,906 to 123,587,880). pydantic (the 5-function selection):
  240,259,929 per repeat (main 240,919,356). diff-match-patch
  (`bench/package_check.py --instructions`): `diff_main` 101.52M (main 101.55M). Wall time, 100 `loads()`, median of 15 interleaved
  rounds: CPython 39.6 ms, main 25.4 ms (1.56x), R5 24.1 ms (1.64x), on a machine whose rounds
  varied by up to 1.9x.
- `tests/forwarder_defaulted_calls.rs` checks the calls that leave defaults out against
  CPython with the caches on and off, while the defaults change under them, and that they
  reach the compiled body (300 direct calls, none on main). The copies' frames are
  checked by `tests/select_regions.rs` (tracebacks through copies, the `RecursionError`
  of a deep cycle) and `tests/v08_frames.rs`.

## 6. Risks

- **The margin is thin.** The projection beats the old build by about 2%, and part of it is an
  estimate (R4's 170K, from the IR). If R2 measures above 2.66M, or R4 saves less than 120K,
  accept 2 is not reachable by this plan, and the next step is a measurement, not more tuning.
  As built, R2 and R3 came in under their targets and R4 saved 96K, not 120K, but from a lower
  start: tomli's `loads()` is at 2.293M after R5, under the old build's 2.427M (5.7). The CI ratios
  (R6) are still the verdict.
- **Code size.** Each member is compiled twice, as its lift and as its copy (or the copy's
  multi-value twin): +27% `.text` for tomli's `_parser` after R2's code-size follow-ups,
  +80% as R2 first built it (5.4), against the +16% section 3 estimated. That costs I1
  misses, which section 2.9 found to dominate generic bodies. Wall time stays the verdict
  in pass 3.
- **Over-wide regions.** A region joins everything connected by calls, so in pydantic's
  glue a large component could pull in cold code. Pass 3 prunes whole regions, so such a
  region is kept only if it wins as a whole. Copies are made only for members with a native
  parameter or result.
- **Rebinding during a call.** The `TypeError` path (4.4) is a deviation. Tests must show it
  is reached only by a rebinding made while a region call is running.
- **Facts B1-B3 widen what the prover trusts.** Each holds only while the intact flag is
  set: a missed watcher event (a member's dict key written through a path the watcher does
  not see) would run a copy on a value it did not prove. The tests mutate every key and
  function attribute the facts depend on before a call and check CPython's result.
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
