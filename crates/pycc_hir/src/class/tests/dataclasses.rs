//! `@dataclass` lowering and its refusals.

use super::*;

// -- #378 (PR-18): dataclass (PEP 557) lowering -----------------------

#[test]
fn a_dataclass_with_two_fields_lowers_successfully() {
    let hir = lower_ok("@dataclass\nclass Point:\n    x: int\n    y: int\n");
    assert_eq!(hir.class_defs.len(), 1);
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.is_dataclass);
    assert_eq!(
        class_def.dataclass_fields,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
    );
    // __init__, __eq__, and __repr__ should be auto-generated.
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__init__"));
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__eq__"));
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__repr__"));
}

#[test]
fn a_dataclass_with_docstring_lowers_successfully() {
    // #744: a class docstring (a bare string-literal expression
    // statement) is a no-op in a dataclass body, not a field or method.
    let hir = lower_ok("@dataclass\nclass Point:\n    \"A point.\"\n    x: int\n    y: int\n");
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.is_dataclass);
    assert_eq!(
        class_def.dataclass_fields,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
    );
}

#[test]
fn a_dataclass_with_a_non_leading_docstring_lowers_successfully() {
    // #744's guard has no position check: place the docstring after a
    // field to exercise the non-leading case directly.
    let hir = lower_ok("@dataclass\nclass Point:\n    x: int\n    \"A point.\"\n    y: int\n");
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.is_dataclass);
    assert_eq!(
        class_def.dataclass_fields,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
    );
}

#[test]
fn a_dataclass_transform_decorator_lowers_successfully() {
    let hir = lower_ok("@dataclass_transform()\nclass Point:\n    x: int\n");
    assert_eq!(hir.class_defs.len(), 1);
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.is_dataclass);
}

#[test]
fn a_dataclass_with_inheritance_merges_parent_fields() {
    let hir = lower_ok(
        "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    b: int\n",
    );
    assert_eq!(hir.class_defs.len(), 2);
    let (_, derived_def) = &hir.class_defs[1];
    assert!(derived_def.is_dataclass);
    // Parent field `a` comes before child field `b`.
    assert_eq!(
        derived_def.dataclass_fields,
        vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int),]
    );
    assert_eq!(
        derived_def.attrs,
        vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int),]
    );
}

#[test]
fn a_dataclass_child_redeclaring_a_parent_field_deduplicates() {
    // When a child dataclass redeclares a field already present in a
    // parent with the *same* type, the merge keeps the parent's field
    // and skips the child's duplicate rather than treating it as a
    // conflict (see the differing-type case rejected as T0052 below).
    let hir = lower_ok(
        "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    a: int\n",
    );
    assert_eq!(hir.class_defs.len(), 2);
    let (_, derived_def) = &hir.class_defs[1];
    assert!(derived_def.is_dataclass);
    // The field `a` appears only once (from the parent).
    assert_eq!(
        derived_def.dataclass_fields,
        vec![("a".to_string(), Ty::Int)]
    );
}

#[test]
fn a_dataclass_grandchild_merges_parent_and_grandparent_fields_with_dedup() {
    // A 3-level inheritance chain where the middle class redeclares a
    // grandparent field with the *same* type. When processing the
    // grandchild, the MRO-based merge iterates over the grandparent's
    // fields first (adding `x`), then the middle class's fields. The
    // middle class's `x` is already in `merged_fields` with a matching
    // type, so it is deduplicated rather than rejected. This covers the
    // same-type "already present" branch in the MRO-based parent-fields
    // merge loop.
    let hir = lower_ok(
        "@dataclass\nclass A:\n    x: int\n@dataclass\nclass B(A):\n    x: int\n\
         @dataclass\nclass C(B):\n    y: int\n",
    );
    assert_eq!(hir.class_defs.len(), 3);
    let (_, c_def) = &hir.class_defs[2];
    assert!(c_def.is_dataclass);
    // `x` (from A, deduplicated against B's redeclaration) comes before
    // `y` (C's own field).
    assert_eq!(
        c_def.dataclass_fields,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
    );
}

