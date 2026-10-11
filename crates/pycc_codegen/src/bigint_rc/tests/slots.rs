//! Refcount guards on slot stores, walrus targets, optionals, range loops and attributes.

use super::*;

#[test]
fn an_int_slot_store_emits_a_guarded_release_of_the_word_it_overwrites() {
    // `v = v` is the minimal named-storage shape: `emit_assign` must
    // release whatever the slot already held *before* storing, and
    // `retain_if_int_duplicate` must retain the loaded word because the
    // slot is about to become a second owner of it.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "reassign".to_string(),
            params: vec![("v".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::Assign {
                    target: "v".to_string(),
                    value: int_name("v"),
                },
                MirStmt::Return(Some(int_name("v"))),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_int_slot_store", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // One retain (the assignment's duplicate; the return moves the
    // slot's reference out, #625), one release (the store's). Every one
    // of them was proved guarded, on its own operand, by
    // `guarded_bigint_refcount_calls`.
    assert_eq!((retains, releases), (1, 1), "got {calls:?}");
}

/// PEP 572 (#774), deep-review follow-up: `retain_if_int_duplicate`'s
/// `MirExpr::NamedExpr` arm. A walrus's yielded word is a *borrow* of
/// its target slot's own reference (see `int_value_is_a_duplicate_
/// reference`'s own `NamedExpr` arm, this module's release-side mirror
/// for exactly this classification), so passing `y := n` directly as a
/// call argument needs the same retain a plain `Name` argument gets --
/// without it, the call site and `y`'s slot would share one reference
/// between two live readers, and the classifier's own non-exhaustive
/// `_ => false` catch-all would have silently skipped it, leaving one
/// fewer retain than owners of that reference.
#[test]
fn a_walrus_wrapped_call_argument_emits_a_guarded_retain() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "twice".to_string(),
                params: vec![("v".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Add,
                    left: Box::new(int_name("v")),
                    right: Box::new(int_name("v")),
                    ty: Ty::Int,
                }))],
            },
            MirItem::Function {
                name: "caller".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::None,
                // A bare expression statement, exactly one of the three
                // placements `violates_walrus_placement`
                // (`pycc_hir/src/stmt.rs`) permits a walrus in --
                // `collect_stmt_bindings`'s `ExprStmt` arm is what
                // predeclares `y`'s storage slot here, by walking into
                // the call argument via `collect_expr_bindings`.
                body: vec![
                    MirStmt::ExprStmt(MirExpr::Call {
                        callee: "twice".to_string(),
                        args: vec![MirExpr::NamedExpr {
                            name: "y".to_string(),
                            value: Box::new(int_name("n")),
                            ty: Ty::Int,
                        }],
                        ty: Ty::Int,
                    }),
                    MirStmt::Return(None),
                ],
            },
        ],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_walrus_call_argument", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // Two retains: `MirExpr::NamedExpr`'s own codegen arm retains `n`'s
    // word before storing it into `y`'s slot (the assignment's own
    // duplicate, exactly like a plain `Assign`), and this fix's new
    // `retain_if_int_duplicate` arm retains it again when the same
    // walrus expression is then read as the call argument -- `y`'s
    // slot and the call both need their own live reference to the one
    // word. Two releases: `y`'s own release-before-store (a runtime
    // no-op on the never-written entry placeholder, proved guarded
    // exactly like the previous test's own release), and the call's
    // `Ty::Int` result being released as a discarded statement-position
    // temporary (`int_value_is_a_duplicate_reference` classifies
    // `Call` as owning, so a discarded call result is released).
    assert_eq!((retains, releases), (2, 2), "got {calls:?}");
}

