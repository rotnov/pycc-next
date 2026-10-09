# pycc Diagnostics Registry

Every code: stable forever, documented via `pycc explain`, covered by at least one test in `tests/diagnostics/`. Format contract in [CLI_SPEC.md](./CLI_SPEC.md).

## Code ranges

| Range | Domain |
|---|---|
| `C0xxx` | compiler-version capability gaps for valid Python |
| `L0xxx` | lexing / parsing / grammar |
| `T0xxx` | type checking |
| `O0xxx` | ownership / memory / threading |
| `E01xx` | unsupported dynamic Python (rejected by design) |
| `I04xx` | CPython interop boundary |
| `W1xxx` | warnings (deprecations, perf hints) |

## Initial registry

| Code | Severity | Message (short form) |
|---|---|---|
| `C0001` | error | valid Python construct is not implemented by this pycc version -- e.g. `hash()` of a type CPython hashes but this version does not yet, "`hash()` of `<ty>` is valid Python but not implemented yet" (#1331), including a user-class instance whose hash pycc does not compile yet, with a help line naming the reason (#1335); and "cannot infer an unannotated private helper's `set[C]` return yet" for an unannotated helper returning a set comprehension whose instance element the constraint solver cannot type yet (#1344, with help naming #1360; annotate the return; a class-constructor element is typed since #1342) |
| `C0002` | error | recognized stdlib module, but the specific imported symbol is not registered (D-136), e.g. `from math import isnan` |
| `C0003` | error | `--ext` cannot carry the signature of a public function, of a public `@staticmethod`/`@classmethod` of a public class, or of a public instance method of a public class that some *published, constructible* class reaches through its method resolution order and whose method name that class resolves to the declaring class rather than to a binding of its own, across the CPython boundary (D-244): `docs/RUNTIME.md`'s `ext` boundary section is the canonical admissibility matrix for what crosses and in which direction, and for which members are in the export set at all, so one unexportable signature fails the whole `--ext` build -- rename the member with a leading underscore (D-038) to drop it from the export set, or build without `--ext`; an entry module's PEP 562 `__getattr__`/`__dir__` hook is also in the export set (#1467), and its fix is to change its signature, not to rename it; so is a compiled class's `__eq__`/`__ne__`/`__lt__`/`__le__`/`__gt__`/`__ge__`/`__hash__`, installed as its carrier type's comparison and hash slots on every class of the module (#1427), whose fix is to make it an instance method the boundary can carry, or, on a PEP 695 generic class, to use an erased `Generic[T]` base instead; since [#1470](https://github.com/rotnov/pycc/issues/1470) an embedded executable installs the same slots and refuses the same bindings with this code, worded "an embedded executable cannot install ...", and also a slot method whose signature needs an export thunk -- one carrying a `tuple` or returning a slice of a `memoryview` argument (it is compiled without export thunks); the refusal covers every class of the program, so an embedded program that built before #1470 with such a class is refused now even if the class never reaches CPython |
| `L0001` | error | syntax error (span + expected set) *(also reused, without an "expected set", for a post-parse context violation caught during HIR lowering -- `break`/`continue` outside a loop, `async for` outside an async function (D-148), `yield`/`yield from` outside a function (D-149), `return`/`break`/`continue` inside a `finally` block that would exit it (PEP 765, D-193, Part 1 of #543), `return`/`break`/`continue` inside an `except*` clause body (PEP 654, #795) -- each of these also when hidden behind an `if`/`elif TYPE_CHECKING:` guard, whose folded-away body is re-walked for context violations only (#905, `crates/pycc_hir/src/stmt/type_checking.rs`) -- and a `from __future__ import ...` CPython rejects -- an unknown feature name including `*` (`future feature <name> is not defined`), `braces` (`not a chance`), or a future import after the docstring-and-future-imports prologue (`from __future__ imports must occur at the beginning of the file`) (#919, D-229) -- since CPython classifies all of these as `SyntaxError` (a `SyntaxWarning` as of CPython 3.14 for the `finally` case specifically) too; `T0024` ("`return` outside a function") is the closest existing precedent for this same "keyword valid only in a specific context" family but lives under `T0xxx` instead, an accepted, deliberately unfixed inconsistency -- see D-148)* |
| `L0002` | error | Python version mismatch (feature needs 3.14 level) |
| `T0001` | error | public function missing annotation |
| `T0002` | error | `Any` outside interop boundary; a module compiled into a `pycc build --ext` artifact is one, where `Any` is the opaque CPython object instead (D-258, #1397) |
| `T0003` | error | untyped empty container needs annotation |
| `T0021` | error | name resolution (including an unbound local), operand, call, or inference type mismatch; also the project-import failures CPython itself rejects (D-222): a name the imported module does not define, a relative import with no parent package or climbing above the top-level package, and a relative target that resolves to no module (none of these for the entry module's relative from-imports under `pycc build --ext --foreign-relative-imports`, #1366, which CPython resolves when the artifact is imported); also `hash()` of a `list`, `dict` or `set`, reported as "unhashable type: `<ty>`", the `TypeError` CPython raises (#1331); also `hash()` of an instance whose class is unhashable (it defines `__eq__` without `__hash__`), or whose `__hash__` does not return an integer, "`__hash__` method should return an integer" (#1335) |
| `T0022` | error | return type mismatch — a written return annotation the body contradicts reads *return type mismatch: expected `int`, found `P`* and carries a `help` suggestion (D-152) -- for a CPython object (a foreign-import value) returned from a `-> bool`/`int`/`float`/`str` function, the explicit conversion such as `bool(...)` (D-258's [#1419](https://github.com/rotnov/pycc/issues/1419) amendment) -- while an *unannotated* private helper whose inferred return type gets pinned two incompatible ways reads *private helper return type: conflicting inferred types `int` and `str`* with no `help` (neither side is canonical). The discriminator is the return annotation, not the function's visibility — the solver walks every module-level function, not only `_`-prefixed helpers (#949). A function with a non-`None` return type whose body can reach its end reads *function `f` can exit without returning `int`*. A `while True:` or `while <non-zero int literal>:` loop never reaches the statement after it, so a body ending in one, left only through `return` or a raise, does not reach its end; any other loop test is conservatively assumed to fall through: `while False:`, `while 0:` and a computed test because the test may be false on entry or later, and a string or float literal test because pycc does not treat it as constant (#1370). The rule relies on `break` being rejected with `C0001` today. In an `--ext` module a comparison method whose body returns `NotImplemented` returns an `object` (#1418), so a container `return` in it reads *expected return type `object`, got `list[int]`* with a help naming that widening, while an `int`, `float`, `bool`, `str`, `None` or instance return is boxed into the object (#1475) |
| `T0023` | error | incompatible assignment |
| `T0024` | error | `return` outside a function |
| `T0025` | error | annotated-assignment initializer incompatible with its declared annotation. A CPython object (a foreign-import value) initializing a `bool`, `int`, `float` or `str` annotation (`b: bool = o == 1`) is refused, not converted, and its `help` names the explicit conversion, as for `T0022` (D-258's [#1419](https://github.com/rotnov/pycc/issues/1419) amendment) |
| `T0026` | error | a later plain or annotated assignment is incompatible with an earlier value-less declaration (`x: int` with no `= ...`) |
| `T0030` | error | non-exhaustive `match` (missing cases listed) |
| `T0031` | error | `@override` without matching base method |
| `T0032` | error | list literal elements must all share one type |
| `T0033` | error | value does not support list/dict/set/frozenset operations (subscript, slicing, item assignment, `for`, `.append()`, `.pop()`, `.get()`, `.add()`, `len()`), or `len()` called with the wrong number of arguments |
| `T0034` | error | list codegen only supports `list[int]` in v0.2 (D-105) — gates both an inferred list-literal element type and a written `list[T]` annotation (D-228) |
| `T0035` | error | dict literal key/value pairs must all share one key type and one value type |
| `T0036` | error | dict codegen only supports `dict[str, int]` in v0.2 (D-122) — gates both an inferred dict-literal key/value type and a written `dict[K, V]` annotation (D-228) |
| `T0037` | error | set literal elements must all share one type |
| `T0038` | error | set element type not compiled yet: a `set[T]` element must be `int` or an instance of a user class (D-122, widened by D-255 in #1343) — gates both an inferred set-literal element type and a written `set[T]` annotation (D-228), and a written `frozenset[T]` annotation (Part 1 of #1319); whether a user class is hashable is decided at the insertion site (`T0054`, or `C0001` for an uncompiled class shape) |
| `T0039` | error | tuple element type not compiled yet (int/bool/float only, D-116) — gates both an inferred tuple-literal element type and each element of a written `tuple[A, B, ...]` annotation (D-228) |
| `T0040` | error | tuple index must be a non-negative literal integer within range (D-116) |
| `T0041` | error | local name may not be bound on every path reaching this use (definite-assignment tracking, issue #118 Part 1, D-147) |
| `T0042` | error | generic-function shape or call-site instantiation rejected beyond the PR-13 v0.2 thin slice (D-133/D-134): more than one PEP 695 type parameter; a type parameter used in a container position (`list[T]`); a call site whose occurrences of the type parameter resolve to different concrete types; a call site whose argument for the type parameter is not `int`/`float`/`bool`/`str`; a generic function's body calling itself or any other generic function (recursive generic instantiation is outside D-134's single-call-site monomorphization slice); also reused (D-135) for a generic `type` alias declaring its own type parameters |
| `T0043` | error | attribute access, attribute assignment, or method call on a value that is not a class instance (D-154, Part 1 of #375) |
| `T0044` | error | class has no attribute or method with the given name (D-154, Part 1 of #375); also a store to an attribute that cannot be written: a class-level attribute (#911), a property without a setter (#377), or an enum member's `value`/`name` ([#1219](https://github.com/rotnov/pycc/issues/1219)); also a store to an attribute no slot declares when every class in the MRO binds `__slots__` ([#1368](https://github.com/rotnov/pycc/issues/1368)); also a `staticmethod(<foreign import>)` class attribute ([#1345](https://github.com/rotnov/pycc/issues/1345), D-256) that is written, or read or called through a `super()` receiver ([#1358](https://github.com/rotnov/pycc/issues/1358)), through a use site that shadows its root, through an instance read that reaches a method shadowing it, through an instance whose subclass resolves the name differently, or through an instance whose slot declines it while an `@property` of the name sits later in the MRO; a mismatched instantiation/method call-site argument list reuses `T0021` instead, exactly like an ordinary function call; also reported (PEP 560, #610/#611) when `ClassName[...]` is used **in value position** on a class that defines no `__class_getitem__` anywhere in its MRO and is not a PEP 695 generic class, matching CPython's `TypeError: type 'C' is not subscriptable` — annotation position is no longer gated on subscriptability, because PEP 649/749 evaluates annotations lazily ([#1130](https://github.com/rotnov/pycc/issues/1130)); and ([#931](https://github.com/rotnov/pycc/issues/931)) when the base of a subscripted annotation resolves to something that is not a class at all — a PEP 695 type parameter (`T[int]`), a builtin scalar (`int[str]`), `Self` inside a class, or a `type` alias to a non-class type other than the buffer carrier (`type Arr = memoryview` then `Arr[float]` resolves to the carrier and is accepted, #1130) — matching the same CPython `TypeError` instead of silently discarding the type argument; an alias to a class behaves as that class, an undefined base keeps `C0001`, and `Any[...]` keeps `T0002` in a `native` program; in a `pycc build --ext` module, where `Any` and `object` are the opaque CPython object (D-258, #1397), `Any[...]` and `object[...]` are `T0044`, since neither is generic |
| `T0045` | error | cannot reassign a `Final` name (PEP 591, #383) — variable-level annotations only (module-level and function-local); `Final` on parameters is out of scope, and a class-body `Final` attribute (#916) is accepted but rejected on write by `T0044` instead |
| `T0046` | error | class does not conform to a protocol (PEP 544 structural typing, #380, D-166) — a concrete class is missing one or more of a protocol's required methods or attributes, or a method/attribute has an incompatible signature/type |
| `T0047` | error | `super()` does not proxy instance attributes (#587) — an attribute established by `self.<attr> = ...` is reachable only through the method's own receiver, matching CPython's `super` object, which proxies class-level attributes and descriptors along the MRO but not the instance's own attributes |
| `T0048` | error | general union type annotation is not supported yet (PEP 604, D-197, #763, Part 1 of #747) — a `X \| Y` annotation is accepted only in the exact `T \| None`/`None \| T` shape (a 2-operand union where one side is literally `None`); any other shape, including a longer `A \| B \| None` chain or a 2-operand union where neither side is `None`, is rejected |
| `T0049` | error | `Optional[T]` annotation is not supported yet for this `T` (PEP 604, D-197, #763, Part 1 of #747; widened to `float`/`bool` by #809, Part 3 of #747) — `T \| None` type-checks only for `T` in `{int, float, bool}`; every other inner type (`str`, `list[...]`, a class instance, or another `Optional[...]`) is a recognized but out-of-scope shape, distinct from `T0048`'s rejection of the general-union shape itself |
| `T0050` | error | walrus assignment (`:=`) value type is not supported yet (PEP 572, #774) — a walrus value is currently restricted to the non-reference-counted scalar types `int`, `float`, `bool`, `None`, and `Optional` of those, since codegen's `MirExpr::NamedExpr` value-yielding read is only worked out for those types' refcount contract |
| `T0051` | error | an `int` literal in one of the 13 named runtime `int`-boundary positions D-179 left in D-141's inventory (a `range()` argument is bigint-capable and excluded, D-179) is out of range for D-061's 63-bit tagged smallint representation (issue #618, restoring `pycc check` as the catch point D-178/#148 knowingly moved to run time for the literal case) — a bigint reaching the same position through arithmetic is unaffected |
| `T0052` | error | cross-MRO attribute redeclaration with a differing declared type (issue #676, D-210) — two distinct classes in one class's own C3-linearized MRO each declare an instance attribute of the same name with a different type; rejected at class-definition time since pycc has no per-instance runtime type tag to safely coerce at the shared, statically-dispatched assignment site |
| `T0053` | error | a parameterized container type annotation (`list[T]`, `set[T]`, `dict[K, V]`, `tuple[A, B, ...]`) was written with the wrong number of type arguments (issue #918, D-228) — `list`/`set` take exactly one, `dict` exactly two, and `tuple` at least one; `tuple[()]` and any `...` type argument (the homogeneous-variadic `tuple[int, ...]`, and the ill-typed `list[...]`/`dict[str, ...]` spellings) are not supported yet and are rejected here |
| `T0054` | error | a set literal, `.add(...)` or a set comprehension (#1344) inserts an instance of a user class that defines `__eq__` without `__hash__` (#1343, D-255) — CPython sets `__hash__ = None` on such a class and raises `TypeError: unhashable type` at run time; pycc reports it statically, with help naming the class that binds `__eq__` |
| `T0055` | error | a tuple-unpacking assignment (`a, b = t`, Part 1 of #891) unpacks a native `tuple[...]` whose length differs from the number of target names — CPython raises `ValueError` at run time with the same message (`too many values to unpack (expected N, got M)` / `not enough values to unpack (expected N, got M)`); pycc knows the length statically and reports it before the program runs, as it does an out-of-range literal tuple index (`T0040`, D-116). A CPython object in an `--ext` build is unpacked and checked at run time instead |
| `O0201` | error | value used after move across scope boundary *(internal-only: never fires on legal Python — see note)* |
| `O0301` | error | non-Shareable value crosses thread boundary without move |
| `O0302` | error | lock-guarded field accessed without holding lock |
| `E0100` | error | `eval`/`exec`/`compile` not compilable |
| `E0101` | error | monkey-patching foreign class/module |
| `E0102` | error | dynamic attribute injection |
| `E0103` | error | dynamic `type()` class creation |
| `E0104` | error | wildcard import *(reserved; not currently emitted — a wildcard `from x import *` is rejected today by the same versioned `C0001` "not implemented yet" capability code every other unsupported import shape uses, not this by-design-rejection code; see D-137)* |
| `E0105` | error | metaclass with non-static side effects |
| `E0106` | warning→error | `__del__` relies on refcount timing |
| `E0107` | error | `sys.getrefcount` unavailable |
| `E0108` | error | import cycle in top-level init, reported as the chain of files that closes it (`import cycle: `a.py` -> `b.py` -> `a.py``), emitted by the driver's project-module loader since #898/D-222 |
| `I0401` | error | untyped value leaks across interop boundary |
| `I0402` | error | CPython-backed direct import root rejected by the effective `allowlist` or `deny` interop policy (`--pure`, `--interop-policy`, or `[interop]` in `pycc.toml`), reported by `check`, `build` and `run` on every host since #1224 |
| `I0403` | error | CPython import the effective interop policy admits but this build cannot embed (an excluded Tcl/Tk root or a `--target` build; a root outside the standard library embeds from `pycc.lock` on macOS and Linux since #1242 and on a Windows host since #1296, and a Windows host embeds standard-library roots since #1286); `pycc build --ext` is required |
| `I0404` | error | operation on a CPython object is not supported yet |
| `I0405` | error | a buffer-typed signature in native mode, under any source spelling (`memoryview`, `ndarray` or `NDArray`), or a function body that allocates artifact-owned buffer storage with `ndarray(n)`/`NDArray(n)` in native mode (#1165); `pycc build --ext` is required |
| `W1001` | warning | unreachable code |
| `W1002` | warning | boxed fallback in hot loop (`--memstats` hint) |

Note on inherited-method copies ([#1337](https://github.com/rotnov/pycc/issues/1337), [D-254](./decisions/D-254-inherited-methods-are-compiled-per-receiver-class.md)): an inherited body is compiled once more for a subclass whose behaviour for it differs. A diagnostic raised only in that copy keeps its own code (typically `T0021`/`T0022`/`T0025` when the receiver escapes into a base-typed slot, or `C0001`/`T0021` for a dataclass capability the subclass lacks), is attributed to the origin body (and so to the file that defines it; the span is the one the check raised it with), and carries the help note ``while compiling `D.m` inherited by subclass `C` ``, which `--error-format json` shows. A diagnostic the origin body already raises at the same place is reported once. Pinned by `tests/issue_1337_receiver_dispatch.rs`.

Note on exception-class dunders ([#1337](https://github.com/rotnov/pycc/issues/1337)): a user exception class whose MRO resolves an instance-protocol dunder (`__str__`, `__bool__`, `__eq__`, ...) to a user class rather than to a builtin exception base is `C0001` at the class definition, naming the dunder and the class defining it and citing Part 3 of #541, because a raised exception value would silently ignore it. `TYPE_SYSTEM.md`'s Part 2 of #541 paragraph owns the rule. Pinned by `crates/pycc_hir/src/class/exception_dunders_tests.rs` and `tests/issue_1337_receiver_dispatch.rs`.

Note on `O0201`: legal Python never observes moves (see MEMORY_OWNERSHIP.md — optimization is semantics-preserving). The code exists for internal assertions and `--emit mir` tooling; if it ever fires on user code, that's a pycc bug, auto-reported.

Adding a new code here requires a matching `EXPLANATIONS` entry in
[`crates/pycc_diag/src/explain.rs`](../crates/pycc_diag/src/explain.rs)
(D-150) — that file's own test suite enforces the two stay in sync, so a
registry row added here without a matching entry fails CI immediately.

`C0001` is a versioned capability diagnostic, not a rejected-by-design
language rule. A construct stops producing it when the corresponding roadmap
slice is implemented; the code remains reserved so older compiler output keeps
an unambiguous meaning. HIR lowering uses it for valid Python outside the
currently implemented frontend subset, with the unsupported AST node as the
primary span and a message that names the rejected construct in Python terms
(`a tuple`, ``an `import` inside a function or block body`` in a function,
``an `import` inside a block body`` in a module-level block, `a call whose
callee is a call expression`); a message never renders an AST node's Rust
`Debug` form, and `tests/diagnostics_test.rs` scans every `.expected.txt`
fixture for the `NodeIndex(` marker such a dump would carry (issue #890).
It also covers the project-import shapes #898/D-222 recognizes but does not
implement yet: a bare `import <project module>` and a `from pkg import
<submodule>` (both bind a module namespace -- ``module namespace bindings
(`import geometry`) are not supported yet``), a PEP 420 namespace package as
an import's *terminal* segment (``namespace package `nspkg` (a directory
without `__init__.py`) is not supported yet``; an intermediate namespace
segment is fine), and a top-level name two linked modules both define
(``top-level name `helper` is already defined by `colA.py`; a separate
namespace per module is not supported yet``, lifted by Part 3 of #881). A
program that seeds the builtin exception classes in one module while another
shadows one of their names is rejected the same way. A protocol class in
return-annotation position (``a protocol class (`P`) as a return type
annotation is not supported yet -- a protocol type is currently supported in
parameter and variable positions only``,
[#934](https://github.com/rotnov/pycc/issues/934)) is rejected at the
annotation's own span, on a module-level function, a method, and a protocol
member declaration alike; the check runs after the annotation lowers, so a
nested `list[P]` or `P | None` still reports its own element-type gate first.
Both spellings of a protocol member returning its *own* protocol reach that
same gate with that same message ([#948](https://github.com/rotnov/pycc/issues/948)):
the protocol's own name (`def clone(self) -> P: ...`, PEP 649/749) and `Self`
(PEP 673). A self-referential parameter or attribute is not rejected -- a
protocol type is supported in those positions.
`pycc_types` also emits `C0001`, at that crate's conventional `1:1` span (it
carries no expression spans), for a capability gap that depends on a value's
*type* and so cannot be seen by HIR lowering: string conversion of a class
instance that is neither a `@dataclass` instance nor a caught builtin
exception, or of a protocol-typed value, as a `print()` argument or an
f-string interpolation ([#977](https://github.com/rotnov/pycc/issues/977),
[D-237](./decisions/D-237-reject-string-conversion-of-a-non-dataclass-non-exception.md)).
The two message shapes are ``string conversion of a `C` instance as a `print()`
argument is not supported yet; `print()` and f-string interpolation can render
only a `@dataclass` instance or a caught builtin exception`` (with a help line
naming the `@dataclass` alternative, and `an f-string interpolation` at the
other site) and ``string conversion of a value typed as protocol `P` as an
f-string interpolation is not supported yet; the concrete class is not known
at the conversion site``. A user class declared under any builtin
exception name is rejected there whatever its shape, because the MIR
rewrites resolve those names before the shape.
An import failure
CPython itself would raise on is `T0021`, not `C0001`. A `from __future__
import ...` is a compiler directive, not a module (#919, D-229): its nine no-op
features lower to nothing, a name CPython rejects is `L0001`, and the one
CPython-valid feature that changes the grammar, `barry_as_FLUFL`, is `C0001`
(``the `barry_as_FLUFL` future feature (`<>` in place of `!=`) is not
supported yet``), as is `from __future__ import x as y` under the
`from ... import x as y` aliasing gap ([#963](https://github.com/rotnov/pycc/issues/963)).
A stdlib module behind `import X as Y` is *not* a gap since
[#962](https://github.com/rotnov/pycc/issues/962) ([D-231](./decisions/D-231-lower-stdlib-import-x-as-y-to-canonical-names-with-an.md)):
`import math as m` binds `m`, `m.sqrt(x)` lowers to the same canonical
`math.sqrt` the unaliased program produces, and a symbol the registry lacks
reports both spellings (``module `math` (imported as `m`) has no importable
symbol named `tan` ``). The alias makes the receiver shadow check alias-aware
too: a local or parameter named `m` around `m.sqrt(m)` is rejected with the
same ``is a local name here, not the stdlib `math` module`` C0001 a local
named `math` gets -- and, because the HIR keeps only the canonical spelling,
a local named `math` while the module only ever writes `m.sqrt` is rejected
as well (a recorded fail-closed residual, closed by
[#768](https://github.com/rotnov/pycc/issues/768)). An alias on a project
module keeps the plain-`import` C0001 (``import of module `geometry` is not
supported yet``, [#964](https://github.com/rotnov/pycc/issues/964)), as does
a dotted name under a project module or package (`import geometry.sub`). Since [#1291](https://github.com/rotnov/pycc/issues/1291) an
alias on an undotted module that is neither a project module nor a
`pycc_std` registration (`import numpy as np`) is a foreign import instead,
binding the CPython module object. Since Part 3 of
[#1138](https://github.com/rotnov/pycc/issues/1138)
([#1381](https://github.com/rotnov/pycc/issues/1381)) a dotted module whose
root is neither a project module nor a project package is one too, plain or
aliased: `import xml.dom` binds the root `xml`, and `import xml.dom as d`
binds the leaf `xml.dom` to `d`. Two spellings of it keep a C0001. A leaf
bound under its own root's name (`import xml.dom as xml`) is refused with
``binding the CPython module `xml.dom` to `xml`, the name of its own
top-level package, is not supported yet``, because pycc tells the root
binding from the leaf binding by whether the bound name is the root. An
unaliased dotted import whose root is a name pycc resolves by its spelling
(`import typing.sub`, which would bind `typing`) is refused with the
spelling C0001 below, naming the root as the bound name. Two dotted imports
that bind the same module to one name (`import os` and `import os.path`, or
`import xml.dom` and `import xml.sax`) are the same binding, not a shadow;
two that bind different modules to one name (`import xml.dom as x` and
`import xml.sax as x`) keep the shadowing C0001. An alias pycc resolves by its spelling
-- any Python builtin (`range`, `super`, `property`, `ValueError`, ...), a
`pycc_std` module name such as `typing`, or a marker pycc recognises without
an import (`TYPE_CHECKING`; the base-class markers `Enum`, `StrEnum`,
`Protocol`, `ABC`; `auto`, `override`, `abstractmethod`, `dataclass`,
`runtime_checkable`, `dataclass_transform`, `field`; and the annotation names
`ClassVar`, `Final`, `Self`, `Annotated`, `Any`, `TypeAlias`, `ndarray`,
`NDArray`, and since [#1378](https://github.com/rotnov/pycc/issues/1378) the
legacy `typing` container aliases `Dict`, `List`, `Set`, `FrozenSet`,
`Tuple`) -- is refused with its own C0001 (``binding the CPython module
`foo` to `range`, a name pycc resolves by its spelling (a Python builtin, a
stdlib module, or a typing, decorator or base-class marker), is not supported
yet``), because the alias would otherwise be read as that other meaning. The
canonical list is `PYTHON_BUILTINS` and `SPELLING_MARKERS` in
`crates/pycc_hir/src/import/spelling.rs`. A foreign import nested in a
module-level block that names a class the module defines, or a seeded builtin
exception class the module mentions, is refused with the same ``import `X`
collides with a class of the same name already defined in this module`` C0001
as the top-level form, at the nested statement.

Since [#1278](https://github.com/rotnov/pycc/issues/1278) an unaliased,
top-level `from X import a, b` of an undotted module that is neither a
project module nor a `pycc_std` registration is a foreign import too: each
listed name binds the CPython object `X.<name>`. Since Part 1 of
[#1138](https://github.com/rotnov/pycc/issues/1138) `X` may be dotted
(`from xml.dom import minidom`) when its root is neither a project module
nor a project package; a dotted `X` under a project root keeps the
``import of module `X` is not supported yet`` C0001. A listed name pycc resolves
by its spelling is refused with the from form's own C0001 (``binding the
CPython object `builtins.range` to `range`, a name pycc resolves by its
spelling (...), is not supported yet``), against the same canonical lists --
so `from os import List` is refused like `from os import range`. The two
buffer-carrier pairs are the exception
([#1380](https://github.com/rotnov/pycc/issues/1380), D-244): an absolute,
unaliased `from numpy import ndarray` or `from numpy.typing import NDArray`
is admitted and keeps the spelling's buffer-annotation meaning, while
`from numpy import NDArray` or `from numpy.typing import ndarray` is still
refused. In a module that writes a carrier import the driver resolves as
foreign -- at module level, or in a module-level `if`/`try` body, including
an `if TYPE_CHECKING:` block that never runs -- every read of the
spelling outside a type annotation is refused at the read with ``reading
`NDArray` outside a type annotation is not supported yet: `from numpy.typing
import NDArray` keeps the name's buffer-annotation meaning in pycc (D-244),
so it may only annotate`` (a call `ndarray(n)`, `isinstance(a, ndarray)`,
`x = NDArray`, a default value, a decorator or a base class, above or below
the import), and every other binding of it (a `def`, a `class`, a parameter,
an assignment, an import that binds it other than the unaliased carrier
import itself such as `import X as ndarray` or `from m import y as ndarray`,
an `except ... as`, a match capture, `type NDArray = ...`) is refused at the
binding with ``binding `ndarray` in a module that imports it with `from numpy
import ndarray` is not supported yet: the import keeps the name's
buffer-annotation meaning in pycc (D-244)``. Both are C0001. Repeating the
carrier import, or guarding it with `if TYPE_CHECKING:`, is the same binding
and is accepted. A module-level item whose own lowering already failed is
not scanned, so `from other import ndarray` reports only its own spelling
refusal, and a module-scope `del ndarray` reports only the imported-name
`del` refusal. A project `numpy.py` or `numpy/typing.py` makes the import
an ordinary project import, which declares no carrier.
Every other from-import shape of such a module keeps its C0001: an aliased
name (`from X import a as b`, [#963](https://github.com/rotnov/pycc/issues/963)),
the wildcard, and a from-import inside a function, class, loop, `with` or
`match` body. Since
[#1383](https://github.com/rotnov/pycc/issues/1383) one nested in a
module-level `if`/`try` body is a foreign import under the top-level form's
rules, reported at its own statement when refused; a nested from-import of a
project or `pycc_std` module keeps the block-body C0001. (A relative import is a project import (D-222) and never reaches
the foreign channel, unless `pycc build --ext --foreign-relative-imports`
(#1366) makes the entry module's relative from-imports foreign; its failures are the `T0021` row above. Under that flag the
entry's absolute imports of its own top-level package are foreign as well (#1382), so they never meet the project-import
diagnostics: a missing module or name there is CPython's runtime `ImportError`, not a `C0001` or `T0021`. The one compile-time refusal is a `C0001` at the entry's line 1 when that package is named like a module `pycc_std` compiles natively, such as `math` (`docs/CLI_SPEC.md`).) A foreign import's identity for the shadowing
rules is its module, its relative level and its name (so `from .x import a`
and `from x import a` are two objects), and `import copy` followed by
`from copy import copy` is the same C0001 as any other rebinding of a foreign
name. Since [#1485](https://github.com/rotnov/pycc/issues/1485) one rebinding is exempt: an `except` handler that
catches a failed import (`ImportError`, `ModuleNotFoundError`, `Exception`, a
tuple naming one, or a bare `except:`) of a module-level `try` whose body
directly imports `N` may rebind `N` with a direct `N = None` (also as one
target of a chained `N = M = None`) or a direct fallback
`from Y import N` / `import Y as N` (`docs/RUNTIME.md`). Any other value
there is a C0001 of its own wording at the `try`: "`N` is rebound in an
`except ImportError` handler to a value other than `None`; only a `None` or a
fallback `import`/`from ... import` statement is supported as the fallback of
a foreign import yet". A handler of such a `try` that only reads `N` without
rebinding it is the `T0021` "name is not defined" refusal. `I0402` and `I0403` are reported once per statement, not once per name:
`from tkinter import Tk, Label` under a native build is one `I0403`, and its
message quotes the whole statement -- an aliased plain import with its alias,
`import xml.dom as d` (#1381).

`pycc_types` also uses it for calls to known Python 3.14
callable builtins that this compiler version does not implement (e.g.
`ValueError("x")`, `Exception("msg")`, `int("5")`, `range(10)` as a
standalone call) -- these are valid Python, not name-resolution failures
(`T0021`), so they are classified as capability gaps. User-defined functions
always take priority: a `def ValueError(...)` is called correctly, not
classified as `C0001`. The same classification applies in both the final
validation pass and the private-helper inference path (issue #142). It is
also used for an *implemented* special-cased builtin called with an argument
shape this version does not support -- `isinstance(f(), int)` (a call as the
inspected operand), `cast(list[int], x)` (a subscripted generic as the
target type, issue #767), and a `cast` whose target type would change the
value's runtime representation, narrow its attribute layout, or cross a
method-override boundary that pycc's static (non-virtual) method dispatch
cannot see through
(`cast(str, 5)`, `cast(int, some_bool)`, a down-cast such as
`cast(Derived, base)`, or an up-cast such as `cast(Base, derived)` where
`Derived` overrides a method `Base` defines; D-198, issue #767) are the
current instances -- for the same reason:
the construct is valid Python that a later slice can implement, not a
by-design rejection. It is also used for a direct method call on a class
that HIR lowering synthesized for a seeded builtin exception (D-188) --
e.g. `except Exception as e: e.__init__("oops")`, or the same call reached
through a user subclass that inherits the method without overriding it
(issue #714) -- because a synthetic class's method-table entry does not
correspond to a real, callable function (D-173 propagates a raised
exception through global runtime state rather than a real allocated
instance); Part 1 of issue #737, issue #711. The same issue #714 also
covers a *construction*, not a dunder call: binding a fresh instantiation
of such a class to a name (`e = MyError("boom")`, for a user subclass
whose MRO reaches a builtin exception without overriding its inherited
constructor) reports `cannot instantiate exception class \`...\` as a
value` -- `raise MyError("boom")` stays the one supported construction
(Part 3 of issue #541). Calling an enum class (`Color()`, `Color(1)`)
reports `cannot call enum class \`...\`` at the call expression, from HIR
lowering's per-item AST scan (`pycc_hir::class::enum_call`, issue #944,
D-233), with a span-less guard of the same text in `pycc_types`'s
`resolve_instantiation` ladder behind it (issue #921) -- the guard still
reports at `1:1` for the residual shapes D-233 enumerates, such as a
module-level rebinding of the class name anywhere in the module
(`Color(); Color = 1`) or a sibling-comprehension rebinding, where the scan
stays suppressed: members are
compile-time singletons reached by name (`Color.RED`), and by-value lookup
is the not-yet-implemented construct; the zero-argument form is a CPython
`TypeError` too and is named as such rather than as unsupported. The scan
folds `if`/`elif TYPE_CHECKING:` bodies exactly as HIR lowering does
(#790), so a call inside such a dead body is not reported. A name that
another module-level `def`, `class`, `import`, or `type` statement also
binds (`def Color()` or `class Color:` beside `class Color(Enum)`) is never
scanned in either order: that program is reported by the name-collision
diagnostic alone; an identical repeated import (`from colors import Color`
twice) binds the same class twice, is not a rebinding, and keeps the
call-expression span, whereas one enum imported under one name through two
module paths (`from colors import Color` beside `from palette import
Color`, `palette` re-exporting it) is indistinguishable from a rebinding
and falls back to the span-less guard at `1:1`. A
scope-local rebinding of the class name (`def f(Color: int) -> None:
Color()`) is not an enum call and keeps its `T0021`. Naming
an enum class as a base (`class Foo(Color): pass`) is rejected from
`validate_bases` on the class header (issue #941): when the enum has
members the message names CPython's own `TypeError: <enum 'Foo'> cannot
extend <enum 'Color'>`, and when it is a member-less docstring-only enum
-- a shape CPython does allow extending -- the message reads
`cannot inherit from member-less enum class \`...\` -- ... not supported
yet`.

A class whose base names one of the eleven builtin types this version
reports by name -- a subset of the types CPython accepts as a base: `int`,
`float`, `str`, `bytes`, `bytearray`, `list`, `tuple`, `dict`, `set`,
`frozenset`, `complex` -- is rejected from
`validate_bases`'s unresolved-base branch, the one that otherwise reports
an unknown class, with `C0001` "class \`X\` inherits from builtin
type \`T\` -- subclassing a builtin type is not supported yet" (Part 1 of
issue #1283, #1318). The failed class's own name is poisoned as for any
failed item, and the message is cascade-shaped, so it is itself suppressed
when an earlier failed item (a `class frozenset:` that did not lower, a
failed `import json as list`) already poisoned the base name (D-219).
Every other unresolved base name -- including `object`, the four types
CPython refuses as a base (`bool`, `range`, `slice`, `memoryview`), and any
of the eleven names that the module rebinds earlier through a type alias,
an import, a `def`, or a top-level binding that itself lowered -- keeps the
`unknown class` text. A rebinding that itself fails to lower and is not
poisoned (a `def frozenset()` whose body is unsupported, a `frozenset =`
assignment whose value is) is invisible to that check, so the class then
reports the builtin-type text alongside the rebinding's own diagnostic.

## Quality bar

- Every error: primary span, ≥1 label, expected/found where applicable, help with a suggestion when one is safe. Populated for arity/type-mismatch, missing-annotation, and literal-index-constraint families as of D-152 (`docs/decisions/D-152-populate-diagnostic-help-for-arity-type.md`). That is a standing contract on those families, not a snapshot of the tree D-152 measured: a diagnostic added to one of them later joins the populated set at its own introduction (`T0053`, D-228 decision 11, is the first such case). Still `None`/empty for name-resolution, capability-limitation, and ambiguous-conflict diagnostics, and human-format output never renders it.
- Suggestions marked machine-applicable must be idempotent + tested. Applying them automatically is intended for a planned `pycc check --fix` flag, which is not yet implemented (see `docs/CLI_SPEC.md`).
- Message text changes are allowed; codes and JSON structure are not (corpus bot fingerprints on code + span shape).
- Every diagnostic a pass can report for a file is emitted (JSON: one object per line). The *first* diagnostic for any input -- code, message, span, position -- was kept byte-identical across the three #864 parts (D-217 rule 2, discharged once D-219 and D-220 landed) so the existing fixtures could verify that collection and ordering changes perturbed nothing; that was a transition invariant, not a release-to-release promise: message text remains changeable under the bullet above, codes and JSON structure do not. The parser reports in ruff's discovery order, which is not always source order. HIR lowering reports, per top-level item in source order, the item's own lowering diagnostic when it fails (and skips the item) followed by one enum-call `C0001` per call of an enum class inside that item that the scan can attribute (D-233's second collection source, #944; a call whose name is shadowed, rebound, or otherwise outside the scan's enumerated limits is not reported by HIR and falls through to the type checker's span-less guard at `1:1`, see the limits above), so one item can contribute several diagnostics; an item that fails only because it names a class or type alias that itself failed to lower (a bare-name annotation or a base class) is skipped silently, with no lowering diagnostic, so a later lowering gap inside such an item surfaces only once the root cause is fixed (D-219); the enum-call scan still runs over such an item, so a call to another, valid enum class inside it is reported (D-233 decision 6). A buffer-carrier misuse `C0001` ([#1380](https://github.com/rotnov/pycc/issues/1380)) is found only once the whole module is lowered, and is placed after the other diagnostics of the item it sits in, so it keeps that order too. Type checking reports one diagnostic per failing function (#868, D-220): a pre-check failure (an incompatible redefinition or attribute redeclaration) is reported alone. Otherwise, if the private-helper solver's list is module-level (a failure in its top-level walk or in a post-body phase such as `propagate_binop_constraints`, and `monomorphize` after checking), that one diagnostic is reported alone and the annotation checker's list is dropped, because a post-body solver diagnostic cannot be matched by function to the checker's entry for the same error. Otherwise the solver's per-function diagnostics are reported in item order, then every checker entry -- per-function or module-level -- whose function the solver did not flag, in the checker's order; a function both phases flag is reported once, with the solver's text. If the solver passes, the checker's list against the solved signatures is reported on its own. Every type diagnostic still carries an empty span and renders at `:1:1` (D-043), so a per-function report is distinguished by its message, not its location, until spans reach `pycc_types`.
- Diagnostics on all Tier-1 platforms byte-identical (paths normalized).
