//! `str` reference-count ownership helpers (D-060): the compile-time
//! classification of which `str`-producing expressions yield a *duplicate*
//! (borrowed) reference, the incref that turns such a read into an owned
//! one, and the release of a `str` slot's old value before a store.
//!
//! Narrow carve-out of `lib.rs` under AGENTS.md's decomposability rule
//! (#545), mirroring `bigint_rc.rs`: exactly the cluster #1298 touches.

use super::*;

/// Whether evaluating `expr` produces a *duplicate* (borrowed) reference to
/// an already-owned `str` rather than a fresh one: a `str`-typed
/// `heap_rc::is_borrowed_read` (a name, an attribute or list element of a
/// borrowed base, a caught exception's message). Everything else -- a
/// literal, a concatenation, an f-string, a call's result, a `BoolOp` or
/// `IfExp` (whose arms take their own reference) -- owns one reference.
pub(super) fn str_value_is_a_duplicate_reference(expr: &MirExpr) -> bool {
    expr.ty() == pycc_mir::Ty::Str && crate::heap_rc::is_borrowed_read(expr)
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
