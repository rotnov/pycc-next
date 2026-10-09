//! PEP 435 (#379, PR-19) enum-class lowering: `lower_enum_class`, split out
//! of `class.rs` under AGENTS.md's "Keep source files decomposable" rule
//! (#892). The enum member-value model itself (`EnumMemberValue`) stays in
//! `class.rs` next to `HirClassDef`, which owns the field it populates, and
//! the `is_enum` detection and dispatch stay in `lower_class` -- only the
//! enum-body validation and member-value derivation live here.
//!
//! `lower_enum_class` keeps its extracted-standalone-function shape (rather
//! than being inlined back into `lower_class`) for the reason recorded on
//! the function itself: cargo-llvm-cov#276's instantiation-merge gap.

use crate::class::EnumMemberValue;
use crate::class::reserved_names::{ClassBodyRoute, reject_reserved_class_attr_name};
use crate::{HirClassDef, HirItem, Ty, unsupported};
use pycc_ast::{Expr, Number, Stmt};
use pycc_diag::Diagnostic;

/// #379 (PR-19): Lower a PEP 435-style enum class. An enum class body is
/// assignments only (`RED = 1`), not method definitions. This function
/// validates the enum body and constructs the `HirClassDef` with
/// `enum_members` populated. Extracted from `lower_class` to isolate the
/// enum-specific code paths (see cargo-llvm-cov#276 for the coverage
/// instantiation issue that motivated the extraction).
///
/// #892: `is_str_enum` is `true` when the class's marker base was
/// `StrEnum` rather than `Enum`. `lower_class` must capture it *before* it
/// clears `bases` (the marker base is consumed, not inherited), so it
/// arrives here as a parameter rather than being re-read from `bases`.
pub(super) fn lower_enum_class(
    def: &pycc_ast::StmtClassDef,
    class_name: String,
    bases: Vec<String>,
    mro: Vec<String>,
    type_param: Option<String>,
    is_str_enum: bool,
) -> Result<(HirClassDef, Vec<HirItem>), Diagnostic> {
    let mut enum_members: Vec<(String, EnumMemberValue)> = Vec::new();
    // #892: the class's single member-value type, as the word that names it
    // in a diagnostic. `StrEnum` fixes it to `str` up front -- CPython 3.14
    // raises `TypeError: 1 is not a string` for `class K(StrEnum): A = 1`,
    // so the base decides the type and a disagreeing member is the error,
    // not the other way round. A plain `Enum` leaves it `None` until the
    // first member fixes it, and every later member must agree.
    let mut value_kind: Option<&'static str> = if is_str_enum { Some("str") } else { None };
    // #892: the value `auto()` would produce for the next `int`-valued
    // member. CPython's `enum.auto` continues from the last explicit value,
    // starting at 1, so this advances past every `int` member -- explicit or
    // auto-derived -- as the body is walked.
    // `None` once a member's value has reached `i64::MAX`, so a following
    // `auto()` reports an overflow instead of silently repeating that value.
    let mut next_auto: Option<i64> = Some(1);
    for stmt in &def.body {
        // #744: a docstring (a bare string-literal expression statement) is
        // a no-op, matching `validate_init_subclass_body`'s existing
        // precedent for the same construct.
        if let Stmt::Expr(expr_stmt) = stmt
            && matches!(*expr_stmt.value, Expr::StringLiteral(_))
        {
            continue;
        }
        let Stmt::Assign(assign) = stmt else {
            return Err(unsupported(
                "an enum class body must contain only member assignments (`RED = 1`) -- \
                 no method definitions or other statements are supported yet",
                pycc_ast::stmt_range(stmt),
            ));
        };
        // The target must be a single bare name (not a tuple or
        // subscript).
        if assign.targets.len() != 1 {
            return Err(unsupported(
                "an enum member assignment must have a single target (`RED = 1`), not \
                 multiple targets",
                assign.range,
            ));
        }
        let Expr::Name(target_name) = &assign.targets[0] else {
            return Err(unsupported(
                "an enum member name must be a bare name (`RED = 1`), not an attribute \
                 access, subscript, or other expression",
                pycc_ast::expr_range(&assign.targets[0]),
            ));
        };
        let member_name = target_name.id.to_string();
        // #975: an enum body is a third route to a class-level binding, so it
        // needs the same reserved-name guard as the two `class/attrs.rs`
        // routes -- `class C(Enum): __init__ = 1` compiles here today while
        // CPython raises `TypeError` at class creation.
        // #979: one of that guard's `Enum`-route checks is class-name-keyed,
        // because CPython's `_EnumDict._is_private` matches the raw key
        // against the literal `_<ClassName>__` prefix.
        reject_reserved_class_attr_name(
            &member_name,
            ClassBodyRoute::Enum {
                class_name: &class_name,
            },
            assign.range.into(),
        )?;
        // Reject duplicate member names (matching CPython's
        // `TypeError: Attempted to reuse key`).
        if enum_members.iter().any(|(name, _)| name == &member_name) {
            return Err(unsupported(
                format!(
                    "enum member `{member_name}` is already defined in class \
                     `{class_name}` -- duplicate member names are not allowed"
                ),
                assign.range,
            ));
        }
        // The value must be an `int` or `str` literal (#892 widened this
        // from `int` alone). A bool literal is rejected (it is a separate
        // type in pycc, not an int subtype for enum-value purposes), as is
        // a non-literal value (e.g. `RED = f()`) -- enum values must be
        // compile-time literals in pycc's static model. The actual value is
        // extracted and carried in `enum_members` so codegen can initialize
        // each member's `value` slot with the correct literal, not a
        // position-derived guess.
        let member_value = match &*assign.value {
            Expr::NumberLiteral(number) => match &number.value {
                Number::Int(i) => {
                    let Some(value) = i.as_i64() else {
                        return Err(unsupported(
                            format!(
                                "enum member `{member_name}` has an integer value that does \
                                 not fit in i64 -- only i64-range values are supported"
                            ),
                            assign.range,
                        ));
                    };
                    EnumMemberValue::Int(value)
                }
                _ => {
                    return Err(unsupported(
                        format!(
                            "enum member `{member_name}` has a non-integer numeric value -- \
                             only `int` and `str` member values are supported"
                        ),
                        assign.range,
                    ));
                }
            },
            Expr::StringLiteral(string) => EnumMemberValue::Str(string.value.to_str().to_string()),
            // #892: `RED = auto()`. The bare name `auto` is matched
            // textually here, exactly as the marker base name `Enum` itself
            // is (this crate's established textual-resolution precedent):
            // `from enum import auto` registers the name, but the call has
            // no value of its own and is never lowered as an expression.
            // Only the bare-name, zero-argument spelling is recognized --
            // `enum.auto()` and `auto(1)` fall through to the catch-all.
            Expr::Call(call)
                if matches!(&*call.func, Expr::Name(name) if name.id.as_str() == "auto")
                    && call.arguments.args.is_empty()
                    && call.arguments.keywords.is_empty() =>
            {
                // A `StrEnum` member's auto value is its own lower-cased
                // name, matching CPython's `StrEnum._generate_next_value_`.
                // In a plain `Enum` it is the next integer -- which makes
                // `auto()` in an otherwise `str`-valued plain `Enum` a
                // mixed-value-type error, handled by the check below rather
                // than by a case of its own.
                if is_str_enum {
                    EnumMemberValue::Str(member_name.to_lowercase())
                } else {
                    let Some(value) = next_auto else {
                        return Err(unsupported(
                            format!(
                                "enum member `{member_name}` uses `auto()`, but the preceding \
                                 member's value is already `i64::MAX` -- only i64-range values \
                                 are supported"
                            ),
                            assign.range,
                        ));
                    };
                    EnumMemberValue::Int(value)
                }
            }
            _ => {
                return Err(unsupported(
                    format!(
                        "enum member `{member_name}` must be assigned an integer or string \
                         literal (`{member_name} = 1`, `{member_name} = \"a\"`), not an \
                         expression or non-literal value"
                    ),
                    assign.range,
                ));
            }
        };
        // #892: every member of one enum class must carry the same value
        // type. Whichever type came first -- from the `StrEnum` base, or
        // from the first member of a plain `Enum` -- binds the rest.
        let member_kind = match member_value {
            EnumMemberValue::Int(value) => {
                next_auto = value.checked_add(1);
                "int"
            }
            EnumMemberValue::Str(_) => "str",
        };
        match value_kind {
            Some(kind) if kind != member_kind && is_str_enum => {
                return Err(unsupported(
                    format!(
                        "enum class `{class_name}` derives from `StrEnum`, so member \
                         `{member_name}` must be assigned a string literal"
                    ),
                    assign.range,
                ));
            }
            Some(kind) if kind != member_kind => {
                return Err(unsupported(
                    format!(
                        "enum member `{member_name}` is `{member_kind}`-valued but enum \
                         class `{class_name}` has `{kind}`-valued members -- every member \
                         of an enum must have the same value type"
                    ),
                    assign.range,
                ));
            }
            _ => value_kind = Some(member_kind),
        }
        enum_members.push((member_name, member_value));
    }
    // An enum class's attrs are the two reserved member attributes
    // (`value`, `name`), so `member.value`/`member.name` resolve via the
    // existing `resolve_attr_get`/MIR slot resolution unchanged. #892 makes
    // `value`'s type per-class, so this is built after the member loop has
    // settled `value_kind`. A body with no members at all (`class E(Enum):
    // "doc"`, accepted since #744) keeps the historical `Ty::Int`.
    let attrs = vec![
        (
            "value".to_string(),
            if value_kind == Some("str") {
                Ty::Str
            } else {
                Ty::Int
            },
        ),
        ("name".to_string(), Ty::Str),
    ];
    // An enum class has no methods, no __init__, and no items. Its
    // members are compile-time singletons allocated by codegen, not
    // runtime-instantiated objects.
    Ok((
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: class_name,
            bases,
            mro,
            attrs,
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param,
            is_enum: true,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members,
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests {
    use crate::Ty;
    use crate::class::EnumMemberValue;
    use crate::class::tests::{assert_c0001, lower_ok};

    // -- #379 (PR-19): PEP 435 enum class lowering ------------------------

    #[test]
    fn generic_enum_class_is_rejected() {
        // `class C[T](Enum):` — a generic class whose single base is `Enum`.
        // The type parameter `T` triggers the generic-enum rejection in
        // `lower_class` (before it delegates here), distinct from the
        // multiple-bases rejection.
        assert_c0001("class C[T](Enum):\n    RED = 1\n");
    }

    #[test]
    fn enum_member_with_multiple_targets_is_rejected() {
        // `RED = GREEN = 1` — a chain assignment with multiple targets,
        // which has `assign.targets.len() == 2`, triggering the
        // multiple-targets rejection. (Tuple unpacking `RED, GREEN = 1, 2`
        // has a single tuple target and hits a different path.)
        assert_c0001("class C(Enum):\n    RED = GREEN = 1\n");
    }

    #[test]
    fn enum_member_with_non_name_target_is_rejected() {
        assert_c0001("class C(Enum):\n    C.RED = 1\n");
    }

    #[test]
    fn enum_member_value_overflowing_i64_is_rejected() {
        assert_c0001("class C(Enum):\n    RED = 99999999999999999999999999\n");
    }

    #[test]
    fn auto_after_an_i64_max_member_is_rejected() {
        // #892: `auto()` derives the next integer from the previous member.
        // After `i64::MAX` there is no next `i64`, and silently repeating
        // `i64::MAX` would give two members the same value, so this is an
        // error -- consistent with the explicit out-of-range literal above.
        assert_c0001("class C(Enum):\n    A = 9223372036854775807\n    B = auto()\n");
    }

    #[test]
    fn enum_member_with_non_literal_value_is_rejected() {
        assert_c0001("x = 1\nclass C(Enum):\n    RED = x\n");
    }

    // -- #379: enum error paths covered via unit tests (not integration
    //    tests) to avoid cargo-llvm-cov issue #276 (instantiation merging) --

    #[test]
    fn enum_body_with_method_is_rejected_via_unit_test() {
        // Exercises the "enum class body must contain only member
        // assignments" error path.
        assert_c0001(
            "class Color(Enum):\n    RED = 1\n    def f(self) -> int:\n        return 1\n",
        );
    }

    #[test]
    fn duplicate_enum_member_is_rejected_via_unit_test() {
        // Exercises the "duplicate enum member" error path.
        assert_c0001("class Color(Enum):\n    RED = 1\n    RED = 2\n");
    }

    #[test]
    fn enum_member_float_value_is_rejected_via_unit_test() {
        // Exercises the "non-integer numeric value" error path, which fires
        // for `Number::Float` inside an `Expr::NumberLiteral`.
        assert_c0001("class Color(Enum):\n    RED = 1.5\n");
    }

    #[test]
    fn enum_member_bool_value_is_rejected_via_unit_test() {
        // `True` parses as `Expr::BooleanLiteral`, not `Expr::NumberLiteral`,
        // so this exercises the catch-all "must be assigned an integer or
        // string literal" arm rather than the `float` arm above.
        assert_c0001("class Color(Enum):\n    RED = True\n");
    }

    #[test]
    fn enum_class_with_docstring_is_accepted() {
        // #744: a class docstring (a bare string-literal expression
        // statement) is a no-op in an enum body, not a member assignment.
        let hir = lower_ok("class Color(Enum):\n    \"A color.\"\n    RED = 1\n    GREEN = 2\n");
        let (_, class_def) = &hir.class_defs[0];
        assert!(
            class_def.is_enum,
            "#921: `lower_enum_class` marks its class"
        );
        assert_eq!(class_def.enum_members.len(), 2);
        assert_eq!(class_def.enum_members[0].0, "RED");
    }

    #[test]
    fn an_enum_class_with_a_non_leading_docstring_is_accepted() {
        // #744's guard has no position check: place the docstring after a
        // member assignment to exercise the non-leading case directly.
        let hir = lower_ok("class Color(Enum):\n    RED = 1\n    \"A color.\"\n    GREEN = 2\n");
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.enum_members.len(), 2);
        assert_eq!(class_def.enum_members[0].0, "RED");
        assert_eq!(class_def.enum_members[1].0, "GREEN");
    }

    #[test]
    fn a_non_string_expression_statement_in_an_enum_body_is_still_rejected() {
        // #744's docstring exemption covers only a bare string-literal
        // expression statement: a bare non-string expression statement in
        // an enum body remains C0001, exercising the guard's false branch
        // distinctly from a non-`Stmt::Expr` statement (which already
        // short-circuits before the guard).
        assert_c0001("class Color(Enum):\n    42\n    RED = 1\n");
    }

    #[test]
    fn valid_enum_class_lowers_via_unit_test() {
        // Covers `lower_enum_class`'s `Ok` return path inside this
        // crate's own unit-test binary, working around
        // cargo-llvm-cov issue #276 (instantiation-merge gap between
        // the library and integration-test binaries).
        let hir = lower_ok("class Color(Enum):\n    RED = 1\n    GREEN = 2\n    BLUE = 3\n");
        assert_eq!(hir.class_defs.len(), 1);
        let (_, class_def) = &hir.class_defs[0];
        assert!(!class_def.is_protocol);
        assert_eq!(class_def.enum_members.len(), 3);
        assert_eq!(class_def.enum_members[0].0, "RED");
        assert_eq!(class_def.enum_members[0].1, EnumMemberValue::Int(1));
    }

    #[test]
    fn single_member_enum_class_lowers_via_unit_test() {
        // Additional `lower_enum_class` `Ok` return coverage with a
        // minimal single-member enum.
        let hir = lower_ok("class Color(Enum):\n    RED = 0\n");
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.enum_members.len(), 1);
        assert_eq!(class_def.enum_members[0].0, "RED");
        assert_eq!(class_def.enum_members[0].1, EnumMemberValue::Int(0));
    }

    // -- #892: string-valued members, `StrEnum`, and `auto()` -------------

    #[test]
    fn a_string_valued_enum_class_lowers() {
        let hir = lower_ok("class Kind(Enum):\n    AXIAL = \"axial\"\n    RADIAL = \"radial\"\n");
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(
            class_def.enum_members,
            vec![
                (
                    "AXIAL".to_string(),
                    EnumMemberValue::Str("axial".to_string())
                ),
                (
                    "RADIAL".to_string(),
                    EnumMemberValue::Str("radial".to_string())
                ),
            ]
        );
        // The class's `value` attribute follows its member-value type.
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Str));
        assert_eq!(class_def.attrs[1], ("name".to_string(), Ty::Str));
    }

    #[test]
    fn an_int_valued_enum_class_keeps_an_int_value_attr() {
        let hir = lower_ok("class Color(Enum):\n    RED = 1\n");
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Int));
    }

    #[test]
    fn a_str_enum_subclass_lowers_with_a_str_value_attr() {
        let hir = lower_ok("class Kind(StrEnum):\n    AXIAL = \"axial\"\n");
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.name, "Kind");
        // The `StrEnum` marker base is consumed exactly like `Enum`.
        assert!(class_def.bases.is_empty());
        assert_eq!(class_def.mro, vec!["Kind".to_string()]);
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Str));
        assert_eq!(
            class_def.enum_members[0].1,
            EnumMemberValue::Str("axial".to_string())
        );
    }

    #[test]
    fn a_member_less_enum_body_keeps_the_historical_int_value_attr() {
        // #744 accepts a docstring-only enum body, so an enum class with no
        // members at all is reachable and has no first member to infer a
        // value type from. It keeps `Ty::Int`, the type it had before #892.
        let hir = lower_ok("class E(Enum):\n    \"Just a docstring.\"\n");
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.enum_members.is_empty());
        // #921: the empty member table is not the enum marker; the
        // provenance flag is, and it is set for a member-less enum too.
        assert!(class_def.is_enum);
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Int));
    }

    #[test]
    fn a_member_less_str_enum_body_still_has_a_str_value_attr() {
        // The `StrEnum` base fixes the value type without reading a member,
        // so the member-less body is `str`-valued, not `int`-valued.
        let hir = lower_ok("class E(StrEnum):\n    \"Just a docstring.\"\n");
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.enum_members.is_empty());
        assert!(class_def.is_enum, "#921: `StrEnum` shares the enum marker");
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Str));
    }

    #[test]
    fn a_str_member_in_an_int_valued_enum_is_rejected() {
        assert_c0001("class E(Enum):\n    A = 1\n    B = \"b\"\n");
    }

    #[test]
    fn an_int_member_in_a_str_valued_enum_is_rejected() {
        // The mirrored ordering of the test above: the mixed-type check has
        // to fire whichever type came first.
        assert_c0001("class E(Enum):\n    A = \"a\"\n    B = 2\n");
    }

    #[test]
    fn a_non_string_member_in_a_str_enum_is_rejected() {
        // Distinct from the plain-`Enum` mixed-type path: the `StrEnum` base
        // fixes the type before any member is read, so the *first* member
        // can already be wrong. Matches CPython 3.14's own
        // `TypeError: 1 is not a string`.
        assert_c0001("class K(StrEnum):\n    A = 1\n");
    }

    #[test]
    fn auto_derives_the_next_int_in_a_plain_enum() {
        // Verified against CPython 3.14.7: `auto()` continues from the last
        // explicit value, starting at 1.
        let hir = lower_ok(
            "class N(Enum):\n    A = auto()\n    B = auto()\n    C = 10\n    D = auto()\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(
            class_def.enum_members,
            vec![
                ("A".to_string(), EnumMemberValue::Int(1)),
                ("B".to_string(), EnumMemberValue::Int(2)),
                ("C".to_string(), EnumMemberValue::Int(10)),
                ("D".to_string(), EnumMemberValue::Int(11)),
            ]
        );
        assert_eq!(class_def.attrs[0], ("value".to_string(), Ty::Int));
    }

    #[test]
    fn auto_derives_the_lowercased_name_in_a_str_enum() {
        let hir = lower_ok(
            "class S(StrEnum):\n    RED = auto()\n    GREEN = \"green\"\n    BLUE = auto()\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(
            class_def.enum_members,
            vec![
                ("RED".to_string(), EnumMemberValue::Str("red".to_string())),
                (
                    "GREEN".to_string(),
                    EnumMemberValue::Str("green".to_string())
                ),
                ("BLUE".to_string(), EnumMemberValue::Str("blue".to_string())),
            ]
        );
    }

    #[test]
    fn auto_in_a_string_valued_plain_enum_is_a_mixed_type_error() {
        // `auto()` in a plain `Enum` yields an integer, so using it after a
        // string member is the ordinary mixed-value-type rejection.
        assert_c0001("class E(Enum):\n    A = \"a\"\n    B = auto()\n");
    }

    #[test]
    fn auto_with_an_argument_is_rejected() {
        // Only the bare-name, zero-argument spelling is recognized.
        assert_c0001("class E(Enum):\n    A = auto(1)\n");
    }

    #[test]
    fn auto_with_a_keyword_argument_is_rejected() {
        assert_c0001("class E(Enum):\n    A = auto(x=1)\n");
    }

    #[test]
    fn a_non_auto_call_member_value_is_rejected() {
        // A zero-argument call to some *other* bare name falls through to
        // the catch-all, exercising the `auto` name guard's false branch.
        assert_c0001("class E(Enum):\n    A = f()\n");
    }

    #[test]
    fn an_attribute_call_member_value_is_rejected() {
        // `enum.auto()` is an attribute call, not a bare name, so it too
        // falls through to the catch-all -- exercising the guard's
        // non-`Expr::Name` callee branch.
        assert_c0001("class E(Enum):\n    A = m.auto()\n");
    }

    #[test]
    fn enum_member_value_is_debuggable_and_comparable() {
        // `EnumMemberValue`'s derived `Debug`/`PartialEq`/`Clone` are part of
        // its public shape; exercise both variants of each so the derived
        // code is covered by execution rather than by a failing assertion.
        let int_value = EnumMemberValue::Int(7);
        let str_value = EnumMemberValue::Str("seven".to_string());
        assert_eq!(int_value.clone(), EnumMemberValue::Int(7));
        assert_eq!(str_value.clone(), EnumMemberValue::Str("seven".to_string()));
        assert_ne!(int_value, EnumMemberValue::Int(8));
        assert_ne!(str_value, EnumMemberValue::Str("eight".to_string()));
        assert_ne!(int_value, str_value);
        assert!(format!("{int_value:?}").contains("Int"));
        assert!(format!("{str_value:?}").contains("Str"));
    }
}
