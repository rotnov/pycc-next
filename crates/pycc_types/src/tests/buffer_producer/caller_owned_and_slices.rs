//! Caller-owned buffer returns and buffer-slice egress.

use super::*;

// ---------------------------------------------------------------------------
// Part 1 of #1175: the caller-owned buffer egress.
//
// The pre-existing egress admitted exactly one provenance -- storage this
// artifact allocated and its exporter's `tp_dealloc` frees. Part 1 admits a
// second: the name a `memoryview` *parameter* binds, returned bare. The two
// share `crate::buffer::admits_buffer_egress` and every narrowing around it,
// so each test below has an artifact-owned twin earlier in this file; what
// is new here is that the caller-owned name reaches the same verdict.
//
// #1178's scope boundary is the *bare* name only. `return b[1:3]` (#1179)
// and returning the result of a call that itself returns a buffer both stay
// refused, and the last two tests pin that so a later widening has to be a
// deliberate edit rather than a side effect.
// ---------------------------------------------------------------------------

/// `def f(b: memoryview, c: memoryview) -> memoryview: ...`.
fn two_buffer_params(body: Vec<HirStmt>) -> HirModule {
    func(
        vec![
            ("b".to_string(), Ty::MemoryView),
            ("c".to_string(), Ty::MemoryView),
        ],
        Ty::MemoryView,
        body,
    )
}

/// Either buffer parameter is admissible, not merely the first. The wrapper
/// decides which one a given call actually returned by pointer identity
/// against each declared buffer slot in turn
/// (`src/ext_build/wrappers.rs::caller_owned_buffer_acquire`), so the type
/// checker must not privilege a position either.
#[test]
fn either_buffer_parameter_is_an_admitted_caller_owned_egress() {
    for name in ["b", "c"] {
        let hir = two_buffer_params(vec![HirStmt::Return(Some(HirExpr::Name(name.to_string())))]);
        assert!(check(&hir).is_ok(), "{name}");
    }
}

/// A branch that returns a different parameter on each arm. Both returns are
/// admitted independently -- the admission is per-`return`, with no
/// whole-function "which buffer does this export hand back" analysis, which
/// is what lets the wrapper's runtime identity chain stay the only place
/// that answers the question.
#[test]
fn a_branch_returning_either_buffer_parameter_is_admitted() {
    let hir = two_buffer_params(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
        orelse: vec![HirStmt::Return(Some(HirExpr::Name("c".to_string())))],
    }]);
    assert!(check(&hir).is_ok());
}

/// The two provenances mix inside one function: a parameter is returned on
/// one arm and artifact-owned storage on the other. Nothing in the admission
/// couples them, and codegen keeps them disjoint at the source -- a
/// parameter never enters `owned_buffers` because
/// `reject_buffer_parameter_rebinding` refuses the only statement that could
/// put it there.
#[test]
fn one_function_may_return_either_provenance_on_its_own_branch() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
            orelse: vec![
                alloc_four("ndarray"),
                HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
            ],
        }],
    );
    assert!(check(&hir).is_ok());
}

/// The solver's half of the admission, on
/// [`the_solver_admits_an_owned_buffer_return`]'s model: an unannotated
/// helper in the same module forces the constraint solver to walk the
/// annotated body too. The solver runs first, so without its own copy of the
/// interception `reject_memoryview_read`'s `C0001` would displace the
/// admission before the check phase ever looked at the `return`.
///
/// This is the subset property `docs/TYPE_SYSTEM.md` states for the two
/// walkers, spelled for the caller-owned provenance.
#[test]
fn the_solver_admits_a_caller_owned_buffer_return() {
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
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
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

/// Review round 5 of #1164's `return`-inside-`finally` narrowing covers the
/// caller-owned provenance too. Part 1 deliberately keeps it there rather
/// than reasoning that a caller-owned view needs no release: the codegen's
/// single per-frame pending-return record is what the narrowing protects,
/// and that record's cardinality does not depend on which provenance the
/// suspended buffer has. Widening it is #1173's question, not Part 1's.
#[test]
fn a_return_inside_a_finally_refuses_the_caller_owned_egress() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![try_returning_from_a_loop_in_finally(
            vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
            HirExpr::Name("b".to_string()),
        )],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
}

