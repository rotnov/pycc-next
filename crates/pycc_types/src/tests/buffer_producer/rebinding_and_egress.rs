//! Rebinding and reallocating owned buffers; returning them.

use super::*;

/// `a = <value>`, the reviewer's own shape.
fn rebind_a(value: HirExpr) -> HirStmt {
    HirStmt::Assign {
        target: "a".to_string(),
        value,
    }
}

/// `range(0, 3, 1)` as a comprehension iterable.
fn comp_range() -> CompIter {
    CompIter::Range {
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
    }
}

/// Every statement shape that rebinds the name `a`, each preceded by
/// `a = ndarray(4)` in [`a_rebinding_of_an_owned_buffer_name_drops_its_
/// stale_provenance`].
///
/// A table rather than prose because it *is* the enumeration the round-11
/// finding asked for: the reviewer's counter-example was the plain `Assign`
/// arm, and the condition it belongs to is "any binder that is not the
/// producer seam". Every member below was confirmed to mask the check
/// phase's own diagnostic before the fix.
///
/// `ForRange` is in the table as a regression pin rather than as a member
/// that was broken: its arm already unifies the existing term against
/// `Ty::Int` and so already reported `T0023`. The `while`/`if` entries are
/// the two join helpers, which is where a per-site invalidation alone is
/// undone by the provenance union.
fn owned_buffer_rebinding_shapes() -> Vec<(&'static str, Vec<HirStmt>)> {
    vec![
        ("assign", vec![rebind_a(HirExpr::IntLiteral(1))]),
        (
            "annassign",
            vec![HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
                is_final: false,
            }],
        ),
        (
            "walrus",
            vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
                name: "a".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            })],
        ),
        (
            "for-range",
            vec![HirStmt::ForRange {
                var: "a".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            }],
        ),
        (
            "for-list",
            vec![
                HirStmt::Assign {
                    target: "lst".to_string(),
                    value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
                },
                HirStmt::ForList {
                    var: "a".to_string(),
                    list: "lst".to_string(),
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
                },
            ],
        ),
        (
            "for-object",
            vec![HirStmt::ForObject {
                var: "a".to_string(),
                iter: Box::new(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("n".to_string())),
                    attr: "rows".to_string(),
                }),
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            }],
        ),
        (
            "list-comprehension",
            vec![HirStmt::ListCompAssign {
                target: "a".to_string(),
                var: "q".to_string(),
                iter: comp_range(),
                cond: None,
                elt: Box::new(HirExpr::Name("q".to_string())),
            }],
        ),
        (
            "set-comprehension",
            vec![HirStmt::SetCompAssign {
                target: "a".to_string(),
                var: "q".to_string(),
                iter: comp_range(),
                cond: None,
                elt: Box::new(HirExpr::Name("q".to_string())),
            }],
        ),
        (
            "dict-comprehension",
            vec![HirStmt::DictCompAssign {
                target: "a".to_string(),
                var: "q".to_string(),
                iter: comp_range(),
                cond: None,
                key: Box::new(HirExpr::Name("q".to_string())),
                value: Box::new(HirExpr::Name("q".to_string())),
            }],
        ),
        (
            "match-capture",
            vec![HirStmt::Match {
                subject: HirExpr::IntLiteral(1),
                cases: vec![pycc_hir::HirMatchCase {
                    pattern: pycc_hir::HirPattern::Capture("a".to_string()),
                    guard: None,
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
                }],
            }],
        ),
        (
            "except-as",
            vec![HirStmt::Try {
                body: vec![HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("a".to_string()),
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }],
        ),
        (
            "except-star-as",
            vec![HirStmt::TryStar {
                body: vec![HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                handlers: vec![pycc_hir::HirExceptHandler {
                    exc_type: Some(vec!["ValueError".to_string()]),
                    name: Some("a".to_string()),
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }],
        ),
        (
            "loop-body",
            vec![HirStmt::While {
                test: HirExpr::BoolLiteral(false),
                body: vec![rebind_a(HirExpr::IntLiteral(1))],
            }],
        ),
        (
            "one-branch",
            vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![rebind_a(HirExpr::IntLiteral(1))],
                orelse: Vec::new(),
            }],
        ),
    ]
}

