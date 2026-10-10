//! memoryview capability gaps, buffer reads/stores and `len` of a buffer.

use super::*;

// Part 1 of #1027: a `memoryview` parameter is admitted at a
// `pycc build --ext` boundary, and the plan's section 3.5 admits *no*
// operation on the value it binds. `reject_memoryview_read` states that by
// refusing the read itself, which is the only expression that can produce a
// `memoryview` at all -- there is no literal and no producing call. Without
// it `pycc_codegen` reaches a local load it has no lowering for and panics
// instead of diagnosing.
//
// The subject's declared return type is `int` rather than `memoryview`, and
// that is the whole of what Part 1 of #1175 changed here. `-> memoryview`
// with `return v` is now the *admitted* caller-owned egress
// (`crate::tests::buffer_producer`), so it no longer reaches this refusal;
// every other read of the name still does, and a mismatched declared return
// type is the shortest one to write. The message this pins is unchanged.
#[test]
fn reading_a_memoryview_parameter_is_a_capability_gap_rather_than_an_ice() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("v".to_string(), Ty::MemoryView)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("`v`"), "{}", err.message);
    assert!(err.message.contains("buffer parameter"), "{}", err.message);
    assert!(err.message.contains("pycc build --ext"), "{}", err.message);
}

// The companion negative: the guard keys on the type, not on the name, so a
// parameter of any other type still reads normally. Without this arm the
// `Ok(())` fall-through of `reject_memoryview_read` is never executed by a
// read that reaches it with a non-`memoryview` binding.
#[test]
fn reading_a_parameter_of_any_other_type_is_unaffected_by_the_memoryview_guard() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("v".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// Part 1 of #1027, round 1 of the pinned review: `annotation_to_ty` parses a
// bare `x: T` declaration as well as a signature, so admitting
// `Ty::MemoryView` there admitted the declaration too -- and a value-less
// `AnnAssign` lowers to a `MirStmt::NoOp`, so the program compiled silently
// where `x: object` is still refused. Both `AnnAssign` arms are covered: the
// function-scope one here, the module-scope one below.
#[test]
fn declaring_a_memoryview_local_is_a_capability_gap() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: Vec::new(),
            return_ty: Ty::Int,
            body: vec![
                HirStmt::AnnAssign {
                    target: "x".to_string(),
                    annotation: Ty::MemoryView,
                    value: None,
                    is_final: false,
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(1))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("declaring `x` as a buffer"),
        "{}",
        err.message
    );
    assert!(err.message.contains("pycc build --ext"), "{}", err.message);
    // Same correction as the read arm: the refusal stands, but not on the
    // claim that a buffer is admitted only as a parameter (#1165).
    assert!(
        !err.message.contains("only as a parameter"),
        "{}",
        err.message
    );
}

// Part 1 of #1027, round 3 of the pinned review: `HirStmt::ForList` holds
// its iterable as a plain `String` (D-105), so `for x in v` resolves through
// `lookup_bound_name` and never reaches `infer_expr_in`'s own `Name` arm.
// Before the guard landed at that shared seam the loop was refused as a
// `T0033` -- "`memoryview` cannot be iterated" -- which is false about
// Python and mislabels a capability gap as a type error.
#[test]
fn iterating_a_memoryview_parameter_is_the_read_capability_gap() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("v".to_string(), Ty::MemoryView)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForList {
                    var: "x".to_string(),
                    list: "v".to_string(),
                    body: Vec::new(),
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(1))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("using `v`, which is bound to a buffer parameter"),
        "{}",
        err.message
    );
    // The prefix above is pinned by eight assertions; the tail may not
    // regress to claiming a buffer originates only as such a parameter,
    // which Part 2a of #1142 (#1165) falsified with `ndarray(n)`.
    assert!(
        !err.message.contains("only as such a parameter"),
        "{}",
        err.message
    );
}

