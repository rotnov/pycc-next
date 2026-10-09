//! `pycc explain <code>`'s content source (D-150): a hand-authored `const`
//! slice of every code registered in `docs/DIAGNOSTICS.md`'s "Initial
//! registry" table, linear-scanned by [`find`] -- the same shape
//! `pycc_std::REGISTRY`/`resolve_symbol` already established for a
//! structurally identical "small hand-authored table, looked up by exact
//! string match" problem (D-136). See D-150 for why this is a `const`
//! slice rather than a runtime `docs/DIAGNOSTICS.md` parser, and for the
//! completeness test's drift-guard mechanism (`tests` module below).
//!
//! Every code registered in `docs/DIAGNOSTICS.md` must have exactly one
//! matching entry here, with the same severity (after this test's
//! `warning→error`-style normalization -- see `tests::registry_severity`
//! below). Adding a new code to that table without a matching entry here
//! fails `completeness_test_covers_every_registered_code_and_severity` in
//! CI; this is deliberate, not a bug in the test.

use crate::Severity;

/// One `docs/DIAGNOSTICS.md` registry entry's long-form documentation:
/// what `pycc explain <code>` prints. `summary` is a short, single-line
/// restatement of the registry table's own "Message (short form)" column;
/// `explanation` is free-form prose describing the real trigger condition
/// (grounded in the code's actual emission site(s), not just the registry
/// table's terse summary -- see D-150); `example` is a minimal Python
/// snippet showing the shape of code the rule is about.
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticExplanation {
    pub code: &'static str,
    pub severity: Severity,
    pub summary: &'static str,
    pub explanation: &'static str,
    pub example: &'static str,
}