/// #770 review (D-197 follow-up): `x: int | None = n` for a borrowed
/// `int` name `n` must retain `n`'s word the moment it is wrapped into
/// the `Optional[int]` struct, exactly like a plain `x = n` retains it
/// for a `Ty::Int` slot (`an_int_slot_store_emits_a_guarded_release_of_
/// the_word_it_overwrites` above). Before this fix,
/// `MirExpr::OptionalWrap`'s codegen arm called `coerce_scalar_to_type`
/// directly on the raw `emit_expr` result with no retain anywhere in
/// the pipeline -- confirmed by inspecting this exact fixture's emitted
/// IR prior to the fix, which contained zero `pycc_rt_bigint_retain`
/// calls at all.
#[test]
fn an_optional_wrap_of_a_borrowed_int_name_retains_the_duplicate_reference() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "wrap".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![
                MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::OptionalWrap(Box::new(int_name("n")), Box::new(Ty::Int)),
                },
                MirStmt::Return(None),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_optional_wrap_retain", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // One retain: `x`'s own store of the `OptionalWrap`'d word. One
    // release too -- `emit_assign` structurally emits
    // `release_optional_int_slot_before_store` before *every* store
    // into an `Optional[int]` slot, including the first, exactly like
    // `Ty::Int`'s own unconditional release-before-store. It is proved
    // guarded here, and at runtime is a no-op for this first store: the
    // slot's entry placeholder (`storage_slot_at_entry`'s own
    // `Ty::Optional` arm) is a valid encoded smallint `0`, which the
    // guard's `(word & 0b11) == 0 && word != 0` test rejects.
    assert_eq!((retains, releases), (1, 1), "got {calls:?}");
}

/// #770 review: the release-side half of the fix above. Once an
/// `Optional[int]` slot holds a retained heap-bigint reference (the
/// previous test), overwriting that slot with a second assignment must
/// release the first one -- exactly mirroring `Ty::Int`'s own
/// release-before-store invariant -- or the retain the previous test
/// proves becomes a permanent per-reassignment leak instead of a
/// balanced reference.
#[test]
fn reassigning_an_optional_int_slot_releases_its_previous_payload() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "rewrap".to_string(),
            params: vec![("n".to_string(), Ty::Int), ("m".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![
                MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::OptionalWrap(Box::new(int_name("n")), Box::new(Ty::Int)),
                },
                MirStmt::Assign {
                    target: "x".to_string(),
                    value: MirExpr::OptionalWrap(Box::new(int_name("m")), Box::new(Ty::Int)),
                },
                MirStmt::Return(None),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_optional_wrap_release", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // Two retains: one per `OptionalWrap`. Two releases, each a
    // guarded, proven-safe `release_optional_int_slot_before_store`
    // call -- the first store's is a runtime no-op on the slot's entry
    // placeholder (see the previous test), but the *second* store's is
    // the one that matters: it retires `x`'s first (retained) payload
    // before overwriting it with the second, which is exactly the
    // release this fix adds to keep the preceding test's retain
    // balanced instead of leaking on every reassignment.
    assert_eq!((retains, releases), (2, 2), "got {calls:?}");
}

/// Issue #769 (Part 2 of #747): the read-side mirror of
/// `an_optional_wrap_of_a_borrowed_int_name_retains_the_duplicate_reference`
/// above. `y = n` for an `Optional[int]`-scoped, currently-narrowed
/// `n` lowers `n`'s read to `MirExpr::OptionalUnwrap`, classified
/// `true` (a duplicate reference, exactly like a plain `Ty::Int` name
/// read) by `int_value_is_a_duplicate_reference`'s own `OptionalUnwrap`
/// arm -- `y`'s store of the extracted payload word must retain it, or
/// `n`'s later reassignment/death would free the bigint out from under
/// `y` despite `y` still being live (the exact D-181/#770-class defect
/// this classification exists to prevent, one layer down from
/// `OptionalWrap`'s own).
#[test]
fn an_optional_unwrap_of_a_narrowed_name_retains_the_duplicate_reference() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "unwrap_dup".to_string(),
            params: vec![("n".to_string(), Ty::Optional(Box::new(Ty::Int)))],
            return_ty: Ty::None,
            body: vec![
                MirStmt::Assign {
                    target: "y".to_string(),
                    value: MirExpr::OptionalUnwrap(
                        Box::new(MirExpr::Name {
                            name: "n".to_string(),
                            ty: Ty::Optional(Box::new(Ty::Int)),
                        }),
                        Box::new(Ty::Int),
                    ),
                },
                MirStmt::Return(None),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_optional_unwrap_retain", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // One retain: `y`'s own store of the unwrapped duplicate word. One
    // release too, for the identical structural reason
    // `an_optional_wrap_of_a_borrowed_int_name_retains_the_duplicate_reference`
    // documents -- `emit_assign` unconditionally emits a guarded
    // release-before-store for every `Ty::Int` slot, including `y`'s
    // first store, proven guarded (and a runtime no-op here, since
    // `y`'s slot has no prior value) by the same
    // `guarded_bigint_refcount_calls` machinery every other assertion
    // in this module relies on.
    assert_eq!((retains, releases), (1, 1), "got {calls:?}");
}

#[test]
fn a_range_loop_over_one_aliased_bound_emits_guarded_retains_and_releases() {
    // `for i in range(n, n, n)`: one heap object reachable as start,
    // stop, and step at once -- the shape where an over-eager release
    // would free a word two other operands still hold. Compiling it at
    // all also exercises the phi-predecessor invariant, since every
    // guard here splits a block the loop's induction phi points at.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "loop_over".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![MirStmt::ForRange {
                var: "i".to_string(),
                start: int_name("n"),
                stop: int_name("n"),
                step: int_name("n"),
                body: Vec::new(),
            }],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_range_aliased_bounds", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // The ownership contract of `MirStmt::ForRange`, stated as a count
    // so that dropping any single one of them fails here rather than
    // only as a slow RSS drift. Four retains: start, stop and step in
    // the preheader (three independent references even though all
    // three alias one object here), plus the induction word once per
    // iteration. Five releases: the loop variable's own slot before
    // each rebind, the induction word once `next` is computed, and
    // three in the exit block (the surviving induction word, stop, and
    // step). Start has no release of its own on purpose -- its
    // preheader retain *is* the induction word's first reference, and
    // releasing it separately would free a word `stop` and `step` still
    // hold in exactly the aliased shape this fixture builds.
    assert_eq!((retains, releases), (4, 5), "got {calls:?}");
}

