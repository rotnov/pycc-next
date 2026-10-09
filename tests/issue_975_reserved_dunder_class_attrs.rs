//! Issue #975 ([D-236]): a class attribute named after the instantiation or
//! class-creation protocol, end to end through the public `pycc` CLI.
//!
//! [D-236]: ../docs/decisions/D-236-reject-a-class-attribute-named-after-the-instantiation.md
//!
//! The defect class is D-198 false acceptance: pycc compiled and ran a
//! program CPython rejects. `class C: __init__: ClassVar[int] = 8` followed
//! by `C()` printed a number under pycc and raised
//! `TypeError: 'int' object is not callable` under CPython 3.13.9, because
//! `ensure_init` decides whether to synthesize a constructor from the method
//! table alone and never consults `class_attrs`.
//!
//! Three properties are pinned here, each of which a narrower fix would have
//! missed:
//!
//! 1. **Three names, not one.** The issue named `__init__`/`__eq__`/`__repr__`;
//!    the set that actually diverges is `__init__`/`__new__`/`__init_subclass__`.
//!    `__eq__` and `__repr__` do *not* diverge in a plain class body, so they
//!    are pinned here as still-accepted. In an `Enum` body they *did* diverge,
//!    for a different reason -- CPython's `_EnumDict` keeps every dunder out of
//!    the member list while pycc lowered it as a member -- which was a separate
//!    defect, [#979](https://github.com/rotnov/pycc/issues/979), closed by
//!    D-238; the enum half of that pin below is inverted accordingly, and the
//!    rest of #979's family lives in
//!    `tests/issue_979_enum_non_member_names.rs`.
//! 2. **Three spellings, not one.** `ClassVar[int] = 8`, `int = 8` and a bare
//!    `= 8` all reach the same hazard, so the guard is not `ClassVar`-gated.
//! 3. **Four class-body routes, not one.** A plain class, a `@dataclass`
//!    body, an `Enum` member list, and (from the #978 review round) a
//!    `@property` getter each reach it by a different path.
//!
//! The already-rejected shapes (a `__slots__` value CPython itself rejects --
//! #1368 admits the value-bound spellings -- and a method collision) are
//! pinned: this change adds a guard, it does not re-implement them.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, source: &str) -> std::path::PathBuf {
    let path = dir.join("main.py");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Asserts that `pycc check` rejects `source` with `code` and a diagnostic