/// The full set of documented diagnostic codes (D-150). Kept in
/// `docs/DIAGNOSTICS.md`'s own table order for easy side-by-side review,
/// not that order has any runtime significance -- [`find`] does a plain
/// linear scan regardless of position.
pub const EXPLANATIONS: &[DiagnosticExplanation] = &[
    DiagnosticExplanation {
        code: "C0001",
        severity: Severity::Error,
        summary: "valid Python construct is not implemented by this pycc version",
        explanation: "\
C0001 is a versioned capability diagnostic, not a rejected-by-design language \
rule: it fires whenever HIR lowering reaches a syntactically valid Python \
statement, expression, or annotation shape that this pycc version's frontend \
does not yet lower -- a `with` statement, a starred or nested \
tuple-unpacking target, an unrecognized import shape, or a type annotation this version's lowering \
does not recognize, for example. Since D-228 (issue #918), widened by \
issue #925, the parameterized container annotations `list[T]`, `set[T]`, \
`dict[K, V]` and `tuple[A, B, ...]` *are* lowered in parameter, return, \
local- and module-variable and type-alias positions, so C0001 no longer \
covers every annotation more complex than a bare name; what it still covers \
there is the bare, unparameterized `list`/`set`/`dict`/`tuple` spelling \
(whose message names the parameterized form to write instead in exactly \
those positions, and stays generic where that form is rejected too) and a \
protocol *attribute* whose type is a container, which is not supported yet \
as a protocol attribute (a container type in a protocol *method*'s parameter \
or return type does lower). A protocol class in return-annotation position \
(`def make() -> P:`, issue #934) is also C0001, on a function, a method, and \
a protocol member declaration alike: a protocol is a compile-time-only \
interface, so a call to such a function has no concrete type to bind, and \
a protocol type is currently supported in parameter and variable positions \
only. String conversion of a value whose type only the checker can see is \
also C0001, reported by `pycc_types` at `1:1` rather than at the expression \
(issue #977, D-237): a `print()` argument or f-string interpolation typed as \
a class instance that is neither a `@dataclass` instance nor a caught \
builtin exception (a plain class, an `Enum` member, a non-dataclass subclass \
of a dataclass, a user exception class, or any user class declared under a \
builtin exception name), or typed as a protocol, whose concrete class is not \
known at the conversion site. It also fires for calls to known Python 3.14 \
callable builtins that this compiler version does not implement (e.g. \
`ValueError(\"x\")`, `Exception(\"msg\")`, `int(\"5\")`, `range(10)` as a \
standalone call) -- these are valid Python, not name-resolution failures \
(`T0021`), so they are classified as capability gaps. User-defined functions \
always take priority: a `def ValueError(...)` is called correctly, not \
classified as `C0001`. The same classification applies in both the final \
validation pass and the private-helper inference path (issue #142). It is \
also used for an *implemented* special-cased builtin called with an argument \
shape this version does not support -- `isinstance(f(), int)` (a call as the \
inspected operand), `cast(list[int], x)` (a subscripted generic as the \
target type, issue #767), and a `cast` whose target type would change the \
value's runtime representation (`cast(str, 5)`, `cast(int, some_bool)`; \
D-198, issue #767) are the current instances -- for the same reason: the \
construct is valid Python that a later slice can implement, not a by-design \
rejection. Two `from __future__ import ...` shapes are C0001 for the same \
reason (issue #919, D-229): `barry_as_FLUFL`, a valid feature that changes \
the grammar (`<>` in place of `!=`) and that the vendored parser does not \
implement, and `from __future__ import x as y`, the `from ... import x as \
y` aliasing gap (issue #963; CPython binds a `_Feature` object pycc never \
models -- a stdlib module behind `import X as Y` lowers since issue #962, \
D-231); the nine no-op \
features lower to nothing, and a feature name CPython itself rejects is \
`L0001`, not C0001. A class-attribute initializer that is not a literal \
is C0001 too (issue #1345, D-256), with one message per shape; a deferred \
shape names its tracking issue: a call, a name or attribute reference, and a container \
(#1348), `staticmethod(...)` of a call, lambda, subscript or other \
non-reference expression (#1348), `classmethod(...)` or a `staticmethod` whose \
root is a pycc function, class or module, or an undefined name (#1347), a `staticmethod` whose root name an earlier \
statement of the same class body binds (#1347), the annotated spelling, a \
conditional import root, and a dunder, class-private or container-dispatched \
name (#1348). Two shapes stay refused, as CPython's own `TypeError` or \
rebinding would make them: `staticmethod(...)` called with other than exactly \
one positional argument (none, several, a keyword or a starred argument), \
and a `staticmethod` that is rebound in the module or class body. \
The one admitted non-literal shape is `name = staticmethod(<foreign import \
reference>)`. Since #1337 (D-254) two more shapes are C0001: an \
inherited method body compiled once more for a subclass that lacks a \
capability the body needs (a `print(self)` of a non-dataclass subclass of \
a dataclass), attributed to the origin body's file with a note naming \
the subclass; and a user exception class whose MRO resolves an \
instance-protocol dunder (`__str__`, `__bool__`, `__eq__`, ...) to a user \
class rather than to a builtin exception base, reported at the class \
definition because a raised exception value would silently ignore that \
dunder (Part 3 of #541). Since #1344 an unannotated private helper that \
returns a set comprehension whose user-class instance element the \
constraint solver cannot type yet is C0001 too, \"cannot infer an \
unannotated private helper's `set[C]` return yet\", with help naming \
#1360; annotating the helper's return (`-> set[C]`) compiles today. \
Apart from the two permanent refusals above, the \
construct remains reserved and stops \
producing C0001 the moment the corresponding roadmap slice is implemented; \
until then the diagnostic's span points at the unsupported node and the \
message names the construct in Python terms, so it stays actionable rather \
than a bare \"not implemented\".",
        example: "\
class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y
",
    },
    DiagnosticExplanation {
        code: "C0002",
        severity: Severity::Error,
        summary: "recognized stdlib module, but the specific imported symbol is not registered",
        explanation: "\
C0002 is distinct from C0001 (D-136): it fires when a `from <module> import \
<name>` statement's module *is* recognized by pycc_std's hand-authored \
registry (currently only `math`), but the specific imported name inside it \
is not one of the registered symbols (currently only `sqrt` and `pi`). This \
distinguishes \"we don't support this import shape at all\" (C0001) from \"we \
support this module, just not this particular symbol yet\" (C0002), and it \
fails the whole import statement rather than partially binding the names \
that did resolve.",
        example: "\
from math import isnan
",
    },
    DiagnosticExplanation {
        code: "C0003",
        severity: Severity::Error,
        summary: "--ext (or, for a comparison or hash dunder, an embedded executable) cannot carry this public function's or exported method's signature across the CPython boundary",
        explanation: "\
C0003 is an `ext`-mode capability gap (D-244) -- and, since #1470, an \
embedded executable's gap for a comparison or hash dunder -- and it is distinct from C0001 \
in what is missing: the construct itself is fully supported -- pycc compiles \
the function without complaint for a `native` executable -- but the \
`PyObject*` boundary that a hosted extension module needs for that \
signature does not exist yet. `docs/RUNTIME.md`'s `ext` boundary section \
is the canonical admissibility matrix: which types cross, in which \
direction, what each admitted one narrows, and which members are in the \
export set at all. Because D-244 rule 1 \
exports *every* public module-level function -- and, since #1143, every \
public `@staticmethod` and `@classmethod` of a public non-exception class, \
and since #1145 every public instance method of such a class when some \
*published* class whose method resolution order contains it is \
*constructible* and resolves that method name to the declaring class \
rather than to a binding of its own, \
published as `mod.Class.method` -- (a stable ABI cannot \
have a per-function opt-out without also having a way to spell it), one \
unexportable public signature \
fails the whole `--ext` build rather than silently shipping an artifact \
missing a name its source clearly defines. A member that is not in the \
export set at all -- a `@property` getter or setter, any member of a \
private or user exception class, and every instance method excluded \
because no host-obtainable instance can ever receive it, which includes \
one every such class shadows with a binding of its own -- is \
never a C0003: it is excluded as representation, not reported as a gap. \
An `@abstractmethod` is covered by that same clause rather than by a rule \
of its own: an abstract class is never constructible, and a constructible \
subclass does not make its abstract base's stub receivable. \
Every gap in a program is \
reported at once. The two fixes are to make the member private under \
D-038's rule -- rename it with a leading underscore, which removes it from \
the export set -- or to build without `--ext`. The one exception is a PEP \
562 module hook (#1467): a top-level `def __getattr__` or `def __dir__` of \
the entry module is in the export set although it is a dunder, and \
renaming it would silently stop the host from calling it, so its fixes are \
to change its signature or to build without `--ext`. The comparison and \
hash dunders (`__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__`, `__ge__`, \
`__hash__`) of every compiled class of the module, since an instance of \
any class can reach the host, are a second such case (#1427): they become \
the carrier type's comparison and hash slots, so one bound as a \
`@staticmethod`, `@classmethod`, `@property` or class attribute, or one \
with an uncarriable signature, is a C0003 whose fix is to make it an \
instance method the boundary can carry, and one on a PEP 695 generic class \
is a C0003 whose fix is an erased `Generic[T]` base instead of PEP 695 \
syntax; never drop it, which would answer `==` and `hash()` by identity. \
Since #1470 an embedded executable -- a plain build of a program that \
imports CPython -- installs the same slots, so the same bindings are a \
C0003 there too, worded for the embedded executable, as is a slot method \
whose signature needs an export thunk (it carries a `tuple` or returns a \
slice of a `memoryview` argument), which an embedded executable, compiled \
without export thunks, cannot call. The refusal covers every class of an \
embedded program, not only one whose instances reach CPython, so such a \
program that built before #1470 is refused now.",
        // The example has to name a signature the boundary still refuses
        // *and* that `native` mode compiles, since the explanation above
        // rests on exactly that contrast. It must be re-pointed whenever
        // the matrix widens: `str` was admitted by #1049 and a scalar-
        // element `tuple` by #1050, which leaves `list` as the nearest
        // remaining gap -- and a durable one, since a `list` is a mutable
        // object with no by-value crossing rather than an unimplemented
        // spelling. The element type is
        // `int` because `pycc_hir::container::check_container_ty` rejects
        // every other `list[T]` with `T0034` before the `--ext` capability
        // scan ever runs -- `list[str]` would be a `T0034` example wearing
        // a `C0003` label. Nothing checks this against the implementation
        // (#1056), so a stale example is a user-facing claim that ships
        // wrong in silence.
        example: "\
def join(parts: list[int]) -> str:
    return \"\"
",
    },
    DiagnosticExplanation {
        code: "L0001",
        severity: Severity::Error,
        summary: "syntax error (span + expected set)",
        explanation: "\
L0001 is pycc's parser-level syntax-error code: the parser (via \
ruff_python_parser) could not build a valid syntax tree from the source, and \
the diagnostic's span and message come directly from the underlying parse \
error. L0001 is also reused, without a parser \"expected set\", for a \
post-parse context violation caught during HIR lowering rather than \
parsing: `break`/`continue` outside a loop, `async for` outside an async \
function, `yield`/`yield from` outside a function, `return`/`break`/ \
`continue` inside a `finally` block that would exit it (PEP 765), \
`return`/`break`/`continue` inside an `except*` clause body (PEP 654), and \
a `from __future__ import ...` CPython rejects: an unknown feature name \
including `*` (`future feature <name> is not defined`), `braces` (`not a \
chance`), or a future import after the docstring-and-future-imports \
prologue (`from __future__ imports must occur at the beginning of the \
file`) -- \
CPython classifies all of these as `SyntaxError` too (a `SyntaxWarning` as \
of CPython 3.14 for the `finally` case specifically), even though pycc only \
detects them one lowering stage later than the grammar itself. `T0024` \
(\"`return` outside a function\") is the closest existing precedent for this \
same \"keyword valid only in a specific context\" family but deliberately \
stayed under `T0xxx` instead, an accepted, unfixed inconsistency.",
        example: "\
def f(:
    pass
",
    },
    DiagnosticExplanation {
        code: "L0002",
        severity: Severity::Error,
        summary: "Python version mismatch (feature needs 3.14 level)",
        explanation: "\
L0002 is reserved for a source construct that requires a Python grammar or \
runtime feature beyond the language level this pycc version's frontend \
targets (3.14, D-012). It is not currently emitted: pycc's parser does not \
yet implement any version-gated grammar dispatch, so there is nothing in \
today's frontend that can distinguish \"valid 3.14 syntax\" from \"syntax that \
needs a later Python version\" -- every construct outside the currently \
implemented subset falls through to C0001 instead. Do not confuse this with \
`pycc.toml`'s separate `[project] python = \"...\"` validation (D-012, \
`src/project_config.rs`), which rejects any value other than the literal \
string `\"3.14\"` with a plain configuration error, not this diagnostic code.",
        example: "\
# a hypothetical future syntax feature beyond Python 3.14 would be
# reported here once L0002 is wired up to a real version check
",
    },
    DiagnosticExplanation {
        code: "T0001",
        severity: Severity::Error,
        summary: "public function missing annotation",
        explanation: "\
A function without a leading underscore is part of a module's public \
surface. pycc's type checker infers types only for private \
(underscore-prefixed) helper functions; every public function must declare \
both its parameter types and its return type explicitly, since the checker \
never performs whole-program inference across a module boundary it cannot \
see into. T0001 fires separately for a missing parameter annotation \
(\"parameter `<name>` of public function `<fn>` needs a type annotation\") \
and a missing return annotation (\"public function `<fn>` needs a return \
type annotation\") -- a function can trigger it more than once if both are \
missing. One parameter case is exempt in a module compiled into a `pycc \
build --ext` artifact (#1409): an unannotated parameter whose default is a \
literal `int`, `float`, `bool` or `str` takes that literal's type, and a \
method parameter defaulting to `None` the opaque CPython object, exactly as \
if it carried that annotation.",
        example: "\
def add(a: int, b: int) -> int:
    return a + b
",
    },
    DiagnosticExplanation {
        code: "T0002",
        severity: Severity::Error,
        summary: "`Any` outside interop boundary",
        explanation: "\
The bare name `Any` is rejected as a type annotation in every module of a \
`native` program: \"`Any` is not permitted in pycc code outside a declared \
interop boundary\" is the literal message HIR lowering's annotation \
resolver produces. pycc's static, whole-function type checking has no way \
to make `Any` sound without an explicit escape hatch. A module compiled into \
a `pycc build --ext` artifact is one such boundary (D-258, #1397): there \
`Any` is the opaque CPython object -- a top type, not PEP 484's gradual \
`Any` -- and crosses the export boundary unchanged, so this code is never \
reported by an `--ext` build. Everywhere else every occurrence of `Any` in \
an annotation is rejected outright, regardless of position; the general \
compiler-classified boundary is the planned v0.7 transparent interop work.",
        example: "\
from typing import Any

def f(x: Any) -> int:
    return 0
",
    },
    DiagnosticExplanation {
        code: "T0003",
        severity: Severity::Error,
        summary: "untyped empty container needs annotation",
        explanation: "\
T0003 reports an empty list or dict literal (`[]`, `{}`) whose element type \
cannot be inferred from anywhere -- CPython accepts `x = []` and infers \
nothing about future element types, but pycc's static checker needs a \
concrete element type before it can type-check any later use of `x`. D-245's \
pre-check pass resolves the common shapes before this code can fire: an \
annotation on the assignment (`x: list[int] = []`), an existing binding for \
the same name, or the first later use that *produces* an element type \
(`x.append(1)`, `d[\"k\"] = 1`). T0003 is what remains once all three fail -- \
including every position that binds no name at all, where there is nothing \
to scan forward from: a call argument (`f([])`), a `return []`, a nested \
literal (`[[]]`, `{\"k\": []}`), or a module-level assignment. An \
unannotated `self.xs = []` in `__init__` reports it too when no source types \
the slot: annotate the attribute (`self.xs: list[int] = []`) or append a value \
to it in one of the class's own methods (`self.xs.append(1)`); a renamed \
receiver (`def __init__(this)`) is named as written. An unannotated \
`self.d = {}` establishing the attribute in `__init__` never reaches this \
check: it is rejected earlier with `C0001`. A tuple-unpacking assignment \
(`L, R = [], []`) does reach it: each `[]` is an element of a tuple display, \
a nested literal, not a name's own value. A class-body declaration (`d: dict[str, int]`) types an \
establishing `self.d = {}` instead, and an establishing `[]` or `{}` of the \
wrong shape for its declared slot does reach this check. A later `self.d = {}` or `self.xs = {}` reset in another method \
does reach it when the slot it stores into is not a `dict`. Where a binding \
name *is* available at a function-scope assignment it is named in the \
message; a module-level assignment keeps the generic wording, because the \
substitution is wired into the function-scope seams only. Inside a function \
body, give the binding an annotation or use the container in a way that \
fixes its element type. At module level neither works -- the pass does not \
run there at all, so `x: list[int] = []` reports this same code -- and the \
remedy is to move the binding into a function body; the `help` on the \
diagnostic itself says whichever of the two applies.",
        example: "\
def f() -> None:
    x = []  # element type not yet inferable from context
",
    },
    DiagnosticExplanation {
        code: "T0021",
        severity: Severity::Error,
        summary: "name resolution (including an unbound local), operand, call, or inference type mismatch",
        explanation: "\
T0021 is the type checker's broadest, most frequently emitted code: it \
covers name resolution (\"name `<x>` is not defined\", or \"local name `<x>` \
is not bound before this use\" for a name that is assigned somewhere in the \
function but not yet on this path), calling a non-callable binding, \
operator/operand type mismatches inside expression inference, and general \
call-argument type mismatches -- including the stdlib-symbol shapes D-136 \
adds (calling a stdlib constant, or referencing a stdlib function without \
calling it). Since #898 (D-222) it also covers the project-import shapes \
CPython itself rejects: importing a name the target module does not define, \
a relative import with no parent package or one that climbs above the \
top-level package, and a relative target that resolves to no module. (An \
import shape pycc merely has not implemented yet stays `C0001`.) Since #1125 (Part 1 of #884) it also covers the call shapes CPython rejects when a keyword \
argument is bound to a parameter by name: an unexpected keyword name, a \
positional-only parameter passed as a keyword, a parameter supplied both \
positionally and by keyword, and a parameter left unsupplied. Since #1331 \
(Part 1 of #1327) it also reports `hash()` of a `list`, `dict` or `set` as \
\"unhashable type: `<ty>`\", the `TypeError` CPython raises for it, reported \
statically; `hash()` of a type CPython can hash but pycc cannot yet is \
`C0001` instead. Since #1335 (Part 1 of #1332) it also reports `hash()` of \
an instance whose class is unhashable (it defines `__eq__` without \
`__hash__`), as \"unhashable type: `<class>`\", or whose `__hash__` does \
not return an integer, as \"`__hash__` method should return an integer\". \
Since #1337 (D-254) it also fires in an inherited method body compiled \
once more for a subclass, when that copy passes the subclass receiver where \
only the base class is accepted (`helper(self)` with `def helper(a: A)`, or \
a dataclass `==` the subclass lacks); the diagnostic is attributed to the \
origin body's file with a note naming the subclass. \
Different call \
sites across `pycc_types` and `pycc_hir` construct T0021 with \
different messages for these distinct situations; the shared code reflects \
that they are all instances of the same underlying category (a name or \
value did not have the type or binding shape an expression needed), not \
that pycc considers them identical errors.",
        example: "\
def f() -> int:
    return y  # `y` is never defined
",
    },
    DiagnosticExplanation {
        code: "T0022",
        severity: Severity::Error,
        summary: "return type mismatch",
        explanation: "\
T0022 fires when a function's `return` statement's value has a type that is \
not assignable to the function's own declared (or, for a private helper, \
inferred) return type -- including the implicit `None` return type when a \
function's body falls off the end without an explicit `return` in a path \
that needs one. It also fires in an inherited method compiled again for a \
subclass (#1337, D-254) whose body returns `self` from a method annotated \
with the base class: the copy's `self` is the subclass, which the base-typed \
return slot would silently widen, so the program is refused and the \
diagnostic carries the note \"while compiling `D.m` inherited by subclass \
`C`\". A CPython object (a foreign-import value, such as the result of \
`o == 1`) returned from a `-> bool`, `-> int`, `-> float` or `-> str` \
function is refused rather than converted implicitly (D-258, #1419): the \
help names the explicit conversion, e.g. `return bool(o == 1)`.",
        example: "\
def f() -> int:
    return \"not an int\"
",
    },
    DiagnosticExplanation {
        code: "T0023",
        severity: Severity::Error,
        summary: "incompatible assignment",
        explanation: "\
T0023 fires when a plain (non-annotated) reassignment's value type is not \
assignable to the type the target name was already bound to from an earlier \
assignment -- e.g. rebinding a name first bound to `int` with a `str` value \
later in the same function. This is distinct from `T0025` (an annotated \
assignment's initializer disagreeing with its own declared annotation) and \
`T0026` (an assignment disagreeing with an earlier value-less `x: <type>` \
declaration): T0023 is specifically \"this name was already inferred to \
have a type from a real value, and this new value doesn't fit it\". \
A `for` target is a reassignment of this kind too: iterating a foreign \
`object` value binds the loop variable to `object`, so \
`for x in obj.items:` reports T0023 when `x` already holds another type \
in the same module scope -- a name keeps one type for its whole scope, \
and the loop target cannot re-type it. That loop is only accepted in a \
module body at all: inside a function body it is refused with `I0404` \
before its target is ever examined, so this T0023 case never arises \
there.",
        example: "\
def f() -> None:
    x = 1
    x = \"now a string\"
",
    },
    DiagnosticExplanation {
        code: "T0024",
        severity: Severity::Error,
        summary: "`return` outside a function",
        explanation: "\
T0024 fires when a `return` statement appears outside any function body -- \
at module (top) level, for example. CPython raises `SyntaxError` for this \
too, but pycc catches it during type checking rather than parsing, so it \
keeps its own `T0xxx` code instead of joining `L0001`'s reused \
context-violation family (`break`/`continue`/`async for`/`yield` outside \
their valid context); this is a deliberate, accepted inconsistency, not an \
oversight. T0024 also takes precedence over L0001's `finally` (PEP 765) and \
`except*` (PEP 654) restrictions: a `return` at module level inside either \
construct reports T0024, matching CPython, whose own fatal error there is \
`'return' outside function` rather than the construct-specific one.",
        example: "\
return 1
",
    },
    DiagnosticExplanation {
        code: "T0025",
        severity: Severity::Error,
        summary: "annotated-assignment initializer incompatible with its declared annotation",
        explanation: "\
T0025 fires when an annotated assignment's own right-hand-side value has a \
type that does not match the annotation written on the same statement, \
e.g. `x: int = \"hello\"`. The annotation itself is what determines `x`'s \
type going forward (not the initializer's inferred type) -- T0025 is the \
check that the initializer is honest about that annotation at the point it \
is declared. It also fires in an inherited method compiled again for a \
subclass (#1337, D-254) whose body binds `self` to a local annotated with \
the base class (`x: A = self`): the copy's `self` is the subclass, which \
the annotation would silently widen, so the program is refused and the \
diagnostic carries the note \"while compiling `D.m` inherited by subclass \
`C`\". A CPython object (a foreign-import value) initializing a `bool`, \
`int`, `float` or `str` annotation, such as `b: bool = o == 1`, is refused \
rather than converted implicitly (D-258, #1419): the help names the \
explicit conversion, e.g. `b: bool = bool(o == 1)`.",
        example: "\
def f() -> None:
    x: int = \"hello\"
",
    },
    DiagnosticExplanation {
        code: "T0026",
        severity: Severity::Error,
        summary: "a later plain or annotated assignment is incompatible with an earlier value-less declaration",
        explanation: "\
T0026 fires when a name was first declared with a value-less annotation \
(`x: int`, with no `= ...`) and a later assignment or re-declaration to that \
same name has a type that is not assignable to the original declaration. \
This is distinct from `T0023` (nothing was previously \"inferred\" here -- \
the name was declared, never assigned, before this point) and `T0025` (the \
mismatch is against an earlier statement's declaration, not the current \
statement's own annotation).",
        example: "\
def f() -> None:
    x: int
    x = \"not an int\"
",
    },
    DiagnosticExplanation {
        code: "T0030",
        severity: Severity::Error,
        summary: "non-exhaustive `match` (missing cases listed)",
        explanation: "\
T0030 is emitted when a `match` statement (PEP 634 structural pattern \
matching) whose patterns do not cover every possible value of the matched \
expression's type. An unguarded irrefutable pattern (`_`, a capture, or \
an `Or` containing one) is always exhaustive. A `bool` subject is \
exhaustive when both `True` and `False` are covered. An enum subject is \
exhaustive when all enum members are covered. All other types require an \
irrefutable pattern to be exhaustive. Non-exhaustive matches are rejected \
by the type checker — add a `case _:` wildcard or cover all remaining \
cases.",
        example: "\
def describe(n: int) -> str:
    match n:
        case 0:
            return \"zero\"
        # missing a case (or a wildcard `case _:`) for every other int
",
    },
    DiagnosticExplanation {
        code: "T0031",
        severity: Severity::Error,
        summary: "`@override` without matching base method",
        explanation: "\
T0031 is emitted when a method decorated `@override` (PEP 698, `typing.\
override`) does not actually override a same-named method or property on \
any base class in the class's MRO. The check walks the MRO (excluding the \
current class itself) looking for a method or property with the same name \
as the `@override`-decorated method. If no match is found, T0031 is \
reported. This catches typos in method names that are intended to override \
a base class method but don't actually match any base class member.",
        example: "\
class Base:
    def run(self) -> None: ...

class Derived(Base):
    @override
    def run_typo(self) -> None: ...  # does not override anything on Base
",
    },
    DiagnosticExplanation {
        code: "T0032",
        severity: Severity::Error,
        summary: "list literal elements must all share one type",
        explanation: "\
T0032 fires when a `list[...]` literal's elements do not all infer to the \
same element type, e.g. mixing `int` and `str` values in one `[...]` \
literal. pycc's v0.2 list representation needs one concrete, uniform \
element type for the whole list (D-105); a genuinely heterogeneous list has \
no such type to assign it.",
        example: "\
def f() -> None:
    xs = [1, \"two\", 3]
",
    },
    DiagnosticExplanation {
        code: "T0033",
        severity: Severity::Error,
        summary: "value does not support list/dict/set/frozenset operations, or `len()` called with the wrong number of arguments",
        explanation: "\
T0033 covers a family of \"this value does not support this container \
operation\" mismatches: subscript (`x[i]`), slicing, item assignment \
(`x[i] = v`), iterating with `for`, and the built-in container methods \
`.append()`/`.pop()`/`.get()`/`.add()`, when the receiver's inferred type \
isn't a `list`/`dict`/`set`/`frozenset` at all, or isn't the specific container kind \
that method requires. It also covers `len(...)` being called with a number \
of arguments other than exactly one, and `len(...)`'s single argument not \
being a `list[T]`/`dict[K, V]`/`set[T]`/`frozenset[T]`.",
        example: "\
def f() -> int:
    x = 5
    return len(x, x)  # len() takes exactly one argument
",
    },
    DiagnosticExplanation {
        code: "T0034",
        severity: Severity::Error,
        summary: "list codegen only supports `list[int]` in v0.2",
        explanation: "\
T0034 fires when a `list[T]` value's element type `T` is anything other \
than `int` -- pycc's v0.2 list codegen (D-105) implements only \
`list[int]`'s runtime representation; `list[str]`, `list[bool]`, nested \
lists, and every other element type are recognized by the type checker \
(so `T0032`'s uniform-element-type check still applies to them) but not \
yet compilable, and are rejected here instead of silently miscompiled.",
        example: "\
def f() -> None:
    xs: list[str] = [\"a\", \"b\"]
",
    },
    DiagnosticExplanation {
        code: "T0035",
        severity: Severity::Error,
        summary: "dict literal key/value pairs must all share one key type and one value type",
        explanation: "\
T0035 fires when a `dict[...]` literal's entries do not all infer to one \
consistent key type and one consistent value type, mirroring `T0032`'s \
uniform-element-type rule for lists -- e.g. mixing a `str` key with an \
`int` key in the same `{...}` literal, or mixing value types across \
entries.",
        example: "\
def f() -> None:
    d = {\"a\": 1, 2: \"b\"}
",
    },
    DiagnosticExplanation {
        code: "T0036",
        severity: Severity::Error,
        summary: "dict codegen only supports `dict[str, int]` in v0.2",
        explanation: "\
T0036 fires when a `dict[K, V]` value's key/value types are anything other \
than exactly `str` keys and `int` values -- pycc's v0.2 dict codegen \
(D-122) implements only `dict[str, int]`'s dense insertion-ordered \
representation; other key/value type combinations are type-checked but not \
yet compilable and are rejected here rather than silently miscompiled.",
        example: "\
def f() -> None:
    d: dict[int, int] = {1: 1, 2: 2}
",
    },
    DiagnosticExplanation {
        code: "T0037",
        severity: Severity::Error,
        summary: "set literal elements must all share one type",
        explanation: "\
T0037 fires when a `set(...)`/`{...}` set literal's elements do not all \
infer to the same element type, mirroring `T0032`'s uniform-element-type \
rule for lists.",
        example: "\
def f() -> None:
    xs = {1, \"two\", 3}
",
    },
    DiagnosticExplanation {
        code: "T0038",
        severity: Severity::Error,
        summary: "set element type not compiled yet (int or a user-class instance only)",
        explanation: "\
T0038 fires when a `set[T]` value's element type `T` is anything other than \
`int` or an instance of a user class -- pycc's set codegen (D-122, widened \
by D-255 in #1343) implements only those elements' representation; other \
element types are type-checked but not yet compilable and are rejected \
here rather than silently miscompiled. `frozenset[T]` shares the gate. \
Whether a particular user class is hashable is a separate question, \
answered at the set literal, `.add(...)` or set comprehension that inserts \
it (T0054, or C0001 for a class shape pycc does not compile yet).",
        example: "\
def f() -> None:
    xs: set[str] = {\"a\", \"b\"}
",
    },
    DiagnosticExplanation {
        code: "T0039",
        severity: Severity::Error,
        summary: "tuple element type not compiled yet (int/bool/float only, D-116)",
        explanation: "\
T0039 fires when a `tuple[...]` value contains an element type other than \
`int`, `bool`, or `float` -- pycc's v0.2 tuple codegen (D-116) represents a \
tuple as a fixed SSA struct value whose fields are scalar types only; \
`str`, nested containers, and other element shapes inside a tuple are not \
yet compilable and are rejected here.",
        example: "\
def f() -> None:
    t: tuple[str, int] = (\"a\", 1)
",
    },
    DiagnosticExplanation {
        code: "T0040",
        severity: Severity::Error,
        summary: "tuple index must be a non-negative literal integer within range",
        explanation: "\
T0040 fires on a tuple-index read (`t[i]`) whose index is not a literal, \
non-negative integer known at compile time, or whose value falls outside \
the tuple's own fixed length -- pycc's v0.2 tuple representation (D-116) \
supports only literal-index reads (no dynamic indexing, no negative \
indices) because each field access lowers directly to a struct-field read.",
        example: "\
def f() -> int:
    t: tuple[int, int] = (1, 2)
    return t[5]  # only indices 0 and 1 exist
",
    },
    DiagnosticExplanation {
        code: "T0041",
        severity: Severity::Error,
        summary: "local name may not be bound on every path reaching this use",
        explanation: "\
T0041 fires when a local name is assigned on only some of the control-flow \
paths that can reach a later read of it -- e.g. inside an `if` with no \
`else`, or inside a `while`/`for` body that might run zero times -- rather \
than on every path (issue #118 Part 1). After a `try`, a name is bound \
when every path that completes the statement binds it (#1289): the `else` \
path after a completed body, plus every handler whose body does not always \
return or raise. A handler that binds nothing (`except E: pass`), a read \
inside `finally`, or a read after the statement of any name some handler \
binds with `except ... as` is still T0041. This is pycc's strict AOT \
equivalent of CPython's `UnboundLocalError`, but caught at compile time via \
a three-state binding model (definitely bound / maybe bound / unbound) \
tracked across `if`/`while`/`for`/`try` control-flow joins, instead of at \
runtime. T0041 is distinct from `T0021`'s \"local name `<name>` is not \
bound before this use\" message: T0021 covers a name never assigned on any \
path reaching the read, while T0041 covers a name assigned on some but not \
all paths.",
        example: "\
def read_value(flag: bool) -> int:
    if flag:
        x = 1
    return x
",
    },
    DiagnosticExplanation {
        code: "T0042",
        severity: Severity::Error,
        summary: "generic-function shape or call-site instantiation rejected beyond the v0.2 thin slice",
        explanation: "\
T0042 covers every way a PEP 695 generic function or `type` alias can fall \
outside pycc's v0.2 thin generics slice (D-133/D-134/D-135): more than one \
type parameter on a single function; a type parameter used in a container \
position (e.g. `list[T]`); a call site whose different occurrences of the \
same type parameter resolve to different concrete types; a call site whose \
argument for the type parameter is not `int`/`float`/`bool`/`str`; a \
generic function's body calling itself or any other generic function \
(recursive generic instantiation is outside the single-call-site \
monomorphization this slice implements); and an argument whose type cannot \
be inferred because it depends on a still-uninstantiated generic parameter.",
        example: "\
def first(x: T, y: T) -> T:
    return x

def use() -> None:
    first(1, \"mismatched\")  # both occurrences of T must resolve the same
",
    },
    DiagnosticExplanation {
        code: "T0043",
        severity: Severity::Error,
        summary: "attribute access, attribute assignment, or method call on a non-instance value",
        explanation: "\
T0043 fires when `base.attr`, `base.attr = value`, or `base.method(...)` is \
used against a `base` whose type is not a class instance (D-154, Part 1 of \
#375) -- e.g. an `int`, a `list[int]`, or any other non-`Ty::Instance` \
value. pycc's v0.3 class model resolves attribute and method access \
entirely at compile time against the base's declared class shape, so the \
base must be known, structurally, to actually be an instance of some class \
before an attribute or method name on it can mean anything.",
        example: "\
def f(x: int) -> int:
    return x.value  # `int` is not a class instance
",
    },
    DiagnosticExplanation {
        code: "T0044",
        severity: Severity::Error,
        summary: "class has no attribute or method with the given name",
        explanation: "\
T0044 fires when `base.attr`, `base.attr = value`, or `base.method(...)` \
names an attribute or method that the base's own class does not declare \
(D-154, Part 1 of #375) -- an instance attribute is only ever established \
by a first assignment inside `__init__`, and a method is only ever one of \
the class body's own `def`s, so any other name is unknown at compile time. \
A mismatched argument count or type at an instantiation or method call site \
is a separate condition and reuses `T0021` instead, exactly like an \
ordinary function call. The same code also covers CPython's `TypeError: \
... is not subscriptable` for `Base[...]` (PEP 560, #610/#611): in value \
position on a class that defines no `__class_getitem__` anywhere in its MRO \
and is not a PEP 695 generic class -- annotation position is no longer \
gated on subscriptability, since PEP 649/749 evaluates annotations lazily \
and the `TypeError` is unreachable from there (#1130) -- and (#931) in \
annotation position when the base resolves to something that is not a \
nameable type at all: a PEP 695 type parameter (`T[int]`), a builtin scalar \
(`int[str]`), `Self` inside a class, or a `type` alias to a non-class type \
(`type A = int` then `A[str]`), the buffer carrier excepted (`type Arr = \
memoryview` then `Arr[float]` resolves to the carrier and is accepted, \
#1130) -- where the type argument used to be silently discarded. A \
`type A = C` alias to a class behaves exactly as `C[...]` would; an \
undefined base keeps its `C0001`; `Any[...]` keeps `T0002` in a `native` \
program, and in a `pycc build --ext` module, where `Any` and `object` are \
the opaque CPython object (D-258), `Any[...]` and `object[...]` are this \
code, since neither is generic.",
        example: "\
class Point:
    def __init__(self, x: int) -> None:
        self.x = x

p = Point(1)
p.y  # `Point` has no attribute named `y`
",
    },
    DiagnosticExplanation {
        code: "T0045",
        severity: Severity::Error,
        summary: "cannot reassign a `Final` name (PEP 591)",
        explanation: "\
T0045 fires when a name declared `Final` (PEP 591) is reassigned after its \
initial binding. A `Final`-annotated name may be assigned exactly once; any \
subsequent assignment -- plain `x = ...` or annotated `x: Final[...] = ...` \
-- is rejected. `Final[X]` unwraps to `X` at HIR-lowering time (the type is \
just `X`; `Final` is a binding-level property, not a type-level one), and \
the non-reassignability is tracked by the type checker's `Environment.finals` \
set, populated from `HirStmt::AnnAssign`'s `is_final` flag. The initial \
assignment's own `check_assignment` call does not see the name in `finals` \
yet (the insert happens after it returns), so only a subsequent \
reassignment triggers T0045. A value-less `Final` declaration (`x: \
Final[int]` with no `= ...`) also marks the name as `Final`; the first real \
assignment to it is allowed, but a second is rejected.",
        example: "\
def f() -> None:
    x: Final[int] = 1
    x = 2  # cannot reassign a Final name
",
    },
    DiagnosticExplanation {
        code: "T0046",
        severity: Severity::Error,
        summary: "class does not conform to a protocol (PEP 544 structural typing)",
        explanation: "\
T0046 fires when a value of a concrete class type is used where a protocol \
type is expected (PEP 544, issue #380) and the concrete class does not \
structurally conform to the protocol -- i.e. it is missing one or more of \
the protocol's required methods or attributes, or a method/attribute it \
does provide has an incompatible signature/type. pycc's protocols are \
compile-time-only structural interfaces (D-166): no runtime protocol \
object or vtable is generated. Conformance is checked when assigning, \
passing, or returning a value against a `Ty::Protocol` annotation, and \
when using `isinstance` with a `@runtime_checkable` protocol. A protocol \
is declared with `class P(Protocol):` and may contain method signatures \
(declaration-only bodies: `...` or `pass`) and annotated attributes (no \
default value). Protocol inheritance (`class Q(P):`) copies and may \
redeclare the base protocol's members. `@runtime_checkable` enables \
`isinstance` checks against the protocol (presence-only, matching \
CPython's own `@runtime_checkable` semantics). `issubclass` with a \
protocol is always rejected, since protocols use structural typing, not \
nominal inheritance.",
        example: "\
from typing import Protocol

class Drawable(Protocol):
    def draw(self) -> None: ...

class Circle:
    def draw(self) -> None:
        pass

class Square:
    pass

d: Drawable = Circle()  # OK -- Circle has draw()
d: Drawable = Square()  # T0046 -- Square lacks draw()
",
    },
    DiagnosticExplanation {
        code: "T0047",
        severity: Severity::Error,
        summary: "`super()` does not proxy instance attributes",
        explanation: "\
T0047 fires when `super().<attr>` names an *instance* attribute -- one \
established by a `self.<attr> = ...` assignment inside `__init__`, and so \
stored in the instance's own slot rather than on any class in the MRO \
(issue #587). A CPython `super` object proxies the class-level attributes \
and descriptors it finds along the MRO; it does not consult the instance's \
`__dict__`, so the same source raises `AttributeError: 'super' object has \
no attribute '<attr>'` under the pinned conformance oracle. pycc used to \
resolve this form against `self`'s own slot and return a value, which meant \
a program compiled and printed a different answer than CPython gave for the \
same expression; it is now rejected instead. Nothing is lost by the \
rejection: `super().<attr>` and a read through the method's own receiver \
read the identical slot, so the fix is always to spell the read \
`<receiver>.<attr>` -- whatever identifier that method spells its receiver \
with (#1181). The class-level members \
a `super` object genuinely does proxy are unaffected: a base class \
`@property` (a class-level descriptor) and a base class class attribute \
(#911), which is a real entry in that class's `__dict__` and folds to its \
literal. Because a `super` object never consults the instance `__dict__`, \
such a class attribute is found even when an unrelated base class \
establishes an instance attribute of the same name -- this code is reported \
only when no class-level member of that name exists anywhere after the \
current class in the MRO. A name declared nowhere in the MRO at all is \
`T0044`, not this code.",
        example: "\
class Vehicle:
    def __init__(self, wheels: int) -> None:
        self.wheels = wheels

class Car(Vehicle):
    def __init__(self, wheels: int) -> None:
        super().__init__(wheels)

    def base_wheels(self) -> int:
        return super().wheels  # T0047 -- write `self.wheels` instead
",
    },
    DiagnosticExplanation {
        code: "T0048",
        severity: Severity::Error,
        summary: "general union type annotation is not supported yet (PEP 604)",
        explanation: "\
T0048 fires when a `X | Y` annotation (PEP 604, D-197, issue #763, Part 1 of \
#747) does not match the one shape this compiler version accepts: exactly \
two operands where one side is literally `None` (`T | None` or \
`None | T`). Any other shape is rejected here -- a 2-operand union where \
neither side is `None` (`int | str`), or a longer chain (`A | B | None`). \
This is a versioned capability gap, not a rejected-by-design language rule: \
CPython accepts every one of these shapes at runtime. A recognized \
`T | None` shape whose inner type `T` this compiler does not yet support is \
`T0049` instead, a distinct, narrower gap.",
        example: "\
def f(x: int | str) -> None:  # T0048 -- neither side is None
    pass

def g(x: int | float | None) -> None:  # T0048 -- 3-operand chain
    pass

def h(x: int | None) -> None:  # OK -- T | None
    pass
",
    },
    DiagnosticExplanation {
        code: "T0049",
        severity: Severity::Error,
        summary: "`Optional[T]` is not supported yet for this `T` (PEP 604)",
        explanation: "\
T0049 fires when a `T | None` annotation matches the recognized 2-operand \
`Optional` shape (see `T0048`) but its inner type `T` is not one of the \
non-refcounted scalar types this compiler supports (D-197, issue #763, \
Part 1 of #747; widened from `int`-only to also accept `float` and `bool` \
by issue #809, Part 3 of #747). This compiler version's `Optional`/`T | \
None` codegen -- the `{ payload, present: i8 }` runtime representation -- \
is exercised and tested only for `Optional[int]`, `Optional[float]`, and \
`Optional[bool]`; every other inner type (`str`, `list[...]`, a class \
instance, or another `Optional[...]`) remains out of scope, mirroring the \
same versioned scope cuts `list[int]`-only (`T0034`) and `dict[str, \
int]`-only (`T0036`) already make. This is a versioned capability gap: a \
future compiler version extends `Optional`/`T | None` to more inner \
types.",
        example: "\
def f(x: str | None) -> None:  # T0049 -- str is not int/float/bool
    pass

def g(x: int | None) -> None:  # OK -- Optional[int]
    pass

def h(x: float | None) -> None:  # OK -- Optional[float]
    pass

def i(x: bool | None) -> None:  # OK -- Optional[bool]
    pass
",
    },
    DiagnosticExplanation {
        code: "T0050",
        severity: Severity::Error,
        summary: "walrus assignment (`:=`) value type is not supported yet (PEP 572)",
        explanation: "\
T0050 fires when a walrus assignment expression's (`target := value`, PEP \
572, issue #774) `value` operand does not have one of the non-reference-\
counted scalar types this compiler version supports for a walrus value: \
`int`, `float`, `bool`, `None`, or `Optional` of those. Codegen's \
`MirExpr::NamedExpr` yields the value it just stored by re-reading the \
slot, exactly like a plain `Name` read, but that read's refcount contract \
(\"the yielded value borrows the slot's reference\") is only worked out for \
`Ty::Int` so far; a reference-counted type such as `str`, `list`, `dict`, \
`set`, `tuple`, or a class instance would need the same duplicate-\
reference handling plain reads of those types already carry, and skipping \
it would silently reproduce the exact D-154 Part 1 use-after-free class \
already fixed once. This is a versioned capability gap, not a rejected-by-\
design language rule: CPython allows a walrus value of any type. A future \
compiler version extends walrus-value support to the remaining \
reference-counted types once their `MirExpr::NamedExpr` refcount \
contract is worked out.",
        example: "\
(n := \"hi\")  # T0050 -- str is not yet a supported walrus value

if (n := 5) > 0:  # OK -- int is a supported walrus value
    print(n)
",
    },
    DiagnosticExplanation {
        code: "T0051",
        severity: Severity::Error,
        summary: "int literal out of range for a runtime int-boundary position (D-061 tagged smallint)",
        explanation: "\
T0051 fires when an `int` literal written directly in one of the compiler's \
13 named runtime `int`-boundary positions -- a list/set/dict literal \
element or value, `list.append()`/`dict.get()` default/`set.add()`'s \
argument, a list index, a slice bound, a \
listcomp/setcomp element or dictcomp value, a subscript-assignment \
value (`name[key] = <literal>`), or a `str` repeat count -- is too large to fit D-061's 63-bit tagged \
smallint representation. D-178 (issue #148) made an out-of-range `int` \
literal materialize as a heap bigint at compile time everywhere else, but \
each of these 13 positions calls a runtime helper \
(`pycc_rt_int_untag_checked`) that only accepts a tagged smallint and \
aborts the whole program at run time (exit 101) if it ever receives a \
bigint. Before this diagnostic existed, `pycc check` could not see this \
coming: the literal type-checked fine, and the program only failed once \
run. T0051 restores `pycc check` as the catch point for the literal case, \
with a diagnostic spanning exactly the out-of-range literal. A `range()` \
argument is deliberately NOT one of these positions: D-179 already made \
`range()` fully bigint-capable in its bounds, step, and a mid-loop-promoting \
induction variable, so an out-of-range literal there is ordinary supported \
behavior, not a boundary failure. A bigint value \
reaching one of the 13 positions through *arithmetic* (not a literal) is \
unaffected and keeps its existing runtime-abort behavior -- narrowing that \
separate case is out of scope here (see issue #618). The `str`-repeat-count \
position is narrower still: it is recognized only when the string side of \
the multiplication is itself a string literal (`\"ab\" * <huge int>`); a \
`str`-typed *variable* multiplied by an oversized literal is a documented, \
out-of-scope gap because identifying it needs type information HIR \
lowering does not have.",
        example: "\
xs = [4611686018427387904]  # T0051 -- out of range for a list-literal element

xs = [1]
xs.append(4611686018427387904)  # T0051 -- out of range for `list.append()`'s value

xs = [1, 2, 3]
y = xs[4611686018427387904]  # T0051 -- out of range for a list index

s = \"ab\" * 4611686018427387904  # T0051 -- out of range for a `str` repeat count

for i in range(4611686018427387904):  # OK -- range() is bigint-capable (D-179), not a T0051 position
    print(i)

xs = [1, 2, 3]
y = xs[5]  # OK -- an ordinary in-range literal
",
    },
    DiagnosticExplanation {
        code: "T0052",
        severity: Severity::Error,
        summary: "cross-MRO attribute redeclaration with a differing declared type",
        explanation: "\
T0052 fires when two distinct classes in one class's own C3-linearized MRO \
each declare an instance attribute of the same name (via `self.<attr> = ...` \
in their own body) but with a different type. This is a class-definition-\
time rejection, not a coercion: pycc compiles each method body exactly \
once (static dispatch, no vtable, no per-subclass monomorphization -- \
see docs/TYPE_SYSTEM.md), so a base-class method's attribute assignment \
resolves the target slot's declared type once, from that base class's own \
declaration. A derived class redeclaring the same attribute with an \
incompatible type cannot be safely coerced at that one shared call site: \
pycc has no per-instance runtime type tag to distinguish, at run time, \
which declaration a given instance was actually built with. Left \
unrejected, this produced either a silently mis-decoded value (D-141's \
tagged representation reads back the wrong Python value) or an outright \
runtime abort/segfault -- entirely undiagnosed at `pycc check` time \
(issue #676, D-210). The rule is symmetric: it does not matter which \
class's declaration is 'wider', and it also rejects a diamond conflict \
between two sibling base classes that neither is the other's ancestor, \
through their common descendant's own MRO, even when that descendant \
never redeclares the attribute itself. An identical redeclaration (same \
type on every class that declares it) is unaffected, and so is an \
ordinary same-class assignment of an admissible narrower value (e.g. a \
`bool` value assigned into that same class's own `int`-declared \
attribute, D-187) -- T0052 only ever compares two distinct classes' own \
declared attribute types, never a single class's declared type against a \
mere assigned value.",
        example: "\
class Base:
    def __init__(self) -> None:
        self.v = 0  # declared int

class Derived(Base):
    def __init__(self) -> None:
        self.v = False  # T0052 -- redeclared bool, conflicts with Base's int

class B1:
    def __init__(self) -> None:
        self.v = 0  # declared int

class B2:
    def __init__(self) -> None:
        self.v = 0.0  # declared float

class D(B1, B2):  # T0052 -- diamond conflict via D's own MRO, even though
    pass          # D itself never redeclares `v`

class C:
    def __init__(self) -> None:
        self.n = 0  # declared int
    def set_true(self) -> None:
        self.n = True  # OK -- same class, bool widens into its own int slot (D-187)
",
    },
    DiagnosticExplanation {
        code: "T0053",
        severity: Severity::Error,
        summary: "container type annotation has the wrong number of type arguments",
        explanation: "\
T0053 fires when a parameterized container type annotation is written with \
an argument count pycc cannot lower: `list[T]` and `set[T]` take exactly \
one type argument, `dict[K, V]` exactly two, and `tuple[A, B, ...]` at \
least one. It is an arity check on the annotation itself, run before the \
element types are gated, so `list[int, str]` reports T0053 rather than a \
misleading element-type diagnostic. Two `tuple` spellings that are legal \
Python typing but outside this version's fixed-arity tuple representation \
(D-116) are also rejected here: the empty tuple `tuple[()]`, which arrives \
as a zero-element subscript, and the homogeneous-variadic `tuple[int, ...]`, \
whose length is not known at compile time. Both remain reserved for a later \
slice (D-228, issue #918); write the explicit fixed-arity form, e.g. \
`tuple[int, int]`, in the meantime. The `...` rejection is checked for every \
container family before arity is, so the ill-typed `list[...]` and \
`dict[str, ...]` spellings report an `...` message rather than an \
argument-count one -- with advice of their own, naming that family's element \
type (`list[int]`, `dict[str, int]`), since `...` is not a valid type \
argument there at all and a fixed-arity `tuple` would be a different \
container.",
        example: "\
def f(d: dict[str]) -> None:  # T0053 -- dict takes exactly 2 type arguments
    pass
",
    },
    DiagnosticExplanation {
        code: "T0054",
        severity: Severity::Error,
        summary: "set element is an instance of an unhashable user class",
        explanation: "\
T0054 fires when a set literal, `.add(...)` or set comprehension (#1344) \
inserts an instance of a user \
class that defines `__eq__` (itself or through a base class) but no \
`__hash__`. CPython sets such a class's `__hash__` to `None`, so the \
insertion raises `TypeError: unhashable type` at run time; pycc knows the \
class statically and reports the same error before the program runs. \
Define `__hash__` on the class that defines `__eq__`, or remove `__eq__` \
so instances hash by identity (#1343, D-255).",
        example: "\
class P:
    def __init__(self, x: int) -> None:
        self.x = x
    def __eq__(self, other: P) -> bool:
        return self.x == other.x

def f() -> int:
    s = {P(1)}  # T0054 -- P defines __eq__ without __hash__
    return len(s)
",
    },
    DiagnosticExplanation {
        code: "T0055",
        severity: Severity::Error,
        summary: "tuple-unpacking assignment names the wrong number of targets",
        explanation: "\
T0055 fires when a tuple-unpacking assignment (`a, b = t`, Part 1 of #891) \
unpacks a native `tuple[...]` whose length differs from the number of target \
names. CPython raises `ValueError` at run time with the same message; pycc \
knows the tuple's length statically, so it reports the error before the \
program runs, for the same reason an out-of-range literal tuple index is \
T0040 rather than a run-time `IndexError` (D-116). A `try`/`except \
ValueError` around such an unpack therefore never runs: the program is \
refused instead. Write exactly as many target names as the tuple has elements. A CPython object's length is not \
known statically: unpacking one in an `--ext` build is checked at run time \
and raises CPython's own `ValueError`.",
        example: "\
t = (1, 2, 3)
a, b = t  # T0055 -- too many values to unpack (expected 2, got 3)
",
    },
    DiagnosticExplanation {
        code: "O0201",
        severity: Severity::Error,
        summary: "value used after move across scope boundary",
        explanation: "\
O0201 is an internal-only diagnostic: legal Python source never triggers it \
in practice, because pycc's move-semantics optimization (uniqueness \
analysis, MEMORY_OWNERSHIP.md) is designed to be semantics-preserving -- \
every optimization it performs is provably safe for the program it was \
applied to. The code exists for the compiler's own internal assertions and \
`--emit mir` tooling output; if it is ever observed firing on real, legal \
user code, that is a pycc bug (auto-reported), not a defect in the user's \
program. There is currently no code path in this pycc version that emits \
O0201 at all -- the ownership/move-tracking pass it would guard has not \
landed yet.",
        example: "\
# O0201 has no user-triggerable source today -- it exists to guard pycc's
# own internal move-semantics invariants, not to reject Python source.
",
    },
    DiagnosticExplanation {
        code: "O0301",
        severity: Severity::Error,
        summary: "non-Shareable value crosses thread boundary without move",
        explanation: "\
O0301 is reserved for pycc's planned thread-safety ownership check \
(MEMORY_OWNERSHIP.md): a value crossing a thread boundary -- as a thread \
target argument, captured by a closure a thread runs, or as a \
`queue.Queue[T]` payload -- must either be `Shareable` (deeply immutable: \
`int`, `str`, a frozen dataclass, a tuple of `Shareable` values, ...) or be \
provably moved (the sender loses access, verified by uniqueness analysis); \
otherwise it is rejected with O0301. It is not currently emitted: pycc has \
no threading support in its frontend yet, so there is no thread-boundary \
crossing for this check to run against.",
        example: "\
# O0301 is reserved for pycc's not-yet-implemented threading support;
# no current pycc source can reach this check.
",
    },
    DiagnosticExplanation {
        code: "O0302",
        severity: Severity::Error,
        summary: "lock-guarded field accessed without holding lock",
        explanation: "\
O0302 is reserved for pycc's planned lock-discipline check \
(MEMORY_OWNERSHIP.md): shared mutable state guarded by a `threading.Lock` \
gets checked lock discipline, and an access to a lock-guarded field without \
provably holding the corresponding lock is rejected with O0302. It is not \
currently emitted: pycc has no threading or lock-discipline support in its \
frontend yet, so there is no guarded-field access for this check to run \
against.",
        example: "\
# O0302 is reserved for pycc's not-yet-implemented lock-discipline check;
# no current pycc source can reach this check.
",
    },
    DiagnosticExplanation {
        code: "E0100",
        severity: Severity::Error,
        summary: "`eval`/`exec`/`compile` not compilable",
        explanation: "\
E0100 is reserved for a call to `eval`, `exec`, or `compile` -- constructs \
that execute Python source discovered at runtime, which is fundamentally \
incompatible with pycc's ahead-of-time compilation model (there is no \
running interpreter to hand dynamically-constructed code to). It is not \
currently emitted: pycc has no dynamic-code rejection yet, so calls to \
these names fall through to the generic `C0001` \"not implemented yet\" \
path today instead.",
        example: "\
def f() -> None:
    eval(\"1 + 1\")
",
    },
    DiagnosticExplanation {
        code: "E0101",
        severity: Severity::Error,
        summary: "monkey-patching foreign class/module",
        explanation: "\
E0101 is reserved for code that reassigns an attribute on a class or module \
pycc did not itself define (monkey-patching) -- a pattern pycc's static, \
whole-program-unaware compilation model cannot support, since it assumes a \
type's method set is fixed at compile time. It is not currently emitted: \
this is part of the planned v0.7 dynamic-Python rejection surface, not yet \
implemented; pycc's frontend does not yet support class definitions at all, \
so there is nothing to monkey-patch today regardless.",
        example: "\
import math

math.sqrt = lambda x: x  # reassigning an attribute on a foreign module
",
    },
    DiagnosticExplanation {
        code: "E0102",
        severity: Severity::Error,
        summary: "dynamic attribute injection",
        explanation: "\
E0102 is reserved for code that adds a new attribute to an object outside \
its class's declared shape at runtime (e.g. via `setattr` with a \
non-literal name, or assigning to an attribute a `__slots__`-less class \
never declared) -- incompatible with pycc's fixed, compile-time-known \
object layout. It is not currently emitted: this is part of the planned \
v0.7 dynamic-Python rejection surface, not yet implemented; pycc has no \
class support yet, so there is no object layout for this check to guard.",
        example: "\
def f(obj: object) -> None:
    setattr(obj, some_dynamic_name, 1)
",
    },
    DiagnosticExplanation {
        code: "E0103",
        severity: Severity::Error,
        summary: "dynamic `type()` class creation",
        explanation: "\
E0103 is reserved for the three-argument `type(name, bases, namespace)` \
call form that constructs a new class object at runtime -- incompatible \
with pycc's ahead-of-time, statically-known class model. It is not \
currently emitted: this is part of the planned v0.7 dynamic-Python \
rejection surface, not yet implemented; pycc has no class support yet, so \
this check has nothing to guard today.",
        example: "\
Point = type(\"Point\", (), {})
",
    },
    DiagnosticExplanation {
        code: "E0104",
        severity: Severity::Error,
        summary: "wildcard import",
        explanation: "\
E0104 is reserved for a wildcard `from <module> import *` statement, which \
pulls an unbounded, not-statically-enumerable set of names into scope -- \
incompatible with pycc's requirement that every binding be resolvable at \
compile time. It is not currently emitted: a wildcard import is rejected \
today by the same versioned `C0001` \"not implemented yet\" capability code \
every other unsupported import shape uses, not this by-design-rejection \
code (D-137) -- E0104 is reserved for when wildcard imports gain a \
dedicated, deliberate rejection path distinct from \"we haven't implemented \
this import shape yet\".",
        example: "\
from math import *
",
    },
    DiagnosticExplanation {
        code: "E0105",
        severity: Severity::Error,
        summary: "metaclass with non-static side effects",
        explanation: "\
E0105 is reserved for a class using a custom `metaclass=` whose \
construction has runtime side effects pycc cannot evaluate ahead of time \
(as opposed to a purely structural metaclass pycc's compiler could resolve \
statically). It is not currently emitted: this is part of the planned v0.7 \
dynamic-Python rejection surface, not yet implemented; pycc has no class or \
metaclass support yet, so this check has nothing to guard today.",
        example: "\
class Meta(type):
    def __new__(mcs, name, bases, ns):
        print(\"class created\")  # a runtime side effect at class-creation time
        return super().__new__(mcs, name, bases, ns)

class C(metaclass=Meta):
    pass
",
    },
    DiagnosticExplanation {
        code: "E0106",
        severity: Severity::Warning,
        summary: "`__del__` relies on refcount timing",
        explanation: "\
E0106 is documented as `warning→error` in `docs/DIAGNOSTICS.md`: today's \
severity is `warning` (its current, initial documented state), and \
escalating it to a hard `error` is documented future intent, not yet \
implemented -- the `Severity` enum currently has only `Error`/`Warning`, no \
third \"will become an error\" state. It is reserved for a `__del__` method \
whose behavior depends on exactly when CPython's refcounting frees the \
object -- pycc's own refcounting model (with RC-elision optimizations, \
MEMORY_OWNERSHIP.md) does not guarantee identical collection timing to \
CPython, so `__del__` logic relying on precise timing is unreliable under \
pycc. It is not currently emitted by any compiler pass: pycc has no class \
support yet, so there is no `__del__` method for this check to inspect.",
        example: "\
class Resource:
    def __del__(self) -> None:
        print(\"freed now\")  # timing may differ from CPython's refcounting
",
    },
    DiagnosticExplanation {
        code: "E0107",
        severity: Severity::Error,
        summary: "`sys.getrefcount` unavailable",
        explanation: "\
E0107 is reserved for a call to `sys.getrefcount`, which exposes CPython's \
own internal reference count -- a value pycc's compiled output has no \
equivalent for once RC-elision optimizations remove some of the increments \
and decrements CPython would have performed (`id()` remains stable across \
moves and is supported; `getrefcount` is not). It is not currently \
emitted: this is part of the planned v0.7 dynamic-Python rejection surface, \
not yet implemented; `sys` is not even a registered stdlib module in \
pycc's frontend yet (D-136), so this call falls through to `C0001` today.",
        example: "\
import sys

def f(x: object) -> int:
    return sys.getrefcount(x)
",
    },
    DiagnosticExplanation {
        code: "E0108",
        severity: Severity::Error,
        summary: "import cycle in top-level init",
        explanation: "\
E0108 reports a cycle in module-level (top-level) initialization order \
reached through `import` statements -- two or more modules whose top-level \
code each depends on the other having already run. Since #898 (D-222) the \
driver loads a project's whole import closure, so it detects such a cycle \
while walking it and reports the chain (`import cycle: `a.py` -> `b.py` -> \
`a.py``) at the import statement that closes it. pycc links the closure \
into one program and runs every dependency's top level before its \
importer's, so a cycle has no valid order at all -- unlike CPython, which \
tolerates some cycles by binding a partially initialized module.",
        example: "\
# a.py
import b

# b.py
import a  # cycle: a's top level needs b, b's top level needs a
",
    },
    DiagnosticExplanation {
        code: "I0401",
        severity: Severity::Error,
        summary: "untyped value leaks across interop boundary",
        explanation: "\
I0401 is reserved for a value crossing the planned CPython interop \
boundary (v0.7) without a type pycc's static checker can verify on the \
other side -- e.g. a value returned from an embedded-CPython call whose \
runtime type is not pinned down by any annotation. It is not currently \
emitted: no check in this pycc version produces it.",
        example: "\
# I0401 is reserved for pycc's not-yet-implemented CPython interop
# boundary (v0.7); no current pycc source can reach this check.
",
    },
    DiagnosticExplanation {
        code: "I0402",
        severity: Severity::Error,
        summary: "CPython-backed direct import root rejected by the effective interop policy",
        explanation: "\
I0402 reports an `import` of a CPython-backed module (one pycc does not \
implement natively and does not resolve inside the project) whose root \
package the effective interop policy rejects (D-128). The policy is \
`--pure` or `--interop-policy auto|allowlist|deny` when given, else the \
`[interop]` table of the project's `pycc.toml`, else `auto`. `auto` admits \
every root, `deny` (also spelled `--pure`) admits none, and `allowlist` \
admits only the roots listed in `[interop] allow`. The message names the \
policy and where it was set. `pycc check`, `pycc build` and `pycc run` \
report it alike, on every host, and before any embedding refusal \
(`I0403`). `pycc build --ext` ignores the policy. Drop the import, list its \
root in `[interop] allow`, or choose a policy that admits it.",
        example: "\
# pycc check --pure app.py
import numpy  # error[I0402]: the `deny` interop policy (set by `--pure`) rejects it
",
    },
    DiagnosticExplanation {
        code: "I0403",
        severity: Severity::Error,
        summary: "CPython import this build cannot embed; `pycc build --ext` is required",
        explanation: "\
I0403 reports an `import` of a module pycc does not implement natively and \
does not resolve inside the project, in a plain `pycc build` or `pycc run` \
that cannot give it a CPython interpreter. It applies only to an import \
whose root the effective interop policy admits; a rejected one is `I0402` \
instead. Since Part 1 of #1026 such an \
import is compiled as a CPython import: the module object is acquired from \
the running interpreter at module-execution time. A `pycc build --ext` \
artifact always has one -- it is an extension module CPython itself loads. \
Since Part 1 of #1028 a plain build has one too: it produces an embedded \
executable that bundles CPython 3.14 in an `OUT.pycc/` directory beside it, \
and since #1242 it also bundles a module outside the standard library from \
the program's `pycc.lock` (a missing or stale lock fails the build with exit \
2, naming `pycc lock`). The remaining cases are refused, with the reason in \
the message: a standard-library module that needs Tcl/Tk (`tkinter`, \
`turtle`, `idlelib`), a `--target` build (the bundled interpreter is the \
build host's own). A Windows host bundles a module outside the standard \
library from `pycc.lock` too since #1296. Rebuild with `--ext`, or drop the \
import.",
        example: "\
import json     # fine under `pycc build`: builds an embedded executable
import tkinter  # error[I0403] under `pycc build`; fine under `pycc build --ext`

def scale(x: float) -> float:
    return x * 2.0
",
    },
    DiagnosticExplanation {
        code: "I0404",
        severity: Severity::Error,
        summary: "operation on a CPython object is not supported yet",
        explanation: "\
I0404 reports an unsupported operation on a value whose type is the opaque \
CPython object type `object` -- a module bound by a CPython `import` under \
`--ext`, or an attribute loaded from one. Reading such a value is not \
itself an error, and #1026 implements nine operations on it: loading a \
further attribute, calling a method with positional \
`int`/`float`/`bool`/`str` arguments, calling the object itself with \
such arguments (#1313), `len`, using it as an \
`if`/`while` condition or comprehension guard, *loading* a subscript \
`o[k]` whose key is an `int`, `float`, `bool` or `str`, iterating it \
with `for`, binding it to a module-level name (#1325), and printing it \
or interpolating it into an f-string (#1340), which renders it with \
CPython's own `str()` or `format()`. Part 1 of #1371 adds comparisons: \
a rich comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`) with the object on \
either side and an `object`, `int`, `float`, `bool` or `str` value on the \
other evaluates CPython's own `PyObject_RichCompare` and yields an \
`object` result usable as a condition or printed; an identity test \
(`is`, `is not`) compares the object with another object or `None`; and \
`isinstance(o, C)` is admitted when `C` is `int`, `float`, `bool`, `str` \
or itself a CPython object; since Part 7 of #1371 also when `C` is \
`list`, `dict` or `tuple`, or a plain class compiled in the same module, \
which is true of the host-side instances of its published family (an \
`--ext` class with an exported method and its published subclasses). \
Part 2a of #1371 lets a second CPython \
object be a method or direct call's argument or a subscript key, and \
admits a call of a subscript result (`callbacks[k](tok)`) under the same \
argument rule; the result is another CPython object. Part 8 of #1371 \
lets each of those three call shapes take keyword arguments \
(`o.split(\",\", maxsplit=1)`, `sorted(xs, reverse=True)`) under the same \
argument rule, and admits `None` as a positional or keyword argument; \
`**` unpacking stays refused by `C0001`. Part 2b of #1371 \
admits a membership test (`k in o`, `k not in o`) of an `int`, `float`, \
`bool`, `str` or `object` item in the object, evaluated by CPython's own \
`PySequence_Contains` with a `bool` result, and a slice load \
(`o[a:b:c]`, any bound omitted) whose bounds are of those same types, \
answered by the object's own `__getitem__` with a `slice` key; the result \
is another CPython object. Part 2d of #1371 builds a list display \
bound to an object slot -- annotated `object`, `Any` or a bare `list`, \
or an empty `[]` assigned to a name whose other binding is a CPython \
object -- as a fresh CPython `list` of its `int`, `float`, `bool`, `str` \
or `object` elements. Part 6 of #1371 admits `and`/`or` with an \
object operand: as a condition the object's truth is `PyObject_IsTrue`, \
and as a value the result is an `object` when the other operand is an \
`object`, `int`, `float`, `bool` or `str`, a native operand boxed only \
when it is the one selected. Part 10 of #1371 admits `not o`: the \
object's `PyObject_IsTrue`, negated into a native `bool`. Part 11 of \
#1371 admits `type(o)` with one object argument: CPython's \
`PyObject_Type`, the object's class as another CPython object. #1435 lets an instance of a regular pycc \
class (not an enum or an exception class) be a positional or keyword \
argument of a method call, a direct call or a call of a subscript \
result, crossing as a carrier of its run-time class. The loop is \
admitted when the iterable is written as an attribute load \
(`for x in o.attr:`), a method call (`for x in o.method(...):`) or a bare \
name bound to such a value (`x = product(\"ab\")`, then `for t in x:`); \
a bare imported module is iterated too, and raises CPython's own \
`TypeError` at run time. Everything else is \
still refused, including `isinstance` against a tuple of classes or a \
pycc exception class, protocol, enum or generic class, a comparison chain with an object link (`a < o < b`, `o is p is q`), any other `type(...)` shape, an \
equality test against a bare `None`, a rich comparison with an operand \
of any other type, a `match` subject, iterating over a subscript load \
(`for x in o[k]:`) or inside a comprehension, \
passing an argument of any other type (an enum member, an exception \
instance, a class method's `cls`) to one of its methods, to the object \
itself or to a subscript result, indexing or slicing with a key \
or bound of any other type, joining an object with a value of any other \
type in an `and`/`or`, and testing membership of an item of any \
other type in an object or of an `object` item in a native container \
(membership between two native values keeps its `C0001`), and a list \
display bound to an object slot with an element of any other type. A subscript call whose base is a bare name and whose key is \
`int`, `float`, `bool` or `str` (`handlers[int](x)`) is still read as a \
generic class instantiation and refused by that path's own diagnostic. Storing through a \
subscript (`o[k] = v`) or a slice (`o[a:b] = v`) and \
iterating a direct call's result (`for x in o(...):`) are still refused \
too, but by their own pre-existing `C0001` diagnostics rather than by \
this code; deleting a slice of an object (`del o[a:b]`) is admitted \
since Part 2c of #1371, with the same bound rule as a slice load. Since \
#1457 an attribute store on an object (`o.x = v`) is admitted with a value \
of any type a method-call argument may have, and this code refuses a \
value of any other type; `del o.x` is admitted on an object, while on any \
other base it keeps a `C0001` located at the target. Since \
#1095 a method named `append`, `pop`, `get` or `add` is \
an ordinary method call on the object too, whatever its arity \
(`o.get(k)`, `o.attr.append(v)`): container lowering claims those four \
spellings, but in a module that can hold an object it keeps the method \
reading beside its own, and an object receiver takes that one; since \
#1425 that includes a module importing an object from a sibling project \
module (`from dep import g`), for its whole body, and since #1482 a \
module's own foreign import admits this method reading for its whole body \
too, so a function defined above `import gc` and called after the import \
has run may call `gc.garbage.append(1)`. A module-level *statement* that \
uses the object is still admitted only *below the import*. Since #1316 and Part 1 of #1333 every \
supported operation except the `for` loop is also admitted inside a function body, and a \
function may bind the object to a local name, return it and pass it to \
another pycc function: a call that runs before a module-level binding has \
bound the name raises `NameError` at run time, and a failing operation \
raises a pycc exception the function can catch. Inside a function body \
this code still refuses iterating the object with `for` (Part 2 of \
#1333) and passing it to a generic function. An unannotated helper whose \
parameter would be inferred as the object and then used as a method-call \
receiver or called reports `T0021` instead (Part 3 of #1333). The \
refusal narrows as the later parts of #1026 (the boundary conversions) and \
of #1371 (calls, subscripts and `raise` on an object, comparison chains \
over an object) land, and \
this code is retired when they have.",
        example: "\
import numpy

match numpy.pi:  # error[I0404]: matching on a CPython object
    case 1:
        pass
",
    },
    DiagnosticExplanation {
        code: "I0405",
        severity: Severity::Error,
        summary: "a buffer type in a signature in native mode; `pycc build --ext` is required",
        explanation: "\
I0405 reports a function whose signature names the buffer type in a build \
that produces a standalone native executable. Any of the three source \
spellings reaches it: `memoryview`, since #1129 the bare name `ndarray`, and since #1134 the \
bare name `NDArray`, which lower to one type and are rendered back with the \
canonical `memoryview` in the message itself. A buffer parameter is a borrowed view of storage a CPython \
interpreter owns: the value exists only because the generated \
`pycc build --ext` wrapper acquired a `Py_buffer` from a real buffer \
exporter before the call and released it again afterwards. A \
native artifact embeds no interpreter, so there is nothing to acquire a \
buffer from and no way such a function could ever be called. Rebuild with \
`pycc build --ext`, or change the annotation to a type a native artifact \
can carry. Under `--ext` both signature positions are admitted, but not \
symmetrically. A parameter is admitted anywhere in the export set. A return \
is admitted from any *public export* -- a module-level `def` and, since \
#1174, a public method of a public class -- in any of three \
provenances. The generated wrapper turns storage the artifact allocated \
with `a = ndarray(n)` into a real `memoryview` the host owns (Part 2b of \
#1142, #1164), and, since Part 1 of #1175 (#1178), returns the bare name a \
buffer *parameter* binds by acquiring a second, independent export on the \
host's own argument object before releasing its own -- so that view spans \
storage the caller owns and the artifact allocates and frees nothing. \
Part 2 of #1175 (#1179) adds the third: a step-free *sub-range* of a \
parameter, `return b[start:stop]`, whose bounds leave the callee frame in \
three out-slots so the wrapper still checks the whole window and CPython \
itself derives the sub-view. A `step` on that slice stays refused, because \
the artifact's view type carries no stride. The same return type \
outside the export set -- a private function, a private method, a method \
of a private or exception class, an instance method no published \
constructible class reaches, a `@property` getter, a specialization -- is \
the `C0001` capability gap, as is an intra-artifact *call* to a buffer-returning \
function or method, which has no wrapper to own the result. A public \
export is refused too when the function contains a \
`return` anywhere inside a `finally` clause, because the compiled frame \
tracks one pending buffer return per call and a suspended outer return \
would outlive the record (#1173). A \
signature is the only position the type is \
admitted at in either mode: a `memoryview` *declaration* (`x: memoryview`, \
at module scope or in a function body) is a `C0001` capability gap in both, \
because the annotation alone produces no value to bind to the name. Since \
Part 2a of #1142 (#1165) a function *body* reaches this same code without any \
buffer in its signature: `a = ndarray(n)` allocates artifact-owned buffer \
storage, which a native artifact would actually run -- the allocation is a \
plain heap allocation that links natively -- so its refusal states the \
boundary rather than the missing interpreter, and `pycc build --ext` is \
again the fix.",
        example: "\
def total(xs: memoryview) -> float:  # error[I0405]: requires `pycc build --ext`
    return 0.0

def mean(xs: ndarray) -> float:      # error[I0405]: another spelling, same code
    return 0.0

def sum(xs: NDArray) -> float:       # error[I0405]: the third spelling, same code
    return 0.0

def zeros(n: int) -> float:
    a = ndarray(n)                   # error[I0405]: allocating buffer storage natively
    return a[0]
",
    },
    DiagnosticExplanation {
        code: "W1001",
        severity: Severity::Warning,
        summary: "unreachable code",
        explanation: "\
W1001 is reserved for a statement pycc's compiler can prove will never \
execute -- code after an unconditional `return`/`raise` in the same block, \
for example. It is not currently emitted by any compiler pass: pycc has no \
reachability/control-flow analysis wired up to detect and report this yet. \
(It does appear as convenient sample data in `pycc_diag`'s own unit tests \
for the generic `render_human`/`render_json` renderers, but those tests \
exercise the renderer's \"a warning with no span\" code path in the \
abstract -- they are not evidence of a real unreachable-code detection \
pass.)",
        example: "\
def f() -> int:
    return 1
    print(\"this line can never run\")
",
    },
    DiagnosticExplanation {
        code: "W1002",
        severity: Severity::Warning,
        summary: "boxed fallback in hot loop (`--memstats` hint)",
        explanation: "\
W1002 is reserved for `pycc build --memstats`'s planned ownership/\
allocation report (MEMORY_OWNERSHIP.md): a hint that a value inside a hot \
loop fell back to a boxed (heap-allocated, refcounted) representation \
where an unboxed one might have been possible. It is not currently \
emitted: `--memstats` itself is planned v0.4 work (escape analysis, move \
semantics, RC elision) and is not implemented in this pycc version yet.",
        example: "\
# W1002 is reserved for pycc's planned --memstats ownership report (v0.4);
# no current pycc source can reach this check.
",
    },
];

/// Looks up a diagnostic code's long-form explanation by exact,
/// case-sensitive string match -- matching every existing diagnostic
/// code's own always-uppercase convention, so no case-insensitive
/// normalization is performed.
pub fn find(code: &str) -> Option<&'static DiagnosticExplanation> {
    EXPLANATIONS.iter().find(|entry| entry.code == code)
}

fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// `pycc explain`'s human-readable output: a header line of the shape
/// `code (severity): summary`, a blank line, the long-form explanation
/// paragraph, a blank line, and an indented `Example:` block. See
/// `docs/CLI_SPEC.md`'s "`pycc explain` output contract" section for the
/// full contract this reproduces.
pub fn render_explanation_human(entry: &DiagnosticExplanation) -> String {
    // Every `EXPLANATIONS` example is a short, non-blank-line snippet (see
    // `completeness_test_covers_every_registered_code_and_severity`'s
    // sibling tests for the full table), so this deliberately does not
    // special-case an empty line to skip its indentation -- that branch
    // would have no real content to exercise it under D-014's coverage
    // gate, and a blank line would just gain trailing spaces instead,
    // which is harmless for `Example:` block output.
    let mut indented_example = String::new();
    for line in entry.example.lines() {
        indented_example.push_str("    ");
        indented_example.push_str(line);
        indented_example.push('\n');
    }
    format!(
        "{} ({}): {}\n\n{}\n\nExample:\n{}",
        entry.code,
        severity_word(entry.severity),
        entry.summary,
        entry.explanation,
        indented_example
    )
}

/// `pycc explain <code> --format json`'s output: `format_version: 1`,
/// `kind: "diagnostic_explanation"` (the disambiguator against `pycc
/// check`'s own, differently-shaped `format_version: 1` diagnostic-array
/// JSON -- see `docs/CLI_SPEC.md`), plus `code`/`severity`/`summary`/
/// `explanation`/`example`.
pub fn render_explanation_json(entry: &DiagnosticExplanation) -> String {
    let value = serde_json::json!({
        "format_version": 1,
        "kind": "diagnostic_explanation",
        "code": entry.code,
        "severity": severity_word(entry.severity),
        "summary": entry.summary,
        "explanation": entry.explanation,
        "example": entry.example,
    });
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn find_returns_some_for_a_known_code() {
        let entry = find("T0001").expect("T0001 is registered");
        assert_eq!(entry.code, "T0001");
        assert_eq!(entry.severity, Severity::Error);
    }

    #[test]
    fn find_returns_none_for_an_unknown_code() {
        assert!(find("X9999").is_none());
    }

    #[test]
    fn render_explanation_human_matches_the_documented_shape() {
        let entry = find("T0001").expect("T0001 is registered");
        let rendered = render_explanation_human(entry);
        assert!(rendered.starts_with("T0001 (error): public function missing annotation\n\n"));
        assert!(rendered.contains("\n\nExample:\n    def add(a: int, b: int) -> int:\n"));
        assert!(rendered.ends_with("        return a + b\n"));
    }

    #[test]
    fn render_explanation_json_matches_the_documented_schema() {
        let entry = find("E0106").expect("E0106 is registered");
        let rendered = render_explanation_json(entry);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["format_version"], 1);
        assert_eq!(parsed["kind"], "diagnostic_explanation");
        assert_eq!(parsed["code"], "E0106");
        assert_eq!(parsed["severity"], "warning");
        assert_eq!(parsed["summary"], "`__del__` relies on refcount timing");
        assert!(
            parsed["explanation"]
                .as_str()
                .unwrap()
                .contains("warning→error")
        );
        assert!(parsed["example"].as_str().unwrap().contains("__del__"));
    }

    /// Drift guard (D-150): parses `docs/DIAGNOSTICS.md`'s "Initial
    /// registry" table at test time (never at runtime -- see this module's
    /// own doc comment) and asserts, both directions, that `EXPLANATIONS`'s
    /// code set exactly matches the documented registry's code set, and
    /// that every shared code's severity matches too. `.unwrap()`/
    /// `.expect()` are used freely here on this repository's own
    /// known-good file (matching `tests/slice0.rs`'s existing precedent of
    /// `.unwrap()`-ing `serde_json::from_str` inside a test body) rather
    /// than factored into a fallible helper -- there is deliberately no
    /// error-return code path here for D-014's coverage gate to require a
    /// fixture for.
    #[test]
    fn completeness_test_covers_every_registered_code_and_severity() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let diagnostics_md =
            std::fs::read_to_string(format!("{manifest_dir}/../../docs/DIAGNOSTICS.md"))
                .expect("docs/DIAGNOSTICS.md must be readable from pycc_diag's own manifest dir");

        // Registry table rows look like:
        // | `T0001` | error | public function missing annotation |
        // Only scan the "## Initial registry" section: the "## Code
        // ranges" table above it also has a backtick-quoted first cell
        // (e.g. `| `C0xxx` | ... |`), so a plain "starts with a
        // backtick-quoted cell" check alone would wrongly pick up its
        // range rows too. Scope by section heading instead, then extract
        // the code cell and the severity cell for every row in it.
        let registry_section = diagnostics_md
            .split("## Initial registry")
            .nth(1)
            .expect("docs/DIAGNOSTICS.md must have an \"## Initial registry\" section")
            .split("\n## ")
            .next()
            .expect("split always yields at least one piece");

        let mut documented: HashSet<(String, String)> = HashSet::new();
        for line in registry_section.lines() {
            let line = line.trim();
            if !line.starts_with("| `") {
                continue;
            }
            let mut cells = line.split('|').map(str::trim).filter(|c| !c.is_empty());
            let code_cell = cells.next().expect("a registry row has a code cell");
            let severity_cell = cells.next().expect("a registry row has a severity cell");
            let code = code_cell.trim_matches('`').to_string();
            // Normalize "warning→error" (E0106's escalation-intent
            // annotation) to its leading word before "→", generically --
            // any future escalating-severity code needs no special case
            // here, only this same normalization.
            let severity = severity_cell
                .split('→')
                .next()
                .expect("split always yields at least one piece")
                .trim()
                .to_string();
            documented.insert((code, severity));
        }

        let documented_codes: HashSet<&str> =
            documented.iter().map(|(code, _)| code.as_str()).collect();
        let explained_codes: HashSet<&str> = EXPLANATIONS.iter().map(|entry| entry.code).collect();
        assert_eq!(
            documented_codes, explained_codes,
            "EXPLANATIONS's code set must exactly match docs/DIAGNOSTICS.md's registry table"
        );

        for entry in EXPLANATIONS {
            let expected_severity = documented
                .iter()
                .find(|(code, _)| code == entry.code)
                .map(|(_, severity)| severity.as_str())
                .expect("every EXPLANATIONS code was just confirmed to be documented");
            assert_eq!(
                severity_word(entry.severity),
                expected_severity,
                "{}'s EXPLANATIONS severity must match docs/DIAGNOSTICS.md (after \
                 warning→error normalization)",
                entry.code
            );
        }
    }
}