#[test]
fn an_int_attribute_slot_store_of_a_bool_emits_a_guarded_release() {
    // #627/D-187: the `MirStmt::AttrSet` release is gated on the
    // *value's* `Ty`, not the slot's. `pycc_mir` now wraps a `bool`
    // stored into an `int`-declared attribute in `MirExpr::IntBoundary`
    // (whose `ty()` is `Ty::Int`), which is what makes this release
    // fire at all -- before #627 the value reported `Ty::Bool` and the
    // bigint the slot still held was dropped on the floor (D-180
    // Consequences item 6).
    //
    // What this pins and what it does not: because `IntBoundary::ty()`
    // is already `Ty::Int`, a hand-built `MirStmt::AttrSet` like this
    // one emits the release on the pre-#627 tree too. This is a
    // *codegen* pin -- that a `Ty::Int`-reporting value reaching an
    // attribute store emits the guarded release. The lowering half,
    // that a `bool` into an `int` slot now produces such a value, is
    // pinned by `pycc_mir`'s
    // `an_attr_set_of_a_bool_into_an_int_declared_slot_widens_through_int_boundary`.
    // Only the two together pin the leak fix; the release itself is
    // invisible in program output, so no conformance fixture can.
    //
    // This module's other tests all build `class_defs: Vec::new()` and
    // never construct an instance, so the class/`Instantiate`/`AttrSet`
    // scaffolding below is duplicated from `crate::tests`'s own
    // `class_instantiation_attribute_and_method_call_codegens_and_runs`.
    // The test lives here rather than there because `refcount_calls_in`
    // and `guarded_bigint_refcount_calls` -- which prove the release is
    // *guarded* on its own operand -- are private to this module, and
    // duplicating that guard-parsing would be the larger duplication.
    // Both files compile in the same `--cfg test` instantiation
    // (`lib.rs`'s `#[cfg(test)] mod tests;`), so the choice is
    // coverage-neutral.
    let self_ty = Ty::Instance(Box::new("Holder".to_string()));
    let init = MirItem::Function {
        name: "Holder.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: int_name("n"),
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "h".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Holder.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string()],
                    args: vec![MirExpr::IntLiteral(0)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "h".to_string(),
                    ty: self_ty,
                },
                slot: 0,
                value: MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
            }),
        ],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_int_attr_slot_bool_store", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // Two releases, one per `AttrSet`: `__init__`'s (into a
    // still-unassigned slot, which the unchecked getter reads as `0`, so
    // its guard is false at runtime) and the
    // top-level store of the encoded `True` word over whatever slot 0
    // then held. The single retain is `__init__`'s own `n` parameter --
    // a duplicate reference the slot becomes a second owner of. The
    // `IntBoundary` value contributes no retain: an `IntBoundary` is a
    // freshly produced word, and
    // `int_value_is_a_duplicate_reference` groups it into its `false`
    // arm. Every call listed was proved guarded, on its own operand, by
    // `guarded_bigint_refcount_calls`.
    assert_eq!((retains, releases), (1, 2), "got {calls:?}");
}

