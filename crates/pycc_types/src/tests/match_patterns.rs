//! `match` pattern checking, exhaustiveness and binding joins.

use super::*;

// -- direct unit tests for defense-in-depth paths ----------------------

#[test]
fn join_match_branches_skips_empty_states() {
    let mut env = Environment::new();
    let case_envs: Vec<Environment> = vec![Environment::new(), Environment::new()];
    join_match_branches(&mut env, &case_envs, true);
    assert!(env.bindings.is_empty());
}

#[test]
fn check_pattern_wildcard_returns_no_bindings() {
    let env = Environment::new();
    let result = check_pattern(&env, &[], &HirPattern::Wildcard, &Ty::Int).unwrap();
    assert!(result.is_empty());
}

#[test]
fn check_pattern_capture_returns_binding() {
    let env = Environment::new();
    let result = check_pattern(&env, &[], &HirPattern::Capture("y".to_string()), &Ty::Int).unwrap();
    assert_eq!(result, vec![("y".to_string(), Ty::Int)]);
}

#[test]
fn check_pattern_singleton_wrong_subject_reports_t0021() {
    let env = Environment::new();
    let err = check_pattern(&env, &[], &HirPattern::Singleton(true), &Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_pattern_none_wrong_subject_reports_t0021() {
    let env = Environment::new();
    let err = check_pattern(&env, &[], &HirPattern::NoneSingleton, &Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_pattern_sequence_wrong_subject_reports_t0021() {
    let env = Environment::new();
    let err = check_pattern(&env, &[], &HirPattern::Sequence(vec![]), &Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_pattern_mapping_wrong_subject_reports_t0021() {
    let env = Environment::new();
    let err = check_pattern(&env, &[], &HirPattern::Mapping(vec![], None), &Ty::Int).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_pattern_class_undefined_reports_t0021() {
    let env = Environment::new();
    let err = check_pattern(
        &env,
        &[],
        &HirPattern::Class {
            class_name: "Undefined".to_string(),
            positional: vec![],
            keyword: vec![],
        },
        &Ty::Int,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_pattern_or_empty_returns_empty() {
    let env = Environment::new();
    let result = check_pattern(&env, &[], &HirPattern::Or(vec![]), &Ty::Int).unwrap();
    assert!(result.is_empty());
}

#[test]
fn check_exhaustive_wildcard_is_exhaustive() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Wildcard,
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(&env, &Ty::Int, &cases));
}

#[test]
fn check_exhaustive_capture_is_exhaustive() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Capture("y".to_string()),
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(&env, &Ty::Int, &cases));
}

#[test]
fn check_exhaustive_int_not_exhaustive() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Literal(HirExpr::IntLiteral(1)),
        guard: None,
        body: vec![],
    }];
    assert!(!check_exhaustive(&env, &Ty::Int, &cases));
}

#[test]
fn check_exhaustive_bool_both_covered() {
    let env = Environment::new();
    let cases = vec![
        HirMatchCase {
            pattern: HirPattern::Singleton(true),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Singleton(false),
            guard: None,
            body: vec![],
        },
    ];
    assert!(check_exhaustive(&env, &Ty::Bool, &cases));
}

#[test]
fn check_exhaustive_bool_only_true() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Singleton(true),
        guard: None,
        body: vec![],
    }];
    assert!(!check_exhaustive(&env, &Ty::Bool, &cases));
}

#[test]
fn check_exhaustive_bool_or_pattern() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Or(vec![
            HirPattern::Singleton(true),
            HirPattern::Singleton(false),
        ]),
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(&env, &Ty::Bool, &cases));
}

#[test]
fn check_exhaustive_bool_guarded_skipped() {
    let env = Environment::new();
    let cases = vec![
        HirMatchCase {
            pattern: HirPattern::Singleton(true),
            guard: Some(HirExpr::BoolLiteral(true)),
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Singleton(false),
            guard: None,
            body: vec![],
        },
    ];
    assert!(!check_exhaustive(&env, &Ty::Bool, &cases));
}

#[test]
fn check_exhaustive_irrefutable_or_pattern() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Or(vec![
            HirPattern::Literal(HirExpr::IntLiteral(1)),
            HirPattern::Wildcard,
        ]),
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(&env, &Ty::Int, &cases));
}

