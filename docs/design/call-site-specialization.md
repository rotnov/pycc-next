# Call-site specialized code: what is delivered, and what it measured

The parts of [compile-little.md](compile-little.md) section 2.5 delivered so far, in the
order they landed, each with the measurement that justified it. Section 2.5 states the
rule; this document is the record of how the shim and the code generator follow it.

**PR 8, first part: bodies specialized for observed types.** A selected function gets
the parameter types pass 2 observed. Only parameters with one exact type in at least 95%
of the sampled calls count (`profile::DOMINANT_PERCENT`; until v0.12.0, in every sampled
call), and only for `int`, `float`, `str` and `bytearray` (`Entry::observed_types`).
Unanimity cost pypng its filter loops: its test suite passes `undo_filter_*` an
`array('B')` 1-2 times in 1,000 sampled calls, so no buffer was typed and the five-filter
read ran at 1.74x CPython's instructions (1.59x wall); with the dominant `bytearray`
typed, 17.85x (10.1x wall), and PngSuite 1.23x to 3.55x (1.11x to 2.61x wall), both from
the same profile. Those types travel with the selection:
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

**PR 7, first part: direct calls between generic bodies ([compile-little.md](compile-little.md) section 2.8).** A call from a compiled body to another
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
out-pointers and calls the twin on the direct path ([compile-little.md](compile-little.md) section 2.8). A stored first
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