// -----------------------------------------------------------------
// #809 (Part 3 of #747): `T0049`'s widened gate now also admits
// `Optional[float]`/`Optional[bool]`, so `int_value_is_a_duplicate_
// reference`'s and `retain_if_int_duplicate`'s `OptionalUnwrap` arms
// (both above) can no longer assume every `OptionalUnwrap` node is
// `Ty::Int`-typed. Tracing both call sites (`int_temporary_word`'s own
// `source_expr.ty() == pycc_mir::Ty::Int` short-circuit guard, and
// `retain_if_int_duplicate`'s enclosing `if let Scalar::Int(word) =
// scalar` guard) shows each already restricts to `Ty::Int` at the
// *caller*, before `OptionalUnwrap`'s own arm is ever consulted -- so
// no code change was needed there, only the stale comment fix above.
// These two tests prove that empirically rather than by inspection
// alone: a `float | None` and a `bool | None` local, each narrowed and
// unwrapped in a shape that would trigger retain-classification if the
// guard were missing, produce zero bigint refcount calls.
// -----------------------------------------------------------------

/// `x: float | None = 5.5; if x is not None: y = x` -- narrows and
/// unwraps a present `Optional[float]`, landing `OptionalUnwrap`'s
/// result in a second (`y`) slot exactly the shape that would call
/// `retain_if_int_duplicate` on a `Ty::Int`-classified `OptionalUnwrap`.
/// A `float` payload is never `Scalar::Int`, so this must emit zero
/// bigint refcount calls.
#[test]
fn a_narrowed_optional_float_unwrap_emits_no_bigint_refcount_calls() {
    let optional_float = Ty::Optional(Box::new(Ty::Float));
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_float.clone(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::Assign {
                    target: "y".to_string(),
                    value: MirExpr::OptionalUnwrap(
                        Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_float,
                        }),
                        Box::new(Ty::Float),
                    ),
                }],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_optional_float_unwrap", &mir);
    assert_eq!(
        calls.len(),
        0,
        "a float payload must never reach the bigint refcount path: {calls:?}"
    );
}

/// The `bool` counterpart of the test directly above: `x: bool | None =
/// True; if x is not None: y = x`. A `bool` payload is never `Scalar::
/// Int` either, so this must also emit zero bigint refcount calls.
#[test]
fn a_narrowed_optional_bool_unwrap_emits_no_bigint_refcount_calls() {
    let optional_bool = Ty::Optional(Box::new(Ty::Bool));
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool.clone(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![MirStmt::Assign {
                    target: "y".to_string(),
                    value: MirExpr::OptionalUnwrap(
                        Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_bool,
                        }),
                        Box::new(Ty::Bool),
                    ),
                }],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_optional_bool_unwrap", &mir);
    assert_eq!(
        calls.len(),
        0,
        "a bool payload must never reach the bigint refcount path: {calls:?}"
    );
}