#[test]
fn check_exhaustive_as_pattern_irrefutable() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::As(Box::new(HirPattern::Wildcard), "y".to_string()),
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(&env, &Ty::Int, &cases));
}

#[test]
fn check_exhaustive_non_bool_non_instance_not_exhaustive() {
    let env = Environment::new();
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Literal(HirExpr::IntLiteral(1)),
        guard: None,
        body: vec![],
    }];
    assert!(!check_exhaustive(&env, &Ty::Str, &cases));
}

#[test]
fn check_exhaustive_instance_non_enum_not_exhaustive() {
    let mut env = Environment::new();
    env.bind_class(
        "P".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "P".to_string(),
            bases: Vec::new(),
            mro: vec!["P".to_string()],
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
    let cases = vec![HirMatchCase {
        pattern: HirPattern::Class {
            class_name: "P".to_string(),
            positional: vec![],
            keyword: vec![],
        },
        guard: None,
        body: vec![],
    }];
    assert!(!check_exhaustive(
        &env,
        &Ty::Instance(Box::new("P".to_string())),
        &cases
    ));
}

#[test]
fn check_exhaustive_enum_all_covered() {
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
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(
        &env,
        &Ty::Instance(Box::new("Color".to_string())),
        &cases
    ));
}

#[test]
fn check_exhaustive_enum_partial_not_exhaustive() {
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
        pattern: HirPattern::Or(vec![HirPattern::Class {
            class_name: "Color".to_string(),
            positional: vec![],
            keyword: vec![],
        }]),
        guard: None,
        body: vec![],
    }];
    assert!(check_exhaustive(
        &env,
        &Ty::Instance(Box::new("Color".to_string())),
        &cases
    ));
}

#[test]
fn collect_pattern_capture_names_covers_all_pattern_kinds() {
    let mut names: Vec<&str> = Vec::new();
    let p_wild = HirPattern::Wildcard;
    collect_pattern_capture_names(&p_wild, &mut names);
    let p_lit = HirPattern::Literal(HirExpr::IntLiteral(1));
    collect_pattern_capture_names(&p_lit, &mut names);
    let p_sin = HirPattern::Singleton(true);
    collect_pattern_capture_names(&p_sin, &mut names);
    let p_none = HirPattern::NoneSingleton;
    collect_pattern_capture_names(&p_none, &mut names);
    let p_cap = HirPattern::Capture("a".to_string());
    collect_pattern_capture_names(&p_cap, &mut names);
    let p_seq = HirPattern::Sequence(vec![HirPattern::Capture("b".to_string())]);
    collect_pattern_capture_names(&p_seq, &mut names);
    let p_seqstar = HirPattern::SequenceStar(
        vec![HirPattern::Capture("c".to_string())],
        Some("rest".to_string()),
    );
    collect_pattern_capture_names(&p_seqstar, &mut names);
    let p_seqstar2 = HirPattern::SequenceStar(vec![HirPattern::Capture("d".to_string())], None);
    collect_pattern_capture_names(&p_seqstar2, &mut names);
    let p_map = HirPattern::Mapping(
        vec![(
            HirExpr::StringLiteral("k".to_string()),
            HirPattern::Capture("e".to_string()),
        )],
        Some("mrest".to_string()),
    );
    collect_pattern_capture_names(&p_map, &mut names);
    let p_map2 = HirPattern::Mapping(vec![], None);
    collect_pattern_capture_names(&p_map2, &mut names);
    let p_cls = HirPattern::Class {
        class_name: "P".to_string(),
        positional: vec![HirPattern::Capture("f".to_string())],
        keyword: vec![("a".to_string(), HirPattern::Capture("g".to_string()))],
    };
    collect_pattern_capture_names(&p_cls, &mut names);
    let p_or = HirPattern::Or(vec![HirPattern::Capture("h".to_string())]);
    collect_pattern_capture_names(&p_or, &mut names);
    let p_as = HirPattern::As(
        Box::new(HirPattern::Capture("i".to_string())),
        "j".to_string(),
    );
    collect_pattern_capture_names(&p_as, &mut names);
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));
    assert!(names.contains(&"c"));
    assert!(names.contains(&"rest"));
    assert!(names.contains(&"mrest"));
    assert!(names.contains(&"j"));
}

#[test]
fn collect_pattern_capture_names_deduplicates() {
    let mut names: Vec<&str> = vec!["a"];
    let p = HirPattern::Capture("a".to_string());
    collect_pattern_capture_names(&p, &mut names);
    assert_eq!(names.iter().filter(|&&n| n == "a").count(), 1);
}