/// containing `needle`.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded:\n{rendered}"
    );
    assert!(
        rendered.contains(code),
        "diagnostic for {tag} should carry {code}, got:\n{rendered}"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

/// Asserts that `pycc check` accepts `source` -- the negative half of the
/// rule, which is what stops the guard from growing past its evidence.
fn assert_accepted(tag: &str, source: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pycc check should accept {tag}, got:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The distinctive fragment of each name's message. Only `__init__` and
/// `__new__` name CPython's `TypeError`; `__init_subclass__` shares its string
/// with the `Enum` route, where CPython does not raise. Neither of the two
/// names a concrete bound *type* in that `TypeError` --
/// `a_non_integer_binding_...` below is the end-to-end pin for that.
const RESERVED: [(&str, &str); 3] = [
    (
        "__init__",
        "resolves a class's constructor from its methods alone",
    ),
    ("__new__", "does not model `__new__` at all"),
    (
        "__init_subclass__",
        "walking the MRO's function definitions",
    ),
];

// -- three names x three spellings, plain class ----------------------------

/// The issue's own reproduction, generalized to the corrected name set. Each
/// of these ran to completion under pycc at `28a1b194` while CPython 3.13.9
/// raised `TypeError` at the `C()` call (or, for `__init_subclass__`, at the
/// subclass declaration).
#[test]
fn a_class_var_named_after_the_instantiation_protocol_is_rejected() {
    for (name, needle) in RESERVED {
        assert_rejected(
            &format!("975_classvar_{name}"),
            &format!(
                "from typing import ClassVar\n\
                 \n\
                 \n\
                 class C:\n\
                 \x20   {name}: ClassVar[int] = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

/// The plain-annotation spelling: no `ClassVar` wrapper, same hazard. A
/// `ClassVar`-gated guard would have fixed one spelling of three.
#[test]
fn a_plain_annotated_attribute_named_after_the_instantiation_protocol_is_rejected() {
    for (name, needle) in RESERVED {
        assert_rejected(
            &format!("975_annotated_{name}"),
            &format!(
                "class C:\n\
                 \x20   {name}: int = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

/// #910's bare-assignment spelling, which reaches the class-attribute path
/// through `lower_unannotated_class_attr` rather than the annotated one.
#[test]
fn a_bare_assignment_named_after_the_instantiation_protocol_is_rejected() {
    for (name, needle) in RESERVED {
        assert_rejected(
            &format!("975_bare_{name}"),
            &format!(
                "class C:\n\
                 \x20   {name} = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

/// Review round on #978: the guard keys on the attribute *name* alone and
/// runs before any value extraction, so the same message is rendered whatever
/// the initializer binds. CPython's own text names the bound type -- it is
/// `'str' object is not callable` for `__init__ = "x"`, `'bool'` for `True`,
/// `'float'` for `1.5` -- so the diagnostic must not name one. Pin both
/// halves end to end: the rejection still fires for a non-integer binding,
/// and the rendered diagnostic never claims a concrete type.
#[test]
fn a_non_integer_binding_is_rejected_without_naming_a_concrete_type() {
    for (name, needle) in [
        (
            "__init__",
            "resolves a class's constructor from its methods alone",
        ),
        ("__new__", "does not model `__new__` at all"),
    ] {
        for (shape, decl) in [
            ("str", format!("{name} = \"x\"")),
            ("classvar_str", format!("{name}: ClassVar[str] = \"x\"")),
            ("bool", format!("{name} = True")),
            ("float", format!("{name} = 1.5")),
        ] {
            let tag = format!("975_nonint_{shape}_{name}");
            let source = format!(
                "from typing import ClassVar\n\
                 \n\
                 \n\
                 class C:\n\
                 \x20   {decl}\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            );
            assert_rejected(&tag, &source, "C0001", needle);

            let dir = ScratchDir::new(&tag).expect("failed to create scratch dir");
            let src = write_fixture(&dir, &source);
            let out = Command::new(pycc_bin())
                .args(["check", src.to_str().unwrap()])
                .output()
                .unwrap();
            let rendered = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(
                !rendered.contains("object is not callable"),
                "diagnostic for {tag} must not name a concrete bound type, got:\n{rendered}"
            );
        }
    }
}

// -- the dataclass path (D-235's two gaps) ---------------------------------

/// D-235's `DATACLASS_IMPLICIT_DUNDERS` lists six names and omits these two,
/// so before this change `@dataclass class P: __new__: ClassVar[int] = 8` ran
/// under pycc and raised `TypeError` under CPython. The universal guard runs
/// in every class body, which closes the gap without touching D-235's set.
#[test]
fn a_dataclass_class_var_named_new_or_init_subclass_is_rejected() {
    for (name, needle) in [
        ("__new__", "does not model `__new__` at all"),
        (
            "__init_subclass__",
            "walking the MRO's function definitions",
        ),
    ] {
        assert_rejected(
            &format!("975_dataclass_{name}"),
            &format!(
                "from dataclasses import dataclass\n\
                 from typing import ClassVar\n\
                 \n\
                 \n\
                 @dataclass\n\
                 class P:\n\
                 \x20   x: int\n\
                 \x20   {name}: ClassVar[int] = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   p = P(1)\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

/// D-235 regression pin: `__init__` in a `@dataclass` body keeps reporting
/// D-235's own message. The two name sets are deliberately disjoint, and
/// `body`'s dataclass check runs before the class-attribute path, so every
/// message D-235 pinned stays byte-stable.
#[test]
fn a_dataclass_class_var_named_init_still_reports_the_d235_message() {
    assert_rejected(
        "975_dataclass_init_d235",
        "from dataclasses import dataclass\n\
         from typing import ClassVar\n\
         \n\
         \n\
         @dataclass\n\
         class P:\n\
         \x20   x: int\n\
         \x20   __init__: ClassVar[int] = 8\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   p = P(1)\n\
         \x20   return 0\n",
        "C0001",
        "a `@dataclass` class implicitly relies on `__init__`",
    );
}

// -- the enum path ---------------------------------------------------------

/// The third route. `class C(Enum): __init__ = 1; B = 2` printed `2` under
/// pycc at `28a1b194` while CPython raised
/// `TypeError: 'int' object is not callable` at class creation, via
/// `enum.py`'s `__set_name__`.
///
/// One enum class per name: `lower_enum_class`'s member loop returns on the
/// first error, so a body listing all three would only ever exercise the
/// first.
#[test]
fn an_enum_member_named_after_the_instantiation_protocol_is_rejected() {
    for (name, needle) in RESERVED {
        assert_rejected(
            &format!("975_enum_{name}"),
            &format!(
                "from enum import Enum\n\
                 \n\
                 \n\
                 class C(Enum):\n\
                 \x20   {name} = 1\n\
                 \x20   B = 2\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

// -- the inherited shape ---------------------------------------------------

/// `__init_subclass__` diverges only once a subclass exists, but the guard
/// fires during the *declaring* class's body walk, which has no whole-program
/// information. The diagnostic therefore points at `A`, not at `class B(A)`.
/// Pinned so a future reader does not go looking for a `B`-site message.
#[test]
fn an_inherited_reserved_attribute_is_rejected_on_the_declaring_class() {
    assert_rejected(
        "975_inherited",
        "from typing import ClassVar\n\
         \n\
         \n\
         class A:\n\
         \x20   __init_subclass__: ClassVar[int] = 8\n\
         \n\
         \n\
         class B(A):\n\
         \x20   pass\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   b = B()\n\
         \x20   return 0\n",
        "C0001",
        "walking the MRO's function definitions",
    );
}

// -- the negatives: the guard must not grow past its evidence ---------------

/// The five names D-235 lists that are *not* in the universal set, plus
/// `__hash__`, stay accepted in a plain class body. Measured under CPython
/// 3.13.9: both engines agree on every one of them.
///
/// Their safety is conditional, not intrinsic -- it rests entirely on every
/// rewrite that consults them being `is_dataclass`-gated today. If that
/// gating is ever removed, this test is what should start failing, and the
/// name belongs in the universal set at that point (D-236's "trigger to
/// revisit").
#[test]
fn dunders_outside_the_instantiation_protocol_stay_accepted_in_a_plain_class() {
    for name in [
        "__eq__",
        "__repr__",
        "__ne__",
        "__str__",
        "__format__",
        "__hash__",
    ] {
        assert_accepted(
            &format!("975_accepted_{name}"),
            &format!(
                "from typing import ClassVar\n\
                 \n\
                 \n\
                 class C:\n\
                 \x20   {name}: ClassVar[int] = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
        );
    }
}

/// The #975 end-to-end pin, inverted by
/// [#979](https://github.com/rotnov/pycc/issues/979) (D-238). It used to
/// assert that an `Enum` member named after a dunder outside D-236's set was
/// accepted, and recorded that as a divergence rather than agreement: the
/// review round on #978 re-measured it and found CPython 3.13.9's `_EnumDict`
/// keeps every dunder out of the member list, so `for c in C` counted two
/// members here and one there, and `C.__repr__.value` printed `1` here where
/// CPython raises `AttributeError`.
///
/// #979 rejects the whole `_EnumDict` non-member family on the `Enum` route,
/// so this program is now `C0001` with its own message. D-236's own set and
/// guard are unaffected: the enum route still reports D-236's strings for the
/// instantiation-protocol names, which `RESERVED`'s enum cases above still
/// pin. The rest of the #979 family is covered end to end in
/// `tests/issue_979_enum_non_member_names.rs`.
#[test]
fn an_enum_member_named_after_an_unreserved_dunder_is_rejected() {
    assert_rejected(
        "975_enum_repr_rejected",
        "from enum import Enum\n\
         \n\
         \n\
         class C(Enum):\n\
         \x20   __repr__ = 1\n\
         \x20   B = 2\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
        "C0001",
        "a dunder-named assignment in an `Enum` body is not supported yet",
    );
}

// -- regression pins for shapes that were already rejected -----------------

/// #910's `__slots__` guard shares the function but not the name set. Since
/// #1368 a bound `__slots__` is parsed by `class::slots`, so a non-iterable
/// value reports CPython's own `TypeError` text.
#[test]
fn the_slots_rejection_quotes_cpythons_type_error() {
    assert_rejected(
        "975_slots",
        "class C:\n\
         \x20   __slots__ = 8\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
        "C0001",
        "CPython raises `TypeError: 'int' object is not iterable`",
    );
}

/// Review round on #978: the shared guard made a fourth shape reachable --
/// `__slots__` in an `Enum` body -- and the plain-class message it first
/// rendered claimed the layout is fixed from `__init__`, which no enum has.
/// Pin the enum-specific text end to end.
///
/// CPython 3.13.9 accepts both shapes (`_EnumDict` keeps a dunder out of the
/// member list): `class C(Enum): __slots__ = "x"` with `A = 1` leaves
/// `C.__slots__ == 'x'` and `list(C) == [C.A]`, and `__slots__ = ()` behaves
/// the same. The rejection is therefore conservative, and the message says so
/// instead of claiming a measured divergence.
#[test]
fn the_enum_slots_rejection_uses_the_enum_specific_message() {
    for (tag, value) in [
        ("975_enum_slots_str", "\"x\""),
        ("975_enum_slots_tuple", "()"),
    ] {
        assert_rejected(
            tag,
            &format!(
                "from enum import Enum\n\
                 \n\
                 \n\
                 class C(Enum):\n\
                 \x20   __slots__ = {value}\n\
                 \x20   A = 1\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   return 0\n"
            ),
            "C0001",
            "`__slots__` in an `Enum` body is not supported yet",
        );
    }
}

/// The method-collision rejection still exists for a non-reserved name, so
/// the new guard did not absorb or shadow `reject_class_attr_collisions`.
#[test]
fn the_method_collision_rejection_is_unchanged_for_an_ordinary_name() {
    assert_rejected(
        "975_collision",
        "from typing import ClassVar\n\
         \n\
         \n\
         class C:\n\
         \x20   f: ClassVar[int] = 8\n\
         \n\
         \x20   def f(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
        "C0001",
        "collides with a method",
    );
}

/// Precedence pin: with both defects present, the class-body-walk guard wins
/// over `reject_class_attr_collisions`. This is consistent with D-235's
/// pinned four-deep precedence (class-body-walk errors come first) rather
/// than a reorder of it -- pinned so a future reorder fails a test instead of
/// silently changing which defect a two-defect program reports.
#[test]
fn the_reserved_name_guard_precedes_the_method_collision_check() {
    assert_rejected(
        "975_precedence",
        "from typing import ClassVar\n\
         \n\
         \n\
         class C:\n\
         \x20   __init__: ClassVar[int] = 8\n\
         \n\
         \x20   def __init__(self) -> None:\n\
         \x20       pass\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
        "C0001",
        "resolves a class's constructor from its methods alone",
    );
}

// -- the fourth class-body route: a `@property` getter --------------------

/// #978 review round: `@property def <name>` reaches a class-level binding of
/// a reserved name through `walk_class_body`'s `MethodKind::PropertyGetter`
/// arm, which none of the three attribute routes covers.
///
/// Measured on CPython 3.13.9 for each name:
///
/// * `@property def __new__` + `C()` -> `TypeError: 'property' object is not
///   callable` at `C()`. pycc at `f3eb908c` **accepted and ran** this
///   program -- the D-198 false acceptance this fix closes.
/// * `@property def __init__` + `C()` -> `TypeError: 'int' object is not
///   callable` at `C()`. The type differs from the previous row because
///   `type.__call__` looks `__init__` up on the *instance*, which invokes the
///   getter and then calls its `int` result. pycc already rejected this, but
///   as `T0021` ("cannot redefine function `C.__init__` with a different
///   signature"), which describes the wrong defect.
/// * `@property def __init_subclass__` + `class B(C)` -> `TypeError:
///   'property' object is not callable` at `class B`. pycc already rejected
///   this too, as an unrelated `C0001` about the MRO hook having to be
///   statically evaluable.
///
/// So this is pinned by message, not merely by exit status: two of the three
/// were already non-zero for the wrong reason.
#[test]
fn a_property_getter_named_after_the_instantiation_protocol_is_rejected() {
    for (name, needle) in RESERVED {
        assert_rejected(
            &format!("975_property_{name}"),
            &format!(
                "class C:\n\
                 \x20   @property\n\
                 \x20   def {name}(self) -> int:\n\
                 \x20       return 1\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
            "C0001",
            needle,
        );
    }
}

/// #978 review round, negative half: the guard keys on the property
/// *spelling*, not on the name alone. A plain `def __init__` must still
/// compile, and an ordinary `@property` must still compile -- the two shapes
/// that a name-only guard on this route would have broken.
#[test]
fn a_plain_constructor_and_an_ordinary_property_stay_accepted() {
    assert_accepted(
        "975_property_plain_init",
        "class C:\n\
         \x20   def __init__(self) -> None:\n\
         \x20       self.x = 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return c.x - 1\n",
    );
    assert_accepted(
        "975_property_ordinary",
        "class C:\n\
         \x20   def __init__(self) -> None:\n\
         \x20       self.x = 7\n\
         \n\
         \x20   @property\n\
         \x20   def value(self) -> int:\n\
         \x20       return self.x\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   print(c.value)\n\
         \x20   return 0\n",
    );
}
