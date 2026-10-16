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

/// Releases a parameter slot's word on the way out of its function (#625)
/// behind the one-test guard `(word & 0b11) == 0`: a parameter slot never
/// holds the empty word `0` (it is stored from an argument, and a `return`
/// that moves it out leaves the smallint `0`, see `return_stmt`), so the
/// non-zero half of [`emit_bigint_refcount_call`]'s guard is dropped from
/// the hottest exit of a small recursive function.
pub(super) fn emit_param_release<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) {
    let i64_type = context.i64_type();
    let low_tag = builder
        .build_and(word, i64_type.const_int(0b11, false), "param_low_tag")
        .expect("build_and should not fail for two i64 values");
    let is_heap = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            low_tag,
            i64_type.const_zero(),
            "param_is_heap",
        )
        .expect("build_int_compare should not fail for two i64 values");
    let function = builder
        .get_insert_block()
        .expect("builder is always positioned inside some block while emitting")
        .get_parent()
        .expect("the block the builder is positioned in always belongs to a function");
    let call_block = context.append_basic_block(function, "bigint_param_call");
    let continue_block = context.append_basic_block(function, "bigint_param_cont");
    builder
        .build_conditional_branch(is_heap, call_block, continue_block)
        .expect("build_conditional_branch should not fail for a well-formed i1");
    builder.position_at_end(call_block);
    builder
        .build_call(rt.bigint_release, &[word.into()], "bigint_param_release")
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
            MirExpr::AttrGet {
                ty: pycc_mir::Ty::Int,
                base,
                ..
            } => crate::heap_rc::is_borrowed_read(base),
            MirExpr::Name {
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
        // An attribute of a fresh base (`f().x`) is retained by the read
        // itself before the base is released (`heap_rc`), so it is owned.
        MirExpr::AttrGet {
            ty: pycc_mir::Ty::Int,
            base,
            ..
        } => crate::heap_rc::is_borrowed_read(base),
        MirExpr::Name {
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
        | MirExpr::RtCall(_)
        | MirExpr::Nd(_)
        | MirExpr::BinOp { .. }
        | MirExpr::Compare { .. }
        | MirExpr::FString(_)
        | MirExpr::ListLiteral(_)
        | MirExpr::EmptyList(_)
        | MirExpr::EmptySet(_)
        | MirExpr::ListAppend { .. }
        | MirExpr::ListInsert { .. }
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
        // A value a generator handed over (`next(g)`) is owned.
        | MirExpr::GenNext { .. }
        | MirExpr::GenToList { .. }
        | MirExpr::GenAnyAll { .. }
        | MirExpr::GenYield { .. }
        | MirExpr::GenYieldFrom { .. }
        | MirExpr::GenSend { .. }
        | MirExpr::GenThrow { .. }
        | MirExpr::GenClose { .. }
        | MirExpr::StopIterationValue { .. }
        // A function value's call returns an owned value, like `Call`'s.
        | MirExpr::MakeClosure { .. }
        | MirExpr::CallClosure { .. }
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
        // A converted object's word, encoded by the same shim helper.
        | MirExpr::ObjectCoerce(_, _)
        // A new tuple object, never an `int`.
        | MirExpr::TupleRebox(_, _)
        // An object read as an index: the shim encodes an inline word (it
        // clamps or refuses anything beyond the inline range,
        // `pycc_ext_obj_coerce` kinds 4..7), so owning it releases nothing.
        | MirExpr::ObjectIndex(..)
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
        // in the doc comment above: `pycc_ext_obj_len` writes
        // `pycc_rt_int_from_i64` of a `Py_ssize_t` size, which is always below
        // 2**62 and so an inline smallint, and the release this
        // classification emits is an unconditional runtime no-op.
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
        // Never `Ty::Int`: a list, an instance, `None` or a `bool`.
        | MirExpr::InstanceOrder(_)
        | MirExpr::InstanceSetContains { .. }
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
        | MirExpr::ObjList { .. }
        // Non-strict mode (#3): an object operator's `.ty()` is `Ty::Object`.
        | MirExpr::ObjBinOp { .. }
        | MirExpr::ObjDisplay { .. }
        | MirExpr::ObjUnaryOp { .. } => false,
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

/// Removes the word [`push_pending_int_release_if_scalar_temporary`]
/// pushed wherever it sits in the stack, for a consumer whose holds outlive
/// later pushes it does not pop itself (`native_operand`). A `None`
/// `pushed` value is a no-op, as for [`pop_pending_int_release`].
pub(super) fn remove_pending_int_release<'ctx>(rt: &RtFns<'ctx>, pushed: Option<IntValue<'ctx>>) {
    if let Some(expected) = pushed {
        let mut stack = rt.exceptions.pending_int_releases.borrow_mut();
        let position = stack.iter().rposition(|word| *word == expected).expect(
            "pycc_codegen: internal error: a held int temporary is missing from \
             pending_int_releases",
        );
        stack.remove(position);
    }
}

#[cfg(test)]
mod tests;
