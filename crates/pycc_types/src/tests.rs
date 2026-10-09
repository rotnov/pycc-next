//! Unit tests for the type-checking crate root.
//!
//! Relocated verbatim from `lib.rs`'s inline `#[cfg(test)] mod tests` under
//! AGENTS.md's decomposability rule (#544 Part 1). As a child module it still
//! sees the crate root's private items directly, so nothing needed widened
//! visibility; only the module's own location changed.

use super::*;

// `lib.rs` used to import this for its own inference code; that code now
// lives in `expr.rs`, which imports it directly, so the crate root no longer
// carries the import these tests inherited through `use super::*`.
use crate::binop::numeric_result_type;

// Cohesive test clusters live in their own child files so this
// already-oversized module does not grow (AGENTS.md decomposability rule):
// #934 for the protocol-return re-cover tests, and #695 for the
// constraint-collection, enum-loop-unrolling, exception-handling,
// generic-class-instantiation, generic-class-substitution,
// generic-method-dedup, generic-method-instantiation,
// generic-monomorphization-arms, import-alias, `__init__`-rank,
// `Optional[T]`-narrowing, pattern-matching, protocol-argument,
// `typing.TYPE_CHECKING`-marker and `typing.cast` clusters.
mod boolop;
mod buffer_producer;
mod compare_chain;
mod comprehension_expr;
mod constraints;
mod constructor_call;
mod empty_container_registry;
mod enum_unrolling;
mod exception_handling;
mod foreign_float_tuple;
mod generic_class_instantiation;
mod generic_class_substitution;
mod generic_method_dedup;
mod generic_method_instantiation;
mod generic_monomorphization_arms;
mod if_exp;
mod import_alias;
mod init_rank;
mod isinstance_narrowing;
mod method_call_return;
mod not_implemented;
mod object_none_return;
mod optional_narrowing;
mod pattern_matching;
mod protocol_argument;
mod protocol_return;
mod try_definite;
mod type_checking_marker;
mod typing_cast;
mod typing_legacy_aliases;
mod while_true_return;

#[test]
fn v0_1_slice_always_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// #433: a bare `HirExpr::Super` should never reach `infer_expr_in` —
// HIR lowering rejects a standalone `super()` with C0001. This test
// bypasses HIR lowering with a hand-built HIR to exercise the
// defense-in-depth guard.
#[test]
fn bare_super_in_check_returns_c0001() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Super)],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
}

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

// PEP 572 (#774): `function_local_names`'s own `collect_named_expr_names_in_
// expr` walk records a walrus target as a function-local name wherever it is
// nested -- including inside a unary operand and a slice bound, which no
// other test in this suite reaches (every other walrus fixture is
// module-level, never inside a `HirItem::Function` body). Hand-built HIR,
// bypassing the parser, to pin the exact nesting shape.
#[test]
fn a_walrus_inside_a_unary_operand_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Compare {
                        op: CmpOpKind::Lt,
                        left: Box::new(HirExpr::UnaryOp {
                            op: UnaryOpKind::USub,
                            operand: Box::new(HirExpr::NamedExpr {
                                name: "n".to_string(),
                                value: Box::new(HirExpr::IntLiteral(3)),
                            }),
                        }),
                        right: Box::new(HirExpr::IntLiteral(0)),
                    },
                    body: vec![HirStmt::Return(Some(HirExpr::Name("n".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_slice_bound_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "g".to_string(),
            params: vec![("lst".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Slice {
                    base: Box::new(HirExpr::Name("lst".to_string())),
                    start: Some(Box::new(HirExpr::NamedExpr {
                        name: "m".to_string(),
                        value: Box::new(HirExpr::IntLiteral(0)),
                    })),
                    stop: None,
                    step: None,
                }),
                HirStmt::Return(Some(HirExpr::Name("m".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn concrete_signatures_take_the_validation_only_fast_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "identity".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(
        concrete_function_signatures(&hir).unwrap()["identity"],
        (vec![Ty::Int], Ty::Int)
    );
    assert!(check(&hir).is_ok());
}

#[test]
fn inferred_signatures_keep_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(concrete_function_signatures(&hir).is_none());
}

#[test]
fn concrete_fast_path_preserves_solver_first_diagnostic_selection() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "takes_int".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "broken".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "takes_int".to_string(),
                        args: vec![HirExpr::StringLiteral("wrong".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::StringLiteral("wrong".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let concrete = concrete_function_signatures(&hir).unwrap();
    let validation_first = check_with_signatures(&hir, &concrete, &local_names).unwrap_err();
    let solver_first = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    let fast_path = checked_function_signatures(&hir, &local_names).unwrap_err();
    let public_check = check(&hir).unwrap_err();

    assert_eq!(validation_first.code, "T0021");
    assert_eq!(solver_first.code, "T0022");
    assert_eq!(fast_path.code, solver_first.code);
    assert_eq!(fast_path.message, solver_first.message);
    assert_eq!(public_check.code, solver_first.code);
    assert_eq!(public_check.message, solver_first.message);
    // #868 (D-220): the per-function list keeps the same selection at index
    // 0, and `broken` is reported exactly once (both phases flag it; the
    // solver's entry wins by key, the checker's `Some(1)` entry is dropped).
    let all = check_all(&hir).unwrap_err();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, solver_first.code);
    assert_eq!(all[0].message, solver_first.message);
    let fast_path_all =
        crate::module::checked_function_signatures_all(&hir, &local_names).unwrap_err();
    assert_eq!(fast_path_all.len(), 1);
    assert_eq!(fast_path_all[0].1.message, solver_first.message);
}

#[test]
fn a_list_literal_still_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path()
 {
    // Any `Ty::Infer` signature anywhere in the module routes `check`
    // through `infer_function_signatures_with_solver_all` first (see
    // `check_all`), which runs `collect_expr_constraints`
    // over every expression in the module, including this list literal.
    // That solver-side pass must stay lenient (`Ok(None)`, recurse only)
    // for list forms -- confirms it doesn't wrongly reject a valid list.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_heterogeneous_list_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The load-bearing counterpart to the test above: the solver's
    // leniency must not swallow a genuine list error -- it has to fall
    // through to the real, list-aware check pass (`check_with_signatures_all`)
    // that runs after the solver, which is what actually raises T0032.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::StringLiteral("two".to_string()),
                ]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0032");
}

#[test]
fn a_for_list_loop_still_type_checks_correctly_when_the_solver_path_runs_first() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![],
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// PR-12 Task 7 (D-118): whole-module `check()` tests for `HirExpr::Slice`,
// exercising the real statement walkers (`check_with_environment_all`'s
// `check_stmt`, and `collect_block_constraints`'s `Assign` arm under the
// solver path) rather than calling `infer_expr`/`collect_expr_constraints`
// directly on a hand-built expression. Mirrors the list-literal pair
// immediately above: Task 3's own ledger records a real regression of
// exactly this shape (a solver arm that looked correct in isolation but
// was never actually reached by the block walker) -- these confirm the
// `Slice` arm added in this task is genuinely wired into both paths, not
// just correct when invoked directly.

#[test]
fn slicing_type_checks_through_the_full_check_pipeline_on_the_fast_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: None,
                    step: None,
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn slicing_a_list_of_str_is_rejected_through_the_full_check_pipeline() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0034");
}

#[test]
fn slicing_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    // Companion to `a_list_literal_still_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path`
    // above: proves `collect_block_constraints`'s ordinary `Assign` arm
    // reaches this task's new `HirExpr::Slice` constraint-collection arm
    // (which must stay lenient, `Ok(None)`, recursing only) without
    // wrongly rejecting a valid slice, and that the real check pass
    // (`check_with_signatures_all`) run afterward still type-checks it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: Some(Box::new(HirExpr::IntLiteral(3))),
                    step: None,
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn slicing_a_list_of_str_is_still_rejected_when_the_solver_path_runs_first() {
    // Load-bearing counterpart: the solver's leniency for `Slice` must
    // not swallow a genuine `T0034` -- it has to fall through to the
    // real, list-aware check pass that runs after the solver.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0034");
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_empty_slice() {
    assert_eq!(homogeneous_private_solver_scalar_list_element(&[]), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_none_element_term() {
    let terms = vec![Some(Ok(Ty::Int)), None];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_none_first_element() {
    // Covers the `.as_ref()?` None path when the first element itself is
    // `None` (distinct from the test above where `None` is at index 1).
    let terms = vec![None, Some(Ok(Ty::Int))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_err_element_term() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Err(0))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_an_err_first_element() {
    // Covers the `.ok()?` None path when the first element is `Some(Err(_))`
    // (distinct from the test above where `Err` is at index 1).
    let terms = vec![Some(Err(0)), Some(Ok(Ty::Int))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_a_non_scalar_first_element() {
    let terms = vec![Some(Ok(Ty::List(Box::new(Ty::Int))))];
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_rejects_heterogeneous_scalars() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Ok(Ty::Bool))];
    // Exact `Ty` equality, NOT `merge_inferred_types` -- `bool` and `int`
    // are not merged even though `merge_inferred_types` would widen them.
    assert_eq!(homogeneous_private_solver_scalar_list_element(&terms), None);
}

#[test]
fn homogeneous_private_solver_scalar_list_element_accepts_homogeneous_scalars() {
    let terms = vec![Some(Ok(Ty::Int)), Some(Ok(Ty::Int))];
    assert_eq!(
        homogeneous_private_solver_scalar_list_element(&terms),
        Some(Ty::Int)
    );
}

#[test]
fn homogeneous_private_solver_scalar_list_element_accepts_a_single_scalar() {
    let terms = vec![Some(Ok(Ty::Str))];
    assert_eq!(
        homogeneous_private_solver_scalar_list_element(&terms),
        Some(Ty::Str)
    );
}

#[test]
fn a_private_helper_can_use_an_annotated_assignment_during_signature_inference() {
    // End-to-end: an annotated assignment inside a private helper whose
    // own signature is still `Ty::Infer` exercises the solver path
    // (`collect_block_constraints`) as well as the later concrete
    // re-check (`check_stmt_in_function`), not just the direct unit
    // tests above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn an_annotated_private_local_preserves_known_bool_initializer_evidence() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::BoolLiteral(true)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    check(&hir).unwrap();
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Int
    ));
}

#[test]
fn an_annotated_private_local_does_not_retag_the_returned_initializer() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_echo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_echo".to_string(),
                args: vec![HirExpr::BoolLiteral(true)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Bool
    ));
}

#[test]
fn an_annotated_private_local_supplies_a_fallback_without_call_site_evidence() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::Name("x".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Int && *return_ty == Ty::Int
    ));
}

#[test]
fn a_container_annotation_cannot_escape_the_scalar_only_private_solver() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::List(Box::new(Ty::Int)),
                    value: Some(HirExpr::Name("x".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer type of parameter `x`"));
}

#[test]
fn a_container_annotated_target_cannot_resolve_an_inferred_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::List(Box::new(Ty::Int)),
                    value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot infer return type of private helper `_helper`")
    );
}

#[test]
fn hard_call_evidence_cannot_turn_a_container_local_into_an_inferred_signature() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("y".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot infer return type of private helper `_helper`")
    );
}

#[test]
fn an_unrelated_container_local_does_not_block_scalar_signature_inference() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_helper".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Int && *return_ty == Ty::Int
    ));
}

#[test]
fn scalar_hard_evidence_on_a_container_local_reaches_final_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::List(Box::new(Ty::Int)),
                        value: Some(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("y".to_string())],
                    }),
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("argument 1 of `_sink` expects `int`, got `list[int]`")
    );
}

#[test]
fn an_annotated_private_local_fallback_propagates_through_a_binary_expression() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("x".to_string())),
                        right: Box::new(HirExpr::IntLiteral(1)),
                    }),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, .. } if params[0].1 == Ty::Int
    ));
}

#[test]
fn an_annotated_private_local_fallback_rechecks_binary_operand_conflicts() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Int,
                    value: Some(HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::Name("x".to_string())),
                        right: Box::new(HirExpr::StringLiteral("bad".to_string())),
                    }),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("right operand of int binary expression")
    );
}

#[test]
fn later_nested_call_evidence_wins_over_an_earlier_annotation_fallback() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_echo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::Function {
                name: "_caller".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_echo".to_string(),
                        args: vec![HirExpr::BoolLiteral(true)],
                    }),
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params[0].1 == Ty::Bool && *return_ty == Ty::Bool
    ));
}

#[test]
fn multiple_annotation_bounds_choose_bool_in_either_declaration_order() {
    for bool_first in [false, true] {
        let int_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "a".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let bool_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "b".to_string(),
            annotation: Ty::Bool,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let assignments = if bool_first {
            vec![bool_assignment, int_assignment]
        } else {
            vec![int_assignment, bool_assignment]
        };
        let mut body = assignments;
        body.push(HirStmt::Return(Some(HirExpr::Name("b".to_string()))));
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body,
            }],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };

        let resolved = check_and_resolve(&hir).unwrap();
        assert!(matches!(
            &resolved.items[0],
            HirItem::Function { params, .. } if params[0].1 == Ty::Bool
        ));
    }
}

#[test]
fn incompatible_annotation_bounds_fail_identically_in_either_declaration_order() {
    let mut messages = Vec::new();
    for string_first in [false, true] {
        let int_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "a".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let string_assignment = HirStmt::AnnAssign {
            is_final: false,
            target: "b".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::Name("x".to_string())),
        };
        let body = if string_first {
            vec![string_assignment, int_assignment]
        } else {
            vec![int_assignment, string_assignment]
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::Function {
                name: "_helper".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body,
            }],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };

        let err = check_and_resolve(&hir).unwrap_err();
        assert_eq!(err.code, "T0021");
        messages.push(err.message);
    }
    assert_eq!(messages[0], messages[1]);
}

#[test]
fn annotated_private_local_rejects_known_int_to_bool_narrowing_with_t0025() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_narrow".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Bool,
                        value: Some(HirExpr::Name("x".to_string())),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_narrow".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0025");
}

#[test]
fn annotated_private_local_binds_when_a_container_subscript_has_no_solver_term() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::Function {
                name: "_first".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Int,
                        value: Some(HirExpr::Subscript {
                            base: Box::new(HirExpr::Name("xs".to_string())),
                            index: Box::new(HirExpr::IntLiteral(0)),
                        }),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[1],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Int
    ));
}

#[test]
fn no_term_initializer_still_reaches_directional_final_validation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::Function {
                name: "_first".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "y".to_string(),
                        annotation: Ty::Bool,
                        value: Some(HirExpr::Subscript {
                            base: Box::new(HirExpr::Name("xs".to_string())),
                            index: Box::new(HirExpr::IntLiteral(0)),
                        }),
                    },
                    HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0025");
}

#[test]
fn a_cloned_environment_keeps_later_function_bindings_isolated() {
    let mut original = Environment::new();
    original.bind_function("original".to_string(), vec![Ty::Int], Ty::Int);

    let mut cloned = original.clone();
    cloned.bind_function("cloned".to_string(), vec![], Ty::None);

    assert!(original.lookup_function("cloned").is_none());
    assert_eq!(
        cloned.lookup_function("cloned"),
        Some(&(Vec::new(), Ty::None))
    );
    assert_eq!(
        original.lookup_function("original"),
        Some(&(vec![Ty::Int], Ty::Int))
    );
}

#[test]
fn infers_an_int_literal_as_int() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::IntLiteral(1)), Ok(Ty::Int));
}

#[test]
fn infers_a_float_literal_as_float() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::FloatLiteral(1.5)), Ok(Ty::Float));
}

#[test]
fn infers_a_bool_literal_as_bool() {
    let env = Environment::new();
    assert_eq!(infer_expr(&env, &HirExpr::BoolLiteral(true)), Ok(Ty::Bool));
}

#[test]
fn infers_a_string_literal_as_str() {
    let env = Environment::new();
    assert_eq!(
        infer_expr(&env, &HirExpr::StringLiteral("hi".to_string())),
        Ok(Ty::Str)
    );
}

#[test]
fn adding_an_int_and_a_str_is_a_clean_type_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::StringLiteral("x".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn adding_two_strings_infers_str() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::StringLiteral("b".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn subtracting_two_strings_is_a_clean_type_error() {
    // Python allows `"a" + "b"` but no other arithmetic operator between
    // two strings -- `"a" - "b"` is a `TypeError` at runtime in CPython.
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Sub,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::StringLiteral("b".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_two_strings_infers_bool() {
    // `"a" == "b"`, `"a" < "b"`, etc. are ordinary, valid Python
    // (lexicographic ordering) -- not covered by numeric_or_bool_compatible
    // before `Ty::Str` became constructible via literals.
    let env = Environment::new();
    for op in [
        CmpOpKind::Eq,
        CmpOpKind::NotEq,
        CmpOpKind::Lt,
        CmpOpKind::LtE,
        CmpOpKind::Gt,
        CmpOpKind::GtE,
    ] {
        let expr = HirExpr::Compare {
            op,
            left: Box::new(HirExpr::StringLiteral("a".to_string())),
            right: Box::new(HirExpr::StringLiteral("b".to_string())),
        };
        assert_eq!(
            infer_expr(&env, &expr),
            Ok(Ty::Bool),
            "comparison {op:?} should type-check"
        );
    }
}

#[test]
fn an_f_string_always_infers_str_regardless_of_interpolated_types() {
    let env = Environment::new();
    let expr = HirExpr::FString(vec![
        FStringPart::Literal("n=".to_string()),
        FStringPart::Interpolation(Box::new(HirExpr::IntLiteral(1))),
    ]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn an_f_string_still_type_checks_its_interpolated_expressions() {
    let env = Environment::new();
    let expr = HirExpr::FString(vec![FStringPart::Interpolation(Box::new(HirExpr::Name(
        "undefined".to_string(),
    )))]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_a_string_and_an_int_is_a_clean_type_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::StringLiteral("a".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_two_ints_infers_bool() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Lt,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn comparing_a_bool_and_an_int_succeeds_since_bool_is_a_subtype_of_int() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn comparing_an_undefined_left_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::Name("undefined".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_an_undefined_right_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn comparing_incompatible_types_is_a_clean_type_error() {
    let mut env = Environment::new();
    // A call to a properly declared, zero-arg, `None`-returning function
    // legitimately infers `Ty::None`, which isn't numeric-like --
    // comparing an int against it is a genuine, both-sides-defined
    // incompatibility.
    env.bind_function("f".to_string(), vec![], Ty::None);
    let expr = HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Call {
            callee: "f".to_string(),
            args: vec![],
        }),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("int") && err.message.contains("None"));
}

#[test]
fn a_binop_treats_bool_as_int() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::BoolLiteral(true)),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_binop_treats_bool_and_float_as_float() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::BoolLiteral(true)),
        right: Box::new(HirExpr::FloatLiteral(1.5)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn a_top_level_return_is_a_clean_diagnostic_not_a_panic() {
    // Regression test (self-review finding, pre-merge): this used to be
    // `panic!(...)`, so a bare `return` at module scope crashed the
    // compiler (exit code 101) instead of producing a diagnostic through
    // the documented exit-1 contract every other error path uses.
    // `ruff_python_parser` does not reject `return` outside a function at
    // the grammar level (CPython itself only rejects it in a later
    // compile pass), so this is reachable from ordinary CLI input.
    let mut env = Environment::new();
    let err = check_stmt(&mut env, &HirStmt::Return(None)).unwrap_err();
    assert_eq!(err.code, "T0024");
}

#[test]
fn check_and_resolve_also_rejects_a_top_level_return_with_t0024() {
    // Regression test: `collect_block_constraints` (the private-helper
    // solver) is invoked over top-level statements with `return_term:
    // None` (no enclosing function), so a top-level `Return` hits its
    // own defensive `let Some(return_term) = return_term else {
    // continue }` arm and is silently skipped by the solver -- it's
    // `check_and_resolve`'s later, ordinary `check_stmt` pass (Pass 2)
    // that actually rejects it with T0024, exactly like the
    // `pycc_types::check` entry point already does.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Return(None))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0024");
}

#[test]
fn a_return_nested_in_a_top_level_if_is_also_a_clean_diagnostic() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Return(None)],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0024");
}

#[test]
fn an_assignment_binds_the_inferred_type_in_the_environment() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_assignment_whose_value_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Name("undefined".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_incompatible_reassignment_is_t0023_and_preserves_the_inferred_type() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("changed".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_annotated_assignment_with_a_matching_value_binds_the_annotation_type() {
    // x: int = True -- bool is assignable to int (matches is_assignable's
    // existing widening rule), and the environment should record Ty::Int
    // (the annotation), not Ty::Bool (the initializer's own inferred type).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::BoolLiteral(true)),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn an_annotated_assignment_with_a_mismatched_value_is_t0025() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::StringLiteral("nope".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0025");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotated_assignment_propagates_an_error_from_the_initializer_expression() {
    // The initializer itself can fail to type-check (here, referencing an
    // undefined name) before `is_assignable` is ever consulted -- the `?`
    // on `infer_expr` inside the new arm must propagate that error rather
    // than being reachable only via the is_assignable mismatch path.
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("undefined".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotation_only_declaration_does_not_bind_a_value_at_module_scope() {
    // x: int alone must only declare x, not bind it -- collect_local_names
    // registers `x` as local independently of this arm, so a premature
    // read still falls through to the existing T0021 mechanism (see the
    // function-scope regression test below for the end-to-end proof).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_then_matching_assignment_binds_the_declared_type() {
    // x: int; x = 1 -- issue #245: the value-less declaration must be
    // retained and honored by the later plain assignment.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn value_less_declaration_then_mismatched_assignment_is_t0026() {
    // x: int; x = "hello" -- issue #245's primary reproduction: this must
    // be rejected, and rejected with T0026 (not T0023 -- nothing was
    // "previously inferred", it was declared, never assigned).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_still_raises_t0021_on_a_premature_read_at_function_scope() {
    // x: int; return x (no assignment in between) -- the declared type
    // must never satisfy `lookup`, so this stays T0021, exactly as an
    // ordinary local without any annotation would.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "x".to_string(),
                        annotation: Ty::Int,
                        value: None,
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn repeated_compatible_value_less_declaration_then_assignment_is_accepted() {
    // x: int; x: int; x = 1 -- a second declaration compatible with the
    // first is accepted (first-declaration-wins), and the later plain
    // assignment still binds the declared type.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn repeated_incompatible_value_less_declaration_is_t0026() {
    // x: int; x: str -- flatly incompatible re-declarations are rejected
    // at the second declaration, before any assignment is even seen.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn valued_redeclaration_matching_an_earlier_value_less_declaration_keeps_the_declared_type() {
    // x: int; x: bool = True -- the *earlier* declaration wins as the
    // sticky representation (Int), since bool is merely assignable to
    // int, not the later annotation (matches check_assignment's own
    // worked bool/int example in its declared-consult branch).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Bool,
            value: Some(HirExpr::BoolLiteral(true)),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn valued_redeclaration_disagreeing_with_an_earlier_value_less_declaration_is_t0026() {
    // x: int; x: str = "hello" -- T0025 passes (the initializer matches
    // its own `str` annotation), so this must be rejected by the
    // declared-consult step with T0026, not silently accepted.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("hello".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn value_less_declaration_after_an_existing_binding_is_a_no_op() {
    // x = 1; x: int -- re-declaring an already-bound name is unchanged,
    // pre-existing, out-of-scope behavior: `declare` must not shadow the
    // real binding with an inert `declared` entry.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_value_less_declaration_then_matching_assignment_binds_the_declared_type() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_value_less_declaration_then_mismatched_assignment_is_t0026() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn function_scope_repeated_incompatible_value_less_declaration_is_t0026() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: None,
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0026");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn child_for_function_clears_a_declared_entry_for_a_shadowed_local_name() {
    // A module-level value-less declaration for `x` must not leak into
    // an unrelated function's own local `x` -- mirrors the existing
    // `bindings`-clearing behavior `child_for_function` already provides.
    let mut module_env = Environment::new();
    check_stmt(
        &mut module_env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    )
    .unwrap();
    let mut fn_env = module_env.child_for_function(&["x"]);
    // The function's own body assigns a `str` to its local `x` -- if the
    // module-level `declared` entry leaked through, this would wrongly
    // fail with T0026 against the stale `Int` declaration.
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(fn_env.lookup("x"), Some(Ty::Str));
}

#[test]
fn function_scope_annotated_assignment_with_a_matching_value_binds_the_annotation_type() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::BoolLiteral(true)),
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_annotated_assignment_with_a_mismatched_value_is_t0025() {
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::StringLiteral("nope".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0025");
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn function_scope_annotated_assignment_propagates_an_error_from_the_initializer_expression() {
    // Same as the module-scope sibling above, but through
    // `infer_expr_in`'s local-vs-global unbound distinction: `x` is a
    // declared local that was never assigned, so referencing it in `y`'s
    // initializer must propagate T0021 via the new arm's `?` rather than
    // silently falling through to the is_assignable check.
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "y".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::Name("x".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(env.lookup("y"), None);
}

#[test]
fn function_scope_annotation_only_declaration_does_not_bind_a_value() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
        Ty::None,
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
}

#[test]
fn an_annotation_only_declaration_does_not_bind_a_value() {
    // x: int alone must not make a later use of x succeed -- it only
    // declares x local, it does not bind it (matching CPython's own
    // UnboundLocalError for this exact shape, verified during planning;
    // this end-to-end module mirrors
    // tests/diagnostics/d0026_annotation_only_unbound.py exactly).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::AnnAssign {
                        is_final: false,
                        target: "x".to_string(),
                        annotation: Ty::Int,
                        value: None,
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "print".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    }),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    // Pin the exact message, not just the shared "T0021" code: this test's
    // entire purpose is proving the value-less form reaches the
    // *local-unbound* T0021 path (`unbound_local`), not the differently
    // worded *undefined-name* T0021 path -- both share the same code, so
    // only the message distinguishes them.
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn an_annotated_re_declaration_that_conflicts_with_an_existing_binding_is_t0023() {
    // `x`'s representation is fixed by its first assignment (Int) --
    // `pycc_mir`'s own `bind_variable` keeps that first type regardless of
    // a later compatible reassignment, so the checker must reject a later
    // annotated re-declaration that would otherwise silently repoint `x`
    // at an incompatible representation (here, `str`) via `is_assignable`
    // trivially comparing the initializer to its own annotation alone.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("s".to_string())),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn function_scope_annotated_re_declaration_that_conflicts_with_an_existing_binding_is_t0023() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("s".to_string())),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn assigning_bool_to_an_int_binding_keeps_the_declared_representation() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BoolLiteral(true),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_for_target_cannot_change_an_existing_binding_representation() {
    let mut env = Environment::new();
    env.bind("value".to_string(), Ty::Str);
    let err = check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "value".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(env.lookup("value"), Some(Ty::Str));
}

#[test]
fn a_for_target_cannot_change_a_parameter_representation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "loop_over".to_string(),
            params: vec![("value".to_string(), Ty::Str)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0023");
}

#[test]
fn direct_function_check_rejects_a_for_target_representation_change() {
    let function = HirItem::Function {
        name: "loop_over".to_string(),
        params: vec![("value".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "value".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_private_for_target_infers_an_unannotated_parameter_as_int() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![HirStmt::ForRange {
                var: "value".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }],
        }
    );
}

#[test]
fn an_if_s_test_must_be_bool_like_and_both_branches_are_checked() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: each branch is checked in an independent clone
    // of env, then joined. `x` is bound only in the body, `y` only in
    // orelse -- both are `Maybe` after the join, so `lookup` (which
    // returns `Some` only for `Definitely`) returns `None` for both.
    // Use `lookup_any` to verify the types are retained.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn an_if_with_both_branches_binding_the_same_name_joins_to_definitely() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Both branches bind `x` as `int` -> join is `Definitely`.
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_no_else_if_makes_body_bindings_maybe() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // No else -> body-only binding is `Maybe`.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_nested_no_else_if_makes_bindings_maybe() {
    // `if a: if b: x = 1` -- x is Maybe after the outer if (no else on
    // either if).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_else_with_nested_if_in_orelse_joins_to_maybe() {
    // `if a: x = 1 else: if b: x = 2` -- x is Maybe (orelse's if has no
    // else, so x is maybe in orelse; join of definite and maybe = maybe).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_body_and_definite_in_orelse_joins_to_maybe() {
    // `if a: if b: x = 1 else: x = 2` -- x is Maybe (body's inner if has
    // no else, so x is maybe in body; join of maybe and definite = maybe).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_both_branches_joins_to_maybe() {
    // `if a: if b: x = 1 else: if c: x = 2` -- x is Maybe in both
    // branches (each branch's inner if has no else, so x is Maybe in
    // each). The join of Maybe and Maybe is Maybe. Exercises the
    // (BindingState::Maybe(_), Some(BindingState::Maybe(_))) arm.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_maybe_in_orelse_only_joins_to_maybe() {
    // `if a: pass else: if b: x = 1` -- x is Maybe (orelse's inner if
    // has no else, so x is maybe in orelse; body doesn't bind x at all).
    // This exercises join_if_branches' pass 2 with a Maybe binding.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_a_while_inside_that_assigns_takes_the_clone_path() {
    // `if a: while b: x = 1` -- the while inside the if body introduces
    // bindings, so the fast path is NOT taken and the clone+join path
    // runs. Exercises introduces_bindings' While arm.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_if_with_a_for_list_inside_that_assigns_takes_the_clone_path() {
    // `if a: for x in items: y = 1` -- the for-list inside the if body
    // introduces bindings, so the fast path is NOT taken. Exercises
    // introduces_bindings' ForList arm.
    let mut env = Environment::new();
    env.bind("items".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ForList {
            var: "x".to_string(),
            list: "items".to_string(),
            body: vec![HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        orelse: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
}

#[test]
fn an_elif_chain_without_else_joins_to_maybe() {
    // `if a: x = 1 elif b: x = 2` (lowered to nested If in orelse) --
    // x is Maybe (inner if has no else).
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn an_elif_chain_with_else_joins_to_definitely() {
    // `if a: x = 1 elif b: x = 2 else: x = 3` -- all paths bind x.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(3),
            }],
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_name_in_if_stays_definitely() {
    // `x = 1; if cond: x = 2` -- x was definite before, stays definite.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_type_mismatch_at_if_join_is_t0023() {
    // `if cond: x = 1 else: x = "hello"` -- incompatible types at join.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::StringLiteral("hello".to_string()),
        }],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_maybe_bound_callee_is_t0041() {
    // `if cond: f = 1` then `f()` -- f is maybe-bound, not callable.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "f".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::ExprStmt(HirExpr::Call {
            callee: "f".to_string(),
            args: vec![],
        }),
    )
    .unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn an_if_whose_test_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::Name("undefined".to_string()),
        body: vec![],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_whose_orelse_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_whose_body_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken) AND an error.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
        ],
        orelse: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_whose_orelse_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken), orelse has error.
    let mut env = Environment::new();
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_with_assignments_whose_body_has_an_error_propagates_via_clone_path() {
    // Body has an assignment (so fast path is NOT taken) AND an error.
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
        ],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_loop_s_test_and_body_are_checked() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop body may execute zero times, so
    // body-only binding `x` is `Maybe` after the loop.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_while_loop_body_then_read_is_t0041() {
    // `while True: x = 1` then `x` read -- T0041 (the loop body may
    // execute zero times, so `x` is maybe-bound).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
        },
    )
    .unwrap();
    let err = check_stmt(&mut env, &HirStmt::ExprStmt(HirExpr::Name("x".to_string()))).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_pre_bound_variable_survives_a_while_loop_as_definite() {
    // `x = 1; while True: x = 2` -- `x` was definitely bound before the
    // loop, so it stays `Definitely` after the loop.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn a_while_loop_with_nested_if_body_joins_to_maybe() {
    // `while True: if True: x = 1` -- x is Maybe after the loop (the
    // inner if has no else, so x is Maybe in the body; join_loop_body
    // downgrades it to Maybe). Exercises the Maybe alternative in
    // join_loop_body's inner match.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                }],
                orelse: vec![],
            }],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_for_range_var_read_after_loop_is_t0041() {
    // `for i in range(3): pass` then `i` read -- T0041 (the loop variable
    // is maybe-bound after the loop).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    let err = check_stmt(&mut env, &HirStmt::ExprStmt(HirExpr::Name("i".to_string()))).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_pre_bound_for_range_var_survives_as_definite() {
    // `i = 0; for i in range(3): pass` then `i` read -- succeeds because
    // `i` was definitely bound before the loop.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::IntLiteral(0),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("i"), Some(Ty::Int));
}

#[test]
fn a_while_loop_whose_test_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::Name("undefined".to_string()),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_while_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_binds_its_variable_as_int_and_checks_its_body() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop may execute zero times, so both the
    // loop variable `i` and body-only binding `x` are `Maybe`.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_name_survives_a_for_range_loop_as_definitely() {
    // `x = 1; for i in range(3): pass` -- x stays Definitely, i is Maybe.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
}

#[test]
fn a_for_range_loop_whose_start_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::Name("undefined".to_string()),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_stop_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::Name("undefined".to_string()),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_step_is_undefined_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::Name("undefined".to_string()),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_loop_rejects_a_non_int_operand() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::StringLiteral("three".to_string()),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range stop"));
    assert_eq!(env.lookup("i"), None);
}

#[test]
fn a_for_range_loop_accepts_bool_as_an_int_subtype() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::BoolLiteral(false),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::BoolLiteral(true),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_binds_its_variable_as_the_list_element_type_and_checks_its_body() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: the loop may execute zero times, so both the
    // loop variable `i` and body-only binding `y` are `Maybe`.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_binds_its_variable_as_str_for_a_list_of_str() {
    // Proves the `ForList` inference isn't hardcoded to `Ty::Int` --
    // it's generic over whatever scalar element type the list actually
    // has (see the `HirExpr::Subscript`/`ListAppend` genericity notes).
    // Bound directly rather than via a `str` list literal: T0034 gates
    // every non-`int` list literal at creation time, so no source-level
    // program can ever produce a live `Ty::List(Ty::Str)` binding --
    // this is the only way to reach this arm with one.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Str)));
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Str));
}

#[test]
fn a_for_dict_loop_binds_its_variable_as_the_key_type() {
    // PR-11 Task 3 (D-123): `for k in d:` iterates a dict's own keys, so
    // `var` binds as the dict's key type (`Ty::Str`), not its value type.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::ForList {
        var: "k".to_string(),
        list: "d".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("k"), None);
    assert_eq!(env.lookup_any("k"), Some(Ty::Str));
}

#[test]
fn a_for_set_loop_binds_its_variable_as_the_element_type() {
    // PR-11 Task 7 (D-123): `for x in s:` iterates a set's own elements,
    // so `var` binds as the set's element type (`Ty::Int` for
    // `set[int]`).
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "x".to_string(),
        list: "s".to_string(),
        body: vec![],
    };
    check_stmt(&mut env, &stmt).unwrap();
    // Issue #118 Part 1: loop variable is Maybe after the loop.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
}

#[test]
fn a_pre_bound_for_list_var_survives_as_definite() {
    // `i = 0; xs = [1, 2, 3]; for i in xs: pass` then `i` read --
    // succeeds because `i` was definitely bound before the loop
    // (issue #118 Part 1).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::IntLiteral(0),
        },
    )
    .unwrap();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    check_stmt(
        &mut env,
        &HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        },
    )
    .unwrap();
    assert_eq!(env.lookup("i"), Some(Ty::Int));
}

#[test]
fn a_for_list_loop_over_an_undefined_list_is_rejected() {
    let mut env = Environment::new();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "undefined".to_string(),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn a_for_list_loop_over_a_non_list_value_is_rejected() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(5),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "x".to_string(),
        body: vec![],
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_for_list_loop_whose_body_statement_is_ill_typed_propagates_the_error() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_rejects_a_conflicting_reassignment_of_its_variable() {
    // `i` is already bound to `str` before the loop -- `check_assignment`
    // itself (not the list/element-type logic above it) is what rejects
    // rebinding it to the list's `int` element type.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "i".to_string(),
            value: HirExpr::StringLiteral("not an int".to_string()),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
    )
    .unwrap();
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![],
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

// -- PR-12 Task 3 (D-117): comprehension type-checking, module scope --

#[test]
fn a_list_comprehension_over_range_type_checks_and_binds_target_as_list_int() {
    // Also pins that `var` is bound (via `check_assignment`) *before*
    // `elt` is checked: `elt` references the loop variable, so this
    // would fail as an unbound local if the ordering were wrong.
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_11_i"), Some(Ty::Int));
}

#[test]
fn check_accepts_a_module_level_comprehension_whose_target_is_read_afterward() {
    // End-to-end pipeline pin, distinct from the hand-held-`Environment`
    // test above: every other comprehension test in this file either
    // drives `check_stmt`/`check_function` directly against a throwaway
    // `Environment`, or runs the full `check`/`check_and_resolve`
    // pipeline over a module expected to *fail*. This is the one test
    // confirming the full `check` pipeline (`module_function_names` ->
    // `concrete_function_environment` -> `check_with_environment_all` ->
    // `check_stmt`) accepts a *valid* module-level comprehension and
    // that its `target` binding genuinely survives into the next
    // top-level statement's environment, exactly like an ordinary
    // `Assign`'s binding already does.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "0comp_11_i".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "len".to_string(),
                args: vec![HirExpr::Name("xs".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_list_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    // Mirrors `If`/`While`: any type is accepted as truthy, no static
    // bool-convertibility restriction.
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn a_list_comprehension_producing_str_is_rejected_as_t0034() {
    // Mirrors `ListLiteral`'s own existing genericity tests
    // (`a_homogeneous_non_int_list_literal_is_rejected_as_t0034`).
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::StringLiteral("x".to_string())),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert!(err.message.contains("list[str]"));
}

#[test]
fn a_list_comprehension_producing_float_is_rejected_as_t0034() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::FloatLiteral(1.5)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert!(err.message.contains("list[float]"));
}

#[test]
fn a_list_comprehension_producing_bool_is_rejected_as_t0034() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::BoolLiteral(true)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert!(err.message.contains("list[bool]"));
}

#[test]
fn a_list_comprehension_over_a_bare_list_name_type_checks_and_binds_target_as_list_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::List` (the `ListCompAssign` tests above all use
    // `CompIter::Range`, never a bare-name list iterable).
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_20_x".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::List(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_20_x"), Some(Ty::Int));
}

#[test]
fn a_set_comprehension_over_a_bare_set_name_type_checks_and_binds_target_as_set_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Set`.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_20_x".to_string(),
        iter: CompIter::Name("s".to_string()),
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Set(Box::new(Ty::Int))));
    assert_eq!(env.lookup("0comp_20_x"), Some(Ty::Int));
}

#[test]
fn a_set_comprehension_producing_a_non_int_element_is_rejected_as_t0038() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::StringLiteral("x".to_string())),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert!(err.message.contains("set[str]"));
}

#[test]
fn a_set_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Set(Box::new(Ty::Int))));
}

