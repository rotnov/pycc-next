//! `@override`, inherited `__init__` and `__init_subclass__` hooks.

use super::*;

#[test]
fn override_on_a_valid_method_lowers_successfully() {
    let hir = lower_ok(
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def f(self) -> int:\n        return 2\n",
    );
    let (_, b_def) = &hir.class_defs[1];
    assert!(b_def.methods.iter().any(|(name, _)| name == "f"));
}

#[test]
fn override_on_a_nonexistent_method_is_t0031() {
    let module = crate::pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def g(self) -> int:\n        return 2\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "T0031");
    assert!(
        diagnostic.message.contains("does not override"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn override_on_a_property_is_valid() {
    let hir = lower_ok(
        "class A:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def x(self) -> int:\n        return 42\n",
    );
    let (_, b_def) = &hir.class_defs[1];
    // The @override-decorated method `x` is a regular method (not a
    // property), and it overrides the property `x` from `A`.
    assert!(b_def.methods.iter().any(|(name, _)| name == "x"));
}

#[test]
fn a_derived_class_without_init_inherits_base_init() {
    let hir = lower_ok(
        "class A:\n    def __init__(self, x: int) -> None:\n        self.x = x\nclass B(A):\n    def f(self) -> int:\n        return self.x\n",
    );
    let (_, b_def) = &hir.class_defs[1];
    // B has no __init__ of its own, but inherits A's.
    assert!(!b_def.methods.iter().any(|(name, _)| name == "__init__"));
    assert_eq!(b_def.mro, vec!["B".to_string(), "A".to_string()]);
}

#[test]
fn a_class_without_init_and_without_base_init_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def f(self) -> int:\n        return 1\n",
    );
    // `B` has no `__init__` and its base `A` has one, so this should
    // succeed (B inherits A's `__init__`). But `B`'s own `f` method
    // means it has a valid class body. The test name is historical --
    // it originally tested `pass` being rejected, but #378 made `pass`
    // valid in class bodies. This now tests that a derived class with
    // a method but no `__init__` inherits the base's `__init__`.
    let hir = lower_checked(&module).unwrap();
    let b_def = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "B")
        .map(|(_, def)| def)
        .unwrap();
    assert!(
        !b_def.methods.iter().any(|(mn, _)| mn == "__init__"),
        "B should not have its own __init__"
    );
}

#[test]
fn a_class_with_no_bases_and_no_init_synthesizes_one() {
    // #912 (D-225): the base-less counterpart of the case above -- with
    // nothing in the MRO to inherit from, the constructor is synthesized
    // rather than the class being rejected.
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def f(self) -> int:\n        return 1\n",
    );
    let hir = lower_checked(&module).unwrap();
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "C")
        .expect("class `C` is lowered");
    assert!(
        class_def.methods.iter().any(|(mn, _)| mn == "__init__"),
        "the synthesized `__init__` joins the method table: {:?}",
        class_def.methods
    );
}

#[test]
fn a_non_name_base_class_expression_is_unsupported() {
    // #432: `class C(SomeMod.Base):` parses `SomeMod.Base` as an
    // `Expr::Attribute`, not an `Expr::Name` -- only a bare name is
    // supported as a base class for now.
    let module = crate::pycc_parser_test_helper::parse(
        "class C(SomeMod.Base):\n    def __init__(self) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a base class must be a bare name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

// -----------------------------------------------------------------------
// #435 (Part B): __init_subclass__ validation unit tests
// -----------------------------------------------------------------------

#[test]
fn init_subclass_with_pass_body_in_subclass_of_base_with_init_subclass_is_accepted() {
    // A base class B defines `__init_subclass__` with `pass`, and a
    // subclass D also defines its own `__init_subclass__` (also
    // `pass`). #854: the unified guard never inspects D's own
    // override -- it validates B's (the nearest ancestor's) body,
    // which exercises the `Stmt::Pass` arm of
    // `validate_init_subclass_body`. Kept as `pass`/`pass` so this
    // fixture continues to demonstrate "D's own override, whatever its
    // shape, plays no role" as directly as possible.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
    );
    let hir = lower_checked(&module);
    assert!(hir.is_ok(), "pass body should be accepted");
}

#[test]
fn init_subclass_with_docstring_in_subclass_of_base_with_init_subclass_is_accepted() {
    // #854: the unified guard validates the nearest ancestor's body,
    // never the subclass's own -- so the docstring is now placed on
    // B's `__init_subclass__` to exercise the `Stmt::Expr` +
    // `StringLiteral` arm; D's own docstring override is never
    // inspected by this guard at all.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        \"docstring\"\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
    );
    let hir = lower_checked(&module);
    assert!(hir.is_ok(), "docstring body should be accepted");
}

