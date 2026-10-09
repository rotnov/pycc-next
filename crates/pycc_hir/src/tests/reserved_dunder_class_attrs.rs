//! Class attributes named after the instantiation/class-creation protocol
//! (#975, [D-236]).
//!
//! [D-236]: ../../../../docs/decisions/D-236-reject-a-class-attribute-named-after-the-instantiation.md
//!
//! Since #980 the file also carries the `@property` getter route's `__slots__`
//! rejection -- a different name set and a different mechanism, sharing only
//! the call site -- together with the deferral pin for `@property def
//! __qualname__` ([#982](https://github.com/rotnov/pycc/issues/982)) and the
//! one for a callable `def __new__`
//! ([#981](https://github.com/rotnov/pycc/issues/981)).
//!
//! [#984](https://github.com/rotnov/pycc/issues/984) widened that `__slots__`
//! half from the `@property` getter to every other `def` spelling and to a
//! `Protocol` body, so the file now also pins the three carrier types CPython
//! names, the two precedence boundaries the widening had to respect (a
//! `@__slots__.setter` with no getter, and an implementation-bodied protocol
//! method), and the negative halves on both method routes.
//!
//! Its own child module for the same reason as `dataclass_class_vars`
//! (AGENTS.md "Keep source files decomposable"): `tests.rs` is already ~7k
//! lines. `use super::*` reaches the parent's private helpers.
//!
//! Every name is exercised on its own, never as one combined class body: the
//! class-body walk and the enum member loop both return on the first error,
//! so a body listing all three would only ever prove the first one fires.

use super::*;

/// The three names, with the distinctive fragment of each one's message.
///
/// Only `__init__` and `__new__` may name CPython's `TypeError` --
/// `__init_subclass__` shares its string with the `Enum` route, where CPython
/// does not raise. Neither of the two names a concrete bound *type* in that
/// `TypeError`; `a_non_integer_binding_...` below is the pin for that.
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

/// #975: the `ClassVar` spelling in a plain class body.
#[test]
fn a_class_var_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(
            &format!("from typing import ClassVar\n\n\nclass C:\n    {name}: ClassVar[int] = 8\n"),
            fragment,
        );
    }
}

/// #975: the plain-annotation spelling. The guard must not be `ClassVar`-gated
/// -- `body`'s non-dataclass branch routes every `AnnAssign` to
/// `lower_class_attr` regardless of the wrapper.
#[test]
fn an_annotated_attribute_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(&format!("class C:\n    {name}: int = 8\n"), fragment);
    }
}

/// #975: #910's bare-assignment spelling, which reaches
/// `lower_unannotated_class_attr` instead.
#[test]
fn a_bare_assignment_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(&format!("class C:\n    {name} = 8\n"), fragment);
    }
}

/// #975: a *value-less* declaration (`__init__: int`, no `=`). The guard runs
/// before the value-presence check, so this input gets the reserved-name
/// message rather than the generic "there is nothing to fold" one. It was
/// already rejected before this change, so only the message differs -- but it
/// is why both message texts say "binding that name ... makes CPython raise"
/// rather than "CPython raises here": a bare annotation creates no class
/// `__dict__` entry in CPython, so the unconditional claim would be false.
#[test]
fn a_value_less_declaration_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(&format!("class C:\n    {name}: int\n"), fragment);
    }
}

/// #975 review round: the guard fires on the attribute *name* alone, so the
/// same message is emitted whatever the initializer binds. CPython's own text
/// names the bound type (`'str' object is not callable` for `__init__ = "x"`),
/// which the message therefore must not do -- naming one concrete type would
/// be wrong for every other binding. Pin both directions: the reserved-name
/// message still fires for a non-integer binding, and it never names a type.
#[test]
fn a_non_integer_binding_is_rejected_without_naming_a_concrete_type() {
    for (name, fragment) in [
        (
            "__init__",
            "resolves a class's constructor from its methods alone",
        ),
        ("__new__", "does not model `__new__` at all"),
    ] {
        for body in [
            format!("class C:\n    {name} = \"x\"\n"),
            format!(
                "from typing import ClassVar\n\n\nclass C:\n    {name}: ClassVar[str] = \"x\"\n"
            ),
            format!("class C:\n    {name} = True\n"),
            format!("class C:\n    {name} = 1.5\n"),
        ] {
            let module = pycc_parser_test_helper::parse(&body);
            let diagnostic = lower_checked(&module).unwrap_err();
            assert_eq!(diagnostic.code, "C0001");
            assert!(diagnostic.message.contains(fragment), "{body}");
            assert!(!diagnostic.message.contains("'int'"), "{body}");
            assert!(
                !diagnostic.message.contains("object is not callable"),
                "{body}"
            );
        }
    }
}

