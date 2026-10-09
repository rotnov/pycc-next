//! Heap-bigint reference counting: the guarded retain/release emitter, the
//! two compile-time ownership classifications it is driven by, and the two
//! helpers that apply them ([D-180], [D-181]).
//!
//! Narrow carve out of `lib.rs` under AGENTS.md's decomposability rule: this
//! is exactly the unit #625 touches, and nothing else is relocated. #633
//! extended the carve-out to the module's own IR-observer test cluster,
//! which had stayed behind in the crate root's test module (`tests.rs`
//! since #545); it is self-contained
//! and exercises only this module's emitter, so it belongs beside it.
//!
//! [D-180]: ../../../docs/decisions/D-180-refcount-heap-bigints-and-release-them-at-named.md
//! [D-181]: ../../../docs/decisions/D-181-release-a-heap-bigint-s-birth-reference-at-every.md

use super::*;

/// Which of `pycc_rt`'s two bigint refcount entry points
/// `emit_bigint_refcount_call` should emit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BigIntRefcount {
    Retain,
    Release,
}

/// Emits a guarded `pycc_rt_bigint_retain`/`_release` call on the D-141
/// encoded int word `word`.
///
/// The guard is emitted *inline*, as LLVM IR, rather than left to the
/// runtime's own no-op arms: the predicate is `(word & 0b11) == 0 && word
/// != 0`, exactly `classify_encoded_int`'s `BigInt` case, so an ordinary
/// smallint loop performs one `and`, two `icmp`s and a not-taken branch per
/// site instead of a call into `pycc_rt`. D-084/D-140's nbody throughput
/// floor is measured on code that is entirely smallint; routing it through
/// an unconditional call per assignment would regress that gate, which is
/// the reason this guard exists at all and must not be simplified away.
///
/// Word `0` is excluded by the guard and *also* handled by the runtime:
/// `storage_slot_at_entry` zero-initializes `int` slots, and `0` is
/// `classify_encoded_int`'s fail-closed pattern rather than a valid encoded
/// int, so a release of a never-stored slot must reach neither the
/// classifier nor a dereference.
///
/// Emits nothing at all when `word` is a compile-time constant -- see the
/// body for why that word is provably not a heap pointer.
///
/// Postcondition, on the non-constant path *only*: leaves the builder
/// positioned in a fresh continuation block. Callers that record a basic
/// block for a phi incoming edge must therefore re-read `get_insert_block()`
/// *after* calling this, never before. On the constant path the builder is
/// left exactly where it was, so such a caller would observe the *same*
/// block rather than a stale one -- re-reading afterwards is correct in both
/// cases, which is why the rule is stated unconditionally even though the
/// block split is not. Do not invert it into "only re-read when the word is
/// non-constant": whether a word is constant is not a property a caller can
/// see, and guessing wrong reintroduces the phi miscompile this note exists
/// to prevent.
pub(super) fn emit_bigint_refcount_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    op: BigIntRefcount,
) {
    // A compile-time-constant word is provably never a heap pointer: every
    // constant an `int` expression can produce is a tagged smallint, a bool
    // identity marker, or the empty-slot word, and a `BigIntObj` address is
    // only ever materialized by a runtime call. Emitting the guard anyway
    // would let inkwell fold all four of its halves into a constant `i1`,
    // leaving a conditional branch whose condition is no longer an SSA
    // definition -- dead IR that the D-180 guard-shape observer tests then
    // cannot describe. Skip it instead of emitting a guard that is known
    // false before it is built.
    if word.is_const() {
        return;
    }
    let i64_type = context.i64_type();
    let low_tag = builder
        .build_and(word, i64_type.const_int(0b11, false), "bigint_low_tag")
        .expect("build_and should not fail for two i64 values");
    let aligned = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            low_tag,
            i64_type.const_zero(),
            "bigint_aligned",
        )
        .expect("build_int_compare should not fail for two i64 values");
    let non_zero = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            word,
            i64_type.const_zero(),
            "bigint_non_zero",
        )
        .expect("build_int_compare should not fail for two i64 values");
    let is_heap = builder
        .build_and(aligned, non_zero, "bigint_is_heap")
        .expect("build_and should not fail for two i1 values");
    let function = builder
        .get_insert_block()
        .expect("builder is always positioned inside some block while emitting")
        .get_parent()
        .expect("the block the builder is positioned in always belongs to a function");
    let call_block = context.append_basic_block(function, "bigint_rc_call");
    let continue_block = context.append_basic_block(function, "bigint_rc_cont");
    builder
        .build_conditional_branch(is_heap, call_block, continue_block)
        .expect("build_conditional_branch should not fail for a well-formed i1");
    builder.position_at_end(call_block);
    let callee = match op {
        BigIntRefcount::Retain => rt.bigint_retain,
        BigIntRefcount::Release => rt.bigint_release,
    };
    builder
        .build_call(callee, &[word.into()], "bigint_rc")
        .expect("build_call should not fail for a well-formed refcount call");
    builder
        .build_unconditional_branch(continue_block)
        .expect("build_unconditional_branch should not fail for a fresh block");
    builder.position_at_end(continue_block);
}

/// Loads `slot`'s current encoded word and releases it. `emit_assign` calls
/// this for every `Ty::Int` slot before storing, which is what makes the
/// slot's single owned reference an invariant: the slot holds `0` (released
/// as a no-op) until its first store, and exactly one owned word after it.
pub(super) fn release_int_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
) {
    let old = builder
        .build_load(context.i64_type(), slot.ptr, "old_int")
        .expect("build_load should not fail for this function's own alloca")
        .into_int_value();
    emit_bigint_refcount_call(context, builder, rt, old, BigIntRefcount::Release);
}

/// `release_int_slot_before_store`'s counterpart for an `Optional[int]`
/// slot (D-197 follow-up, #763/#770 review): loads the slot's current
/// `{ i64, i8 }` struct and releases field 0 (the payload word)
/// unconditionally, ignoring the present/absent flag in field 1.
///
/// Ignoring the flag is deliberately safe rather than merely convenient:
/// both producers of an `Optional[int]` struct value guarantee field 0 is
/// always a syntactically valid D-141-encoded `int` regardless of
/// presence -- `coerce_scalar_to_type`'s absent-payload placeholder arm and
/// `default_value_for_type`'s own `Ty::Optional(_)` arm both explicitly
/// encode a smallint `0` there rather than a raw zero word, for exactly
/// this reason (see either's doc comment). `emit_bigint_refcount_call`'s
/// own inline guard then does the rest: a smallint or bool-identity marker
/// takes the not-taken branch, and only an actual owned heap-bigint
/// reference ever reaches the runtime call.
///
/// `storage_slot_at_entry` gives every `Optional[int]` slot this same valid
/// placeholder at function entry (mirroring `Ty::Int`'s own zero-init
/// immediately above), which is what makes calling this at the very first
/// store into a slot safe rather than a read of uninitialized alloca
/// memory.
///
/// This is `emit_assign`'s release-side half of the retain
/// `MirExpr::OptionalWrap`'s codegen arm now performs on a borrowed `int`
/// payload (see `emit_expr`'s `OptionalWrap` arm in `lib.rs`) -- without
/// it, that retain would be a permanent per-reassignment leak instead of a
/// balanced reference.
pub(super) fn release_optional_int_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
) {
    let struct_ty = context.struct_type(
        &[context.i64_type().into(), context.i8_type().into()],
        false,
    );
    let old = builder
        .build_load(struct_ty, slot.ptr, "old_optional")
        .expect("build_load should not fail for this function's own alloca")
        .into_struct_value();
    let payload = builder
        .build_extract_value(old, 0, "old_optional_payload")
        .expect("build_extract_value should not fail reading field 0 of a well-formed struct")
        .into_int_value();
    emit_bigint_refcount_call(context, builder, rt, payload, BigIntRefcount::Release);
}

