//! Buffer producers on the solver path and `Final` buffer names.

use super::*;

/// The solver's plain-`Assign` seam: an unannotated helper may allocate and
/// read back one element, exactly as an annotated function may.
#[test]
fn the_solver_admits_a_producer_in_an_unannotated_helper() {
    let hir = unannotated_helper_module(
        vec![
            HirStmt::Assign {
                target: "a".to_string(),
                value: call("ndarray", vec![HirExpr::Name("n".to_string())]),
            },
            HirStmt::Return(Some(element())),
        ],
        Ty::Float,
    );
    assert!(check(&hir).is_ok());
}

/// The solver's `AnnAssign` seam, which the plain-`Assign` seam above does
/// not reach: the annotated spelling of the same statement.
#[test]
fn the_solver_admits_an_annotated_producer_in_an_unannotated_helper() {
    let hir = unannotated_helper_module(
        vec![
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::MemoryView,
                value: Some(call("ndarray", vec![HirExpr::Name("n".to_string())])),
                is_final: false,
            },
            HirStmt::Return(Some(element())),
        ],
        Ty::Float,
    );
    assert!(check(&hir).is_ok());
}

/// A diagnostic raised while the solver collects the length argument's own
/// constraints propagates out of both seams rather than being swallowed by
/// the producer's admission.
#[test]
fn the_solver_propagates_a_refusal_from_the_length_argument() {
    for value in [
        // A nested producer: the *inner* call is in a length-argument
        // position, not an assignment's right-hand side, so collecting the
        // outer call's length argument is what raises the refusal.
        call(
            "ndarray",
            vec![call("ndarray", vec![HirExpr::IntLiteral(4)])],
        ),
        call("ndarray", vec![HirExpr::Name("missing".to_string())]),
    ] {
        for stmt in [
            HirStmt::Assign {
                target: "a".to_string(),
                value: value.clone(),
            },
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::MemoryView,
                value: Some(value.clone()),
                is_final: false,
            },
        ] {
            let hir = unannotated_helper_module(
                vec![stmt.clone(), HirStmt::Return(Some(element()))],
                Ty::Float,
            );
            assert!(check(&hir).is_err(), "{stmt:?}");
        }
    }
}

/// The solver's own parameter-rebinding guard, in both assignment
/// spellings. Without it the solver admits the rebinding, marks `b`
/// artifact-owned, and the later read of `b` raises the *owned* refusal --
/// which `module::merge_solver_first` then prefers over the check phase's
/// correct *parameter* refusal, making #1165's acceptance criterion 2 false
/// for this reachable shape.
///
/// `_h` takes a concrete `memoryview` parameter but leaves its return type
/// inferred, which is what routes it through the constraint solver rather
/// than the concrete fast path (see `unannotated_helper_module`).
#[test]
fn the_solver_refuses_rebinding_a_buffer_parameter() {
    for assignment in [
        HirStmt::Assign {
            target: "b".to_string(),
            value: call("ndarray", vec![HirExpr::IntLiteral(4)]),
        },
        HirStmt::AnnAssign {
            target: "b".to_string(),
            annotation: Ty::MemoryView,
            value: Some(call("ndarray", vec![HirExpr::IntLiteral(4)])),
            is_final: false,
        },
    ] {
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                HirItem::Function {
                    name: "_h".to_string(),
                    params: vec![("b".to_string(), Ty::MemoryView)],
                    return_ty: Ty::Infer,
                    body: vec![
                        assignment.clone(),
                        HirStmt::Return(Some(HirExpr::Name("b".to_string()))),
                    ],
                },
                HirItem::Function {
                    name: "f".to_string(),
                    params: vec![("v".to_string(), Ty::MemoryView)],
                    return_ty: Ty::MemoryView,
                    body: vec![HirStmt::Return(Some(call(
                        "_h",
                        vec![HirExpr::Name("v".to_string())],
                    )))],
                },
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{assignment:?}");
        assert!(
            err.message
                .contains("a buffer parameter of a `pycc build --ext` export"),
            "{}",
            err.message
        );
    }
}