/// #975: the two names D-235's `DATACLASS_IMPLICIT_DUNDERS` omits are closed
/// on the dataclass path too, because the universal guard runs in every class
/// body. `__init__` is deliberately absent here -- it is owned by
/// `DATACLASS_IMPLICIT_DUNDERS`, whose check runs first and whose message
/// D-235 pinned.
#[test]
fn a_dataclass_class_var_named_new_or_init_subclass_is_rejected() {
    for (name, fragment) in [
        ("__new__", "does not model `__new__` at all"),
        (
            "__init_subclass__",
            "walking the MRO's function definitions",
        ),
    ] {
        assert_capability_error_message(
            &format!(
                "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    {name}: ClassVar[int] = 8\n"
            ),
            fragment,
        );
    }
}

/// #913/D-235 regression pin: `__init__` in a `@dataclass` body must keep
/// reporting D-235's own message, not the new universal one. This is what
/// keeping the two name sets disjoint buys, and it is the single most likely
/// thing to break if a future change merges them.
#[test]
fn a_dataclass_class_var_named_init_still_reports_the_d235_message() {
    assert_capability_error_message(
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    __init__: ClassVar[int] = 8\n",
        "a `@dataclass` class implicitly relies on `__init__`",
    );
}

/// #975: the `Enum` member loop is the third route into a class-level
/// binding. One enum class per name -- the loop returns on the first `?`, so
/// a single body listing all three would silently test only `__init__`.
#[test]
fn an_enum_member_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(
            &format!("from enum import Enum\n\n\nclass C(Enum):\n    {name} = 1\n    B = 2\n"),
            fragment,
        );
    }
}

/// #975: the guard fires on the declaring class's own body, before any
/// subclass is seen. `__init_subclass__` diverges only once `class B(A)`
/// exists, but the class-body walk has no whole-program information, so the
/// rejection is reported at `A` -- pin that, so a future reader does not
/// expect a `B`-site diagnostic.
#[test]
fn an_inherited_reserved_attribute_is_rejected_on_the_declaring_class() {
    assert_capability_error(
        "from typing import ClassVar\n\n\nclass A:\n    __init_subclass__: ClassVar[int] = 8\n\n\nclass B(A):\n    pass\n",
        "walking the MRO's function definitions",
        Span { start: 43, end: 79 },
    );
}

/// #975 negative: the five names D-235 lists that are *not* in the universal
/// set stay accepted outside a dataclass, plus `__hash__`. Their safety rests
/// on every consuming rewrite being `is_dataclass`-gated; if that gating is
/// ever removed, this test is the thing that should start failing.
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
        let module = pycc_parser_test_helper::parse(&format!(
            "from typing import ClassVar\n\n\nclass C:\n    {name}: ClassVar[int] = 8\n"
        ));
        lower_checked(&module)
            .unwrap_or_else(|e| panic!("`{name}` must still lower in a plain class body: {e:?}"));
    }
}

/// The #975 pin, inverted by [#979](https://github.com/rotnov/pycc/issues/979)
/// (D-238). Until then this test asserted that an `Enum` member named after a
/// dunder outside D-236's set was still lowered as an ordinary member, and its
/// own comment recorded that as a *divergence* rather than agreement: the #978
/// review round measured that CPython 3.13.9's `_EnumDict` keeps every dunder
/// out of the member list, so the program below had two members here and one
/// there, and `C.__repr__.value` printed `1` here where CPython raises
/// `AttributeError`.
///
/// #979 closes that divergence by rejecting the whole `_EnumDict` non-member
/// family on the `Enum` route, so the program is now `C0001`. D-236's own set
/// and messages are untouched -- the name is rejected by the third check in
/// `reject_reserved_class_attr_name`, not by D-236's, which is what
/// `an_enum_member_named_after_the_instantiation_protocol_is_rejected` and
/// `the_enum_slots_message_describes_the_enum_route` still pin. The rest of
/// the family lives in `super::enum_non_member_names`.
#[test]
fn an_enum_member_named_after_an_unreserved_dunder_is_rejected() {
    assert_capability_error_message(
        "from enum import Enum\n\n\nclass C(Enum):\n    __repr__ = 1\n    B = 2\n",
        "a dunder-named assignment in an `Enum` body is not supported yet",
    );
}

