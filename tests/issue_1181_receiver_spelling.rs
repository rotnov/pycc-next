//! #1181: an instance method's receiver may be spelled any identifier.
//!
//! Python does not reserve `self` -- it is a PEP 8 convention, and the
//! language binds the first positional parameter of an instance method
//! whatever it is called. pycc used to refuse any other spelling with
//! `C0001`, which made it the third-ranked blocking diagnostic in #1181's
//! own census of a large annotated real-world codebase.
//!
//! `crates/pycc_hir/src/class/receiver.rs` owns the rule and states it once;
//! this file is the public-CLI evidence for it. Every accepted shape is run
//! against its `self`-spelled twin, because the whole claim is that the two
//! compile and run *identically*; every rejected shape asserts the exit
//! status and the reason, so a later change to a guard fails a test rather
//! than silently reopening one of the two wrong answers the guards exist to
//! prevent (see `receiver.rs`'s module doc comment for both).

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

fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("receiver.py"), source).expect("write the subject");
    dir
}

fn run(dir: &Path) -> Output {
    pycc()
        .arg("run")
        .arg(dir.join("receiver.py"))
        .output()
        .expect("pycc should spawn")
}

fn check(dir: &Path) -> Output {
    pycc()
        .arg("check")
        .arg(dir.join("receiver.py"))
        .output()
        .expect("pycc should spawn")
}

/// Runs `source` and returns its stdout, asserting a clean exit.
fn run_ok(category: &str, source: &str) -> String {
    let dir = fixture(category, source);
    let output = run(&dir);
    assert!(
        output.status.success(),
        "expected `{category}` to run cleanly\nstdout: {}\nstderr: {}",
        stdout_of(&output),
        stderr_of(&output)
    );
    stdout_of(&output)
}

/// The core claim: `source` with its receiver spelled `this` prints exactly
/// what the byte-identical program with the receiver renamed to `self`
/// prints. `source` must spell its receiver `this` everywhere.
fn twins_agree(category: &str, source: &str) {
    assert!(
        source.contains("this"),
        "the renamed-receiver twin must actually spell the receiver `this`"
    );
    let renamed = run_ok(&format!("{category}_renamed"), source);
    let canonical = run_ok(
        &format!("{category}_canonical"),
        &source.replace("this", "self"),
    );
    assert_eq!(
        renamed, canonical,
        "the renamed-receiver program and its `self`-spelled twin must print the same thing"
    );
    assert!(
        !renamed.is_empty(),
        "the fixture must print something, or the comparison proves nothing"
    );
}

/// Asserts `source` is refused by `pycc check` with `C0001` and a message
/// containing `needle`.
fn refused(category: &str, source: &str, needle: &str) {
    let dir = fixture(category, source);
    let output = check(&dir);
    assert!(
        !output.status.success(),
        "expected `{category}` to be refused, but it passed `check`"
    );
    let reported = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        reported.contains("C0001"),
        "expected a `C0001` for `{category}`, got: {reported}"
    );
    assert!(
        reported.contains(needle),
        "expected the `{category}` diagnostic to mention {needle:?}, got: {reported}"
    );
}

// -- The completion criteria -------------------------------------------------

/// Written first, deliberately: of every completion criterion this is the
/// only one whose post-change behavior could not be observed before the
/// change, because the shape did not compile at all. CPython prints `11`
/// here, and so must both twins.
#[test]
fn a_renamed_receiver_works_under_super() {
    twins_agree(
        "1181_super",
        "\
class B:
    def peek(self) -> int:
        return 1


class D(B):
    def __init__(this) -> None:
        this.v = 10

    def probe(this) -> int:
        return super().peek() + this.v


d = D()
print(d.probe())
",
    );
}

#[test]
fn a_renamed_receiver_reads_and_stores_an_attribute() {
    twins_agree(
        "1181_attr",
        "\
class C:
    def __init__(this, v: int) -> None:
        this.v = v

    def bump(this) -> int:
        this.v = this.v + 41
        return this.v


c = C(1)
print(c.bump())
print(c.v)
",
    );
}