#[test]
fn a_dict_comprehension_over_a_bare_dict_name_type_checks_and_binds_target_as_dict_str_int() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Dict`, binding `var` as the dict's *key* type (mirroring
    // `ForList`'s own `for k in d:` behavior, D-123).
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_20_k".to_string(),
        iter: CompIter::Name("d".to_string()),
        cond: None,
        key: Box::new(HirExpr::Name("0comp_20_k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(
        env.lookup("y"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
    assert_eq!(env.lookup("0comp_20_k"), Some(Ty::Str));
}

#[test]
fn a_dict_comprehension_producing_a_non_str_int_pair_is_rejected_as_t0036() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::IntLiteral(1)),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert!(err.message.contains("dict[int, int]"));
}

#[test]
fn a_dict_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    check_stmt(&mut env, &stmt).unwrap();
    assert_eq!(
        env.lookup("y"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    // Reuses `ForList`'s own existing message
    // (`a_for_list_loop_over_a_non_list_value_is_rejected`).
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_comprehension_over_an_undefined_iterable_name_is_rejected_as_not_defined() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("undefined".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}
#[test]
fn a_comprehension_over_a_maybe_bound_iterable_is_t0041() {
    // `if cond: xs = [1, 2, 3]` then `[x for x in xs]` -- xs is Maybe
    // after the if (no else), so `lookup_bound_name` raises T0041
    // (issue #118 Part 1).
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            }],
            orelse: vec![],
        },
    )
    .unwrap();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("xs".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn an_entirely_unannotated_private_helper_returning_a_comprehension_target_infers_list_int() {
    // #1254: the solver now binds a comprehension statement's `target` to
    // its container type (`bind_comp_target`), exactly as a plain
    // assignment of a `list[int]` literal already was (`def _h(): xs =
    // [1, 2]; return xs` resolves on the baseline). This test used to pin
    // the gap as a `T0021` "local name `xs` is not bound before this use";
    // CPython runs the program, so the return type is now inferred.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::ListCompAssign {
                    target: "xs".to_string(),
                    var: "0comp_11_i".to_string(),
                    iter: CompIter::Range {
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(3),
                        step: HirExpr::IntLiteral(1),
                    },
                    cond: None,
                    elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
                },
                HirStmt::Return(Some(HirExpr::Name("xs".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::List(Box::new(Ty::Int))
    ));
}

// `resolve_comp_iter`'s own `CompIter::Range` operand checks (shared by
// every comprehension kind and both scopes) -- exercised once here via
// a list comprehension, mirroring `a_for_range_loop_whose_start_is_
// undefined_propagates_the_error` and its stop/step siblings.

#[test]
fn a_comprehension_over_a_range_with_a_non_int_start_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::StringLiteral("x".to_string()),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range start expects"));
}

#[test]
fn a_comprehension_over_a_range_with_a_non_int_stop_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::StringLiteral("x".to_string()),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range stop expects"));
}

#[test]
fn a_comprehension_over_a_range_with_a_non_int_step_propagates_the_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::StringLiteral("x".to_string()),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("range step expects"));
}

// Per-arm error-propagation pins (module scope): each of `resolve_comp_iter`'s
// call, the loop-variable `check_assignment`, `cond`, and `elt`/`key`/`value`
// is its own `?`-guarded call site in each of the three comprehension arms,
// so each needs its own error-propagating test for D-014's coverage gate,
// distinct from the "rejected with a specific diagnostic" tests above.

#[test]
fn a_set_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_dict_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Name("x".to_string()),
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = check_stmt(&mut env, &stmt).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_list_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_set_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_dict_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable() {
    let mut env = Environment::new();
    env.bind("0comp_11_i".to_string(), Ty::Str);
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0023");
}

#[test]
fn a_list_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        elt: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_if_filter_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_list_comprehension_s_elt_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::ListCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_elt_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::SetCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_key_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::Name("undefined".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_value_propagates_an_ill_typed_error() {
    let mut env = Environment::new();
    let stmt = HirStmt::DictCompAssign {
        target: "y".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(check_stmt(&mut env, &stmt).unwrap_err().code, "T0021");
}

#[test]
fn referencing_an_assigned_name_infers_its_bound_type() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(
        infer_expr(&env, &HirExpr::Name("x".to_string())),
        Ok(Ty::Int)
    );
}

#[test]
fn adding_two_ints_infers_int() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_binop_with_an_undefined_left_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::Name("undefined".to_string())),
        right: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_binop_with_an_undefined_right_operand_propagates_the_error() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn true_division_of_two_ints_infers_float() {
    assert_eq!(
        numeric_result_type(BinOpKind::Div, Ty::Int, Ty::Int),
        Ok(Ty::Float)
    );
    assert_eq!(
        numeric_result_type(BinOpKind::Div, Ty::Bool, Ty::Bool),
        Ok(Ty::Float)
    );
}

#[test]
fn floor_division_of_two_ints_still_infers_int() {
    assert_eq!(
        numeric_result_type(BinOpKind::FloorDiv, Ty::Int, Ty::Int),
        Ok(Ty::Int)
    );
}

#[test]
fn referencing_an_undefined_name_is_a_clean_error_not_a_panic() {
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::Name("undefined".to_string())).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined"));
}

#[test]
fn adding_an_int_and_a_float_promotes_to_float() {
    let env = Environment::new();
    let expr = HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(HirExpr::FloatLiteral(2.5)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn an_empty_list_literal_cannot_be_inferred() {
    // Since #1021 (D-245) an empty literal that survives to the check
    // phase is one the pre-pass found no evidence for, so it reports
    // T0003 rather than borrowing T0021.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::ListLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0003");
    assert!(err.message.contains("empty list literal"));
}

#[test]
fn a_homogeneous_int_list_literal_infers_list_of_int() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn a_heterogeneous_list_literal_is_rejected_as_t0032() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("two".to_string()),
        HirExpr::IntLiteral(3),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0032");
    // Exact match, not a substring check: this is the same message text
    // baked into `tests/diagnostics/d0032_heterogeneous_list_literal
    // .expected.txt`, which can't currently run end-to-end (`pycc_mir`
    // is still non-exhaustive against Task 7's HIR forms) -- this unit
    // test is what actually guards that fixture's wording today.
    assert_eq!(
        err.message,
        "list element type mismatch: expected int (from the first element), found str"
    );
}

#[test]
fn a_list_literal_of_bool_and_int_is_rejected_since_homogeneity_uses_exact_ty_equality() {
    // D-105 requires the *exact same* `Ty` for every element -- unlike
    // `is_assignable`'s bool-is-an-int-subtype rule used elsewhere in
    // the crate, `[1, True]` is still T0032.
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1), HirExpr::BoolLiteral(true)]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0032");
}

#[test]
fn a_homogeneous_non_int_list_literal_is_rejected_as_t0034() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::StringLiteral("b".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
    // Exact match: same wording as
    // `tests/diagnostics/d0034_list_element_type_not_int.expected.txt`
    // (currently unrunnable end-to-end -- see the T0032 test above).
    assert_eq!(
        err.message,
        "list[str] is not compiled yet (D-105) -- only list[int] is"
    );
}

#[test]
fn a_list_literal_propagates_an_ill_typed_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::ListLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_a_list_infers_its_element_type() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_a_list_of_str_infers_str_proving_subscript_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn subscripting_with_a_bool_index_is_accepted_since_bool_is_an_int_subtype() {
    // D-086: `bool` is accepted wherever `int` is expected at an operand
    // boundary (mirroring `is_assignable`'s existing param/assignment
    // rule) -- `xs[True]` is ordinary, CPython-valid Python (PEP 285),
    // not a type error. Found by an automated PR review (PR #236) and
    // confirmed against a real `pycc build` before this fix.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_with_a_non_int_index_is_rejected() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("zero".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("list index must be"));
}

#[test]
fn subscripting_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    // Exact match: same wording as
    // `tests/diagnostics/d0033_subscript_on_non_list.expected.txt`
    // (currently unrunnable end-to-end -- see the T0032 test above).
    assert_eq!(err.message, "`int` does not support indexing");
}

#[test]
fn subscripting_a_non_list_value_with_a_non_int_index_reports_t0033_not_t0021() {
    // Both operands are ill-typed at once: the base doesn't support
    // indexing at all, AND the index isn't int-compatible. T0033
    // ("does this value support the operation at all") must fire
    // before T0021 ("is this operand's type wrong"). The `match
    // base_ty { Ty::List(..) => ..., Ty::Dict(..) => ..., Ty::Tuple(..)
    // => ..., other => T0033 }` dispatch above already gives this
    // precedence structurally -- a non-container base falls straight
    // to the `other` arm without ever reaching a container's own
    // index-type check -- but PR-10's own final whole-branch review
    // found and pinned this exact compound case as a regression test
    // before container types existed, so it is kept here too.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("zero".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`int` does not support indexing");
}

#[test]
fn subscripting_propagates_an_ill_typed_base_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("undefined".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_propagates_an_ill_typed_index_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// PR-12 Task 7 (D-118): `HirExpr::Slice` type-checking. Only
// `list[int]` ships slicing in v0.2 -- see the arm's own doc comment in
// `infer_expr_in` for the full base-type/element-type/bound-type
// diagnostic-order rationale.

#[test]
fn slicing_a_list_of_int_with_both_bounds_infers_list_of_int() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_start_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: Some(Box::new(HirExpr::IntLiteral(1))),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_stop_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: None,
        step: Some(Box::new(HirExpr::IntLiteral(1))),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_step_omitted_type_checks() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::IntLiteral(1))),
        stop: Some(Box::new(HirExpr::IntLiteral(3))),
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_with_every_bound_omitted_type_checks_as_xs_colon() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_a_dict_is_rejected_as_t0033() {
    // Mirrors real CPython: `d[1:2]` raises `TypeError` there too (D-118
    // -- dict/set reuse `Subscript`'s own `T0033` code, not a new one).
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("d".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`dict[str, int]` does not support slicing (only list[int] does)"
    );
}

#[test]
fn slicing_a_set_is_rejected_as_t0033() {
    // Mirrors real CPython: `s[1:2]` raises `TypeError` there too.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("s".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`set[int]` does not support slicing (only list[int] does)"
    );
}

#[test]
fn slicing_a_tuple_is_rejected_as_t0033_as_an_explicit_deferral() {
    // D-118: unlike dict/set (never supported for slicing), `tuple[...]`
    // slicing is a real, explicit v0.2 scope cut -- but it still reuses
    // the same `T0033` code, not a distinct one.
    let mut env = Environment::new();
    env.bind("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("t".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`tuple[int, int]` does not support slicing (only list[int] does)"
    );
}

#[test]
fn slicing_a_scalar_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("x".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`int` does not support slicing (only list[int] does)"
    );
}

#[test]
fn slicing_a_list_of_str_is_rejected_as_t0034() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert_eq!(
        err.message,
        "list codegen only supports `list[int]` in v0.2, cannot slice `list[str]`"
    );
}

#[test]
fn slicing_a_list_of_float_is_rejected_as_t0034() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Float)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert_eq!(
        err.message,
        "list codegen only supports `list[int]` in v0.2, cannot slice `list[float]`"
    );
}

#[test]
fn slicing_a_list_of_bool_is_rejected_as_t0034() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Bool)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert_eq!(
        err.message,
        "list codegen only supports `list[int]` in v0.2, cannot slice `list[bool]`"
    );
}

#[test]
fn slicing_with_a_non_int_start_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::StringLiteral("a".to_string()))),
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice start must be `int`, got `str`");
}

#[test]
fn slicing_with_a_non_int_stop_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: Some(Box::new(HirExpr::FloatLiteral(1.0))),
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice stop must be `int`, got `float`");
}

#[test]
fn slicing_with_a_non_int_step_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: None,
        stop: None,
        step: Some(Box::new(HirExpr::StringLiteral("a".to_string()))),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "slice step must be `int`, got `str`");
}

#[test]
fn slicing_with_a_bool_bound_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, mirroring `Subscript`'s own index check: `xs[True:]` is
    // ordinary, CPython-valid Python (`bool` is an `int` subtype, PEP
    // 285), not a type error.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::BoolLiteral(true))),
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::List(Box::new(Ty::Int))));
}

#[test]
fn slicing_reports_the_base_type_error_before_any_bound_error() {
    // D-118 diagnostic-order pin: the base-type gate (T0033) fires
    // before a bound's own type is ever inspected.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("x".to_string())),
        start: Some(Box::new(HirExpr::StringLiteral("bad".to_string()))),
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
}

#[test]
fn slicing_reports_the_element_type_error_before_any_bound_error() {
    // D-118 diagnostic-order pin: the element-type gate (T0034) fires
    // before a bound's own type is ever inspected.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::StringLiteral("bad".to_string()))),
        stop: None,
        step: None,
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0034");
}

#[test]
fn slicing_propagates_an_ill_typed_base_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("undefined".to_string())),
        start: None,
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn slicing_propagates_an_ill_typed_bound_s_error() {
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::Name("undefined".to_string()))),
        stop: None,
        step: None,
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn appending_a_matching_value_to_a_list_infers_none() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_a_str_to_a_list_of_str_infers_none_proving_append_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::StringLiteral("a".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_to_an_undefined_name_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn appending_to_a_local_name_read_before_assignment_is_unbound_local() {
    // `ListAppend`'s `list` field is a plain `String`, not an
    // `HirExpr::Name`, so it must replicate `HirExpr::Name`'s own
    // "unbound local" vs. "not defined" distinction by hand
    // (`lookup_bound_name`) rather than going through `infer_expr_in`'s
    // `Name` arm.
    let env = Environment::new();
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr_in(&env, &["x"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn appending_to_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.append()`"));
}

#[test]
fn appending_a_mismatched_value_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot append"));
}

#[test]
fn appending_a_bool_to_a_list_of_int_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, matching `Subscript`'s own index check in `lib.rs`:
    // `x.append(True)` on an already-typed `list[int]` is ordinary,
    // CPython-valid Python (`bool` is an `int` subtype) --
    // `is_assignable` applies here, not
    // ListLiteral's stricter exact-equality homogeneity rule (which
    // answers a different question: inferring an as-yet-unknown element
    // type, not checking a value against an already-known one). Found
    // by an automated whole-branch review (PR #236) and confirmed
    // against a real `pycc build` before this fix.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_propagates_an_ill_typed_value_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// -- PR-12 Task 10 (D-119): remaining container methods depth --------
// `list.pop()`, `dict.get(key, default)`, `set.add(value)` -- each
// mirrors `ListAppend`'s own `infer_expr_in` test coverage exactly
// (base-type gate, value/key/default-type gate, `lookup_bound_name`'s
// own "not defined"/"unbound local" distinction, propagation, D-086
// bool-subtypes-int leniency, genericity).

#[test]
fn popping_the_last_element_of_a_list_of_int_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn popping_from_a_list_of_str_infers_str_proving_pop_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn popping_from_an_undefined_name_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn popping_from_a_local_name_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    let err = infer_expr_in(&env, &["x"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn popping_from_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.pop()`"));
}

#[test]
fn getting_a_str_key_with_a_matching_int_default_infers_int() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn getting_a_bool_default_for_a_dict_of_int_values_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, matching `ListAppend`'s own leniency: `bool` is an `int`
    // subtype, so a `bool` default for a `dict[str, int]` is ordinary,
    // CPython-valid Python, not a type error.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn getting_from_an_undefined_dict_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn getting_from_a_local_dict_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr_in(&env, &["d"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn getting_from_a_non_dict_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Int);
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.get()`"));
}

#[test]
fn getting_with_a_mismatched_key_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::IntLiteral(1)),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot look up"));
}

#[test]
fn getting_with_a_mismatched_default_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot use"));
}

#[test]
fn getting_propagates_an_ill_typed_key_s_error() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::Name("undefined".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn getting_propagates_an_ill_typed_default_s_error() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn adding_an_int_value_to_a_set_of_int_infers_none() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn adding_a_bool_value_to_a_set_of_int_is_accepted_since_bool_is_an_int_subtype() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn adding_to_a_set_of_str_is_refused_by_the_set_element_check() {
    // #1343: `.add` checks its element (`set_element.rs`) because a `set[R]`
    // parameter reaches it with no literal checked. `set[str]` never gets
    // past the annotation gate, so this pins the defensive arm.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Str)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::StringLiteral("a".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0038");
}

#[test]
fn adding_to_an_undefined_set_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::SetAdd {
        set: "undefined".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn adding_to_a_local_set_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr_in(&env, &["s"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn adding_to_a_non_set_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Int);
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.add()`"));
}

#[test]
fn adding_to_a_frozenset_is_rejected_as_t0033_naming_frozenset() {
    // Part 1 of #1319: a `frozenset[int]` is immutable, so `.add` is
    // refused by the same arm as any other non-set receiver, and the
    // message names the receiver's own type.
    let mut env = Environment::new();
    env.bind("fs".to_string(), Ty::FrozenSet(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "fs".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`frozenset[int]` does not support `.add()`");
}

#[test]
fn adding_a_mismatched_value_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot add"));
}

#[test]
fn adding_propagates_an_ill_typed_value_s_error() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// Whole-module `check()` tests, mirroring `HirExpr::Slice`'s own pair
// (PR-12 Task 7, D-118) exactly: Task 3's own ledger records a real
// regression of exactly this shape (a solver arm that looked correct in
// isolation but was never actually reached by the block walker) -- these
// confirm each new arm is genuinely wired into both the fast
// (`check_with_environment_all`) and solver (`collect_block_constraints`)
// paths, not just correct when `infer_expr`/`collect_expr_constraints`
// are called directly on a hand-built expression.

#[test]
fn list_pop_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::ListPop {
                    list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn dict_get_or_default_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path()
 {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::DictGetOrDefault {
                    dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                    key: Box::new(HirExpr::StringLiteral("a".to_string())),
                    default: Box::new(HirExpr::IntLiteral(0)),
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn set_add_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::SetAdd {
                set: "s".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            })),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn list_pop_on_a_list_typed_parameter_infers_the_scalar_element_type_inside_an_unannotated_private_helper()
 {
    // D-146 (#239): `collect_expr_constraints`'s `ListPop` arm now looks
    // up `list` in `env.bindings` and, when bound to a `Ty::List` element-
    // type carrier, extracts the scalar element type -- the carrier is
    // destructured, never unified. Here `xs` is a parameter typed
    // `list[int]`, so `xs.pop()` produces `Some(Ok(Ty::Int))`, `y` is
    // bound to `Ok(Ty::Int)`, and `return y` resolves the unannotated
    // return to `Ty::Int`. This was the pre-existing D-116 solver binding
    // gap: before this fix the `ListPop` arm returned `Ok(None)`, so `y`
    // was never bound and the later `return y` failed with the actively
    // misleading "not bound before this use" even though the assignment
    // was textually right above it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::Assign {
                    target: "y".to_string(),
                    value: HirExpr::ListPop {
                        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                    },
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let signatures = infer_function_signatures_with_solver(&hir, &local_names).unwrap();
    assert_eq!(
        signatures["_h"],
        (vec![Ty::List(Box::new(Ty::Int))], Ty::Int)
    );
    assert!(check(&hir).is_ok());
}

#[test]
fn dict_get_or_default_assigned_inside_an_unannotated_private_helper_reaches_the_genuine_cannot_infer_return_type_diagnostic()
 {
    // Same pre-existing D-116 gap as the `ListPop` test immediately
    // above, exercised through `DictGetOrDefault`'s own
    // `collect_expr_constraints` arm (also `Ok(None)`) instead. Issue
    // #771/D-199: `y` is now tracked as definitely-but-opaquely bound
    // (`opaque_bindings`), so `return y` no longer misfires as an unbound
    // local ("not bound before this use") -- it correctly falls through
    // to the genuine "cannot infer return type" diagnostic instead, since
    // the solver still has no type term for `y` (that half of the gap is
    // unchanged; only the misleading consequence is fixed).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::Assign {
                    target: "y".to_string(),
                    value: HirExpr::DictGetOrDefault {
                        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                        key: Box::new(HirExpr::StringLiteral("a".to_string())),
                        default: Box::new(HirExpr::IntLiteral(0)),
                    },
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "cannot infer return type of private helper `_h`; add an annotation"
    );
}

#[test]
fn len_of_a_list_of_int_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn len_of_a_list_of_str_infers_int_proving_len_is_not_int_specific() {
    // Proves `len`'s own check isn't hardcoded to `Ty::Int` -- generic
    // over any scalar element type, same discipline already applied to
    // `Subscript`/`ListAppend`/`ForList` (D-105's own genericity claim).
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn len_with_no_arguments_is_rejected_as_t0033() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`len` expects exactly 1 argument, got 0");
}

#[test]
fn len_with_two_arguments_is_rejected_as_t0033() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`len` expects exactly 1 argument, got 2");
}

#[test]
fn len_of_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(
        err.message,
        "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `int`"
    );
}

#[test]
fn len_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn float_of_an_int_infers_float() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_of_a_bool_infers_float() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Bool);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_of_a_float_infers_float() {
    // Proves `float`'s own check isn't a narrowing-only conversion --
    // an already-`float` argument is accepted too (identity), same
    // discipline as `len_of_a_list_of_str_infers_int_proving_len_is_
    // not_int_specific`'s own genericity claim above.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Float);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn float_with_no_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`float` expects exactly 1 argument, got 0");
}

#[test]
fn float_with_two_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`float` expects exactly 1 argument, got 2");
}

/// Also the pin for PR 4a of #1083's deliberate omission: `float`'s gate now
/// admits `Ty::Object` too, but the message still enumerates only the three
/// spellable types, because in a native build `object` is unspellable in an
/// annotation (`docs/TYPE_SYSTEM.md`'s `object` row; D-258 admits it only in
/// an `--ext` module) and "pass an `object`" is advice nobody can act on.
#[test]
fn float_of_a_str_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Str);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`float` expects an `int`, `float`, or `bool` argument, got `str`"
    );
}

#[test]
fn float_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn math_sqrt_and_pi_type_check_through_the_constraint_solver_path() {
    // `math.sqrt`/`math.pi`'s `infer_expr_in` coverage above only
    // exercises pass 3 (the final, concrete-environment check). A
    // private helper with an unannotated (`Ty::Infer`) return forces
    // `collect_expr_constraints` (the solver pass) to resolve the same
    // call/name -- this is `inferred_signatures_keep_the_constraint_
    // solver_path`'s own precedent, applied to the new stdlib branch.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_root_plus_pi".to_string(),
            // `x` is annotated (not `Ty::Infer`) deliberately: like
            // `float`/`len`, `math.sqrt`'s solver-pass branch only
            // validates an already-concretely-resolved argument term
            // (see that branch's own doc comment on the "lenient-
            // until-known" precedent) -- it does not itself unify an
            // unresolved parameter's inference variable against the
            // stdlib signature's expected argument type. Only the
            // function's own `Ty::Infer` *return* is left for the
            // solver to determine here.
            params: vec![("x".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Call {
                    callee: "math.sqrt".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                }),
                right: Box::new(HirExpr::Name("math.pi".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let signatures = infer_function_signatures_with_solver(&hir, &local_names).unwrap();
    assert_eq!(signatures["_root_plus_pi"], (vec![Ty::Float], Ty::Float));
    assert!(check(&hir).is_ok());
}

#[test]
fn math_sqrt_wrong_arity_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_wrong_argument_type_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::StringLiteral("x".to_string())],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_pi_called_like_a_function_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.pi".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_used_as_a_bare_value_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_ref".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name(
                "math.sqrt".to_string(),
            )))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_dotted_name_whose_module_part_is_not_a_registered_stdlib_module_is_an_ordinary_undefined_name()
{
    // `std_qualified_symbol`'s `resolve_module` step is defense-in-depth
    // against a hand-constructed `HirModule` (this crate's own public
    // API accepts any `&HirModule`, not only one `pycc_hir::lower_checked`
    // produced) -- `pycc_hir`'s real lowering never emits a dotted name
    // whose module part fails to resolve (see `lower_expr`'s own
    // `Expr::Attribute` arm), but this crate does not get to assume
    // that invariant holds for every caller.
    let env = Environment::new();
    let expr = HirExpr::Name("os.path".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `os.path` is not defined");
}

#[test]
fn math_sqrt_of_a_float_infers_float() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::FloatLiteral(2.0)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn math_pi_infers_float() {
    let env = Environment::new();
    let expr = HirExpr::Name("math.pi".to_string());
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn math_sqrt_with_no_arguments_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`math.sqrt` expects 1 argument(s), got 0");
}

#[test]
fn math_sqrt_of_a_str_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Str);
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`math.sqrt` expects `float`, got `str`");
}

#[test]
fn math_sqrt_propagates_an_ill_typed_argument_s_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.sqrt".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn math_pi_called_like_a_function_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "math.pi".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`math.pi` is a stdlib constant, not a function, and cannot be called"
    );
}

#[test]
fn math_sqrt_used_as_a_bare_value_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Name("math.sqrt".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "`math.sqrt` is a stdlib function and must be called, e.g. `math.sqrt(...)`"
    );
}

#[test]
fn enum_marker_used_as_a_bare_value_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Name("enum.Enum".to_string());
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("class marker"));
}

#[test]
fn enum_marker_called_like_a_function_is_rejected_as_t0021() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "enum.Enum".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("class marker"));
}

#[test]
fn enum_marker_used_as_a_bare_value_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_ref".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name(
                "enum.Enum".to_string(),
            )))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn enum_marker_called_like_a_function_is_rejected_by_the_constraint_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad_call".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "enum.Enum".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn math_sqrt_call_is_shadowed_by_a_bound_local_named_math() {
    // Post-review finding: a real local/parameter legally named `math`
    // must shadow the stdlib module -- CPython would raise
    // `AttributeError` calling `.sqrt` on whatever `math` is actually
    // bound to here, not silently call libm's `sqrt`.
    let err = infer_expr_in(
        &Environment::new(),
        &["math"],
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "`math` is a local name here, not the stdlib `math` module -- attribute access on a non-module value is not supported yet"
    );
}

#[test]
fn math_pi_reference_is_shadowed_by_a_bound_local_named_math() {
    let err = infer_expr_in(
        &Environment::new(),
        &["math"],
        &HirExpr::Name("math.pi".to_string()),
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(
        err.message,
        "`math` is a local name here, not the stdlib `math` module -- attribute access on a non-module value is not supported yet"
    );
}

#[test]
fn math_sqrt_call_is_shadowed_by_a_module_level_binding_named_math() {
    let mut env = Environment::new();
    env.bind("math".to_string(), Ty::Float);
    let err = infer_expr(
        &env,
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn math_sqrt_and_pi_are_shadowed_by_a_bound_local_through_the_constraint_solver_path() {
    // The other half of the fix -- the private-helper solver pass
    // (`collect_expr_constraints`) needs the same shadowing guard as
    // `infer_expr_in` above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_shadowed".to_string(),
            params: vec![("math".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::Name("math".to_string())],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn math_pi_bare_reference_is_shadowed_by_a_bound_local_through_the_constraint_solver_path() {
    // The `HirExpr::Name` (bare-reference) half of the solver-path
    // shadowing guard, as opposed to the `HirExpr::Call` half already
    // covered by `math_sqrt_and_pi_are_shadowed_by_a_bound_local_
    // through_the_constraint_solver_path` above.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_shadowed_pi".to_string(),
            params: vec![("math".to_string(), Ty::Float)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("math.pi".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_user_defined_float_function_takes_priority_over_the_builtin() {
    // Post-merge review finding: unlike `len`/`print`, which have been
    // hand-recognized since before this compiler could compile
    // user-declared functions at all, `float` was undefined until #181,
    // so `def float(x: int) -> int: return x + 1` was a valid, working
    // program on `main` immediately before this builtin landed --
    // reproduced directly against a pristine `main` checkout, printing
    // `6`. Without this priority check, the builtin would silently
    // intercept the call and infer `Ty::Float` instead of the user's own
    // declared `Ty::Int` return type.
    let mut env = Environment::new();
    env.bind_function("float".to_string(), vec![Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::IntLiteral(5)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_user_defined_float_function_accepting_a_non_numeric_argument_type_checks() {
    // The other half of the same finding: the builtin's own argument-type
    // gate (`int`/`float`/`bool` only) must not apply when the user's own
    // `float` accepts something else entirely, e.g. `str`.
    let mut env = Environment::new();
    env.bind_function("float".to_string(), vec![Ty::Str], Ty::Str);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::StringLiteral("hello".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

// -- Issue #142: callable builtin classification as C0001 ------------

#[test]
fn known_callable_builtins_table_is_sorted() {
    // The `is_known_callable_builtin` binary search relies on
    // `KNOWN_CALLABLE_BUILTINS` being sorted by Rust's `str` ordering.
    // If this invariant is ever violated, binary search silently returns
    // false negatives -- a known builtin would be misclassified as
    // T0021 instead of C0001. The explicit `let` bindings ensure both
    // values are always evaluated, and the `assert!` carries no format
    // args -- the idiom these tests adopted while they lived in `lib.rs`
    // and counted toward the D-014 denominator.
    for w in KNOWN_CALLABLE_BUILTINS.windows(2) {
        let prev = w[0];
        let next = w[1];
        assert!(prev <= next, "KNOWN_CALLABLE_BUILTINS is not sorted");
        // Strict ordering (no duplicates): `prev < next` is the real
        // invariant, but `prev <= next` above already covers both
        // values. This redundant strict check catches duplicates without
        // introducing an uncovered format-arg branch.
        assert!(prev != next, "KNOWN_CALLABLE_BUILTINS has a duplicate");
    }
}

#[test]
fn known_callable_builtins_table_has_135_entries() {
    assert_eq!(
        KNOWN_CALLABLE_BUILTINS.len(),
        135,
        "KNOWN_CALLABLE_BUILTINS should have 135 entries (139 builtins minus print/len/float/isinstance/issubclass, plus __import__)"
    );
}

#[test]
fn known_callable_builtins_excludes_already_implemented() {
    // `print`, `len`, `float`, `isinstance`, and `issubclass` are already
    // hand-recognized and must not appear in the table -- they would never
    // reach this fallback.
    assert!(!is_known_callable_builtin("print"));
    assert!(!is_known_callable_builtin("len"));
    assert!(!is_known_callable_builtin("float"));
    assert!(!is_known_callable_builtin("isinstance"));
    assert!(!is_known_callable_builtin("issubclass"));
}

#[test]
fn is_known_callable_builtin_finds_table_entries() {
    // Representative samples from both halves of the table (exception
    // classes and ordinary builtins).
    assert!(is_known_callable_builtin("ValueError"));
    assert!(is_known_callable_builtin("Exception"));
    assert!(is_known_callable_builtin("int"));
    assert!(is_known_callable_builtin("range"));
    assert!(is_known_callable_builtin("zip"));
    assert!(is_known_callable_builtin("ArithmeticError"));
    assert!(is_known_callable_builtin("ZeroDivisionError"));
}

#[test]
fn is_known_callable_builtin_rejects_unknown_names() {
    assert!(!is_known_callable_builtin("totally_undefined"));
    assert!(!is_known_callable_builtin("print"));
    assert!(!is_known_callable_builtin(""));
}

#[test]
fn value_error_call_produces_c0001_not_t0021() {
    // `ValueError("x")` is valid Python -- the builtin genuinely exists
    // in Python 3.14 -- so it must be classified as a capability gap
    // (C0001), not a name-resolution failure (T0021).
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("ValueError"));
}

#[test]
fn exception_call_produces_c0001() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "Exception".to_string(),
        args: vec![HirExpr::StringLiteral("msg".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("Exception"));
}

#[test]
fn other_callable_builtins_produce_c0001() {
    // A representative sample of other callable builtins from the table.
    for name in ["int", "str", "bool", "range", "zip", "dict", "list", "sum"] {
        let env = Environment::new();
        let expr = HirExpr::Call {
            callee: name.to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        };
        let err = infer_expr(&env, &expr).unwrap_err();
        assert_eq!(err.code, "C0001", "builtin `{name}` should be C0001");
        assert!(err.message.contains(name));
    }
}

#[test]
fn a_genuinely_undefined_function_still_produces_t0021() {
    // A typo or genuinely undefined name must retain the T0021
    // name-resolution behavior, not be reclassified as C0001.
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "totally_undefined".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined function"));
}

#[test]
fn a_user_defined_value_error_takes_priority_over_c0001() {
    // A user `def ValueError(...)` is called correctly, not classified as
    // C0001 -- user definitions always take priority over the builtin
    // classification, matching `float`'s own user-definition-takes-
    // priority precedent.
    let mut env = Environment::new();
    env.bind_function("ValueError".to_string(), vec![Ty::Str], Ty::Int);
    let expr = HirExpr::Call {
        callee: "ValueError".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_user_defined_exception_takes_priority_over_c0001() {
    let mut env = Environment::new();
    env.bind_function("Exception".to_string(), vec![Ty::Str], Ty::Bool);
    let expr = HirExpr::Call {
        callee: "Exception".to_string(),
        args: vec![HirExpr::StringLiteral("msg".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Bool));
}

#[test]
fn range_as_a_standalone_call_produces_c0001() {
    // `range` works in `for` loops (via `HirStmt::ForRange`) but not as
    // a standalone call -- a standalone `range(10)` is C0001.
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "range".to_string(),
        args: vec![HirExpr::IntLiteral(10)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("range"));
}

#[test]
fn import_dunder_call_produces_c0001_not_t0021() {
    // `__import__("math")` is valid Python -- the callable dunder
    // `__import__` is the one dunder users legitimately call, so it
    // is included in the builtin table. It must be classified as C0001,
    // not T0021 (Codex review P2).
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "__import__".to_string(),
        args: vec![HirExpr::StringLiteral("math".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(err.message.contains("__import__"));
}

// -- PR-11 Task 3 (D-123): dict[str, int] type-checking --------------

#[test]
fn an_empty_dict_literal_cannot_be_inferred() {
    // Since #1021 (D-245) an empty literal that survives to the check
    // phase is one the pre-pass found no evidence for, so it reports
    // T0003 rather than borrowing T0021.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::DictLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0003");
    assert!(err.message.contains("empty dict literal"));
}

#[test]
fn a_homogeneous_dict_literal_infers_dict_of_str_int() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::IntLiteral(2),
        ),
    ]);
    assert_eq!(
        infer_expr(&env, &expr),
        Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_dict_literal_with_mismatched_value_types_is_rejected_as_t0035() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::StringLiteral("oops".to_string()),
        ),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0035");
    assert_eq!(
        err.message,
        "dict entry type mismatch: expected str: int (from the first pair), found str: str"
    );
}

#[test]
fn a_dict_literal_with_mismatched_key_types_is_rejected_as_t0035() {
    // Same code as the value-mismatch case above, but exercises the
    // `this_key_ty != key_ty` half of the homogeneity check's `||`
    // independently of the value half.
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (HirExpr::IntLiteral(2), HirExpr::IntLiteral(2)),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0035");
    assert_eq!(
        err.message,
        "dict entry type mismatch: expected str: int (from the first pair), found int: int"
    );
}

#[test]
fn a_homogeneous_non_str_int_dict_literal_is_rejected_as_t0036() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::StringLiteral("b".to_string()),
    )]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert_eq!(
        err.message,
        "dict[str, str] is not compiled yet (D-122) -- only dict[str, int] is"
    );
}

#[test]
fn a_bad_dict_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The dict-shaped counterpart to
    // `a_heterogeneous_list_literal_is_still_rejected_when_the_solver_path_runs_first`
    // above: `collect_expr_constraints`'s own `DictLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for `Ty::Dict`
    // either), so an unrelated private helper with an unresolved (`Ty::Infer`)
    // signature must not let the solver's own leniency swallow a genuine T0036 --
    // it has to fall through to the real, dict-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::StringLiteral("b".to_string()),
                )]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0036");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_first_key_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::Name("undefined".to_string()),
        HirExpr::IntLiteral(1),
    )]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_first_value_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::Name("undefined".to_string()),
    )]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_later_key_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::Name("undefined".to_string()),
            HirExpr::IntLiteral(2),
        ),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_literal_propagates_an_ill_typed_later_value_s_error() {
    let env = Environment::new();
    let expr = HirExpr::DictLiteral(vec![
        (
            HirExpr::StringLiteral("a".to_string()),
            HirExpr::IntLiteral(1),
        ),
        (
            HirExpr::StringLiteral("b".to_string()),
            HirExpr::Name("undefined".to_string()),
        ),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn subscripting_a_dict_infers_its_value_type() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::StringLiteral("a".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn subscripting_a_dict_with_a_mismatched_key_type_is_rejected() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "dict key type mismatch: expected `str`, found `int`"
    );
}

#[test]
fn len_of_a_dict_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

/// `check_try_star_stmt`'s own comment on its `if let Some(exc_types) =
/// &handler.exc_type` check explains that a bare `except*:` is rejected by
/// ruff's own parser as a syntax error (PEP 654 requires every `except*`
/// clause to name a type), so `handler.exc_type` is always `Some` by the
/// time an `HirStmt::TryStar` reaches type-checking through the public
/// `check`/`build` CLI -- there is no valid Python source that reaches this
/// function with `exc_type: None`. `HirExceptHandler.exc_type` is still
/// `Option<Vec<String>>` at the type level, though, so the `None` arm is a
/// real branch the compiler must still exhaustively handle; a fixture that
/// builds a `HirStmt::TryStar` directly (bypassing the parser) and passes a
/// handler with `exc_type: None` is the only way to prove it once
/// unconditionally skips the whole type-validation block and falls through
/// to bind the handler's name (if any) and check its body, exactly like a
/// handler whose single named type happens to validate cleanly.
#[test]
fn check_try_star_stmt_skips_type_validation_for_a_handler_with_no_named_type() {
    let mut env = Environment::default();
    let body: Vec<HirStmt> = vec![];
    let handlers = vec![pycc_hir::HirExceptHandler {
        exc_type: None,
        name: Some("e".to_string()),
        body: vec![],
    }];
    let orelse: Vec<HirStmt> = vec![];
    let finalbody: Vec<HirStmt> = vec![];

    check_try_star_stmt(&mut env, &[], &body, &handlers, &orelse, &finalbody, None)
        .expect("a handler with no named type should skip validation, not error");
}

#[test]
fn local_name_collection_ignores_a_dict_set_target() {
    // `d[k] = v` mutates an existing binding's contents, not a name --
    // unlike `Assign`/`AnnAssign`/`ForList`, it must not be collected as
    // a new local.
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert_eq!(function_local_names(&params, &body), Vec::<&str>::new());
}

#[test]
fn contains_return_treats_a_dict_set_as_not_a_return() {
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert!(!contains_return(&body));
}

#[test]
fn block_always_returns_treats_a_dict_set_as_not_always_returning() {
    let body = vec![HirStmt::DictSet {
        dict: "x".to_string(),
        key: HirExpr::StringLiteral("a".to_string()),
        value: HirExpr::IntLiteral(1),
    }];
    assert!(!block_always_returns(&body));
}

fn a_list_comp_assign_stmt() -> HirStmt {
    HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    }
}

fn a_set_comp_assign_stmt() -> HirStmt {
    HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
    }
}

fn a_dict_comp_assign_stmt() -> HirStmt {
    HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "0comp_11_i".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    }
}

#[test]
fn local_name_collection_includes_a_list_comp_assign_s_target_and_synthesized_var() {
    // PR-12 Task 3 (D-117): unlike `DictSet` above, a comprehension
    // introduces two brand-new local names (`target` and the
    // synthesized `var`), not zero.
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_list_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn local_name_collection_skips_a_list_comp_assign_s_target_and_var_when_already_local() {
    // Exercises the `!is_local(...)` guard's "already local" branch: a
    // parameter sharing the comprehension's `target`/`var` names must
    // not be pushed a second time.
    let params: Vec<(String, Ty)> = vec![
        ("xs".to_string(), Ty::Infer),
        ("0comp_11_i".to_string(), Ty::Infer),
    ];
    let body = vec![a_list_comp_assign_stmt()];
    let names = function_local_names(&params, &body);
    assert_eq!(names, vec!["xs", "0comp_11_i"]);
}

#[test]
fn local_name_collection_includes_a_set_comp_assign_s_target_and_var() {
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_set_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn local_name_collection_includes_a_dict_comp_assign_s_target_and_var() {
    let params: Vec<(String, Ty)> = vec![];
    let body = vec![a_dict_comp_assign_stmt()];
    let mut names = function_local_names(&params, &body);
    names.sort_unstable();
    assert_eq!(names, vec!["0comp_11_i", "xs"]);
}

#[test]
fn contains_return_treats_a_list_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_list_comp_assign_stmt()]));
}