/// #910 regression pin: `__slots__` keeps its own message and is not absorbed
/// into the new set. Since #1368 only the value-less spelling reaches it.
#[test]
fn the_slots_message_is_unchanged() {
    assert_capability_error_message(
        "class C:\n    __slots__: int\n",
        "this `__slots__` spelling is not supported yet",
    );
}

/// Review round on #978: `__slots__` in an `Enum` body is a fourth shape the
/// shared guard made reachable, and D-154's "the layout is fixed from
/// `__init__`" explanation is false there -- `lower_enum_class` produces no
/// `__init__` and no instance layout at all. Pin the route-specific message,
/// and pin that the plain-class explanation is *not* what gets rendered.
///
/// CPython 3.13.9 accepts this program (`_EnumDict` keeps a dunder out of the
/// member list, so `C.__slots__` is `'x'` and `list(C)` is `[C.A]`), so the
/// rejection is conservative -- the message says so rather than claiming a
/// measured divergence.
#[test]
fn the_enum_slots_message_describes_the_enum_route() {
    let source = "from enum import Enum\n\n\nclass C(Enum):\n    __slots__ = \"x\"\n    A = 1\n";
    assert_capability_error_message(source, "`__slots__` in an `Enum` body is not supported yet");

    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert!(
        !diagnostic
            .message
            .contains("only a value-bound `__slots__`"),
        "the enum route must not borrow D-154's plain-class explanation, got: {}",
        diagnostic.message
    );
}

/// The empty-tuple shape, which CPython also accepts (`E.__slots__ == ()`,
/// `list(E) == [E.A]`). Without the guard it would be rejected anyway, but for
/// the unrelated reason that `()` is not an `int`/`str` literal member value --
/// so pin that the reserved-name guard wins and the `__slots__` message is what
/// a reader sees.
#[test]
fn an_empty_slots_tuple_in_an_enum_body_reports_the_slots_message() {
    assert_capability_error_message(
        "from enum import Enum\n\n\nclass C(Enum):\n    __slots__ = ()\n    A = 1\n",
        "`__slots__` in an `Enum` body is not supported yet",
    );
}

/// #975 precedence pin: the new guard runs during the class-body walk, at the
/// *head* of D-235's pinned four-deep precedence, so it wins over
/// `reject_class_attr_collisions` when a class both defines `def __init__` and
/// binds `__init__` as an attribute. Pin it so a future reorder fails here
/// rather than silently changing which defect a two-defect program reports.
#[test]
fn the_reserved_name_guard_precedes_the_method_collision_check() {
    assert_capability_error_message(
        "from typing import ClassVar\n\n\nclass C:\n    __init__: ClassVar[int] = 8\n\n    def __init__(self) -> None:\n        pass\n",
        "resolves a class's constructor from its methods alone",
    );
}

/// #978 review round: the `@property` getter arm is the *fourth* class-body
/// route to a class-level binding of one of these names, and the only one
/// that does not pass through `reject_reserved_class_attr_name`.
/// `classify_decorator` routes `@property def __new__` to
/// `MethodKind::PropertyGetter`, so before this guard existed none of the
/// three attribute routes saw it: `ensure_init` synthesized a constructor
/// from the method table alone and pycc accepted `C()` while CPython 3.13.9
/// raised `TypeError: 'property' object is not callable` there.
///
/// The three names are not equally affected today, which is why each is
/// pinned by *message* rather than merely by rejection. Only `__new__` was a
/// D-198 false acceptance; `__init__` was already rejected as `T0021`
/// ("cannot redefine function `C.__init__` with a different signature") and
/// `__init_subclass__` as an unrelated `C0001` about the MRO hook needing to
/// be statically evaluable. Both of those described the wrong defect, so this
/// test fails if either reverts to its old diagnostic.
#[test]
fn a_property_getter_named_after_the_instantiation_protocol_is_rejected() {
    for (name, fragment) in RESERVED {
        assert_capability_error_message(
            &format!("class C:\n    @property\n    def {name}(self) -> int:\n        return 1\n"),
            fragment,
        );
    }
}

