//! Structural pattern matching lowering (`matching::lower_match`).
//!
//! Covers literal, singleton, capture, sequence, star, mapping, mapping-rest,
//! class, or, as, and wildcard patterns, guards, empty cases, and
//! `nest_match_alternatives` over an empty alternative list.

use crate::*;
use pycc_hir::{
    CmpOpKind, HirClassDef, HirExpr, HirItem, HirMatchCase, HirModule, HirPattern, HirStmt, Ty,
};

// -- #381: match statement MIR lowering coverage -----------------------

fn match_module(cases: Vec<HirMatchCase>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::Match {
                subject: HirExpr::Name("x".to_string()),
                cases,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

fn match_module_list(cases: Vec<HirMatchCase>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Match {
                subject: HirExpr::Name("x".to_string()),
                cases,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

fn match_module_dict(cases: Vec<HirMatchCase>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("k".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::Match {
                subject: HirExpr::Name("x".to_string()),
                cases,
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

#[test]
fn lowers_match_with_literal_pattern_to_mir() {
    let hir = match_module(vec![
        HirMatchCase {
            pattern: HirPattern::Literal(HirExpr::IntLiteral(1)),
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(0)],
            })],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_singleton_pattern_to_mir() {
    let hir = match_module(vec![
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
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_none_singleton_pattern_to_mir() {
    let hir = match_module(vec![HirMatchCase {
        pattern: HirPattern::NoneSingleton,
        guard: None,
        body: vec![],
    }]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_capture_pattern_to_mir() {
    let hir = match_module(vec![HirMatchCase {
        pattern: HirPattern::Capture("y".to_string()),
        guard: None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("y".to_string())],
        })],
    }]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_sequence_pattern_to_mir() {
    let hir = match_module_list(vec![
        HirMatchCase {
            pattern: HirPattern::Sequence(vec![
                HirPattern::Capture("a".to_string()),
                HirPattern::Capture("b".to_string()),
            ]),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_sequence_star_pattern_to_mir() {
    let hir = match_module_list(vec![
        HirMatchCase {
            pattern: HirPattern::SequenceStar(
                vec![HirPattern::Capture("a".to_string())],
                Some("rest".to_string()),
            ),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_mapping_pattern_to_mir() {
    let hir = match_module_dict(vec![
        HirMatchCase {
            pattern: HirPattern::Mapping(
                vec![(
                    HirExpr::StringLiteral("k".to_string()),
                    HirPattern::Capture("v".to_string()),
                )],
                None,
            ),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_mapping_rest_pattern_to_mir() {
    let hir = match_module_dict(vec![
        HirMatchCase {
            pattern: HirPattern::Mapping(
                vec![(
                    HirExpr::StringLiteral("k".to_string()),
                    HirPattern::Capture("v".to_string()),
                )],
                Some("rest".to_string()),
            ),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_class_pattern_to_mir() {
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "P".to_string(),
        bases: Vec::new(),
        mro: vec!["P".to_string()],
        attrs: vec![("a".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), "P.__init__".to_string())],
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
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::Match {
                subject: HirExpr::Name("x".to_string()),
                cases: vec![
                    HirMatchCase {
                        pattern: HirPattern::Class {
                            class_name: "P".to_string(),
                            positional: vec![HirPattern::Capture("a".to_string())],
                            keyword: vec![("a".to_string(), HirPattern::Capture("a2".to_string()))],
                        },
                        guard: None,
                        body: vec![],
                    },
                    HirMatchCase {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: vec![],
                    },
                ],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("P".to_string(), class_def)],
    };
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_or_pattern_to_mir() {
    let hir = match_module(vec![
        HirMatchCase {
            pattern: HirPattern::Or(vec![
                HirPattern::Literal(HirExpr::IntLiteral(1)),
                HirPattern::Literal(HirExpr::IntLiteral(2)),
                HirPattern::Literal(HirExpr::IntLiteral(3)),
            ]),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_as_pattern_to_mir() {
    let hir = match_module(vec![
        HirMatchCase {
            pattern: HirPattern::As(
                Box::new(HirPattern::Literal(HirExpr::IntLiteral(1))),
                "y".to_string(),
            ),
            guard: None,
            body: vec![],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_guard_to_mir() {
    let hir = match_module(vec![
        HirMatchCase {
            pattern: HirPattern::Capture("y".to_string()),
            guard: Some(HirExpr::Compare {
                op: CmpOpKind::Gt,
                left: Box::new(HirExpr::Name("y".to_string())),
                right: Box::new(HirExpr::IntLiteral(3)),
            }),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("y".to_string())],
            })],
        },
        HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        },
    ]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_empty_cases_to_mir() {
    let hir = match_module(vec![]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn lowers_match_with_wildcard_only_to_mir() {
    let hir = match_module(vec![HirMatchCase {
        pattern: HirPattern::Wildcard,
        guard: None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(0)],
        })],
    }]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}

#[test]
fn nest_match_alternatives_with_empty_alternatives_returns_seq() {
    let inner_body = vec![MirStmt::ExprStmt(MirExpr::IntLiteral(0))];
    let else_chain = MirStmt::ExprStmt(MirExpr::IntLiteral(1));
    let result = nest_match_alternatives(&[], inner_body, else_chain);
    assert!(matches!(result, MirStmt::Seq(body) if body.len() == 1));
}

#[test]
fn lowers_match_with_or_pattern_with_bindings_to_mir() {
    let hir = match_module(vec![HirMatchCase {
        pattern: HirPattern::Or(vec![
            HirPattern::Capture("a".to_string()),
            HirPattern::Capture("b".to_string()),
        ]),
        guard: None,
        body: vec![],
    }]);
    let mir = build(&hir);
    assert!(!mir.items.is_empty());
}