/// The receiver used in every position other than an attribute access: a
/// call to another of its own methods, returned as a value, passed as a call
/// argument, tested with `isinstance`, and interpolated in an f-string.
#[test]
fn a_renamed_receiver_calls_another_method_and_flows_as_a_value() {
    twins_agree(
        "1181_flow",
        "\
class C:
    def __init__(this, v: int) -> None:
        this.v = v

    def double(this) -> int:
        return this.v * 2

    def quad(this) -> int:
        return this.double() + this.double()

    def via(this) -> int:
        return read(this)

    def kind(this) -> bool:
        return isinstance(this, C)


def read(c: C) -> int:
    return c.v


c = C(3)
print(c.quad())
print(c.via())
print(c.kind())
",
    );
}

/// The ownership arm: a refcounted (`str`) attribute read through a renamed
/// receiver inside an allocating loop. A refcount defect shows up as a crash
/// or a wrong string, not as a type error, so it needs its own fixture.
#[test]
fn a_renamed_receiver_holds_a_refcounted_attribute_across_a_loop() {
    twins_agree(
        "1181_refcount",
        "\
class C:
    def __init__(this, s: str) -> None:
        this.s = s

    def shout(this) -> str:
        return this.s + \"!\"


c = C(\"hi\")
i = 0
last = \"\"
while i < 1000:
    last = c.shout()
    i = i + 1
print(last)
",
    );
}

/// Every method kind that shares the one relaxed `_ =>` arm of
/// `lower_method`'s method-kind `match`: a `@property` getter, a
/// `@<name>.setter` setter, an `@abstractmethod` (whose body is *not*
/// lowered -- it gets a synthesized `Return(None)`, so the alias is
/// prepended to that instead), a `@dataclass` body method, a
/// positional-only receiver, and multiple inheritance. `@classmethod` and
/// `@staticmethod` keep their own arms and their own rules, and are pinned
/// separately below.
#[test]
fn a_renamed_receiver_works_for_every_method_kind_sharing_the_relaxed_arm() {
    twins_agree(
        "1181_kinds",
        "\
from abc import ABC, abstractmethod
from dataclasses import dataclass


class Shape(ABC):
    @abstractmethod
    def area(this) -> int: ...


class Sq(Shape):
    def __init__(this, n: int) -> None:
        this.n = n

    @property
    def side(this) -> int:
        return this.n

    @side.setter
    def side(this, v: int) -> None:
        this.n = v

    def area(this) -> int:
        return this.n * this.n

    def posonly(this, /, k: int) -> int:
        return this.n + k


@dataclass
class Pt:
    x: int

    def shifted(this, d: int) -> int:
        return this.x + d


class L:
    def __init__(this) -> None:
        this.a = 1

    def l_of(this) -> int:
        return this.a


class R:
    def r_of(this) -> int:
        return 20


class Both(L, R):
    def total(this) -> int:
        return this.l_of() + this.r_of()


s = Sq(4)
print(s.area())
print(s.side)
s.side = 5
print(s.side)
print(s.posonly(2))
print(Pt(7).shifted(3))
print(Both().total())
",
    );
}

/// Work item 3's regression pin: a `Protocol` member declared with a
/// renamed receiver, conforming and non-conforming, plus the positional-only
/// receiver whose member signature came out correct *by accident* before
/// #1181 (`posonlyargs` was never consulted, so the name test failed and the
/// `else` branch lowered `args` = `[x]`). A bare `args.split_first()` would
/// have eaten `x` and reported a fresh `T0046` on a working program.
#[test]
fn a_renamed_receiver_in_a_protocol_body_conforms_and_refuses() {
    twins_agree(
        "1181_protocol",
        "\
from typing import Protocol


class P(Protocol):
    def val(this, x: int) -> int: ...


class Impl:
    def val(this, x: int) -> int:
        return x + 1


def use(p: P) -> int:
    return p.val(4)


print(use(Impl()))
",
    );
    twins_agree(
        "1181_protocol_posonly",
        "\
from typing import Protocol


class P(Protocol):
    def val(this, /, x: int) -> int: ...


class Impl:
    def val(this, x: int) -> int:
        return x * 3


def use(p: P) -> int:
    return p.val(5)


print(use(Impl()))
",
    );
    let dir = fixture(
        "1181_protocol_nonconforming",
        "\
from typing import Protocol


class P(Protocol):
    def val(this, x: int) -> int: ...


class Impl:
    def other(this, x: int) -> int:
        return x


def use(p: P) -> int:
    return p.val(4)


print(use(Impl()))
",
    );
    let output = check(&dir);
    assert!(
        !output.status.success(),
        "a non-conforming implementation must still be refused"
    );
    let reported = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        reported.contains("T0046"),
        "expected the protocol-conformance code, got: {reported}"
    );
}