/// Codex review round 8: the solver collected the producer's length term and
/// threw it away, so `a = ndarray("x")` in an unannotated helper recorded an
/// owned buffer, the later read raised the owned-buffer `C0001`, and
/// `module::merge_solver_first` reported that instead of the check phase's
/// correct `T0033`.
#[test]
fn the_solver_refuses_a_non_int_producer_length() {
    for callee in ["ndarray", "NDArray"] {
        let producer = call(callee, vec![HirExpr::StringLiteral("x".to_string())]);
        for assignment in [
            HirStmt::Assign {
                target: "a".to_string(),
                value: producer.clone(),
            },
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::MemoryView,
                value: Some(producer.clone()),
                is_final: false,
            },
        ] {
            let hir = unannotated_helper_module(
                vec![
                    assignment.clone(),
                    read_whole_buffer(),
                    HirStmt::Return(Some(HirExpr::IntLiteral(0))),
                ],
                Ty::Int,
            );
            let err = check(&hir).unwrap_err();
            assert_eq!(err.code, "T0033", "{assignment:?}");
            assert_eq!(
                err.message,
                format!("`{callee}` expects an `int` element count, got `str`"),
            );
            assert!(
                !err.message.contains(OWNED_BUFFER_REFUSAL),
                "{}",
                err.message
            );
        }
    }
}