/// `int`'s counterpart to [`incref_if_str_duplicate`], deliberately kept as
/// a separate function applied at a strictly smaller set of call sites.
///
/// `incref_if_str_duplicate` runs at every site that *evaluates* a `str`,
/// including the two that merely consume the value without taking ownership
/// (`print`'s argument and `to_str`'s operand). That is harmless for `str`
/// because those two paths decref again themselves, but an `int` retain
/// there would be orphaned -- a leak with no matching release. So the
/// int-side retain is applied only where the value's new home actually
/// takes ownership: an assignment target, a `return` value, an instance
/// attribute, a call argument, and -- since D-182 -- a `Ty::Tuple`
/// literal's element field.
///
/// The predicate shares its first two shapes with
/// `str_value_is_a_duplicate_reference` (whose doc comment is the canonical
/// list of borrowed `str` reads, including `str`-only ones such as
/// `ExceptionMessage`): a bare `Name` or `AttrGet` read yields a *second*
/// reference to a word something else already owns, whereas every other
/// int-producing expression (`IntLiteral`, arithmetic, a call result)
/// freshly constructs its value already owning exactly one reference.
///
/// A `Ty::Int` element read out of a *tuple* is the third borrowed shape,
/// and is matched here in addition to those two. Under D-182 a
/// `MirExpr::TupleLiteral` element goes through this very helper at
/// ingress, so a tuple field holds a reference of its own; but
/// `MirExpr::Subscript`'s tuple branch hands the stored word straight back
/// out **without transferring** that reference, which stays with the
/// field. The reader therefore owns nothing and must take its own
/// reference -- which is what this arm does.
///
/// D-181 shipped this arm under the opposite premise (the field was a pure
/// alias, retaining nothing) and recorded the converse direction --
/// overwriting the supplying name before the tuple is read -- as a known,
/// unfixed use-after-free tracked by
/// [#633](https://github.com/rotnov/pycc/issues/633). D-182 closes that
/// direction by adding the ingress retain, and in doing so replaces the
/// arm's old **unenforced precondition** (sound only while the supplying
/// name is still bound) with a real reference. The classification itself
/// is unchanged; only its justification is.
///
/// Deliberately **not** shared with `int_value_is_a_duplicate_reference`
/// even though the two predicates agree on today's three borrowed shapes.
/// The two sides fail in opposite directions and must be free to diverge:
/// a missing retain here leaks, while a missing "borrowed" classification
/// on the release side frees a live word. Sharing one predicate would make
/// every future ownership refinement a simultaneous edit to both an
/// over-approximating and an under-approximating consumer.
///
/// `retain_if_int_duplicate` has 7 call sites in the current tree
/// (`lib.rs`: `MirExpr::OptionalWrap`, `MirExpr::TupleLiteral`'s element
/// loop, `MirExpr::NamedExpr`, `build_call_to_with_leading_args`'s argument
/// loop, `MirStmt::Assign`, `MirStmt::Return`, `MirStmt::AttrSet`). Every one
/// of them is assign-shaped -- the retained reference's new home is a slot,
/// a parameter, a field, or a return value -- except two, which additionally
/// bracket the call with a `mark`/`truncate` pair on
/// `rt.exceptions.pending_int_releases` (#638, D-208) because a *later*
/// sibling's raising evaluation can abandon the retain before the transfer
/// it is staged for ever completes:
///
/// - `MirExpr::TupleLiteral`'s element loop (retain, then a field transfer
///   via `build_insert_value`)
/// - `build_call_to_with_leading_args`'s argument loop (retain, then a
///   transfer into the callee's parameter slot)
///
/// Those two call the exception-edge-aware
/// [`retain_if_int_duplicate_and_track_for_exception_edge`] instead of this
/// function directly, closing
/// [#834](https://github.com/rotnov/pycc/issues/834): a duplicate/borrowed
/// operand's extra retain used to go untracked on that edge, the one gap
/// D-208's own release mechanism left open for a *retained* (as opposed to
/// owning) reference. The other five call sites have no such bracket and no
/// exception edge to protect (`MirStmt::Assign`/`Return`/`AttrSet` release
/// or hand off their slot synchronously; `MirExpr::NamedExpr`/`OptionalWrap`
/// are themselves assign-shaped reads with no staged transfer at all), so
/// they keep calling this function unchanged.
pub(super) fn retain_if_int_duplicate<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: Scalar<'ctx>,
) -> Scalar<'ctx> {
    retain_if_int_duplicate_reporting(context, builder, rt, source_expr, scalar).0
}

/// The classification behind [`retain_if_int_duplicate`], extracted so a
/// caller can learn whether the retain actually happened -- needed by
/// [`retain_if_int_duplicate_and_track_for_exception_edge`], which must push
/// the retained word onto `rt.exceptions.pending_int_releases` only when a
/// retain occurred. Returns the retained word itself (`Some`) rather than a
/// separate `bool`, so a caller that only cares about the word never needs
/// to re-destructure `scalar` to recover it -- the "was it an `int`, and did
/// it retain" question and "here is the word" answer are the same value.
///
/// Whether `source_expr` is a *duplicate* (borrowed) `Ty::Int`-producing
/// shape: a bare `Name`/`AttrGet` read yields a second reference to a word
/// something else already owns, whereas every other int-producing
/// expression (`IntLiteral`, arithmetic, a call result) freshly constructs
/// its value already owning exactly one reference. A `Ty::Int` element read
/// out of a *tuple* is a third borrowed shape (D-182): a
/// `MirExpr::TupleLiteral` element goes through this function at ingress, so
/// a tuple field holds a reference of its own, but `MirExpr::Subscript`'s
/// tuple branch hands the stored word straight back out **without
/// transferring** that reference, which stays with the field -- the reader
/// therefore owns nothing and must take its own reference here.
///
/// D-181 shipped this arm under the opposite premise (the field was a pure
/// alias, retaining nothing) and recorded the converse direction --
/// overwriting the supplying name before the tuple is read -- as a known,
/// unfixed use-after-free tracked by
/// [#633](https://github.com/rotnov/pycc/issues/633). D-182 closes that
/// direction by adding the ingress retain, and in doing so replaces the
/// arm's old **unenforced precondition** (sound only while the supplying
/// name is still bound) with a real reference. The classification itself
/// is unchanged; only its justification is.
///
/// Deliberately **not** shared with `int_value_is_a_duplicate_reference`
/// even though the two predicates agree on today's three borrowed shapes.
/// The two sides fail in opposite directions and must be free to diverge:
/// a missing retain here leaks, while a missing "borrowed" classification
/// on the release side frees a live word. Sharing one predicate would make
/// every future ownership refinement a simultaneous edit to both an
/// over-approximating and an under-approximating consumer.
fn retain_if_int_duplicate_reporting<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: Scalar<'ctx>,
) -> (Scalar<'ctx>, Option<IntValue<'ctx>>) {
    let retained_word = if let Scalar::Int(word) = scalar {
        let is_duplicate = match source_expr {
            MirExpr::Name {
                ty: pycc_mir::Ty::Int,
                ..
            }
            | MirExpr::AttrGet {
                ty: pycc_mir::Ty::Int,
                ..
            }
            // PEP 572 (#774): mirrors `int_value_is_a_duplicate_reference`'s
            // own `NamedExpr` arm below -- a walrus yields a borrow of its
            // target slot's own reference, not a fresh one, so a call
            // argument that is itself a bare walrus needs the same retain
            // a plain `Name` read gets here.
            | MirExpr::NamedExpr {
                ty: pycc_mir::Ty::Int,
                ..
            } => true,
            // No `Ty::Int` test alongside the tuple test: the enclosing
            // `if let Scalar::Int` has already established that this
            // element is an `int` word, so a `Ty::Float`/`Ty::Bool` tuple
            // field never reaches here at all.
            MirExpr::Subscript { base, .. } => matches!(base.ty(), pycc_mir::Ty::Tuple(_)),
            // Issue #769 (Part 2 of #747): same borrowed-read shape as
            // `Name { ty: Ty::Int, .. }` immediately above -- see
            // `int_value_is_a_duplicate_reference`'s own `OptionalUnwrap`
            // arm (`bigint_rc.rs`) for the full reasoning this mirrors. No
            // `Ty::Int` guard needed here either, for the identical reason
            // the tuple arm omits one: this function's enclosing
            // `if let Scalar::Int` has already established the payload word
            // this arm is deciding whether to retain.
            MirExpr::OptionalUnwrap(_, _) => true,
            // #1346: a sequence yields its `value`'s word, so it is a
            // duplicate exactly when `value` is.
            MirExpr::Sequence { value, .. } => {
                return retain_if_int_duplicate_reporting(context, builder, rt, value, scalar);
            }
            _ => false,
        };
        if is_duplicate {
            emit_bigint_refcount_call(context, builder, rt, word, BigIntRefcount::Retain);
        }
        is_duplicate.then_some(word)
    } else {
        None
    };
    (scalar, retained_word)
}

/// The exception-edge-aware counterpart of [`retain_if_int_duplicate`], used
/// only at the two call sites that bracket the retain with a
/// `mark`/`truncate` pair on `rt.exceptions.pending_int_releases` (#638,
/// D-208): `MirExpr::TupleLiteral`'s element loop and
/// `build_call_to_with_leading_args`'s argument loop. See
/// [`retain_if_int_duplicate`]'s own doc comment for why exactly these two,
/// and no other, call site needs this form.
///
/// Closes [#834](https://github.com/rotnov/pycc/issues/834): before this
/// function existed, a duplicate/borrowed operand's extra retain at these
/// two sites was invisible to the exception-edge release mechanism -- a
/// later sibling's raising evaluation would abandon the retained reference
/// with nothing to release it, a pure leak (never a use-after-free, since
/// the original owner's own reference is untouched). When
/// [`retain_if_int_duplicate_reporting`] reports a retain happened, this
/// pushes the word onto `rt.exceptions.pending_int_releases` via
/// [`push_word_onto_pending_int_releases`] so the caller's existing
/// `mark`/`truncate`/exception-unwind machinery covers it exactly like an
/// owning temporary's own push already does at that same site: on the
/// normal path the caller truncates back to `mark` without releasing
/// (ownership has transferred to the aggregate field / parameter slot,
/// mirroring the owning-temporary case at that exact site), and on the
/// exception path the installed unwind target releases every pending entry,
/// including this one.
///
/// [`retain_if_int_duplicate_reporting`]'s `Some(word)` return already *is*
/// the "a retain happened" fact together with the word to push, so this
/// function needs no separate `Ty::Int` re-check of `scalar` to recover it.
pub(super) fn retain_if_int_duplicate_and_track_for_exception_edge<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let (scalar, retained_word) =
        retain_if_int_duplicate_reporting(context, builder, rt, source_expr, scalar);
    if let Some(word) = retained_word {
        push_word_onto_pending_int_releases(rt, word);
    }
    scalar
}