/// A `Protocol` member that declares no parameter at all -- the third branch
/// of the split, which strips nothing. Both the member and a conforming
/// method end up with zero parameters beyond the receiver, so the program is
/// accepted and `p.val()` dispatches to the bound method, exactly as CPython
/// resolves it. The arm exists to execute that branch and to pin the fact
/// that stripping is *positional*: before #1181 the same source reached the
/// name-keyed `else` branch and happened to agree.
#[test]
fn a_protocol_member_with_no_parameters_strips_nothing() {
    assert_eq!(
        run_ok(
            "1181_protocol_empty",
            "\
from typing import Protocol


class P(Protocol):
    def val() -> int: ...


class Impl:
    def val(this) -> int:
        return 1


def use(p: P) -> int:
    return p.val()


print(use(Impl()))
"
        ),
        "1\n"
    );
}

/// A comprehension variable spelled like the receiver shadows it only inside
/// the comprehension, in pycc as in CPython -- so it is *accepted*, unlike
/// every other shape that writes the receiver's name.
#[test]
fn a_comprehension_variable_may_shadow_a_renamed_receiver() {
    twins_agree(
        "1181_comprehension",
        "\
class C:
    def __init__(this, v: int) -> None:
        this.v = v

    def probe(this) -> int:
        xs = [this for this in range(3)]
        return xs[2] + this.v


print(C(10).probe())
",
    );
}

// -- Guard 1: no occurrence of `self` ---------------------------------------