/// The same masking through the `AnnAssign` arm's *other* missing
/// condition: an annotation that does not admit a buffer. The solver bound
/// `a` as artifact-owned regardless of what `a` was declared as, so the
/// later read displaced the check phase's `T0025`.
///
/// The expected message is taken from the check phase's own answer to the
/// identical statement in an annotated function rather than written out
/// here, which is what pins the two phases to the one canonical `T0025`
/// (`crate::annotation_initializer_mismatch`).
#[test]
fn the_solver_refuses_a_producer_under_an_annotation_that_rejects_it() {
    for annotation in [Ty::Int, Ty::Str, Ty::List(Box::new(Ty::Int))] {
        let assignment = HirStmt::AnnAssign {
            target: "a".to_string(),
            annotation: annotation.clone(),
            value: Some(call("ndarray", vec![HirExpr::IntLiteral(4)])),
            is_final: false,
        };
        let solver = check(&unannotated_helper_module(
            vec![
                assignment.clone(),
                read_whole_buffer(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
            Ty::Int,
        ))
        .unwrap_err();
        let checker = check(&func(
            vec![],
            Ty::Int,
            vec![
                assignment.clone(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        ))
        .unwrap_err();
        assert_eq!(solver.code, "T0025", "{annotation:?}");
        assert_eq!(solver.message, checker.message, "{annotation:?}");
        assert_eq!(solver.help, checker.help, "{annotation:?}");
        assert!(
            !solver.message.contains(OWNED_BUFFER_REFUSAL),
            "{}",
            solver.message
        );
    }
}

/// The keep path for both guards above: a *valid* producer in an
/// unannotated helper is still recognized, still marks the name
/// artifact-owned, and a later whole-buffer read is still the owned-buffer
/// `C0001`. Neither guard may over-suppress into admitting nothing.
#[test]
fn a_valid_producer_in_an_unannotated_helper_still_owns_its_buffer() {
    for annotation in [None, Some(Ty::MemoryView)] {
        let value = call("ndarray", vec![HirExpr::IntLiteral(4)]);
        let assignment = match annotation.clone() {
            None => HirStmt::Assign {
                target: "a".to_string(),
                value,
            },
            Some(annotation) => HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation,
                value: Some(value),
                is_final: false,
            },
        };
        let hir = unannotated_helper_module(
            vec![
                assignment.clone(),
                read_whole_buffer(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
            Ty::Int,
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{assignment:?}");
        assert!(
            err.message.contains(OWNED_BUFFER_REFUSAL),
            "{}",
            err.message
        );
    }
}

/// The length guard admits a `bool`, exactly as the check phase's own
/// `Int | Bool` match does (`docs/TYPE_SYSTEM.md` rule 4/D-086), and admits
/// a length term that is still unresolved at the seam -- the helper's own
/// unannotated parameter, which is the admitted shape the guard must not
/// refuse.
#[test]
fn the_solver_admits_a_bool_length_and_an_unresolved_one() {
    for length in [HirExpr::BoolLiteral(true), HirExpr::Name("n".to_string())] {
        let hir = unannotated_helper_module(
            vec![
                HirStmt::Assign {
                    target: "a".to_string(),
                    value: call("ndarray", vec![length.clone()]),
                },
                HirStmt::Return(Some(element())),
            ],
            Ty::Float,
        );
        assert!(check(&hir).is_ok(), "{length:?}");
    }
}

/// `a: Final[<annotation>] = ndarray(4)`.
fn final_producer(annotation: Ty) -> HirStmt {
    HirStmt::AnnAssign {
        target: "a".to_string(),
        annotation,
        value: Some(call("ndarray", vec![HirExpr::IntLiteral(4)])),
        is_final: true,
    }
}

/// The fourth member of the same class, found by enumerating
/// `check_assignment` rather than reported: PEP 591's `T0045` is one of the
/// refusals that function applies to *every* assignment target, and the
/// solver's producer seam applied none of it but the parameter-rebinding
/// guard. A reassignment of a `Final` buffer name was therefore admitted by
/// the solver, recorded as artifact-owned, and the later read displaced the
/// check phase's `T0045` with the owned-buffer `C0001`.
///
/// Both producer arms are driven: the reassignment is the plain `Assign`
/// spelling in the first case and the annotated one in the second.
#[test]
fn the_solver_refuses_reassigning_a_final_buffer_name() {
    for reassignment in [
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
                final_producer(Ty::MemoryView),
                reassignment.clone(),
                read_whole_buffer(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
            Ty::Int,
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0045", "{reassignment:?}");
        assert_eq!(err.message, "cannot reassign `Final` name `a`");
        assert!(
            !err.message.contains(OWNED_BUFFER_REFUSAL),
            "{}",
            err.message
        );
    }
}

/// The keep path for the `Final` guard: the declaration's *own* first
/// assignment is not a reassignment, so a `Final` buffer binding is still
/// admitted and still artifact-owned. A guard recording finality before the
/// binding, rather than after it as the check phase does, would refuse this.
#[test]
fn a_final_buffer_declaration_is_still_admitted_and_still_owned() {
    let hir = unannotated_helper_module(
        vec![
            final_producer(Ty::MemoryView),
            read_whole_buffer(),
            HirStmt::Return(Some(HirExpr::IntLiteral(0))),
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

/// A `Final` name declared inside one arm of an `if` is still `Final` after
/// the join, so a reassignment below it is refused: the solver collects each
/// branch in a clone of the environment, and the join must union the set the
/// way it already unions buffer provenance.
#[test]
fn final_names_survive_a_branch_and_a_loop_join() {
    for wrap in [
        |stmt: HirStmt| HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![stmt],
            orelse: Vec::new(),
        },
        |stmt: HirStmt| HirStmt::While {
            test: HirExpr::BoolLiteral(false),
            body: vec![stmt],
        },
    ] {
        let hir = unannotated_helper_module(
            vec![
                wrap(final_producer(Ty::MemoryView)),
                HirStmt::Assign {
                    target: "a".to_string(),
                    value: call("ndarray", vec![HirExpr::IntLiteral(8)]),
                },
                read_whole_buffer(),
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
            Ty::Int,
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0045");
    }
}