#[test]
fn contains_return_treats_a_set_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_set_comp_assign_stmt()]));
}

#[test]
fn contains_return_treats_a_dict_comp_assign_as_not_a_return() {
    assert!(!contains_return(&[a_dict_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_list_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_list_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_set_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_set_comp_assign_stmt()]));
}

#[test]
fn block_always_returns_treats_a_dict_comp_assign_as_not_always_returning() {
    assert!(!block_always_returns(&[a_dict_comp_assign_stmt()]));
}

#[test]
fn private_helper_parameter_is_inferred_through_a_comprehension_s_elt() {
    // Regression test (pinned-reviewer finding, pre-merge, P1): an
    // earlier version of `collect_block_constraints`'s comprehension arm
    // was a bare no-op -- it never recursed into `iter`/`cond`/`elt`
    // at all, unlike every sibling arm in this function (`ForRange`,
    // `ForList`, `DictSet` above all still recurse into their own
    // sub-expressions even though they don't register a term for their
    // own container). That meant a call expression appearing only
    // inside a comprehension's `elt` never participated in this
    // solver's argument<->parameter unification (see
    // `private_parameter_is_inferred_by_forwarding_into_an_annotated_callee`
    // for the plain, non-comprehension version of this exact mechanism):
    // `_forward`'s unannotated `x` parameter, used only as an argument
    // to the fully-annotated `_sink`, used to spuriously fail with
    // "cannot infer type of parameter `x`; add an annotation" even
    // though `x: int` is the only type consistent with this program.
    // `bind_comp_loop_var` + recursing into `elt` (Step 5's actual fix)
    // makes this resolve correctly and reach the real check pass, which
    // accepts the program (`_sink` returns `int`, satisfying the list
    // comprehension's own `T0034` gate).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "0comp_11_i".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::Call {
                            callee: "_sink".to_string(),
                            args: vec![HirExpr::Name("x".to_string())],
                        }),
                    },
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn bind_comp_loop_var_unifies_a_range_comprehension_s_loop_variable_with_an_existing_term() {
    // Exercises `bind_comp_loop_var`'s `unify_terms(existing, Ok(Ty::Int), ...)`
    // branch (mirrors `ForRange`'s own analogous branch, exercised by
    // `collect_block_constraints_unifies_a_for_range_loop_variable_s_existing_term`-
    // style coverage elsewhere in this file): the loop variable is
    // already bound to an inferred term (via forwarding into another
    // annotated helper) before the comprehension re-binds it to `Ty::Int`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "_sink".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    }),
                    HirStmt::ListCompAssign {
                        target: "y".to_string(),
                        var: "x".to_string(),
                        iter: CompIter::Range {
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
                            step: HirExpr::IntLiteral(1),
                        },
                        cond: None,
                        elt: Box::new(HirExpr::IntLiteral(1)),
                    },
                    HirStmt::Return(None),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn a_dict_set_item_with_matching_value_type_checks_cleanly() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::IntLiteral(1),
            )]),
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("b".to_string()),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap();
    // `d[k] = v` never rebinds `x`'s own environment type, unlike an
    // ordinary `Assign`.
    assert_eq!(
        env.lookup("x"),
        Some(Ty::Dict(Box::new((Ty::Str, Ty::Int))))
    );
}

#[test]
fn a_dict_set_item_accepts_a_bool_value_since_bool_is_an_int_subtype() {
    // Mirrors `appending_a_bool_to_a_list_of_int_is_accepted_since_bool_is_an_int_subtype`
    // (D-086): `d[k] = True` is ordinary, CPython-valid Python.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::BoolLiteral(true),
        },
    )
    .unwrap();
}

#[test]
fn a_dict_set_item_with_wrong_key_type_is_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::IntLiteral(1),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "dict key type mismatch: expected `str`, found `int`"
    );
}

#[test]
fn a_dict_set_item_with_wrong_value_type_is_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::StringLiteral("oops".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "cannot assign `str` to a dict value of `int`");
}

#[test]
fn a_dict_set_item_on_a_non_dict_value_is_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`int` does not support item assignment");
}

#[test]
fn a_dict_set_item_on_a_list_value_is_still_t0033() {
    // D-123 supersedes D-105's "no subscript assignment target anywhere
    // in this file" HIR-level invariant (see `pycc_hir`'s own
    // `subscript_assignment_to_a_bare_name_base_lowers_to_dict_set`
    // test), but `list[int]` itself is still read-only-indexed -- this
    // is where that invariant is actually enforced now.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`list[int]` does not support item assignment");
}

#[test]
fn a_dict_set_item_over_an_undefined_name_is_rejected_as_not_defined() {
    let mut env = Environment::new();
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("is not defined"));
}

#[test]
fn a_dict_set_item_propagates_an_ill_typed_key_expression_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::Name("undefined".to_string()),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_dict_set_item_propagates_an_ill_typed_value_expression_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let err = check_stmt(
        &mut env,
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::Name("undefined".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn function_scope_dict_set_item_with_matching_value_type_checks_cleanly() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap();
}

#[test]
fn function_scope_dict_set_item_over_an_unbound_local_is_unbound_local() {
    // Mirrors the local-vs-global unbound distinction `lookup_bound_name`
    // already gives every other function-scope name lookup (e.g.
    // `ForList`'s own `list` field) -- a declared-but-unassigned local
    // reports `unbound_local`, not "not defined".
    let mut env = Environment::new();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::StringLiteral("a".to_string()),
            value: HirExpr::IntLiteral(1),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("is not bound before this use"));
}

// -- PR-11 Task 7 (D-123): set[int] type-checking ---------------------

#[test]
fn an_empty_set_literal_cannot_be_inferred() {
    // Unlike `DictLiteral(vec![])`, which is reachable from real source
    // (`{}` parses as an empty dict), an empty `SetLiteral` can only
    // ever be hand-built HIR -- Python has no empty-set literal spelling
    // at all.
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::SetLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("empty set literal"));
}

#[test]
fn a_homogeneous_set_literal_infers_set_of_int() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::IntLiteral(2),
        HirExpr::IntLiteral(3),
    ]);
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Set(Box::new(Ty::Int))));
}

#[test]
fn a_set_literal_with_mismatched_element_types_is_rejected_as_t0037() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("oops".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0037");
    assert_eq!(
        err.message,
        "set element type mismatch: expected int (from the first element), found str"
    );
}

#[test]
fn a_homogeneous_non_int_set_literal_is_rejected_as_t0038() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::StringLiteral("b".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert_eq!(
        err.message,
        "set[str] is not compiled yet (D-122) -- only set[int] and a set of a user-class instance are"
    );
    assert!(err.message.contains("set[str]"));
}

#[test]
fn a_bad_set_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The set-shaped counterpart to
    // `a_bad_dict_literal_is_still_rejected_when_the_solver_path_runs_first`:
    // `collect_expr_constraints`'s own `SetLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for
    // `Ty::Set` either), so an unrelated private helper with an
    // unresolved (`Ty::Infer`) signature must not let the solver's own
    // leniency swallow a genuine T0038 -- it has to fall through to the
    // real, set-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::SetLiteral(vec![
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::StringLiteral("b".to_string()),
                ]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0038");
}

#[test]
fn a_set_literal_propagates_an_ill_typed_first_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_set_literal_propagates_an_ill_typed_later_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::SetLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::Name("undefined".to_string()),
    ]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn indexing_a_set_reports_t0033() {
    // Mirrors real CPython: sets are not subscriptable either (D-123) --
    // no explicit `Ty::Set` arm exists in `Subscript`'s own match, and
    // the generic `other` fallthrough already covers it.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("x".to_string())),
        index: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`set[int]` does not support indexing");
}

#[test]
fn len_of_a_set_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::Call {
        callee: "len".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

// -- PR-11b Task 3 (D-116): tuple[...] construction + indexing --------

#[test]
fn a_tuple_literal_of_int_bool_float_infers_ty_tuple() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::BoolLiteral(true),
        HirExpr::FloatLiteral(2.5),
    ]);
    assert_eq!(
        infer_expr(&env, &expr),
        Ok(Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])))
    );
}

#[test]
fn an_empty_tuple_literal_is_rejected_as_t0021() {
    let env = Environment::new();
    let err = infer_expr(&env, &HirExpr::TupleLiteral(vec![])).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("empty tuple literal"));
}

#[test]
fn a_tuple_literal_with_a_string_element_is_rejected_as_t0039() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("a".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0039");
    assert_eq!(
        err.message,
        "tuple element type `str` is not compiled yet (D-116) -- only int/bool/float elements are"
    );
}

#[test]
fn a_tuple_literal_propagates_an_ill_typed_element_s_error() {
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![HirExpr::Name("undefined".to_string())]);
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn a_tuple_literal_reports_an_earlier_element_s_t0039_before_a_later_undefined_name() {
    // D-228 decision 4: `check_tuple_element_ty` runs per element *inside*
    // this loop rather than as a whole-`Ty` postcheck, so the composed
    // `(1, "a", undefined_name)` shape pins which diagnostic wins. The gate
    // fires on element 1 before element 2 is ever inferred, so `T0039` is
    // reported and the undefined name is never reached. Folding the
    // per-element gate into a whole-`Ty` postcheck like `T0034`/`T0036`/
    // `T0038` would silently flip this to `T0021`.
    let env = Environment::new();
    let expr = HirExpr::TupleLiteral(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("a".to_string()),
        HirExpr::Name("undefined".to_string()),
    ]);
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0039");
    assert_eq!(
        err.message,
        "tuple element type `str` is not compiled yet (D-116) -- only int/bool/float elements are"
    );
}

#[test]
fn tuple_index_with_a_literal_in_range_int_infers_the_positional_element_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "t".to_string(),
                value: HirExpr::TupleLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::BoolLiteral(true),
                ]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("t".to_string())),
                    index: Box::new(HirExpr::IntLiteral(1)),
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn subscripting_a_tuple_with_a_literal_index_infers_the_positional_element_type() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(2)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Float));
}

#[test]
fn tuple_index_with_a_non_literal_expression_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    env.bind("i".to_string(), Ty::Int);
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::Name("i".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
    assert_eq!(
        err.message,
        "tuple index must be a non-negative literal integer within range"
    );
}

#[test]
fn tuple_index_out_of_range_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(5)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
}

#[test]
fn a_negative_literal_tuple_index_is_rejected_as_t0040() {
    let mut env = Environment::new();
    env.bind(
        "t".to_string(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
    );
    let expr = HirExpr::Subscript {
        base: Box::new(HirExpr::Name("t".to_string())),
        index: Box::new(HirExpr::IntLiteral(-1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0040");
}

#[test]
fn a_bad_tuple_literal_is_still_rejected_when_the_solver_path_runs_first() {
    // The tuple-shaped counterpart to
    // `a_bad_set_literal_is_still_rejected_when_the_solver_path_runs_first`:
    // `collect_expr_constraints`'s own `TupleLiteral` arm always returns
    // `Ok(None)` (it has no unification-friendly representation for
    // `Ty::Tuple` either), so an unrelated private helper with an
    // unresolved (`Ty::Infer`) signature must not let the solver's own
    // leniency swallow a genuine T0039 -- it has to fall through to the
    // real, tuple-aware check pass.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::TupleLiteral(vec![HirExpr::StringLiteral("a".to_string())]),
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0039");
}

#[test]
fn a_top_level_binary_addition_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::IntLiteral(1)),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_top_level_reference_to_an_undefined_name_is_a_clean_error() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name(
            "undefined".to_string(),
        )))],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_top_level_call_to_a_previously_defined_function_type_checks() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "main".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_call_a_sibling_function_defined_before_it() {
    // Regression test for D-040: `check_function`'s own env used to be
    // seeded empty, so `main` couldn't see `helper` even though both are
    // ordinary module-level functions -- a valid, non-recursive call
    // between two sibling functions was wrongly rejected with T0021.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![HirExpr::IntLiteral(5)],
                    }],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_call_a_sibling_function_defined_after_it() {
    // Same gap as above, but exercising the pre-registration pass (D-039)
    // from the *other* direction: `main` is checked first (it's first in
    // the module) yet still must see `helper`, which is defined later.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![HirExpr::IntLiteral(5)],
                    }],
                })],
            },
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_read_a_module_level_global_defined_before_it() {
    // Regression test for D-041: reading a module global from a function
    // body needs no `global` declaration in real Python (that's only
    // required to *rebind* one) -- child_for_function used to reset
    // bindings to empty, so `f`'s body couldn't see `x` even though it's
    // an ordinary module-level constant, not some caller's local.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_can_read_a_module_level_global_defined_after_it() {
    // Same gap, other direction: a function is only ever *called* after
    // the module has (typically) finished running top to bottom, so a
    // global defined later in the file is still visible inside an
    // earlier function's body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(5),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_parameter_shadows_a_module_level_global_of_the_same_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("global".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // If the global (Ty::Str) leaked through instead of the parameter
    // (Ty::Int), this would fail with a T0022 return-type mismatch.
    assert!(check(&hir).is_ok());
}

#[test]
fn a_later_local_assignment_blocks_fallback_to_a_same_named_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn a_read_before_local_assignment_is_local_even_without_a_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn local_name_collection_deduplicates_assignment_and_for_targets() {
    let body = vec![
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["x", "i"]);
}

#[test]
fn local_name_collection_deduplicates_for_list_targets() {
    let body = vec![
        HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        },
        HirStmt::ForList {
            var: "i".to_string(),
            list: "ys".to_string(),
            body: vec![],
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["i"]);
}

#[test]
fn local_name_collection_deduplicates_annotated_assignment_targets() {
    let body = vec![
        HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
        },
        HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        },
    ];

    assert_eq!(function_local_names(&[], &body), vec!["x"]);
}

#[test]
fn contains_return_treats_an_annotated_assignment_as_not_a_return() {
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "x".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(1)),
    }];
    assert!(!contains_return(&body));
}

#[test]
fn block_always_returns_treats_an_annotated_assignment_as_not_a_return() {
    let body = vec![HirStmt::AnnAssign {
        is_final: false,
        target: "x".to_string(),
        annotation: Ty::Int,
        value: None,
    }];
    assert!(!block_always_returns(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_for_list_loop_body() {
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("i".to_string())))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn block_always_returns_treats_a_for_list_loop_as_not_always_returning() {
    // A `for` loop's body might never execute (an empty list), so it can
    // never on its own guarantee a function returns -- same treatment as
    // `ForRange` gets in this same match.
    let body = vec![HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("i".to_string())))],
    }];
    assert!(!block_always_returns(&body));
}

#[test]
fn a_call_before_local_assignment_cannot_fall_back_to_a_global_function() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![],
                    }),
                    HirStmt::Assign {
                        target: "helper".to_string(),
                        value: HirExpr::IntLiteral(1),
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `helper` is not bound before this use"
    );
}

#[test]
fn a_call_before_local_assignment_cannot_fall_back_to_print() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![],
                }),
                HirStmt::Assign {
                    target: "print".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `print` is not bound before this use"
    );
}

#[test]
fn a_bound_local_value_cannot_fall_back_to_a_function_registry_entry() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::Assign {
                        target: "helper".to_string(),
                        value: HirExpr::IntLiteral(1),
                    },
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: "helper".to_string(),
                        args: vec![],
                    }),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

#[test]
fn a_parameter_cannot_fall_back_to_a_same_named_builtin() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("print".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

#[test]
fn direct_function_check_uses_the_same_lexical_local_classification() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
            HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(2),
            },
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn direct_function_check_treats_an_unbound_call_target_as_a_local_read() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            }),
            HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            },
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "local name `helper` is not bound before this use"
    );
}

#[test]
fn direct_function_check_rejects_calling_a_bound_local_value() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            },
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            }),
        ],
    };

    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// #133: a module-level value binding must shadow a same-named user
/// function at every later call site (CPython raises `TypeError: 'int'
/// object is not callable`; the pre-fix checker resolved the call
/// through the function registry and accepted it).
#[test]
fn a_module_value_binding_shadows_a_same_named_function_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// #133: the same shadowing rule covers builtins -- `print = 1` makes a
/// later `print()` a call on an `int`, which CPython rejects at runtime.
#[test]
fn a_module_value_binding_shadows_a_builtin_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "print".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

/// #133: pass 3 checks annotated function bodies against the final
/// module environment (D-041), so a body call whose target is
/// module-bound to a value is rejected through `infer_expr_in`'s gate.
#[test]
fn an_annotated_body_call_sees_the_module_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// #133: the solver path (an underscore private helper with an inferred
/// signature) seeds its body environment from the module globals, so
/// `collect_expr_constraints`'s mirrored gate rejects the same shape.
/// Mirrors the issue's own third reproduction end to end through
/// `check_and_resolve`.
#[test]
fn a_private_helper_call_sees_the_module_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "_call_helper".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's first recorded ordering consequence, pinned as deliberate: a
/// body call is rejected whenever the callee is value-bound anywhere at
/// module top level, even when the program's only call executes *before*
/// the rebinding in CPython's dynamic order (this exact module runs fine
/// under CPython: `caller()` returns 1, then `helper = 2` rebinds). Pass
/// 3 checks bodies against the final module environment (D-041), and
/// D-110 accepts the stricter-than-one-dynamic-trace rejection rather
/// than importing #22's execution-order model.
#[test]
fn a_rebinding_after_the_only_call_still_rejects_the_body_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::Call {
                    callee: "caller".to_string(),
                    args: vec![],
                },
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's second recorded ordering consequence: the binding gate is
/// callee-first, so a shadowed callee with an additionally-invalid
/// argument reports the non-callable callee, not the argument's own
/// error -- uniform with the pre-existing local gate, though CPython's
/// evaluation order would surface the argument's `NameError` first.
#[test]
fn a_shadowed_callee_reports_before_its_argument_errors() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![HirExpr::Name("undefined_name".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// D-110's third recorded consequence: a bound value that shadows
/// nothing at all now reports the accurate non-callable diagnostic
/// instead of final validation's misleading "call to undefined function".
#[test]
fn calling_a_bound_value_that_shadows_nothing_reports_non_callable() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "x".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `x` is bound to a non-callable value");
}

/// The discriminating case for `collect_expr_constraints`' mirror gate
/// (D-110): the bound callee is neither `print` nor any `def`, inside an
/// inference-signature helper. Without the mirror, `signatures.get`
/// misses, the call stays unresolved, and the solver dead-ends in
/// "cannot infer return type of private helper `_h`; add an annotation"
/// *before* pass 3 ever runs -- a misleading message, since no
/// annotation fixes calling an `int`. This is the one shape where the
/// mirror's presence changes the observable diagnostic; a shadowed
/// `print` would instead be resolved by the special case and caught by
/// pass 3's own gate later (see the mirror-gate comment).
#[test]
fn a_private_helper_calling_a_shadowed_non_function_needs_the_mirror_gate() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "x".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `x` is bound to a non-callable value");
}

/// Behavior pin for the shadowed-builtin shape inside an
/// inference-signature helper: the mirror gate fires before the `print`
/// special case, so the accurate non-callable diagnostic wins. (Pass 3
/// would independently reject this shape even without the mirror; the
/// discriminating case for the mirror itself is the test above.)
#[test]
fn a_private_helper_calling_a_shadowed_builtin_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "print".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `print` is bound to a non-callable value");
}

/// The round-5 blocker's pin (PR #252's review): a *compatible-type*
/// value reassignment after the `def` must re-shadow it, even though
/// `check_assignment`'s already-bound branch never calls `bind()` --
/// clearing the def-rebound flag only inside `bind()` left it
/// permanently stuck for any name with a pre-`def` binding, so this
/// exact module resolved the function where CPython raises `TypeError`.
#[test]
fn a_compatible_reassignment_after_the_def_shadows_it_again() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The round-6 pin (PR #252's review): a private helper's *parameter*
/// that collides with a module-level def-rebound name re-binds it for
/// that body, so calling the parameter reports the accurate
/// non-callable diagnostic -- before the fix the per-function env setup
/// stripped the name from `bindings` but left the stale def-rebound
/// flag, skipping the mirror gate and mislabeling a bound parameter as
/// "not bound before this use".
#[test]
fn a_parameter_colliding_with_a_def_rebound_name_reports_non_callable() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![("helper".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The solver-path twin of the test above, proving the two environment
/// walks agree: the solver's Assign arm already cleared its def-rebound
/// flag unconditionally, and after the round-5 fix the `Environment`
/// walk does too.
#[test]
fn a_compatible_reassignment_after_the_def_shadows_it_in_the_solver_too() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(2),
            }),
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "name `helper` is bound to a non-callable value"
    );
}

/// The round-4 blocker's pin (PR #252's review): the `def`-rebinding
/// fact must NOT erase the representation record, so D-040's sticky
/// rule still rejects `helper = 1; def helper(): ...; helper = "leaked"`
/// with T0023 -- a version that cleared `bindings` at the `def` let this
/// program reach codegen with an `int`-allocated slot stored as `str`.
#[test]
fn a_value_reassignment_after_the_def_still_enforces_representation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::StringLiteral("leaked".to_string()),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0023");
    assert_eq!(
        err.message,
        "cannot assign `str` to `helper`, previously inferred as `int`"
    );
}

/// D-110's source-order `def` rebinding (PR #252's review caught the
/// initial gate rejecting this): a later `def` rebinds the name over an
/// earlier value binding, so the call resolves the function exactly
/// as CPython does (`helper = 1; def helper(): ...; helper()` prints
/// the function's result there).
#[test]
fn a_later_def_rebinds_over_an_earlier_value_binding() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

/// The same source-order `def` rebinding seen from a function body:
/// pass 3's final module environment reflects the net binding, so a
/// body call to a name whose value binding was cleared by a later
/// `def` resolves the function.
#[test]
fn a_body_call_resolves_when_a_later_def_rebinds_the_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "caller".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

/// And from the solver path: the accumulated globals clear a value
/// binding when the same-named `def` follows it, so an
/// inference-signature helper's body resolves the function.
#[test]
fn a_private_helper_resolves_when_a_later_def_rebinds_the_value() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
            },
            HirItem::Function {
                name: "_h".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![],
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check_and_resolve(&hir).is_ok());
}

/// #133's deliberate boundary with #22: pass 2 checks top-level code in
/// source order, so a call *before* the shadowing assignment still
/// resolves the function -- the binding simply does not exist yet at
/// that point. Only the already-active binding shadows.
#[test]
fn a_top_level_call_before_the_shadowing_assignment_still_resolves() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "helper".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "helper".to_string(),
                args: vec![],
            })),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "helper".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn infer_expr_in_rejects_call_before_def_in_top_level() {
    // Directly exercises the call-before-def check in infer_expr_in's
    // HirExpr::Call arm for top-level code (not inside a function body).
    let mut env = Environment::new();
    // Manually insert into functions without adding to defined_functions,
    // simulating a function that exists but hasn't been "executed" yet
    // in source order.
    Arc::make_mut(&mut env.functions).insert("foo".to_string(), (vec![], Ty::None));
    // Do NOT insert "foo" into defined_functions -- simulates a call
    // before the def's source-order position.
    let expr = HirExpr::Call {
        callee: "foo".to_string(),
        args: vec![],
    };
    let err = infer_expr_in(&env, &[], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot call function `foo` before its definition")
    );
}

#[test]
fn infer_expr_in_resolves_super_attr_get() {
    // Directly exercises the HirExpr::Super arm of infer_expr_in's
    // HirExpr::AttrGet branch (#433). The resolved member is a base
    // class *property* rather than an instance attribute: since #587
    // only class-level members resolve through `super()`, a property
    // is what proves this arm still routes to a successful
    // `resolve_super_attr_get` rather than only to its rejection.
    use pycc_hir::PropertyDef;
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    // #915: a receiver-less `super()` is now `C0001`, so this fixture binds
    // the `self` a real method body's environment carries.
    env.bind("self".to_string(), Ty::Instance(Box::new("B".to_string())));
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![("_x".to_string(), Ty::Int)],
            methods: vec![("__init__".to_string(), "A.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "A.x".to_string(),
                setter: None,
            }],
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
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
    env.bind_function(
        "A.x".to_string(),
        vec![Ty::Instance(Box::new("A".to_string()))],
        Ty::Int,
    );
    let expr = HirExpr::AttrGet {
        base: Box::new(HirExpr::Super),
        attr: "x".to_string(),
    };
    assert_eq!(infer_expr_in(&env, &[], &expr), Ok(Ty::Int));
}

#[test]
fn infer_expr_in_resolves_super_method_call() {
    // Directly exercises the HirExpr::Super arm of infer_expr_in's
    // HirExpr::MethodCall branch (#433), including the argument-
    // inference closure (`.map(|arg| infer_expr_in(...))`).
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    // #915: a receiver-less `super()` is now `C0001`, so this fixture binds
    // the `self` a real method body's environment carries.
    env.bind("self".to_string(), Ty::Instance(Box::new("B".to_string())));
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![],
            methods: vec![("add".to_string(), "A.add".to_string())],
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
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
    env.bind_function(
        "A.add".to_string(),
        vec![Ty::Instance(Box::new("A".to_string())), Ty::Int],
        Ty::Int,
    );
    let expr = HirExpr::MethodCall {
        base: Box::new(HirExpr::Super),
        method: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    assert_eq!(infer_expr_in(&env, &[], &expr), Ok(Ty::Int));
}

#[test]
fn infer_expr_in_super_method_call_propagates_arg_error() {
    // Exercises the `?` error-propagation path in the super().method()
    // argument-inference closure (`.collect::<Result<Vec<_>, _>>()?`).
    // An undefined name in the argument list causes `infer_expr_in` to
    // return `Err`, which the `?` operator propagates.
    let mut env = Environment::new();
    env.current_class = Some("B".to_string());
    env.bind_class(
        "A".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: vec![],
            mro: vec!["A".to_string()],
            attrs: vec![],
            methods: vec![("add".to_string(), "A.add".to_string())],
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
        "B".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "B".to_string(),
            bases: vec!["A".to_string()],
            mro: vec!["B".to_string(), "A".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
    env.bind_function(
        "A.add".to_string(),
        vec![Ty::Instance(Box::new("A".to_string())), Ty::Int],
        Ty::Int,
    );
    let expr = HirExpr::MethodCall {
        base: Box::new(HirExpr::Super),
        method: "add".to_string(),
        args: vec![HirExpr::Name("undefined_var".to_string())],
    };
    let err = infer_expr_in(&env, &[], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_local_first_assignment_does_not_inherit_the_globals_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::StringLiteral("global".to_string()),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_self_referential_first_local_assignment_cannot_read_the_global() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::BinOp {
                            op: BinOpKind::Add,
                            left: Box::new(HirExpr::Name("x".to_string())),
                            right: Box::new(HirExpr::IntLiteral(1)),
                        },
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn an_assignment_nested_in_if_classifies_the_name_as_function_local() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::Assign {
                            target: "x".to_string(),
                            value: HirExpr::IntLiteral(2),
                        }],
                        orelse: vec![],
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn a_nested_for_target_classifies_the_name_as_function_local() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "i".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("i".to_string())),
                    HirStmt::While {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::ForRange {
                            var: "i".to_string(),
                            start: HirExpr::IntLiteral(0),
                            stop: HirExpr::IntLiteral(1),
                            step: HirExpr::IntLiteral(1),
                            body: vec![],
                        }],
                    },
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.message, "local name `i` is not bound before this use");
}

#[test]
fn a_for_target_is_local_while_its_range_operands_are_evaluated() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "i".to_string(),
                value: HirExpr::IntLiteral(3),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Name("i".to_string()),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                }],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `i` is not bound before this use");
}

#[test]
fn private_helper_inference_rejects_a_read_before_local_assignment() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "_f".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![
                    HirStmt::ExprStmt(HirExpr::Name("x".to_string())),
                    HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "local name `x` is not bound before this use");
}

#[test]
fn check_function_the_public_api_still_has_no_sibling_visibility() {
    // `check_function` is a standalone entry point with no module
    // context, so it must keep working exactly as before: it only ever
    // sees its own signature (needed for recursion), never a sibling's.
    let function = HirItem::Function {
        name: "main".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "helper".to_string(),
            args: vec![],
        })],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_function_body_is_now_checked() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_bare_call_infers_none() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "print".to_string(),
        args: vec![],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn calling_an_undefined_function_is_a_clean_error() {
    let env = Environment::new();
    let expr = HirExpr::Call {
        callee: "undefined".to_string(),
        args: vec![],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("undefined"));
}

#[test]
fn calling_a_defined_function_infers_its_declared_return_type() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn a_wide_call_uses_the_heap_argument_type_buffer() {
    let mut env = Environment::new();
    env.bind_function("wide".to_string(), vec![Ty::Int; 5], Ty::Int);
    let expr = HirExpr::Call {
        callee: "wide".to_string(),
        args: vec![HirExpr::IntLiteral(1); 5],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn calling_a_function_with_a_bool_argument_for_an_int_parameter_succeeds() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::BoolLiteral(true)],
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn calling_a_function_with_the_wrong_number_of_arguments_is_a_clean_error() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("expects 2 argument"));
}

#[test]
fn an_int_argument_for_a_float_parameter_is_a_clean_error() {
    // D-086: pycc requires an explicit `float(...)` conversion at typed
    // boundaries (parameters, returns, assignments) rather than
    // following the Python typing spec's numeric-tower rule that `int`
    // is accepted wherever `float` is annotated. This is a deliberate
    // deviation from `mypy --strict` (which accepts this call), not an
    // oversight -- see D-086's rationale in
    // docs/decisions/D-086-two-type-boundary-strictness-decisions-equality.md.
    let mut env = Environment::new();
    env.bind_function("identity".to_string(), vec![Ty::Float], Ty::Float);
    let expr = HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("argument 1 of `identity` expects `float`, got `int`")
    );
}

#[test]
fn calling_a_function_with_a_wrong_typed_argument_is_a_clean_error() {
    let mut env = Environment::new();
    env.bind_function("add".to_string(), vec![Ty::Int, Ty::Int], Ty::Int);
    let expr = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::FloatLiteral(2.5)],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("argument 2")
            && err.message.contains("int")
            && err.message.contains("float")
    );
}

#[test]
fn a_wide_calls_later_inference_error_precedes_an_earlier_type_mismatch() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int; 5], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![
            HirExpr::StringLiteral("wrong".to_string()),
            HirExpr::IntLiteral(2),
            HirExpr::IntLiteral(3),
            HirExpr::IntLiteral(4),
            HirExpr::Name("undefined".to_string()),
        ],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `undefined` is not defined");
}

#[test]
fn calling_a_function_with_an_undefined_argument_propagates_the_error() {
    let mut env = Environment::new();
    env.bind_function("f".to_string(), vec![Ty::Int], Ty::None);
    let expr = HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_function_s_body_is_checked_against_its_declared_param_types() {
    let function = HirItem::Function {
        name: "add".to_string(),
        params: vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("a".to_string())),
            right: Box::new(HirExpr::Name("b".to_string())),
        }))],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_return_with_no_value_when_none_is_expected_succeeds() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_return_with_no_value_when_a_value_is_expected_is_a_clean_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn a_return_type_mismatch_is_a_clean_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn a_value_returning_function_must_return_on_every_path() {
    let function = HirItem::Function {
        name: "answer".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        })],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
    assert!(err.message.contains("can exit without returning"));
}

#[test]
fn an_if_with_returns_in_both_branches_satisfies_the_return_contract() {
    let function = HirItem::Function {
        name: "choose".to_string(),
        params: vec![("condition".to_string(), Ty::Bool)],
        return_ty: Ty::Int,
        body: vec![HirStmt::If {
            test: HirExpr::Name("condition".to_string()),
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn an_if_with_only_one_returning_branch_is_t0022() {
    let function = HirItem::Function {
        name: "choose".to_string(),
        params: vec![("condition".to_string(), Ty::Bool)],
        return_ty: Ty::Int,
        body: vec![HirStmt::If {
            test: HirExpr::Name("condition".to_string()),
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0022");
}

#[test]
fn a_return_whose_value_is_undefined_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name(
            "undefined".to_string(),
        )))],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn recursion_is_supported_since_the_function_s_own_signature_is_in_scope() {
    let function = HirItem::Function {
        name: "count".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "count".to_string(),
            args: vec![HirExpr::Name("n".to_string())],
        }))],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_function_s_if_while_and_for_bodies_are_checked_against_its_return_type() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::While {
                    test: HirExpr::BoolLiteral(true),
                    body: vec![HirStmt::ForRange {
                        var: "i".to_string(),
                        start: HirExpr::IntLiteral(0),
                        stop: HirExpr::IntLiteral(1),
                        step: HirExpr::IntLiteral(1),
                        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                    }],
                }],
                orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
            },
            HirStmt::Return(Some(HirExpr::IntLiteral(2))),
        ],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_bad_return_nested_in_if_while_and_for_is_still_caught() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(1),
                    step: HirExpr::IntLiteral(1),
                    body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                }],
            }],
            orelse: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
#[should_panic(expected = "check_function called with a non-Function HirItem")]
fn check_function_panics_on_a_non_function_item() {
    let _ = check_function(&HirItem::TopLevelStmt(HirStmt::Return(None)));
}