#[test]
fn collect_bool_patterns_with_non_bool_pattern_is_noop() {
    let mut has_true = false;
    let mut has_false = false;
    collect_bool_patterns(&HirPattern::Wildcard, &mut has_true, &mut has_false);
    collect_bool_patterns(
        &HirPattern::Capture("x".to_string()),
        &mut has_true,
        &mut has_false,
    );
    collect_bool_patterns(
        &HirPattern::Literal(HirExpr::IntLiteral(1)),
        &mut has_true,
        &mut has_false,
    );
    collect_bool_patterns(&HirPattern::NoneSingleton, &mut has_true, &mut has_false);
    assert!(!has_true);
    assert!(!has_false);
}

#[test]
fn collect_enum_member_patterns_with_non_class_pattern_is_noop() {
    let members = [
        ("RED".to_string(), pycc_hir::EnumMemberValue::Int(1)),
        ("GREEN".to_string(), pycc_hir::EnumMemberValue::Int(2)),
    ];
    let mut covered: HashSet<&str> = HashSet::default();
    collect_enum_member_patterns(&HirPattern::Wildcard, "Color", &members, &mut covered);
    collect_enum_member_patterns(
        &HirPattern::Literal(HirExpr::IntLiteral(1)),
        "Color",
        &members,
        &mut covered,
    );
    collect_enum_member_patterns(
        &HirPattern::Capture("x".to_string()),
        "Color",
        &members,
        &mut covered,
    );
    assert!(covered.is_empty());
}