#[test]
fn init_subclass_without_base_having_init_subclass_is_not_validated() {
    // A class defines `__init_subclass__` but no base class has it.
    // The unconditional nearest-MRO-ancestor `find_map` walk finds no
    // ancestor defining the hook and returns `None`, so validation does
    // NOT run and even a non-trivial body is accepted (it's just a
    // regular method).
    let module = crate::pycc_parser_test_helper::parse(
        "class D:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\n",
    );
    let hir = lower_checked(&module);
    assert!(
        hir.is_ok(),
        "non-trivial body without base init_subclass should be accepted"
    );
}

#[test]
fn subclass_without_own_init_subclass_inherits_statically_evaluable_base_hook() {
    // A base class B defines `__init_subclass__` with a `pass` body,
    // and a subclass D does NOT define its own `__init_subclass__`.
    // D's methods don't include `__init_subclass__`, so this takes the
    // #585 inherited-hook path (re-validating B's body against D's
    // creation site) rather than the #435 own-hook path -- but since
    // B's body is statically evaluable, that re-validation still
    // succeeds.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let hir = lower_checked(&module);
    assert!(
        hir.is_ok(),
        "subclass inheriting a statically-evaluable __init_subclass__ should be accepted"
    );
}

#[test]
fn subclass_without_own_init_subclass_rejects_side_effecting_inherited_body() {
    // #585: a base class B defines `__init_subclass__` with a
    // side-effecting body, and a subclass D inherits it unchanged
    // (does not override it). CPython would invoke B's hook when D is
    // created; pycc cannot run that side-effecting body at
    // class-creation time, so this must now be rejected at D's
    // creation site -- unlike `init_subclass_without_base_having_init_subclass_is_not_validated`,
    // where B is never subclassed and therefore stays legal.
    // B's `__init_subclass__` is deliberately followed by `__init__`
    // (not the last statement in the body) so the reverse walk over
    // `base_ast.body` also exercises its `_ => {}` catch-all arm before
    // finding the match.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init_subclass__(self) -> None:\n        print(1)\n    def __init__(self) -> None:\n        return\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("inherited"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn base_alone_never_subclassed_with_side_effecting_init_subclass_stays_legal() {
    // #585's boundary case, restated explicitly: a base class defining
    // `__init_subclass__` with a side-effecting body must still compile
    // when it is never subclassed in this module -- CPython never
    // invokes the hook until a subclass actually exists, so rejecting
    // it here (at the base's own definition) would over-reject a
    // legal, standalone class. This is the same program shape as
    // `init_subclass_without_base_having_init_subclass_is_not_validated`
    // above; kept as its own #585-focused test for direct traceability
    // to the issue's own boundary requirement.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\n",
    );
    let hir = lower_checked(&module);
    assert!(
        hir.is_ok(),
        "a base class's own side-effecting __init_subclass__ must stay legal when it is \
         never subclassed"
    );
}