// The module-scope arm of the same refusal: a top-level `y: memoryview`.
#[test]
fn declaring_a_memoryview_at_module_scope_is_a_capability_gap() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "y".to_string(),
            annotation: Ty::MemoryView,
            value: None,
            is_final: false,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("declaring `y` as a buffer"),
        "{}",
        err.message
    );
}

// The guard sits *ahead* of each arm's value/no-value split, so the valued
// shape routes through the same contract -- without this the guard could be
// moved below the split and both value-less tests above would still pass,
// while `x: memoryview = <expr>` silently reported something else.
#[test]
fn declaring_a_memoryview_with_an_initializer_is_the_same_capability_gap() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "y".to_string(),
            annotation: Ty::MemoryView,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: false,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("declaring `y` as a buffer"),
        "{}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// Part 2 of #1027: `b[i]` on a `memoryview` parameter.
//
// The carve-out is an interception ahead of base inference, so each test
// below pins one half of it: the shapes it admits, and the shapes that must
// still reach `reject_memoryview_read`'s `C0001` unchanged. The surface it
// opens is exactly one expression, which is why the refusal cluster is the
// larger half.
// ---------------------------------------------------------------------------

/// Builds `def f(b: memoryview, i: int) -> <ret>: <body>`, the one shape a
/// `memoryview` can appear in, for the Part 2 cluster below.
fn memoryview_subject(return_ty: Ty, body: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![
                ("b".to_string(), Ty::MemoryView),
                ("i".to_string(), Ty::Int),
            ],
            return_ty,
            body,
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// `b[i]` is a `float`, and reaches that answer without the `C0001` the
/// bare name would get -- the interception runs before the base is inferred.
#[test]
fn indexing_a_memoryview_parameter_yields_a_float() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::Name("i".to_string())),
        }))],
    );
    assert!(check(&hir).is_ok());
}

/// D-086: `bool` is accepted wherever `int` is expected at an operand
/// boundary, and an index is exactly that boundary -- `b[True]` is ordinary
/// Python, not a type error. The same `is_assignable` reasoning the list
/// arm records.
#[test]
fn a_bool_index_into_a_memoryview_is_accepted() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::BoolLiteral(true)),
        }))],
    );
    assert!(check(&hir).is_ok());
}

/// A non-int-compatible index reuses `T0021`, the same operand-mismatch
/// code the list arm reuses. No new diagnostic code is introduced.
#[test]
fn a_non_int_index_into_a_memoryview_is_t0021() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::StringLiteral("k".to_string())),
        }))],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("`memoryview` index"),
        "{}",
        err.message
    );
    assert!(err.message.contains("`str`"), "{}", err.message);
}

/// The carve-out admits one level and no more: `b[i][j]` subscripts a
/// `float`, which is `T0033` -- "does not support indexing" -- and not a
/// second buffer read. Pinned because the interception keys on the *base
/// expression*, so a nested subscript must fall through to the ordinary
/// path.
#[test]
fn indexing_the_result_of_a_buffer_read_is_t0033() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Subscript {
                base: Box::new(HirExpr::Name("b".to_string())),
                index: Box::new(HirExpr::Name("i".to_string())),
            }),
            index: Box::new(HirExpr::Name("i".to_string())),
        }))],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("`float`"), "{}", err.message);
}

/// PEP 572 (#774), the walrus on the *value* side: `y := b[i]` binds a
/// `float`, so the buffer read has to answer before D-141's own walrus
/// value gate runs. Without the interception this is a `C0001`.
#[test]
fn a_walrus_over_a_buffer_read_binds_a_float() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::NamedExpr {
            name: "y".to_string(),
            value: Box::new(HirExpr::Subscript {
                base: Box::new(HirExpr::Name("b".to_string())),
                index: Box::new(HirExpr::Name("i".to_string())),
            }),
        }))],
    );
    assert!(check(&hir).is_ok());
}

