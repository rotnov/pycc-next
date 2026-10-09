//! `str` reference-count ownership helpers (D-060): the compile-time
//! classification of which `str`-producing expressions yield a *duplicate*
//! (borrowed) reference, the incref that turns such a read into an owned
//! one, and the release of a `str` slot's old value before a store.
//!
//! Narrow carve-out of `lib.rs` under AGENTS.md's decomposability rule
//! (#545), mirroring `bigint_rc.rs`: exactly the cluster #1298 touches.

use super::*;

/// Whether evaluating `expr` produces a *duplicate* (borrowed) reference to
/// an already-owned `str` rather than a fresh object owning exactly one
/// reference from its own construction. This doc comment is the canonical
/// list of borrowed `str` reads; every other description of it
/// cross-references this function. The classification is purely
/// syntactic. The borrowed reads are exactly the nodes the `matches!` below
/// names: a bare `str`-typed `Name`, a `str`-typed `AttrGet`, and
/// `ExceptionMessage`. Every other str-producing expression
/// (`StringLiteral`, string concatenation, an f-string, a `Call`'s return
/// value, a `BoolOp`, ...) freshly constructs its result and already owns
/// exactly one reference (D-060, Task 7).
///
/// Gated on `ty: Ty::Str`, not just the bare-`Name` shape (Task 5, D-089).
/// The gate was originally added because `emit_expr`'s `Name` arm carried a
/// `Ty::List(_)`-typed read in `Scalar::Str` too, which made a bare
/// `list[T]`-typed `Name` indistinguishable from a `str`-typed one at the
/// `Scalar` level -- and `incref_if_str_duplicate` below dispatches on the
/// `Scalar` variant alone, so without the gate it would have called
/// `pycc_rt_str_incref` on a list pointer.
///
/// Task 11a (D-107) removed that reuse: a list read is now `Scalar::List`,
/// so the two are no longer confusable and this gate is no longer what
/// prevents the spurious incref. It is kept because it is independently the
/// correct contract for this function -- it answers "is this a duplicate
/// reference to an already-owned *`str`*", and a non-`str` `Name` is not
/// one, whatever `Scalar` variant it happens to produce. Behavior-identical
/// either way for every reachable case: `incref_if_str_duplicate` only ever
/// consults this function *after* confirming `scalar` is `Scalar::Str`.
///
/// `MirExpr::AttrGet { ty: Ty::Str, .. }` (D-154, Part 1 of #375) is a
/// duplicate reference for exactly the same reason a bare `Name` is: the
/// instance's own slot keeps its copy of the pointer after this read
/// returns one too, so both `to_str`'s pass-through and every ordinary
/// store site (`Assign`, a call argument, a dict key/value, ...) would
/// otherwise treat this read's result as freshly-owned and eventually
/// decref it once too many, underflowing the refcount and freeing the
/// `PyStrObj` while the instance's own slot still points at it -- a
/// reliably reproducible use-after-free caught in review, not merely a
/// theoretical gap (D-154 Part 1's own post-merge finding).
///
/// A `MirExpr::BoolOp` (#1211) or `MirExpr::IfExp` (#1395) is owning here,
/// like every node this `matches!` does not name: each of its value arms
/// increfs a duplicate operand inside that arm, so its `str` result is
/// always a fresh reference.
///
/// `MirExpr::ExceptionMessage` (#1298) -- `print(e)` and `f"{e}"` on a caught
/// exception binding -- is a duplicate reference for the same field-load
/// reason as `AttrGet`: `pycc_rt_exception_message` returns the exception's
/// own `message` pointer borrowed and unretained, and the exception keeps
/// owning it after the read. Classifying it as owning made every rendering
/// release a reference it never took, so the second `print(e)` read freed
/// memory.
pub(super) fn str_value_is_a_duplicate_reference(expr: &MirExpr) -> bool {
    // #1346: a sequence yields its `value`'s pointer unchanged, so it is a
    // duplicate exactly when `value` is.
    if let MirExpr::Sequence { value, .. } = expr {
        return str_value_is_a_duplicate_reference(value);
    }
    matches!(
        expr,
        MirExpr::Name {
            ty: pycc_mir::Ty::Str,
            ..
        } | MirExpr::AttrGet {
            ty: pycc_mir::Ty::Str,
            ..
        } | MirExpr::ExceptionMessage(_)
    )
}

/// Increments a `str` scalar's refcount when `source_expr` is a borrowed
/// read (see `str_value_is_a_duplicate_reference`) -- binding a
/// second owning reference to the same `PyStrObj` without this would leave
/// the original binding's own eventual decref underflowing the refcount
/// (D-060, Task 7). A no-op for every non-`Str` scalar.
pub(super) fn incref_if_str_duplicate<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source_expr: &MirExpr,
    scalar: Scalar<'ctx>,
) -> Scalar<'ctx> {
    if let Scalar::Str(ptr) = scalar {
        if str_value_is_a_duplicate_reference(source_expr) {
            builder
                .build_call(rt.str_incref, &[ptr.into()], "str_incref")
                .expect("build_call should not fail for a well-formed incref");
        }
        Scalar::Str(ptr)
    } else {
        scalar
    }
}

/// Only meaningful for `Ty::Str` targets: loads the target's predeclared
/// slot and decrefs its current value before the new value overwrites it.
/// String slots start as null, whose runtime decref is a no-op, so the same
/// path is correct for both first assignment and reassignment and prevents
/// loop-body-first bindings from leaking earlier iteration values (D-074).
pub(super) fn decref_str_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
) {
    let slot = &locals[target];
    if slot.ty != pycc_mir::Ty::Str {
        panic!(
            "pycc_codegen: internal error: string assignment target `{target}` has a non-string storage slot"
        );
    }
    let old = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            slot.ptr,
            "old_str",
        )
        .expect("build_load should not fail for this function's own alloca")
        .into_pointer_value();
    builder
        .build_call(rt.str_decref, &[old.into()], "str_decref_old")
        .expect("build_call should not fail for a well-formed decref");
}