#[test]
fn an_if_s_test_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::Name("undefined".to_string()),
            body: vec![],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_s_orelse_ill_typed_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_in_a_function_whose_body_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
            ],
            orelse: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn an_if_with_assignments_in_a_function_whose_orelse_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_while_with_assignments_in_a_function_whose_body_has_an_error_propagates_via_clone_path() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::Name("undefined".to_string())),
            ],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_while_s_test_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::While {
            test: HirExpr::Name("undefined".to_string()),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_start_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::Name("undefined".to_string()),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_stop_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::Name("undefined".to_string()),
            step: HirExpr::IntLiteral(1),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_range_s_step_undefined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(3),
            step: HirExpr::Name("undefined".to_string()),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_binds_its_variable_as_the_list_element_type_in_a_function_body() {
    // Direct `check_stmt_in_function` call (matching this file's own
    // established pattern, e.g. the `AnnAssign` tests just above) rather
    // than `check_function`: a for-loop body might never execute (an
    // empty list), so `block_always_returns` always treats it as "does
    // not always return" (same as `ForRange`) -- a function whose only
    // statement is a `for` loop can never satisfy a non-`None` return
    // type, regardless of what's inside the loop body.
    let mut env = Environment::new();
    env.bind("xs".to_string(), Ty::List(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "i".to_string(),
        list: "xs".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("i".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["xs", "i", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("i"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("i"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn a_for_dict_loop_binds_its_variable_as_the_key_type_in_a_function_body() {
    // Mirrors the list-shaped test above (PR-11 Task 3, D-123): `for k in
    // d:` binds `k` as the dict's key type, not its value type.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let stmt = HirStmt::ForList {
        var: "k".to_string(),
        list: "d".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("k".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["d", "k", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("k"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("k"), Some(Ty::Str));
    assert_eq!(env.lookup_any("y"), Some(Ty::Str));
}

#[test]
fn a_for_set_loop_binds_its_variable_as_the_element_type_in_a_function_body() {
    // Mirrors the dict-shaped test above (PR-11 Task 7, D-123): `for x in
    // s:` binds `x` as the set's element type.
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let stmt = HirStmt::ForList {
        var: "x".to_string(),
        list: "s".to_string(),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    check_stmt_in_function(&mut env, &["s", "x", "y"], &stmt, Ty::None).unwrap();
    // Issue #118 Part 1: loop variable and body-only binding are Maybe.
    assert_eq!(env.lookup("x"), None);
    assert_eq!(env.lookup("y"), None);
    assert_eq!(env.lookup_any("x"), Some(Ty::Int));
    assert_eq!(env.lookup_any("y"), Some(Ty::Int));
}

#[test]
fn a_function_local_if_with_no_else_then_read_is_t0041() {
    // `def f(): if cond: x = 1; return x` -- T0041 (possibly-unbound
    // read in function body).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_function_local_while_then_read_is_t0041() {
    // `def f(): while cond: x = 1; return x` -- T0041.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::While {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_function_local_both_branches_if_then_read_succeeds() {
    // `def f(): if cond: x = 1 else: x = 2; return x` -- succeeds.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_local_for_range_then_read_is_t0041() {
    // `def f(): for i in range(3): x = i; return x` -- T0041 (the loop
    // body may execute zero times, so `x` is maybe-bound).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::Name("i".to_string()),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_function_local_for_range_var_then_read_is_t0041() {
    // `def f(): for i in range(3): pass; return i` -- T0041 (the loop
    // variable is maybe-bound after the loop).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_function_local_pre_bound_then_for_range_var_read_succeeds() {
    // `def f(): i = 0; for i in range(3): pass; return i` -- succeeds
    // because `i` was definitely bound before the loop.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::Assign {
                    target: "i".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(3),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_local_for_list_then_read_is_t0041() {
    // `def f(xs: list[int]): for i in xs: x = i; return x` -- T0041.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ForList {
                    var: "i".to_string(),
                    list: "xs".to_string(),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::Name("i".to_string()),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn a_function_local_pre_bound_for_list_var_survives_as_definite() {
    // `def f(xs: list[int]): i = 0; for i in xs: pass; return i` --
    // succeeds because `i` was definitely bound before the loop
    // (issue #118 Part 1).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::Assign {
                    target: "i".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::ForList {
                    var: "i".to_string(),
                    list: "xs".to_string(),
                    body: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("i".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_function_local_maybe_then_definite_upgrade_succeeds() {
    // `def f(c: bool): if c: x = 1; x = 2; return x` -- the maybe-bound
    // `x` from the if-body is upgraded to definite by the unconditional
    // `x = 2`, so the return read succeeds.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("c".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("c".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn a_for_list_s_list_not_defined_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "undefined".to_string(),
            body: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn a_for_list_s_list_read_before_a_later_local_assignment_is_unbound_local() {
    // `xs` is a declared local of this function (it's assigned further
    // down), so reading it in the `for` clause before that assignment
    // must be `unbound_local`, not "not defined" -- the same
    // is_local-aware distinction `HirExpr::Name` itself makes.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ForList {
                var: "i".to_string(),
                list: "xs".to_string(),
                body: vec![],
            },
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
        ],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn a_for_list_loop_over_a_non_list_value_in_a_function_body_is_rejected() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "x".to_string(),
            body: vec![],
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_for_list_loop_whose_body_statement_is_ill_typed_in_a_function_body_propagates_the_error() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(HirExpr::Name("undefined".to_string()))],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_for_list_loop_rejects_a_conflicting_reassignment_of_its_variable_in_a_function_body() {
    // Same as the module-scope sibling above: `check_assignment` itself
    // is what rejects rebinding an already-`str` parameter to the list's
    // `int` element type.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("i".to_string(), Ty::Str),
            ("xs".to_string(), Ty::List(Box::new(Ty::Int))),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![],
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

// -- PR-12 Task 3 (D-117): comprehension type-checking, function scope --

#[test]
fn a_list_comprehension_over_range_type_checks_in_a_function_body() {
    // Also pins that `var` is bound before `elt` is checked at function
    // scope (mirrors the module-scope sibling above): if the ordering
    // were wrong, `elt`'s reference to the loop variable would fail as
    // an unbound local.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_list_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_list_comprehension_producing_a_non_int_element_is_rejected_as_t0034_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::StringLiteral("x".to_string())),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0034");
    assert!(err.message.contains("list[str]"));
}

#[test]
fn a_set_comprehension_over_a_bare_set_parameter_type_checks_in_a_function_body() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Set` at function scope.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("s".to_string(), Ty::Set(Box::new(Ty::Int)))],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_20_x".to_string(),
            iter: CompIter::Name("s".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_20_x".to_string())),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_set_comprehension_producing_a_non_int_element_is_rejected_as_t0038_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::StringLiteral("x".to_string())),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0038");
    assert!(err.message.contains("set[str]"));
}

#[test]
fn a_set_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_dict_comprehension_over_a_bare_dict_parameter_type_checks_in_a_function_body() {
    // Exercises `resolve_comp_iter`'s `CompIter::Name` branch resolving
    // to `Ty::Dict` at function scope, binding `var` as the dict's key
    // type.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_20_k".to_string(),
            iter: CompIter::Name("d".to_string()),
            cond: None,
            key: Box::new(HirExpr::Name("0comp_20_k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_dict_comprehension_producing_a_non_str_int_pair_is_rejected_as_t0036_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::IntLiteral(1)),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0036");
    assert!(err.message.contains("dict[int, int]"));
}

#[test]
fn a_dict_comprehension_s_if_filter_of_a_non_bool_type_still_type_checks_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::StringLiteral("truthy".to_string()))),
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    check_function(&function).unwrap();
}

#[test]
fn a_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_comprehension_over_a_forward_referenced_iterable_is_rejected_as_unbound_local() {
    // Mirrors `a_for_list_loop_over_a_forward_referenced_list...`-style
    // behavior: `xs` is a genuine function-local name (assigned later in
    // the same body), so referencing it before that assignment is
    // `unbound_local`, not `not defined`.
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ListCompAssign {
                target: "y".to_string(),
                var: "0comp_11_i".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: None,
                elt: Box::new(HirExpr::IntLiteral(1)),
            },
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
        ],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

// Per-arm error-propagation pins (function scope): the same set as the
// module-scope block above, `check_stmt_in_function`'s own three arms.

#[test]
fn a_set_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_dict_comprehension_over_a_non_list_dict_set_iterable_is_rejected_as_t0033_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Name("x".to_string()),
            cond: None,
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    let err = check_function(&function).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("cannot be iterated"));
}

#[test]
fn a_list_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_set_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_dict_comprehension_rejects_a_conflicting_reassignment_of_its_loop_variable_in_a_function_body()
{
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![("0comp_11_i".to_string(), Ty::Str)],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0023");
}

#[test]
fn a_list_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            elt: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_if_filter_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("undefined".to_string()))),
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_list_comprehension_s_elt_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_set_comprehension_s_elt_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_key_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::Name("undefined".to_string())),
            value: Box::new(HirExpr::IntLiteral(1)),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn a_dict_comprehension_s_value_propagates_an_ill_typed_error_in_a_function_body() {
    let function = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::StringLiteral("k".to_string())),
            value: Box::new(HirExpr::Name("undefined".to_string())),
        }],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn private_identity_signature_is_inferred_from_its_call_site_and_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn check_and_resolve_takes_the_fast_path_for_an_already_concrete_valid_module() {
    // Pre-existing gap noticed while chasing this task's own 100%
    // coverage requirement: every other `check_and_resolve` test in this
    // file uses a `Ty::Infer` signature, so `checked_function_signatures_all`'s
    // fast-path *success* return (`concrete_function_signatures` is
    // `Some` and `check_with_signatures_all` succeeds) was never exercised.
    // Unrelated to this task's list-typing work, but cheap and in-scope
    // to close here rather than leave dangling.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "identity".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(resolved, hir);
}

#[test]
fn check_and_resolve_materializes_private_signatures_without_mutating_input() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }
    );
    assert_eq!(
        hir.items[0],
        HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
        }
    );
}

#[test]
fn annotated_int_result_propagates_back_to_a_private_binary_parameter() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }
    );
}

// ---- #574: string repetition through both consumers of
// `numeric_result_type` — the validation pass (`infer_expr_in`'s
// `HirExpr::BinOp` arm) and the private-helper constraint solver
// (`propagate_binop_constraints`). ----

#[test]
fn a_public_function_may_return_string_repetition() {
    // Validation-pass consumer: `check` never runs the solver for a
    // fully annotated module.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Int)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_public_function_may_return_string_repetition_with_the_count_first() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Bool)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::Name("count".to_string())),
                right: Box::new(HirExpr::StringLiteral("ab".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    assert!(check(&hir).is_ok());
}

#[test]
fn a_public_function_still_cannot_multiply_a_str_by_a_float() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Float)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_private_helper_infers_str_from_string_repetition() {
    // Constraint-solver consumer: the helper's parameter and return type
    // are both `Ty::Infer`, so `check_and_resolve` reaches
    // `propagate_binop_constraints`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    // Extracted with `assert!(matches!(..))` rather than a `let ... else
    // { unreachable!() }`, because the `else` arm would have been an
    // unexercised line and region under the D-014 gate while these tests
    // lived in `lib.rs`. This is the idiom the surrounding
    // private-inference tests already use.
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { params, return_ty, .. }
            if params == &vec![("count".to_string(), Ty::Int)] && *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_infers_str_with_the_count_on_the_left() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::Name("count".to_string())),
                    right: Box::new(HirExpr::StringLiteral("ab".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_infers_str_from_a_bool_repetition_count() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::BoolLiteral(true)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert!(matches!(
        &resolved.items[0],
        HirItem::Function { return_ty, .. } if *return_ty == Ty::Str
    ));
}

#[test]
fn a_private_helper_still_rejects_a_float_repetition_count() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::FloatLiteral(1.5)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_private_helper_still_rejects_subtracting_an_int_from_a_str() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Sub,
                    left: Box::new(HirExpr::StringLiteral("ab".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_repetition_over_an_undefined_operand_is_still_reported() {
    // Validation pass: repetition acceptance must not swallow operand
    // diagnostics raised before `numeric_result_type` ever sees a pair
    // of types.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "banner".to_string(),
            params: vec![("count".to_string(), Ty::Int)],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Mul,
                left: Box::new(HirExpr::Name("missing".to_string())),
                right: Box::new(HirExpr::Name("count".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("missing"));
}

#[test]
fn a_private_helper_repetition_over_an_undefined_operand_fails_closed() {
    // Solver path: an operand that never resolves leaves the helper's
    // return term unresolved, and `resolved_private_signature_term`
    // fails closed with "cannot infer return type" rather than
    // inventing a signature. Repetition does not change that.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_rep".to_string(),
                params: vec![("count".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(HirExpr::Name("missing".to_string())),
                    right: Box::new(HirExpr::Name("count".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_rep".to_string(),
                args: vec![HirExpr::IntLiteral(3)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer return type"));
    assert!(err.message.contains("_rep"));
}

#[test]
fn annotated_int_result_propagates_back_to_a_right_binary_parameter() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };

    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(
        resolved.items[0],
        HirItem::Function {
            name: "_inc".to_string(),
            params: vec![("value".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }
    );
}

#[test]
fn annotated_int_result_rejects_a_known_string_left_operand() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::StringLiteral("wrong".to_string())),
                right: Box::new(HirExpr::Name("value".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn annotated_int_result_rejects_a_known_string_right_operand() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_bad".to_string(),
            params: vec![("value".to_string(), Ty::Infer)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("value".to_string())),
                right: Box::new(HirExpr::StringLiteral("wrong".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_parameter_is_inferred_by_forwarding_into_an_annotated_callee() {
    // Regression test (self-review finding, pre-merge): the solver used
    // to only unify a call argument against a callee's parameter when
    // the callee's own parameter term was itself unresolved. When the
    // callee is fully annotated (its parameter term is already
    // `Ok(Ty::Int)`), an unresolved *caller* argument variable never got
    // constrained in that direction, even though `unify_terms` itself
    // already supports it symmetrically -- so `_forward` below used to
    // fail with a spurious "add an annotation" T0021 instead of
    // correctly inferring `x: int` from forwarding into `_sink`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "_sink".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_binary_helper_signature_is_inferred_across_operator_constraints() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_add".to_string(),
                params: vec![
                    ("left".to_string(), Ty::Infer),
                    ("right".to_string(), Ty::Infer),
                ],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("left".to_string())),
                    right: Box::new(HirExpr::Name("right".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_add".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_true_division_helper_infers_a_float_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_ratio".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Div,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::IntLiteral(2)),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ratio".to_string(),
                value: HirExpr::Call {
                    callee: "_ratio".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_helper_without_a_return_infers_none() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_log".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::FString(vec![
                    FStringPart::Literal("equal=".to_string()),
                    FStringPart::Interpolation(Box::new(HirExpr::Compare {
                        op: CmpOpKind::Eq,
                        left: Box::new(HirExpr::IntLiteral(1)),
                        right: Box::new(HirExpr::IntLiteral(1)),
                    })),
                ])],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_helper_with_a_bare_return_infers_none() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_stop".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(None)],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_constant_helper_infers_a_float_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_constant".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::FloatLiteral(1.5)))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn private_range_helper_infers_its_parameter_as_int() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_loop".to_string(),
            params: vec![("limit".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Name("limit".to_string()),
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::While {
                    test: HirExpr::BoolLiteral(false),
                    body: vec![HirStmt::If {
                        test: HirExpr::BoolLiteral(true),
                        body: vec![HirStmt::ExprStmt(HirExpr::Call {
                            callee: "print".to_string(),
                            args: vec![HirExpr::Name("item".to_string())],
                        })],
                        orelse: vec![],
                    }],
                }],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    check(&hir).unwrap();
}

#[test]
fn unresolved_private_parameter_requests_an_annotation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_constant".to_string(),
            params: vec![("unused".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("parameter `unused`"));
}

#[test]
fn unresolved_private_return_requests_an_annotation() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("missing".to_string())))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("return type"));
}

#[test]
fn undefined_call_cannot_silently_resolve_a_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Call {
                callee: "missing".to_string(),
                args: vec![],
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_binary_operand_cannot_silently_resolve_a_private_return() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_unknown".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("missing".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_private_binary_parameters_request_annotations() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_add".to_string(),
            params: vec![
                ("left".to_string(), Ty::Infer),
                ("right".to_string(), Ty::Infer),
            ],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("left".to_string())),
                right: Box::new(HirExpr::Name("right".to_string())),
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn unresolved_call_argument_does_not_invent_a_private_parameter_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::Name("missing".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_parameter_inference_rejects_conflicting_call_sites() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::StringLiteral("one".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_return_inference_rejects_conflicting_return_types() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_choose".to_string(),
            params: vec![("condition".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::If {
                test: HirExpr::Name("condition".to_string()),
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
                orelse: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
                    "one".to_string(),
                )))],
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0022");
}

fn nested_private_call_conflict() -> HirExpr {
    HirExpr::Call {
        callee: "_sink".to_string(),
        args: vec![HirExpr::Call {
            callee: "_identity".to_string(),
            args: vec![HirExpr::StringLiteral("wrong".to_string())],
        }],
    }
}

fn private_constraint_error_fixture(stmt: HirStmt) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_identity".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
            },
            HirItem::Function {
                name: "_sink".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![HirStmt::Return(None)],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
            HirItem::Function {
                name: "_probe".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![stmt],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

#[test]
fn private_f_string_and_return_propagate_nested_constraint_errors() {
    let stmt = HirStmt::Return(Some(HirExpr::FString(vec![FStringPart::Interpolation(
        Box::new(nested_private_call_conflict()),
    )])));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_base_wrapping_a_conflict_is_rejected_end_to_end() {
    // PR-12 Task 7 (D-118). NOTE what this test does and does NOT pin:
    // it proves `check()`'s real, end-to-end pipeline still rejects a
    // nested private-helper call conflict when it sits inside a
    // `Slice`'s `base`, *inside a private helper's own body* (not just
    // at module top level, which the pair above this whole group
    // already covers) -- a real and valuable thing to test on its own.
    // It does NOT, on its own, prove that `collect_expr_constraints`'s
    // `Slice` arm itself recurses into `base`: `check_with_environment_all`'s
    // Pass 3 independently re-type-checks every function body via
    // `infer_expr_in` regardless of what the solver's own
    // constraint-collection pass did or didn't visit, so this exact
    // conflict is still caught by Pass 3 even with the solver arm's
    // recursion mutated away to a bare `Ok(None)` no-op (confirmed by
    // temporarily making that mutation and rerunning this test: it kept
    // passing). `private_slice_base_recursion_pins_an_otherwise_unconstrained_parameter`
    // below is the test that actually is sensitive to the solver arm's
    // own recursion, using a construction where Pass 3 cannot mask a
    // missing visit (a `Ty::Infer` parameter whose *only* constraining
    // use anywhere in the module is inside a `Slice`'s `base`, so
    // signature resolution itself -- which gates whether Pass 3 ever
    // runs at all -- depends on the solver reaching it).
    let stmt = HirStmt::Return(Some(HirExpr::Slice {
        base: Box::new(nested_private_call_conflict()),
        start: None,
        stop: None,
        step: None,
    }));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_bound_wrapping_a_conflict_is_rejected_end_to_end() {
    // Same as above, but the conflict is nested inside a bound
    // (`start`), not `base`. Same caveat applies: this alone doesn't
    // pin the solver arm's own recursion (Pass 3 masks it identically),
    // see `private_slice_start_bound_recursion_pins_an_otherwise_unconstrained_parameter`
    // below for the test that actually does.
    let stmt = HirStmt::Return(Some(HirExpr::Slice {
        base: Box::new(HirExpr::IntLiteral(1)),
        start: Some(Box::new(nested_private_call_conflict())),
        stop: None,
        step: None,
    }));
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_slice_base_recursion_pins_an_otherwise_unconstrained_parameter() {
    // Load-bearing counterpart to the pair above (added after review:
    // those two tests' original doc comments overclaimed what they
    // pinned -- Pass 3 independently re-type-checks every function body
    // regardless of solver recursion, so a genuine conflict is caught
    // either way; verified by temporarily gutting the `Slice` arm in
    // `collect_expr_constraints` to a bare `Ok(None)` and confirming
    // those two tests still passed).
    //
    // This test's construction is different in a way that matters:
    // `_forward`'s own parameter `xs` is `Ty::Infer`, and its *only*
    // appearance anywhere in this module is wrapped in a call
    // (`_id_list(xs)`) embedded in a `Slice`'s `base`. Nothing else ever
    // constrains `xs`'s type. `collect_expr_constraints`'s `Call` arm is
    // the one place that performs real unification (`unify_terms`)
    // between an argument's term and the callee's own (here: concretely
    // annotated) parameter term -- so `xs`'s term becomes concretely
    // `list[int]` if and only if the solver actually visits
    // `_id_list(xs)`, which only happens if the outer `Slice` arm
    // recurses into `base`.
    //
    // If that recursion is missing, `xs`'s term never resolves, and
    // `infer_function_signatures_with_solver_all`'s own final resolution
    // loop raises "cannot infer type of parameter `xs`... add an
    // annotation" *before `check_with_signatures_all`/Pass 3 ever runs for
    // this function* -- Pass 3 cannot mask a failure that happens
    // before Pass 3 is even reached. Confirmed empirically: gutting the
    // `Slice` arm to a bare `Ok(None)` flips this test from `Ok(())` to
    // exactly that `Err`; restoring the real arm makes it pass again.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_id_list".to_string(),
                params: vec![("v".to_string(), Ty::List(Box::new(Ty::Int)))],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("xs".to_string(), Ty::Infer)],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Slice {
                    base: Box::new(HirExpr::Call {
                        callee: "_id_list".to_string(),
                        args: vec![HirExpr::Name("xs".to_string())],
                    }),
                    start: Some(Box::new(HirExpr::IntLiteral(1))),
                    stop: None,
                    step: None,
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn private_slice_start_bound_recursion_pins_an_otherwise_unconstrained_parameter() {
    // Same load-bearing shape as the test above, but the embedded call
    // sits in a bound (`start`) rather than `base`: `_forward`'s own
    // parameter `x` is `Ty::Infer` and is used *only* wrapped in
    // `_id(x)` inside the slice's `start` bound. A bare `Name("x")`
    // bound would NOT be load-bearing here -- looking up a name in
    // `collect_expr_constraints` performs no unification of its own,
    // only the `Call` arm does, which is why this test wraps `x` in a
    // call rather than mirroring the brief-review's own bare `xs[x:]`
    // sketch literally.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_id".to_string(),
                params: vec![("v".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
            },
            HirItem::Function {
                name: "_forward".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::List(Box::new(Ty::Int)),
                body: vec![HirStmt::Return(Some(HirExpr::Slice {
                    base: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
                    start: Some(Box::new(HirExpr::Call {
                        callee: "_id".to_string(),
                        args: vec![HirExpr::Name("x".to_string())],
                    })),
                    stop: None,
                    step: None,
                }))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn private_if_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
        orelse: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_while_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_operand_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ForRange {
        var: "item".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: nested_private_call_conflict(),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_body_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ForRange {
        var: "item".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(nested_private_call_conflict())],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_compare_left_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(nested_private_call_conflict()),
        right: Box::new(HirExpr::IntLiteral(1)),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_compare_right_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(nested_private_call_conflict()),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_binary_left_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(nested_private_call_conflict()),
        right: Box::new(HirExpr::IntLiteral(1)),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_binary_right_propagates_nested_constraint_errors() {
    let stmt = HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::IntLiteral(1)),
        right: Box::new(nested_private_call_conflict()),
    });
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_assignment_propagates_nested_constraint_errors() {
    let stmt = HirStmt::Assign {
        target: "value".to_string(),
        value: nested_private_call_conflict(),
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_assignment_with_an_unresolved_value_is_checked_after_inference() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_assign".to_string(),
            params: vec![],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Assign {
                target: "value".to_string(),
                value: HirExpr::Name("missing".to_string()),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_if_test_propagates_nested_constraint_errors() {
    let stmt = HirStmt::If {
        test: nested_private_call_conflict(),
        body: vec![],
        orelse: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_while_test_propagates_nested_constraint_errors() {
    let stmt = HirStmt::While {
        test: nested_private_call_conflict(),
        body: vec![],
    };
    assert_eq!(
        check(&private_constraint_error_fixture(stmt))
            .unwrap_err()
            .code,
        "T0021"
    );
}

#[test]
fn private_range_parameter_rejects_a_conflicting_call_site_type() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_loop".to_string(),
                params: vec![("limit".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::ForRange {
                    var: "item".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Name("limit".to_string()),
                    step: HirExpr::IntLiteral(1),
                    body: vec![],
                }],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_loop".to_string(),
                args: vec![HirExpr::StringLiteral("wrong".to_string())],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_implicit_none_return_rejects_an_int_constrained_call_site() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_noop".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "_noop".to_string(),
                    args: vec![],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0022");
}

#[test]
fn private_division_return_rejects_an_int_constrained_call_site() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_ratio".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Div,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::IntLiteral(2)),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ForRange {
                var: "item".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "_ratio".to_string(),
                    args: vec![HirExpr::IntLiteral(4)],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![],
            }),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn private_binary_constraint_rejects_incompatible_resolved_operands() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "_bad_add".to_string(),
                params: vec![("value".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("value".to_string())),
                    right: Box::new(HirExpr::StringLiteral("wrong".to_string())),
                }))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "_bad_add".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
}

#[test]
fn direct_check_function_rejects_an_unresolved_private_signature() {
    let function = HirItem::Function {
        name: "_identity".to_string(),
        params: vec![("value".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Name("value".to_string())))],
    };
    assert_eq!(check_function(&function).unwrap_err().code, "T0021");
}

#[test]
fn type_solver_covers_concrete_and_union_merge_paths() {
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    assert!(
        !unify_terms(
            Ok(Ty::Int),
            Ok(Ty::Bool),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert!(
        unify_terms(
            Ok(Ty::Int),
            Ok(Ty::Str),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .is_err()
    );

    let empty_left = fresh_term(&mut parents, &mut concrete);
    let empty_right = fresh_term(&mut parents, &mut concrete);
    assert!(
        unify_terms(
            empty_left.clone(),
            empty_right.clone(),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert!(
        !unify_terms(
            empty_left,
            empty_right,
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );

    let typed_left = fresh_term(&mut parents, &mut concrete);
    let typed_right = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        typed_left.clone(),
        Ok(Ty::Bool),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed_right.clone(),
        Ok(Ty::Int),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed_left,
        typed_right.clone(),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert_eq!(
        resolved_term(typed_right, &mut parents, &concrete),
        Some(Ty::Int)
    );

    let typed = fresh_term(&mut parents, &mut concrete);
    let empty = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        typed.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        typed,
        empty.clone(),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert_eq!(resolved_term(empty, &mut parents, &concrete), Some(Ty::Str));

    let conflicting_left = fresh_term(&mut parents, &mut concrete);
    let conflicting_right = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        conflicting_left.clone(),
        Ok(Ty::Int),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    unify_terms(
        conflicting_right.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0021",
        "test",
    )
    .unwrap();
    assert!(
        unify_terms(
            conflicting_left,
            conflicting_right,
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .is_err()
    );

    let reversed = fresh_term(&mut parents, &mut concrete);
    assert!(
        unify_terms(
            Ok(Ty::Float),
            reversed.clone(),
            &mut parents,
            &mut concrete,
            "T0021",
            "test",
        )
        .unwrap()
    );
    assert_eq!(
        resolved_term(reversed, &mut parents, &concrete),
        Some(Ty::Float)
    );
}

/// #949: with a known declared type, `unify_terms_with_declared` reports a
/// declared-vs-actual mismatch instead of the ambiguous "conflicting inferred
/// types" wording, and picks the actual operand by comparing against the
/// declared type rather than by position -- the `(Ok, Ok)` arm passes the
/// declared type first, while the `(Err(var), Ok(ty)) | (Ok(ty), Err(var))`
/// arm passes the already-inferred actual first and the declared type second.
#[test]
fn unify_terms_with_declared_names_the_declared_type_in_either_operand_order() {
    let mut parents = Vec::new();
    let mut concrete = Vec::new();

    // `(Ok, Ok)`: declared is the left operand.
    let declared_first = unify_terms_with_declared(
        Ok(Ty::Int),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0022",
        "declared return type",
        Some(&Ty::Int),
    )
    .unwrap_err();
    assert_eq!(declared_first.code, "T0022");
    assert_eq!(
        declared_first.message,
        "return type mismatch: expected `int`, found `str`"
    );
    assert_eq!(declared_first.help.as_deref(), Some("return a `int` value"));

    // `(Ok(ty), Err(var))` with the variable already resolved to `str`: the
    // conflict is raised with the *actual* type first, so a positional
    // assumption would invert the message.
    let inferred = fresh_term(&mut parents, &mut concrete);
    unify_terms(
        inferred.clone(),
        Ok(Ty::Str),
        &mut parents,
        &mut concrete,
        "T0022",
        "private helper return type",
    )
    .unwrap();
    let declared_second = unify_terms_with_declared(
        Ok(Ty::Int),
        inferred.clone(),
        &mut parents,
        &mut concrete,
        "T0022",
        "declared return type",
        Some(&Ty::Int),
    )
    .unwrap_err();
    assert_eq!(
        declared_second.message,
        "return type mismatch: expected `int`, found `str`"
    );
    assert_eq!(
        declared_second.help.as_deref(),
        Some("return a `int` value")
    );

    // Without a declared type the original ambiguous wording is unchanged,
    // including its operand order, and carries no `help` (D-152 leaves
    // genuinely ambiguous conflicts unpopulated).
    let ambiguous = unify_terms(
        Ok(Ty::Int),
        inferred,
        &mut parents,
        &mut concrete,
        "T0022",
        "private helper return type",
    )
    .unwrap_err();
    assert_eq!(
        ambiguous.message,
        "private helper return type: conflicting inferred types `str` and `int`"
    );
    assert_eq!(ambiguous.help, None);
}

fn generic_identity_fn(param_ty: Ty, return_ty: Ty) -> HirItem {
    HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), param_ty)],
        return_ty,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    }
}

#[test]
fn check_generic_function_accepts_consistent_single_type_param() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    assert!(check_generic_function(&func).is_ok());
}

#[test]
fn check_generic_function_rejects_scalar_specific_op_on_bare_param() {
    // `def f[T](x: T) -> T: return x + 1` -- `T` is opaque, so `+` on a
    // bare `Ty::Param` value must fail exactly like any other
    // type-incompatible operand pair, reusing the existing `T0021`
    // numeric-operator diagnostic (`numeric_result_type`'s own
    // "operator ... not defined for ..." arm), not a new code.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("x".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_generic_function_rejects_returning_own_param_as_a_concrete_scalar() {
    // `def bad[T](x: T) -> int: return x` -- a generic function returning
    // its *own* `Ty::Param` as a concrete scalar must be rejected with
    // T0022: after monomorphization `T` is substituted with the call-site
    // scalar (e.g. `str`), which would not match the declared `int` return
    // type and panic at codegen. `is_assignable`'s `from == Ty::Param`
    // clause lets this pass structurally (the clause is needed for
    // non-generic functions reading generic class instance attributes),
    // so `check_stmt_in_function`'s `Return` arm narrows it via the
    // threaded `own_type_param` context.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "bad".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn check_generic_function_rejects_two_distinct_type_parameters() {
    // Defense in depth (`crates/pycc_hir`'s own frontend arity gate
    // already prevents this from real source, Task 1): a
    // hand-constructed `HirItem` with two distinct `Ty::Param` names
    // across its signature must still be rejected here.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_container_position_type_parameter() {
    // Defense in depth, same rationale as the two-type-parameter test
    // above. `crates/pycc_hir`'s `annotation_to_ty` does lower a
    // `list[T]`-shaped annotation since D-228 (issue #918), but it
    // rejects a `Ty::Param` element there itself, with this same `T0042`
    // code and wording and a real span -- precisely because
    // `substitute_ty` is not recursive. So a container-position type
    // parameter still cannot reach this function from real source, and
    // this can only be exercised via a hand-built `HirItem`.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_substitutes_int_and_mangles_name() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let instantiation = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    assert_eq!(instantiation.mangled_name, "0gen_identity__T_int");
    assert_eq!(instantiation.return_ty, Ty::Int);
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_identity__T_int".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }
    );
}

#[test]
fn instantiate_generic_call_substitutes_str_at_a_different_call_site() {
    // Two call sites resolving `T` to different concrete types must
    // produce two independent, differently-mangled specializations
    // (D-134's own monomorphization requirement) -- and the same
    // concrete type from two call sites must produce the identical
    // mangled name and body, which is what Task 3 dedupes on.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let str_instantiation = instantiate_generic_call(&func, &[Ty::Str]).unwrap();
    assert_eq!(str_instantiation.mangled_name, "0gen_identity__T_str");
    assert_eq!(str_instantiation.return_ty, Ty::Str);

    let int_instantiation_1 = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    let int_instantiation_2 = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    assert_eq!(
        int_instantiation_1.mangled_name,
        int_instantiation_2.mangled_name
    );
    assert_eq!(int_instantiation_1, int_instantiation_2);
}

#[test]
fn instantiate_generic_call_substitutes_nested_ann_assign_annotation() {
    // Exercises the recursive body-substitution walk itself: a nested
    // `AnnAssign` inside an `If` body is the one HIR shape that carries
    // an embedded `Ty` beyond the function's own signature, and must be
    // substituted too, not just `params`/`return_ty`. Compares the whole
    // specialized item by value (rather than destructuring it with a
    // `let-else`/`unreachable!()`, which the coverage gate would
    // otherwise always flag as an untaken branch when this test still
    // lived in `lib.rs`).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param.clone(),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "y".to_string(),
                annotation: param,
                value: None,
            }],
            orelse: vec![],
        }],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Bool]).unwrap();
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_bool".to_string(),
            params: vec![("x".to_string(), Ty::Bool)],
            return_ty: Ty::Bool,
            body: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Bool,
                    value: None,
                }],
                orelse: vec![],
            }],
        }
    );
}

#[test]
fn instantiate_generic_call_substitutes_nested_while_forrange_forlist_bodies() {
    // Exercises the remaining recursive-body-walk arms (`While`,
    // `ForRange`, `ForList`) that the `If` test above doesn't reach --
    // each must recurse into its own nested `body` to find and
    // substitute a further-nested `AnnAssign`.
    let param = Ty::Param(Box::new("T".to_string()));
    let nested = |marker: &str| {
        vec![HirStmt::AnnAssign {
            is_final: false,
            target: marker.to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }]
    };
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![
            HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: nested("w"),
            },
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: nested("r"),
            },
            HirStmt::ForList {
                var: "e".to_string(),
                list: "xs".to_string(),
                body: nested("l"),
            },
        ],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    let expected_nested = |marker: &str| {
        vec![HirStmt::AnnAssign {
            is_final: false,
            target: marker.to_string(),
            annotation: Ty::Int,
            value: None,
        }]
    };
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_int".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::While {
                    test: HirExpr::BoolLiteral(true),
                    body: expected_nested("w"),
                },
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(1),
                    step: HirExpr::IntLiteral(1),
                    body: expected_nested("r"),
                },
                HirStmt::ForList {
                    var: "e".to_string(),
                    list: "xs".to_string(),
                    body: expected_nested("l"),
                },
            ],
        }
    );
}

#[test]
#[should_panic(expected = "instantiate_generic_call called with a non-Function HirItem")]
fn instantiate_generic_call_panics_on_a_non_function_item() {
    let _ = instantiate_generic_call(&HirItem::TopLevelStmt(HirStmt::Return(None)), &[]);
}

#[test]
#[should_panic(expected = "check_function called with a non-Function HirItem")]
fn check_generic_function_skips_the_shape_gate_for_a_top_level_statement() {
    // `check_generic_function`'s own shape gate only applies to a
    // `HirItem::Function` -- a `TopLevelStmt` item has no signature to
    // scan for a type parameter at all, so it skips straight to
    // `check_function`, which still panics on a non-`Function` item
    // exactly as it always has (see `check_function_panics_on_a_non_function_item`
    // above): this test's own job is only to prove the `if let` shape
    // gate itself doesn't reject a `TopLevelStmt` some other way first.
    let item = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::IntLiteral(1)));
    let _ = check_generic_function(&item);
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_dict_key() {
    // Defense in depth: a hand-built `dict[T, str]`-shaped parameter,
    // covering `scan_signature_ty_for_param`'s `Ty::Dict` arm on its
    // key position (the `?` on the key's own recursive call).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "d".to_string(),
            Ty::Dict(Box::new((Ty::Param(Box::new("T".to_string())), Ty::Str))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_dict_value() {
    // Covers the dict value position too (the key succeeds, so this
    // exercises the `Ty::Dict` arm's second recursive call instead of
    // its first).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "d".to_string(),
            Ty::Dict(Box::new((Ty::Str, Ty::Param(Box::new("T".to_string()))))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_set() {
    // Defense in depth: covers `scan_signature_ty_for_param`'s
    // `Ty::List(elem) | Ty::Set(elem)` arm's `Set` alternative
    // specifically (the `List` case is already covered by
    // `check_generic_function_rejects_container_position_type_parameter`
    // above).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::Set(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_container_position_type_parameter_in_the_return_type() {
    // Every other container-position test above puts the offending
    // shape in a parameter; `generic_type_param_name` scans
    // `return_ty` too (via its own separate call), which needs its own
    // test to cover that specific call site's error path.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_tuple_element() {
    // Defense in depth: covers `scan_signature_ty_for_param`'s
    // `Ty::Tuple` arm.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "t".to_string(),
            Ty::Tuple(Box::new(vec![
                Ty::Int,
                Ty::Param(Box::new("T".to_string())),
            ])),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_accepts_a_non_generic_tuple_typed_parameter() {
    // `check_generic_function` scans every function's signature, not
    // just an actually-generic one -- a plain, fully concrete
    // `Ty::Tuple` parameter with no `Ty::Param` anywhere inside it must
    // scan clean, exercising `scan_signature_ty_for_param`'s `Ty::Tuple`
    // arm's own success path (its `for` loop finishing without any
    // element raising `T0042`).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])))],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    assert!(check_generic_function(&func).is_ok());
}

#[test]
fn instantiate_generic_call_leaves_a_concrete_sibling_parameter_unchanged() {
    // A generic function can mix a `T`-typed parameter with an
    // ordinary concrete-typed one (see the assignability-rejection
    // test above for the failure path); when the call site's argument
    // for that concrete parameter is compatible, `substitute_ty` must
    // still run on it and simply clone it through unchanged, since it
    // carries no `Ty::Param` occurrence to substitute.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Str, Ty::Int]).unwrap();
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_str".to_string(),
            params: vec![("x".to_string(), Ty::Str), ("n".to_string(), Ty::Int),],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }
    );
}

#[test]
fn instantiate_generic_call_rejects_inconsistent_call_site_substitution() {
    // `def f[T](x: T, y: T) -> T` called as `f(1, "a")` -- both
    // occurrences of `T` must agree (D-134's own named example).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), param.clone()),
            ("y".to_string(), param.clone()),
        ],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Str]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_non_scalar_call_site_argument() {
    // A call site whose argument type used to resolve `T` is not one
    // of the four scalars is D-134's own named rejection case (e.g.
    // passing a `list[int]` value where `T` is inferred).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let err = instantiate_generic_call(&func, &[Ty::List(Box::new(Ty::Int))]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_wrong_arity() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn instantiate_generic_call_rejects_unresolvable_type_parameter() {
    // `T` appears only in the return type, never in any parameter --
    // no call-site argument can resolve it.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_non_generic_function() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_checks_non_generic_parameter_assignability() {
    // A generic function can mix a `T`-typed parameter with an
    // ordinary concrete-typed one; the concrete one still gets
    // checked for assignability using the existing `is_assignable`
    // (and the existing `T0021` "argument expects" message shape),
    // not silently skipped.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Str]).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn instantiate_generic_call_rejects_two_distinct_type_parameters() {
    // Defense in depth, mirrors `check_generic_function`'s own test.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_container_position_type_parameter() {
    // Defense in depth, mirrors `check_generic_function`'s own test.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = instantiate_generic_call(&func, &[Ty::List(Box::new(Ty::Int))]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

// PR-13 Task 3 (D-133/D-134): full pipeline wiring -- `check`/
// `check_and_resolve` dispatching a generic call site through
// `instantiate_generic_call`, and `check_and_resolve`'s own
// `monomorphize` pass rewriting call sites and dropping/appending
// items so `pycc_mir::build` only ever sees ordinary concrete
// functions.

fn find_function<'a>(module: &'a HirModule, name: &str) -> Option<&'a HirItem> {
    module
        .items
        .iter()
        .find(|item| matches!(item, HirItem::Function { name: n, .. } if n == name))
}

fn count_function(module: &HirModule, name: &str) -> usize {
    module
        .items
        .iter()
        .filter(|item| matches!(item, HirItem::Function { name: n, .. } if n == name))
        .count()
}

#[test]
fn check_alone_type_checks_a_module_containing_a_valid_generic_function() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn check_reports_the_same_generic_diagnostic_as_check_and_resolve() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![func],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
}

#[test]
fn check_and_resolve_rejects_a_generic_call_site_with_the_wrong_arity() {
    // Exercises `infer_expr_in`'s own generic-dispatch arm propagating
    // an `instantiate_generic_call` error (as opposed to
    // `check_generic_function`'s shape gate, covered by the sibling
    // test above) through both public entry points.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity.clone(), top.clone()],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0021");
}

#[test]
fn check_and_resolve_rejects_a_type_parameter_nested_in_a_parameter_container() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![func],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
}

#[test]
fn ty_contains_param_detects_a_param_nested_in_every_container_shape() {
    let t = Ty::Param(Box::new("T".to_string()));
    assert!(ty_contains_param(&t));
    assert!(ty_contains_param(&Ty::List(Box::new(t.clone()))));
    assert!(ty_contains_param(&Ty::Set(Box::new(t.clone()))));
    assert!(ty_contains_param(&Ty::Dict(Box::new((Ty::Str, t.clone())))));
    assert!(ty_contains_param(&Ty::Dict(Box::new((t.clone(), Ty::Int)))));
    assert!(ty_contains_param(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        t.clone()
    ]))));
    assert!(!ty_contains_param(&Ty::Int));
    assert!(!ty_contains_param(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        Ty::Str
    ]))));
}

#[test]
fn is_generic_signature_detects_a_param_nested_in_a_parameter_container() {
    assert!(is_generic_signature(
        &[(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string()))))
        )],
        &Ty::None,
    ));
    assert!(is_generic_signature(
        &[],
        &Ty::List(Box::new(Ty::Param(Box::new("T".to_string()))))
    ));
    assert!(!is_generic_signature(
        &[("x".to_string(), Ty::Int)],
        &Ty::Int
    ));
}

#[test]
fn check_and_resolve_monomorphizes_a_nested_generic_call_and_drops_the_original() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());
    assert!(find_function(&resolved, "0gen_identity__T_int").is_some());
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    // Compared against the whole expected statement (not a
    // `let-else { panic!() }` destructure) so the never-taken failure
    // arm was not its own uncovered branch while these tests lived in
    // `lib.rs` -- mirrors the convention described in Task 2's own note
    // on `unreachable!()` in test bodies.
    assert_eq!(
        resolved.items[0],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_int".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }],
        }))
    );
}

#[test]
fn monomorphize_lets_a_non_generic_function_read_a_module_global_defined_later_in_the_file_alongside_a_generic_function()
 {
    // Fix-round regression test (finding #1): `check` (via
    // `check_with_environment_all`'s three-pass D-041 discipline) and
    // `check_and_resolve`/`monomorphize` must agree on validity for a
    // module that mixes a generic function with a non-generic function
    // reading a module-level global assigned *after* that function's
    // own `def` -- Python only evaluates a function body when called,
    // typically after the whole module has already run top to bottom.
    // Before this fix, `monomorphize` walked `hir.items` in a single
    // source-order pass, so `uses_global`'s body was rewritten before
    // the later `g: int = 5` top-level assignment had grown `env`,
    // wrongly reporting `g` as undefined even though `check` (using the
    // correct two-phase order) already accepted the exact same program.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let uses_global = HirItem::Function {
        name: "uses_global".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("g".to_string())))],
    };
    let global_assign = HirItem::TopLevelStmt(HirStmt::AnnAssign {
        is_final: false,
        target: "g".to_string(),
        annotation: Ty::Int,
        value: Some(HirExpr::IntLiteral(5)),
    });
    let call_uses_global = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "uses_global".to_string(),
            args: vec![],
        }],
    }));
    let call_identity = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            identity,
            uses_global.clone(),
            global_assign,
            call_uses_global,
            call_identity,
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // `check` accepts this module (three-pass discipline lets
    // `uses_global` see `g` regardless of source position).
    assert!(check(&hir).is_ok());
    // `check_and_resolve`/`monomorphize` must accept it too, and must
    // not have dropped or mis-rewritten the non-generic function.
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(find_function(&resolved, "uses_global"), Some(&uses_global));
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_monomorphizes_two_call_sites_at_different_concrete_types_into_distinct_specializations()
 {
    // Fix-round regression test (finding #3): the actual monomorphization
    // crux is that two call sites at *different* concrete types produce
    // two distinct, correctly-routed specializations -- not the same
    // specialization reused, and not swapped between call sites.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let call_int = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let call_str = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::StringLiteral("s".to_string())],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, call_int, call_str],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());

    let int_specialization = find_function(&resolved, "0gen_identity__T_int")
        .expect("an `int` specialization must exist");
    let str_specialization = find_function(&resolved, "0gen_identity__T_str")
        .expect("a `str` specialization must exist");
    assert_ne!(int_specialization, str_specialization);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);

    // Each call site's rewritten callee must point to its own
    // specialization -- not the same one, not swapped. The original
    // generic `identity` def (index 0) is dropped entirely, so the two
    // top-level call statements shift down to indices 0 and 1.
    assert_eq!(
        resolved.items[0],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_int".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }],
        }))
    );
    assert_eq!(
        resolved.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "0gen_identity__T_str".to_string(),
                args: vec![HirExpr::StringLiteral("s".to_string())],
            }],
        }))
    );
}