/// The walrus on the *index* side: `b[(n := 0)]`. The interception still
/// infers the index, so the binding is recorded -- `pycc_mir`'s
/// `collect_named_expr_bindings` walks both sides of `BufferGet` for the
/// same reason, and a slot that is never allocated is a miscompile.
#[test]
fn a_walrus_inside_a_buffer_index_is_still_bound() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(0)),
            }),
        }))],
    );
    assert!(check(&hir).is_ok());
}

/// The monomorphizer walks every expression of every item when the module
/// holds a generic function, and its own `Subscript` arm recurses into the
/// base -- which would call `infer_expr_in` on the bare name and report the
/// `C0001` the carve-out exists to avoid. `is_memoryview_base` is the
/// second, separate guard that stops it.
///
/// `check_and_resolve` rather than `check`: the monomorphizer is a
/// *post-check* phase, reached from `check_and_resolve_all_keyed` alone, so a
/// `check`-only assertion here never executes the guard it names (found while
/// covering the same seam for `len(b)` in #1116).
#[test]
fn a_buffer_read_survives_a_module_that_holds_a_generic_function() {
    let mut hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::Name("i".to_string())),
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "ident".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    });
    assert!(check_and_resolve(&hir).is_ok());
}

/// `b.attr` in that same generic-bearing module stays refused: the
/// monomorphizer's `AttrGet` arm shares `is_class_name_base` with the
/// `Subscript` one, and widening that predicate instead of adding a second
/// would have silently opened this shape too.
#[test]
fn an_attribute_read_of_a_memoryview_stays_refused_beside_a_generic() {
    let mut hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::AttrGet {
            base: Box::new(HirExpr::Name("b".to_string())),
            attr: "nbytes".to_string(),
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "ident".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    });
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

/// And so does `b.tolist()`, the `MethodCall` half of that same pair.
#[test]
fn a_method_call_on_a_memoryview_stays_refused_beside_a_generic() {
    let mut hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("b".to_string())),
            method: "tolist".to_string(),
            args: Vec::new(),
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "ident".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    });
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

// ---------------------------------------------------------------------------
// #1116: `len(b)` on a `memoryview` parameter.
//
// The second interception, built the same way as `b[i]` above and pinned the
// same way: the shapes it admits, the shapes that must still reach
// `reject_memoryview_read`, and the three walkers it has to hold for at once
// (the check phase, the constraint solver, and the monomorphizer).
// ---------------------------------------------------------------------------

/// `len(b)` is an `int`, and reaches that answer without the `C0001` the
/// bare name would get -- the interception runs before the argument is
/// inferred. This test is the inversion of Part 2's own
/// `len_of_a_memoryview_is_still_a_capability_gap`, whose refusal #1116
/// closes.
#[test]
fn len_of_a_memoryview_parameter_yields_an_int() {
    let hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        }))],
    );
    assert!(check(&hir).is_ok());
}

/// The whole point of the issue: `len(b)` has to *compose*, not merely
/// type-check. A `range(len(b))` loop whose body indexes `b` is the sweep
/// shape `docs/TESTING.md`'s prerequisite 1 needs, and it exercises the
/// interception at a position where the result is consumed as an `int`
/// rather than immediately returned.
#[test]
fn len_of_a_memoryview_composes_as_a_range_bound() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![
            HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::FloatLiteral(0.0),
            },
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "len".to_string(),
                    args: vec![HirExpr::Name("b".to_string())],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::Assign {
                    target: "s".to_string(),
                    value: HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("s".to_string())),
                        right: Box::new(HirExpr::Subscript {
                            base: Box::new(HirExpr::Name("b".to_string())),
                            index: Box::new(HirExpr::Name("i".to_string())),
                        }),
                    },
                }],
            },
            HirStmt::Return(Some(HirExpr::Name("s".to_string()))),
        ],
    );
    assert!(check(&hir).is_ok());
}