// -----------------------------------------------------------------
// #1116: `MirExpr::BufferLen`'s own `int_value_is_a_duplicate_reference`
// arm, exercised at the one site that actually consults it.
//
// That arm classifies the `len(b)` word as *owned*, which is what makes
// `int_temporary_word` hand `MirStmt::ForRange`'s emitter a word to
// release in `after_bb`. The classification fails in opposite
// directions -- a wrongly-"borrowed" answer leaks, a wrongly-"owning"
// one frees a live word -- so the arm is worth a deterministic,
// always-run exerciser. The three `tests.rs` buffer-length tests all use
// `return len(b)` as the whole body, a shape that never reaches this
// classification at all; the only other place it runs end to end is
// `tests/issue_1116_ext_buffer_len.rs`'s `#[ignore]`d hosted oracle,
// which needs CPython development headers and is therefore absent from
// an ordinary signal.
// -----------------------------------------------------------------

/// `for i in range(len(b)): pass` inside a D-244 `--ext` export, which
/// is the only shape that can bind a `memoryview` at all.
///
/// Stated as retain/release counts, the same way
/// `a_range_loop_over_one_aliased_bound_emits_guarded_retains_and_
/// releases` pins `ForRange`'s ownership contract: the emitter's
/// treatment of a `BufferLen` `stop` is invisible in the program's
/// output and in its RSS (the count is always an inline D-141 smallint,
/// so the release it emits is a runtime no-op), so a count is the only
/// place the classification is observable.
#[test]
fn a_range_loop_over_a_buffer_length_releases_the_count_it_owns() {
    // `start`/`step` are the lowering's own `0`/`1` literals. They are
    // not compile-time constants by the time the guard sees them --
    // every `range` operand is routed through
    // `pycc_rt_range_normalize_operand` first -- so they carry their own
    // guard sites, and the counts below account for all three operands.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "sweep".to_string(),
            params: vec![("b".to_string(), Ty::MemoryView)],
            return_ty: Ty::None,
            body: vec![MirStmt::ForRange {
                var: "i".to_string(),
                start: MirExpr::IntLiteral(0),
                stop: MirExpr::BufferLen {
                    base: Box::new(MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::MemoryView,
                    }),
                },
                step: MirExpr::IntLiteral(1),
                body: Vec::new(),
            }],
        }],
        ..Default::default()
    };
    let calls = refcount_calls_with_options(
        "bigint_rc_range_over_buffer_len",
        &mir,
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
    );
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // `MirStmt::ForRange`'s ownership contract, as counts. Four
    // retains: the loop's own unconditional retain of `start`, `stop`
    // and `step` in the preheader, plus the induction word once per
    // bind of the visible target. Eight releases: the three birth
    // references `release_if_int_temporary` retires right after those
    // retains -- and this is the one the `BufferLen` arm decides, since
    // `stop` is the `len(b)` word -- the target slot's previous word
    // inside `emit_assign`, the induction word once `next` is computed,
    // and three in `for_after` (the surviving induction word, `stop`
    // and `step`). Plus one: the smallint copy of the loop
    // (`for_range_fast`) binds the target through `emit_assign` too,
    // releasing the slot's previous word; it retains nothing, since its
    // induction word is always a smallint.
    //
    // Classifying `BufferLen` as a borrowed duplicate instead makes
    // `int_temporary_word` answer `None` for `stop`, which drops
    // exactly one release from this total and leaks the count's birth
    // reference; the mirror error, classifying an operand the loop does
    // not own as owning, frees a live word. Neither is visible in the
    // program's output or in its RSS here -- a buffer length is always
    // an inline D-141 smallint, so the release is a runtime no-op -- so
    // this count is the only place the classification is observable.
    assert_eq!((retains, releases), (4, 9), "got {calls:?}");
}