/// Whether evaluating `expr` yields a word this expression **borrows** from
/// some other owner, rather than a freshly owned one -- the release-side
/// mirror of `retain_if_int_duplicate`'s own inline classification (D-181).
///
/// Deliberately a separate predicate from that one, and deliberately an
/// exhaustive `match` rather than a `matches!`: the two sides fail in
/// opposite directions (a missing retain leaks; a wrongly-"owning" release
/// frees a live word), and exhaustiveness makes a future `MirExpr` variant
/// a compile error here instead of a silent double-free.
///
/// Four borrowed shapes today:
///
/// - `Name { ty: Int }`, `AttrGet { ty: Int }`, and `NamedExpr { ty: Int }`
///   read a word a storage slot still owns (see the `NamedExpr` arm below
///   for why a walrus is a borrow rather than a fresh reference).
/// - A `Subscript` on a *tuple* base (already known `Ty::Int`, see below).
///   Under D-182 `MirExpr::TupleLiteral` retains a borrowed element at
///   ingress, so the field holds a reference; but the tuple branch of
///   `MirExpr::Subscript` returns that same word unchanged without
///   transferring that reference, so the read is still a borrow. Keeping
///   this arm `borrowed` is load-bearing: flipping it to owning would make
///   `t[0] + 1` retire the tuple's own ingress reference on a mere read
///   and reintroduce the #633 direction-A use-after-free D-181 closed.
///   The *list* `Subscript` branch is not borrowed: D-141's
///   container ingress rejects bigints outright, so a list element is
///   always a smallint the guard filters out anyway.
///
/// Everything else is **owning**, including every other container egress
/// (`DictGet`, `ListPop`, `DictGetOrDefault`): those all read D-141
/// container payloads, which by that same ingress rule are never heap
/// bigints, so classifying them owning emits a release whose inline guard
/// is statically false in practice and can never free a live word.
///
/// `Call { ty: Int }` is owning **conditionally**, and the premise is worth
/// stating: a user function's `return` runs `retain_if_int_duplicate` on
/// its value (D-180 decision 6) and that retain has no matching release at
/// the callee's boundary (D-180 residual 3), so a returned word does arrive
/// with a reference the caller may retire. Should that boundary ever gain
/// its own release, this classification must change with it. `len` needs no
/// such argument: it lowers to `Call { callee: "len", ty: Int }` whose
/// result is `raw_i64_to_tagged_int`'s odd-tagged smallint, so its release
/// is an unconditional runtime no-op.
fn int_value_is_a_duplicate_reference(expr: &MirExpr) -> bool {
    match expr {
        // #1346: a sequence yields its `value`'s word unchanged, so it owns
        // exactly what `value` owns.
        MirExpr::Sequence { value, .. } => int_value_is_a_duplicate_reference(value),
        MirExpr::Name {
            ty: pycc_mir::Ty::Int,
            ..
        }
        | MirExpr::AttrGet {
            ty: pycc_mir::Ty::Int,
            ..
        }
        // PEP 572 (#774): `target := value`. Codegen for `NamedExpr` stores
        // `value` into `name`'s slot and then re-reads that same slot to
        // yield the result -- exactly the "assignment followed by a
        // name-load" shape the issue's own design calls for (see
        // `pycc_codegen::emit_expr_unchecked`'s `MirExpr::NamedExpr` arm).
        // The yielded word is therefore a borrow of the slot's own
        // reference, precisely like `Name { ty: Int }` immediately above,
        // not a second owned reference -- classifying it `owning` here
        // would make the temporary-release logic free a word the slot
        // still owns, reintroducing the same use-after-free class D-181
        // closed for plain names.
        | MirExpr::NamedExpr {
            ty: pycc_mir::Ty::Int,
            ..
        }
        // Issue #769 (Part 2 of #747; re-verified for #809's widened
        // `T0049`): unlike `OptionalWrap` (whose `.ty()` is always
        // `Ty::Optional(_)`, so it can never reach this `Ty::Int`-classified
        // function at all -- see its own arm below), `OptionalUnwrap`'s
        // `.ty()` (`pycc_mir::MirExpr::ty`) is `(**inner).clone()` -- it
        // reaches *this* `Ty::Int`-scoped arm only for the `T = int` case;
        // `T0049` now also admits `T = float`/`T = bool`
        // (`crates/pycc_hir/src/func.rs`), but those give `.ty() ==
        // Ty::Float`/`Ty::Bool`, which this function's own `Name { ty:
        // Ty::Int, .. }` sibling arm's exact-`Ty::Int` guard already
        // excludes -- and this whole function is reached only via
        // `int_temporary_word`'s own `source_expr.ty() == pycc_mir::Ty::Int`
        // short-circuit guard, so a float/bool-inner `OptionalUnwrap` never
        // reaches here regardless. For the `T = int` case that does reach
        // here, its codegen arm (`emit_expr`'s `OptionalUnwrap` arm in
        // `lib.rs`) performs a plain `build_extract_value` out of a struct
        // that was itself just loaded from the narrowed name's own local
        // slot -- the exact same "borrowed read out of a slot this
        // expression does not own" shape as the plain `Name { ty: Ty::Int,
        // .. }` arm immediately above, so it is classified identically:
        // `true` (a duplicate reference that needs its own retain wherever
        // it is stored into a second owning slot).
        | MirExpr::OptionalUnwrap(_, _) => true,
        // No `Ty::Int` test alongside the tuple test, for the same reason
        // `retain_if_int_duplicate`'s own tuple arm omits one: the only
        // caller, `int_temporary_word`, has already established that this
        // expression is `Ty::Int`, so a `Ty::Float`/`Ty::Bool` tuple field
        // never reaches this arm and testing for it here would be a
        // permanently-false branch.
        MirExpr::Subscript { base, .. } => matches!(base.ty(), pycc_mir::Ty::Tuple(_)),
        // One combined arm for every remaining variant, including the
        // non-`Ty::Int` `Name`/`AttrGet` reads the two arms above did not
        // claim. Grouped rather than enumerated one-per-line so the whole
        // "owning" answer is a single coverage region instead of twenty.
        MirExpr::Name { .. }
        | MirExpr::AttrGet { .. }
        | MirExpr::IntLiteral(_)
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::IntBoundary(_)
        | MirExpr::StringLiteral(_)
        | MirExpr::NoneLiteral
        | MirExpr::NotImplemented
        | MirExpr::Call { .. }
        | MirExpr::BinOp { .. }
        | MirExpr::Compare { .. }
        | MirExpr::FString(_)
        | MirExpr::ListLiteral(_)
        | MirExpr::EmptyList(_)
        | MirExpr::ListAppend { .. }
        | MirExpr::DictLiteral(_)
        | MirExpr::EmptyDict(_)
        | MirExpr::DictGet { .. }
        | MirExpr::SetLiteral { .. }
        | MirExpr::TupleLiteral(_)
        | MirExpr::Slice { .. }
        | MirExpr::ListPop { .. }
        | MirExpr::DictGetOrDefault { .. }
        | MirExpr::SetAdd { .. }
        | MirExpr::Instantiate(_)
        | MirExpr::NullInstance { .. }
        // `ExceptionMessage`'s own `.ty()` is always `Ty::Str` (Part 3A of
        // #541, #736), never `Ty::Int`, so like `OptionalWrap` immediately
        // below it can never reach this function as the `Ty::Int`-classified
        // expression `int_temporary_word` passes in; it joins the combined
        // "owning" answer for the same reason.
        | MirExpr::ExceptionMessage(_)
        // `ExceptionTypeTest`'s `.ty()` is always `Ty::Bool` (#1337), never
        // `Ty::Int`, for the same reason as `ExceptionMessage` above.
        | MirExpr::ExceptionTypeTest { .. }
        // `OptionalWrap`'s own `.ty()` is always `Ty::Optional(_)`, never
        // `Ty::Int` (D-197, #763), so like every other non-`Ty::Int`
        // variant grouped in this arm it can never reach this function as
        // the `Ty::Int`-classified expression `int_temporary_word` passes
        // in; it joins the combined "owning" answer for the same reason.
        | MirExpr::OptionalWrap(_, _)
        // `ObjectBox`'s `.ty()` is always `Ty::Object` (Part 2 of #1387),
        // for the same reason as `OptionalWrap` immediately above.
        | MirExpr::ObjectBox(_)
        // #1476: an `int` narrowed read is a word the shim just encoded
        // from the CPython object -- always inline (a value outside the
        // inline range raises instead), so owning it releases nothing.
        | MirExpr::ObjectUnbox(_, _)
        // Non-`Ty::Int` `NamedExpr` joins the combined "owning" answer for
        // the same reason the non-`Ty::Int` `Name`/`AttrGet` arms do just
        // above: `int_temporary_word`'s caller has already established
        // `Ty::Int`, so this arm is reached only when `NamedExpr`'s own
        // `.ty()` is *not* `Ty::Int` and the specific arm above did not
        // match -- a permanently-unreachable-for-Int case grouped here
        // purely so the match stays exhaustive.
        | MirExpr::NamedExpr { .. }
        // #1254 (D-250): a comprehension's `.ty()` is always a container,
        // never `Ty::Int`, so like `Not` below it can never reach this
        // function; it joins the combined answer for the same reason.
        | MirExpr::Comprehension(_)
        // `Not`'s own `.ty()` is always `Ty::Bool` (#604, Part 3 of #573),
        // never `Ty::Int`, so like `OptionalWrap`/`ExceptionMessage` above
        // it can never reach this function as the `Ty::Int`-classified
        // expression `int_temporary_word` passes in; it joins the combined
        // "owning" answer for the same reason.
        | MirExpr::Not(_)
        // #1211 (Part 3 of #1018): an `int`-typed `and`/`or` result is
        // always owned. Each value arm retains a duplicate operand inside
        // that arm (`boolop.rs`), so the joined word is a fresh reference
        // whichever operand was selected.
        | MirExpr::BoolOp { .. }
        // #1395: an `int`-typed conditional expression result is always
        // owned. Each branch arm retains a duplicate branch value inside
        // that arm (`if_exp.rs`), so the joined word is a fresh reference
        // whichever branch ran.
        | MirExpr::IfExp { .. }
        // #1212 (Part 4 of #1018): a chained comparison's `.ty()` is always
        // `Ty::Bool`, never `Ty::Int`, so like `Not` above it can never
        // reach this function; it joins the combined answer for the same
        // reason.
        | MirExpr::CompareChain { .. }
        // D-244, Part 2 of #1026: `ObjAttrGet`'s own `.ty()` is always
        // `Ty::Object`, never `Ty::Int`, so like `OptionalWrap`/`Not` above
        // it can never reach this function as the `Ty::Int`-classified
        // expression `int_temporary_word` passes in; it joins the combined
        // "not a duplicate reference" answer for the same reason.
        | MirExpr::ObjAttrGet { .. }
        // PR 2b of #1081: `ObjMethodCall` joins `ObjAttrGet` directly above
        // on the identical argument -- its own `.ty()` is always
        // `Ty::Object`, never `Ty::Int`.
        | MirExpr::ObjMethodCall { .. }
        // #1313: `ObjCall`'s `.ty()` is always `Ty::Object` too.
        | MirExpr::ObjCall { .. }
        // Part 8 of #1371: and so is its keyword form's.
        | MirExpr::ObjKeywordCall(_)
        // PR 3a of #1082: `ObjLen`'s own `.ty()` *is* `Ty::Int`, so unlike
        // the three nodes above it really can reach this function. It joins
        // the "owning" answer on the scalar `len`'s own argument, restated
        // in the doc comment above: `pycc_ext_obj_len` writes a word that
        // `pycc_rt_ext_int_encode` produced, and that encode *fails* outside
        // D-141's inline range, so the word is always an inline smallint and
        // the release this classification emits is an unconditional runtime
        // no-op.
        | MirExpr::ObjLen { .. }
        // PR 3b of #1082: `ObjSubscript` joins `ObjMethodCall` rather than
        // `ObjLen` -- its own `.ty()` is always `Ty::Object`, never
        // `Ty::Int`, so it can never reach this function either.
        | MirExpr::ObjSubscript { .. }
        // PR 4c of #1083: `ObjUnpackFloatTuple` joins `ObjSubscript` rather
        // than `ObjLen` -- its own `.ty()` is always a `Ty::Tuple` of
        // `float`s, never `Ty::Int`, so it can never reach this function
        // either.
        | MirExpr::ObjUnpackFloatTuple { .. }
        // Part 1 of #891: `ObjUnpack`'s `.ty()` is always `Ty::Object`.
        | MirExpr::ObjUnpack { .. }
        // Part 2 of #1027: `BufferGet` joins them for the same structural
        // reason -- its own `.ty()` is always `Ty::Float`, never `Ty::Int`,
        // so it can never reach this function at all.
        | MirExpr::BufferGet { .. }
        // #1116: `BufferLen` joins `ObjLen` rather than `BufferGet` -- its
        // own `.ty()` *is* `Ty::Int`, so it really can reach this function,
        // and it joins the "owning" answer because codegen builds its word
        // with `raw_i64_to_tagged_int` from a raw `i64` count. That always
        // yields an inline D-141 smallint (a buffer length is bounded by the
        // exporter's `shape[0]`), so the release this classification emits
        // is an unconditional runtime no-op, exactly as for the scalar
        // `len` `Call` grouped above.
        | MirExpr::BufferLen { .. }
        // #1165: `BufferAlloc` joins `BufferGet`'s group rather than
        // `BufferLen`'s answer-with-a-reason -- its own `.ty()` is always
        // `Ty::MemoryView`, never `Ty::Int`, so like `BufferGet` it can
        // never reach this function at all.
        | MirExpr::BufferAlloc { .. }
        // Part 1 of #1319: its `.ty()` is always a `frozenset`, never
        // `Ty::Int`, so it can never reach this function either.
        | MirExpr::FrozenSetFrom { .. }
        // #1335: `hash(instance)` *is* `Ty::Int`, and owning: its word is
        // born by `pycc_rt_int_from_i64`, exactly like `hash(x)`'s `Call`.
        | MirExpr::InstanceHash { .. }
        // Part 1 of #1371: an object comparison's `.ty()` is `Ty::Object`
        // or `Ty::Bool`, and `isinstance`'s is `Ty::Bool` -- never
        // `Ty::Int` -- so neither can reach this function.
        | MirExpr::ObjCompare { .. }
        | MirExpr::ObjIsInstance { .. }
        // Part 2b of #1371: a membership test's `.ty()` is `Ty::Bool` and an
        // object slice's is `Ty::Object` -- never `Ty::Int`.
        | MirExpr::ObjContains { .. }
        | MirExpr::ObjSlice { .. }
        // Part 11 of #1371: `type(o)`'s `.ty()` is `Ty::Object`.
        | MirExpr::ObjType { .. }
        // Part 2d of #1371: a list display built as a CPython `list` is
        // `Ty::Object`.
        | MirExpr::ObjList { .. } => false,
    }
}