#[test]
fn check_and_resolve_dedupes_two_call_sites_with_the_same_concrete_type() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let use_twice = HirItem::Function {
        name: "use_twice".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![
            HirStmt::Assign {
                target: "a".to_string(),
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            },
            HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            },
            HirStmt::Return(Some(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("a".to_string())),
                right: Box::new(HirExpr::Name("b".to_string())),
            })),
        ],
    };
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(3)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, use_twice, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_monomorphizes_a_generic_function_alongside_an_inferred_private_helper() {
    // `_helper`'s own `Ty::Infer` signature defeats `concrete_function_environment`/
    // `concrete_function_signatures`'s fast path, forcing the solver-inferred
    // path -- this exercises `check_with_signatures_all`'s own `bind_generic`
    // registration (the fast path's registration lives in
    // `concrete_function_environment` instead, already covered by every
    // other test in this group).
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    // `_helper`'s call site deliberately does not itself involve a
    // generic call -- the solver path's own constraint collection
    // (`collect_expr_constraints`, a separate implementation from
    // `infer_expr_in`) has no generic-call dispatch of its own, so
    // mixing the two would exercise a distinct, unrelated gap rather
    // than the `bind_generic` registration this test targets.
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("v".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Name("v".to_string())))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::IntLiteral(5)],
    }));
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert!(find_function(&resolved, "identity").is_none());
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn an_unannotated_private_helper_fed_a_generic_call_s_result_reports_a_clean_diagnostic_instead_of_leaking_ty_param()
 {
    // Fix-round regression test (finding #2): `_helper(v)` has no
    // annotation, so its parameter type must be inferred from its call
    // site's argument -- `_helper(identity(1))`, where `identity` is a
    // still-generic function whose call this solver-based inference
    // path cannot instantiate (it has no notion of
    // `instantiate_generic_call`). Before this fix, the solver
    // unified `_helper`'s fresh inference variable directly against
    // `identity`'s raw, uninstantiated `Ty::Param("T")` signature,
    // producing a confusing diagnostic that named the internal `T`
    // representation directly (e.g. "operator Add is not defined for
    // `T` and `int`"). This must instead fail with a clean, dedicated
    // diagnostic that never mentions `Ty::Param`/`T` as if it were a
    // real type.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("v".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("v".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        !err.message.contains("Ty::Param") && !err.message.contains("\"T\""),
        "diagnostic must never leak the raw `Ty::Param` internal representation, got: {}",
        err.message
    );
    // `check` (the validation-only entry point) must fail the same way.
    let check_err = check(&hir).unwrap_err();
    assert_eq!(check_err.code, "T0042");
}

#[test]
fn an_unannotated_private_helper_s_argument_passed_directly_into_a_generic_parameter_reports_a_clean_diagnostic()
 {
    // Complements the test above by exercising the *other* operand of
    // `collect_expr_constraints`'s own `Ty::Param`-leak guard: here it
    // is the callee's own uninstantiated parameter type
    // (`identity`'s `x: T`) that is `Ok(Ty::Param(_))`, not the
    // caller-supplied argument -- `_helper(x)`'s own unannotated `x` is
    // the unresolved (`Err`) side instead. Both operands of the `||`
    // need their own case to reach `true` (the other test's argument
    // is already resolved when this one's is not, and vice versa),
    // since Rust's `||` short-circuits and a covering test for only one
    // operand leaves the other's `true` branch unreached.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![("x".to_string(), Ty::Infer)],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![HirExpr::IntLiteral(1)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn an_unannotated_private_helper_s_return_type_fed_directly_from_a_generic_call_reports_a_clean_diagnostic()
 {
    // Exercises `unify_terms`'s own dedicated `Ty::Param`-leak guard
    // directly (as opposed to `collect_expr_constraints`'s own Call-arm
    // guard, covered by the two tests above): a private helper's
    // *return* type is unified against its body's final expression
    // unconditionally (`collect_block_constraints`'s `Return` handling
    // has no analogous pre-guard of its own), so `_helper`'s
    // unannotated return type unifying directly against `identity(1)`'s
    // own uninstantiated `Ty::Param` return term must still produce a
    // clean `T0042`, not a leaked-`Ty::Param` diagnostic.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let helper = HirItem::Function {
        name: "_helper".to_string(),
        params: vec![],
        return_ty: Ty::Infer,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }))],
    };
    let use_helper = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "_helper".to_string(),
        args: vec![],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, helper, use_helper],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_binop_and_compare() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
    };
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![],
        return_ty: Ty::Bool,
        body: vec![HirStmt::Return(Some(HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
            right: Box::new(HirExpr::IntLiteral(2)),
        }))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f, g],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_an_fstring_interpolation() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::Str,
        body: vec![HirStmt::Return(Some(HirExpr::FString(vec![
            FStringPart::Literal("x=".to_string()),
            FStringPart::Interpolation(Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ])))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_list_set_and_tuple_literals() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }]),
            },
            HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                }]),
            },
            HirStmt::Assign {
                target: "zs".to_string(),
                value: HirExpr::TupleLiteral(vec![
                    HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(3)],
                    },
                    HirExpr::IntLiteral(4),
                ]),
            },
            HirStmt::Return(None),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_a_dict_literal() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                },
            )]),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_subscript_and_slice() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2),
                    HirExpr::IntLiteral(3),
                ]),
            },
            HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::Subscript {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    index: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    }),
                },
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::Slice {
                    base: Box::new(HirExpr::Name("xs".to_string())),
                    start: Some(Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    })),
                    stop: Some(Box::new(HirExpr::IntLiteral(2))),
                    step: None,
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_list_append_and_set_add() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::Assign {
                target: "ys".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ExprStmt(HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                value: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                }),
            }),
            HirStmt::ExprStmt(HirExpr::SetAdd {
                set: "ys".to_string(),
                value: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                }),
            }),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_dict_get_or_default() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::Assign {
                target: "v".to_string(),
                value: HirExpr::DictGetOrDefault {
                    dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                    key: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::StringLiteral("a".to_string())],
                    }),
                    default: Box::new(HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(0)],
                    }),
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_if_while_and_for_range() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::If {
                test: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                },
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
                orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            },
            HirStmt::While {
                test: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(false)],
                },
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                })],
            },
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(3)],
                },
                step: HirExpr::IntLiteral(1),
                body: vec![HirStmt::ExprStmt(HirExpr::Name("i".to_string()))],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_bool"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_list() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ForList {
                var: "x".to_string(),
                list: "xs".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_dict() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::ForList {
                var: "k".to_string(),
                list: "d".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("k".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
}

#[test]
fn check_and_resolve_rewrites_a_generic_call_inside_for_list_over_a_set() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ForList {
                var: "x".to_string(),
                list: "s".to_string(),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                })],
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_ann_assign_with_and_without_a_value() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AnnAssign {
                is_final: false,
                target: "y".to_string(),
                annotation: Ty::Int,
                value: None,
            },
            HirStmt::AnnAssign {
                is_final: false,
                target: "z".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }),
            },
            HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::IntLiteral(2),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_a_dict_set_statement() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            },
            HirStmt::DictSet {
                dict: "d".to_string(),
                key: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::StringLiteral("b".to_string())],
                },
                value: HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                },
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_str"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn check_and_resolve_rewrites_generic_calls_inside_comprehension_assignments() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "0comp_v".to_string(),
                iter: CompIter::Range {
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::Call {
                        callee: "identity".to_string(),
                        args: vec![HirExpr::IntLiteral(3)],
                    },
                    step: HirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                })),
                elt: Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::Name("0comp_v".to_string())],
                }),
            },
            HirStmt::SetCompAssign {
                target: "ys".to_string(),
                var: "0comp_w".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(true)],
                })),
                elt: Box::new(HirExpr::Name("0comp_w".to_string())),
            },
            HirStmt::DictCompAssign {
                target: "zs".to_string(),
                var: "0comp_u".to_string(),
                iter: CompIter::Name("ys".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::BoolLiteral(false)],
                })),
                key: Box::new(HirExpr::StringLiteral("k".to_string())),
                value: Box::new(HirExpr::Name("0comp_u".to_string())),
            },
            HirStmt::SetCompAssign {
                target: "ks".to_string(),
                var: "0comp_p".to_string(),
                iter: CompIter::Name("zs".to_string()),
                cond: None,
                elt: Box::new(HirExpr::IntLiteral(1)),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
    assert_eq!(count_function(&resolved, "0gen_identity__T_bool"), 1);
}

#[test]
fn check_and_resolve_leaves_a_list_pop_expression_untouched_as_a_structural_leaf() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::ExprStmt(HirExpr::ListPop {
                list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(2)],
            }),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_inside_a_function_body() {
    // Bypasses ordinary validation deliberately, same rationale as the
    // `ForList`/`CompIter` defensive-fallback tests above: a real
    // program with this wrong-arity call would already be rejected by
    // `check`/`check_and_resolve` before `monomorphize` ever ran. This
    // exercises the `?` propagation out of `rewrite_generic_calls_in_stmt`
    // inside `monomorphize`'s own `HirItem::Function` branch.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_top_level_statement() {
    // Same rationale as the function-body variant above, for
    // `monomorphize`'s `HirItem::TopLevelStmt` branch.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_dict_comp_assign_cond() {
    // Bypasses ordinary validation deliberately, same rationale as the
    // other direct `monomorphize` tests in this group -- exercises the
    // `?` propagation out of `rewrite_generic_calls_in_stmt`'s
    // `DictCompAssign` arm specifically for its `cond` sub-expression.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::DictCompAssign {
                target: "zs".to_string(),
                var: "0comp_v".to_string(),
                iter: CompIter::Name("xs".to_string()),
                cond: Some(Box::new(HirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                })),
                key: Box::new(HirExpr::StringLiteral("k".to_string())),
                value: Box::new(HirExpr::IntLiteral(1)),
            },
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

/// A generic call that always fails `instantiate_generic_call` with
/// `T0021` (wrong arity) -- used throughout the error-propagation group
/// below to prove every structural recursion position in
/// `rewrite_generic_calls_in_expr`/`rewrite_generic_calls_in_stmt`/
/// `rewrite_comp_iter` actually propagates a nested failure instead of
/// silently swallowing it.
fn bad_generic_call() -> HirExpr {
    HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }
}

fn assert_monomorphize_propagates_error(body: Vec<HirStmt>) {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body,
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

/// `rewrite_generic_calls_in_stmt`'s `TryStar` arm only runs at all when the
/// module contains a generic function/class/protocol elsewhere (`monomorphize`
/// short-circuits to a plain clone otherwise, per its own leading gate) --
/// unlike the `assert_monomorphize_propagates_error` group below, this
/// exercises the *success* path, including the `handler.name` binding to
/// `ExceptionGroup` (Part 3 of #382, #542, PEP 654), which no error-only
/// fixture reaches since every one of those uses `name: None`.
#[test]
fn monomorphize_rewrites_a_try_star_handler_binding_when_the_module_has_a_generic_function() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::TryStar {
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: Some("e".to_string()),
                body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
            finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect(
        "a module with a generic function and an unrelated try*/except* elsewhere \
         should monomorphize successfully",
    );
}

// PR-13 Task 3: every structural recursion position in
// `rewrite_generic_calls_in_expr`/`rewrite_generic_calls_in_stmt`/
// `rewrite_comp_iter` needs its own covering error-propagation case for
// the 100%-region D-014 gate -- grouped into a few multi-assertion
// tests (rather than one `#[test]` per position) since each case is a
// one-line `bad_generic_call()` substitution, not independent behavior
// worth its own test name.

#[test]
fn rewrite_generic_calls_in_expr_propagates_errors_from_every_recursive_position() {
    // `Call` args (line ~3213) and the `arg_tys` collection step (~3219,
    // via an unresolved argument name rather than `instantiate_generic_call`
    // itself).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![bad_generic_call()],
    })]);
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::Name("undefined".to_string())],
    })]);
    // `BinOp`/`Compare`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(bad_generic_call()),
        right: Box::new(HirExpr::IntLiteral(1)),
    })]);
    // `FString`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::FString(vec![
        FStringPart::Interpolation(Box::new(bad_generic_call())),
    ]))]);
    // `ListLiteral`/`SetLiteral`/`TupleLiteral`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::ListLiteral(vec![
        bad_generic_call(),
    ]))]);
    // `DictLiteral`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::StringLiteral("a".to_string()),
        bad_generic_call(),
    )]))]);
    // `Subscript`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(bad_generic_call()),
        index: Box::new(HirExpr::IntLiteral(0)),
    })]);
    // `Slice`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(bad_generic_call()),
        start: None,
        stop: None,
        step: None,
    })]);
    // `ListAppend`/`SetAdd`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        value: Box::new(bad_generic_call()),
    })]);
    // `DictGetOrDefault`.
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(bad_generic_call()),
        default: Box::new(HirExpr::IntLiteral(0)),
    })]);
    // `AttrGet`'s own `base` (D-154).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::AttrGet {
        base: Box::new(bad_generic_call()),
        attr: "x".to_string(),
    })]);
    // `MethodCall`'s own `base` and its per-argument loop (D-154).
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(bad_generic_call()),
        method: "m".to_string(),
        args: vec![],
    })]);
    assert_monomorphize_propagates_error(vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::IntLiteral(0)),
        method: "m".to_string(),
        args: vec![bad_generic_call()],
    })]);
}

#[test]
fn rewrite_generic_calls_in_stmt_propagates_errors_from_every_recursive_position() {
    // `Assign`.
    assert_monomorphize_propagates_error(vec![HirStmt::Assign {
        target: "y".to_string(),
        value: bad_generic_call(),
    }]);
    // `AnnAssign` (`Some(value)`).
    assert_monomorphize_propagates_error(vec![HirStmt::AnnAssign {
        is_final: false,
        target: "y".to_string(),
        annotation: Ty::Int,
        value: Some(bad_generic_call()),
    }]);
    // `If`'s own `test`, `body`, and `orelse`.
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: bad_generic_call(),
        body: vec![],
        orelse: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        orelse: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::If {
        test: HirExpr::BoolLiteral(true),
        body: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `While`'s own `test` and `body`.
    assert_monomorphize_propagates_error(vec![HirStmt::While {
        test: bad_generic_call(),
        body: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::While {
        test: HirExpr::BoolLiteral(true),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `ForRange`'s own bounds and `body`.
    assert_monomorphize_propagates_error(vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: bad_generic_call(),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(1),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `ForList`'s own `body`.
    assert_monomorphize_propagates_error(vec![
        HirStmt::Assign {
            target: "xs".to_string(),
            value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        },
        HirStmt::ForList {
            var: "x".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        },
    ]);
    // `DictSet`'s own `key`/`value`.
    assert_monomorphize_propagates_error(vec![
        HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::DictLiteral(vec![(
                HirExpr::StringLiteral("a".to_string()),
                HirExpr::IntLiteral(1),
            )]),
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: bad_generic_call(),
            value: HirExpr::IntLiteral(1),
        },
    ]);
    // `ListCompAssign`/`SetCompAssign`/`DictCompAssign`'s own
    // `rewrite_comp_iter` propagation (a bad `CompIter::Range` bound)
    // and their own `cond`/`elt`/`key`/`value` propagation.
    assert_monomorphize_propagates_error(vec![HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(bad_generic_call()),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::SetCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        elt: Box::new(bad_generic_call()),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: bad_generic_call(),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(HirExpr::StringLiteral("k".to_string())),
        value: Box::new(HirExpr::IntLiteral(1)),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::DictCompAssign {
        target: "xs".to_string(),
        var: "v".to_string(),
        iter: CompIter::Range {
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
        },
        cond: None,
        key: Box::new(bad_generic_call()),
        value: Box::new(HirExpr::IntLiteral(1)),
    }]);
    // `Return(Some(value))`.
    assert_monomorphize_propagates_error(vec![HirStmt::Return(Some(bad_generic_call()))]);
    // `AttrSet`'s own `base`/`value` (D-154) -- both share the same
    // `for sub in [base, value]` loop, so a single erroring `base` case
    // exercises the loop's own `?`.
    assert_monomorphize_propagates_error(vec![HirStmt::AttrSet {
        base: bad_generic_call(),
        attr: "x".to_string(),
        value: HirExpr::IntLiteral(0),
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::AttrSet {
        base: HirExpr::IntLiteral(0),
        attr: "x".to_string(),
        value: bad_generic_call(),
    }]);
    // `Match`'s own `subject`, `guard`, and `body` (#381).
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: bad_generic_call(),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![],
        }],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: Some(bad_generic_call()),
            body: vec![],
        }],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Match {
        subject: HirExpr::IntLiteral(0),
        cases: vec![HirMatchCase {
            pattern: HirPattern::Wildcard,
            guard: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
    }]);
    // `Try`'s own body, handler body, else body, and finally body (#382).
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `TryStar`'s own body, handler body, else body, and finally body (Part
    // 3 of #382, #542, PEP 654) -- mirrors the four `Try` cases directly
    // above, since `collect_block_constraints` propagates each of the four
    // blocks' errors with its own `?` rather than sharing `Try`'s.
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(bad_generic_call())],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(bad_generic_call())],
        }],
        orelse: vec![],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::ExprStmt(bad_generic_call())],
        finalbody: vec![],
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::ExprStmt(bad_generic_call())],
    }]);
    // `Raise`'s own exc and cause (#382).
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: Some(bad_generic_call()),
        cause: None,
    }]);
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: None,
        cause: Some(bad_generic_call()),
    }]);
    // The builtin exception constructor is deliberately not re-checked
    // during monomorphization, but a bad generic call nested in its
    // message still has to propagate its instantiation diagnostic.
    assert_monomorphize_propagates_error(vec![HirStmt::Raise {
        exc: Some(HirExpr::Call {
            callee: "ValueError".to_string(),
            args: vec![bad_generic_call()],
        }),
        cause: None,
    }]);
}

#[test]
fn rewrite_generic_calls_in_stmt_raise_with_no_generic_calls_succeeds() {
    // Cover the `Ok(())` return in `rewrite_generic_calls_in_stmt`'s
    // `Raise` arm when both `exc` and `cause` are `None` (bare raise).
    // The module includes a generic function so `monomorphize` does not
    // take its early-return path.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Raise {
            exc: None,
            cause: None,
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect("monomorphize should succeed for bare raise");
}

#[test]
fn rewrite_generic_calls_in_stmt_raise_with_non_generic_cause_succeeds() {
    // Cover the `if let Some(cause)` block's normal completion and the
    // `Ok(())` return in `rewrite_generic_calls_in_stmt`'s `Raise` arm
    // when `cause` is `Some` with a non-generic expression (IntLiteral).
    // `rewrite_generic_calls_in_expr` succeeds for `IntLiteral`, so the
    // block completes normally and `Ok(())` is reached.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::Raise {
            exc: Some(HirExpr::IntLiteral(0)),
            cause: Some(HirExpr::IntLiteral(1)),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    monomorphize(&hir).expect("monomorphize should succeed for raise with literal cause");
}

#[test]
fn monomorphize_defends_a_for_list_over_a_non_container_binding_defensively() {
    // Bypasses ordinary validation deliberately -- `for x in xs` where
    // `xs: int` would already be rejected by `check`/`check_and_resolve`
    // before `monomorphize` ever ran. Calling `monomorphize` directly
    // exercises its own defensive fallback for an already-invalid
    // binding, matching this file's existing "defense in depth" test
    // convention (see e.g. `instantiate_generic_call_rejects_two_distinct_type_parameters`).
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ForList {
            var: "x".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn monomorphize_defends_a_comprehension_over_a_non_container_binding_defensively() {
    // Same defensive-fallback rationale as the `ForList` test above,
    // for `rewrite_comp_iter`'s own `CompIter::Name` fallback arm.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("xs".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "0comp_v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

// ---------------------------------------------------------------
// PR-13 final review fixes: self/mutual generic recursion (Critical),
// consistent `type_aliases` on both `check_and_resolve` paths (I1),
// and a generic body seeing its non-generic siblings (I3).
// ---------------------------------------------------------------

/// `def rec[T](x: T, n: int) -> T` whose body calls `rec` again --
/// the exact shape that used to be accepted by `check` and then ICE
/// in `pycc_mir` during `build`.
fn self_recursive_generic_module() -> HirModule {
    let param = Ty::Param(Box::new("T".to_string()));
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "rec".to_string(),
                params: vec![("x".to_string(), param.clone()), ("n".to_string(), Ty::Int)],
                return_ty: param,
                body: vec![
                    HirStmt::If {
                        test: HirExpr::Compare {
                            op: CmpOpKind::Gt,
                            left: Box::new(HirExpr::Name("n".to_string())),
                            right: Box::new(HirExpr::IntLiteral(0)),
                        },
                        body: vec![HirStmt::Return(Some(HirExpr::Call {
                            callee: "rec".to_string(),
                            args: vec![
                                HirExpr::Name("x".to_string()),
                                HirExpr::BinOp {
                                    op: BinOpKind::Sub,
                                    left: Box::new(HirExpr::Name("n".to_string())),
                                    right: Box::new(HirExpr::IntLiteral(1)),
                                },
                            ],
                        }))],
                        orelse: vec![],
                    },
                    HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
                ],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "rec".to_string(),
                    args: vec![HirExpr::IntLiteral(5), HirExpr::IntLiteral(3)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// A call to the enclosing generic function itself -- the shape
/// `reject_generic_calls_in_block` must find wherever it is nested.
fn self_call() -> HirExpr {
    HirExpr::Call {
        callee: "f".to_string(),
        args: vec![],
    }
}

fn benign() -> HirExpr {
    HirExpr::IntLiteral(0)
}

fn assert_self_call_found(stmt: HirStmt) {
    let err = reject_generic_calls_in_block(&Environment::new(), "f", &[stmt]).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("calls itself"));
}

#[test]
fn the_generic_recursion_gate_finds_a_self_call_in_every_statement_position() {
    // `reject_generic_calls_in_stmt` mirrors
    // `rewrite_generic_calls_in_stmt`'s own exhaustive statement walk;
    // every arm must actually reach the calls it holds, so each one
    // gets a case here (D-014's region gate would otherwise be
    // satisfied by an arm that silently visits nothing).
    let cases = vec![
        HirStmt::ExprStmt(self_call()),
        HirStmt::Assign {
            target: "t".to_string(),
            value: self_call(),
        },
        HirStmt::AnnAssign {
            is_final: false,
            target: "t".to_string(),
            annotation: Ty::Int,
            value: Some(self_call()),
        },
        HirStmt::Return(Some(self_call())),
        HirStmt::If {
            test: self_call(),
            body: vec![],
            orelse: vec![],
        },
        HirStmt::If {
            test: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
            orelse: vec![],
        },
        HirStmt::If {
            test: benign(),
            body: vec![],
            orelse: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::While {
            test: self_call(),
            body: vec![],
        },
        HirStmt::While {
            test: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: self_call(),
            stop: benign(),
            step: benign(),
            body: vec![],
        },
        HirStmt::ForRange {
            var: "i".to_string(),
            start: benign(),
            stop: benign(),
            step: benign(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![HirStmt::ExprStmt(self_call())],
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: self_call(),
            value: benign(),
        },
        HirStmt::DictSet {
            dict: "d".to_string(),
            key: benign(),
            value: self_call(),
        },
        HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Range {
                start: self_call(),
                stop: benign(),
                step: benign(),
            },
            cond: None,
            elt: Box::new(benign()),
        },
        HirStmt::ListCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: Some(Box::new(self_call())),
            elt: Box::new(benign()),
        },
        HirStmt::SetCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(self_call()),
        },
        HirStmt::DictCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Range {
                start: benign(),
                stop: benign(),
                step: benign(),
            },
            cond: Some(Box::new(benign())),
            key: Box::new(self_call()),
            value: Box::new(benign()),
        },
        HirStmt::DictCompAssign {
            target: "ys".to_string(),
            var: "v".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            key: Box::new(benign()),
            value: Box::new(self_call()),
        },
    ];
    for case in cases {
        assert_self_call_found(case);
    }
}

#[test]
fn the_generic_recursion_gate_finds_a_self_call_in_every_expression_position() {
    // Same exhaustiveness requirement one level down, for
    // `reject_generic_calls_in_expr`. Every recursive position gets a
    // case whose call is exactly there, so no arm can regress into
    // silently skipping a sub-expression.
    let cases = vec![
        HirExpr::Call {
            callee: "other".to_string(),
            args: vec![self_call()],
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(self_call()),
            right: Box::new(benign()),
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(benign()),
            right: Box::new(self_call()),
        },
        HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(benign()),
            right: Box::new(self_call()),
        },
        HirExpr::FString(vec![
            FStringPart::Literal("x".to_string()),
            FStringPart::Interpolation(Box::new(self_call())),
        ]),
        HirExpr::ListLiteral(vec![self_call()]),
        HirExpr::SetLiteral(vec![self_call()]),
        HirExpr::TupleLiteral(vec![self_call()]),
        HirExpr::ObjectList(vec![self_call()]),
        HirExpr::DictLiteral(vec![(self_call(), benign())]),
        HirExpr::DictLiteral(vec![(benign(), self_call())]),
        HirExpr::Subscript {
            base: Box::new(self_call()),
            index: Box::new(benign()),
        },
        HirExpr::Subscript {
            base: Box::new(benign()),
            index: Box::new(self_call()),
        },
        HirExpr::Slice {
            base: Box::new(self_call()),
            start: None,
            stop: None,
            step: None,
        },
        HirExpr::Slice {
            base: Box::new(benign()),
            start: Some(Box::new(benign())),
            stop: Some(Box::new(benign())),
            step: Some(Box::new(self_call())),
        },
        HirExpr::ListAppend {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            value: Box::new(self_call()),
        },
        HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(self_call()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(self_call()),
            default: Box::new(benign()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(benign()),
            default: Box::new(self_call()),
        },
    ];
    for case in cases {
        assert_self_call_found(HirStmt::ExprStmt(case));
    }
}

#[test]
fn the_generic_recursion_gate_accepts_a_body_with_no_generic_call_anywhere() {
    // The success tail of every arm above: the same shapes, none of
    // which contains a self-call or a call to a registered generic.
    let leaves = vec![
        HirExpr::IntLiteral(1),
        HirExpr::FloatLiteral(1.0),
        HirExpr::BoolLiteral(true),
        HirExpr::StringLiteral("s".to_string()),
        HirExpr::Name("x".to_string()),
        HirExpr::ListPop {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
        },
        HirExpr::Call {
            callee: "other".to_string(),
            args: vec![benign()],
        },
        HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(benign()),
            right: Box::new(benign()),
        },
        HirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(benign()),
            right: Box::new(benign()),
        },
        HirExpr::FString(vec![
            FStringPart::Literal("x".to_string()),
            FStringPart::Interpolation(Box::new(benign())),
        ]),
        HirExpr::ListLiteral(vec![benign()]),
        HirExpr::SetLiteral(vec![benign()]),
        HirExpr::TupleLiteral(vec![benign()]),
        HirExpr::DictLiteral(vec![(benign(), benign())]),
        HirExpr::Subscript {
            base: Box::new(benign()),
            index: Box::new(benign()),
        },
        HirExpr::Slice {
            base: Box::new(benign()),
            start: Some(Box::new(benign())),
            stop: None,
            step: None,
        },
        HirExpr::ListAppend {
            list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
            value: Box::new(benign()),
        },
        HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(benign()),
        },
        HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(benign()),
            default: Box::new(benign()),
        },
    ];
    let mut body: Vec<HirStmt> = leaves.into_iter().map(HirStmt::ExprStmt).collect();
    body.push(HirStmt::AnnAssign {
        is_final: false,
        target: "t".to_string(),
        annotation: Ty::Int,
        value: None,
    });
    body.push(HirStmt::Return(None));
    assert!(reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok());
}

#[test]
fn check_rejects_a_self_recursive_generic_function() {
    // The validation-only entry point must reject this, not just
    // `check_and_resolve`: the whole defect was `check` (i.e. `pycc
    // check`) accepting a program `pycc build` then panicked on.
    let err = check(&self_recursive_generic_module()).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        err.message.contains("calls itself"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn check_and_resolve_rejects_a_self_recursive_generic_function() {
    // The `build` half of the same gap: `check_and_resolve` is what
    // produces the HIR `pycc_mir` consumes, so the old panic path
    // ("`$fn:rec` has no recorded type") is provably unreachable only
    // if this errors before `monomorphize` runs.
    let err = check_and_resolve(&self_recursive_generic_module()).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(err.message.contains("calls itself"));
}

#[test]
fn check_rejects_a_generic_function_calling_another_generic_function() {
    // Indirect/mutual recursion between two generic functions is
    // rejected by the same gate: `f` cannot be proven non-recursive
    // without a whole-module call-graph analysis D-134's thin slice
    // does not have.
    let param = Ty::Param(Box::new("T".to_string()));
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![("y".to_string(), param.clone())],
        return_ty: param.clone(),
        body: vec![HirStmt::Return(Some(HirExpr::Name("y".to_string())))],
    };
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "g".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        }))],
    };
    // `g` is declared *after* `f`, proving the gate does not depend on
    // source order (pass 1 binds every signature before any body runs).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![f, g],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0042");
    assert!(
        err.message.contains("calls generic function `g`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn a_generic_function_body_can_call_a_non_generic_sibling() {
    // I3: this used to report a factually false `T0021` "call to
    // undefined function `helper`" because the generic body was
    // checked without the module's function environment.
    let param = Ty::Param(Box::new("T".to_string()));
    let helper = HirItem::Function {
        name: "helper".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("n".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let g = HirItem::Function {
        name: "g".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                }],
            }),
            HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            helper,
            g,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![HirExpr::IntLiteral(7)],
                }],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
    let resolved = check_and_resolve(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_g__T_int"), 1);
}

#[test]
fn check_and_resolve_returns_empty_type_aliases_on_both_paths() {
    // I1: a resolved module's `type_aliases` value must not depend on
    // whether the module happened to contain a generic function.
    // D-135 aliases are fully discharged during HIR lowering, so the
    // resolved HIR's own field is empty by design on both paths.
    let param = Ty::Param(Box::new("T".to_string()));
    let aliases = vec![("MyInt".to_string(), Ty::Int)];
    let non_generic = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }],
        type_aliases: aliases.clone(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let generic = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            generic_identity_fn(param.clone(), param),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: aliases,
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(
        check_and_resolve(&non_generic)
            .unwrap()
            .type_aliases
            .is_empty(),
        "no-generics path must discharge `type_aliases`"
    );
    assert!(
        check_and_resolve(&generic).unwrap().type_aliases.is_empty(),
        "monomorphization path must discharge `type_aliases`"
    );
}

// ---- Issue #22 review fixes: incompatible redefinition rejection ----

#[test]
fn incompatible_redefinition_with_different_param_count_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn checked_function_signatures_rejects_incompatible_redefinition() {
    // Exercises the fast path in checked_function_signatures_all that calls
    // check_incompatible_redefinitions before trying concrete or solver.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_different_return_type_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_different_param_type_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn compatible_redefinition_with_same_signature_is_accepted() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn check_incompatible_redefinitions_rejects_infer_signature_mismatch() {
    // Issue #402: a same-arity redefinition where one signature still
    // carries Ty::Infer (unresolved by the solver) and the other is
    // concrete must be rejected, not silently accepted. Ty::Infer is an
    // ordinary unit variant under Ty's derived PartialEq, so it only
    // ever equals another Ty::Infer at the same position -- comparing
    // it unconditionally against check_incompatible_redefinitions's
    // `seen` map correctly flags this as a mismatch.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn check_and_resolve_rejects_the_issue_402_reproduction_fixture() {
    // Issue #402's own reproduction fixture, exercised through
    // check_and_resolve (the `pycc build` path): first `foo` is fully
    // unannotated (Ty::Infer for both the parameter and the return
    // type), second `foo` is fully concrete and a different shape.
    // This is now rejected by the pre-resolution
    // check_incompatible_redefinitions call inside
    // checked_function_signatures_all, before check_and_resolve ever
    // builds a resolved HIR.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
    assert!(
        err.message.contains("<inferred>"),
        "message should render the unresolved first signature's Ty::Infer \
             positions as `<inferred>`, got: {}",
        err.message
    );
}

#[test]
fn checked_function_signatures_rejects_the_issue_402_reproduction_fixture() {
    // Same fixture as check_and_resolve_rejects_the_issue_402_reproduction_fixture
    // and check_incompatible_redefinitions_rejects_infer_signature_mismatch,
    // but through `checked_function_signatures`'s own separate call site
    // (mirrors checked_function_signatures_rejects_incompatible_redefinition
    // above, which exercises that same call site with a fully-concrete,
    // different-arity fixture instead of an Infer-involving one).
    // `check_incompatible_redefinitions` is called directly from both
    // `check_all` and `checked_function_signatures_all`; each entry point is
    // exercised independently rather than relying on one to cover the
    // other.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
}

// -- shared PEP 695 generic-class fixture helpers, used through
// `super::` only by the `generic_*` child modules (#695) --------------

/// Helper: builds a `HirModule` with a generic class `C[T]` whose
/// `__init__` takes `x: T` and stores `self.x = x`, plus a single
/// `C[int](1)` call site at module scope. This is the minimal module
/// that exercises the full `check_and_resolve` → `monomorphize` →
/// `instantiate_generic_class_methods` pipeline.
fn generic_class_module_with_call() -> HirModule {
    let param = Ty::Param(Box::new("T".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), param),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "C.__init__".to_string())],
        type_param: Some("T".to_string()),
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
    };
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    }
}

/// Helper: a `GenericClassInstantiate` expression for `C[int](1)`.
fn gci_expr() -> HirExpr {
    HirExpr::GenericClassInstantiate {
        class: "C".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::IntLiteral(1)],
    }
}

// -- #676 (D-210): cross-MRO attribute redeclaration with a differing
// declared type ------------------------------------------------------

/// Helper for the `check_incompatible_attribute_redeclarations` tests
/// below: builds a minimal `HirClassDef` with the given name, bases, MRO,
/// and own declared attributes, and every other field at its "no extra
/// feature" default -- mirroring the boilerplate already established by
/// `generic_class_module_with_call`'s own `HirClassDef` literal above.
fn minimal_class_def(
    name: &str,
    bases: &[&str],
    mro: &[&str],
    attrs: &[(&str, Ty)],
) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: bases.iter().map(|s| s.to_string()).collect(),
        mro: mro.iter().map(|s| s.to_string()).collect(),
        attrs: attrs
            .iter()
            .map(|(attr_name, ty)| (attr_name.to_string(), ty.clone()))
            .collect(),
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
    }
}

/// Wraps a set of `HirClassDef`s (already carrying their own name, in
/// `(name, def)` pairs) into a minimal `HirModule` with no functions or
/// top-level statements -- `check_incompatible_attribute_redeclarations`
/// only ever reads `hir.class_defs`.
fn module_with_class_defs(class_defs: Vec<(String, HirClassDef)>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs,
    }
}

#[test]
fn cross_mro_attribute_redeclaration_of_int_as_bool_is_rejected() {
    // The issue's own primary repro direction: Base declares `int`,
    // Derived redeclares `bool` for the same attribute name.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Bool)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v')
            && err.message.contains("Base")
            && err.message.contains("Derived"),
        "got: {}",
        err.message
    );
}

#[test]
fn cross_mro_attribute_redeclaration_of_bool_as_int_is_rejected_symmetrically() {
    // The mirror direction (criterion 3): Base declares `bool`, Derived
    // redeclares `int`. Confirms the rule is symmetric/direction-
    // independent, not special-cased to "narrower into wider".
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Bool)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn cross_mro_attribute_redeclaration_of_float_as_int_is_rejected() {
    // The independently-discovered generalization: the defect (and its
    // fix) is not bool/int-specific. A float/int redeclaration must be
    // rejected the same way.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Float)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn same_class_bool_value_into_an_int_declared_attribute_is_not_rejected() {
    // D-187 regression: this check only ever compares two distinct
    // classes' own *declared* attribute types -- never a single class's
    // declared type against an assigned *value*. A same-class `int`-
    // declared attribute assigned a `bool` value elsewhere in the same
    // class's own methods involves no cross-class redeclaration at all,
    // and `attrs` records only the one declared type for `n` on `C`, so
    // this must not fire T0052.
    let c = minimal_class_def("C", &[], &["C"], &[("n", Ty::Int)]);
    let hir = module_with_class_defs(vec![("C".to_string(), c)]);
    assert!(check(&hir).is_ok());
}

#[test]
fn identical_type_redeclaration_across_the_mro_is_not_rejected() {
    // The existing `mro_attrs` dedup case: both Base and Derived declare
    // `v` as `int` -- an identical redeclaration, not a differing one.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    assert!(check(&hir).is_ok());
}

#[test]
fn a_base_class_only_attribute_with_no_redeclaration_is_not_rejected() {
    // Baseline "no MRO conflict at all" case: Derived inherits `v` from
    // Base without redeclaring it anywhere.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def("Derived", &["Base"], &["Derived", "Base"], &[]);
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    assert!(check(&hir).is_ok());
}