#[test]
fn a_dataclass_field_type_conflict_across_mro_is_rejected_via_unit_test() {
    // Mirrors `pycc_types::tests::dataclass_field_type_conflict_across_mro_is_rejected_during_hir_lowering`
    // inside this crate's own unit-test binary, working around
    // cargo-llvm-cov issue #276 (instantiation-merge gap between
    // the library and integration-test binaries): a dataclass field
    // redeclared with a *differing* type across the MRO must be
    // rejected with T0052, exercising the second merge loop's
    // conflicting-type arm.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "@dataclass\nclass Base:\n    v: int\n@dataclass\nclass Derived(Base):\n    v: str\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "T0052");
    assert!(
        diagnostic.message.contains('v')
            && diagnostic.message.contains("int")
            && diagnostic.message.contains("str"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_dataclass_field_type_conflict_between_two_bases_is_rejected_via_unit_test() {
    // Mirrors `pycc_types::tests::dataclass_field_type_conflict_between_two_bases_is_rejected_during_hir_lowering`
    // inside this crate's own unit-test binary, for the same
    // cargo-llvm-cov instantiation-merge reason as the sibling test
    // above: two independent dataclass bases declaring the same
    // field with differing types must be rejected with T0052,
    // exercising the first merge loop's conflicting-type arm.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "@dataclass\nclass A:\n    v: int\n@dataclass\nclass B:\n    v: str\n@dataclass\nclass Derived(A, B):\n    pass\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "T0052");
    assert!(
        diagnostic.message.contains('v')
            && diagnostic.message.contains("int")
            && diagnostic.message.contains("str"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_call_decorator_with_a_non_name_callee_is_rejected() {
    // `@some.attr()` -- a call whose callee is an attribute access, not
    // a bare name. `classify_class_decorator`'s `Expr::Call` arm hits
    // the `let Expr::Name(name) = call.func.as_ref() else { ... }`
    // rejection.
    assert_c0001("@some.attr()\nclass C:\n    x: int\n");
}

#[test]
fn a_call_decorator_with_an_unknown_name_is_rejected() {
    // `@other()` -- a call whose callee is a bare name but not
    // `dataclass_transform` or `dataclass`.
    assert_c0001("@other()\nclass C:\n    x: int\n");
}

#[test]
fn a_dataclass_with_duplicate_field_names_is_rejected() {
    assert_c0001("@dataclass\nclass C:\n    x: int\n    x: int\n");
}

#[test]
fn a_dataclass_with_an_unsupported_annotation_is_rejected() {
    // `annotation_to_ty` returns an error for `undefined_type`, which
    // propagates through the `?` on the `annotation_to_ty(...)` call.
    assert_c0001("@dataclass\nclass C:\n    x: undefined_type\n");
}

#[test]
fn a_dataclass_with_a_non_method_non_annassign_statement_is_rejected() {
    // #910 accepts a bare `y = 5` as an inferred class attribute, but
    // only outside a `@dataclass`: in one, a bare assignment is a
    // class-level default for a field declared elsewhere, not a
    // constant, so it still falls through to the catch-all. This is the
    // one site exercising that gate's `is_dataclass` arm.
    assert_c0001("@dataclass\nclass C:\n    x: int\n    y = 5\n");
}

#[test]
fn a_zero_field_dataclass_lowers_successfully() {
    let hir = lower_ok("@dataclass\nclass Empty:\n    pass\n");
    assert_eq!(hir.class_defs.len(), 1);
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.is_dataclass);
    assert!(class_def.dataclass_fields.is_empty());
    // __init__, __eq__, and __repr__ should still be auto-generated.
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__init__"));
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__eq__"));
    assert!(class_def.methods.iter().any(|(mn, _)| mn == "__repr__"));
}

#[test]
fn a_bare_field_call_in_a_dataclass_is_rejected() {
    // `x: int = field()` -- a bare `field()` call with no arguments.
    assert_c0001("@dataclass\nclass C:\n    x: int = field()\n");
}

#[test]
fn a_dataclass_inheriting_from_a_non_dataclass_is_rejected() {
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class Base:\n    def __init__(self) -> None:\n        return\n@dataclass\nclass Derived(Base):\n    b: int\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot inherit from non-dataclass class `Base`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_explicit_init_in_a_dataclass_is_rejected() {
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "@dataclass\nclass C:\n    x: int\n    def __init__(self, x: int) -> None:\n        self.x = x\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("auto-generates `__init__`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_explicit_eq_in_a_dataclass_is_rejected() {
    assert_c0001(
        "@dataclass\nclass C:\n    x: int\n    def __eq__(self, other: C) -> bool:\n        return self.x == other.x\n",
    );
}

#[test]
fn an_explicit_repr_in_a_dataclass_is_rejected() {
    assert_c0001(
        "@dataclass\nclass C:\n    x: int\n    def __repr__(self) -> str:\n        return \"C\"\n",
    );
}

#[test]
fn multiple_decorators_on_a_dataclass_are_rejected() {
    // #380 (PR-20): `@dataclass` combined with `@runtime_checkable`
    // is rejected — a protocol class cannot be a dataclass.
    assert_c0001("@dataclass\n@runtime_checkable\nclass C:\n    x: int\n");
}

#[test]
fn a_dataclass_with_options_is_rejected() {
    assert_c0001("@dataclass(frozen=True)\nclass C:\n    x: int\n");
}

#[test]
fn a_non_name_field_target_in_a_dataclass_is_rejected() {
    assert_c0001("@dataclass\nclass C:\n    self.x: int\n");
}

#[test]
fn a_plain_default_in_a_dataclass_field_is_rejected() {
    assert_c0001("@dataclass\nclass C:\n    x: int = 42\n");
}

#[test]
fn a_field_default_in_a_dataclass_is_rejected() {
    assert_c0001("@dataclass\nclass C:\n    x: int = field(default=0)\n");
}

#[test]
fn a_field_default_factory_in_a_dataclass_is_rejected() {
    assert_c0001("@dataclass\nclass C:\n    x: int = field(default_factory=int)\n");
}

// -- #378 scalar-field restriction: non-scalar types are rejected -----

#[test]
fn a_self_referential_dataclass_field_is_rejected() {
    // `next: Node` resolves to `Ty::Instance("Node")` via
    // `annotation_to_ty`'s self-referential class-name arm. Without
    // the scalar-slot-type check, this panics in codegen (the
    // attribute-slot storage is a single word, with no representation
    // for a class instance). The check rejects it with C0001 instead.
    assert_c0001("@dataclass\nclass Node:\n    next: Node\n");
}

#[test]
fn a_self_typed_dataclass_field_is_rejected() {
    // `next: Self` resolves to `Ty::Instance("Node")` via
    // `annotation_to_ty`'s `Self` arm (PEP 673). Same root cause as
    // the self-referential class-name case above.
    assert_c0001("@dataclass\nclass Node:\n    next: Self\n");
}

#[test]
fn a_none_typed_dataclass_field_is_rejected() {
    // `x: None` resolves to `Ty::None` via `annotation_to_ty`'s
    // `NoneLiteral` arm. `None` is not a scalar slot type.
    assert_c0001("@dataclass\nclass C:\n    x: None\n");
}

#[test]
fn a_cross_class_instance_dataclass_field_is_rejected() {
    // `x: Other` in class `C` is rejected by `annotation_to_ty` itself
    // (the self-referential class-name arm only matches the enclosing
    // class's own name, not other classes; `Other` is not a builtin
    // type or alias, so it falls through to "type annotation `Other`
    // is not supported yet"). This is still C0001, but reached before
    // the scalar-slot-type check -- included here to document that
    // cross-class instance fields are rejected regardless of which
    // guard fires first.
    assert_c0001("@dataclass\nclass Other:\n    pass\n@dataclass\nclass C:\n    x: Other\n");
}

#[test]
fn a_dataclass_field_with_a_non_scalar_type_gives_a_clear_message() {
    // Verify the diagnostic message names the field and its type, so
    // the user can identify which field and what type caused the
    // rejection.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "@dataclass\nclass Node:\n    next: Node\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("dataclass field `next`"),
        "unexpected message: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("not a scalar slot type"),
        "unexpected message: {}",
        diagnostic.message
    );
}