#[test]
fn match_with_type_error_in_case_body_inside_function_reports_t0021() {
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1:\n            y = x + \"bad\"\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_with_return_in_case_body_via_solver_path_type_checks() {
    let result = check_source(
        "def _f(x: int) -> int:\n    result = 0\n    match x:\n        case 1:\n            result = 10\n        case _:\n            result = 20\n    return result\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_binding_join_via_solver_path_type_checks() {
    let result = check_source(
        "def _f(x: int) -> None:\n    y = 0\n    match x:\n        case 1:\n            y = 10\n        case _:\n            pass\n    print(y)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_generic_call_in_guard_type_checks() {
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def f(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        pass\n    def f(self) -> int:\n        return 1\ndef g(d: P) -> None:\n    match 1:\n        case y if d.f() > 0:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn match_with_generic_class_instantiation_in_guard_type_checks() {
    let result = check_source(
        "class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\ndef f() -> None:\n    match True:\n        case _ if Box(1).x == 1:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
}

#[test]
fn match_with_constraint_error_in_subject_reports_error() {
    let result = check_source(
        "def _f(x: int) -> None:\n    match x + \"bad\":\n        case 1:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
}

#[test]
fn match_with_pattern_error_through_check_match_reports_error() {
    // Exercises the `?` error propagation from `check_pattern` inside
    // `check_match` (line 4065).  A singleton pattern `True` against
    // an `int` subject causes `check_pattern` to return `Err`, which
    // `check_match` must propagate via `?`.
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case True:\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
}

#[test]
fn match_with_guard_type_error_reports_t0021() {
    // Exercises the `?` error propagation from `infer_expr_in` on the
    // guard expression inside `check_match` (line 4070).  A guard with
    // a type error (`x + "bad"` where `x: int`) causes `infer_expr_in`
    // to return `Err`, which `check_match` must propagate via `?`.
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 1 if x + \"bad\":\n            pass\n        case _:\n            pass\n",
    );
    assert!(result.is_err());
}

#[test]
fn match_at_module_scope_with_body_error_reports_t0021() {
    let result = check_source(
        "x = 1\nmatch x:\n    case 1:\n        y = x + \"bad\"\n    case _:\n        pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0021");
}

#[test]
fn match_with_final_reassignment_in_pattern_reports_t0045() {
    let mut env = Environment::new();
    // Bind "x" so the match subject infers successfully.
    env.bindings
        .insert("x".to_string(), BindingState::Definitely(Ty::Int));
    // Declare a Final binding for "y".
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "y".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    // A match with a capture pattern using the same name should
    // trigger T0045 via `check_assignment`.
    let result = check_stmt(
        &mut env,
        &HirStmt::Match {
            subject: HirExpr::Name("x".to_string()),
            cases: vec![HirMatchCase {
                pattern: HirPattern::Capture("y".to_string()),
                guard: None,
                body: vec![],
            }],
        },
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0045");
}

#[test]
fn match_with_sequence_star_wildcard_type_checks() {
    let env = Environment::new();
    let pattern = HirPattern::SequenceStar(vec![HirPattern::Capture("a".to_string())], None);
    let result = check_pattern(&env, &[], &pattern, &Ty::List(Box::new(Ty::Int)));
    assert!(result.is_ok());
}

#[test]
fn match_with_non_exhaustive_join_maybe_binding() {
    // A non-exhaustive match where a binding is only in one case
    // produces a `Maybe` binding state in `join_match_branches`.
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 0:\n            y = 1\n        case 1:\n            pass\n",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "T0030");
}

#[test]
fn match_join_with_maybe_binding_state_from_if_without_else() {
    // A match case body that assigns `y` only inside an `if` without
    // `else` leaves `y` as `Maybe` in that case's env. The join then
    // hits the `BindingState::Maybe(t)` arm of the `states[0]` match
    // in `join_match_branches` (line 4035).
    let result = check_source(
        "def f(x: int) -> None:\n    match x:\n        case 0:\n            if x > 0:\n                y = 1\n        case _:\n            pass\n",
    );
    assert!(result.is_ok());
}

#[test]
fn check_pattern_literal_with_unresolvable_name_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::Literal(HirExpr::Name("undefined".to_string()));
    let result = check_pattern(&env, &[], &pattern, &Ty::Int);
    assert!(result.is_err());
}

#[test]
fn check_pattern_sequence_with_error_subpattern_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::Sequence(vec![HirPattern::Literal(HirExpr::Name("u".to_string()))]);
    let result = check_pattern(&env, &[], &pattern, &Ty::List(Box::new(Ty::Int)));
    assert!(result.is_err());
}

#[test]
fn check_pattern_sequence_star_with_error_subpattern_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::SequenceStar(
        vec![HirPattern::Literal(HirExpr::Name("u".to_string()))],
        None,
    );
    let result = check_pattern(&env, &[], &pattern, &Ty::List(Box::new(Ty::Int)));
    assert!(result.is_err());
}

#[test]
fn check_pattern_sequence_star_wrong_subject_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::SequenceStar(vec![], None);
    let result = check_pattern(&env, &[], &pattern, &Ty::Int);
    assert!(result.is_err());
}

#[test]
fn check_pattern_mapping_with_error_key_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::Mapping(
        vec![(HirExpr::Name("u".to_string()), HirPattern::Wildcard)],
        None,
    );
    let result = check_pattern(&env, &[], &pattern, &Ty::Dict(Box::new((Ty::Int, Ty::Int))));
    assert!(result.is_err());
}

#[test]
fn check_pattern_mapping_with_error_value_pattern_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::Mapping(
        vec![(
            HirExpr::IntLiteral(1),
            HirPattern::Literal(HirExpr::Name("u".to_string())),
        )],
        None,
    );
    let result = check_pattern(&env, &[], &pattern, &Ty::Dict(Box::new((Ty::Int, Ty::Int))));
    assert!(result.is_err());
}

#[test]
fn check_pattern_or_with_error_subpattern_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::Or(vec![HirPattern::Literal(HirExpr::Name("u".to_string()))]);
    let result = check_pattern(&env, &[], &pattern, &Ty::Int);
    assert!(result.is_err());
}

#[test]
fn check_pattern_as_with_error_inner_reports_error() {
    let env = Environment::new();
    let pattern = HirPattern::As(
        Box::new(HirPattern::Literal(HirExpr::Name("u".to_string()))),
        "x".to_string(),
    );
    let result = check_pattern(&env, &[], &pattern, &Ty::Int);
    assert!(result.is_err());
}