// -----------------------------------------------------------------------
// #854: unified nearest-ancestor guard -- new regression tests.
//
// Truth table for the four B/D triviality combinations (B = base's
// `__init_subclass__`, D = subclass's own override, if any):
//   B trivial   / D absent     -> accept (existing:
//     `subclass_without_own_init_subclass_inherits_statically_evaluable_base_hook`)
//   B side-effecting / D absent -> reject (existing:
//     `subclass_without_own_init_subclass_rejects_side_effecting_inherited_body`)
//   B trivial   / D trivial    -> accept (existing, above)
//   B trivial   / D side-effecting -> accept (D's own override is never
//     checked) -- Test D
//   B side-effecting / D trivial    -> reject (D's trivial override does
//     not save it -- B's body is still the one CPython invokes) -- Tests
//     A and B
// -----------------------------------------------------------------------

#[test]
fn side_effecting_base_hook_with_trivial_subclass_override_is_rejected() {
    // Test A: B's `__init_subclass__` does `return` (a `Stmt::Return`,
    // caught only by the `_ =>` catch-all arm in
    // `validate_init_subclass_body` -- every other rejecting test in
    // this suite uses a non-string `Stmt::Expr` such as `print(...)`,
    // which exercises a different arm). D(B) defines its own plain,
    // trivial override (`pass`). #854's bug: before the fix, D's own
    // trivial override made the (buggy) `if` branch validate D's body
    // instead of B's, silently accepting a program CPython would
    // reject at D's creation (B's side-effecting hook is the one
    // actually invoked). After the fix, B's body is what's validated
    // regardless of D's own override, so this is rejected.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        return\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__init_subclass__"));
}

#[test]
fn classmethod_decorated_ancestor_hook_with_trivial_subclass_override_is_rejected() {
    // Test B: the genuine #854 Gap-2 bug. B's `__init_subclass__` is
    // `@classmethod`-decorated and side-effecting (`print(1)`); D(B)
    // defines its own plain, trivial override (`pass`). Before the
    // fix, this was silently *accepted*: D's own hook is found in
    // `methods` (own-hook check true) -> enters the `if` branch ->
    // the `base_has_init_subclass` gate queries `cd.methods` on B,
    // finds nothing (a `@classmethod`-decorated hook lives in
    // `class_methods`, not `methods`) -> validation is skipped
    // entirely. CPython would invoke B's side-effecting hook at D's
    // creation. The unified guard scans B's raw AST by statement shape
    // (decorator-agnostic), so it now correctly finds and validates
    // B's body, rejecting this program.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__init_subclass__"));
}

#[test]
fn classmethod_decorated_ancestor_hook_with_no_subclass_override_is_rejected() {
    // Test C: regression lock for classmethod-ancestor detection when
    // the subclass defines no override at all. B's `__init_subclass__`
    // is `@classmethod`-decorated and side-effecting; D(B) defines no
    // `__init_subclass__`. This shape was already correctly rejected
    // before #854 (the pre-existing `else if`/#585 ancestor-lookup
    // path was always decorator-agnostic, since it scans raw AST by
    // statement shape); this test locks that behavior through the
    // refactor.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__init_subclass__"));
}

#[test]
fn trivial_base_hook_with_classmethod_decorated_subclass_override_is_accepted() {
    // Test D: regression lock -- a subclass's own `@classmethod`-
    // decorated override is correctly ignored. B's `__init_subclass__`
    // is trivial (`pass`); D(B)'s own override is `@classmethod`-
    // decorated and side-effecting. D's own override, decorated or
    // not, is never the hook CPython invokes at D's own creation (only
    // relevant if something later subclasses D), so this must be
    // accepted -- B's trivial body is the one validated. This shape
    // was already correctly accepted before #854, for an unrelated,
    // accidental reason (D's own hook lives in `class_methods`, which
    // bypassed the old `methods`-table own-hook check entirely,
    // routing into the ancestor-lookup path that validates B's
    // trivial body); this test locks in the now-*intentional* correct
    // behavior through the refactor.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\n",
    );
    let hir = lower_checked(&module);
    assert!(
        hir.is_ok(),
        "a subclass's own classmethod-decorated __init_subclass__ override must never be \
         the body validated at its own creation"
    );
}