/// The interception claims only the one-argument shape, so a mis-arity
/// `len(b, i)` is not silently answered `int`: it falls through to the
/// ordinary path, where the established diagnostic order (arguments are
/// inferred before arity is validated -- see `infer_expr_in`'s own comment
/// at the `arg_tys` construction) makes the bare-name read's `C0001` the
/// first refusal reached. The `T0033` arity error is what a *non*-
/// `memoryview` operand gets there; what this arm pins is that the guard
/// releases the shape at all rather than claiming every `len` whose first
/// argument is a buffer.
#[test]
fn a_two_argument_len_on_a_memoryview_is_still_refused() {
    let hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![
                HirExpr::Name("b".to_string()),
                HirExpr::Name("i".to_string()),
            ],
        }))],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001", "{}", err.message);
}

/// The interception keys on the *argument*, not on the callee alone: any
/// other builtin taking the bare name still reaches `C0001`. Without this
/// arm the `callee == "len"` half of the guard is never observed failing
/// against a `memoryview` argument.
#[test]
fn another_builtin_taking_a_memoryview_is_still_a_capability_gap() {
    let hir = memoryview_subject(
        Ty::None,
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        })],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("using `b`, which is bound to a buffer parameter"),
        "{}",
        err.message
    );
}

/// And it keys on the binding's *type*: `len(i)` on the `int` parameter of
/// the same subject falls through to the ordinary `len` block, which
/// refuses a non-container operand with `T0033`. This pins that the new
/// guard did not widen `len`'s accepted operand set beyond `memoryview`.
#[test]
fn len_of_a_non_memoryview_name_is_unaffected_by_the_new_guard() {
    let hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("i".to_string())],
        }))],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0033", "{}", err.message);
}

/// The monomorphizer is the third walker over the same expression, and its
/// `Call` arm recurses into every argument -- which reaches the bare name
/// and its `C0001` -- so it needs a guard of its own, exactly as its
/// `Subscript` arm already does.
///
/// `check_and_resolve` rather than `check`: the monomorphizer is a
/// *post*-check phase (`module.rs`'s `check_and_resolve_all_keyed` runs it
/// after signatures resolve), so `check` alone never reaches it and a
/// module that passes `pycc check` could still fail in `pycc build`. The
/// generic `ident` is what makes `monomorphize` walk the module at all
/// rather than taking its no-generics early return.
#[test]
fn a_buffer_length_read_survives_monomorphization_beside_a_generic() {
    let mut hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "ident".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    });
    assert!(check_and_resolve(&hir).is_ok());
}

/// The solver is the second walker, and it runs only when something in the
/// module forces it: any `Ty::Infer` signature routes `check` through
/// `infer_function_signatures_with_solver_all` first, which runs
/// `collect_expr_constraints` over every expression -- including this
/// `len(b)`, whose argument recursion would otherwise reach the solver's
/// own shared `Name` read seam and its `C0001`. The unannotated private
/// helper is the established way this file forces that path (see
/// `a_list_literal_still_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path`).
#[test]
fn len_of_a_memoryview_survives_the_solver_path() {
    let mut hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "_constant".to_string(),
        params: vec![],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
    });
    assert!(check(&hir).is_ok());
}

/// Aliasing stays refused: `c = b` would let the borrowed view outlive the
/// expression that produced it.
#[test]
fn aliasing_a_memoryview_into_a_local_is_still_a_capability_gap() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![
            HirStmt::Assign {
                target: "c".to_string(),
                value: HirExpr::Name("b".to_string()),
            },
            HirStmt::Return(Some(HirExpr::FloatLiteral(0.0))),
        ],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