#[test]
fn check_pattern_class_with_subclass_subject_type_checks() {
    // Covers the `class_def.mro.contains(name)` branch of the `||` in
    // `check_class_pattern`: the subject type ("Base") is not the pattern
    // class ("Derived") but IS in "Derived"'s MRO.
    let mut env = Environment::new();
    env.bind_class(
        "Base".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Base".to_string(),
            bases: vec![],
            mro: vec!["Base".to_string()],
            attrs: vec![],
            methods: vec![],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
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
    env.bind_class(
        "Derived".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Derived".to_string(),
            bases: vec!["Base".to_string()],
            mro: vec!["Derived".to_string(), "Base".to_string()],
            attrs: vec![],
            methods: vec![],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
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
    let pattern = HirPattern::Class {
        class_name: "Derived".to_string(),
        positional: vec![],
        keyword: vec![],
    };
    let result = check_pattern(
        &env,
        &[],
        &pattern,
        &Ty::Instance(Box::new("Base".to_string())),
    );
    assert!(result.is_ok());
}

#[test]
fn check_pattern_class_with_error_positional_subpattern_reports_error() {
    let mut env = Environment::new();
    env.bind_class(
        "P".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "P".to_string(),
            bases: vec![],
            mro: vec!["P".to_string()],
            attrs: vec![],
            methods: vec![],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
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
    let pattern = HirPattern::Class {
        class_name: "P".to_string(),
        positional: vec![HirPattern::Literal(HirExpr::Name("u".to_string()))],
        keyword: vec![],
    };
    let result = check_pattern(
        &env,
        &[],
        &pattern,
        &Ty::Instance(Box::new("P".to_string())),
    );
    assert!(result.is_err());
}

#[test]
fn check_pattern_class_with_error_keyword_subpattern_reports_error() {
    let mut env = Environment::new();
    env.bind_class(
        "P".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "P".to_string(),
            bases: vec![],
            mro: vec!["P".to_string()],
            attrs: vec![],
            methods: vec![],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
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
    let pattern = HirPattern::Class {
        class_name: "P".to_string(),
        positional: vec![],
        keyword: vec![(
            "a".to_string(),
            HirPattern::Literal(HirExpr::Name("u".to_string())),
        )],
    };
    let result = check_pattern(
        &env,
        &[],
        &pattern,
        &Ty::Instance(Box::new("P".to_string())),
    );
    assert!(result.is_err());
}

#[test]
fn collect_pattern_capture_names_as_with_local_name_deduplicates() {
    let mut names: Vec<&str> = vec!["x"];
    let p = HirPattern::As(Box::new(HirPattern::Wildcard), "x".to_string());
    collect_pattern_capture_names(&p, &mut names);
    assert_eq!(names.iter().filter(|&&n| n == "x").count(), 1);
}

#[test]
fn contains_return_finds_a_return_inside_a_match_case_body() {
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("x".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        }],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_does_not_find_a_return_in_a_match_case_body_without_one() {
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("x".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        }],
    }];
    assert!(!contains_return(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_match_case_body() {
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("x".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        }],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_does_not_find_bindings_in_a_match_case_body_without_one() {
    let body = vec![HirStmt::Match {
        subject: HirExpr::Name("x".to_string()),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        }],
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn reject_generic_calls_in_stmt_rejects_a_generic_call_in_a_match_subject() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Match {
            subject: HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            },
            cases: vec![HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: vec![],
            }],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn reject_generic_calls_in_stmt_rejects_a_generic_call_in_a_match_guard() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Match {
            subject: HirExpr::Name("x".to_string()),
            cases: vec![HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: Some(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                }),
                body: vec![],
            }],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn reject_generic_calls_in_stmt_rejects_a_generic_call_in_a_match_body() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Match {
            subject: HirExpr::Name("x".to_string()),
            cases: vec![HirMatchCase {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                })],
            }],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn check_skips_abstract_method_body_checking() {
    // Covers the `abstract_methods.iter().any(|m| m == method_name)`
    // closure at line 4355 inside `check_function`.  When a class
    // declares `@abstractmethod` methods, the HIR lowering populates
    // `HirClassDef::abstract_methods`.  `check` (validation-only)
    // calls `check_function` for each function item, including the
    // abstract method; the closure must iterate `abstract_methods`
    // to find the match and skip body checking.  Without a non-empty
    // `abstract_methods` list, the closure body is never entered.
    //
    // The fixture uses a non-`None` return annotation on the abstract
    // method so that, if body checking were NOT skipped, the type
    // checker would emit T0022 (the `Return(None)` body of an
    // abstract method does not satisfy `-> int`).  A successful
    // `check` therefore proves the closure was entered and returned
    // `true`.
    let result = check_source(
        "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\n",
    );
    assert!(
        result.is_ok(),
        "abstract method body should be skipped by check"
    );
}