#[test]
fn diamond_sibling_base_classes_with_differing_types_are_rejected() {
    // Section 3/7's corrected comparison rule: two sibling base classes
    // `B1`/`B2` (neither an ancestor of the other) each declare the same
    // attribute name with different types, and `D(B1, B2)` inherits both
    // without redeclaring it itself. A rule limited to "class vs. its own
    // ancestor" would miss this entirely -- the check must walk `D`'s own
    // linearized MRO and compare `B1` against `B2` directly as a same-MRO
    // sibling pair.
    let b1 = minimal_class_def("B1", &[], &["B1"], &[("v", Ty::Int)]);
    let b2 = minimal_class_def("B2", &[], &["B2"], &[("v", Ty::Float)]);
    let d = minimal_class_def("D", &["B1", "B2"], &["D", "B1", "B2"], &[]);
    let hir = module_with_class_defs(vec![
        ("B1".to_string(), b1),
        ("B2".to_string(), b2),
        ("D".to_string(), d),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn checked_function_signatures_also_rejects_a_cross_mro_attribute_redeclaration() {
    // `check_incompatible_attribute_redeclarations` is called from both
    // `check_all` and `checked_function_signatures_all` (the latter reachable
    // from `pycc build`'s `check_and_resolve` without an earlier `pycc
    // check`/`check` call) -- each entry point is exercised
    // independently, mirroring how
    // `checked_function_signatures_rejects_the_issue_402_reproduction_fixture`
    // exercises `check_incompatible_redefinitions`'s own second call site.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Bool)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_and_resolve_rejects_the_issue_676_bool_reproduction_fixture() {
    // The issue's own exact end-to-end repro, through the real
    // parser/lowerer/`check_and_resolve` pipeline (the `pycc build`
    // path): Base's own `__init__` declares `v` as `bool`, a base-class
    // method assigns `True` into it (matching Base's own declared type,
    // so this alone is fine), and Derived's own `__init__` redeclares
    // the same attribute name as `int`. This must now be rejected with
    // `T0052` at compile time, not silently mis-decoded or aborted at
    // run time.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 0\nd = Derived()\nd.set_true()\nprint(d.v)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_and_resolve_rejects_the_issue_676_float_int_generalization_fixture() {
    // The independently-discovered generalization, through the real
    // pipeline: Base declares a `float`-typed attribute and assigns into
    // it from another method; Derived's own `__init__` redeclares the
    // same attribute name as `int`. Before this change this compiled
    // with no diagnostic and segfaulted at run time (exit 139); it must
    // now be rejected with `T0052` at compile time instead.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = 1.0\n    def set_it(self) -> None:\n        self.v = 2.5\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 1\nd = Derived()\nd.set_it()\nprint(d.v)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_rejects_the_issue_676_bool_reproduction_fixture_via_the_pycc_check_path() {
    // The two `check_and_resolve_rejects_the_issue_676_*` tests above
    // exercise `pycc build`'s own entry point end-to-end; this mirrors
    // the issue's own bool reproduction through `check` directly (the
    // `pycc check` CLI path, which validates without resolving/
    // monomorphizing), confirming both public entry points reject the
    // same real-source module rather than only one.
    let module = pycc_parser::parse(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 0\nd = Derived()\nd.set_true()\nprint(d.v)\n",
    )
    .expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn a_bare_base_instance_with_no_derived_redeclaration_still_widens_bool_into_int() {
    // The decisive counter-example against "coerce" (D-210's own
    // rationale), pinned as a regression: with no conflicting
    // redeclaration anywhere in the module, a bare `Base()` instance
    // calling a method that assigns a `bool` into its own `int`-declared
    // attribute must still type-check and keep D-187's widening
    // behavior -- this module must NOT be rejected by T0052 merely
    // because a `Base` class exists.
    let hir = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nb = Base()\nb.set_true()\nprint(b.v)\n",
    )
    .expect("no cross-MRO conflict exists here -- must type-check");
    assert!(
        hir.class_defs.iter().any(|(name, _)| name == "Base"),
        "Base should still be a registered class"
    );
}

#[test]
fn subclassing_a_monomorphized_generic_specialization_is_already_rejected_pre_existing() {
    // D-210's own false-positive sweep (section 3) flagged this as an
    // unverified residual: whether a non-generic class can subclass an
    // already-monomorphized generic specialization (`class
    // Derived(C[int]):`). Verified here: it is already rejected with
    // `C0001` ("a base class must be a bare name") before the new
    // `check_incompatible_attribute_redeclarations` check would ever run
    // -- a subscript expression is not a bare base-class name, so this
    // path can never reach `check_incompatible_attribute_redeclarations`'s
    // MRO walk at all. Pinned here so a future change to base-class
    // parsing can't silently open this path without this test noticing.
    let module = pycc_parser::parse(
        "class C[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\nclass Derived(C[int]):\n    pass\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn dataclass_field_type_conflict_across_mro_is_rejected_during_hir_lowering() {
    // Round-2 deep review pinned the pre-fix behavior (a dataclass
    // field-name conflict was silently discarded by `pycc_hir::class`'s
    // merge/dedup before `check_incompatible_attribute_redeclarations`
    // ever ran) as "expected". A later external bot review (round 4,
    // chatgpt-codex-connector) correctly flagged that as the same class of
    // silent cross-type bug issue #676 itself was filed about, just
    // reached through dataclass field merging instead of a base-class
    // method's attribute assignment: Base and Derived each declare a
    // field `v` with a *different* type, and merging must not silently
    // keep the least-derived type -- it must reject the conflict, exactly
    // as a non-dataclass cross-MRO redeclaration is rejected.
    // The conflict is now caught during HIR lowering itself (not by
    // `check_and_resolve`), since dataclass field merging happens in
    // `pycc_hir::class::lower_class` -- so this goes through
    // `pycc_hir::lower_checked` directly rather than `check_source`,
    // which `.expect()`s lowering to succeed.
    let module = pycc_parser::parse(
        "from dataclasses import dataclass\n@dataclass\nclass Base:\n    v: int\n@dataclass\nclass Derived(Base):\n    v: str\nd = Derived(1)\nprint(d.v)\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).expect_err(
        "a dataclass field redeclared with a differing type across the MRO must be rejected",
    );
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v') && err.message.contains("int") && err.message.contains("str"),
        "got: {}",
        err.message
    );
}

#[test]
fn dataclass_field_type_conflict_between_two_bases_is_rejected_during_hir_lowering() {
    // The diamond case: two independent dataclass bases each declare the
    // same field name with a different type, and a derived dataclass
    // inherits from both. The merge must reject this exactly as the
    // base-vs-own-declaration case above, not only when the conflict
    // involves the derived class's own redeclaration.
    let module = pycc_parser::parse(
        "from dataclasses import dataclass\n@dataclass\nclass A:\n    v: int\n@dataclass\nclass B:\n    v: str\n@dataclass\nclass Derived(A, B):\n    pass\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).expect_err(
        "two dataclass bases redeclaring the same field with different types must be rejected",
    );
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v') && err.message.contains("int") && err.message.contains("str"),
        "got: {}",
        err.message
    );
}

#[test]
fn infer_signature_redefinition_is_rejected_regardless_of_body_evidence() {
    // Issue #402: the rejection is structural, based on each
    // definition's raw pre-resolution shape -- not on what the solver
    // would eventually infer from the body. The first `foo`'s body
    // would resolve `x`/the return type to `int` if it ran, but the
    // second `foo` is concretely `(str) -> str`; the raw shapes
    // (Infer, Infer) vs (Str, Str) already disagree, so this is
    // rejected before the solver ever sees the first body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::Str,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
}

#[test]
fn compatible_infer_signature_redefinition_with_call_site_evidence_is_accepted() {
    // Issue #402 must-not-regress: two structurally identical
    // Ty::Infer signatures (an unannotated helper redefined verbatim)
    // must still be accepted -- the fix only rejects a *mismatch*
    // between raw shapes, not an unresolved shape on its own. A call
    // site is required so the solver has evidence to resolve `x`/the
    // return type; without one, this fixture would fail for an
    // unrelated reason ("cannot infer type of parameter").
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "foo".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_ok());
}

#[test]
fn check_and_resolve_rejects_incompatible_redefinition_with_infer_signature() {
    // Issue #402: two functions share the name `foo`; the first has
    // Ty::Infer, the second has a concrete but incompatible (different
    // arity) signature. Before this fix, only a resolved-arity mismatch
    // like this one was caught, and only by a post-resolution recheck
    // inside `check_and_resolve`. That recheck is gone now: this raw
    // shape mismatch (arity, independent of Infer) is caught by the
    // pre-resolution `check_incompatible_redefinitions` call inside
    // `checked_function_signatures_all`, reached before `check_and_resolve`
    // ever builds a resolved HIR.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_bare_return_type_is_rejected() {
    // Issue #402's most plausible real-world trip (see the plan's own
    // §3.5): a first definition that annotates its parameter but
    // leaves the return type bare (Ty::Infer), redefined with a fully
    // concrete signature that adds a return annotation. Params agree;
    // only the return position differs by Infer-vs-concrete -- this
    // isolates that the fix triggers on the return position alone.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn mangled_method_name_does_not_collide_with_same_named_top_level_function() {
    // Issue #402 §3.6 regression: `check_incompatible_redefinitions`
    // iterates the same flat `hir.items` list a class's own methods are
    // merged into, name-keyed with no class metadata. A method always
    // lowers into a mangled `<ClassName>.<method_name>` name (never
    // containing a bare `.`-free `NAME` token), so it can never
    // conflate with a bare top-level function of the unmangled name --
    // confirmed here with deliberately *different* signatures: if the
    // name-keyed map ever conflated `"Foo.bar"` and `"bar"`, this
    // fixture would trip T0021 for a differing param type "under the
    // same name". Accepting it is a stronger check than accepting two
    // identical signatures would be.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "Foo.bar".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "bar".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_ok());
}

// ---- Issue #359 (Part 2 of #118): solver definite-assignment tracking ----

#[test]
fn solver_if_no_else_marks_body_only_binding_as_maybe() {
    // `def _f(cond: bool):` with `if cond: x = 1` (no else) — x is
    // maybe-bound. The solver should track x in `maybe_bindings` so
    // `return x` does not unify the return type with x's type.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_if_with_else_marks_both_branch_binding_as_definite() {
    // `if cond: x = 1` + `else: x = 2` — both branches bind x, so x
    // is definitely bound (not in `maybe_bindings`).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(!env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_if_no_else_does_not_leak_binding_into_orelse() {
    // Before #359, both branches shared the same env, so a binding
    // from the body leaked into the orelse. With cloning, the orelse
    // branch should NOT see x from the body.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x", "y"])
    };
    // Body assigns x; orelse assigns y. After the if, both x and y
    // should be maybe-bound (each was introduced by only one branch).
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.maybe_bindings.contains("y"));
    assert!(env.bindings.contains_key("x"));
    assert!(env.bindings.contains_key("y"));
}

#[test]
fn solver_if_no_else_marks_opaque_body_only_binding_as_maybe() {
    // Issue #771 join-site follow-up: `if cond: y = d.get("a", 0)` (no
    // else), where `DictGetOrDefault` is one of the constructs whose
    // `collect_expr_constraints` arm returns `Ok(None)` (no solver term).
    // Before `join_if_branches_solver` learned to merge `opaque_bindings`,
    // `y` was dropped from every tracked set by the join (it never enters
    // `body_env.bindings`, only `body_env.opaque_bindings`), so a later
    // read of `y` misfired as an unbound local even though the assignment
    // is textually right above it. It must now be maybe-bound: present in
    // `env.opaque_bindings` and `env.maybe_bindings`, absent from
    // `env.bindings` (the solver still has no term for it).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            },
        }],
        orelse: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_if_with_else_marks_both_branch_opaque_binding_as_definite() {
    // Issue #771 join-site follow-up: `if cond: y = d.get(..) else: y =
    // d.get(..)` -- both branches opaquely assign `y`, so it is
    // definitely bound afterward, exactly like the real-term case in
    // `solver_if_with_else_marks_both_branch_binding_as_definite` above.
    // Before the join-site fix this was dropped from every tracked set
    // (not just left "maybe"), which is the case the deep-review pass
    // flagged as not actually branch-conditional in the definite-
    // assignment sense: it reproduced the exact #771 misdiagnosis even
    // though every path through the `if` assigns `y`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let opaque_get = || HirStmt::Assign {
        target: "y".to_string(),
        value: HirExpr::DictGetOrDefault {
            dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
            key: Box::new(HirExpr::StringLiteral("a".to_string())),
            default: Box::new(HirExpr::IntLiteral(0)),
        },
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![opaque_get()],
        orelse: vec![opaque_get()],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(!env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only() {
    // Issue #771 join-site follow-up, second reviewer pass: `y` is already
    // opaquely bound *before* the `if` (e.g. from `y = d.get("a", 0)` two
    // lines above), then reassigned to a real, solver-representable term in
    // only ONE branch (`if cond: y = 1` / no `else`). Before the
    // `pre_existing` fix (computed as `env.bindings.keys()` alone, blind to
    // `env.opaque_bindings`), this name looked "newly introduced" to the
    // join helper on the body side (its `bindings` entry) and also on the
    // orelse side (its untouched `opaque_bindings` entry), so both sides
    // appeared to "introduce" it and it was wrongly folded into
    // `env.maybe_bindings.remove` (i.e. marked *definitely* bound) --
    // exposing the body-only term everywhere, including on the path where
    // `y` was never reassigned. With `pre_existing` now covering
    // `opaque_bindings`, `y` is correctly recognized as already present
    // before the `if` on both sides, so the join's introduced/not-introduced
    // classification leaves its `maybe_bindings` status untouched (it was
    // not maybe before, and reassignment in only one branch does not by
    // itself make a previously-definite name newly maybe under this
    // solver's existing merge semantics for pre-existing names -- the same
    // conservative "leave alone" treatment a pre-existing real-term entry
    // already gets, since `join_if_branches_solver`'s unconditional
    // `entry().or_insert()` merge never overwrites a `bindings` entry that
    // was already populated before the `if`; this test's assertions confirm
    // the analogous protection now holds when the pre-existing entry lives
    // in `opaque_bindings` instead of `bindings`).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    // `y` must never become newly *maybe* bound: it was already definitely
    // (if opaquely) bound before the `if`, and a one-branch reassignment
    // does not retroactively make a pre-existing name conditional.
    assert!(!env.maybe_bindings.contains("y"));
    // `y` must still be resolvable as opaque afterward -- the whole point
    // of this test is that the orelse (no-op) path's opaque value for `y`
    // is not silently discarded in favor of the body-only real term.
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_orelse_branch_only() {
    // Mirror image of `solver_if_reassigns_pre_existing_opaque_binding_in_one_branch_only`
    // above, exercising the `orelse_env.bindings` merge loop's `pre_existing`
    // guard instead of the `body_env.bindings` one -- `join_if_branches_solver`
    // has two separate, textually near-identical merge loops (one per
    // branch), and only writing a body-side test left the orelse-side
    // `continue` unexercised. Here `y` is again pre-existing-opaque before
    // the `if`, but this time only the `orelse` branch reassigns it to a
    // real term; `body` is empty.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(!env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_if_reassigns_pre_existing_opaque_binding_in_both_branches_to_real_terms() {
    // Issue #771 join-site follow-up, THIRD reviewer pass: `y` is
    // pre-existing-opaque before the `if`, and this time BOTH branches
    // reassign it to a real, solver-representable term (`if cond: y = 1
    // else: y = 2`). Unlike the single-branch siblings above, this is not
    // branch-conditional at all -- every path assigns `y` a concrete type --
    // so it must end up definitely bound with a real term, not left opaque.
    // The naive form of the `pre_existing` guard added for the single-branch
    // case (skip whenever the name is pre-existing-opaque, regardless of
    // what the other branch did) incorrectly fired in BOTH merge loops here,
    // since neither loop had inserted into `env.bindings` yet when the other
    // ran. Because `env.opaque_bindings` was never cleared by the join (the
    // name was already present there before the `if`, and the opaque-merge
    // loop only ever inserts), the skipped real terms did not reproduce
    // `unbound_local` -- `HirExpr::Name` still resolved `y` via the
    // surviving stale opaque marker, silently masking both branches' actual
    // concrete terms back down to "opaque, no term available" instead. The
    // fix consults the *other* branch's own `bindings` map instead of
    // `env.bindings`'s own mutation state, so this case correctly falls
    // through to the ordinary first-wins merge and `y` ends up with a real,
    // solver-representable term as it should.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        opaque_bindings: HashSet::from(["y".to_string()]),
        ..ConstraintEnvironment::empty(&["cond", "y"])
    };
    let body = vec![HirStmt::If {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        orelse: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    // `y` must not become newly maybe-bound: every path through the `if`
    // assigns it, so it stays definitely bound.
    assert!(!env.maybe_bindings.contains("y"));
    // `y` must be resolvable as a real term afterward -- it must not be
    // dropped from every tracked set, which would silently mask both
    // branches' concrete terms back down to "opaque, no term available"
    // (see the comment above).
    assert!(env.bindings.contains_key("y"));
    // `env.opaque_bindings` still carries a stale "y" marker here: the join
    // never removes from that set, only inserts into it, and this name was
    // already present there before the `if`. This is harmless only because
    // `HirExpr::Name`'s lookup checks `env.bindings` first and a present
    // real term always wins over a stale opaque marker for the same name
    // (see the lookup-order comment on `HirExpr::Name` in constraints.rs and
    // the merge-loop comment on `join_if_branches_solver` in solver.rs).
    // Pinned explicitly so a future change to that lookup order would be
    // caught here rather than silently reintroducing a masking regression.
    assert!(env.opaque_bindings.contains("y"));
}

#[test]
fn solver_while_loop_marks_opaque_body_only_binding_as_maybe() {
    // Issue #771 join-site follow-up: a `while` body may execute zero
    // times, so `y` assigned only in the body via an opaque initializer
    // is maybe-bound after the loop, exactly like a real-term binding
    // introduced there (`solver_while_loop_marks_body_only_binding_as_maybe`
    // above). Exercises `join_loop_body_solver`'s `opaque_bindings` merge.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([
            ("cond".to_string(), Ok(Ty::Bool)),
            ("d".to_string(), Ok(Ty::Dict(Box::new((Ty::Str, Ty::Int))))),
        ]),
        ..ConstraintEnvironment::empty(&["cond", "d", "y"])
    };
    let body = vec![HirStmt::While {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::DictGetOrDefault {
                dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            },
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("y"));
    assert!(env.opaque_bindings.contains("y"));
    assert!(!env.bindings.contains_key("y"));
}

#[test]
fn solver_while_loop_marks_body_only_binding_as_maybe() {
    // A `while` body may execute zero times, so x assigned only in
    // the body is maybe-bound after the loop.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![HirStmt::While {
        test: HirExpr::Name("cond".to_string()),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_for_range_marks_loop_variable_as_maybe() {
    // A `for` loop may execute zero times, so the loop variable is
    // maybe-bound after the loop (if newly introduced).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i"]);
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("i"));
    assert_eq!(env.bindings.get("i"), Some(&Ok(Ty::Int)));
}

#[test]
fn solver_for_range_body_only_binding_is_maybe() {
    // A binding introduced only in the for-loop body is maybe-bound
    // after the loop (the loop may not execute).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["i", "x"]);
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    assert!(env.maybe_bindings.contains("i"));
    assert!(env.maybe_bindings.contains("x"));
}

#[test]
fn solver_for_range_pre_existing_binding_stays_definite() {
    // A binding that existed before the loop stays definite (not in
    // maybe_bindings) after the loop.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["i", "x"])
    };
    let body = vec![HirStmt::ForRange {
        var: "i".to_string(),
        start: HirExpr::IntLiteral(0),
        stop: HirExpr::IntLiteral(3),
        step: HirExpr::IntLiteral(1),
        body: vec![HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
    }];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    // x was pre-existing → stays definite (not maybe)
    assert!(!env.maybe_bindings.contains("x"));
    // i was newly introduced → maybe
    assert!(env.maybe_bindings.contains("i"));
}

#[test]
fn solver_maybe_bound_name_skips_unification_in_name_arm() {
    // A maybe-bound name returns Ok(None) from the Name arm, so
    // `return x` does not unify the return type with x's type.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        maybe_bindings: HashSet::from(["x".to_string()]),
        ..ConstraintEnvironment::empty(&["x"])
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &HirExpr::Name("x".to_string()),
    )
    .unwrap();

    // Ok(None) — the type term is not available for unification
    assert_eq!(term, None);
}

#[test]
fn solver_unconditional_assignment_upgrades_maybe_to_definite() {
    // Issue #359: an unconditional assignment after a maybe-binding
    // join upgrades the name back to definitely bound, mirroring the
    // validation pass's contract: `if c: x = 1; x = 2; return x` is
    // valid because `x = 2` makes `x` definitely bound.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment {
        bindings: HashMap::from([("cond".to_string(), Ok(Ty::Bool))]),
        ..ConstraintEnvironment::empty(&["cond", "x"])
    };
    let body = vec![
        HirStmt::If {
            test: HirExpr::Name("cond".to_string()),
            body: vec![HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }],
            orelse: vec![],
        },
        // Unconditional assignment upgrades x to definitely bound
        HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    ];

    collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap();

    // x should NOT be in maybe_bindings after the unconditional assignment
    assert!(!env.maybe_bindings.contains("x"));
    assert!(env.bindings.contains_key("x"));
}

#[test]
fn solver_definitely_bound_name_returns_its_term() {
    // A definitely-bound name (not in maybe_bindings) returns its
    // type term as before.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut binops = DeferredConstraints::default();
    let env = ConstraintEnvironment {
        bindings: HashMap::from([("x".to_string(), Ok(Ty::Int))]),
        ..ConstraintEnvironment::empty(&["x"])
    };

    let term = collect_expr_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut binops,
        &env,
        &HirExpr::Name("x".to_string()),
    )
    .unwrap();

    assert_eq!(term, Some(Ok(Ty::Int)));
}

#[test]
fn solver_private_helper_with_maybe_bound_return_does_not_infer_wrong_type() {
    // End-to-end: a private helper with `if cond: x = 1; return x`
    // should not infer a wrong return type from the maybe-bound x.
    // The solver conservatively refuses to infer the return type
    // (T0021 "cannot infer return type") because `return x` skips
    // unification when x is maybe-bound. With an explicit annotation,
    // the validation pass catches T0041 separately (see the next test).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    // The solver cannot infer the return type because x is maybe-bound
    // and `return x` skips unification. This is the conservative
    // behavior the issue requires.
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot infer return type"));
}

#[test]
fn solver_private_helper_with_annotation_catches_t0041_for_maybe_bound() {
    // End-to-end: with an explicit return annotation, the solver
    // doesn't need to infer the return type, so the validation pass
    // runs and catches T0041 for the maybe-bound read of x.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0041");
}

#[test]
fn solver_private_helper_with_both_branches_binding_infers_correctly() {
    // End-to-end: a private helper with both if-branches binding x
    // should infer the return type correctly (x is definitely bound).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(2),
                    }],
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn solver_private_helper_with_unconditional_upgrade_infers_correctly() {
    // End-to-end: `if c: x = 1; x = 2; return x` — the unconditional
    // `x = 2` after the maybe-binding `if` upgrades x to definitely
    // bound, so the solver can infer the return type from `return x`.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_helper".to_string(),
            params: vec![("cond".to_string(), Ty::Bool)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Name("cond".to_string()),
                    body: vec![HirStmt::Assign {
                        target: "x".to_string(),
                        value: HirExpr::IntLiteral(1),
                    }],
                    orelse: vec![],
                },
                HirStmt::Assign {
                    target: "x".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

// -- PEP 591 (#383): Final[X] reassignment diagnostic T0045 ------------

#[test]
fn final_annotated_name_with_initial_value_succeeds() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn final_annotated_name_reassignment_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_value_less_declaration_then_first_assignment_succeeds() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn final_value_less_declaration_then_two_assignments_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_reassignment_in_function_scope_is_t0045() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_reassignment_via_valued_annassign_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(2)),
            is_final: true,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_does_not_block_a_non_final_reassignment_of_a_different_name() {
    // A `Final` declaration on `x` must not affect a different name `y`.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Int));
}

#[test]
fn child_for_function_clears_a_final_entry_for_a_shadowed_local_name() {
    // A module-level `Final` declaration for `x` must not leak into an
    // unrelated function's own local `x` — mirrors the existing
    // `bindings`/`declared`-clearing behavior.
    let mut module_env = Environment::new();
    check_stmt(
        &mut module_env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let mut fn_env = module_env.child_for_function(&["x"]);
    // The function's own body assigns to its local `x` — if the
    // module-level `finals` entry leaked through, this would fail with
    // T0045.
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(42),
        },
        Ty::None,
    )
    .unwrap();
    // A second assignment in the function body should also succeed
    // (the function-local `x` is not `Final`).
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(99),
        },
        Ty::None,
    )
    .unwrap();
}

// -- #380: Protocol monomorphization and marker value tests ----------

/// Parses, lowers, and type-checks source code, returning the
/// resolved HIR or the diagnostic.
fn check_source(source: &str) -> Result<HirModule, pycc_diag::Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check_and_resolve(&hir)
}

// -- #579 (Part 3 of #572): the stdlib decorator-marker surface -----
//
// pycc recognizes `@dataclass`/`@override`/`@dataclass_transform()` as
// bare names without an import (the `Final`/`Annotated`/`Enum`
// precedent). What #579 adds is that the *import* CPython itself
// requires now resolves, so the PEP 557/698/3129 conformance fixtures
// can carry it and still run under the pinned oracle. These tests pin
// the composed shape -- import plus decorator in one program -- rather
// than only the registry lookup, which `pycc_std`'s own tests cover.

#[test]
fn dataclasses_import_composes_with_a_dataclass_decorated_class() {
    let result = check_source(
        "from dataclasses import dataclass\n@dataclass\nclass Point:\n    x: int\n    y: int\np = Point(1, 2)\nprint(p)\n",
    );
    assert!(
        result.is_ok(),
        "`from dataclasses import dataclass` plus `@dataclass` must type-check: {result:?}"
    );
}

#[test]
fn typing_override_import_composes_with_an_override_decorated_method() {
    let result = check_source(
        "from typing import override\nclass Base:\n    def __init__(self) -> None:\n        self.x = 0\n    def f(self) -> int:\n        return 1\nclass Derived(Base):\n    def __init__(self) -> None:\n        return\n    @override\n    def f(self) -> int:\n        return 2\nd = Derived()\nprint(d.f())\n",
    );
    assert!(
        result.is_ok(),
        "`from typing import override` plus `@override` must type-check: {result:?}"
    );
}

#[test]
fn typing_dataclass_transform_import_composes_with_its_decorator() {
    // The `@dataclass_transform()`-as-`@dataclass` treatment is a
    // deliberate semantic approximation (D-196, #749) and is why
    // no byte-for-byte fixture exercises it; the import resolving is
    // still in scope here, and this pins it.
    let result = check_source(
        "from typing import dataclass_transform\n@dataclass_transform()\nclass Tagged:\n    tag: int\nt = Tagged(1)\nprint(t.tag)\n",
    );
    assert!(
        result.is_ok(),
        "`from typing import dataclass_transform` plus its decorator must type-check: {result:?}"
    );
}

#[test]
fn protocol_typed_parameter_function_call_triggers_monomorphization() {
    // This exercises the rewrite_protocol_calls_in_expr and
    // substitute_body_protocol paths in check_and_resolve.
    let result = check_source(
        "from typing import Protocol\nclass Drawable(Protocol):\n    def draw(self) -> None: ...\nclass Circle:\n    def __init__(self) -> None:\n        self.x = 0\n    def draw(self) -> None:\n        return\ndef render(d: Drawable) -> None:\n    d.draw()\nc = Circle()\nrender(c)\n",
    );
    assert!(
        result.is_ok(),
        "protocol-typed parameter call should type-check"
    );
    // Verify that the monomorphized HIR contains a specialization
    // with the concrete type substituted.
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, .. } = item {
            name.contains("0gen_")
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "monomorphized HIR should contain a protocol specialization"
    );
}

#[test]
fn protocol_marker_used_as_a_value_is_t0021() {
    // This directly exercises the marker_is_not_a_value function.
    let diag = marker_is_not_a_value("Protocol");
    assert_eq!(diag.code, "T0021");
    assert!(
        diag.message.contains("marker symbol"),
        "unexpected message: {}",
        diag.message
    );
}

#[test]
fn abc_marker_used_as_a_value_is_t0021() {
    let diag = marker_is_not_a_value("ABC");
    assert_eq!(diag.code, "T0021");
}

#[test]
fn runtime_checkable_decorator_used_as_a_value_is_t0021() {
    let diag = marker_is_not_a_value("runtime_checkable");
    assert_eq!(diag.code, "T0021");
}

#[test]
fn protocol_typed_parameter_with_if_statement_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's If arm — the
    // call to the protocol-parameter function must be inside a
    // *non-protocol* function's if body.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    if True:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_while_loop_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    while False:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_for_range_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    for i in range(1):\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_return_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's Return arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> int:\n    return proto_fn(c)\nc = C()\nprint(caller())\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_assignment_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's Assign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_annassign_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's AnnAssign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x: int = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn isinstance_with_protocol_in_generic_context_triggers_rewrite_special_case() {
    // This exercises the isinstance/issubclass special case in
    // rewrite_generic_calls_in_expr.
    let result = check_source(
        "from typing import Protocol, runtime_checkable\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nprint(isinstance(c, P))\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_for_list_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ForList arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [1, 2]\n    for x in xs:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_expr_stmt_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's ExprStmt arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_wrapped_in_a_walrus_triggers_monomorphization() {
    // PEP 572 (#774), deep-review follow-up: `rewrite_protocol_calls_in_expr`
    // is a non-exhaustive recursion (a trailing `_ => {}` catch-all) over
    // only the shapes a protocol-typed call can appear inside. Without a
    // dedicated `HirExpr::NamedExpr` arm, `(x := proto_fn(c))` fell into
    // that catch-all: the call site was never rewritten to its
    // specialization, while `monomorphize_protocol_params` unconditionally
    // drops the original `proto_fn` item regardless, leaving a dangling
    // call to a function no longer in the module. Also reads `x` on the
    // next line, exercising `bind_local_types_in_stmt`'s `ExprStmt` arm
    // pre-binding the walrus target for this same pass.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    (x := proto_fn(c))\n    print(x)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_method_call_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's MethodCall arm.
    // `print(proto_fn(c))` is a Call whose argument is a Call to the
    // protocol-parameter function — the recursion into args hits the
    // Call arm, and the outer print is also a Call.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    print(proto_fn(c))\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_attr_get_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's AttrGet arm.
    // We need the protocol function to return an instance so we can
    // access an attribute on the result.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 42\n    def foo(self) -> int:\n        return 1\nclass Q(Protocol):\n    def get_c(self) -> C: ...\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def get_c(self) -> C:\n        return C()\ndef proto_fn(q: Q) -> C:\n    return q.get_c()\ndef caller() -> None:\n    print(proto_fn(d).x)\nd = D()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_binop_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's BinOp arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c) + 1\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_compare_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's Compare arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = proto_fn(c) == 1\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_list_literal_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's ListLiteral arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x = [proto_fn(c)]\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_parameter_with_non_protocol_param_covers_protocol_param_name_none() {
    // This exercises the None branch in protocol_param_name (when
    // a non-protocol parameter precedes the protocol parameter).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(n: int, p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    proto_fn(0, c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_attribute_access_on_unknown_member_is_t0044() {
    // This exercises the t0044_unknown_member error path in
    // resolve_attr_get for protocol-typed variables.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef read_unknown(p: P) -> None:\n    print(p.y)\nc = C()\nread_unknown(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_method_call_on_unknown_member_is_t0044() {
    // This exercises the t0044_unknown_member error path in
    // resolve_method_call for protocol-typed variables.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef call_unknown(p: P) -> None:\n    p.bar()\nc = C()\ncall_unknown(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn protocol_to_protocol_assignability_with_different_protocols_is_t0046() {
    // This exercises the protocol-to-protocol mismatch path in
    // is_assignable_env and assignable_error.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef assign_proto(p: P) -> None:\n    q: Q = p\nc = C()\nassign_proto(c)\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0046");
}

#[test]
fn protocol_instantiation_is_c0001() {
    // This exercises the protocol instantiation rejection path.
    let err = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\np = P()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn protocol_typed_annassign_mismatch_is_t0046() {
    // This exercises the protocol annotation mismatch path in
    // AnnAssign checking.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nx: P = 1\n",
        )
        .unwrap_err();
    assert!(err.code == "T0046" || err.code == "T0021");
}

#[test]
fn protocol_to_same_protocol_assignability_succeeds() {
    // This exercises is_assignable_env's same-protocol arm
    // (class.rs line 242: `return true`).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef fn(p: P) -> None:\n    q: P = p\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_to_inherited_protocol_assignability_succeeds() {
    // This exercises is_assignable_env's inherited-protocol arm
    // (class.rs line 248: `return true` when MRO includes to_name).
    // Q inherits P, so Q's MRO includes P. Assigning Q to P should
    // succeed because Q has everything P requires.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\n    def bar(self) -> int:\n        return 2\ndef fn(q: Q) -> None:\n    p: P = q\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_attribute_access_on_conforming_class_succeeds() {
    // This exercises resolve_attr_get's protocol attribute lookup
    // success path (class.rs line 475: `return Ok(ty.clone())`).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 42\ndef fn(p: P) -> None:\n    print(p.x)\nc = C()\nfn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_if_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's If arm (lines 5188-5192)
    // by having an if statement inside a protocol-parameter function.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    if True:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_while_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    while False:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_for_range_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    for i in range(1):\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_for_list_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's ForList arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> None:\n    xs = [1]\n    for x in xs:\n        p.foo()\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_function_body_with_annassign_for_substitute_stmt_protocol() {
    // This exercises substitute_stmt_protocol's AnnAssign arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef proto_fn(p: P) -> None:\n    y: int = p.x\nc = C()\nproto_fn(c)\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_if_orelse_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's If orelse
    // iteration (lines 6782-6783).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    if False:\n        proto_fn(c)\n    else:\n        proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_method_call_context_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_expr's MethodCall arm
    // (lines 6898-6902). We need a protocol function that returns an
    // instance, then call a method on the result.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nclass Q(Protocol):\n    def get_c(self) -> C: ...\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def get_c(self) -> C:\n        return C()\ndef proto_fn(q: Q) -> C:\n    return q.get_c()\ndef caller() -> None:\n    proto_fn(d).foo()\nd = D()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn isinstance_inside_generic_function_triggers_rewrite_special_case() {
    // This exercises the isinstance/issubclass special case in
    // rewrite_generic_calls_in_expr (lines 5560-5569). We need a
    // protocol-typed parameter function to prevent the monomorphization
    // early return, and a generic (unannotated-parameter) function
    // whose body contains isinstance.
    let result = check_source(
        "from typing import Protocol, runtime_checkable\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef _generic_fn(x) -> int:\n    isinstance(x, P)\n    return x\nc = C()\nproto_fn(c)\nprint(_generic_fn(42))\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_protocol_typed_arg_covers_non_instance_break() {
    // This exercises the break when arg_ty is Ok(Protocol) instead
    // of Ok(Instance) in rewrite_protocol_calls_in_expr.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    x: P = c\n    print(proto_fn(x))\nc = C()\ncaller()\n",
    );
    // This may or may not succeed depending on whether the
    // monomorphization handles protocol-typed args gracefully.
    // We just need to exercise the code path.
    let _ = result;
}

#[test]
fn protocol_call_as_method_argument_covers_methodcall_args() {
    // This exercises the MethodCall arm's args iteration in
    // rewrite_protocol_calls_in_expr (lines 6910-6911).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\n    def bar(self, y: int) -> int:\n        return y\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    print(c.bar(proto_fn(c)))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_typed_local_variable_method_call_reaches_mir() {
    // This exercises the `Ty::Protocol(name) => name.as_str()` arm
    // in MIR's `class_def_of` (line 1855) by creating a protocol-
    // typed local variable inside a non-protocol function.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef caller() -> None:\n    x: P = C()\n    print(x.foo())\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn qualified_decorator_marker_called_directly_is_t0021() {
    // This exercises the marker callee path (line 2743) in
    // infer_expr_in's Call arm by using the qualified form
    // `abc.abstractmethod()` which resolves via std_qualified_symbol.
    let err = check_source(
        "class C:\n    def __init__(self) -> None:\n        return\nabc.abstractmethod()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_used_as_value_is_t0021() {
    // This exercises the DecoratorMarker arm (line 2557) in
    // infer_expr_in's Name arm by using the qualified form
    // `abc.abstractmethod` as a value (not a call).
    let err = check_source(
        "class C:\n    def __init__(self) -> None:\n        return\nx = abc.abstractmethod\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_called_in_private_helper_is_t0021() {
    // This exercises the marker callee path (line 1195) in
    // collect_expr_constraints' Call arm (the solver path) by
    // using the qualified form in a private helper.
    let err = check_source(
            "class C:\n    def __init__(self) -> None:\n        return\ndef _helper() -> int:\n    abc.abstractmethod()\n    return 1\n_helper()\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_decorator_marker_as_value_in_private_helper_is_t0021() {
    // This exercises the DecoratorMarker arm (line 1045) in
    // collect_expr_constraints' Name arm (the solver path) by
    // using the qualified form as a value in a private helper.
    let err = check_source(
            "class C:\n    def __init__(self) -> None:\n        return\ndef _helper() -> int:\n    x = abc.abstractmethod\n    return 1\n_helper()\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn qualified_annotation_marker_used_as_value_is_t0021() {
    // #762: exercises the dedicated `AnnotationMarker` arm (routed to
    // `annotation_marker_is_not_a_value`, not the generic
    // `marker_is_not_a_value`, per PR #766 review) folded into the marker
    // arm of infer_expr_in's Name handler (expr.rs) by using the
    // qualified form `typing.Final` as a value (not an annotation
    // subscript).
    let err = check_source("import typing\nx = typing.Final\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker") && err.message.contains("typing.Final[int]"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_as_value_in_private_helper_is_t0021() {
    // #762: exercises the dedicated `AnnotationMarker` arm folded into the
    // marker arm of collect_expr_constraints' Name handler (constraints.rs,
    // the solver path) by using the qualified form `typing.Annotated`
    // as a value in a private helper.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.Annotated\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_called_is_t0021() {
    // #762 / PR #766 review: exercises the AnnotationMarker branch of the
    // call-site marker guard in expr.rs's infer_expr_in (the `Function`
    // let-else fallthrough), by calling `typing.Final` directly instead of
    // using it as a bare-name value or an annotation subscript.
    let err = check_source("import typing\nx = typing.Final()\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_annotation_marker_called_in_private_helper_is_t0021() {
    // #762 / PR #766 review: exercises the same call-site AnnotationMarker
    // branch in constraints.rs's collect_expr_constraints (the solver
    // path), by calling `typing.Annotated` directly inside a private
    // helper.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.Annotated()\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("annotation marker"),
        "expected an annotation-specific message, got: {}",
        err.message
    );
}

// -- #380 W1: protocol monomorphization through DictSet/AttrSet/comp arms -

#[test]
fn protocol_call_in_dict_set_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's DictSet arm
    // (lines 6851-6854) — the protocol-parameter function call is the
    // *value* of a `d[k] = ...` item assignment.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    d = {\"k\": 0}\n    d[\"k\"] = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_attr_set_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's AttrSet arm
    // (lines 6855-6858) — the protocol-parameter function call is the
    // *value* of an `obj.attr = ...` attribute assignment.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    obj = C()\n    obj.x = proto_fn(c)\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_list_comp_elt_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // ListCompAssign arm (lines 6859-6865) with no `if` filter —
    // the protocol-parameter function call is the *elt* of a list
    // comprehension, covering the `cond == None` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [proto_fn(c) for i in range(3)]\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_set_comp_elt_with_cond_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // SetCompAssign arm (lines 6859-6865, shared with
    // ListCompAssign) with an `if` filter — covering the
    // `cond == Some(c)` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {proto_fn(c) for i in range(3) if i > 0}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_dict_comp_value_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // DictCompAssign arm (lines 6866-6872) with no `if` filter —
    // the protocol-parameter function call is the *value* of a
    // dict comprehension, covering the `cond == None` branch.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {\"k\": proto_fn(c) for i in range(3)}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_in_dict_comp_value_with_cond_triggers_monomorphization() {
    // This exercises rewrite_protocol_calls_in_stmt's
    // DictCompAssign arm (lines 6866-6872) with an `if` filter —
    // covering the `cond == Some(c)` branch for the dict-comp arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = {\"k\": proto_fn(c) for i in range(3) if i > 0}\nc = C()\ncaller()\n",
    );
    assert!(result.is_ok());
}

// -- Bug 2: function-local variable as protocol call argument ----------

#[test]
fn protocol_call_with_function_local_variable_argument_compiles() {
    // A function-local variable (not a module-level global) passed
    // to a protocol-parameter function must be resolved through the
    // monomorphization pass's child environment, not the module env.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, .. } = item {
            name.contains("0gen_")
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "monomorphized HIR should contain a protocol specialization for a local-variable argument"
    );
}

#[test]
fn protocol_call_with_local_from_if_block_compiles() {
    // A local variable assigned inside an if block (after being
    // pre-bound) and then passed to a protocol-parameter function —
    // exercises bind_local_types_in_stmt's If arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    if True:\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_for_range_compiles() {
    // A local variable reassigned inside a for-range loop body and
    // then passed to a protocol-parameter function — exercises
    // bind_local_types_in_stmt's ForRange arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    for i in range(1):\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_for_list_compiles() {
    // A for-list loop over a list[int] alongside a protocol call —
    // exercises bind_local_types_in_stmt's ForList arm (binding the
    // loop variable's type from the list's element type).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    xs = [1, 2, 3]\n    for x in xs:\n        print(x)\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_local_from_while_block_compiles() {
    // A local variable reassigned inside a while loop body and then
    // passed to a protocol-parameter function — exercises
    // bind_local_types_in_stmt's While arm.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c = C()\n    while False:\n        c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_annassign_local_compiles() {
    // A local variable bound via AnnAssign (with a value) passed to
    // a protocol-parameter function — exercises
    // bind_local_types_in_stmt's AnnAssign arm with a value.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c: C = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

#[test]
fn protocol_call_with_valueless_annassign_then_assign_compiles() {
    // A value-less AnnAssign (`c: C`) followed by an assignment —
    // exercises bind_local_types_in_stmt's AnnAssign arm without a
    // value (binding from the annotation type).
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef proto_fn(p: P) -> int:\n    return p.foo()\ndef caller() -> None:\n    c: C\n    c = C()\n    print(proto_fn(c))\ncaller()\n",
    );
    assert!(result.is_ok());
}

// -- Bug 4: nested protocol-parameter function calls -------------------

#[test]
fn nested_protocol_parameter_functions_both_get_specialized() {
    // One protocol-parameter function calling another — the inner
    // call in the specialized body must also be rewritten.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef inner(p: P) -> int:\n    return p.foo()\ndef outer(p: P) -> int:\n    return inner(p)\nc = C()\nprint(outer(c))\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let specialization_count = resolved
        .items
        .iter()
        .filter(|item| {
            if let HirItem::Function { name, .. } = item {
                name.contains("0gen_")
            } else {
                false
            }
        })
        .count();
    assert!(
        specialization_count >= 2,
        "both inner and outer should be specialized, found {specialization_count}"
    );
}

// -- Bug 5: multiple protocol-typed parameters ------------------------

#[test]
fn multiple_protocol_parameters_all_get_specialized() {
    // A function with two different protocol-typed parameters — both
    // must be substituted with their respective concrete types.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nclass D:\n    def __init__(self) -> None:\n        self.x = 0\n    def bar(self) -> int:\n        return 2\ndef combine(a: P, b: Q) -> int:\n    return a.foo() + b.bar()\nc = C()\nd = D()\nprint(combine(c, d))\n",
    );
    assert!(result.is_ok());
    let resolved = result.unwrap();
    let has_specialization = resolved.items.iter().any(|item| {
        if let HirItem::Function { name, params, .. } = item {
            name.contains("0gen_combine")
                && params.iter().all(|(_, ty)| !matches!(ty, Ty::Protocol(_)))
        } else {
            false
        }
    });
    assert!(
        has_specialization,
        "combine should be specialized with both protocol types replaced"
    );
}

#[test]
fn multiple_protocol_parameters_same_protocol_compiles() {
    // A function with two parameters of the same protocol type —
    // both should be substituted with the same concrete type.
    let result = check_source(
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef combine(a: P, b: P) -> int:\n    return a.foo() + b.foo()\nc = C()\nprint(combine(c, c))\n",
    );
    assert!(result.is_ok());
}

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
    let mut covered: HashSet<&str> = HashSet::new();
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
fn protocol_argument_mismatch_emits_t0046() {
    // This exercises the assignable_error call (line 2894) when
    // a non-conforming class is passed to a protocol-typed parameter.
    let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\ndef bar(p: P) -> int:\n    return p.foo()\nbar(C())\n",
        )
        .unwrap_err();
    assert_eq!(err.code, "T0046");
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

// -- #382 parse/check helpers, shared by the tests that stayed here (the
// `#382 coverage tests` run below) and, through `super::`, by the
// `exception_handling` and `enum_unrolling` child modules (#695) --

fn parse_check_resolve(src: &str) -> Result<HirModule, Diagnostic> {
    let module = pycc_parser::parse(src).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("lowering must succeed");
    check_and_resolve(&hir)
}

fn parse_check(src: &str) -> Result<(), Diagnostic> {
    let module = pycc_parser::parse(src).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("lowering must succeed");
    check(&hir)
}

// -- #382 coverage tests --

#[test]
fn contains_return_finds_a_return_inside_a_try_body() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_handler() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_else() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(3)))],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_finally() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Return(Some(HirExpr::IntLiteral(4)))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_does_not_find_a_return_in_a_try_without_one() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!contains_return(&body));
}

