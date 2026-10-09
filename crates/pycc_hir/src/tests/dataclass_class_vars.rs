//! `ClassVar` inside a `@dataclass` body (#913, [D-235]).
//!
//! [D-235]: ../../../../docs/decisions/D-235-reject-a-dataclass-field-that-shares-its-name-with-a.md
//!
//! Extracted from the parent `tests` module so that file does not keep
//! growing (AGENTS.md "Keep source files decomposable", the same reason
//! `subscript_annotations` was split out under #663). A child module sees
//! the parent's private helpers -- `assert_capability_error_message` and
//! the crate-root glob -- through `use super::*`.

use super::*;

/// #913: a `ClassVar` in a `@dataclass` body is a class attribute, not a
/// field. It must land in `class_attrs` and must be absent from both
/// `dataclass_fields` and `attrs` -- the latter is the D-154 slot layout,
/// which `lower_class` fills from the merged field list.
#[test]
fn a_class_var_in_a_dataclass_body_lowers_to_a_class_attribute() {
    let module = pycc_parser_test_helper::parse(
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    y: int\n    LIMIT: ClassVar[int] = 10\n",
    );
    let hir = lower_checked(&module).expect("the dataclass body must lower");
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "P")
        .expect("class `P` must be lowered");

    assert_eq!(
        class_def.class_attrs,
        vec![("LIMIT".to_string(), Ty::Int, ClassAttrValue::Int(10))]
    );
    assert_eq!(
        class_def.dataclass_fields,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)]
    );
    assert_eq!(
        class_def.attrs,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)]
    );
}

/// #913/D-235: a `ClassVar` named after a dunder the dataclass relies on
/// implicitly. CPython's `dataclasses` keeps the class attribute, so the
/// program either loses the synthesized method or raises
/// `TypeError: 'int' object is not callable` at the use site; pycc
/// synthesizes and rewrites unconditionally, so the spelling is rejected.
/// Every one of `body::DATACLASS_IMPLICIT_DUNDERS` is exercised
/// individually -- a single or-pattern arm (or one `contains` hit) is not
/// reliable evidence that the whole set is honoured (see
/// `class/init_slot.rs`'s own note on the `slot_ty_from_init_rhs` match).
#[test]
fn a_class_var_named_after_an_implicit_dataclass_dunder_is_rejected() {
    for name in [
        "__init__",
        "__eq__",
        "__repr__",
        "__ne__",
        "__str__",
        "__format__",
    ] {
        assert_capability_error_message(
            &format!("@dataclass\nclass C:\n    x: int\n    {name}: ClassVar[int] = 8\n"),
            &format!("a `ClassVar` named `{name}` is not allowed in a `@dataclass` body"),
        );
    }
}

/// #913/D-235: a dunder pycc does *not* rewrite for a dataclass stays a
/// legal `ClassVar`. `__hash__` is the boundary case: CPython's
/// `dataclasses` leaves an explicitly bound `__hash__` in the class
/// `__dict__` alone, so `P.__hash__` reads `8` under both implementations.
#[test]
fn a_class_var_named_after_a_dunder_pycc_does_not_rewrite_is_accepted() {
    let module = pycc_parser_test_helper::parse(
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    __hash__: ClassVar[int] = 8\n",
    );
    let hir = lower_checked(&module).expect("a non-rewritten dunder `ClassVar` must lower");
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "P")
        .expect("class `P` must be lowered");

    assert_eq!(
        class_def.class_attrs,
        vec![("__hash__".to_string(), Ty::Int, ClassAttrValue::Int(8))]
    );
}

/// #913: a dataclass field must not share its name with a `ClassVar`
/// declared in the same body, in either declaration order. CPython's two
/// orders disagree with each other (field-first drops the field;
/// `ClassVar`-first turns it into a field with a default), and neither is
/// representable while dataclass field defaults are unsupported.
#[test]
fn a_dataclass_field_colliding_with_an_own_class_var_is_rejected() {
    for source in [
        "@dataclass\nclass C:\n    x: int\n    x: ClassVar[int] = 1\n",
        "@dataclass\nclass C:\n    x: ClassVar[int] = 1\n    x: int\n",
    ] {
        assert_capability_error_message(
            source,
            "dataclass field `x` of class `C` shares its name with the class attribute `C.x`",
        );
    }
}

/// #913: the same check across the MRO -- an own field over a base's
/// `ClassVar`. CPython turns the base's value into the field's *default*,
/// so its `__init__` takes an optional parameter where pycc's synthesized
/// one would take a required one.
#[test]
fn a_dataclass_field_colliding_with_a_base_class_var_is_rejected() {
    assert_capability_error_message(
        "@dataclass\nclass A:\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B(A):\n    LIMIT: int\n",
        "dataclass field `LIMIT` of class `B` shares its name with the class attribute `A.LIMIT`",
    );
}

/// #913: the cross-base split, in the base order whose *mirror image* is a
/// mis-compile. `D(B, A)` reaches the second MRO base before it finds the
/// `ClassVar`, which is the only shape that exercises `class_attr_owner`'s
/// "this base does not declare it, keep walking" arm.
#[test]
fn a_dataclass_field_colliding_with_a_sibling_bases_class_var_is_rejected() {
    assert_capability_error_message(
        "@dataclass\nclass A:\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B:\n    LIMIT: int\n\n\n@dataclass\nclass D(B, A):\n    pass\n",
        "dataclass field `LIMIT` of class `D` shares its name with the class attribute `A.LIMIT`",
    );
}

/// #913: a `ClassVar` with no initializer stays `C0001` inside a
/// `@dataclass` body exactly as it does outside one. CPython allows it
/// (the name simply has no value), but #911's fold model needs a literal.
#[test]
fn a_value_less_class_var_in_a_dataclass_body_is_rejected() {
    assert_capability_error_message(
        "@dataclass\nclass C:\n    x: int\n    LIMIT: ClassVar[int]\n",
        "class attribute `LIMIT` has no value",
    );
}

/// #913: the diagnostic precedence the new field/class-attribute check
/// creates. The `T0052` field-type conflict is raised inside the merge
/// loop, which runs before the new check, so a program with both defects
/// keeps reporting `T0052`.
#[test]
fn a_dataclass_field_type_conflict_outranks_the_class_var_collision() {
    let module = pycc_parser_test_helper::parse(
        "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    v: int\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B(A):\n    v: float\n    LIMIT: int\n",
    );
    let diagnostic = lower_checked(&module).expect_err("the conflict must be reported");
    assert_eq!(diagnostic.code, "T0052");
}