/// ...and the solver's copy of it, for the same reason the owned twin has
/// one: the solver runs first, so an interception that admitted the egress
/// without re-checking the narrowing would let the program through.
#[test]
fn the_solver_refuses_a_caller_owned_egress_with_a_return_inside_a_finally() {
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
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![try_returning_from_a_loop_in_finally(
                    vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
                    HirExpr::Name("b".to_string()),
                )],
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
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
}

/// #1175's scope boundary, first arm: returning the *result of a call* that
/// itself returns a buffer stays refused. The admission matches an
/// `HirExpr::Name` operand only, so an intra-artifact call result never
/// reaches it -- and it must not, because the wrapper's runtime provenance
/// test compares the returned pointer against its own `args[i]` buffer
/// slots, which a value produced inside a callee frame need not match.
/// Admitting it would hand the host a view over storage nothing pins.
#[test]
fn returning_an_intra_artifact_call_result_stays_refused() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "g".to_string(),
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![HirStmt::Return(Some(call(
                    "g",
                    vec![HirExpr::Name("b".to_string())],
                )))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_err());
}

// ---------------------------------------------------------------------------
// Part 2 of #1175 (#1179): a *slice* of a buffer parameter.
//
// The third buffer-return provenance. Everything below keys on the shape
// `crate::buffer::admitted_buffer_return` now reports, which both walkers
// consume through the same `admits_buffer_egress` formula.
// ---------------------------------------------------------------------------

/// `<base>[<start>:<stop>]`, or `[<start>:<stop>:<step>]`.
fn slice_of(
    base: HirExpr,
    start: Option<HirExpr>,
    stop: Option<HirExpr>,
    step: Option<HirExpr>,
) -> HirExpr {
    HirExpr::Slice {
        base: Box::new(base),
        start: start.map(Box::new),
        stop: stop.map(Box::new),
        step: step.map(Box::new),
    }
}

/// `def f(b: memoryview) -> memoryview: return b[...]`.
fn returning_slice_of_b(start: Option<HirExpr>, stop: Option<HirExpr>) -> HirModule {
    func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            start,
            stop,
            None,
        )))],
    )
}

/// The admitted shape, in all four bound spellings. Absent bounds are not a
/// separate provenance: the compiled body encodes them as `0` and
/// `i64::MAX`, which is why they must type-check identically here.
#[test]
fn returning_a_slice_of_a_buffer_parameter_is_admitted() {
    let bounds = [
        (Some(HirExpr::IntLiteral(1)), Some(HirExpr::IntLiteral(3))),
        (None, None),
        (Some(HirExpr::IntLiteral(1)), None),
        (None, Some(HirExpr::IntLiteral(3))),
    ];
    for (start, stop) in bounds {
        let hir = returning_slice_of_b(start.clone(), stop.clone());
        assert!(check(&hir).is_ok(), "{start:?}..{stop:?}");
    }
}

/// A negative bound is admitted here and resolved host-side by CPython's own
/// `slice` semantics, so nothing in the type system needs to know about it.
#[test]
fn a_negative_slice_bound_is_admitted() {
    // The HIR lowerer folds `-2` straight into a literal, so this is the
    // shape the admitted branch really sees.
    let hir = returning_slice_of_b(Some(HirExpr::IntLiteral(-2)), None);
    assert!(check(&hir).is_ok());
}

/// A non-literal bound is admitted: the bounds leave the compiled frame
/// through out-pointers, so any `int`-typed expression serves.
#[test]
fn a_computed_slice_bound_is_admitted() {
    let hir = func(
        vec![
            ("b".to_string(), Ty::MemoryView),
            ("i".to_string(), Ty::Int),
        ],
        Ty::MemoryView,
        vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            Some(HirExpr::Name("i".to_string())),
            None,
            None,
        )))],
    );
    assert!(check(&hir).is_ok());
}

/// D2: a present `step` is refused even when it is the literal `1` that
/// changes nothing. The boundary carries `{ptr, len}` and no stride, so the
/// refusal keys on the syntax rather than on the value -- a `step` whose
/// value is only known at run time could not be refused any later.
#[test]
fn a_step_refuses_the_slice_egress() {
    for step in [HirExpr::IntLiteral(2), HirExpr::IntLiteral(1)] {
        let hir = func(
            vec![("b".to_string(), Ty::MemoryView)],
            Ty::MemoryView,
            vec![HirStmt::Return(Some(slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::IntLiteral(1)),
                Some(HirExpr::IntLiteral(3)),
                Some(step.clone()),
            )))],
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{}", err.message);
        assert!(err.message.contains("with a `step`"), "{}", err.message);
        assert!(err.message.contains("`b`"), "{}", err.message);
        // Deliberately not `reject_memoryview_read`'s wording: that message
        // tells the author to read one element at a time, which is not the
        // fix for this program.
        assert!(
            !err.message.contains("read one element at a time"),
            "{}",
            err.message
        );
    }
}