/// #978 review round, negative half: only the *property spelling* is
/// rejected on this route. A plain `def __init__` is a
/// `MethodKind::Regular` and must keep lowering -- a guard that keyed on the
/// method name instead of the method kind would reject every constructor in
/// the language.
#[test]
fn a_plain_init_method_is_not_rejected_by_the_property_route() {
    let module =
        pycc_parser_test_helper::parse("class C:\n    def __init__(self) -> None:\n        pass\n");
    lower_checked(&module).expect("a plain `def __init__` must still lower");
}

/// #978 review round, negative half: an ordinary `@property` is untouched.
/// This is the `None` arm of `instantiation_protocol_message` inside
/// `reject_reserved_property_name` -- the *protocol* half of that guard must
/// not grow past its three names. `__slots__` is rejected on the same route
/// since #980, but by a separate check ahead of that match, under its own
/// message; `a_benign_dunder_property_getter_stays_accepted` below is the
/// corresponding negative for it.
#[test]
fn an_ordinary_property_getter_is_not_rejected() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    @property\n    def value(self) -> int:\n        return 7\n",
    );
    lower_checked(&module).expect("`@property def value` must still lower");
}

/// #980: `@property def __slots__` is rejected on this route, under its own
/// message. Its divergence is real but different in mechanism from the
/// protocol names' -- CPython 3.13.9 raises `TypeError: 'property' object is
/// not iterable` while the `class` statement itself executes, because
/// `type.__new__` iterates `__slots__`, so the class is never created --
/// while pycc lowered the getter as an ordinary property and ran the program
/// (a D-198 false acceptance, measured at `f8e9d2e3`).
///
/// Pinned from both sides, like `the_enum_slots_message_describes_the_enum_route`:
/// the property-route needle must be present, and the plain attribute route's
/// D-154 explanation must be absent. Substring assertions alone would pass if
/// a future change wired this route through `slots_message(Plain)`, which is
/// exactly the false account #980 was opened to avoid.
#[test]
fn a_property_getter_named_slots_is_rejected() {
    let source = "class C:\n    @property\n    def __slots__(self) -> int:\n        return 1\n";
    assert_capability_error_message(
        source,
        "a `@property` getter named `__slots__` is not supported yet",
    );

    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert!(
        !diagnostic
            .message
            .contains("only a value-bound `__slots__`"),
        "the property route must not borrow D-154's plain-class explanation, got: {}",
        diagnostic.message
    );
}

/// #980, companion pin: the same getter in a `@dataclass` body reaches the
/// same arm and reports the same message. `super::super::body`'s dataclass
/// pre-check matches only `__init__`, `__eq__` and `__repr__`, so `__slots__`
/// falls through to the property guard rather than to D-235's set.
#[test]
fn a_dataclass_property_getter_named_slots_is_rejected() {
    assert_capability_error_message(
        "from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    x: int\n\n    @property\n    def __slots__(self) -> int:\n        return 1\n",
        "a `@property` getter named `__slots__` is not supported yet",
    );
}

/// #980, negative half: a benign dunder property stays accepted. `__doc__` is
/// measured to agree on both engines (CPython 3.13.9 and pycc both run
/// `@property def __doc__(self) -> int: return 1` and print `1`), so the new
/// branch must not grow into "reject every dunder property".
#[test]
fn a_benign_dunder_property_getter_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    @property\n    def __doc__(self) -> int:\n        return 1\n",
    );
    lower_checked(&module).expect("`@property def __doc__` must still lower");
}

/// #982: `@property def __qualname__` is **not** routed through this guard.
/// It diverges too -- CPython 3.13.9 raises `TypeError: type __qualname__
/// must be a str, not property` at class creation while pycc runs the program
/// -- but on the *attribute* route the same name is **value-typed**:
/// `__qualname__: int = 1` diverges while `__qualname__: str = "D"` agrees on
/// both engines. D-236 pins this guard as value-independent, so closing only
/// the property half would leave the name half-closed under a rule that does
/// not generate it; the full fix needs a third generating rule and its own
/// decision entry. Tracked as
/// [#982](https://github.com/rotnov/pycc/issues/982). Pin the current
/// acceptance so that issue's fix has to invert this test deliberately.
#[test]
fn a_property_getter_named_qualname_is_left_to_issue_982() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    @property\n    def __qualname__(self) -> int:\n        return 1\n",
    );
    lower_checked(&module)
        .expect("`@property def __qualname__` is out of D-236's scope until #982 is fixed");
}