/// Passing it onward stays refused.
#[test]
fn passing_a_memoryview_to_another_function_is_still_a_capability_gap() {
    let mut hir = memoryview_subject(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "g".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        }))],
    );
    hir.items.push(HirItem::Function {
        name: "g".to_string(),
        params: vec![("v".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
    });
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

/// Slicing stays refused: `b[0:1]` is a separate HIR node with no Part 2
/// lowering, and its base inference is the ordinary one.
#[test]
fn slicing_a_memoryview_is_still_a_capability_gap() {
    let hir = memoryview_subject(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Slice {
            base: Box::new(HirExpr::Name("b".to_string())),
            start: Some(Box::new(HirExpr::IntLiteral(0))),
            stop: Some(Box::new(HirExpr::IntLiteral(1))),
            step: None,
        }))],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

// ---------------------------------------------------------------------------
// Part 1 of #1142: the element *store* `b[i] = v`.
//
// The store arrives as `HirStmt::DictSet`, so `check_dict_set` dispatches on
// the target's resolved type before it asks for a `Ty::Dict`. Each test below
// pins one arm of that dispatch, and the refusal cluster above is unchanged:
// the store is the second admitted use of the name, not a widening of the
// read surface.
// ---------------------------------------------------------------------------

/// `b[i] = v` type-checks with an `int` index and a `float` value, without
/// the `C0001` the bare name still gets everywhere else.
#[test]
fn storing_a_float_into_a_memoryview_element_is_accepted() {
    let hir = memoryview_subject(
        Ty::None,
        vec![HirStmt::DictSet {
            dict: "b".to_string(),
            key: HirExpr::Name("i".to_string()),
            value: HirExpr::FloatLiteral(1.5),
        }],
    );
    assert!(check(&hir).is_ok());
}

/// D-086 again, on the index only: `b[True] = 1.0` is ordinary Python.
#[test]
fn a_bool_index_on_a_memoryview_store_is_accepted() {
    let hir = memoryview_subject(
        Ty::None,
        vec![HirStmt::DictSet {
            dict: "b".to_string(),
            key: HirExpr::BoolLiteral(true),
            value: HirExpr::FloatLiteral(1.5),
        }],
    );
    assert!(check(&hir).is_ok());
}

/// The index arm reuses the load's own `T0021` and its exact wording, so
/// `b["k"]` and `b["k"] = 1.0` read the same.
#[test]
fn a_non_int_index_on_a_memoryview_store_is_t0021() {
    let hir = memoryview_subject(
        Ty::None,
        vec![HirStmt::DictSet {
            dict: "b".to_string(),
            key: HirExpr::StringLiteral("k".to_string()),
            value: HirExpr::FloatLiteral(1.5),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("`memoryview` index must be `int`"),
        "{}",
        err.message
    );
    assert!(err.message.contains("`str`"), "{}", err.message);
}

/// The element type is `float` and D-086 grants no int-to-float widening,
/// so `b[i] = 1` is refused rather than silently converted -- which is also
/// what lets codegen see exactly one scalar shape at the value position.
#[test]
fn storing_a_non_float_into_a_memoryview_element_is_t0021() {
    for (value, rendered) in [
        (HirExpr::IntLiteral(1), "`int`"),
        (HirExpr::StringLiteral("v".to_string()), "`str`"),
    ] {
        let hir = memoryview_subject(
            Ty::None,
            vec![HirStmt::DictSet {
                dict: "b".to_string(),
                key: HirExpr::Name("i".to_string()),
                value,
            }],
        );
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(
            err.message.contains("to a `memoryview` element of `float`"),
            "{}",
            err.message
        );
        assert!(err.message.contains(rendered), "{}", err.message);
    }
}

/// The dispatch keys on the target's type, so every other target is
/// untouched: a `list[int]` still gets `T0033` from the same function, and
/// the store surface opened here is exactly one type wide.
#[test]
fn a_store_into_a_non_dict_non_buffer_target_is_still_t0033() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("xs".to_string(), Ty::Set(Box::new(Ty::Int)))],
            return_ty: Ty::None,
            body: vec![HirStmt::DictSet {
                dict: "xs".to_string(),
                key: HirExpr::IntLiteral(0),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0033");
}

/// A store through a name that is not bound at all is still the "not
/// defined" refusal, not a buffer arm: admitting the buffer at a store
/// target widens which *type* is accepted there, never which names are.
#[test]
fn a_store_into_an_undefined_name_is_unchanged() {
    let hir = memoryview_subject(
        Ty::None,
        vec![HirStmt::DictSet {
            dict: "nope".to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::FloatLiteral(1.0),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"), "{}", err.message);
}
