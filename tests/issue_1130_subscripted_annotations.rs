//! #1130: a subscripted type annotation whose base resolves to a type pycc
//! can name nominally is accepted, with the type argument erased.
//!
//! The rule, stated once and derived from throughout this file:
//!
//! > In **annotation** position, a subscript whose base resolves to a type
//! > pycc can name nominally -- a class in `class_defs`, a `type` alias to
//! > one, or the buffer carrier (`Ty::MemoryView`, any spelling, directly
//! > or through an alias) -- is accepted, with the type argument **erased
//! > without being lowered**. Inside a class's own body the enclosing class's
//! > own name is answered before the six reserved names, so its subscripted
//! > form resolves to that class and is accepted too. A base that resolves to
//! > something that is not a nameable type -- a PEP 695 type parameter,
//! > `Self`, a builtin scalar, or an alias to a non-class `Ty` -- keeps
//! > #931's `T0044`. An undefined base keeps `C0001`; `Any[...]` keeps
//! > `T0002`; a non-bare-name base keeps #889's rejection. **Value position
//! > is unchanged.**
//!
//! Why accepting is right at all: CPython 3.14 evaluates annotations lazily
//! (PEP 649/749), so `class C: pass` + `def f(a: C[int])` raises no
//! `TypeError` at definition time -- that error surfaces only through
//! `typing.get_type_hints`, and pycc builds no runtime `__annotations__`
//! object for any function, class or module, so no pycc-compiled program can
//! reach it. Value position is the opposite: `x = C[int]` is evaluated
//! eagerly and really does raise, so `pycc_types`' value-position path (#610)
//! keeps its own `T0044`. That asymmetry is CPython's own.
//!
//! Scope: this issue admits the subscripted *form*. It does not make any of
//! the #1039 census's 19 array-parameter occurrences compile -- every one of
//! them also needs a name binding that does not exist yet: attribute bases
//! are #889, `import numpy as np` is #883, and `from numpy.typing import ...`
//! was refused by the foreign-import path until #1380 admitted
//! `from numpy.typing import NDArray` as a buffer-carrier import. (`NDArray` itself was unregistered
//! when this file was written; #1134 has since registered it as a third
//! source spelling of the buffer carrier, which is why the undefined-base
//! regression guard below now names `Nonexistent` instead.)
//!
//! Every test below resolves its answer on the program before `plan_ext`
//! probes the host toolchain, so none is `#[ignore]`d -- except the single
//! hosted arm at the end, which builds an artifact and loads it into a live
//! interpreter, following `tests/issue_1129_ndarray_buffer_carrier.rs`'s own
//! convention for that one shape.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `source` as the entry module of a fresh scratch directory.
fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("sub_probe.py"), source).expect("write the subject");
    dir
}

/// `pycc check` on the fixture's entry module. `check` renders diagnostics on
/// stdout where `build` renders them on stderr, so both streams are read and
/// concatenated rather than guessing which one a subcommand uses.
fn check(dir: &Path) -> (bool, String) {
    let out = pycc()
        .arg("check")
        .arg(dir.join("sub_probe.py"))
        .output()
        .expect("pycc should spawn");
    (
        out.status.success(),
        format!("{}{}", stdout_of(&out), stderr_of(&out)),
    )
}

fn assert_accepts(category: &str, source: &str) {
    let dir = fixture(category, source);
    let (ok, text) = check(&dir);
    assert!(ok, "expected `{source}` to compile, got: {text}");
}

fn assert_rejects(category: &str, source: &str, code: &str) -> String {
    let dir = fixture(category, source);
    let (ok, text) = check(&dir);
    assert!(!ok, "expected `{source}` to be rejected, got: {text}");
    assert!(text.contains(&format!("error[{code}]")), "{text}");
    text
}

/// A class whose only distinguishing feature is a method, so an acceptance
/// can be pinned to *that* class rather than to "something compiled".
const NOMINAL: &str = "\
class NDArray:
    def __init__(self) -> None:
        self.v = 1

    def m(self) -> int:
        return self.v