/// #978 review round: a *callable* `__new__` -- `def __new__(self) -> int` --
/// is likewise **not** routed through this guard. D-236 rejects binding one of
/// the three names to a **non-callable** object, which is the literal text of
/// all three messages; a `def` binds exactly what CPython's protocol expects,
/// so none of them describes this defect. It is still a divergence: measured
/// at `1095b427`, CPython 3.13.9 binds `__new__`'s return value, so
/// `c = C(); c.f()` raises `AttributeError: 'int' object has no attribute
/// 'f'`, while pycc's `ensure_init` synthesizes a constructor from the method
/// table alone and prints `1`. Closing it needs a fourth message, a call site
/// keyed on [`MethodKind::Regular`], and a decision on the implicitly-static
/// `@staticmethod def __new__` twin, so it is tracked as
/// [#981](https://github.com/rotnov/pycc/issues/981). Pin the current
/// acceptance so that issue's fix has to invert this test deliberately.
#[test]
fn a_plain_new_method_is_left_to_issue_981() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    def __new__(self) -> int:\n        return 7\n",
    );
    lower_checked(&module).expect("`def __new__` is out of D-236's scope until #981 is fixed");
}

/// #984: the carrier CPython names for each non-`@property` `def __slots__`
/// spelling, as `(source prefix, carrier)`.
///
/// Measured at `e77b4b13` against CPython 3.13.9: a bare `def` and an
/// `@abstractmethod` both bind a plain `function`, while `@staticmethod` and
/// `@classmethod` bind their own wrapper objects. Those three strings are the
/// whole reachable range of `method_slots_carrier`, which is why it has two
/// named arms and one wildcard and no more -- a fourth arm would be a dead
/// region under D-014's 100% region gate.
///
/// `@override def __slots__` is deliberately absent, and no test for it can be
/// written that would not pass for the wrong reason: `@override` requires a
/// base class declaring the same name, and that base's own `def __slots__` is
/// rejected by this guard first, at the *base's* line. It folds into the same
/// wildcard arm as the bare `def` in any case.
const SLOTS_METHOD_SPELLINGS: [(&str, &str); 3] = [
    (
        "    def __slots__(self) -> int:\n        return 1\n",
        "function",
    ),
    (
        "    @staticmethod\n    def __slots__() -> int:\n        return 1\n",
        "staticmethod",
    ),
    (
        "    @classmethod\n    def __slots__(cls) -> int:\n        return 1\n",
        "classmethod",
    ),
];

/// The account the #984 message must *not* borrow: the plain attribute route's
/// explanation (#1368; D-154's before it), which describes an unmodelled
/// spelling on a class that is created rather than a class creation that fails.
const D154_FRAGMENT: &str = "only a value-bound `__slots__`";

/// Asserts the #984 message from both sides: the carrier-specific needle is
/// present and D-154's instance-layout account is absent. A `contains`-only
/// assertion would still pass if a future change wired this route through
/// `slots_message(Plain)`, which is exactly the false account #984 exists to
/// avoid.
fn assert_method_slots_message(source: &str, carrier: &str) {
    assert_capability_error_message(source, &format!("a `{carrier}` object is not iterable"));
    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();
    assert!(
        diagnostic
            .message
            .starts_with("a `def __slots__` in a class body is not supported yet"),
        "the method route must use its own #984 message, got: {}",
        diagnostic.message
    );
    assert!(
        !diagnostic.message.contains(D154_FRAGMENT),
        "the method route must not borrow D-154's plain-class explanation, got: {}",
        diagnostic.message
    );
}

/// #984: every non-`@property` `def __slots__` spelling in a plain class body
/// is rejected, each naming the carrier type CPython itself names.
///
/// Measured at `e77b4b13`: all three compiled and ran under pycc (`check`
/// exit 0, `run` prints `1`) while CPython 3.13.9 raised
/// `TypeError: '<carrier>' object is not iterable` at class creation and never
/// created the class -- the D-198 false acceptance D-224 forbids. The
/// divergence is value-independent: none of the three carriers is ever
/// iterable, whatever the method returns.
#[test]
fn a_method_named_slots_is_rejected_in_a_plain_class() {
    for (spelling, carrier) in SLOTS_METHOD_SPELLINGS {
        assert_method_slots_message(&format!("class C:\n{spelling}"), carrier);
    }
}