// Part 3 of #382 (#542, PEP 654): `contains_return`/`introduces_bindings`
// share their `HirStmt::Try` match arm with `HirStmt::TryStar` via an `|`
// pattern, but the `Try`-only tests above never actually construct a
// `TryStar` value, so that shared arm's binding pattern is only ever matched
// through `Try`. Mirror each `Try` case once for `TryStar` so the shared arm
// is exercised through both variants.

#[test]
fn contains_return_finds_a_return_inside_a_try_star_body() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_handler() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(2)))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_else() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Return(Some(HirExpr::IntLiteral(3)))],
        finalbody: vec![],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_finds_a_return_inside_a_try_star_finally() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Return(Some(HirExpr::IntLiteral(4)))],
    }];
    assert!(contains_return(&body));
}

#[test]
fn contains_return_does_not_find_a_return_in_a_try_star_without_one() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!contains_return(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_body() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_handler() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_else() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Assign {
            target: "w".to_string(),
            value: HirExpr::IntLiteral(3),
        }],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_star_finally() {
    let body = vec![HirStmt::TryStar {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Assign {
            target: "v".to_string(),
            value: HirExpr::IntLiteral(4),
        }],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_does_not_find_bindings_in_a_try_star_without_any() {
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(3))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(4))],
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_body() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(1),
        }],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_handler() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::IntLiteral(2),
            }],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_else() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![HirStmt::Assign {
            target: "w".to_string(),
            value: HirExpr::IntLiteral(3),
        }],
        finalbody: vec![],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_finds_bindings_inside_a_try_finally() {
    let body = vec![HirStmt::Try {
        body: vec![],
        handlers: vec![],
        orelse: vec![],
        finalbody: vec![HirStmt::Assign {
            target: "v".to_string(),
            value: HirExpr::IntLiteral(4),
        }],
    }];
    assert!(introduces_bindings(&body));
}

#[test]
fn introduces_bindings_does_not_find_bindings_in_a_try_without_any() {
    let body = vec![HirStmt::Try {
        body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(1))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(2))],
        }],
        orelse: vec![],
        finalbody: vec![],
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn introduces_bindings_treats_raise_as_not_introducing_bindings() {
    let body = vec![HirStmt::Raise {
        exc: Some(HirExpr::Call {
            callee: "ValueError".to_string(),
            args: vec![HirExpr::StringLiteral("bad".to_string())],
        }),
        cause: None,
    }];
    assert!(!introduces_bindings(&body));
}