";

// ---------------------------------------------------------------------------
// T5: the issue's own reproducer.
// ---------------------------------------------------------------------------

#[test]
fn the_issue_s_reproducer_compiles_and_names_the_class() {
    // Verbatim from the issue body, which is the one shape that reaches
    // `T0044` today: a base that resolves to a *known class*. (The issue's
    // title example, an undefined `NDArray`, is `C0001` and is unchanged by
    // this issue -- an undefined base still keeps its cascade-shaped
    // `C0001`, asserted in the regression guard below.)
    assert_accepts(
        "1130_reproducer",
        "class NDArray:\n    pass\n\n\ndef f(a: NDArray[int], n: int) -> float:\n    return 0.0\n",
    );
    // ...and the parameter really is that class, not merely "something that
    // compiled": a member read only `NDArray` supports type-checks.
    assert_accepts(
        "1130_reproducer_member",
        &format!("{NOMINAL}\ndef f(a: NDArray[int]) -> int:\n    return a.m()\n"),
    );
}

// ---------------------------------------------------------------------------
// T6: the type argument is erased without being lowered.
// ---------------------------------------------------------------------------

#[test]
fn the_type_argument_is_erased_without_being_lowered() {
    // The accepting path never lowers the argument, so an argument pycc
    // could not lower at all is accepted: a nested subscript on an attribute
    // base (#889's territory), and an undefined name. This is the property
    // that makes the design usable at all -- 13 of the #1039 census's 19
    // array-parameter occurrences carry `np.floating[Any]`, `np.floating` or
    // `np.bool_` as the argument, every one an attribute expression #889
    // blocks. A design that modelled the argument would be dead on arrival.
    for (category, argument) in [
        ("1130_erase_nested", "np.floating[Any]"),
        ("1130_erase_undefined", "Nonexistent"),
    ] {
        assert_accepts(
            category,
            &format!(
                "class NDArray:\n    pass\n\n\ndef f(a: NDArray[{argument}]) -> float:\n    return 0.0\n"
            ),
        );
    }
    // The contrast that bounds the claim: the D-228 container path *does*
    // lower its arguments and is untouched here, so the same attribute
    // expression inside `list[...]` keeps #889's `C0001`.
    let text = assert_rejects(
        "1130_container_argument_unchanged",
        "def f(a: list[np.floating]) -> float:\n    return 0.0\n",
        "C0001",
    );
    assert!(
        text.contains("only a bare name type annotation is supported so far"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// T7: the precedence ladder. `annotation_to_ty`'s `Expr::Name` arm answers a
// bare name in five levels: a PEP 695 type parameter, `Self`, the enclosing
// class's own name, the six reserved names (`int`/`float`/`bool`/`str`/
// `Any`/`memoryview`), then `class_defs` and the alias table. The subscript
// arm's direct `class_defs` lookup now agrees with that ladder.
//
// Every case is written in *parameter* position on purpose. Module scope is
// not parameter position in either direction: `x: memoryview = 1` is a
// `C0001` the parameter form is not, and `class bool[T]: pass` +
// `x: bool[int] = 1` fails on the *assignment* (`T0025`) rather than on the
// annotation. Both traps disappear in parameter position.
// ---------------------------------------------------------------------------

#[test]
fn level_3_the_enclosing_class_s_own_name_still_wins_inside_its_own_body() {
    // The `class_name` carve-out in the gate, and this is its only guard.
    // Measured on the branch point: both programs exit 0 today, and they
    // must keep doing so. Without `Some(base) != class_name` in the gate,
    // the subscripted one flips accept -> `T0044`, because the enclosing
    // class's own name is one of the six reserved names and the gate would
    // then skip the `class_defs` lookup that the self-referential entry
    // `lower_class` pushes exists to serve.
    assert_accepts(
        "1130_level3_subscripted",
        "class memoryview[T]:\n    def m(self, x: memoryview[int]) -> int:\n        return 1\n",
    );
    // T7 assertion 3: the bare and subscripted forms are pinned as agreeing.
    assert_accepts(
        "1130_level3_bare",
        "class memoryview:\n    def m(self, x: memoryview) -> int:\n        return 1\n",
    );
}

#[test]
fn level_4_a_reserved_name_beats_a_shadow_class_in_a_subscript() {
    // The behavior change edit (a) exists for, and the one accept-to-reject
    // class this issue introduces. Before it, deleting the subscriptability
    // rejection would have made `class int[T]: pass` + `int[str]` resolve to
    // `Ty::Int` -- a builtin scalar -- with the type argument silently
    // discarded, which is exactly the hole #931 closed.
    //
    // Accept-to-reject is *not* the whole of this ladder, and this test is
    // not the whole of the reserved-name story: the `memoryview` arm of the
    // same six names goes the other way (reject -> accept, resolving to the
    // buffer carrier), and `ndarray` -- not a reserved name at all -- flips
    // reject -> accept resolving to the user's own class. Both are pinned in
    // `level_4_the_carrier_spellings_flip_the_other_way_in_the_same_ladder`.
    //
    // The shape matters: `class int: pass` (plain, hook-less) is *already*
    // `T0044` on the branch point, so it would pass identically before and
    // after and pin nothing. A PEP 695 generic shadow class exits 0 today
    // and is the actual flip. Measured on the branch point: both scalar
    // programs below exit 0, and the `Any` one does too.
    for scalar in ["int", "str"] {
        let text = assert_rejects(
            "1130_level4_scalar",
            &format!(
                "class {scalar}[T]:\n    pass\n\n\ndef f(a: {scalar}[int]) -> float:\n    return 0.0\n"
            ),
            "T0044",
        );
        assert!(
            text.contains(&format!("builtin type `{scalar}` is not subscriptable")),
            "{text}"
        );
    }
    // `Any` moves to `T0002` rather than `T0044`, in every shape. Asserted
    // on the *code* only: that diagnostic is built with a zero span, so a
    // rendered-caret or snapshot assertion would not say what it means.
    // A `@classmethod` hook is used because the plain shape is already
    // `T0044` today; its declared return type is deliberately not the
    // shadowed name, which would make the hook's own signature a `T0022`.
    assert_rejects(
        "1130_level4_any",
        "class Any:\n    @classmethod\n    def __class_getitem__(cls, item: int) -> float:\n        return 0.0\n\n\ndef f(a: Any[str]) -> float:\n    return 0.0\n",
        "T0002",
    );
}

#[test]
fn level_4_the_carrier_spellings_flip_the_other_way_in_the_same_ladder() {
    // The reject-to-accept half of the reserved-name ladder, which the test
    // above deliberately does not cover. Both cells are measured against the
    // branch point (`4d5a9677`), where each was
    // ``error[T0044]: class `<name>` does not define `__class_getitem__` ``.
    //
    // `memoryview` is one of the six names `name_resolves_before_class_defs`
    // answers before the class table, so a plain hook-less shadow class does
    // not win: the subscript resolves to the buffer carrier. Proven by use
    // rather than by exit code -- an accept alone cannot tell the carrier
    // from the shadow class -- with the carrier's own `--ext`-parameter
    // diagnostic, which the user's class could never produce.
    assert_accepts(
        "1130_level4_memoryview_shadow",
        "class memoryview:\n    pass\n\n\ndef f(a: memoryview[float]) -> float:\n    return 1.0\n",
    );
    let text = assert_rejects(
        "1130_level4_memoryview_shadow_use",
        "class memoryview:\n    def m(self) -> int:\n        return 1\n\n\ndef f(a: memoryview[float]) -> int:\n    return a.m()\n",
        "C0001",
    );
    assert!(text.contains("bound to a buffer parameter"), "{text}");
    // The bare form already resolved to the carrier on the branch point, so
    // the subscripted form is not taking on a new meaning -- it is only
    // ceasing to disagree with the bare one.
    let bare = assert_rejects(
        "1130_level4_memoryview_shadow_bare",
        "class memoryview:\n    def m(self) -> int:\n        return 1\n\n\ndef f(a: memoryview) -> int:\n    return a.m()\n",
        "C0001",
    );
    assert!(bare.contains("bound to a buffer parameter"), "{bare}");
    // `ndarray`, one of the carrier's sibling spellings (#1129, joined by
    // #1134's `NDArray`), is an ordinary identifier resolved *after* the
    // class table, so the same shape flips reject -> accept, resolving to
    // the user's own class instead. `a.m()` type-checking is what proves
    // the class won rather than the carrier.
    assert_accepts(
        "1130_level4_ndarray_shadow_use",
        "class ndarray:\n    def m(self) -> int:\n        return 1\n\n\ndef f(a: ndarray[float]) -> int:\n    return a.m()\n",
    );
}

// ---------------------------------------------------------------------------
// T8: the buffer carrier, keyed on the resolved type rather than the spelling.
// ---------------------------------------------------------------------------

/// The `--ext` subject, at a subscripted parameter.
const SUBJECT: &str = "\
def total(b: ndarray[float]) -> float:
    s: float = 0.0
    i: int = 0
    for i in range(len(b)):
        s = s + b[i]
    return s
";

#[test]
fn every_way_of_naming_the_buffer_carrier_is_subscriptable() {
    // Every registered spelling, and an alias to one. The accept is keyed
    // on the resolved `Ty::MemoryView` rather than on the spelling, so a
    // further spelling is admitted by its registration alone -- and the
    // alias form, which a name-keyed arm could not have admitted without
    // being edited again, works for free. `NDArray` is the witness that the
    // claim was true rather than merely plausible: #1134 registered it in
    // `annotation_to_ty`'s `Expr::Name` arm alone and this arm admitted its
    // subscripted form with no edit. Each is proved by the element read:
    // only the carrier yields `float` from `b[0]`.
    for (category, spelling, prelude) in [
        ("1130_carrier_memoryview", "memoryview", ""),
        ("1130_carrier_ndarray", "ndarray", ""),
        ("1130_carrier_ndarray_capitalized", "NDArray", ""),
        ("1130_carrier_alias", "Arr", "type Arr = memoryview\n\n\n"),
    ] {
        assert_accepts(
            category,
            &format!("{prelude}def f(b: {spelling}[float]) -> float:\n    return b[0]\n"),
        );
    }
}

#[test]
fn a_nameable_type_that_is_not_the_carrier_keeps_its_rejection() {
    // The accept is keyed on `Ty::MemoryView` only, not on "any nominal
    // type": `Self[int]` resolves to `Ty::Instance` through the very same
    // recursion and must keep #931's rejection.
    let text = assert_rejects(
        "1130_carrier_not_self",
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self, x: Self[int]) -> int:\n        return 1\n",
        "T0044",
    );
    assert!(text.contains("`Self` is not subscriptable"), "{text}");
}

// ---------------------------------------------------------------------------
// T9: a protocol base.
// ---------------------------------------------------------------------------

#[test]
fn a_subscripted_protocol_is_a_protocol_in_parameter_position_and_still_refused_in_return_position()
{
    const PROTOCOL: &str = "\
from typing import Protocol


class P(Protocol):
    def m(self) -> int: ...

";
    // `P[int]` resolves to `Ty::Protocol("P")`, so a protocol-typed
    // parameter is what it becomes -- proved by dispatching the member.
    assert_accepts(
        "1130_protocol_param",
        &format!("{PROTOCOL}\ndef f(a: P[int]) -> int:\n    return a.m()\n"),
    );
    // ...and the same annotation in *return* position is then refused by
    // `lower_return_annotation`'s own protocol guard, not by this issue.
    // Pinned so a later reader does not read that refusal as #1130 failing.
    let text = assert_rejects(
        "1130_protocol_return",
        &format!("{PROTOCOL}\ndef f(a: int) -> P[int]:\n    raise ValueError(\"x\")\n"),
        "C0001",
    );
    assert!(
        text.contains("a protocol class (`P`) as a return type annotation is not supported yet"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// T11: a newly-accepted annotation fed back into the arm that accepted it.
// ---------------------------------------------------------------------------

#[test]
fn a_subscripted_annotation_may_now_appear_on_the_right_of_a_type_alias() {
    // `type A = C[int]` is `T0044` today and lowers after this change,
    // binding `A = Ty::Instance("C")`. That alias then lands in the very
    // table the subscript arm consults, so `type A = C[int]` followed by
    // `x: A[str]` is a composition this change newly makes reachable -- the
    // one case where a newly-accepted annotation feeds back into the arm
    // being changed. Both halves are proved by the member read.
    assert_accepts(
        "1130_alias_rhs",
        &format!("{NOMINAL}\ntype A = NDArray[int]\n\n\ndef f(x: A) -> int:\n    return x.m()\n"),
    );
    assert_accepts(
        "1130_alias_rhs_resubscripted",
        &format!(
            "{NOMINAL}\ntype A = NDArray[int]\n\n\ndef g(y: A[str]) -> int:\n    return y.m()\n"
        ),
    );
}

// ---------------------------------------------------------------------------
// T12: the three class-body annotation positions, which do *not* move to the
// same place.
// ---------------------------------------------------------------------------

#[test]
fn the_class_body_positions_move_to_three_different_answers() {
    // A value-less class-body annotation and a dataclass field: the `T0044`
    // becomes each position's own slot-type `C0001`, because the subscripted
    // annotation now lands where the equivalent *bare* `NDArray` annotation
    // already did. A diagnostic-code swap, not a behavior change. Since
    // #1266 the value-less class-body spelling is an instance attribute
    // declaration; since #1389 a class of this program is an admitted slot
    // type, so the refusal is the declaration's never-assigned one (`D` has
    // no `__init__`), not a type refusal.
    let text = assert_rejects(
        "1130_class_attribute",
        "class NDArray:\n    pass\n\n\nclass D:\n    x: NDArray[int]\n",
        "C0001",
    );
    assert!(
        text.contains(
            "instance attribute `x` declared in class `D` is never assigned at the top level \
             of `D.__init__`"
        ),
        "{text}"
    );
    let text = assert_rejects(
        "1130_dataclass_field",
        "from dataclasses import dataclass\n\n\nclass NDArray:\n    pass\n\n\n@dataclass\nclass D:\n    x: NDArray[int]\n",
        "C0001",
    );
    assert!(
        text.contains("dataclass field `x` has type `NDArray`, which is not a scalar slot type"),
        "{text}"
    );
    // A *protocol* attribute is not a code swap: a protocol attribute typed
    // by an instance is already legal, so this position moves from `T0044`
    // to no diagnostic at all. That is this change's one silent
    // accept-flip, and it is asserted as an acceptance rather than hidden.
    assert_accepts(
        "1130_protocol_attribute",
        "from typing import Protocol\n\n\nclass NDArray:\n    pass\n\n\nclass P(Protocol):\n    x: NDArray[int]\n",
    );
}

// ---------------------------------------------------------------------------
// T13: the D-219 cascade classifier.
// ---------------------------------------------------------------------------

#[test]
fn a_body_error_under_a_newly_accepted_parameter_is_the_reported_one() {
    // The classifier itself does not change; the exposure runs the other
    // way. A parameter that previously died at `T0044` now type-checks, so
    // errors in the function's *body* that the poisoned-binding suppression
    // used to hide surface instead. The body error must be what is reported,
    // with no stale cascade suppression left over from the annotation.
    let text = assert_rejects(
        "1130_cascade",
        "class NDArray:\n    pass\n\n\ndef f(a: NDArray[int]) -> int:\n    return undefined_name\n",
        "T0021",
    );
    assert!(
        text.contains("name `undefined_name` is not defined"),
        "{text}"
    );
    assert!(!text.contains("T0044"), "{text}");
}

// ---------------------------------------------------------------------------
// T10: #931's reject set, visibly intact.
// ---------------------------------------------------------------------------

#[test]
fn the_931_reject_set_is_intact() {
    // Every base that resolves to something which is *not* a nameable type
    // keeps its rejection. This is the guard that #931 is not being reverted
    // wholesale -- what #1130 moves is the *class* arm and the carrier, not
    // this set.
    let text = assert_rejects(
        "1130_reject_self",
        "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self, x: Self[int]) -> int:\n        return 1\n",
        "T0044",
    );
    assert!(text.contains("`Self` is not subscriptable"), "{text}");

    let text = assert_rejects(
        "1130_reject_type_param",
        "def f[T](x: T[int]) -> int:\n    return 1\n",
        "T0044",
    );
    assert!(
        text.contains("type parameter `T` is not subscriptable"),
        "{text}"
    );

    let text = assert_rejects(
        "1130_reject_scalar",
        "def f(x: int[str]) -> int:\n    return 1\n",
        "T0044",
    );
    assert!(
        text.contains("builtin type `int` is not subscriptable"),
        "{text}"
    );

    let text = assert_rejects(
        "1130_reject_scalar_alias",
        "type A = int\n\n\ndef f(x: A[str]) -> int:\n    return 1\n",
        "T0044",
    );
    assert!(
        text.contains("type alias `A` is not subscriptable"),
        "{text}"
    );

    // An undefined base keeps the exact `C0001` `module::cascade_name`
    // parses back (D-219) -- this is the issue *title*'s example, which was
    // never `T0044` and is unchanged.
    //
    // The title example spelled that undefined base `NDArray`, which #1134
    // has since registered as a third source spelling of the buffer carrier,
    // so a bare `NDArray[int]` is now an *accept*. The property this arm
    // pins is "an undefined base keeps its cascade-shaped `C0001`", not the
    // spelling it was first written with, so it is restated on a name
    // nothing binds. `tests/issue_1134_ndarray_capitalized_spelling.rs`'s
    // `the_1130_undefined_base_reproducer_is_now_an_accept` carries this
    // exact program as an acceptance, so the flip is asserted somewhere
    // rather than merely removed here.
    let text = assert_rejects(
        "1130_reject_undefined",
        "def f(a: Nonexistent[int]) -> float:\n    return 0.0\n",
        "C0001",
    );
    assert!(
        text.contains("type annotation `Nonexistent` is not supported yet"),
        "{text}"
    );

    // `Any[...]` keeps `T0002`.
    assert_rejects(
        "1130_reject_any",
        "def f(a: Any[int]) -> float:\n    return 0.0\n",
        "T0002",
    );

    // A non-bare-name base keeps #889's rejection.
    let text = assert_rejects(
        "1130_reject_attribute_base",
        "def f(a: np.NDArray[int]) -> float:\n    return 0.0\n",
        "C0001",
    );
    assert!(
        text.contains("a subscripted type annotation's base must be a bare class name"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// The hosted arm.
// ---------------------------------------------------------------------------

/// A subscripted carrier parameter really does build and run as a buffer.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's coverage
/// interpreter is a different one. Every other test in this file resolves
/// its answer before `plan_ext` probes the host toolchain.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subscripted_carrier_parameter_builds_and_accepts_a_buffer() {
    let dir = fixture("1130_hosted", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("sub_probe.py"))
        .arg("-o")
        .arg(dir.join("sub_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import array, sub_probe\n\
             assert not sub_probe.__file__.endswith('.py'), sub_probe.__file__\n\
             data = array.array('d', [1.5, -2.25, 3.0])\n\
             assert sub_probe.total(data) == 2.25, sub_probe.total(data)\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "ok\n");
}