/// The admitted branch type-checks its own bounds, because the early exit
/// bypasses the ordinary `HirExpr::Slice` arm that would otherwise do it.
/// Without this the bound would reach an `i64` out-slot store with no
/// diagnostic in front of it.
#[test]
fn a_non_int_slice_bound_is_refused_by_the_check_phase() {
    for (label, start, stop) in [
        ("start", Some(HirExpr::FloatLiteral(1.5)), None),
        ("stop", None, Some(HirExpr::FloatLiteral(1.5))),
    ] {
        let hir = returning_slice_of_b(start, stop);
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0021", "{}", err.message);
        assert_eq!(
            err.message,
            format!("slice {label} must be `int`, got `float`")
        );
    }
}

/// ...and a possibly-unbound `int` bound is checked at run time
/// (`UnboundLocalError`) like any other read of one.
#[test]
fn a_possibly_unbound_slice_bound_is_checked_at_run_time() {
    let hir = func(
        vec![
            ("b".to_string(), Ty::MemoryView),
            ("k".to_string(), Ty::Int),
        ],
        Ty::MemoryView,
        vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                orelse: Vec::new(),
            },
            HirStmt::Return(Some(slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::Name("x".to_string())),
                None,
                None,
            ))),
        ],
    );
    check(&hir).expect("a possibly-unbound `int` is checked at run time");
}

/// Artifact-owned storage is *not* widened by this part: `a = ndarray(4);
/// return a[1:3]` keeps falling through to the owned-use refusal. The
/// `!owned` term lives in `admits_buffer_egress`'s formula and nowhere else,
/// so this is the one test that pins it.
#[test]
fn slicing_artifact_owned_storage_stays_refused() {
    let hir = func(
        vec![],
        Ty::MemoryView,
        vec![
            alloc_four("ndarray"),
            HirStmt::Return(Some(slice_of(
                HirExpr::Name("a".to_string()),
                Some(HirExpr::IntLiteral(1)),
                Some(HirExpr::IntLiteral(3)),
                None,
            ))),
        ],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(OWNED_BUFFER_REFUSAL),
        "{}",
        err.message
    );
}

/// The interception is a `return`-position one: a slice of a buffer
/// parameter anywhere else stays `C0001` through `reject_memoryview_read`,
/// exactly as a bare mention of the name does.
#[test]
fn a_buffer_slice_outside_return_position_stays_refused() {
    let elsewhere = [
        HirStmt::Assign {
            target: "x".to_string(),
            value: slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::IntLiteral(1)),
                None,
                None,
            ),
        },
        HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::IntLiteral(1)),
                None,
                None,
            )),
            index: Box::new(HirExpr::IntLiteral(0)),
        })),
    ];
    for stmt in elsewhere {
        let hir = func(
            vec![("b".to_string(), Ty::MemoryView)],
            Ty::MemoryView,
            vec![stmt],
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{}", err.message);
    }
}

/// The base must be a bare name. A slice of anything else is not this
/// provenance and never reaches the admitted branch.
#[test]
fn a_slice_whose_base_is_not_a_name_is_not_admitted() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![HirStmt::Return(Some(slice_of(
            slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::IntLiteral(1)),
                None,
                None,
            ),
            Some(HirExpr::IntLiteral(0)),
            None,
            None,
        )))],
    );
    assert!(check(&hir).is_err());
}

/// The `return`-inside-`finally` narrowing covers this provenance too, for
/// the same single-pending-return-record reason the other two carry it.
#[test]
fn a_return_inside_a_finally_refuses_the_slice_egress() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![try_returning_from_a_loop_in_finally(
            vec![HirStmt::Return(Some(slice_of(
                HirExpr::Name("b".to_string()),
                Some(HirExpr::IntLiteral(1)),
                None,
                None,
            )))],
            HirExpr::Name("b".to_string()),
        )],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
}