/// Releases the birth reference of a freshly built `int` word at a site
/// that consumes the word and then discards it (D-181, Part 2 of #146).
///
/// The mirror of `retain_if_int_duplicate`: that helper adds a reference
/// where a value acquires a *second* owner, this one retires the reference
/// a value was born with once its single consumer is done with it. Callers
/// pass the *source expression* -- ownership is a compile-time property of
/// how the word was produced, not something recoverable from the `i64`.
///
/// `MirExpr` is a tree and this crate's emitters evaluate each node exactly
/// once, so every produced word has exactly one consumer and the release
/// dominates nothing else that reads it. Exactly-once does *not* hold: a
/// D-173 exception edge can branch out of an expression after an operand
/// has been evaluated but before the release below is reached, so a pending
/// exception skips it. That is an enumerated D-181 residual leak, not an
/// unsoundness -- skipping a release can only leak.
///
/// The `Ty::Int` test is not redundant with the classification: callers
/// pass the word *after* `to_numeric_encoded_int`, which promotes a
/// `Ty::Bool` operand into an encoded word. Such a word is one of D-141's
/// two bool-identity markers, never a heap pointer, so there is nothing to
/// release and no reason to emit a guard for it.
pub(super) fn release_if_int_temporary<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    word: IntValue<'ctx>,
) {
    if let Some(word) = int_temporary_word(source_expr, word) {
        emit_bigint_refcount_call(context, builder, rt, word, BigIntRefcount::Release);
    }
}

/// The classification behind `release_if_int_temporary`, exposed as a value
/// for the one shape that cannot release at the point of decision: a
/// comprehension's `stop`/`step` range operands, which `pycc_rt_range_
/// continue` re-reads on every iteration and which therefore have to be
/// released in the loop's `after_bb` instead.
///
/// `Some(word)` means "this word carries a birth reference someone must
/// retire"; `None` means it is borrowed, or not a `Ty::Int` value at all.
pub(super) fn int_temporary_word<'ctx>(
    source_expr: &MirExpr,
    word: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    (source_expr.ty() == pycc_mir::Ty::Int && !int_value_is_a_duplicate_reference(source_expr))
        .then_some(word)
}

/// `release_if_int_temporary` for a site that still holds a `Scalar` rather
/// than an already-encoded word (a discarded statement result, a loop or
/// `if` condition, a `print`/f-string operand).
///
/// A non-`Scalar::Int` value owns no bigint reference by construction, so
/// those kinds fall through untouched.
pub(super) fn release_scalar_if_int_temporary<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) {
    if let Scalar::Int(word) = scalar {
        release_if_int_temporary(context, builder, rt, source_expr, *word);
    }
}

/// Pushes `source_expr`'s already-evaluated `word` onto
/// `rt.exceptions.pending_int_releases` (#638, D-208) when it is a fresh,
/// not-yet-released `Ty::Int` birth reference -- the same classification
/// `release_if_int_temporary` uses, exposed here so a caller can protect an
/// operand across a sibling's evaluation instead of releasing it
/// immediately. Returns the pushed word (for the matching
/// [`pop_pending_int_release`] call) or `None` when nothing was pushed
/// (borrowed, non-`Ty::Int`, or a duplicate reference -- see
/// [`int_temporary_word`]).
///
/// Every call site pushes right after the operand's own `emit_expr` (and,
/// where applicable, `retain_if_int_duplicate`) call returns, before the
/// next sibling is evaluated -- see `guard_statement_effects`'s own doc
/// comment for why this ordering is what makes the exception-edge release
/// correct for arbitrarily nested sibling evaluation.
pub(super) fn push_pending_int_release_if_temporary<'ctx>(
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    word: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let pushed = int_temporary_word(source_expr, word);
    if let Some(w) = pushed {
        push_word_onto_pending_int_releases(rt, w);
    }
    pushed
}