/// One arm per *binding* position plus one read arm. A reads-only scan would
/// miss the `match` capture, and that is not a cosmetic gap: emulating the
/// lowering with a `case self:` capture yields 71 where CPython yields 11,
/// because `super()` reads the local named `self` and the capture rebound it.
#[test]
fn a_renamed_receiver_method_may_not_mention_self() {
    const CASES: [(&str, &str); 9] = [
        (
            "1181_self_read",
            "class C:\n    def m(this) -> int:\n        return self\n",
        ),
        (
            "1181_self_assign",
            "class C:\n    def m(this) -> int:\n        self = 1\n        return self\n",
        ),
        (
            "1181_self_annassign",
            "class C:\n    def m(this) -> int:\n        self: int = 1\n        return 0\n",
        ),
        (
            "1181_self_augassign",
            "class C:\n    def m(this) -> int:\n        self += 1\n        return 0\n",
        ),
        (
            "1181_self_for",
            "class C:\n    def m(this) -> int:\n        for self in range(3):\n            pass\n        return 0\n",
        ),
        (
            "1181_self_with",
            "class C:\n    def m(this) -> int:\n        with open(\"f\") as self:\n            pass\n        return 0\n",
        ),
        (
            "1181_self_walrus",
            "class C:\n    def m(this) -> int:\n        if (self := 1) > 0:\n            pass\n        return 0\n",
        ),
        (
            "1181_self_match",
            "class C:\n    def m(this, k: int) -> int:\n        match k:\n            case self:\n                return 1\n        return 0\n",
        ),
        (
            "1181_self_except",
            "class C:\n    def m(this) -> int:\n        try:\n            pass\n        except ValueError as self:\n            pass\n        return 0\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "cannot also use the name `self`");
    }
}

/// The three binding positions that need their own statement forms rather
/// than an expression target: `import ... as self`, `global self`, and
/// `nonlocal self`. Each is separately rejected by `stmt::lower_body` with
/// its own `C0001`, which is exactly why the guard runs *before* that pass:
/// otherwise the receiver's own reason would never be reported.
#[test]
fn a_renamed_receiver_method_may_not_bind_self_by_statement() {
    const CASES: [(&str, &str); 4] = [
        (
            "1181_self_import",
            "class C:\n    def m(this) -> int:\n        import math as self\n        return 0\n",
        ),
        (
            "1181_self_import_from",
            "class C:\n    def m(this) -> int:\n        from math import pi as self\n        return 0\n",
        ),
        (
            "1181_self_global",
            "class C:\n    def m(this) -> int:\n        global self\n        return 0\n",
        ),
        (
            "1181_self_nonlocal",
            "class C:\n    def m(this) -> int:\n        nonlocal self\n        return 0\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "cannot also use the name `self`");
    }
}

/// The binding positions that need a destructuring target, a module-name
/// root, a `type` statement, or a non-`MatchAs` pattern. Each is a distinct
/// arm of the scan, and each is reachable Python: a scan that stopped at the
/// bare-name cases would let `self, x = 1, 2` through and silently rebind
/// the canonical receiver.
#[test]
fn a_renamed_receiver_method_may_not_bind_self_by_destructuring_or_alias() {
    const CASES: [(&str, &str); 7] = [
        (
            "1181_self_tuple",
            "class C:\n    def m(this) -> int:\n        self, x = 1, 2\n        return 0\n",
        ),
        (
            "1181_self_list",
            "class C:\n    def m(this) -> int:\n        [self, x] = [1, 2]\n        return 0\n",
        ),
        (
            "1181_self_starred",
            "class C:\n    def m(this) -> int:\n        *self, x = [1, 2, 3]\n        return 0\n",
        ),
        (
            "1181_self_for_tuple",
            "class C:\n    def m(this) -> int:\n        for self, y in []:\n            pass\n        return 0\n",
        ),
        (
            "1181_self_import_root",
            "class C:\n    def m(this) -> int:\n        import self.sub\n        return 0\n",
        ),
        (
            "1181_self_type_alias",
            "class C:\n    def m(this) -> int:\n        type self = int\n        return 0\n",
        ),
        (
            "1181_self_match_star",
            "class C:\n    def m(this, k: int) -> int:\n        match k:\n            case [*self]:\n                return 1\n        return 0\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "cannot also use the name `self`");
    }
    // The scan stops at the first occurrence, but the AST walk it rides on
    // does not know that: an `except` handler and a later `case` pattern are
    // still visited after an earlier statement in the same `try`/`match` has
    // already matched. These two fixtures put the occurrence *before* those
    // visits so the short-circuit in each override actually runs.
    refused(
        "1181_self_found_before_handler",
        "class C:\n    def m(this) -> int:\n        try:\n            self = 1\n        except ValueError as e:\n            pass\n        return 0\n",
        "cannot also use the name `self`",
    );
    refused(
        "1181_self_found_before_pattern",
        "class C:\n    def m(this, k: int) -> int:\n        match k:\n            case 1:\n                self = 1\n            case 2:\n                pass\n        return 0\n",
        "cannot also use the name `self`",
    );
    // `MatchMapping`'s `**rest` capture is the last pattern form that binds a
    // name without producing an `Expr::Name`.
    refused(
        "1181_self_match_mapping_rest",
        "class C:\n    def m(this, k: int) -> int:\n        match k:\n            case {**self}:\n                return 1\n        return 0\n",
        "cannot also use the name `self`",
    );
}

/// A *parameter* named `self` alongside a renamed receiver would collide with
/// the canonical parameter name the receiver is lowered under, so the guard
/// reads the parameter list too, not only the body.
#[test]
fn a_renamed_receiver_method_may_not_declare_a_parameter_named_self() {
    refused(
        "1181_self_param",
        "class C:\n    def m(this, self: int) -> int:\n        return 0\n",
        "cannot also use the name `self`",
    );
}

/// A module-level global named `self` is legal Python, and pycc resolves a
/// method's read of a module global today. With a renamed receiver a bare
/// `self` read would resolve to the canonical *parameter* instead, so the
/// guard has to reject it. This is a deliberate over-rejection of valid
/// Python, and the diagnostic says why.
#[test]
fn a_module_level_global_named_self_is_deliberately_over_rejected() {
    refused(
        "1181_self_global_read",
        "self: int = 5\n\n\nclass C:\n    def m(this) -> int:\n        return self\n",
        "cannot also use the name `self`",
    );
    // The `self`-spelled twin of that same program resolves the parameter,
    // exactly as it did before #1181 -- the over-rejection is confined to
    // the renamed-receiver case.
    assert_eq!(
        run_ok(
            "1181_self_global_twin",
            "self: int = 5\n\n\nclass C:\n    def m(self) -> int:\n        return 1\n\n\nprint(C().m())\n"
        ),
        "1\n"
    );
}

/// The boundary the guard must *not* over-reject: an attribute named
/// `.self`. It carries an `Identifier` rather than an `Expr::Name`, so it is
/// neither a read nor a binding of a local, and a scan that keyed on the
/// spelling alone would refuse a working program.
#[test]
fn an_attribute_named_self_is_not_an_occurrence() {
    twins_agree(
        "1181_dot_self",
        "\
class Inner:
    def __init__(this) -> None:
        this.self = 7


class C:
    def __init__(this) -> None:
        this.v = 1

    def probe(this, i: Inner) -> int:
        return i.self + this.v


print(C().probe(Inner()))
",
    );
}

// -- Guard 2: no rebinding of the receiver ----------------------------------

/// The motivating arm, with CPython's own answer recorded so the cost of the
/// narrowing is visible to whoever later lifts it: for a `probe` that does
/// `other = D(7); <receiver> = other; return super().peek()`, **CPython
/// prints 7** -- its zero-argument `super()` reads the frame's first local's
/// *current* value. pycc binds the receiver once at entry, so the alias shape
/// would print 1. A diagnostic is the correct answer where the alternative is
/// a silently wrong one.
#[test]
fn rebinding_a_renamed_receiver_is_refused_rather_than_answered_wrongly() {
    refused(
        "1181_rebind_super",
        "\
class B:
    def peek(self) -> int:
        return 1


class D(B):
    def __init__(this) -> None:
        this.v = 1

    def probe(this) -> int:
        other = D()
        this = other
        return super().peek()


print(D().probe())
",
        "rebinding a method's receiver",
    );
}

/// One arm per binding position, the same override set guard 1 uses.
/// Annotated assignment is on the list because `this: C = other` is accepted
/// and *working* today for a `self`-spelled receiver, which is precisely why
/// the guard has to see it.
#[test]
fn every_rebinding_position_of_a_renamed_receiver_is_refused() {
    const CASES: [(&str, &str); 9] = [
        (
            "1181_rebind_assign",
            "class C:\n    def m(this) -> int:\n        this = C()\n        return 0\n",
        ),
        (
            "1181_rebind_annassign",
            "class C:\n    def m(this) -> int:\n        this: C = C()\n        return 0\n",
        ),
        (
            "1181_rebind_augassign",
            "class C:\n    def m(this) -> int:\n        this += 1\n        return 0\n",
        ),
        (
            "1181_rebind_for",
            "class C:\n    def m(this) -> int:\n        for this in range(3):\n            pass\n        return 0\n",
        ),
        (
            "1181_rebind_with",
            "class C:\n    def m(this) -> int:\n        with open(\"f\") as this:\n            pass\n        return 0\n",
        ),
        (
            "1181_rebind_walrus",
            "class C:\n    def m(this) -> int:\n        if (this := 1) > 0:\n            pass\n        return 0\n",
        ),
        (
            "1181_rebind_match",
            "class C:\n    def m(this, k: int) -> int:\n        match k:\n            case this:\n                return 1\n        return 0\n",
        ),
        (
            "1181_rebind_except",
            "class C:\n    def m(this) -> int:\n        try:\n            pass\n        except ValueError as this:\n            pass\n        return 0\n",
        ),
        (
            "1181_rebind_import",
            "class C:\n    def m(this) -> int:\n        import math as this\n        return 0\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "rebinding a method's receiver");
    }
}

/// A nested `def` or `class` spelled like the receiver is a binding too. Both
/// are separately unsupported inside a method body, so the guard's own
/// message is what a user sees -- the same reason the scan runs before
/// `stmt::lower_body`.
#[test]
fn a_nested_definition_named_like_the_receiver_is_refused() {
    const CASES: [(&str, &str); 2] = [
        (
            "1181_rebind_def",
            "class C:\n    def m(this) -> int:\n        def this() -> int:\n            return 1\n        return 0\n",
        ),
        (
            "1181_rebind_class",
            "class C:\n    def m(this) -> int:\n        class this:\n            pass\n        return 0\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "rebinding a method's receiver");
    }
}

// -- Preserved rejections ---------------------------------------------------

/// A method with no parameters at all is still refused -- #1181 relaxes the
/// receiver's *spelling*, never its presence. The message no longer asserts
/// the parameter is called `self`.
#[test]
fn a_method_with_no_parameters_is_still_refused() {
    refused(
        "1181_no_params",
        "class C:\n    def m() -> int:\n        return 1\n",
        "must take a receiver as its first parameter",
    );
}

/// The receiver's own structural rejections survive the relaxation, reworded
/// so they name the receiver rather than asserting its spelling.
#[test]
fn a_receiver_with_a_default_or_an_annotation_is_still_refused() {
    refused(
        "1181_receiver_default",
        "class C:\n    def m(this: \"C\" = None) -> int:\n        return 1\n",
        "cannot have a default value",
    );
    refused(
        "1181_receiver_annotation",
        "class C:\n    def m(this: \"C\") -> int:\n        return 1\n",
        "annotation on a method's receiver parameter",
    );
}

/// The `@property`/`@<name>.setter` arity rules survive, likewise reworded.
#[test]
fn the_property_arity_rules_are_unchanged_for_a_renamed_receiver() {
    refused(
        "1181_getter_arity",
        "class C:\n    @property\n    def v(this, k: int) -> int:\n        return k\n",
        "must take only its receiver",
    );
    refused(
        "1181_setter_arity",
        "class C:\n    @property\n    def v(this) -> int:\n        return 1\n\n    @v.setter\n    def v(this) -> None:\n        pass\n",
        "exactly one parameter besides its receiver",
    );
}

/// `@classmethod` and `@staticmethod` are governed by their own arms of
/// `lower_method`'s method-kind `match`, which #1181 does not touch: `cls` is
/// still required, and a static method still takes no receiver.
#[test]
fn classmethod_and_staticmethod_are_unchanged() {
    assert_eq!(
        run_ok(
            "1181_decorated_kinds",
            "\
class C:
    @classmethod
    def make(cls, n: int) -> int:
        return n + 1

    @staticmethod
    def scale(n: int) -> int:
        return n * 3


print(C.make(1))
print(C.scale(2))
"
        ),
        "2\n6\n"
    );
    refused(
        "1181_classmethod_renamed",
        "class C:\n    @classmethod\n    def make(klass, n: int) -> int:\n        return n\n",
        "`cls`",
    );
}

/// The issue's own "preserved" criterion: a `@staticmethod` declaring a
/// *non-receiver* parameter named `self` must not start being treated as a
/// receiver. `crates/pycc_types/src/class/super_call.rs` keys on the
/// `self` *binding* rather than on a method kind, so this program still
/// passes `check` exactly as it did before #1181. Its `pycc build` is a
/// pre-existing codegen panic, wholly independent of this change and
/// deliberately not exercised here.
#[test]
fn a_staticmethod_parameter_named_self_is_not_treated_as_a_receiver() {
    let dir = fixture(
        "1181_static_self_param",
        "\
class B:
    def peek(self) -> int:
        return 1


class D(B):
    @staticmethod
    def probe(self: int) -> int:
        return super().peek() + self
",
    );
    let output = check(&dir);
    assert!(
        output.status.success(),
        "a `@staticmethod` with a non-receiver parameter named `self` must keep passing \
         `check`\nstdout: {}\nstderr: {}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The three dunders CPython implicitly rebinds to a different method kind.
/// All three were rejected before #1181 *by the very spelling rule it
/// removes*, so without the replacement guard each would silently have
/// become a plain instance method -- a fresh CPython deviation introduced by
/// a change whose whole point is removing one.
#[test]
fn an_implicitly_rebound_dunder_still_requires_self() {
    const CASES: [(&str, &str); 3] = [
        (
            "1181_dunder_new",
            "class C:\n    def __new__(x) -> int:\n        return 1\n",
        ),
        (
            "1181_dunder_init_subclass",
            "class C:\n    def __init_subclass__(x) -> None:\n        pass\n",
        ),
        (
            "1181_dunder_class_getitem",
            "class C:\n    def __class_getitem__(i) -> int:\n        return 1\n",
        ),
    ];
    for (category, source) in CASES {
        refused(category, source, "must spell its first parameter `self`");
    }
}

/// `collect_init_attrs`' companion: `slot_ty_from_init_rhs` resolves an RHS
/// naming the receiver through the *source* spelling, so a renamed receiver
/// establishes the same instance-typed slot its `self`-spelled twin does
/// (#1389) -- not the unresolvable-name rejection -- and an unknown name's
/// message quotes the receiver as the user spelled it.
#[test]
fn an_init_rhs_naming_a_renamed_receiver_resolves_to_the_receivers_own_type() {
    twins_agree(
        "1181_init_rhs_receiver",
        "class C:\n    def __init__(this, v: int) -> None:\n        this.v = v\n        \
         this.x = this\nc = C(4)\nprint(c.x.x.v)\n",
    );
    refused(
        "1181_init_rhs_unknown",
        "class C:\n    def __init__(this, v: int) -> None:\n        this.x = nope\n",
        "`this.<attr> = nope` must reference one of `__init__`'s own parameters",
    );
}

// -- The `--ext` export path ------------------------------------------------

/// The issue's `pycc build --ext` criterion, plus the two buffer shapes that
/// pin `src/ext_build.rs`'s `body_stores_into` / `body_returns_slice_of`
/// walks: both are exhaustive with no `_` arm and both return `false` for an
/// `HirStmt::Assign`, so the prepended receiver alias cannot change either
/// memory-safety answer.
///
/// Asserted as the absence of any diagnostic rather than as a successful
/// build, exactly as `tests/issue_1174_method_buffer_return.rs`'s own
/// admission arm is and for its reason: the build reaches `clang` and fails
/// there on a host whose `python3` is older than 3.13 or carries no
/// development headers, and that failure is not what this arm owns. The
/// interpreter half lives in the `#[ignore]`d test below.
#[test]
fn a_renamed_receiver_is_admitted_as_an_ext_export() {
    const SUBJECT: &str = "\
class Grid:
    def __init__(this, w: int) -> None:
        this.w = w

    def area(this) -> int:
        return this.w * 3

    def fill(this, buf: memoryview) -> int:
        buf[0] = 1.5
        return this.w

    def rows(this) -> memoryview:
        a = ndarray(3)
        a[0] = 2.5
        return a
";
    for (category, source) in [
        ("1181_ext_renamed", SUBJECT),
        ("1181_ext_canonical", &SUBJECT.replace("this", "self")),
    ] {
        let dir = fixture(category, source);
        let build = pycc()
            .arg("build")
            .arg(dir.join("receiver.py"))
            .arg("-o")
            .arg(dir.join("gridmod"))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        let err = stderr_of(&build);
        assert!(!err.contains("error[C0003]"), "{category}: {err}");
        assert!(!err.contains("error[C0001]"), "{category}: {err}");
        assert!(!err.contains("error[I0405]"), "{category}: {err}");
        assert!(!err.contains("error[T0"), "{category}: {err}");
        assert!(!err.contains("panicked"), "{category}: {err}");
    }
}

/// The interpreter half of the `--ext` criterion: a real CPython imports the
/// artifact and calls the exported method whose receiver is renamed,
/// repeatedly, so a receiver-aliasing refcount defect would surface. Ignored
/// for the reason `tests/issue_1145_ext_instance_methods.rs` gives -- it
/// depends on the machine, not on the change.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_renamed_receiver_ext_export_is_callable_from_cpython() {
    let dir = fixture(
        "1181_ext_import",
        "\
class Grid:
    def __init__(this, w: int) -> None:
        this.w = w

    def area(this) -> int:
        return this.w * 3
",
    );
    let build = pycc()
        .arg("build")
        .arg(dir.join("receiver.py"))
        .arg("-o")
        .arg(dir.join("gridmod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "\
import gridmod
g = gridmod.Grid(4)
for _ in range(100):
    assert g.area() == 12, g.area()
",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}