#[test]
fn grandchild_validates_parents_own_override_not_grandparents_hook() {
    // Test E (revised from the posted plan -- see the D-214 write-up and
    // the session's own retrospective entry for the full reasoning):
    // the guard runs unconditionally at *every* class's own creation
    // against *that class's own* nearest ancestor, so a class sitting
    // under a side-effecting, introspectable hook is illegal at its own
    // creation regardless of what it overrides -- there is no way for a
    // legally-created three-level chain to have a side-effecting
    // grandparent shadowed by a trivial parent, since the parent's own
    // creation would already have been rejected by the grandparent's
    // hook. B's `__init_subclass__` is trivial (`pass`, satisfying D's
    // own creation, whose nearest ancestor is B); D(B)'s own override is
    // side-effecting (`print(1)`); Grandchild(D) defines no override.
    // Grandchild's ancestor lookup must hit D first (nearest ancestor
    // that defines the hook) and validate *D's own* body, not skip past
    // it to B's trivial one -- this locks in that a parent's own
    // override, not a grandparent's, is what a child's creation is
    // checked against.
    let module = crate::pycc_parser_test_helper::parse(
        "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        print(1)\nclass Grandchild(D):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__init_subclass__"));
}

#[test]
fn multiple_inheritance_nearest_mro_hook_wins_over_farther_side_effecting_one() {
    // Test E2: nearest-ancestor-in-MRO-order, not "any ancestor",
    // discriminated via multiple inheritance rather than a linear
    // chain (a linear chain cannot produce this shape -- see the
    // previous test's comment). `class D(M, B)` linearizes via C3 to
    // `[D, M, B, object]`. M's `__init_subclass__` is trivial (`pass`);
    // B's is side-effecting (`print(1)`); D defines no override of its
    // own. `skip(1)` must hit M first (nearer in D's own MRO) and
    // validate M's trivial body, never reaching B's side-effecting one
    // -- exactly the lookup CPython performs (`super(D, D)` searches
    // the MRO in order and stops at the first hit).
    let module = crate::pycc_parser_test_helper::parse(
        "class M:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\nclass D(M, B):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let hir = lower_checked(&module);
    assert!(
        hir.is_ok(),
        "the nearer ancestor in MRO order (M) must be validated, not the farther one (B), \
         even though both define the hook"
    );
}

#[test]
fn multiple_inheritance_skips_hookless_ancestor_to_reject_farther_side_effecting_one() {
    // Reject-direction sibling of
    // `multiple_inheritance_nearest_mro_hook_wins_over_farther_side_effecting_one`.
    // That test only proves the nearest hook-defining ancestor wins
    // when it happens to also be the immediate base -- every existing
    // fixture's `skip(1)` MRO candidate (position 1) either already
    // defines `__init_subclass__` or terminates the search entirely, so
    // a hypothetical regression that narrowed the lookup to just
    // `mro.get(1)` instead of a real "walk the whole MRO, skipping
    // classes that don't define the hook" search would still pass every
    // other test in this suite. This fixture closes that gap: `class M`
    // defines `__init__` only (introspectable, but no
    // `__init_subclass__` of its own -- the search must skip past it),
    // `class B` defines a side-effecting `__init_subclass__`, and
    // `class D(M, B)` linearizes via C3 to `[D, M, B, object]` with no
    // override of its own. The nearest-MRO-ancestor search must skip
    // hookless M and match B, rejecting on B's side-effecting body.
    let module = crate::pycc_parser_test_helper::parse(
        "class M:\n    def __init__(self) -> None:\n        return\nclass B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\nclass D(M, B):\n    def __init__(self) -> None:\n        super().__init__()\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("__init_subclass__"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn runtime_checkable_on_non_protocol_is_rejected_via_unit_test() {
    // Covers the `@runtime_checkable` on a non-protocol class error
    // path (lines 1082-1089) inside this crate's own unit-test binary.
    assert_c0001("@runtime_checkable\nclass C:\n    pass\n");
}