A direct call ([compile-little.md](compile-little.md) section 2.8) on CPython 3.15 asks `Py_EnterRecursiveCall` once per
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
- **A parameter is specialized only where a loop computes with its native value.** A
  native parameter costs a conversion at every call (the guard's `pycc_ext_obj_unbox_str`
  of a `str` is about 150 instructions), and the generic body it replaces already takes
  an exact `int` or `str` operand's common case inline (the fast paths and tagged locals
  above). It saves only where an operation repeated by a loop has nothing but native
  operands: `text1[x1] == text2[y1]` with `x1` and `y1` native `int`s. An operation
  whose other operand is an object converts on every iteration instead, which the
  generic body never does. So once an attempt with the observed types type-checks, the
  driver lowers it to MIR and looks, in each copy, for an operation inside a loop (a
  `while` test or body, a `for` body, a comprehension) that reads the parameter natively
  with only native operands beside it: arithmetic, a comparison, an index or slice, a
  builtin's or a `str` method's argument, a comprehension's source, or the sequence a
  `for` walks (`pycc_mir::natively_used`). A native local computed from it carries the
  use back: pypng's `undo_filter_paeth` reads `filter_unit` once, in `ai =
  -filter_unit`, and its loop then indexes with the native `ai`. A read that is boxed,
  passed on, stored or meets an object operand (including one converted for the use,
  `pycc_ext_obj_coerce` of an object index) does not count, nor does a read outside a
  loop that feeds no such local. A parameter with
  no such read is left an object and the module is checked again
  (`crate::specialize::with_specialization`); a copy with no parameter left is not
  made. `--report-fallbacks` says `not specialized: ... meets no native operand in a
  loop`. A `bytearray`'s native form is its object, so it is always kept.

  tomli is the case that found it. Its scanners take `src: str` and rebind `pos`, so
  `pos` stays an object and every use of `src` meets it: in `skip_chars`, `while
  src[pos] in chars` converted `pos` to a native index (`pycc_ext_obj_coerce`, through
  `PyNumber_AsSsize_t`, about 190 instructions), made a native one-character string,
  packed it into a new CPython `str` and searched the object `chars` with it (hashing
  the new string) at every step, where the generic body reads the cached character and
  one bit of the frozenset's ASCII mask. Specialized, `skip_chars` ran 1,325 more
  instructions per call. Measured over pass 3's training workload (tomli's test suite
  and 100 `loads()` of its benchmark document, callgrind, CPython 3.15.0b4), with
  tomli's 36 candidates compiled and one function at a time specialized as the profile
  observed:

  | specialized | instructions | against none specialized |
  |---|---|---|
  | none | 860.8M | |
  | `skip_chars` (`src: str`) | 976.7M | +115.9M |
  | `parse_key` (`src`) | 874.7M | +13.9M |
  | `parse_basic_str` (`src`) | 869.1M | +8.3M |
  | `key_value_rule` (`src`) | 865.5M | +4.7M |
  | `Flags.is_` (`flag: int`) | 865.2M | +4.4M |
  | `parse_array` (`src`, `nest_lvl: int`) | 864.9M | +4.1M |
  | `skip_comments_and_array_ws` (`src`) | 864.7M | +3.9M |
  | `parse_inline_table` (`src`, `nest_lvl`) | 863.2M | +2.3M |
  | `skip_until` (`src`, `expect: str`) | 862.3M | +1.5M |
  | `Flags.set` (`flag`) | 861.2M | +0.4M |
  | all ten | 1,025.6M | +164.8M |

  CPython ran the same workload in 1,047.4M (in pass 3's own count, 1,022.5M against
  1,039.4M with the 36 specialized and 1,037.9M with the 19 hot ones), so pass 3 kept
  none of tomli's candidates. With the rule, none of the ten is specialized (each
  parameter meets only `pos` or an object set), the 36 run 859.4M, and pass 3 keeps
  the 19 hot ones at -14.5% of the workload's instructions (874.0M against 1,022.5M).
  `diff_bisect` keeps `text1: str, text2: str`, whose loops index them with native
  `int`s; its `deadline: int`, compared only with `time.time()`'s object, is no longer
  specialized: `diff_main` of the speed test's first 2,000 characters ran 260.9M
  instructions before and 260.1M after (CPython 1,029.7M). pypng's
  `undo_filter_average` and `undo_filter_paeth` keep `filter_unit: int` (through `ai`);
  `undo_filter_sub` (which passes it to `range()` once) and `undo_filter_up` (which
  never reads it) no longer convert it: pass 3's count for pypng's 30 candidates went
  from 4,219.3M to 4,218.3M (CPython 5,077.6M), and it keeps the same 7 hot functions.
  Without the local's use carried back, the four lost their specialization and the 30
  ran 4,239.4M.

`tests/select_observed_types.rs` checks both paths against CPython: the observed `str`s;
a list, a tuple, an `int` or `None`; `str` subclasses that change indexing or `len`; a
NaN, a large `int` and a `str` that leave unchanged; `hash` of a NaN, a reflected
`__radd__`, and a `len` bound by a parameter, a `case` capture or an `except ... as`; `+x`,
`s.strip()`, `s[:]`, `s + ''` and `s.split()` against CPython's identities; a rebound
parameter; and parameters left objects because their loops meet only objects (tomli's
`skip_chars`, and a `float` multiplied with a list's items).

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
pydantic phases ([compile-little.md](compile-little.md) section 1's whole-module numbers and PR 4's candidates). Per top-level
call it can cost much more. The table below replays the top-level calls each function
received during define and schema (5 repetitions), on fresh deep copies, giving ns per
call (medians of 5 processes of 15 passes each):

| function | calls | CPython | generic lift | typed (observed) | ideal C |
|---|---|---|---|---|---|
| `_make_json_hashable` | 2,580 | 5,961 | 5,499 (1.08x) | n/a: no single-type parameter | 1,601 (3.7x) |
| `_DefinitionsRemapping.remap_json_schema` | 120 | 25,834 | 29,054 (0.89x) | n/a: no single-type parameter | not written |
| `GenerateJsonSchema._sort_recursive` | 170 | 6,661 | 10,306 (0.65x) | 12,864 (0.52x; `parent_key: str`) | not written |

The ideal C is `bench/headroom/hrfast.c` ([compile-little.md](compile-little.md) section 1). Typing the one `str` parameter of
`_sort_recursive` makes it slower: its recursive calls still go through the object
protocol, and the guard adds a test per call. Pass 3 rejects such a set, since it loses.
The fix is either of:
- compile a body only when it is typed where it matters;
- prove per function that the generic lift pays, as pass 3 already does per set.