/// #984: an `@abstractmethod` spelling on an `ABC` reaches the same wildcard
/// carrier arm as the bare `def` and reports `function` too (measured: CPython
/// 3.13.9 raises `TypeError: 'function' object is not iterable`, from
/// `<frozen abc>`'s `__new__`). Pinned separately from the bare `def` because
/// it is a different `MethodKind` reaching the same arm.
#[test]
fn an_abstract_method_named_slots_is_rejected() {
    assert_method_slots_message(
        "from abc import ABC, abstractmethod\n\n\nclass C(ABC):\n    @abstractmethod\n    def __slots__(self) -> int:\n        ...\n",
        "function",
    );
}

/// #984, companion pin: the same bare `def` in a `@dataclass` body reaches the
/// same arm and reports the same message. `super::super::body`'s dataclass
/// pre-check matches only `__init__`, `__eq__` and `__repr__`, so `__slots__`
/// falls through to the reserved-name guard rather than to D-235's set.
#[test]
fn a_dataclass_method_named_slots_is_rejected() {
    assert_method_slots_message(
        "from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    x: int\n\n    def __slots__(self) -> int:\n        return 1\n",
        "function",
    );
}

/// #984: a `Protocol` body is a *separate* call site, not the same one. A
/// protocol class returns through `super::super::protocol::lower_protocol_class`
/// before the method loop runs at all, so a guard placed only in
/// `super::super::body` would leave this shape falsely accepted (measured at
/// `e77b4b13`: `pycc check` exit 0, CPython 3.13.9 `TypeError: 'function'
/// object is not iterable`).
#[test]
fn a_protocol_method_named_slots_is_rejected() {
    assert_method_slots_message(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    def __slots__(self) -> int:\n        ...\n",
        "function",
    );
}

/// #984, precedence pin for the `Protocol` call site: the new check is placed
/// *after* every rejection that arm already had, so an implementation-bodied
/// protocol method named `__slots__` still reports the declaration-style-body
/// diagnostic rather than the new one. Only the shape that is falsely accepted
/// today changes its answer.
#[test]
fn a_protocol_slots_method_with_a_body_still_reports_the_declaration_rule() {
    assert_capability_error_message(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    def __slots__(self) -> int:\n        return 1\n",
        "must have a declaration-style body",
    );
}

/// #984, precedence pin for the `PropertySetter` short-circuit. A
/// `@__slots__.setter` with no getter *does* reach the guard --
/// `classify_decorator` only requires the setter's own `def` name to match the
/// decorated property name, and the "requires a preceding getter" rejection
/// lives after the guard's call site -- so without the short-circuit the new
/// message would preempt it. That would be false in every clause: measured at
/// `e77b4b13` pycc already rejected this program, and CPython 3.13.9 raises
/// `NameError: name '__slots__' is not defined` while evaluating the decorator
/// expression, not a `TypeError` about a non-iterable object.
#[test]
fn a_slots_setter_without_a_getter_keeps_the_missing_getter_message() {
    assert_capability_error_message(
        "class C:\n    @__slots__.setter\n    def __slots__(self, v: int) -> None:\n        pass\n",
        "requires a preceding `@property` getter",
    );
}

/// #984, negative half: a benign dunder *method* stays accepted. `def
/// __doc__(self) -> int` runs and prints `1` under both engines (measured on
/// CPython 3.13.9), so the guard must not grow into "reject every dunder
/// method". This is also the `Ok(())` arm of `reject_reserved_method_name` and
/// of `reject_reserved_protocol_method_name`.
#[test]
fn a_benign_dunder_method_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "class C:\n    def __doc__(self) -> int:\n        return 1\n",
    );
    lower_checked(&module).expect("`def __doc__` must still lower");
}

/// #984, negative half for the protocol route specifically: the same benign
/// dunder in a `Protocol` body stays accepted too, covering
/// `reject_reserved_protocol_method_name`'s `Ok(())` arm from its own call
/// site rather than only through the plain-class one.
#[test]
fn a_benign_dunder_protocol_method_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "from typing import Protocol\n\n\nclass P(Protocol):\n    def __doc__(self) -> int:\n        ...\n",
    );
    lower_checked(&module).expect("`def __doc__` in a protocol body must still lower");
}
