//! Possibly-unbound owned buffers and returns inside `finally`.

use super::*;

/// `return a` from a function declared `-> memoryview`, after `body`.
fn egress_after(body: Vec<HirStmt>) -> HirModule {
    let mut stmts = body;
    stmts.push(HirStmt::Return(Some(HirExpr::Name("a".to_string()))));
    func(vec![("c".to_string(), Ty::Bool)], Ty::MemoryView, stmts)
}

/// `match c: case True: <cases.0>  case _: <cases.1>`, the one `match` shape
/// this file needs: a `bool` subject two singleton patterns cover.
fn match_bool(first: Vec<HirStmt>, wildcard: Vec<HirStmt>) -> HirStmt {
    HirStmt::Match {
        subject: HirExpr::Name("c".to_string()),
        cases: vec![
            pycc_hir::HirMatchCase {
                pattern: pycc_hir::HirPattern::Singleton(true),
                guard: None,
                body: first,
            },
            pycc_hir::HirMatchCase {
                pattern: pycc_hir::HirPattern::Wildcard,
                guard: None,
                body: wildcard,
            },
        ],
    }
}

/// `try: <body> except ValueError: <handler>`.
fn try_except(body: Vec<HirStmt>, handler: Vec<HirStmt>) -> HirStmt {
    HirStmt::Try {
        body,
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: handler,
        }],
        orelse: Vec::new(),
        finalbody: Vec::new(),
    }
}