/// Round-11 review finding 2: `a = ndarray(4)` then `a = 1` then `b = a` in
/// an unannotated private helper reported the artifact-owned `C0001` for the
/// read instead of the check phase's `T0023` for the reassignment. The
/// solver kept `a`'s original `Ok(Ty::MemoryView)` term (ordinary
/// assignments are first-term-wins) *and* its `owned_buffers` marker, so the
/// read was refused before `module::merge_solver_first` could prefer the
/// correct answer.
///
/// Every shape in [`owned_buffer_rebinding_shapes`] is driven, and the
/// expected diagnostic is taken from the check phase's own answer to the
/// identical program without the read rather than written out here -- the
/// technique `the_solver_refuses_a_producer_under_an_annotation_that_rejects
/// _it` established, and what keeps this table from pinning a message that
/// could drift from `check_assignment`'s.
#[test]
fn a_rebinding_of_an_owned_buffer_name_drops_its_stale_provenance() {
    // Every mismatch is collected rather than asserted in place: the table
    // is the enumeration, so a failure must name *which* shapes regressed,
    // not only the first one.
    let mut wrong: Vec<String> = Vec::new();
    for (label, rebinding) in owned_buffer_rebinding_shapes() {
        let mut body = vec![alloc_four("ndarray")];
        body.extend(rebinding);
        let mut without_read = body.clone();
        without_read.push(HirStmt::Return(Some(HirExpr::IntLiteral(0))));
        body.push(read_whole_buffer());
        body.push(HirStmt::Return(Some(HirExpr::IntLiteral(0))));

        let expected = check(&unannotated_helper_module(without_read, Ty::Int)).unwrap_err();
        let actual = check(&unannotated_helper_module(body, Ty::Int)).unwrap_err();
        if actual.code != expected.code
            || actual.message != expected.message
            || actual.message.contains(OWNED_BUFFER_REFUSAL)
            || actual.message.contains(PARAMETER_BUFFER_REFUSAL)
        {
            wrong.push(format!(
                "{label}: got {} `{}`, expected {} `{}`",
                actual.code, actual.message, expected.code, expected.message
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The reviewer's own program, pinned code-for-code rather than against the
/// check phase's answer, in both the unannotated-helper spelling that the
/// solver infers and the fully annotated spelling that it does not. The
/// annotated spelling masked identically before the fix -- this solver walks
/// every function body, not only the ones whose signature it has to infer --
/// so a fix verified on the helper alone would have left half the class
/// open.
#[test]
fn reassigning_an_owned_buffer_to_an_int_reports_the_reassignment() {
    let body = vec![
        alloc_four("ndarray"),
        rebind_a(HirExpr::IntLiteral(1)),
        read_whole_buffer(),
        HirStmt::Return(Some(HirExpr::IntLiteral(0))),
    ];
    for hir in [
        unannotated_helper_module(body.clone(), Ty::Int),
        func(vec![], Ty::Int, body.clone()),
    ] {
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0023");
        assert_eq!(
            err.message,
            "cannot assign `int` to `a`, previously inferred as `memoryview`"
        );
        assert!(!err.message.contains(OWNED_BUFFER_REFUSAL));
        assert!(!err.message.contains(PARAMETER_BUFFER_REFUSAL));
    }
}

/// The keep path the invalidation must not swallow: reassigning an owned
/// name to *another* buffer keeps it owned, because codegen frees the
/// previous allocation before the store (D-074) and `check_assignment`
/// admits the rebinding for exactly that reason. Driven through both
/// producer arms.
#[test]
fn reallocating_an_owned_buffer_keeps_its_provenance() {
    for realloc in [
        HirStmt::Assign {
            target: "a".to_string(),
            value: call("ndarray", vec![HirExpr::IntLiteral(8)]),
        },
        HirStmt::AnnAssign {
            target: "a".to_string(),
            annotation: Ty::MemoryView,
            value: Some(call("ndarray", vec![HirExpr::IntLiteral(8)])),
            is_final: false,
        },
    ] {
        let hir = unannotated_helper_module(
            vec![
                alloc_four("ndarray"),
                realloc.clone(),
                read_whole_buffer(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
            Ty::Int,
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{realloc:?}");
        assert!(
            err.message.contains(OWNED_BUFFER_REFUSAL),
            "{}",
            err.message
        );
    }
}

/// The join's own keep path, and why the invalidation is keyed on "some
/// branch binds this name to something else" rather than on intersecting the
/// two owned sets: a name each branch allocates is still owned after the
/// join, and a name only one branch allocates must not be dropped merely
/// because the other branch never mentions it.
#[test]
fn a_buffer_allocated_in_every_branch_is_still_owned_after_the_join() {
    let hir = unannotated_helper_module(
        vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![alloc_four("ndarray")],
                orelse: vec![alloc_four("NDArray")],
            },
            HirStmt::Return(Some(element())),
        ],
        Ty::Float,
    );
    assert!(check(&hir).is_ok());
}

/// The contested case the join helper is for: one branch allocates, the
/// other binds the same name to an `int`. Neither per-site invalidation nor
/// the provenance union can answer this alone -- the branch that allocates
/// re-adds the marker and its `Ty::MemoryView` term wins the first-term-wins
/// binding merge -- so the read reported the owned-buffer `C0001` over the
/// check phase's `T0023` for the conflicting join.
#[test]
fn a_buffer_allocated_in_only_one_branch_is_contested_by_the_other() {
    let hir = unannotated_helper_module(
        vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![alloc_four("ndarray")],
                orelse: vec![rebind_a(HirExpr::IntLiteral(1))],
            },
            read_whole_buffer(),
            HirStmt::Return(Some(HirExpr::IntLiteral(0))),
        ],
        Ty::Int,
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0023");
    assert!(!err.message.contains(OWNED_BUFFER_REFUSAL));
    assert!(!err.message.contains(PARAMETER_BUFFER_REFUSAL));
}

/// The solver half of Part 2b of #1142 (#1164)'s egress interception. The
/// module carries an unannotated helper so the constraint solver -- not the
/// concrete fast path -- is what walks `g`'s `return a`, and the admission
/// must hold there too: the solver runs first, so without its own
/// interception the owned-buffer `C0001` would displace the admission before
/// the check phase ever looked.
#[test]
fn the_solver_admits_an_owned_buffer_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("n".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("n".to_string())))],
            },
            HirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: Ty::MemoryView,
                body: vec![
                    alloc_four("ndarray"),
                    HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
                ],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(call(
                    "_h",
                    vec![HirExpr::IntLiteral(4)],
                )))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

/// An *inferred* return type declines the egress admission: only a written
/// `-> memoryview` annotation admits one, so an unannotated helper that
/// returns its own owned storage keeps the owned-buffer refusal.
#[test]
fn an_inferred_return_type_does_not_admit_an_owned_buffer_return() {
    let hir = unannotated_helper_module(
        vec![
            alloc_four("ndarray"),
            HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
        ],
        Ty::Int,
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains(OWNED_BUFFER_REFUSAL),
        "{}",
        err.message
    );
}

/// An intra-artifact call to a buffer-returning function is refused by the
/// check phase, naming the callee. This is what keeps
/// `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView` panic
/// unreachable from source (#1164's third completion criterion).
#[test]
fn calling_a_buffer_returning_function_is_refused() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "make".to_string(),
                params: vec![],
                return_ty: Ty::MemoryView,
                body: vec![
                    alloc_four("ndarray"),
                    HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
                ],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::MemoryView,
                body: vec![HirStmt::Return(Some(call("make", vec![])))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("calling `make`, whose return type is a buffer"),
        "{}",
        err.message
    );
    assert!(
        !err.message.contains(OWNED_BUFFER_REFUSAL),
        "{}",
        err.message
    );
}

