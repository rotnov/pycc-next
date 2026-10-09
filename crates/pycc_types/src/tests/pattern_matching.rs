//! `match` statement unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! tests that exercise the #381 pattern-matching type-checking coverage, so
//! the split keeps one cohesive feature in one place. As a child module this
//! still sees the parent's private items directly through `use super::*`, so
//! nothing needed widened visibility; only the tests' location changed.

use super::*;

// -- #381: match statement type checking coverage ----------------------

#[test]
fn match_with_int_literal_pattern_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_string_literal_pattern_type_checks() {
    let result = check_source(
        "def f(x: str) -> None:\n    match x:\n        case \"hi\":\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_float_literal_pattern_type_checks() {
    let result = check_source(
        "def f(x: float) -> None:\n    match x:\n        case 3.14:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_bool_singleton_pattern_type_checks() {
    let result = check_source(
        "def f(x: bool) -> None:\n    match x:\n        case True:\n            pass\n        case False:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_none_singleton_pattern_type_checks() {
    let result = check_source(
        "def f() -> None:\n    pass\ndef g(x: int) -> None:\n    match f():\n        case None:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_capture_pattern_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case y:\n            print(y)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_wildcard_pattern_type_checks() {
    let result =
        check_source("def f(x: int) -> None:\n    match x:\n        case _:\n            pass\n");
    assert!(result.is_ok());
}

#[test]
fn match_with_sequence_pattern_type_checks() {
    let result = check_source(
        "x = [1, 2]\nmatch x:\n    case [a, b]:\n        print(a)\n    case _:\n        pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_sequence_star_pattern_type_checks() {
    let result = check_source(
        "x = [1, 2]\nmatch x:\n    case [a, *rest]:\n        print(a)\n    case _:\n        pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_mapping_pattern_type_checks() {
    let result = check_source(
        "x = {\"k\": 1}\nmatch x:\n    case {\"k\": v}:\n            print(v)\n    case _:\n        pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_mapping_rest_pattern_type_checks() {
    let result = check_source(
        "x = {\"k\": 1}\nmatch x:\n    case {\"k\": v, **rest}:\n            print(v)\n    case _:\n        pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_class_pattern_type_checks() {
    let result = check_source(
        "class P:\n    def __init__(self, a: int) -> None:\n        self.a = a\ndef f(x: P) -> None:\n    match x:\n        case P(a):\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_class_keyword_pattern_type_checks() {
    let result = check_source(
        "class P:\n    def __init__(self) -> None:\n        self.a = 1\ndef f(x: P) -> None:\n    match x:\n        case P(a=1):\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_or_pattern_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 | 2 | 3:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_as_pattern_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 as y:\n            print(y)\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_guard_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case y if y > 3:\n            print(y)\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_non_exhaustive_reports_t0030() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 0:\n            pass\n        case 1:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0030");
}

#[test]
fn match_bool_exhaustive_type_checks() {
    let result = check_source(
        "def f(x: bool) -> None:\n    match x:\n        case True:\n            pass\n        case False:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_bool_non_exhaustive_reports_t0030() {
    let result = check_source(
        "def f(x: bool) -> None:\n    match x:\n        case True:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0030");
}

#[test]
fn match_guard_not_bool_reports_t0021() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case y if y:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_singleton_wrong_subject_reports_t0021() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case True:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_none_wrong_subject_reports_t0021() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case None:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_sequence_wrong_subject_reports_t0021() {
    let result = check_source(
        "x = 1\nmatch x:\n    case [a, b]:\n        pass\n    case _:\n        pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_mapping_wrong_subject_reports_t0021() {
    let result = check_source(
        "x = 1\nmatch x:\n    case {\"k\": v}:\n        pass\n    case _:\n        pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_mapping_wrong_key_type_reports_t0021() {
    let result = check_source(
        "x = {\"k\": 1}\nmatch x:\n    case {1: v}:\n        pass\n    case _:\n        pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_class_undefined_reports_t0021() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case Undefined():\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_class_wrong_subject_reports_t0021() {
    let result = check_source(
        "class P:\n    def __init__(self) -> None:\n        pass\ndef f(x: int) -> None:\n    match x:\n        case P():\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_literal_wrong_subject_reports_t0021() {
    let result = check_source(
        "def f(x: str) -> None:\n    match x:\n        case 1:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_or_pattern_different_bindings_reports_t0021() {
    let env = Environment::new();
    let pattern = HirPattern::Or(vec![
        HirPattern::As(
            Box::new(HirPattern::Literal(HirExpr::IntLiteral(1))),
            "a".to_string(),
        ),
        HirPattern::As(
            Box::new(HirPattern::Literal(HirExpr::IntLiteral(2))),
            "b".to_string(),
        ),
    ]);
    let err = check_pattern(&env, &[], &pattern, &Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("or-pattern alternatives must bind the same set of names")
    );
}

#[test]
fn match_with_return_in_case_type_checks() {
    let result = check_source(
        "def f(x: int) -> int:\n    match x:\n        case 1:\n            return 10\n        case _:\n            return 0\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_binding_in_case_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    result = 0\n    match x:\n        case 1:\n            result = 10\n        case _:\n            result = 20\n    print(result)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_tuple_subject_type_checks() {
    let result = check_source(
        "x = (1, 2)\nmatch x:\n    case [a, b]:\n        print(a)\n    case _:\n        pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_enum_exhaustive_type_checks() {
    let mut env = Environment::new();
    env.bind_class(
        "Color".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Color".to_string(),
            bases: Vec::new(),
            mro: vec!["Color".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: vec![
                ("RED".to_string(), pycc_hir::EnumMemberValue::Int(1)),
                ("GREEN".to_string(), pycc_hir::EnumMemberValue::Int(2)),
            ],
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    let cases = vec![
        HirMatchCase {
            pattern: HirPattern::Class {
                class_name: "Color".to_string(),
                positional: vec![],
                keyword: vec![],
            },
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Class {
                class_name: "Color".to_string(),
                positional: vec![],
                keyword: vec![],
            },
            guard: None,
            body: vec![],
        },
    ];
    assert!(check_exhaustive(
        &env,
        &Ty::Instance(Box::new("Color".to_string())),
        &cases
    ));
}

#[test]
fn match_enum_non_exhaustive_reports_t0030() {
    let mut env = Environment::new();
    env.bind_class(
        "Color".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Color".to_string(),
            bases: Vec::new(),
            mro: vec!["Color".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: vec![
                ("RED".to_string(), pycc_hir::EnumMemberValue::Int(1)),
                ("GREEN".to_string(), pycc_hir::EnumMemberValue::Int(2)),
            ],
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Class {
            class_name: "Color".to_string(),
            positional: vec![],
            keyword: vec![],
        },
        guard: Some(HirExpr::BoolLiteral(true)),
        body: vec![],
    }];
    assert!(!check_exhaustive(
        &env,
        &Ty::Instance(Box::new("Color".to_string())),
        &cases
    ));
}

#[test]
fn match_enum_exhaustive_with_other_class_pattern() {
    // A case pattern with a different class name than the enum subject
    // exercises the guard-false branch of `collect_enum_member_patterns`'s
    // `HirPattern::Class { class_name: cn, .. } if cn == class_name` arm.
    let mut env = Environment::new();
    env.bind_class(
        "Color".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Color".to_string(),
            bases: Vec::new(),
            mro: vec!["Color".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: vec![
                ("RED".to_string(), pycc_hir::EnumMemberValue::Int(1)),
                ("GREEN".to_string(), pycc_hir::EnumMemberValue::Int(2)),
            ],
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    env.bind_class(
        "Other".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Other".to_string(),
            bases: Vec::new(),
            mro: vec!["Other".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    let cases = vec![
        HirMatchCase {
            pattern: HirPattern::Class {
                class_name: "Color".to_string(),
                positional: vec![],
                keyword: vec![],
            },
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Class {
                class_name: "Color".to_string(),
                positional: vec![],
                keyword: vec![],
            },
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Class {
                class_name: "Other".to_string(),
                positional: vec![],
                keyword: vec![],
            },
            guard: None,
            body: vec![],
        },
    ];
    assert!(check_exhaustive(
        &env,
        &Ty::Instance(Box::new("Color".to_string())),
        &cases
    ));
}

#[test]
fn match_with_inferred_subject_type_checks() {
    let result =
        check_source("x = 1\nmatch x:\n    case 1:\n        pass\n    case _:\n        pass\n");
    assert!(result.is_ok());
}

#[test]
fn match_with_or_pattern_same_bindings_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 | 2 | 3:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_or_pattern_no_bindings_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 | 2 | 3:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_guarded_case_not_exhaustive_reports_t0030() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case y if y > 0:\n            pass\n        case 0:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0030");
}

#[test]
fn match_with_irrefutable_or_pattern_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 | _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_as_pattern_irrefutable_type_checks() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case _ as y:\n            print(y)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_generic_call_in_subject_type_checks() {
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def f(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        pass\n    def f(self) -> int:\n        return 1\ndef g(d: P) -> int:\n    match d.f():\n        case 1:\n            return 1\n        case _:\n            return 0\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_generic_class_instantiation_in_subject_type_checks() {
    let result = check_source(
        "class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\ndef f() -> None:\n    match Box(1):\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}