/// The definite-assignment half of the egress admission (review round 3 of
/// #1164), refusing direction, over **every join form that can leave a name
/// in `owned_buffers` while its binding joins back as `Maybe`**.
///
/// `owned_buffers` joins as a *union* and the binding joins on the
/// `Definitely`/`Maybe`/unbound lattice, so an admission that consults only
/// the former accepts a buffer that exists on one path and not another.
/// `pycc check` reported success for `if c: a = ndarray(4)` / `return a`, and
/// codegen then loaded the null-initialized slot and handed the host an
/// internal `SystemError` where the definite-assignment contract in
/// `docs/TYPE_SYSTEM.md` owes a `T0041`.
///
/// The table is the enumeration, not an example: `if` without `else`, an
/// `if`/`else` that binds on one arm only, a `while` body, a `ForRange` body,
/// a `match` case, and a `try` body are every construct whose join can
/// produce that disagreement. `ForList` is the same join helper as
/// `ForRange` (both route through `join_loop_body`) and is covered at the
/// public-CLI level in `tests/issue_1164_memoryview_egress.rs`. A `for`
/// **loop variable** is provably not a member: binding it runs the
/// rebind-over-owned-buffer rule, which drops the name from `owned_buffers`
/// before the loop join is taken, so the name can never be both owned and
/// `Maybe` -- `a = ndarray(4)` followed by `for a in range(n)` is the
/// ordinary `T0023` for the conflicting rebinding, asserted below.
///
/// The diagnostic asserted is `T0041` specifically, not merely "an error":
/// falling through to the ordinary path must reach the possibly-unbound read,
/// not a type mismatch and not the owned-buffer `C0001`.
#[test]
fn a_possibly_unbound_owned_buffer_is_refused_at_the_egress_return() {
    let alloc = || vec![alloc_four("ndarray")];
    let forms: Vec<(&str, HirStmt)> = vec![
        (
            "if without else",
            HirStmt::If {
                test: HirExpr::Name("c".to_string()),
                body: alloc(),
                orelse: Vec::new(),
            },
        ),
        (
            "if/else binding one arm",
            HirStmt::If {
                test: HirExpr::Name("c".to_string()),
                body: alloc(),
                orelse: vec![HirStmt::Assign {
                    target: "other".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
            },
        ),
        (
            "while body",
            HirStmt::While {
                test: HirExpr::Name("c".to_string()),
                body: alloc(),
            },
        ),
        (
            "for-range body",
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(2),
                step: HirExpr::IntLiteral(1),
                body: alloc(),
            },
        ),
        ("match case", match_bool(alloc(), Vec::new())),
        ("try body", try_except(alloc(), Vec::new())),
    ];
    for (label, form) in forms {
        let err = check(&egress_after(vec![form])).unwrap_err();
        assert_eq!(err.code, "T0041", "{label}: {}", err.message);
        assert!(
            err.message.contains("may not be bound on every path"),
            "{label}: {}",
            err.message
        );
    }

    // The provably-impossible member, pinned rather than argued: a `for`
    // target that displaces an owned name is not owned after the rebinding,
    // so no join can hand the egress an owned-and-`Maybe` name this way.
    let rebound = egress_after(vec![
        alloc_four("ndarray"),
        HirStmt::ForRange {
            var: "a".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(2),
            step: HirExpr::IntLiteral(1),
            body: Vec::new(),
        },
    ]);
    let err = check(&rebound).unwrap_err();
    assert_eq!(err.code, "T0023", "{}", err.message);
}

/// The regression direction of the test above: on every one of those join
/// forms, a buffer that *is* definitely assigned is still returned.
///
/// The narrowing must cost the admission nothing it had. Each arm binds the
/// owned name on every path the join can take -- both `if` arms, both `match`
/// cases, and, for the two loop forms, before the loop as well as inside it,
/// which is what leaves the pre-existing `Definitely` binding in place across
/// the join.
///
/// #1289 gave the `try` form its admitted twin: a name bound by the body and
/// by every handler that can fall through the statement is definitely bound
/// after it, so a buffer allocated on both paths is returned. The last arm
/// pins the refusing side of that rule on the equivalent `int` program -- a
/// handler that falls through without binding the name still leaves it
/// possibly unbound -- so the `try body` refusal above is the
/// definite-assignment contract and not a buffer-specific one.
#[test]
fn a_definitely_assigned_owned_buffer_is_still_admitted_on_every_join_form() {
    let alloc = || vec![alloc_four("ndarray")];
    let admitted: Vec<(&str, Vec<HirStmt>)> = vec![
        (
            "if/else binding both arms",
            vec![HirStmt::If {
                test: HirExpr::Name("c".to_string()),
                body: alloc(),
                orelse: vec![alloc_four("NDArray")],
            }],
        ),
        (
            "while body over a pre-bound name",
            vec![
                alloc_four("ndarray"),
                HirStmt::While {
                    test: HirExpr::Name("c".to_string()),
                    body: alloc(),
                },
            ],
        ),
        (
            "for-range body over a pre-bound name",
            vec![
                alloc_four("ndarray"),
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(2),
                    step: HirExpr::IntLiteral(1),
                    body: alloc(),
                },
            ],
        ),
        (
            "match binding every case",
            vec![match_bool(alloc(), vec![alloc_four("NDArray")])],
        ),
        (
            "try body and its only handler both binding",
            vec![try_except(alloc(), vec![alloc_four("NDArray")])],
        ),
        (
            "unconditional rebinding after a one-armed join",
            vec![
                HirStmt::If {
                    test: HirExpr::Name("c".to_string()),
                    body: alloc(),
                    orelse: Vec::new(),
                },
                alloc_four("NDArray"),
            ],
        ),
    ];
    for (label, body) in admitted {
        assert!(
            check(&egress_after(body)).is_ok(),
            "{label} should still be admitted"
        );
    }

    // A handler that falls through binding only another name leaves `a`
    // possibly unbound: the same program with an `int` reads it checked at
    // run time (`UnboundLocalError`), which a buffer's static ownership
    // tracking cannot.
    let scalar = func(
        vec![("c".to_string(), Ty::Bool)],
        Ty::Int,
        vec![
            try_except(
                vec![HirStmt::Assign {
                    target: "a".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                vec![HirStmt::Assign {
                    target: "b".to_string(),
                    value: HirExpr::IntLiteral(2),
                }],
            ),
            HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
        ],
    );
    assert!(check(&scalar).is_ok());
}

/// The negative twin of `the_solver_admits_an_owned_buffer_return`: with the
/// unannotated helper in the module that routes it through the *constraint
/// solver*, a possibly-unbound owned buffer is still refused with `T0041`.
///
/// This is a walker-attribution pin rather than a discriminating one, and the
/// distinction is worth stating because the module is deliberately the solver
/// module. The solver's own `!maybe_bindings` conjunct is **not**
/// independently observable: `collect_expr_constraints`'s `Name` arm tests
/// `maybe_bindings` before it reaches `bindings`, so a maybe-bound operand
/// answers `Ok(None)` and the fall-through unifies nothing either way. What
/// this test pins is that routing a program through the solver cannot lose
/// the check phase's `T0041` -- the failure mode the egress admission has now
/// produced twice, where a walker's early return swallows a diagnostic the
/// other walker owns.
#[test]
fn the_solver_declines_a_possibly_unbound_owned_buffer_return() {
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
                    HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![alloc_four("ndarray")],
                        orelse: Vec::new(),
                    },
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
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041", "{}", err.message);
}

/// Review round 5 of #1164: the buffer egress is refused outright in a
/// function that also contains a `return` inside a `finally`.
///
/// That shape leaves two returns suspended at once, which the codegen's
/// *single* per-frame pending-return record cannot describe -- the inner
/// return supersedes the record and releases the orphaned outer buffer,
/// then a raising finalizer cancels the inner return and the outer one
/// resumes over freed storage, segfaulting the hosting interpreter. The
/// narrowing makes that record's cardinality assumption a checked property
/// of every admitted program.
#[test]
fn a_return_inside_a_finally_refuses_the_buffer_egress() {
    let hir = func(
        vec![],
        Ty::MemoryView,
        vec![
            alloc_four("ndarray"),
            try_returning_from_a_loop_in_finally(
                vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
                HirExpr::Name("a".to_string()),
            ),
        ],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
    assert!(
        !err.message.contains(OWNED_BUFFER_REFUSAL_TAIL),
        "{}",
        err.message
    );
}

/// The solver's half of the same narrowing, on
/// [`the_solver_admits_an_owned_buffer_return`]'s model: the module carries
/// an unannotated helper so the constraint solver walks the annotated body
/// too. The solver runs first, so without its own copy of the fourth
/// conjunct it would admit the egress and the program would compile.
#[test]
fn the_solver_refuses_a_buffer_egress_with_a_return_inside_a_finally() {
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
                    try_returning_from_a_loop_in_finally(
                        vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
                        HirExpr::Name("a".to_string()),
                    ),
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
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
    assert!(
        err.message.contains(RETURN_IN_FINALLY_REFUSAL),
        "{}",
        err.message
    );
}

/// The narrowing is gated at the egress admission, not on the function as a
/// whole: a `-> int` function with a `return` inside a `finally` keeps
/// exactly the behavior it had before review round 5. Without this pin the
/// refusal could be moved up to `check_function_in` and break every
/// non-buffer program with that shape while every other test here stayed
/// green.
#[test]
fn a_return_inside_a_finally_leaves_a_non_buffer_function_alone() {
    let hir = func(
        vec![],
        Ty::Int,
        vec![try_returning_from_a_loop_in_finally(
            vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            HirExpr::IntLiteral(3),
        )],
    );
    assert!(check(&hir).is_ok());
}

/// A `finally` with no `return` beneath it does not narrow anything: the
/// surviving admitted shape is a single pending return with finalizers
/// interposed, which `tests/issue_1164_memoryview_egress.rs` exercises
/// end-to-end against a real CPython host.
#[test]
fn a_finally_without_a_return_still_admits_the_buffer_egress() {
    let hir = func(
        vec![],
        Ty::MemoryView,
        vec![
            alloc_four("ndarray"),
            HirStmt::Try {
                body: vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
                handlers: Vec::new(),
                orelse: Vec::new(),
                finalbody: vec![alloc_four("ndarray")],
            },
        ],
    );
    assert!(check(&hir).is_ok());
}