/// Both provenances of a caller-owned egress may mix inside one function,
/// and two buffer parameters may be sliced independently.
#[test]
fn a_whole_view_and_a_slice_may_share_one_function() {
    let hir = two_buffer_params(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
        orelse: vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("c".to_string()),
            Some(HirExpr::IntLiteral(1)),
            Some(HirExpr::IntLiteral(3)),
            None,
        )))],
    }]);
    assert!(check(&hir).is_ok());
}

// ---------------------------------------------------------------------------
// ...and the solver's copy of each of the above seams. The solver runs
// first, so `docs/TYPE_SYSTEM.md`'s subset property is only preserved if its
// own interception admits and refuses exactly what the check phase does.
// ---------------------------------------------------------------------------

/// A module whose annotated `g(b: memoryview) -> memoryview` has `body`,
/// beside an unannotated helper that forces the constraint solver to run.
fn solver_module_returning(body: Vec<HirStmt>) -> HirModule {
    HirModule {
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
                params: vec![
                    ("b".to_string(), Ty::MemoryView),
                    ("i".to_string(), Ty::Int),
                ],
                return_ty: Ty::MemoryView,
                body,
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
    }
}

/// The solver admits the slice egress, including a bound it must collect
/// constraints for rather than infer concretely.
#[test]
fn the_solver_admits_a_buffer_slice_return() {
    for start in [HirExpr::IntLiteral(1), HirExpr::Name("i".to_string())] {
        let hir = solver_module_returning(vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            Some(start.clone()),
            Some(HirExpr::IntLiteral(3)),
            None,
        )))]);
        assert!(check(&hir).is_ok(), "{start:?}");
    }
}

/// ...and refuses a `step` there, rather than admitting a program the check
/// phase would refuse.
#[test]
fn the_solver_refuses_a_step_on_the_slice_egress() {
    let hir = solver_module_returning(vec![HirStmt::Return(Some(slice_of(
        HirExpr::Name("b".to_string()),
        Some(HirExpr::IntLiteral(1)),
        Some(HirExpr::IntLiteral(3)),
        Some(HirExpr::IntLiteral(2)),
    )))]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(err.message.contains("with a `step`"), "{}", err.message);
}

/// ...and type-checks the bounds itself. Only a bound the solver resolved to
/// a concrete type is judged here, which is what keeps its admission a
/// subset of the check phase's rather than a different set.
#[test]
fn the_solver_refuses_a_non_int_slice_bound() {
    for (label, start, stop) in [
        ("start", Some(HirExpr::FloatLiteral(1.5)), None),
        ("stop", None, Some(HirExpr::FloatLiteral(1.5))),
    ] {
        let hir = solver_module_returning(vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            start,
            stop,
            None,
        )))]);
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0021", "{}", err.message);
        assert_eq!(
            err.message,
            format!("slice {label} must be `int`, got `float`")
        );
    }
}

/// ...and propagates a diagnostic raised while collecting the bound's own
/// constraints, rather than swallowing it and admitting a program whose
/// bound never type-checked. Reading the buffer parameter itself inside the
/// bound is such a bound: the solver's `Name` arm refuses every
/// `memoryview` read (`reject_memoryview_read`), and that refusal has to
/// travel out of the admitted branch instead of being lost with the term.
#[test]
fn the_solver_propagates_a_failure_inside_a_slice_bound() {
    for (start, stop) in [
        (Some(HirExpr::Name("b".to_string())), None),
        (None, Some(HirExpr::Name("b".to_string()))),
    ] {
        let hir = solver_module_returning(vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            start,
            stop,
            None,
        )))]);
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{}", err.message);
    }
}

/// ...and keeps the `return`-inside-`finally` narrowing.
#[test]
fn the_solver_refuses_a_slice_egress_with_a_return_inside_a_finally() {
    let hir = solver_module_returning(vec![try_returning_from_a_loop_in_finally(
        vec![HirStmt::Return(Some(slice_of(
            HirExpr::Name("b".to_string()),
            Some(HirExpr::IntLiteral(1)),
            None,
            None,
        )))],
        HirExpr::Name("b".to_string()),
    )]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
}

/// ...and does not widen artifact-owned storage either.
#[test]
fn the_solver_refuses_slicing_artifact_owned_storage() {
    let hir = solver_module_returning(vec![
        alloc_four("ndarray"),
        HirStmt::Return(Some(slice_of(
            HirExpr::Name("a".to_string()),
            Some(HirExpr::IntLiteral(1)),
            None,
            None,
        ))),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(OWNED_BUFFER_REFUSAL),
        "{}",
        err.message
    );
}