/// ...and the solver refuses the same call with the same text, so the two
/// walkers cannot drift. The unannotated helper is what routes the call
/// through the solver rather than the concrete fast path.
#[test]
fn the_solver_refuses_calling_a_buffer_returning_function() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "make".to_string(),
                params: vec![],
                return_ty: Ty::MemoryView,
                body: vec![
                    alloc_four("ndarray"),
                    HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
                ],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("n".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(call("make", vec![])))],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(call(
                    "_h",
                    vec![HirExpr::IntLiteral(4)],
                )))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("calling `make`, whose return type is a buffer"),
        "{}",
        err.message
    );
}

/// The egress admission in `check_stmt_in_function` skips the ordinary
/// assignability check whenever the returned name is in `owned_buffers`,
/// trusting that membership implies a `Ty::MemoryView` term. That invariant
/// is maintained by the contested-join invalidation rule, not by the
/// admission itself, so this pins the one shape where a regression in that
/// rule would turn into a *silent* admission rather than a diagnostic: a
/// contested name returned from a function that really is annotated
/// `-> memoryview`. Every other contested read reports through a position
/// the admission never reaches.
#[test]
fn a_contested_buffer_is_not_silently_admitted_by_the_egress_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "make".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                HirStmt::If {
                    test: HirExpr::BoolLiteral(true),
                    body: vec![alloc_four("ndarray")],
                    orelse: vec![rebind_a(HirExpr::IntLiteral(1))],
                },
                HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0023", "{}", err.message);
    assert!(
        !err.message.contains(OWNED_BUFFER_REFUSAL),
        "{}",
        err.message
    );
}