#[test]
fn raise_with_wrong_arg_count_is_t0021() {
    let err = parse_check("raise ValueError(\"a\", \"b\")\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("exactly 1 argument"));
}

#[test]
fn raise_with_wrong_arg_type_is_t0021() {
    let err = parse_check("raise ValueError(42)\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("str"));
}

#[test]
fn raise_with_no_args_is_t0021() {
    let err = parse_check("raise ValueError()\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("exactly 1 argument"));
}

#[test]
fn raise_user_defined_class_instance_is_t0021() {
    // A user-defined class is not a builtin exception class.
    let src = "\
class C:
    def __init__(self):
        pass
raise C()
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("can only raise"));
}

#[test]
fn try_except_as_binding_in_function_collects_local_name() {
    // This exercises `collect_local_names` for the `Try` handler name
    // binding path inside a function body.
    let src = "\
def f() -> int:
    y = 0
    try:
        x = 1
    except ValueError as e:
        y = 2
    return y
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_except_as_binding_with_typed_handler_in_function() {
    // This exercises `check_try_stmt`'s handler binding path with a
    // typed exception handler inside a function.
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError as e:
        return 0
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_with_solver_path_succeeds() {
    // Force the solver path by using a private helper with an
    // unannotated parameter. This exercises `collect_block_constraints`
    // for `Try` and `Raise` on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn walrus_with_solver_path_succeeds() {
    // PEP 572 (#774): force the solver path with a private helper that
    // contains a walrus in its body, exercising `constraints::
    // collect_expr_constraints`'s own `HirExpr::NamedExpr` arm -- the
    // solver's own private-helper type-inference path never otherwise
    // recurses into that arm anywhere else in this test suite.
    let src = "\
def _helper(x) -> int:
    if (n := 1) > 0:
        pass
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn walrus_target_read_after_an_if_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): unlike
    // `walrus_with_solver_path_succeeds` above (which never reads `n` after
    // the `if`, so it exercises `collect_expr_constraints`'s own
    // `NamedExpr` arm for coverage without observing whether the binding
    // it produces is actually usable), this test reads `n` in the `return`
    // that follows -- a walrus in an `if`/`while` test always executes
    // whenever the statement itself is reached, so `n` is definitely bound
    // there, not merely maybe-bound the way a name assigned in only one
    // branch would be. Before `bind_named_expr_targets`, the solver path's
    // `HirExpr::Name` arm found `n` in `local_names` (via `collect_named_
    // expr_names_in_expr`) but never in `env.bindings`, producing a false
    // T0021 "not bound" diagnostic for this legal, T0050-conformant code.
    let src = "\
def _helper(x) -> int:
    if (n := 1) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn walrus_target_read_after_a_while_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `While` arm's
    // own call to `bind_named_expr_targets` is a separate line from the
    // `If` arm's -- this repository's 100% line-coverage gate (D-014)
    // needs a solver-path (unannotated private helper) `while` test with a
    // walrus to exercise it, since module-scope code (the existing
    // `while (n := 1) > 0` tests elsewhere in this file) never reaches
    // `constraints::collect_block_constraints` at all -- that solver only
    // runs for a private helper whose signature the annotation-driven fast
    // path cannot resolve.
    let src = "\
def _helper(x) -> int:
    while (n := x) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn bare_expr_stmt_walrus_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the three
    // tests above all place the walrus as an `if`/`while` test, never
    // as a standalone `ExprStmt` -- that leaves `bind_named_expr_
    // targets`'s own call from the `HirStmt::ExprStmt` arm (as opposed
    // to the `If`/`While` arms) unexercised, and none of them exercise
    // its `?`'s error-propagation branch at all (`m` in every prior
    // test's walrus value already has a term by the time it is read,
    // so `collect_expr_constraints` inside `bind_named_expr_targets`
    // never returns `Err` there). `m` here is local to `_helper` (it
    // is assigned two lines below) but not yet bound at the point the
    // walrus reads it, so the recursive `collect_expr_constraints`
    // call inside `bind_named_expr_targets` fails with T0021 before
    // the walrus's own binding step, or the outer `collect_expr_
    // constraints` call on the `ExprStmt` itself, is ever reached.
    let src = "\
def _helper(x) -> int:
    (n := m)
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_inside_an_if_test_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `If` arm's
    // own error-propagation sibling of the test above -- `m` is local
    // but unbound at the point the walrus inside the `if` test reads
    // it, so `bind_named_expr_targets`'s own recursive call fails
    // before `collect_expr_constraints` is reached on `test` itself.
    let src = "\
def _helper(x) -> int:
    if (n := m) > 0:
        pass
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_inside_a_while_test_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `While`
    // arm's own error-propagation sibling of the two tests above.
    let src = "\
def _helper(x) -> int:
    while (n := m) > 0:
        pass
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_unary_op_in_an_if_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): every prior
    // `bind_named_expr_targets` test above places the walrus directly
    // as the `Compare`'s own left operand, never inside another
    // recursive shape -- `bind_named_expr_targets`'s `UnaryOp` arm is
    // otherwise unexercised by this whole test suite. `-(n := 1)`
    // nests the walrus inside a `UnaryOp` within the `if` test, and
    // `n` is read afterward to confirm the binding the pre-pass
    // produces is actually usable (not merely exercised for coverage).
    let src = "\
def _helper(x) -> int:
    if -(n := 1) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

// PEP 572 (#774), deep-review follow-up (round 4), coverage closure: the
// tests above close every *line* in `bind_named_expr_targets`, but 13 of
// its recursive call sites still show an uncovered `?`-error-propagation
// *region* -- every arm besides the top-level `NamedExpr` arm itself only
// ever originates an `Err` by recursing into a nested `NamedExpr`, and no
// existing test places a forward-reference-erroring walrus in most of
// these nested positions. Each test below goes through
// `collect_block_constraints` directly (this repository's established
// low-level solver-path pattern, e.g.
// `collect_block_constraints_propagates_an_error_from_the_initializer_expression`
// above) rather than `parse_check_resolve`, because several of the target
// HIR shapes (`DictGetOrDefault`, `GenericClassInstantiate`) have no
// parseable Python source that constructs them directly. `bind_named_expr_
// targets` itself has no `pub` qualifier, so these tests cannot call it in
// isolation -- `collect_block_constraints`'s `ExprStmt` arm is the only
// reachable entry point, exactly like every other test in this file.
// `m` is always declared local (so `is_local` is true) but never bound, so
// reading it inside a walrus's own value fails with T0021 (`unbound_local`)
// -- an undeclared/global name would instead resolve silently via the
// `Name` arm's `None => Ok(None)` fallback and never error at all.

#[test]
fn walrus_nested_inside_another_walrus_value_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `NamedExpr`'s own recursive `bind_named_expr_targets(value)?`
    // call (source line 974): the outer walrus's value is itself a walrus
    // whose own value forward-references `m`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["n", "m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
        name: "n".to_string(),
        value: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_call_arg_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Call`'s own `bind_named_expr_targets(arg)?` call inside its
    // argument loop (source line 994).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_an_fstring_interpolation_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `FString`'s own `bind_named_expr_targets(inner)?` call
    // inside its interpolation-part loop (source line 1008).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::FString(vec![
        FStringPart::Interpolation(Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        })),
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_list_literal_element_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises the shared `ListLiteral`/`SetLiteral`/`TupleLiteral` arm's
    // own `bind_named_expr_targets(e)?` call inside its element loop
    // (source line 1015).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::ListLiteral(vec![
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_an_object_list_element_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Part 2d of #1371: the same shared element-loop arm, entered through
    // its `ObjectList` alternative (an object-slot list display).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::ObjectList(vec![
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_subscript_base_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Subscript`'s own `bind_named_expr_targets(base)?` call
    // (source line 1020). The index is a harmless literal that is never
    // reached because the base's error propagates first.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        index: Box::new(HirExpr::IntLiteral(0)),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_slice_base_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Slice`'s own `bind_named_expr_targets(base)?` call (source
    // line 1029).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        start: None,
        stop: None,
        step: None,
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_slice_bound_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Slice`'s own `bind_named_expr_targets(bound)?` call inside
    // its `[start, stop, step].into_iter().flatten()` loop (source line
    // 1031). The base is a harmless bare name -- `bind_named_expr_targets`'s
    // `Name` arm never looks the name up, so it always succeeds regardless
    // of binding state -- so the base's own recursion (line 1029) succeeds
    // and the loop is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        })),
        stop: None,
        step: None,
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_literal_key_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `DictLiteral`'s own `bind_named_expr_targets(k)?` call
    // (source line 1040).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
        HirExpr::Name("v".to_string()),
    )]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_literal_value_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `DictLiteral`'s own `bind_named_expr_targets(v)?` call
    // (source line 1041). The key is a harmless bare name so the key's own
    // recursion (line 1040) succeeds and the value is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::Name("k".to_string()),
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    )]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_get_or_default_key_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `DictGetOrDefault`'s own `bind_named_expr_targets(key)?`
    // call (source line 1046).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        default: Box::new(HirExpr::Name("v".to_string())),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_method_call_base_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `MethodCall`'s own `bind_named_expr_targets(base)?` call
    // (source line 1053).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        method: "foo".to_string(),
        args: vec![],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_method_call_arg_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `MethodCall`'s own `bind_named_expr_targets(arg)?` call
    // inside its argument loop (source line 1055). The base is a harmless
    // bare name so the base's own recursion (line 1053) succeeds and the
    // argument loop is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::Name("xs".to_string())),
        method: "foo".to_string(),
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_generic_class_instantiate_arg_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `GenericClassInstantiate`'s own `bind_named_expr_targets(arg)?`
    // call inside its argument loop (source line 1061).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
        class: "C".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn raise_with_solver_path_succeeds() {
    // Force the solver path with a private helper that has a raise.
    let src = "\
def _helper(x) -> int:
    if x == 0:
        raise ValueError(\"bad\")
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_with_generic_call_on_solver_path_succeeds() {
    // Force the solver path and exercise generic call rewriting
    // inside a try body.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def _helper(x) -> int:
    try:
        c = C[int](x)
        return c.x
    except ValueError:
        return 0
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_from_with_solver_path_succeeds() {
    // Force the solver path with a raise-from inside a function.
    let src = "\
def _helper(x) -> int:
    if x == 0:
        raise ValueError(\"bad\") from RuntimeError(\"cause\")
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn bare_raise_with_solver_path_succeeds() {
    // Force the solver path with a bare raise inside an except handler.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        raise
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_variable_bound_to_exception_instance_checks_successfully() {
    // `raise e` where `e` is bound to a builtin exception instance via
    // `except ValueError as e`. This exercises `check_raise_operand`'s
    // non-Call path: the expression is a Name, not a Call, so it falls
    // through to `infer_expr_in` which returns `Ty::Instance("ValueError")`,
    // and the `is_builtin_exception_class` check passes.
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError as e:
        raise e
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn raise_variable_bound_to_non_exception_is_t0021() {
    // `raise x` where `x` is an int — exercises the `_ =>` arm of
    // `check_raise_operand`'s match on the inferred type.
    let err = parse_check("x = 1\nraise x\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("can only raise"));
}

#[test]
fn solver_try_body_type_error_propagates() {
    // Force the solver path and trigger a type error inside the try
    // body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the try body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x + \"bad\"
    except ValueError:
        y = 0
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    // The error should be about the type mismatch, not about raise.
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_handler_type_error_propagates() {
    // Force the solver path and trigger a type error inside the
    // handler body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the handler body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_else_type_error_propagates() {
    // Force the solver path and trigger a type error inside the else
    // body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the else body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    else:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn solver_try_finally_type_error_propagates() {
    // Force the solver path and trigger a type error inside the
    // finally body. This exercises the `?` error propagation in
    // `collect_block_constraints` for the finally body on the solver path.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    finally:
        y = x + \"bad\"
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn walrus_wrapping_a_generic_call_is_rewritten_and_collected() {
    // PEP 572 (#774): exercises `monomorphize::rewrite_generic_calls_in_expr`'s
    // and `collect_generic_class_instantiations_from_expr`'s own
    // `HirExpr::NamedExpr` arms -- both walks are exhaustive over every
    // `HirExpr` (see their own doc comments), and neither is otherwise
    // reached by any other test in this suite. Hand-built HIR (mirroring
    // this file's other `monomorphize(&hir)` tests, which bypass `check`
    // entirely) with the walrus's own value directly wrapping the call to
    // the generic `identity` function -- one node for both walks to visit.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_wrapping_a_wrong_arity_generic_call_propagates_the_rewrite_error() {
    // The error-propagation sibling of the success test above: exercises
    // `rewrite_generic_calls_in_expr`'s `HirExpr::NamedExpr` arm's own `?`
    // -- a generic call nested inside a walrus's value that itself fails to
    // rewrite. Mirrors
    // `monomorphize_propagates_an_instantiation_error_from_inside_a_function_body`
    // above exactly (wrong-arity call to `identity`, expecting `T0021`),
    // with the call wrapped in a walrus.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
            }),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn walrus_target_read_later_as_a_generic_calls_argument_resolves() {
    // PEP 572 (#774), deep-review follow-up: the opposite direction from
    // `walrus_wrapping_a_generic_call_is_rewritten_and_collected` above --
    // here the walrus is its own statement, and the generic call comes
    // *after*, reading the walrus target as a plain name. This exercises
    // `rewrite_generic_calls_in_stmt`'s `ExprStmt` arm delegating to
    // `rewrite_generic_calls_in_expr`'s `NamedExpr` arm, which must bind
    // `n` into `env` (this pass starts every function body from a fresh
    // `Environment`, not the one `check_and_resolve` already populated) so
    // the later `HirExpr::Name("n")` resolves instead of raising a
    // spurious "not defined" error for code the checker already accepted.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![
            HirStmt::ExprStmt(HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            }),
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::Name("n".to_string())],
            }),
        ],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_target_inside_an_if_test_resolves_in_the_branch_body() {
    // PEP 572 (#774), deep-review follow-up: covers the same
    // fresh-`Environment` gap as the test above, but for a walrus nested
    // inside an `if` test (`rewrite_generic_calls_in_stmt`'s `If` arm calls
    // `rewrite_generic_calls_in_expr` on `test` directly, not through the
    // `ExprStmt` arm) with the read happening in the branch body rather
    // than a sibling statement.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::If {
            test: HirExpr::NamedExpr {
                name: "n".to_string(),
                value: Box::new(HirExpr::IntLiteral(1)),
            },
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "identity".to_string(),
                args: vec![HirExpr::Name("n".to_string())],
            })],
            orelse: vec![],
        }],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let resolved = monomorphize(&hir).unwrap();
    assert_eq!(count_function(&resolved, "0gen_identity__T_int"), 1);
}

#[test]
fn walrus_wrapping_a_generic_call_propagates_a_t0050_error_from_its_own_type() {
    // PEP 572 (#774), deep-review follow-up: `rewrite_generic_calls_in_expr`'s
    // `NamedExpr` arm added an explicit `let ty = infer_expr_in(..)?;` (to
    // capture the type for its own new `env.bind` call) where the arm
    // previously just tail-returned `infer_expr_in(..)` directly -- that
    // turns what was a single fall-through region into a real branch, so
    // the error side needs its own coverage. Hand-built HIR (bypassing
    // `check`, same as the sibling tests above) with a walrus whose value
    // is a `str`, which `infer_expr_in`'s own `NamedExpr` arm rejects with
    // T0050 (walrus values are restricted to non-reference-counted
    // scalars) -- exercising the `?`'s early-return path.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let f = HirItem::Function {
        name: "f".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
            name: "z".to_string(),
            value: Box::new(HirExpr::StringLiteral("bad".to_string())),
        })],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, f],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0050");
}

#[test]
fn generic_call_in_try_else_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the try orelse path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        x = 0
    else:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_try_finally_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the try finally path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        x = 0
    finally:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_raise_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the Raise path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    // A generic class instantiation as the raise expression is not a
    // builtin exception class call, so it falls through to the
    // generic-call rewrite path and then gets rejected as T0021.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn generic_call_in_try_body_rejects_unsupported() {
    // Exercise `reject_generic_calls_in_block` for the Try body path.
    // A generic call in a module-level try body where the generic
    // class is not in scope should be rejected.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
try:
    c = C[int](1)
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_in_raise_cause_rewrites_successfully() {
    // Exercise `rewrite_generic_calls_in_stmt` for the Raise cause path
    // and `collect_generic_class_instantiations_from_stmt` for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise ValueError(\"bad\") from C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_with_as_binding_succeeds() {
    // Force the solver path with `except ... as e` to exercise the
    // `as` binding insertion in the solver's `Try` arm.
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError as e:
        y = 0
    return y
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn raise_inside_function_with_generic_call_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for the `Raise` arm
    // and `collect_generic_class_instantiations_from_stmt` for the
    // same. The key is that the raise is valid (ValueError) and the
    // function also contains a generic call, so monomorphization runs
    // and processes both the generic call and the raise statement.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    c = C[int](1)
    x = c.x
    raise ValueError(\"bad\")
";
    parse_check_resolve(src).expect("the rewritten generic call is followed by a terminal raise");
}

#[test]
fn try_with_generic_call_and_raise_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for both the `Try` and
    // `Raise` arms, and `collect_generic_class_instantiations_from_stmt`
    // for the same. A generic call before the try and a raise after
    // ensures both arms are reached during monomorphization.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    c = C[int](1)
    x = c.x
    try:
        x = 1
    except ValueError:
        x = 0
    raise ValueError(\"bad\")
";
    parse_check_resolve(src).expect("the final raise makes every path terminal");
}

#[test]
fn try_with_generic_call_in_handler_rewrites() {
    // Exercise `rewrite_generic_calls_in_stmt` for the `Try` handler
    // body path and `collect_generic_class_instantiations_from_stmt`
    // for the same.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        c = C[int](1)
        x = c.x
    return x
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_handler_binding_is_visible_during_generic_rewrite() {
    let src = "\
def identity[T](value: T) -> T:
    return value
try:
    raise ValueError(\"bad\")
except ValueError as error:
    raise error from RuntimeError(\"cause\")
else:
    pass
finally:
    pass
";
    parse_check_resolve(src)
        .expect("the validated except-as binding must remain visible during rewriting");
}

#[test]
fn reject_generic_calls_in_try_and_raise() {
    // Exercise `reject_generic_calls_in_block` for both the `Try` and
    // `Raise` arms. A generic call inside a try body and a raise with
    // a generic call in the same module-level scope ensures both arms
    // are reached during the rejection pass.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
try:
    c = C[int](1)
    print(c.x)
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

// -- #382: reject_generic_calls_in_stmt Try and Raise arms --

#[test]
fn reject_generic_calls_processes_try_and_raise_arms() {
    // Exercise `reject_generic_calls_in_stmt`'s `Try` and `Raise` arms
    // directly. A generic function body containing a `Try` and a `Raise`
    // statement ensures both arms are reached during the rejection pass.
    let body = vec![
        HirStmt::Try {
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["ValueError".to_string()]),
                name: None,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            }],
            orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
            finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        },
        HirStmt::Raise {
            exc: Some(HirExpr::Name("some_exc".to_string())),
            cause: Some(HirExpr::Name("some_cause".to_string())),
        },
    ];
    // `reject_generic_calls_in_block` processes each statement through
    // `reject_generic_calls_in_stmt`. The `Try` and `Raise` arms should
    // be reached without error (no generic calls inside them).
    assert!(
        reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok(),
        "reject_generic_calls_in_block should succeed for try/raise without generic calls"
    );
}

#[test]
fn reject_generic_calls_processes_try_star_arm() {
    // Exercise `reject_generic_calls_in_stmt`'s `HirStmt::TryStar` half of
    // its combined `Try { .. } | TryStar { .. }` arm (#542) directly --
    // `reject_generic_calls_processes_try_and_raise_arms` above only
    // reaches the `Try` alternative's own field-destructure lines.
    let body = vec![HirStmt::TryStar {
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        handlers: vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }],
        orelse: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        finalbody: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
    }];
    assert!(
        reject_generic_calls_in_block(&Environment::new(), "f", &body).is_ok(),
        "reject_generic_calls_in_block should succeed for a try* without generic calls"
    );
}

// -- #382: check_stmt_sequence_shared ? error propagation in try bodies --

#[test]
fn check_try_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1 + \"bad\"\n    except ValueError:\n        x = 0\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_handler_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_else_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 0\n    else:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn check_try_finally_body_type_error_propagates() {
    let err = parse_check(
            "def f() -> int:\n    try:\n        x = 1\n    except ValueError:\n        x = 0\n    finally:\n        x = 1 + \"bad\"\n    return x\n",
        ).unwrap_err();
    assert!(err.message.contains("operator"));
}

// -- #382: join_if_branches ? error propagation --

#[test]
fn try_handler_with_incompatible_binding_is_t0023() {
    // A handler that rebinds a pre-existing variable to an incompatible
    // type triggers `join_if_branches`'s T0023 error path.
    let err = parse_check(
            "def f() -> int:\n    x = 1\n    try:\n        x = 2\n    except ValueError:\n        x = \"bad\"\n    return x\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn try_else_with_incompatible_binding_is_t0023() {
    // An else body that rebinds a pre-existing variable to an
    // incompatible type triggers `join_if_branches`'s T0023 error path.
    let err = parse_check(
            "def f() -> int:\n    x = 1\n    try:\n        x = 2\n    except ValueError:\n        x = 3\n    else:\n        x = \"bad\"\n    return x\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

// -- #382: join_if_branches ? error propagation via `except ... as` --
// The `as` binding uses `henv.bind()` directly (no `check_assignment`),
// so a type mismatch between the pre-try binding and the `as` binding
// is caught by `join_if_branches`, not by `check_assignment`.

#[test]
fn try_handler_as_binding_incompatible_type_is_t0023() {
    // `e` is `int` before the try; `except ValueError as e` rebinds `e`
    // to `Instance("ValueError")` via `henv.bind()` (bypassing
    // `check_assignment`). `join_if_branches` detects the mismatch.
    let err = parse_check(
            "def f() -> int:\n    e = 0\n    try:\n        pass\n    except ValueError as e:\n        pass\n    return e\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

#[test]
fn try_star_handler_as_binding_incompatible_type_is_t0023() {
    // Mirrors `try_handler_as_binding_incompatible_type_is_t0023` for
    // `except*` (#542). A same-block reassignment (`x = "bad"`) is caught
    // directly by `check_assignment` (via `check_stmt_sequence_shared`) before
    // `check_try_star_stmt` ever reaches its own
    // `join_if_branches(&mut joined, &previous, handler_env)?` loop, so
    // that specific `?` needs a mismatch that bypasses `check_assignment`
    // instead: `e` is `int` before the try; `except* ValueError as e`
    // rebinds `e` to `Instance("ExceptionGroup")` via `henv.bind()`
    // directly (exactly like the plain-`try` case), which
    // `join_if_branches` then detects as incompatible with the pre-try
    // binding.
    let err = parse_check(
            "def f() -> int:\n    e = 0\n    try:\n        pass\n    except* ValueError as e:\n        pass\n    return 1\n",
        ).unwrap_err();
    assert_eq!(err.code, "T0023");
}

// -- #382: check_raise_operand ? error propagation --

#[test]
fn raise_with_type_error_in_argument_propagates() {
    // `raise ValueError(1 + "bad")` — the argument expression has a type
    // error, which propagates through the `?` in `check_raise_operand`'s
    // builtin-exception-class-call path.
    let err = parse_check("raise ValueError(1 + \"bad\")\n").unwrap_err();
    assert!(err.message.contains("operator"));
}

#[test]
fn raise_with_type_error_in_non_call_expr_propagates() {
    // `raise 1 + "bad"` — the expression is not a Call, so it falls
    // through to `infer_expr_in` which returns a type error, propagating
    // through the `?` in `check_raise_operand`'s non-Call path.
    let err = parse_check("raise 1 + \"bad\"\n").unwrap_err();
    assert!(err.message.contains("operator"));
}

// -- #382: block_always_returns with non-empty else body --

#[test]
fn try_with_return_in_body_handler_and_else_always_returns() {
    // A try where body, all handlers, and else all return — the try
    // block always returns. This covers the `block_always_returns(orelse)`
    // branch of the `||` in `block_always_returns`'s `Try` arm.
    let src = "\
def f() -> int:
    try:
        return 1
    except ValueError:
        return 2
    else:
        return 3
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_star_with_return_in_body_handler_and_else_always_returns() {
    // Mirrors `try_with_return_in_body_handler_and_else_always_returns`
    // for `except*` (#542): covers the `HirStmt::TryStar` half of
    // `block_always_returns`'s combined `Try { .. } | TryStar { .. }`
    // pattern -- the `Try` arm's own field-destructure lines are already
    // exercised by the plain-`try` test above, but the `TryStar`
    // alternative's own lines are only reached when the analyzed function
    // actually contains a `try*`.
    //
    // #795 (PEP 654): the handler terminates with a `raise` rather than a
    // `return` -- CPython rejects a `return` inside an `except*` clause body
    // outright, and pycc now reports `L0001` for it during lowering, so this
    // fixture could no longer reach type checking at all. `HirStmt::Raise`
    // is `block_always_returns`'s other terminal statement, so the
    // `handled_paths_terminate` conjunct this test exists to exercise is
    // still driven to `true`.
    let src = "\
def f() -> int:
    try:
        return 1
    except* ValueError:
        raise RuntimeError(\"boom\")
    else:
        return 3
";
    parse_check(src).expect("check should succeed");
}

// -- #382: collect_generic_class_instantiations Raise arm --

#[test]
fn collect_generic_class_instantiations_from_raise() {
    // A generic class instantiation inside a raise expression exercises
    // the `Raise` arm of `collect_generic_class_instantiations_from_stmt`.
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
raise C[int](1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

// -- #382: collect_block_constraints ? propagation on the solver path --
// These tests force the solver path (via an untyped `_helper(x)`
// parameter) and trigger an immediate error in `collect_expr_constraints`
// (by calling a non-callable binding), which propagates through the `?`
// in the `Try` arm of `collect_block_constraints`.

#[test]
fn solver_try_body_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y()
    except ValueError:
        y = 0
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_handler_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_else_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    else:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn solver_try_finally_non_callable_error_propagates() {
    let src = "\
def _helper(x) -> int:
    y = 0
    try:
        y = x
    except ValueError:
        y = 0
    finally:
        y()
    return y
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

// ------------------------------------------------------------------
// PEP 560 (#610): value-position `C[x]` dispatches to
// `C.__class_getitem__(x)`.
// ------------------------------------------------------------------

const CLASS_GETITEM_STATIC: &str = "\
class C:
    def __init__(self) -> None:
        self.x = 1
    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key + 1
";

#[test]
fn class_getitem_dispatches_a_static_hook_in_value_position() {
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[3]\n");
    assert!(
        parse_check(&src).is_ok(),
        "`C[3]` must resolve through the class's own `__class_getitem__`"
    );
}

#[test]
fn class_getitem_dispatch_survives_the_generic_rewrite_pass() {
    // `check_and_resolve` runs `rewrite_generic_calls_in_expr`, whose
    // `Subscript` arm must not recurse into the bare class-name base:
    // doing so would infer it as a value and fail with T0021.
    //
    // The generic function is load-bearing: `monomorphize` returns early
    // for a module that declares no generic function and no protocol
    // parameter, and the rewrite pass would never run at all without it.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = C[3]
"
    );
    assert!(parse_check_resolve(&src).is_ok());
}

#[test]
fn a_class_name_attribute_read_survives_the_generic_rewrite_pass() {
    // The `AttrGet` arm of `rewrite_generic_calls_in_expr` shares the
    // `Subscript` arm's problem: `A.X` names the class in a position that is
    // not a value expression, so recursing into the base infers a bare class
    // name and fails with T0021. As above, the generic function is
    // load-bearing -- without it `monomorphize` returns before the rewrite
    // pass runs.
    let src = "\
class A:
    X: int = 2

def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = A.X
";
    assert!(parse_check_resolve(src).is_ok());
}

#[test]
fn a_class_name_static_method_call_survives_the_generic_rewrite_pass() {
    // The `MethodCall` arm's own spelling of the same skip: `A.m()` calls a
    // `@staticmethod` through the class name, so its base is not a value
    // expression either.
    let src = "\
class A:
    @staticmethod
    def m() -> int:
        return 2

def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = A.m()
";
    assert!(parse_check_resolve(src).is_ok());
}

#[test]
fn class_getitem_index_error_propagates_through_the_generic_rewrite_pass() {
    // The rewrite pass still descends into the *index* of a class-name
    // subscript, so an instantiation error there must surface rather than
    // being skipped along with the base.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def ident[T](x: T) -> T:
    return x

value: int = C[ident(1, 2)]
"
    );
    assert_eq!(parse_check_resolve(&src).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_subscript_index() {
    // The rewrite pass descends into a subscript's index regardless of
    // whether its base is a bare class name, so an instantiation error
    // there must propagate. `check_and_resolve` rejects the bad call
    // before `monomorphize` ever sees it, so the module is built by hand
    // and handed to `monomorphize` directly, as the sibling
    // error-propagation tests above already do.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
        index: Box::new(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        }),
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn class_getitem_propagates_an_index_inference_error() {
    // The index is inferred before the hook is resolved, so an undefined
    // name there is reported as such instead of reaching the class hook.
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[missing]\n");
    assert_eq!(parse_check(&src).unwrap_err().code, "T0021");
}

#[test]
fn class_getitem_dispatches_a_classmethod_hook_in_value_position() {
    // CPython makes `__class_getitem__` an implicit classmethod, so the
    // explicit `@classmethod` spelling must resolve too.
    let src = "\
class C:
    def __init__(self) -> None:
        self.x = 1
    @classmethod
    def __class_getitem__(cls, key: int) -> int:
        return key + 1

value: int = C[3]
";
    assert!(parse_check(src).is_ok());
}

#[test]
fn class_getitem_result_type_is_the_hook_s_return_type() {
    // Binding the result to a mismatched annotation must fail, proving
    // the subscript's type comes from the hook rather than from a
    // container-element fallback.
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: str = C[3]\n");
    assert!(parse_check(&src).is_err());
}

#[test]
fn class_getitem_argument_type_is_checked_against_the_hook() {
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[\"k\"]\n");
    assert!(parse_check(&src).is_err());
}

#[test]
fn class_getitem_value_position_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override()
 {
    // Issue #693 deep-review, Finding 1: `resolve_static_or_class_method_call`
    // walks the full MRO checking `static_methods` first, and only if none
    // of the MRO's classes declare the hook there does it walk the full MRO
    // a second time checking `class_methods` -- so a base class's
    // `@staticmethod` hook always outranks a derived class's `@classmethod`
    // override, regardless of MRO order. `pycc_hir`'s annotation-position
    // resolver (`class_getitem_return_ty`) must replicate this exact
    // two-pass order rather than combining both tables into one MRO pass;
    // see `pycc_hir::tests::an_annotation_subscript_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override`
    // for the annotation-position half of this regression pair -- both
    // must resolve `D[3]` to `int` (the base's hook), never `str` (the
    // derived override).
    let src = "\
class C:
    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key
    def __init__(self) -> None:
        self.x = 1

class D(C):
    @classmethod
    def __class_getitem__(cls, key: int) -> str:
        return \"overridden\"

value: int = D[3]
";
    assert!(
        parse_check(src).is_ok(),
        "`D[3]` must resolve through the base class `C`'s `@staticmethod` \
         hook (return type `int`), not `D`'s own `@classmethod` override \
         (return type `str`)"
    );
}

#[test]
fn subscripting_a_class_without_the_hook_is_rejected() {
    // CPython raises `TypeError: type 'C' is not subscriptable` here.
    let src = "\
class C:
    def __init__(self) -> None:
        self.x = 1

value: int = C[3]
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn a_value_binding_shadowing_a_class_name_indexes_as_a_value() {
    // Only *type aliases* collide with a class name in `pycc_hir`, so a
    // plain rebinding is accepted and `C[0]` must then read the list
    // element rather than dispatching a class hook.
    let src = format!("{CLASS_GETITEM_STATIC}\nC = [1, 2, 3]\nvalue: int = C[0]\n");
    assert!(
        parse_check(&src).is_ok(),
        "a rebound class name must index as the value it now holds"
    );
}

#[test]
fn a_local_shadowing_a_class_name_indexes_as_a_value() {
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def f() -> int:
    C = [1, 2, 3]
    return C[0]

value: int = f()
"
    );
    assert!(parse_check(&src).is_ok());
}

// -- #974 round 5: a module-scope rebinding of a class's own name ----
//
// A function body is checked (D-041 late binding) against the module
// environment as it stands after *all* top-level code has run, so
// `binding_state` cannot tell a rebinding that executes before the call
// from one that executes after it. CPython answers those two differently
// (`1` versus `2`), so neither resolution is correct for both and the read
// is rejected with `C0001` instead -- see `expr::class_name_dispatch`.
//
// All four shapes below are rejected: rebind-before and rebind-after,
// crossed with a class attribute the class declares itself and one it
// inherits. The `Subscript` and `MethodCall` dispatch sites carry the same
// guard and get one case each. The parameter-shadow case that closes the
// set is the round-2/3 behavior this rejection must **not** widen to.

/// A class, a same-shaped stand-in to rebind its name to, and a function
/// reading the class attribute through the bare class name.
const REBIND_OWN_ATTR: &str = "\
class A:
    X: int = 2

class D:
    def __init__(self) -> None:
        self.X = 1

def f() -> int:
    return A.X
";

/// [`REBIND_OWN_ATTR`] with the read going through a *derived* class, so
/// the attribute resolves through the MRO walk #974 added.
const REBIND_INHERITED_ATTR: &str = "\
class A:
    X: int = 2

class B(A):
    pass

class D:
    def __init__(self) -> None:
        self.X = 1

def f() -> int:
    return B.X
";

#[test]
fn rebinding_a_class_name_after_the_read_is_rejected() {
    // CPython prints `2` here: `f()` runs before `A = D()` executes.
    let src = format!("{REBIND_OWN_ATTR}\nprint(f())\nA = D()\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_before_the_read_is_rejected() {
    // CPython prints `1` here: `A` is the `D` instance by the time `f()`
    // runs. The environment pass 3 checks against is identical to the case
    // above, which is exactly why neither can be resolved.
    let src = format!("{REBIND_OWN_ATTR}\nA = D()\nprint(f())\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_derived_class_name_after_the_read_is_rejected() {
    let src = format!("{REBIND_INHERITED_ATTR}\nprint(f())\nB = D()\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_derived_class_name_before_the_read_is_rejected() {
    let src = format!("{REBIND_INHERITED_ATTR}\nB = D()\nprint(f())\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_read_through_class_getitem_is_rejected() {
    // The `Subscript` dispatch site carries the same guard as the
    // `AttrGet` one above; inside a function body it rejects too.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def f() -> int:
    return C[0]

C = [1, 2, 3]
print(f())
"
    );
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_called_as_a_static_method_is_rejected() {
    // The `MethodCall` dispatch site, third of the three guards.
    let src = "\
class A:
    @staticmethod
    def m() -> int:
        return 2

class D:
    def __init__(self) -> None:
        self.x = 1

def f() -> int:
    return A.m()

A = D()
print(f())
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_parameter_shadowing_a_class_name_still_reads_the_parameter() {
    // The round-2/3 behavior the rejection above must not widen to: a
    // *parameter* (like a function-local) shadows the class without any
    // ordering ambiguity, because it is bound at the call, not by module
    // top-level code.
    let src = "\
class B:
    X: int = 2

class D:
    def __init__(self) -> None:
        self.X = 1

def f(B: D) -> int:
    return B.X

print(f(D()))
";
    assert!(
        parse_check(src).is_ok(),
        "a parameter shadowing a class name must still read the parameter"
    );
}

// -- #603: a unary operand is a general expression -------------------
//
// The unary arms these cases drive live in this crate's generic-call,
// generic-class, and protocol walkers. `tests/issue_603_unary_general_operand.rs`
// already pins the observable program output end to end; these unit
// tests exist so the same arms are executed by this crate's own test
// binary too, which is what the D-014 region gate actually measures.

#[test]
fn a_unary_operand_composes_with_a_generic_call_in_both_directions() {
    let result = check_source(
        "def ident[T](v: T) -> T:\n    return v\n\n\nx = 5\nprint(ident(-x))\nprint(-ident(x))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a generic call: {result:?}"
    );
}

#[test]
fn a_unary_operand_inside_a_generic_function_body_is_walked_through() {
    let result = check_source(
        "def negate[T](v: T, n: int) -> int:\n    return -n\n\n\nprint(negate(1, 5))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand inside a generic body must resolve: {result:?}"
    );
}

#[test]
fn a_unary_operand_composes_with_a_generic_class_instantiation() {
    let result = check_source(
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n    def size(self) -> int:\n        return 1\n\n\nb = Box(5)\nprint(-b.size())\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a generic class instantiation: {result:?}"
    );
}

#[test]
fn a_unary_operand_composes_with_a_protocol_dispatched_call() {
    let result = check_source(
        "from typing import Protocol\n\n\nclass Scaled(Protocol):\n    def scale(self) -> int: ...\n\n\nclass Two:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def scale(self) -> int:\n        return self.v * 2\n\n\ndef apply(s: Scaled) -> int:\n    return -s.scale()\n\n\nprint(apply(Two(4)))\n",
    );
    assert!(
        result.is_ok(),
        "a unary operand must compose with a protocol call: {result:?}"
    );
}

#[test]
fn an_error_inside_a_unary_operand_propagates_through_a_generic_body() {
    let result = check_source(
        "def negate[T](v: T, n: int) -> int:\n    return -missing(n)\n\n\nprint(negate(1, 5))\n",
    );
    assert!(
        result.is_err(),
        "an unresolvable unary operand must be rejected: {result:?}"
    );
}

#[test]
fn an_error_inside_a_unary_operand_propagates_at_module_level() {
    let result = check_source("x = -missing(1)\n");
    assert!(
        result.is_err(),
        "an unresolvable unary operand must be rejected: {result:?}"
    );
}

#[test]
fn an_unbound_local_under_a_unary_in_a_generic_body_propagates() {
    let result = check_source(
        "def negate[T](v: T) -> int:\n    a = -b\n    b = 1\n    return a\n\n\nprint(negate(1))\n",
    );
    let message = result
        .expect_err("an unbound local must be rejected")
        .message;
    assert!(
        message.contains("local name `b` is not bound"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_failing_generic_call_under_a_unary_propagates() {
    let result =
        check_source("def ident[T](v: T) -> T:\n    return v\n\n\nx = -ident(\"s\")\nprint(x)\n");
    assert!(
        result.is_err(),
        "a unary over a `str`-instantiated generic call must be rejected: {result:?}"
    );
}

// A unary whose operand is still an inference variable cannot be checked
// with `unary_result_type` yet, so the solver defers it as the exact
// binary constraint `pycc_mir` lowers it to. Both operators take that
// path, and only an unannotated helper parameter -- whose type comes from
// signature inference rather than an annotation -- reaches it.

#[test]
fn a_negated_inferred_parameter_defers_to_a_subtraction_constraint() {
    let result = check_source("def _neg(n):\n    return -n\n\n\nx: int = 4\nprint(_neg(x))\n");
    assert!(
        result.is_ok(),
        "a negated inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

#[test]
fn a_unary_plus_inferred_parameter_defers_to_an_addition_constraint() {
    let result = check_source("def _pos(n):\n    return +n\n\n\nx: int = 4\nprint(_pos(x))\n");
    assert!(
        result.is_ok(),
        "a unary-plus inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

#[test]
fn a_deferred_unary_constraint_still_rejects_a_non_numeric_operand() {
    let result = check_source("def _bad(s):\n    return -s\n\n\nt = \"ab\"\nprint(_bad(t))\n");
    assert!(
        result.is_err(),
        "a deferred unary constraint over `str` must be rejected: {result:?}"
    );
}

// #604 (Part 3 of #573). `~` shares the same deferred `Sub`-shaped path as
// `-`/`+` above (only the `matches!(op, UnaryOpKind::UAdd)` check tells them
// apart), so this is the `Invert` counterpart of
// `a_negated_inferred_parameter_defers_to_a_subtraction_constraint`.
#[test]
fn a_bitwise_invert_inferred_parameter_defers_to_a_subtraction_constraint() {
    let result = check_source("def _inv(n):\n    return ~n\n\n\nx: int = 4\nprint(_inv(x))\n");
    assert!(
        result.is_ok(),
        "an inverted inferred parameter must resolve through the deferred constraint: {result:?}"
    );
}

// #604: `not` has its own dedicated arm in `collect_expr_constraints`
// (unlike `-`/`+`/`~`, it never defers to a binary constraint at all -- its
// result is always `Ty::Bool` regardless of the operand's resolved type),
// exercised here through an inferred parameter specifically so the operand
// is still an unresolved inference variable when this arm runs.
#[test]
fn a_logical_not_inferred_parameter_always_resolves_to_bool() {
    let result = check_source("def _not(n):\n    return not n\n\n\nx: int = 4\nprint(_not(x))\n");
    assert!(
        result.is_ok(),
        "a logical-not inferred parameter must resolve to bool: {result:?}"
    );
}

// -- D-197 (#763, Part 1 of #747): `Ty::Optional`, `T | None` assignability,
// and `is`/`is not` on `None` ------------------------------------------

#[test]
fn a_bare_int_value_widens_to_an_optional_int_annotation() {
    // `is_assignable`'s one-way `Ty::Optional` widening rule: a bare `int`
    // value is assignable to an `int | None` annotation.
    let result = check_source("x: int | None = 5\nprint(x is None)\n");
    assert!(
        result.is_ok(),
        "a bare int must widen to `int | None`: {result:?}"
    );
}

#[test]
fn a_none_literal_widens_to_an_optional_int_annotation() {
    let result = check_source("x: int | None = None\nprint(x is not None)\n");
    assert!(
        result.is_ok(),
        "`None` must widen to `int | None`: {result:?}"
    );
}

#[test]
fn reversed_operand_order_none_or_int_is_equivalent_to_int_or_none() {
    let result = check_source("x: None | int = None\nprint(x is None)\n");
    assert!(
        result.is_ok(),
        "`None | int` must accept the same values as `int | None`: {result:?}"
    );
}

#[test]
fn an_optional_int_value_narrows_back_to_a_bare_int_annotation_inside_is_not_none() {
    // Superseded by issue #769 (Part 2 of #747, D-205): flow-sensitive
    // narrowing of a top-level `is not None` test now exists, so an
    // `Optional[int]` read *inside* the `is not None` body resolves to
    // plain `int` and is assignable to a bare `int`-annotated target. See
    // `narrowing`'s own test module for the full narrowing test suite;
    // this one specifically guards the exact shape this test used to
    // assert was rejected, so a future regression here is caught by name.
    let result =
        check_source("x: int | None = 5\nif x is not None:\n    y: int = x\n    print(y)\n");
    assert!(
        result.is_ok(),
        "a narrowed `Optional[int]` read inside `is not None` must be assignable to plain `int`: {result:?}"
    );
}

#[test]
fn is_none_on_an_optional_int_type_checks_as_bool() {
    let result = check_source(
        "def f(flag: bool) -> int | None:\n    if flag:\n        return 5\n    return None\n\n\nprint(f(True) is None)\n",
    );
    assert!(
        result.is_ok(),
        "`is None` against an `Optional[int]` expression must type-check: {result:?}"
    );
}

#[test]
fn is_not_none_on_a_plain_none_typed_expression_type_checks_as_bool() {
    let result = check_source("x = None\nprint(x is not None)\n");
    assert!(
        result.is_ok(),
        "`is not None` against a `None`-typed expression must type-check: {result:?}"
    );
}

#[test]
fn is_none_against_a_non_optional_operand_is_rejected_with_t0021() {
    // HIR lowering already guarantees one operand of an `is`/`is not`
    // comparison is syntactically `None` (`Expr::Compare`'s `is_none_operand_shape`
    // check); the *other* operand's static type must be `Ty::Optional(_)` or
    // `Ty::None` for `infer_expr_in`'s `Compare` arm to accept it here.
    let result = check_source("x: int = 5\nprint(x is None)\n");
    let err = result.expect_err("`is None` against a plain `int` must be rejected");
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("is") && err.message.to_lowercase().contains("none"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn is_not_none_against_a_str_operand_is_rejected_with_t0021() {
    let result = check_source("x: str = \"a\"\nprint(x is not None)\n");
    let err = result.expect_err("`is not None` against a plain `str` must be rejected");
    assert_eq!(err.code, "T0021");
}

#[test]
fn none_on_the_left_of_is_still_reads_the_other_operands_type() {
    // Mirrors `is_none_on_an_optional_int_type_checks_as_bool` but with the
    // `None` literal as the *left* operand (`None is x`) instead of the
    // right (`x is None`). `infer_expr_in`'s `Compare` arm picks the "other"
    // operand's type from whichever side is *not* the `NoneLiteral`, so this
    // exercises the `left` branch of that selection specifically.
    let result = check_source("x: int | None = 5\nprint(None is x)\n");
    assert!(
        result.is_ok(),
        "`None is x` against an `Optional[int]` must type-check as bool: {result:?}"
    );
}

// -- PEP 572 (#774): the walrus assignment (`:=`) value-type restriction
// (T0050) -----------------------------------------------------------

#[test]
fn a_walrus_with_an_int_value_type_checks() {
    let result = check_source("if (n := 7) > 5:\n    print(n)\n");
    assert!(
        result.is_ok(),
        "an `int`-valued walrus must type-check: {result:?}"
    );
}

#[test]
fn a_walrus_with_an_optional_int_value_type_checks() {
    // `is_walrus_value_ty_supported`'s `Ty::Optional(inner)` arm recurses
    // into `inner` -- here an `Optional[int]` walrus value, reached via a
    // `Name` already declared `int | None`.
    let result = check_source("x: int | None = 5\nif (y := x) is not None:\n    print(y)\n");
    assert!(
        result.is_ok(),
        "an `Optional[int]`-valued walrus must type-check: {result:?}"
    );
}

#[test]
fn a_walrus_with_a_str_value_is_rejected_with_t0050() {
    let result = check_source("s = \"hi\"\nif (y := s) == \"hi\":\n    print(y)\n");
    let err = result.expect_err("a `str`-valued walrus must be rejected");
    assert_eq!(err.code, "T0050");
}

#[test]
fn a_walrus_with_an_optional_str_value_is_rejected_with_t0050() {
    // `is_walrus_value_ty_supported`'s `Ty::Optional(inner)` arm recurses
    // into `inner` and, for an unsupported inner type such as `str`,
    // reports unsupported -- the region gap this closes is that recursive
    // `false` return specifically (the sibling `Optional[int]` test above
    // only exercises the recursive `true` return). A real parsed program
    // can never reach this: `annotation_to_ty` (`pycc_hir/src/func.rs`) is
    // the one place a `Ty::Optional` is ever constructed from source, and
    // it rejects every inner type but `int`/`float`/`bool` at lowering
    // time with T0049,
    // before `pycc_types` ever runs -- so `Ty::Optional(Ty::Str)` only
    // exists as a hand-built `Environment` binding here, bypassing
    // `check_source`/lowering entirely to reach the type-checker's own
    // `is_walrus_value_ty_supported` directly.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Optional(Box::new(Ty::Str)));
    let expr = HirExpr::NamedExpr {
        name: "y".to_string(),
        value: Box::new(HirExpr::Name("x".to_string())),
    };
    let err =
        infer_expr(&env, &expr).expect_err("an `Optional[str]`-valued walrus must be rejected");
    assert_eq!(err.code, "T0050");
}

#[test]
fn a_walrus_whose_value_fails_to_infer_propagates_the_error_before_the_t0050_check() {
    // `infer_expr_in`'s own `NamedExpr` arm (`crates/pycc_types/src/expr.rs`)
    // computes `value`'s type via its own `infer_expr_in(..)?` call before
    // ever reaching `is_walrus_value_ty_supported` -- that `?`'s error
    // branch is distinct from `collect_named_expr_bindings`'s identically-
    // shaped call in `lib.rs` (already covered elsewhere in this file):
    // `collect_named_expr_bindings` always runs first, over the same
    // `value`, so any error reachable through an ordinary `check_source`
    // program is always caught there first, never here. Reaching this
    // arm's own failure requires calling `infer_expr` directly (as the
    // sibling `Optional[str]` test above does) on a `NamedExpr` whose
    // `value` is a plain undefined-name read.
    let env = Environment::new();
    let expr = HirExpr::NamedExpr {
        name: "y".to_string(),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).expect_err("an undefined-name walrus value must be rejected");
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_walrus_in_a_generic_functions_body_does_not_trip_the_t0042_self_call_scan() {
    // `reject_generic_calls_in_expr`'s own `HirExpr::NamedExpr` arm (#774):
    // a generic function's body is pre-scanned for illegal self-calls
    // (D-134/T0042) before it is type-checked at all, and that scan walks
    // every `HirExpr` including a bare walrus expression statement.
    let result = check_source("def identity[T](value: T) -> T:\n    (z := 1)\n    return value\n");
    assert!(
        result.is_ok(),
        "a walrus with no generic self-call must not trip T0042: {result:?}"
    );
}

#[test]
fn a_walrus_nested_inside_a_list_slice_and_dict_literal_binds_every_name() {
    // Module-level `collect_named_expr_bindings` walk (lib.rs), exercising
    // its `ListLiteral`/`SetLiteral`/`TupleLiteral`, `Slice`, and
    // `DictLiteral` arms in one bare expression statement -- mirroring
    // `pycc_mir`'s own equivalent walk test for the same #774 nesting shape.
    let result = check_source(
        "lst = [1, 2, 3]\n[a := 1]\nlst[(b := 0):]\n{\"k\": (c := 3)}\nprint(a)\nprint(b)\nprint(c)\n",
    );
    assert!(
        result.is_ok(),
        "every nested walrus target must be bound and visible afterward: {result:?}"
    );
}

// The test above only exercises `collect_named_expr_bindings`'s
// `ListLiteral`/`SetLiteral`/`TupleLiteral`, `Slice`, and `DictLiteral`
// arms. This one covers every other container-shaped arm at module scope:
// `Call` args, `BinOp`, `Compare`, an `FString` interpolation, a
// non-`Slice` `Subscript`, `ListAppend`, `SetAdd`, `AttrGet` (via a `Call`
// base), `MethodCall` args, `GenericClassInstantiate` args, and
// `DictGetOrDefault`.
//
// The fixture deliberately includes a generic class (`Box[T]`) so that
// `monomorphize`'s top-level rewrite pass actually runs (its early-return
// fast path is skipped whenever the module has a generic function/class or
// a protocol-typed parameter): `rewrite_generic_calls_in_expr`'s own
// `NamedExpr` arm calls `env.bind`, so a module-level walrus name bound by
// an earlier bare statement stays visible to a later top-level statement's
// name lookup inside that pass, exactly like the ordinary
// `check`/`check_and_resolve` path. Goes through `check_source` (full
// `check_and_resolve`, `monomorphize` included) rather than `check` alone,
// so a regression in either pass's env bookkeeping fails this test.
#[test]
fn a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope() {
    let result = check_source(
        "\
class Box[T]:
    def __init__(self, x: T):
        self.x = x
class C:
    def __init__(self, x: int):
        self.x = x
    def m(self, y: int) -> int:
        return y
def f(v: int) -> int:
    return v
def make_c(v: int) -> C:
    return C(v)
c = C(3)
lst = [1, 2, 3]
s = {1, 2}
dct = {\"k\": 2}
print(f((a := 1)))
print((b := 2) + 1)
print((dd := 3) == 3)
print(f\"{(e := 4)}\")
print(lst[(g := 0)])
lst.append((h := 5))
s.add((i := 6))
print(make_c((j := 0)).x)
print(c.m((k := 7)))
print(Box[int]((l := 8)).x)
print(dct.get(f\"k{(m := 1)}\", 0))
{(p := 9)}
((q := 10),)
print(a, b, dd, e, g, h, i, j, k, l, m, p, q)
",
    );
    assert!(
        result.is_ok(),
        "every nested walrus target across every container shape must be bound and visible \
         afterward: {result:?}"
    );
}

// PEP 572 (#774): `collect_named_expr_bindings` is called from six call
// sites -- module-scope `check_stmt`'s `ExprStmt`/`If`/`While` arms and
// function-scope `check_stmt_in_function`'s same three arms -- each behind
// a `?`. These six tests exercise that error-propagation branch at each
// site: `n` is first bound `str`, then a nested walrus tries to rebind it
// `int` (a `T0050`-supported value type), which `check_assignment` rejects
// as an incompatible redefinition (`T0023`).
#[test]
fn a_failing_walrus_binding_in_a_module_bare_expr_stmt_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_module_if_test_propagates_t0023() {
    let err = check_source("n = \"hi\"\nif (n := 1) > 0:\n    pass\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_module_while_test_propagates_t0023() {
    let err = check_source("n = \"hi\"\nwhile (n := 1) > 0:\n    pass\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_bare_expr_stmt_propagates_t0023() {
    let err = check_source("def f() -> int:\n    n = \"hi\"\n    (n := 1)\n    return 0\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_if_test_propagates_t0023() {
    let err = check_source(
        "def f() -> int:\n    n = \"hi\"\n    if (n := 1) > 0:\n        pass\n    return 0\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_while_test_propagates_t0023() {
    let err = check_source(
        "def f() -> int:\n    n = \"hi\"\n    while (n := 1) > 0:\n        pass\n    return 0\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

// PEP 572 (#774): the comprehensive
// `a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope`
// test above only exercises `collect_named_expr_bindings`'s recursive `?`
// propagation on its *success* path for every container shape -- each
// recursive `collect_named_expr_bindings(env, local_names, ..)?` call is its
// own coverage region distinct from the call's success path, and none of
// them is exercised when every nested walrus binds cleanly. These tests
// each nest a *failing* walrus (`n` already bound `str`, rebound `int`) one
// level inside a specific container shape so that shape's own recursive `?`
// takes its error branch instead. Every fixture here is expected to fail
// during `check`, before `check_and_resolve` ever reaches `monomorphize`.
#[test]
fn a_failing_walrus_nested_inside_a_named_exprs_own_value_propagates_t0023() {
    // Exercises the `NamedExpr` arm's own recursive call into `value`
    // (line ~996) failing, as opposed to `check_assignment` on the outer
    // name failing -- the outer name `m` binds cleanly; it's the *inner*
    // `n` rebind nested inside `m`'s value that fails.
    let err = check_source("n = \"hi\"\n(m := (n := 1))\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_type_error_inside_a_named_exprs_own_value_propagates_through_infer_expr_in() {
    // Exercises the `NamedExpr` arm's own `infer_expr_in(env, local_names,
    // value)` call (line ~997) failing on a `value` that contains no
    // nested walrus at all (so the preceding `collect_named_expr_bindings`
    // recursion succeeds trivially) but is otherwise ill-typed.
    let err = check_source("(m := 1 + \"x\")\n").expect_err("adding int and str is a type error");
    assert_ne!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_call_argument_propagates_t0023() {
    let err = check_source("def f(v: int) -> int:\n    return v\nn = \"hi\"\nf((n := 1))\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_an_fstring_interpolation_propagates_t0023() {
    let err = check_source("n = \"hi\"\nf\"{(n := 1)}\"\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_list_literal_element_propagates_t0023() {
    let err = check_source("n = \"hi\"\n[(n := 1)]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_subscript_base_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)[0]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_slice_base_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)[0:1]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_slice_bound_propagates_t0023() {
    let err = check_source("n = \"hi\"\nlst = [1, 2, 3]\nlst[(n := 1):]\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_literal_key_propagates_t0023() {
    let err = check_source("n = \"hi\"\n{(n := 1): 2}\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_literal_value_propagates_t0023() {
    let err = check_source("n = \"hi\"\n{1: (n := 1)}\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_get_or_default_key_propagates_t0023() {
    let err = check_source("n = \"hi\"\ndct = {\"k\": 2}\ndct.get((n := 1), 0)\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_method_call_base_propagates_t0023() {
    let err = check_source(
        "class C:\n    def __init__(self, x: int):\n        self.x = x\n    def m(self) -> int:\n        return self.x\nn = \"hi\"\n(n := C(1)).m()\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_method_call_argument_propagates_t0023() {
    let err = check_source(
        "class C:\n    def __init__(self, x: int):\n        self.x = x\n    def m(self, y: int) -> int:\n        return y\nn = \"hi\"\nc = C(1)\nc.m((n := 1))\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_generic_class_instantiate_argument_propagates_t0023() {
    let err = check_source(
        "class Box[T]:\n    def __init__(self, x: T):\n        self.x = x\nn = \"hi\"\nBox[int]((n := 1))\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

// PEP 572 (#774): `collect_named_expr_names_in_expr` (function-local-name
// detection, distinct from `collect_named_expr_bindings` above) has two
// arms not reached by the two existing function-body fixtures
// (`a_walrus_inside_a_unary_operand_..`/`a_walrus_inside_a_slice_bound_..`
// above): the `is_local` *true* branch (a second walrus reusing an
// already-registered name skips the push) and the
// `ListLiteral`/`SetLiteral`/`TupleLiteral` arm. Hand-built HIR, mirroring
// this file's other function-body walrus fixtures.
#[test]
fn a_walrus_reusing_an_already_local_name_in_a_function_body_skips_the_duplicate_push() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Compare {
                        op: CmpOpKind::Lt,
                        left: Box::new(HirExpr::NamedExpr {
                            name: "x".to_string(),
                            value: Box::new(HirExpr::IntLiteral(1)),
                        }),
                        right: Box::new(HirExpr::NamedExpr {
                            name: "x".to_string(),
                            value: Box::new(HirExpr::IntLiteral(2)),
                        }),
                    },
                    body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_list_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::ListLiteral(vec![HirExpr::NamedExpr {
                        name: "a".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

// The `ListLiteral`/`SetLiteral`/`TupleLiteral` match arm above is one
// source line shared by all three variants, but each `|`-alternative still
// carries its own coverage region -- the test above only ever exercises
// the `ListLiteral` alternative. These two exercise the other two
// alternatives directly (not nested inside one another: a
// `set[..]`-typed tuple element is independently rejected by D-116 as
// "not compiled yet", so `TupleLiteral(vec![SetLiteral(..)])` is not a
// legal fixture here -- each container shape gets its own flat walrus).
#[test]
fn a_walrus_inside_a_set_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::SetLiteral(vec![HirExpr::NamedExpr {
                        name: "b".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_tuple_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::TupleLiteral(vec![HirExpr::NamedExpr {
                        name: "c".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("c".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

/// Part 2d of #1371: the same walk through an object-slot list display's
/// `|`-alternative. Only the empty-container pre-pass builds one from
/// source, never in a test position, so the fixture is hand-built.
#[test]
fn a_walrus_inside_an_object_list_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::ObjectList(vec![HirExpr::NamedExpr {
                        name: "d".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("d".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

/// Part 2d of #1371: `rewrite_generic_calls_in_expr` walks an object-slot
/// list display's elements and types the display as the object.
#[test]
fn rewrite_generic_calls_in_expr_walks_an_object_list_display() {
    let mut env = Environment::new();
    let mut expr = HirExpr::ObjectList(vec![HirExpr::IntLiteral(1)]);
    let mut instantiations = Vec::new();
    let mut seen = HashSet::new();
    let ty =
        rewrite_generic_calls_in_expr(&mut env, &[], &mut expr, &mut instantiations, &mut seen)
            .unwrap();
    assert_eq!(ty, Ty::Object);
    assert!(instantiations.is_empty());
}

// -- #911 (Part 1 of #885): class-level attributes at the checking seam ----
//
// `tests/issue_911_class_attrs.rs` drives these same programs through the
// compiled `pycc` binary; the tests below pin the same contract directly at
// this crate's own seams -- `class::resolve_attr_get`'s class-attribute
// fallback, `class::check_attr_set`'s write rejection, `expr`'s class-name
// read and its plain-name base restriction, and `check_class_pattern`'s
// keyword rejection.

/// A class with one class attribute of each supported scalar type, plus a
/// single instance slot. `body` is appended at module level.
fn class_attr_source(body: &str) -> String {
    format!(
        "class C:\n    X: int = 1\n    K: str = \"a\"\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\n{body}"
    )
}

#[test]
fn a_class_attribute_read_through_an_instance_type_checks() {
    // `resolve_attr_get`'s class-attribute fallback, reached only after the
    // property walk and the instance-slot walk both miss.
    check_source(&class_attr_source("c = C()\nprint(c.X)\nprint(c.K)\n"))
        .expect("an instance read of a class attribute must type-check");
}

#[test]
fn a_class_attribute_read_through_the_class_name_type_checks() {
    // `infer_expr_in`'s class-name attribute arm -- the only class-name
    // attribute read pycc supports.
    check_source(&class_attr_source("print(C.X)\n"))
        .expect("a class-name read of a class attribute must type-check");
}

#[test]
fn an_unknown_class_name_attribute_on_a_class_with_class_attributes_is_t0044() {
    let err = check_source(&class_attr_source("print(C.nope)\n"))
        .expect_err("an unknown class-name attribute must be rejected");
    assert_eq!(err.code, "T0044");
}

#[test]
fn a_class_attribute_read_off_a_non_name_base_is_rejected() {
    // The fold discards the base expression, so the base is restricted to a
    // bare name; a call-shaped base would otherwise be silently dropped.
    let err = check_source(
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\ndef make() -> C:\n    return C()\n\n\nprint(make().X)\n",
    )
    .expect_err("a call-shaped base must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message
            .contains("can only be read through a plain name"),
        "unexpected diagnostic: {}",
        err.message
    );
}

#[test]
fn writing_a_class_attribute_is_rejected_by_check_attr_set() {
    // Both writes that actually reach `check_attr_set`: through an instance
    // binding, and through `self` in a method other than `__init__` (a write
    // inside `__init__` is rejected earlier, by `pycc_hir`).
    for source in [
        class_attr_source("c = C()\nc.X = 5\n"),
        "class C:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def bump(self) -> None:\n        self.X = 5\n"
            .to_string(),
    ] {
        let err = check_source(&source).expect_err("a class-attribute write must be rejected");
        assert_eq!(err.code, "T0044");
        assert!(
            err.message
                .contains("it is a class-level attribute of class `C`"),
            "unexpected diagnostic: {}",
            err.message
        );
    }
}

#[test]
fn a_subclass_init_writing_an_inherited_class_attribute_name_is_rejected() {
    // The one shape `pycc_hir::class::body::reject_class_attr_collisions`
    // does *not* cover: the class attribute lives on an ancestor and the
    // subclass's `__init__` declares an instance slot of the same name, so
    // HIR sees no collision within either class's own tables. Only
    // `check_attr_set`'s full-MRO `lookup_class_attr_through_mro` walk stops
    // it -- without which `pycc_mir`'s instance-read fold would walk the same
    // MRO and fold `d.LIMIT` to the ancestor's `1` while the write targeted a
    // real slot. Narrowing that walk to the class's own `class_attrs` must
    // fail this test. The message names the receiver's own class (`Derived`),
    // since the lookup starts there and walks up to `Base`'s declaration.
    let err = check_source(
        "class Base:\n    LIMIT: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.LIMIT = 5\n",
    )
    .expect_err("a subclass write to an inherited class-attribute name must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message
            .contains("it is a class-level attribute of class `Derived`"),
        "unexpected diagnostic: {}",
        err.message
    );
}

#[test]
fn a_class_attribute_class_pattern_keyword_is_rejected() {
    // PEP 634: a class attribute has no per-instance value to match against.
    let err = check_source(&class_attr_source(
        "c = C()\nmatch c:\n    case C(X=v):\n        print(v)\n    case _:\n        print(0)\n",
    ))
    .expect_err("a class-attribute class-pattern keyword must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message.contains("not an instance attribute"),
        "unexpected diagnostic: {}",
        err.message
    );
}

// -- #915: `super().CLASS_CONST` reaches a base class's class attribute ----
//
// `tests/issue_915_super_class_attr.rs` drives these programs through the
// compiled `pycc` binary and asserts their runtime output; the tests below
// pin the same contract at this crate's own seam, `class::resolve_super_attr_get`.

#[test]
fn a_class_attribute_read_through_super_type_checks() {
    // The new class-attribute loop, reached after the property walk misses.
    check_source(
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
    )
    .expect("`super().X` must reach a base class's class attribute");
}

#[test]
fn a_class_attribute_on_the_second_base_of_a_diamond_is_reached_through_super() {
    // The loop must walk the *current class's* post-current MRO slice, not
    // the first base's own MRO -- `C` is reachable only through `D`'s
    // linearization `[D, B, C, A, object]`.
    check_source(
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    Y: int = 42\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().Y\n\n\nprint(D().read())\n",
    )
    .expect("`super().Y` must reach a class attribute on the second base of a diamond");
}

#[test]
fn an_instance_attribute_on_a_sibling_base_does_not_mask_a_class_attribute() {
    // The class-attribute loop runs before the `T0047` instance-attribute
    // rejection: a CPython `super` object never sees the instance
    // `__dict__`, so `C`'s `self.X` must not mask `B`'s class attribute.
    // CPython prints `1` for this program; pycc rejected it with `T0047`
    // before #915.
    //
    // #969 narrowed the original program used here: it also gave `B` its own
    // `self.n`, which makes `B`'s flat instance layout `[n]` while `D`'s is
    // `[X, n]` -- not a prefix -- so `pycc_hir`'s layout gate now rejects that
    // class shape with `C0001` before the checker ever runs (D-234). Dropping
    // `B`'s instance attribute leaves `B`'s layout empty, which is a prefix of
    // everything, and exercises exactly the same `super()` resolution path.
    // The rejected shape is pinned in `tests/issue_969_mi_slot_layout.rs`.
    check_source(
        "class B:\n    X: int = 1\n\n\nclass C:\n    def __init__(self) -> None:\n        self.X = 5\n\n\nclass D(B, C):\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
    )
    .expect("a sibling base's instance attribute must not mask a class attribute");
}

#[test]
fn a_class_attribute_declared_only_on_the_current_class_is_not_reachable_through_super() {
    // `mro[current_pos + 1..]`, not the full MRO: CPython raises
    // `AttributeError: 'super' object has no attribute 'X'` here.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    X: int = 7\n\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(Derived().read())\n",
    )
    .expect_err("a class attribute declared only on the current class is not reachable");
    assert_eq!(err.code, "T0044");
}

#[test]
fn super_inside_a_classmethod_is_rejected_as_a_capability_gap() {
    // #915: a `@classmethod` has no `self` receiver for `pycc_mir`'s
    // `Super` arm to bind, so the form is a `C0001` capability gap rather
    // than a compiler abort.
    let err = check_source(
        "class Base:\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @classmethod\n    def read(cls) -> int:\n        return super().X\n\n\nprint(Derived.read())\n",
    )
    .expect_err("`super()` inside a `@classmethod` must be rejected");
    assert_eq!(err.code, "C0001");
}

#[test]
fn super_method_call_inside_a_staticmethod_is_rejected_as_a_capability_gap() {
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.n = 0\n\n    def m(self) -> int:\n        return 1\n\n\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @staticmethod\n    def read() -> int:\n        return super().m()\n\n\nprint(Derived.read())\n",
    )
    .expect_err("`super().m()` inside a `@staticmethod` must be rejected");
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_class_attribute_earlier_in_the_slice_outranks_a_later_property() {
    // One pass over the slice, all member kinds per class: `B` contributes
    // the class attribute and the later `C` a `@property` of the same name.
    // CPython yields the class attribute (`int`), not the property.
    check_source(
        "class A:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass B(A):\n    X: int = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nclass C(A):\n    def __init__(self) -> None:\n        self.n = 0\n\n    @property\n    def X(self) -> int:\n        return 99\n\n\nclass D(B, C):\n    def __init__(self) -> None:\n        self.n = 0\n\n    def read(self) -> int:\n        return super().X\n\n\nprint(D().read())\n",
    )
    .expect("an earlier class attribute must outrank a later property");
}

// ---------------------------------------------------------------------
// PR 4a of #1083 (Part 4 of #1026): `float(o)` and `bool(o)`.
//
// `tests/issue_1083_foreign_conversions.rs` drives the same claims through
// the public CLI; these pin the two arms directly, including the shapes the
// CLI cannot reach because `pycc_hir` or an earlier pass refuses them first.
// ---------------------------------------------------------------------

#[test]
fn float_of_a_foreign_object_is_admitted_as_float() {
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "float".to_string(),
        args: vec![HirExpr::Name("o".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap(), Ty::Float);
}

#[test]
fn bool_of_a_foreign_object_is_admitted_as_bool() {
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::Name("o".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap(), Ty::Bool);
}

#[test]
fn bool_of_a_non_object_keeps_its_c0001_refusal() {
    // Fork 1 of the plan, stated openly: Part 4 relaxes exactly the object
    // case. `bool(1)` is still the capability-gap refusal it was, and
    // #1017/#1018 own the general builtin-conversion story.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "C0001");
}

#[test]
fn bool_of_a_foreign_object_with_a_second_argument_keeps_its_c0001_refusal() {
    // The arm admits exactly one argument, so a mis-arity call falls through
    // to the same C0001 every other unsupported `bool` shape gets rather
    // than inventing a T0021 of its own.
    let mut env = Environment::new();
    env.bind("o".to_string(), Ty::Object);
    let expr = HirExpr::Call {
        callee: "bool".to_string(),
        args: vec![
            HirExpr::Name("o".to_string()),
            HirExpr::Name("o".to_string()),
        ],
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "C0001");
}

#[test]
fn int_and_str_of_a_foreign_object_are_admitted_as_int_and_str() {
    // Part 4 of #1026 (PR 4b of #1083), `bool_of_a_foreign_object_is_
    // admitted_as_bool`'s claim for the other two conversions.
    for (callee, expected) in [("int", Ty::Int), ("str", Ty::Str)] {
        let mut env = Environment::new();
        env.bind("o".to_string(), Ty::Object);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::Name("o".to_string())],
        };
        assert_eq!(infer_expr(&env, &expr).unwrap(), expected, "{callee}");
    }
}

#[test]
fn int_and_str_of_a_non_object_keep_their_c0001_refusal() {
    // Fork 1 of the plan, stated openly: PR 4b relaxes exactly the object
    // case, so `int(1)` and `str(1)` are still the capability-gap refusals
    // they were.
    for callee in ["int", "str"] {
        let mut env = Environment::new();
        env.bind("x".to_string(), Ty::Int);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        };
        assert_eq!(
            infer_expr(&env, &expr).unwrap_err().code,
            "C0001",
            "{callee}"
        );
    }
}

#[test]
fn int_and_str_of_a_foreign_object_with_a_second_argument_keep_their_c0001_refusal() {
    // Each arm admits exactly one argument, so a mis-arity call falls through
    // to the same C0001 every other unsupported shape of that name gets
    // rather than inventing a T0021 of its own.
    for callee in ["int", "str"] {
        let mut env = Environment::new();
        env.bind("o".to_string(), Ty::Object);
        let expr = HirExpr::Call {
            callee: callee.to_string(),
            args: vec![
                HirExpr::Name("o".to_string()),
                HirExpr::Name("o".to_string()),
            ],
        };
        assert_eq!(
            infer_expr(&env, &expr).unwrap_err().code,
            "C0001",
            "{callee}"
        );
    }
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