/// The innermost primitive both
/// [`push_pending_int_release_if_temporary`] and
/// [`retain_if_int_duplicate_and_track_for_exception_edge`] share: pushes an
/// already-classified `word` onto `rt.exceptions.pending_int_releases`
/// unconditionally. Extracted rather than duplicated so the two callers'
/// classification logic stays free to diverge (an owning temporary's word
/// vs. a duplicate's retained word) while the actual stack mutation has one
/// definition.
fn push_word_onto_pending_int_releases<'ctx>(rt: &RtFns<'ctx>, word: IntValue<'ctx>) {
    rt.exceptions.pending_int_releases.borrow_mut().push(word);
}

/// The `Scalar`-holding counterpart of
/// [`push_pending_int_release_if_temporary`], mirroring how
/// `release_scalar_if_int_temporary` relates to `release_if_int_temporary`:
/// a non-`Scalar::Int` value owns no bigint reference by construction and is
/// never pushed.
pub(super) fn push_pending_int_release_if_scalar_temporary<'ctx>(
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) -> Option<IntValue<'ctx>> {
    if let Scalar::Int(word) = scalar {
        push_pending_int_release_if_temporary(rt, source_expr, *word)
    } else {
        None
    }
}

/// Pops the word `push_pending_int_release_if_temporary`/
/// `push_pending_int_release_if_scalar_temporary` pushed, once the sibling
/// evaluation it was protecting against has completed on the normal
/// (fallthrough) path. A `None` `pushed` value means the push was a no-op,
/// so the pop is one too -- callers do not need their own `if let Some`
/// guard around this call.
///
/// Asserts (debug-only, per `bigint_rc.rs`'s own established convention of
/// `assert!`/`expect` over a `panic!`-in-a-closure shape) that the popped
/// value is exactly the one that was pushed: any mismatch means a push/pop
/// pair was nested incorrectly at some call site, which is a codegen bug
/// this fix must not silently miscompile past.
pub(super) fn pop_pending_int_release<'ctx>(rt: &RtFns<'ctx>, pushed: Option<IntValue<'ctx>>) {
    if let Some(expected) = pushed {
        let popped = rt.exceptions.pending_int_releases.borrow_mut().pop();
        debug_assert_eq!(
            popped,
            Some(expected),
            "push/pop mismatch on pending_int_releases"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

    // -----------------------------------------------------------------
    // #624 (D-180) codegen-depth: the inline bigint-refcount guard.
    //
    // The behavioral tests for this feature live in
    // `tests/issue_146_bigint_release.rs` (real source through the real
    // CLI, plus two peak-RSS oracles). Those prove the *net effect* -- a
    // loop's peak RSS stops tracking its trip count -- but they cannot see
    // the two structural properties the emitter is actually responsible
    // for, because both are invisible in a program's output and in its RSS:
    //
    //   1. Every `pycc_rt_bigint_retain`/`_release` call is reached only
    //      through `emit_bigint_refcount_call`'s inline `(word & 0b11) == 0
    //      && word != 0` guard, on the *same* word the call receives. An
    //      unguarded call is correct-but-slow, so it regresses D-084/D-140's
    //      nbody throughput floor silently rather than failing a test.
    //   2. That guard splits the current basic block, so any caller
    //      recording a block for a phi incoming edge must re-read
    //      `get_insert_block()` afterwards. A stale phi predecessor is
    //      malformed IR that LLVM's own verifier rejects.
    //
    // These are in-crate rather than in `tests/` on purpose:
    // `compile_to_object_with_observer`, the only way to reach the emitted
    // module before it becomes an object file, is private -- so this
    // follows `an_oversized_int_literal_materializes_a_runtime_bigint` in
    // the crate root's own test module (`tests.rs`), which is this
    // repository's actual
    // IR-inspection convention, rather than
    // `tests/slice1_codegen_depth.rs`'s
    // hand-built-MIR-through-`compile_to_object` one.
    //
    // They live beside the emitter they exercise rather than in the crate
    // root's test module, under AGENTS.md's decomposability rule: this
    // cluster is
    // self-contained -- no test outside it uses `RefcountCall`,
    // `guarded_bigint_refcount_calls`, `refcount_calls_in` or `int_name` --
    // so it is exactly the cohesive unit that rule points at.
    // -----------------------------------------------------------------

    /// One `pycc_rt_bigint_retain`/`_release` call recovered from emitted
    /// LLVM IR: the callee's bare symbol name and the encoded word it is
    /// actually passed.
    #[derive(Debug, PartialEq, Eq)]
    struct RefcountCall {
        callee: String,
        word: String,
    }

    /// Returns every `pycc_rt_bigint_*` refcount call in `ir`, and panics
    /// unless each one is dominated by exactly the guard
    /// `emit_bigint_refcount_call` emits, evaluated on the same operand the
    /// call receives.
    ///
    /// The chain proved per call, backwards from the call:
    /// the call sits in a `bigint_rc_call*` block; that block is the taken
    /// arm of `br i1 %c, label %bigint_rc_call*, label %bigint_rc_cont*`;
    /// `%c = and i1 %a, %b`; `%a = icmp eq i64 %t, 0` with `%t = and i64
    /// %w, 3`; `%b = icmp ne i64 %w, 0`; and the call's own argument is
    /// that same `%w`.
    fn guarded_bigint_refcount_calls(ir: &str) -> Vec<RefcountCall> {
        // Every name `emit_bigint_refcount_call` produces is a fixed
        // literal, and LLVM disambiguates those only *within* a function --
        // the first guard of two different functions is `%bigint_is_heap`
        // in both. So each function body is analyzed on its own maps, and a
        // call can never be validated against a guard chain read out of
        // some other function. `split` yields the module header first,
        // which `skip(1)` drops.
        ir.split("\ndefine ")
            .skip(1)
            .flat_map(guarded_bigint_refcount_calls_in_one_function)
            .collect()
    }

    /// `guarded_bigint_refcount_calls` for a single `define` body.
    fn guarded_bigint_refcount_calls_in_one_function(ir: &str) -> Vec<RefcountCall> {
        // `%name = <rhs>` for every SSA value in the function.
        let mut defs: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        // `taken block -> (condition, not-taken block)` for every `br i1`.
        let mut branches: std::collections::HashMap<&str, (&str, &str)> =
            std::collections::HashMap::new();
        // Every refcount call, tagged with the block it was found in.
        let mut calls: Vec<(String, RefcountCall)> = Vec::new();
        let mut block = String::new();

        for raw in ir.lines() {
            let line = raw.trim();
            // A block label is the only unindented `name:` line inside a
            // function body; LLVM appends a `; preds = ...` comment to it.
            // The leftover of the `define` line itself is the only such
            // line here, and it carries no colon.
            if !raw.starts_with([' ', '@', '%', '}', '!']) {
                if let Some((label, _)) = line.split_once(':') {
                    block = label.to_string();
                }
                continue;
            }
            if let Some((name, rhs)) = line.split_once(" = ") {
                defs.insert(name, rhs);
            }
            if let Some(rest) = line.strip_prefix("br i1 ") {
                let parts: Vec<&str> = rest.split(", label ").collect();
                assert_eq!(
                    parts.len(),
                    3,
                    "a conditional branch always has a condition and two targets"
                );
                branches.insert(parts[1], (parts[0], parts[2]));
            }
            if let Some(rest) = line.strip_prefix("call void @pycc_rt_bigint_") {
                let (op, args) = rest
                    .split_once('(')
                    .expect("a call instruction always has an argument list");
                let word = args
                    .trim_end_matches(')')
                    .strip_prefix("i64 ")
                    .expect("both refcount entry points take exactly one i64")
                    .to_string();
                calls.push((
                    block.clone(),
                    RefcountCall {
                        callee: format!("pycc_rt_bigint_{op}"),
                        word,
                    },
                ));
            }
        }

        // Only `assert*!` and `expect` are used from here on, never a
        // local `unwrap_or_else(|| panic!(..))` closure: an error closure
        // that never runs is an uncovered function under D-014's region
        // gate, whereas `assert!`'s cold arm lives in `core`.
        for (block, call) in &calls {
            assert!(
                block.starts_with("bigint_rc_call"),
                "{} is emitted in block `{block}`, not a guard's own call block",
                call.callee
            );
            let key: &str = &format!("%{block}");
            let (cond, cont) = *branches
                .get(key)
                .expect("a guard's call block is always the target of its own branch");
            assert!(
                cont.starts_with("%bigint_rc_cont"),
                "`{block}`'s not-taken arm is `{cont}`, not a guard continuation"
            );
            let (aligned, non_zero) = defs[cond]
                .strip_prefix("and i1 ")
                .and_then(|rest| rest.split_once(", "))
                .expect("the guard condition is an `and` of its two halves");
            let low_tag = defs[aligned]
                .strip_prefix("icmp eq i64 ")
                .and_then(|rest| rest.strip_suffix(", 0"))
                .expect("the guard's alignment half compares a masked word against 0");
            let tagged_word = defs[low_tag]
                .strip_prefix("and i64 ")
                .and_then(|rest| rest.strip_suffix(", 3"))
                .expect("the guard's alignment half masks the word with 0b11");
            let tested_word = defs[non_zero]
                .strip_prefix("icmp ne i64 ")
                .and_then(|rest| rest.strip_suffix(", 0"))
                .expect("the guard's non-zero half compares the word against 0");
            assert_eq!(
                tagged_word, tested_word,
                "the guard's two halves test different words"
            );
            assert_eq!(
                tagged_word, call.word,
                "{} is passed a word the guard never tested",
                call.callee
            );
        }

        calls.into_iter().map(|(_, call)| call).collect()
    }

    /// Compiles `mir` and runs `guarded_bigint_refcount_calls` over the
    /// emitted module.
    ///
    /// LLVM's own verifier is what catches a phi whose incoming block
    /// stopped being a real predecessor after `emit_bigint_refcount_call`
    /// split it -- but this helper deliberately does not call it. Callers do
    /// not need to: `compile_to_object_with_observer` already runs
    /// `verify_module` on this exact module *before* it invokes the
    /// observer, so simply reaching this closure means the module already
    /// passed. Calling `module.verify()` a second time here would bypass
    /// `verify_module`'s `#[cfg(windows)]` skip and reintroduce D-029:
    /// inkwell's `Module::verify` calls `LLVMDisposeMessage` on its
    /// *success* path too, which faults against the prebuilt LLVM 22.1.1
    /// the Windows runner uses -- an access violation that kills the whole
    /// test binary rather than failing one test.
    fn refcount_calls_in(label: &str, mir: &MirModule) -> Vec<RefcountCall> {
        refcount_calls_with_options(label, mir, &CompileOptions::default())
    }

    /// `refcount_calls_in` with an explicit [`CompileOptions`], for the one
    /// fixture whose MIR only compiles under `ext: true` (a `memoryview`
    /// parameter, which D-244 admits nowhere else).
    fn refcount_calls_with_options(
        label: &str,
        mir: &MirModule,
        options: &CompileOptions,
    ) -> Vec<RefcountCall> {
        let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
        let obj_path = dir.join(format!("{label}.o"));
        let mut calls = Vec::new();
        // D-029: never let `print_to_string`'s `LLVMString` drop; route it
        // through `llvm_string_to_owned` exactly as the sibling tests do.
        let mut observer = |module: &inkwell::module::Module<'_>, _| {
            calls = guarded_bigint_refcount_calls(&llvm_string_to_owned(module.print_to_string()));
        };
        compile_to_object_with_observer(mir, &obj_path, options, Some(&mut observer))
            .expect("codegen should succeed");
        calls
    }

    fn int_name(name: &str) -> MirExpr {
        MirExpr::Name {
            name: name.to_string(),
            ty: Ty::Int,
        }
    }

    // -----------------------------------------------------------------
    // #638 (D-208) codegen-depth: releasing a bigint temporary on the
    // exception-unwinding edge.
    //
    // `tests/issue_638_bigint_exception_release.rs` proves the net effect
    // through the real CLI (`pycc run`'s value-correctness assertions and
    // `pycc build`'s peak-RSS marginal comparison), but neither can see the
    // structural property `guard_statement_effects`'s own `#638` doc
    // comment describes: an `effect_exc_unwind` block, guarded exactly like
    // every other `guard_statement_effects` site, releasing the pending
    // stack's snapshot before branching to the installed exception target
    // -- and, symmetrically, that this extra block is never emitted at all
    // when nothing is pending (the codegen-time check that keeps the
    // D-084/D-140 nbody hot loop's own guard sites at their original
    // two-block shape). These follow the refcount-call cluster's own
    // `compile_to_object_with_observer` convention immediately above.
    // -----------------------------------------------------------------

    /// Compiles `mir` and returns the emitted module's textual IR.
    ///
    /// Skips `guarded_bigint_refcount_calls`'s own per-call guard-chain
    /// proof (this cluster is checking for the presence/absence of an
    /// entire extra block, not validating every call's guard) but keeps
    /// its D-029 `LLVMString`-disposal discipline: route the IR through
    /// `llvm_string_to_owned` rather than let `print_to_string`'s return
    /// value drop on its own.
    fn emitted_ir(label: &str, mir: &MirModule) -> String {
        let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
        let obj_path = dir.join(format!("{label}.o"));
        let mut ir = String::new();
        let mut observer = |module: &inkwell::module::Module<'_>, _| {
            ir = llvm_string_to_owned(module.print_to_string());
        };
        compile_to_object_with_observer(
            mir,
            &obj_path,
            &CompileOptions::default(),
            Some(&mut observer),
        )
        .expect("codegen should succeed");
        ir
    }

    #[test]
    fn an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge() {
        // `(n + n) + (1 // n)`: the outer `+`'s left operand is a fresh,
        // owned `BinOp` result -- `int_temporary_word` classifies it as a
        // pending release candidate before the right operand (`1 // n`, a
        // `FloorDiv`, one of `expression_can_set_exception`'s three
        // exception-settable `BinOp`s) is evaluated. Reaching `1 // n`'s own
        // `guard_statement_effects` call with that word still pending must
        // emit an `effect_exc_unwind` block that releases it.
        let owned_sum = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(int_name("n")),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let raising_floordiv = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::FloorDiv,
            left: Box::new(MirExpr::IntLiteral(1)),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "outer_binop_exception_edge".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Add,
                    left: Box::new(owned_sum),
                    right: Box::new(raising_floordiv),
                    ty: Ty::Int,
                }))],
            }],
            class_defs: Vec::new(),
        };
        let ir = emitted_ir("bigint_rc_exc_unwind_present", &mir);
        assert!(
            ir.contains("effect_exc_unwind"),
            "a pending owned operand at the exception guard must emit the \
             #638 unwind block; got:\n{ir}"
        );
        // `contains()` above matches the branch instruction's own label
        // reference (`label %effect_exc_unwind`) as readily as the block's
        // definition, so isolate the block's own body by its label
        // *definition* line specifically (an unindented `effect_exc_unwind:`
        // line, per `guarded_bigint_refcount_calls_in_one_function`'s own
        // convention for recognizing a block label).
        //
        // This cannot split on the next `\neffect_exc_cont` occurrence: in
        // `guard_statement_effects`, `effect_exc_cont` is *created* before
        // `effect_exc_unwind` (so it is appended, and therefore emitted in
        // the IR text, first) whenever the pending-releases stack is
        // non-empty -- exactly this test's fixture. For a single guard site
        // that puts the only `effect_exc_cont` occurrence *before*
        // `effect_exc_unwind:` in the string, so splitting the
        // already-past-that-point substring on `\neffect_exc_cont` would
        // never find a second occurrence and `.next()` would silently hand
        // back the rest of the entire module -- including the fallthrough
        // path's own D-181 release calls -- rather than failing loudly.
        // Instead, treat `effect_exc_unwind` as the root of its own small
        // control-flow subgraph and walk it structurally: the block itself
        // ends in a *conditional* branch (`emit_bigint_refcount_call`'s own
        // heap-tag guard, per its doc comment on `guarded_bigint_refcount_calls`
        // above), whose taken arm (`bigint_rc_call*`) holds the actual
        // release and whose not-taken arm (`bigint_rc_cont*`) re-joins to
        // branch on to the exception target. Parse every basic block's body
        // and outgoing edges once, then collect the closure reachable from
        // `effect_exc_unwind` -- this follows exactly the blocks
        // `guard_statement_effects`'s own snapshot loop emits, regardless of
        // where in the text another guard site's blocks happen to land.
        let mut block_bodies: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        let mut block_order: Vec<&str> = Vec::new();
        let mut current_block: Option<&str> = None;
        for raw in ir.lines() {
            let line = raw.trim();
            if !raw.starts_with([' ', '@', '%', '}', '!']) {
                if let Some((label, _)) = line.split_once(':') {
                    current_block = Some(label);
                    block_order.push(label);
                    block_bodies.entry(label).or_default();
                }
                continue;
            }
            if let Some(label) = current_block {
                block_bodies.get_mut(label).unwrap().push(line);
            }
        }
        assert!(
            block_bodies.contains_key("effect_exc_unwind"),
            "the block label was just found by contains() above, so it \
             must parse as a block; got:\n{ir}"
        );
        // A block's outgoing edges, read off its own terminator line.
        let successors = |label: &str| -> Vec<&str> {
            let body = &block_bodies[label];
            let terminator = body
                .last()
                .expect("every parsed block has a terminator line");
            if let Some(rest) = terminator.strip_prefix("br i1 ") {
                rest.split(", label ")
                    .skip(1)
                    .map(|s| s.trim_start_matches('%'))
                    .collect()
            } else if let Some(rest) = terminator.strip_prefix("br label ") {
                vec![rest.trim_start_matches('%')]
            } else {
                Vec::new()
            }
        };
        let mut visited: Vec<&str> = Vec::new();
        let mut stack = vec!["effect_exc_unwind"];
        while let Some(label) = stack.pop() {
            if visited.contains(&label) {
                continue;
            }
            visited.push(label);
            stack.extend(successors(label));
        }
        // Report the reachable blocks in the order they appear in the IR so
        // the assertion failure message below reads like a normal block
        // listing rather than a BFS/DFS visitation order.
        let unwind_block = block_order
            .iter()
            .filter(|label| visited.contains(label))
            .flat_map(|label| block_bodies[label].iter().copied())
            .collect::<Vec<&str>>()
            .join("\n");
        assert!(
            unwind_block.contains("call void @pycc_rt_bigint_release"),
            "the #638 unwind block must release the pending operand before \
             branching to the exception target; block body:\n{unwind_block}"
        );
    }

    #[test]
    fn a_lone_raising_binop_with_nothing_pending_emits_no_unwind_block() {
        // `1 // n` alone: no sibling has already produced an owned `Ty::Int`
        // temporary before this `FloorDiv`'s own `guard_statement_effects`
        // call, so `rt.exceptions.pending_int_releases` is empty when that
        // call runs. This is the codegen-time check
        // `guard_statement_effects`'s own doc comment describes as what
        // keeps the D-084/D-140 nbody hot loop's guard sites at their
        // original two-block shape -- proven here directly rather than only
        // by the nbody throughput gate, which this environment cannot run
        // locally (no exact CPython 3.14.7 oracle available).
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "lone_raising_binop".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::FloorDiv,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(int_name("n")),
                    ty: Ty::Int,
                }))],
            }],
            class_defs: Vec::new(),
        };
        let ir = emitted_ir("bigint_rc_exc_unwind_absent", &mir);
        assert!(
            !ir.contains("effect_exc_unwind"),
            "a guard site with nothing pending must keep its original \
             two-block shape, not gain an extra unwind block; got:\n{ir}"
        );
    }

    #[test]
    fn an_early_tuple_elements_owned_word_is_released_on_a_later_elements_exception_edge() {
        // `(n + n, n, 1 // n)`: the first element is a fresh, owned `BinOp`
        // result -- `int_temporary_word` classifies it as a pending release
        // candidate before the third element (`1 // n`, a `FloorDiv`, one of
        // `expression_can_set_exception`'s three exception-settable
        // `BinOp`s) is evaluated. The second element (a bare `Name` read) is
        // a *borrowed* reference and must not itself be pushed. Reaching
        // `1 // n`'s own `guard_statement_effects` call with the first
        // element's word still pending must emit an `effect_exc_unwind`
        // block that releases it -- exactly mirroring
        // `an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge`
        // above, but for `MirExpr::TupleLiteral`'s own element loop rather
        // than `build_call_to_with_leading_args`'s or `BinOp`'s.
        let owned_sum = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(int_name("n")),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let raising_floordiv = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::FloorDiv,
            left: Box::new(MirExpr::IntLiteral(1)),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "tuple_literal_exception_edge".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                    base: Box::new(MirExpr::TupleLiteral(vec![
                        owned_sum,
                        int_name("n"),
                        raising_floordiv,
                    ])),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }))],
            }],
            class_defs: Vec::new(),
        };
        let ir = emitted_ir("bigint_rc_tuple_exc_unwind_present", &mir);
        assert!(
            ir.contains("effect_exc_unwind"),
            "a pending owned tuple element at a later element's exception \
             guard must emit the #638 unwind block; got:\n{ir}"
        );
        // Same structural walk as
        // `an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge`
        // above: isolate `effect_exc_unwind`'s own reachable closure rather
        // than splitting on the next `effect_exc_cont` occurrence, for the
        // identical reason that test's own comment explains.
        let mut block_bodies: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        let mut block_order: Vec<&str> = Vec::new();
        let mut current_block: Option<&str> = None;
        for raw in ir.lines() {
            let line = raw.trim();
            if !raw.starts_with([' ', '@', '%', '}', '!']) {
                if let Some((label, _)) = line.split_once(':') {
                    current_block = Some(label);
                    block_order.push(label);
                    block_bodies.entry(label).or_default();
                }
                continue;
            }
            if let Some(label) = current_block {
                block_bodies.get_mut(label).unwrap().push(line);
            }
        }
        assert!(
            block_bodies.contains_key("effect_exc_unwind"),
            "the block label was just found by contains() above, so it \
             must parse as a block; got:\n{ir}"
        );
        let successors = |label: &str| -> Vec<&str> {
            let body = &block_bodies[label];
            let terminator = body
                .last()
                .expect("every parsed block has a terminator line");
            if let Some(rest) = terminator.strip_prefix("br i1 ") {
                rest.split(", label ")
                    .skip(1)
                    .map(|s| s.trim_start_matches('%'))
                    .collect()
            } else if let Some(rest) = terminator.strip_prefix("br label ") {
                vec![rest.trim_start_matches('%')]
            } else {
                Vec::new()
            }
        };
        let mut visited: Vec<&str> = Vec::new();
        let mut stack = vec!["effect_exc_unwind"];
        while let Some(label) = stack.pop() {
            if visited.contains(&label) {
                continue;
            }
            visited.push(label);
            stack.extend(successors(label));
        }
        let unwind_block = block_order
            .iter()
            .filter(|label| visited.contains(label))
            .flat_map(|label| block_bodies[label].iter().copied())
            .collect::<Vec<&str>>()
            .join("\n");
        assert!(
            unwind_block.contains("call void @pycc_rt_bigint_release"),
            "the #638 unwind block must release the pending tuple element \
             before branching to the exception target; block body:\n{unwind_block}"
        );
    }

    #[test]
    fn a_successfully_completed_tuple_literal_truncates_without_releasing() {
        // `(n + n, n)`: both elements complete evaluation successfully, so
        // the tuple-literal element loop's own `pending_int_releases` entry
        // for the first (owned) element must be truncated away -- never
        // released -- because ownership of that word transferred into the
        // aggregate's own field via `build_insert_value`. Releasing it here
        // instead would double-free the word `Ty::Tuple`'s own D-124
        // slot-death release (or the value's ordinary consumer) later
        // releases again.
        //
        // There is no exception-settable sibling in this fixture, so no
        // `effect_exc_unwind` block is emitted at all (mirroring
        // `a_lone_raising_binop_with_nothing_pending_emits_no_unwind_block`
        // above) -- the only way this test can fail is if a stray
        // unconditional release call for the first element leaked into the
        // normal fallthrough path.
        let owned_sum = MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(int_name("n")),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "tuple_literal_success_path".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                    base: Box::new(MirExpr::TupleLiteral(vec![owned_sum, int_name("n")])),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }))],
            }],
            class_defs: Vec::new(),
        };
        let ir = emitted_ir("bigint_rc_tuple_success_path", &mir);
        assert!(
            !ir.contains("effect_exc_unwind"),
            "no sibling element here can set D-173's pending-exception \
             state, so no unwind block should be emitted; got:\n{ir}"
        );
        assert!(
            !ir.contains("call void @pycc_rt_bigint_release"),
            "the success path must truncate the pending entry for the \
             first (owned) tuple element without ever releasing it; got:\n{ir}"
        );
    }

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
        // Two retains (the assignment's duplicate and the return's), one
        // release (the store's). Every one of them was proved guarded, on
        // its own operand, by `guarded_bigint_refcount_calls`.
        assert_eq!((retains, releases), (2, 1), "got {calls:?}");
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

    /// #146 Part 2 (D-181). `int_cmp` raises `OverflowError` and returns its
    /// `0` sentinel on a bigint operand (Part C of #1038, formerly an abort),
    /// and `Compare` is not an `expression_can_set_exception` checkpoint, so
    /// unlike before Part C these releases *are* reachable with a real heap
    /// word: they run against the freshly owned operands after the raise, and
    /// freeing them there is exactly what keeps the pending exception from
    /// leaking them. A runtime-value assertion therefore cannot pin this
    /// shape, and the static retain/release counts below are the only place it
    /// is observable at all.
    ///
    /// `(n + n) < (n + n)`: each inner `+` has two borrowed `Name` operands
    /// and releases neither, while the comparison itself consumes two
    /// freshly built words and releases both.
    #[test]
    fn an_int_comparison_releases_both_of_its_freshly_built_operands() {
        let sum = || MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(int_name("n")),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "compare".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Bool,
                body: vec![MirStmt::Return(Some(MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::Lt,
                    left: Box::new(sum()),
                    right: Box::new(sum()),
                    ty: Ty::Bool,
                }))],
            }],
            class_defs: Vec::new(),
        };
        let calls = refcount_calls_in("bigint_rc_compare_operands", &mir);
        let retains = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_retain")
            .count();
        let releases = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_release")
            .count();
        assert_eq!((retains, releases), (0, 2), "got {calls:?}");
    }

    /// The same for a `*` result. `int_mul`/`int_floordiv`/`int_floormod`/
    /// `int_pow` all reject a bigint operand exactly like `int_cmp` does,
    /// and all four reach the identical release site in `MirExpr::BinOp`'s
    /// own `Ty::Int` arm, so one of them stands for the group.
    #[test]
    fn a_multiplying_binop_releases_both_of_its_freshly_built_operands() {
        let sum = || MirExpr::BinOp {
            op: pycc_mir::BinOpKind::Add,
            left: Box::new(int_name("n")),
            right: Box::new(int_name("n")),
            ty: Ty::Int,
        };
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "multiply".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Mul,
                    left: Box::new(sum()),
                    right: Box::new(sum()),
                    ty: Ty::Int,
                }))],
            }],
            class_defs: Vec::new(),
        };
        let calls = refcount_calls_in("bigint_rc_mul_operands", &mir);
        let retains = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_retain")
            .count();
        let releases = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_release")
            .count();
        assert_eq!((retains, releases), (0, 2), "got {calls:?}");
    }

    /// #146 Part 2 (D-181): the tuple-egress classification, from both
    /// sides at once. `t[0]` is *borrowed*: under D-182 the tuple field
    /// holds its own ingress reference, and a read hands the same word
    /// back out without transferring it, so the reader owns nothing. The
    /// `+` therefore releases only its literal operand -- releasing `t[0]`
    /// too would retire the tuple's own reference on a read, which is
    /// exactly the #633 direction-A use-after-free D-181 closed.
    ///
    /// The `TupleLiteral` itself is where the two retains come from: `n`
    /// is a borrowed `Name`, so D-182's ingress retain fires once per
    /// element.
    ///
    /// The literal is `2^62` rather than a small one on purpose: only an
    /// out-of-range literal is materialized by a runtime
    /// `pycc_rt_int_from_i64` call, so only that shape produces a word
    /// whose heap-ness is unknown at compile time. An in-range literal is
    /// a constant tagged smallint, `emit_bigint_refcount_call` skips it,
    /// and the release this test is about would not exist to observe.
    #[test]
    fn a_tuple_element_operand_is_borrowed_while_a_literal_operand_is_owned() {
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "read_tuple".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::Assign {
                        target: "t".to_string(),
                        value: MirExpr::TupleLiteral(vec![int_name("n"), int_name("n")]),
                    },
                    MirStmt::Return(Some(MirExpr::BinOp {
                        op: pycc_mir::BinOpKind::Add,
                        left: Box::new(MirExpr::Subscript {
                            base: Box::new(MirExpr::Name {
                                name: "t".to_string(),
                                ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                            }),
                            index: Box::new(MirExpr::IntLiteral(0)),
                        }),
                        right: Box::new(MirExpr::IntLiteral(4_611_686_018_427_387_904)),
                        ty: Ty::Int,
                    })),
                ],
            }],
            class_defs: Vec::new(),
        };
        let calls = refcount_calls_in("bigint_rc_tuple_operand", &mir);
        let retains = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_retain")
            .count();
        let releases = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_release")
            .count();
        assert_eq!((retains, releases), (2, 1), "got {calls:?}");
    }

    /// D-181 decision 12: an in-range `IntLiteral` operand is a *constant*
    /// tagged smallint, so `emit_bigint_refcount_call` must emit nothing at
    /// all for it -- not even a guard LLVM would fold away.
    ///
    /// This pins the early return mechanically. Without it the literal `1`
    /// below still reaches the emitter, whose four guard halves inkwell then
    /// constant-folds, leaving a `bigint_rc_call`/`bigint_rc_cont` pair
    /// behind a branch whose condition is no longer an SSA definition. The
    /// other operand is a `Name`, which is borrowed and never released, so
    /// the literal is the function's only candidate refcount site and the
    /// whole body must therefore be free of guard blocks.
    #[test]
    fn an_in_range_literal_operand_emits_no_refcount_guard_at_all() {
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "borrowed_plus_literal".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Add,
                    left: Box::new(int_name("n")),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                }))],
            }],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new("bigint_rc_const_literal")
            .expect("failed to create scratch dir");
        let obj_path = dir.join("bigint_rc_const_literal.o");
        let mut ir = String::new();
        // D-029: never let `print_to_string`'s `LLVMString` drop; route it
        // through `llvm_string_to_owned` exactly as the sibling tests do.
        let mut observer = |module: &inkwell::module::Module<'_>, _| {
            ir = llvm_string_to_owned(module.print_to_string());
        };
        compile_to_object_with_observer(
            &mir,
            &obj_path,
            &CompileOptions::default(),
            Some(&mut observer),
        )
        .expect("codegen should succeed");
        assert!(
            !ir.contains("bigint_rc_call"),
            "a constant word must not reach the guard emitter, but the IR \
             contains a `bigint_rc_call` block:\n{ir}"
        );
        assert!(
            !ir.contains("bigint_rc_cont"),
            "a constant word must not reach the guard emitter, but the IR \
             contains a `bigint_rc_cont` block:\n{ir}"
        );
    }

    /// The retain side of the same classification (#633 direction A):
    /// binding a tuple element to an `int` local makes that local a second
    /// owner of a word the tuple's supplier still holds, so the bind must
    /// retain. Without this arm, overwriting the local released a reference
    /// it never owned and freed the supplier's object out from under it.
    #[test]
    fn binding_a_tuple_element_to_an_int_local_retains_the_shared_word() {
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "bind_tuple".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::Assign {
                        target: "t".to_string(),
                        value: MirExpr::TupleLiteral(vec![int_name("n"), int_name("n")]),
                    },
                    MirStmt::Assign {
                        target: "y".to_string(),
                        value: MirExpr::Subscript {
                            base: Box::new(MirExpr::Name {
                                name: "t".to_string(),
                                ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                            }),
                            index: Box::new(MirExpr::IntLiteral(0)),
                        },
                    },
                    MirStmt::Return(Some(int_name("y"))),
                ],
            }],
            class_defs: Vec::new(),
        };
        let calls = refcount_calls_in("bigint_rc_tuple_bind", &mir);
        let retains = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_retain")
            .count();
        let releases = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_release")
            .count();
        // D-182's two ingress retains for the `TupleLiteral`'s borrowed
        // `n` elements, plus the tuple bind's retain and the `return`'s,
        // against the `y` slot's own release-before-store.
        assert_eq!((retains, releases), (4, 1), "got {calls:?}");
    }

    /// Counts the refcount calls in a one-statement function whose only
    /// body is `t = TupleLiteral(elements)`, with `params` in scope.
    fn tuple_literal_refcount_counts(
        label: &str,
        params: Vec<(String, Ty)>,
        elements: Vec<MirExpr>,
    ) -> (usize, usize) {
        let mir = MirModule {
            items: vec![MirItem::Function {
                name: "build_tuple".to_string(),
                params,
                return_ty: Ty::None,
                body: vec![MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::TupleLiteral(elements),
                }],
            }],
            class_defs: Vec::new(),
        };
        let calls = refcount_calls_in(label, &mir);
        let retains = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_retain")
            .count();
        let releases = calls
            .iter()
            .filter(|c| c.callee == "pycc_rt_bigint_release")
            .count();
        (retains, releases)
    }

    /// #633 direction B (D-182): `MirExpr::TupleLiteral` takes a reference
    /// for the field it is about to store -- but only for a *borrowed*
    /// element word.
    ///
    /// This is the assertion that discriminates D-182's chosen shape from
    /// the unconditional-retain shape it rejected. An owning element
    /// already arrives holding the single reference the field will keep;
    /// retaining it too would leave rc at 2 with one owner, which a future
    /// `Ty::Tuple` slot-death release under D-124 could never balance.
    ///
    /// Nothing releases a tuple field today, so every case here expects a
    /// release count of zero -- D-182's accepted, deliberate imbalance.
    #[test]
    fn a_tuple_literal_retains_only_its_borrowed_int_elements() {
        // A borrowed `Name` element: exactly one ingress retain. The
        // in-range `IntLiteral` sibling is a *constant* tagged smallint,
        // so `emit_bigint_refcount_call` emits nothing at all for it.
        assert_eq!(
            tuple_literal_refcount_counts(
                "bigint_rc_tuple_ingress_borrowed",
                vec![("n".to_string(), Ty::Int)],
                vec![int_name("n"), MirExpr::IntLiteral(1)],
            ),
            (1, 0)
        );

        // An *owning* element: no ingress retain. The literal is `2^62`
        // rather than a small one on purpose -- only an out-of-range
        // literal is materialized by a runtime `pycc_rt_int_from_i64`
        // call, so only that shape produces a non-constant word that
        // `emit_bigint_refcount_call` would not have skipped anyway. An
        // in-range literal would prove nothing about the classification.
        assert_eq!(
            tuple_literal_refcount_counts(
                "bigint_rc_tuple_ingress_owning",
                Vec::new(),
                vec![
                    MirExpr::IntLiteral(4_611_686_018_427_387_904),
                    MirExpr::IntLiteral(1),
                ],
            ),
            (0, 0)
        );

        // `float`/`bool` elements are not `int` words at all: the
        // `Scalar::Int` test in `retain_if_int_duplicate` filters them out
        // before the source-expression classification is even consulted.
        assert_eq!(
            tuple_literal_refcount_counts(
                "bigint_rc_tuple_ingress_non_int",
                vec![("f".to_string(), Ty::Float), ("b".to_string(), Ty::Bool)],
                vec![
                    MirExpr::Name {
                        name: "f".to_string(),
                        ty: Ty::Float,
                    },
                    MirExpr::Name {
                        name: "b".to_string(),
                        ty: Ty::Bool,
                    },
                ],
            ),
            (0, 0)
        );
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
        // and `step`).
        //
        // Classifying `BufferLen` as a borrowed duplicate instead makes
        // `int_temporary_word` answer `None` for `stop`, which drops
        // exactly one release from this total and leaks the count's birth
        // reference; the mirror error, classifying an operand the loop does
        // not own as owning, frees a live word. Neither is visible in the
        // program's output or in its RSS here -- a buffer length is always
        // an inline D-141 smallint, so the release is a runtime no-op -- so
        // this count is the only place the classification is observable.
        assert_eq!((retains, releases), (4, 8), "got {calls:?}");
    }
}
