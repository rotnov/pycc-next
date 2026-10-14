//! The release of a *native* operand a foreign packer borrows: the fresh
//! `str`, list or bigint `int` handed to a call on a CPython object
//! (`json.dumps(str(i))`, `o.get(k + "x")`, `f(x=[i])`) or used as an
//! object's subscript key (`o[str(i)]`).
//!
//! Every `pycc_ext_obj_pack_*` helper borrows its pycc-side value and
//! builds a CPython object of its own (`foreign_pack`): a `str` is copied
//! into a `PyUnicode`, a list into a CPython `list`, an `int` into a
//! `PyLong`. A fresh native operand therefore still belongs to the
//! compiled code once it is packed, and nothing else ever releases it:
//! before this module the argument of every such call leaked once per
//! evaluation (a 400,000-iteration `json.dumps(str(i))` loop grew the
//! peak resident set by ~20 MB where CPython stays flat).
//!
//! [`hold`] puts the operand on the exception state's pending stacks --
//! `pending_heap_releases` for a counted value, `pending_int_releases` for
//! a bigint word -- so a later sibling's or the operation's own failure
//! edge releases it, and [`NativeHeld::release`] retires those entries and
//! releases the operand on the fallthrough, after the operation. A
//! borrowed read (a name, an attribute) owns nothing and is never
//! released, by the same `heap_rc`/`bigint_rc` classification every other
//! consumer uses. A CPython `object` operand is `object_release`'s, so it
//! is skipped here and no operand is released twice.

use super::*;
use crate::bigint_rc::{push_pending_int_release_if_scalar_temporary, remove_pending_int_release};
use crate::heap_rc::HeapKind;

/// One native operand held from its evaluation until the operation that
/// packs it is done.
#[must_use = "a held native operand must be released, or the pending stacks drift"]
pub(super) struct NativeHeld<'a, 'ctx> {
    source: &'a MirExpr,
    scalar: Scalar<'ctx>,
    heap: Option<(HeapKind, PointerValue<'ctx>)>,
    int: Option<IntValue<'ctx>>,
}

/// Holds `scalar`, the already-evaluated operand `expr`, when it is a fresh
/// native counted value or bigint word (see the module doc).
pub(super) fn hold<'a, 'ctx>(
    rt: &RtFns<'ctx>,
    expr: &'a MirExpr,
    scalar: Scalar<'ctx>,
) -> NativeHeld<'a, 'ctx> {
    let source = crate::object_release::operand_source(expr);
    let (heap, int) = if matches!(scalar, Scalar::Object(_)) {
        (None, None)
    } else {
        (
            crate::heap_rc::hold_temporary_entry(rt, source, &scalar),
            push_pending_int_release_if_scalar_temporary(rt, source, &scalar),
        )
    };
    NativeHeld {
        source,
        scalar,
        heap,
        int,
    }
}

impl<'ctx> NativeHeld<'_, 'ctx> {
    /// Retires the hold and releases the operand, now that the operation
    /// that packed it is done.
    pub(super) fn release(
        self,
        context: &'ctx Context,
        builder: &inkwell::builder::Builder<'ctx>,
        rt: &RtFns<'ctx>,
    ) {
        if let Some(entry) = self.heap {
            crate::heap_rc::unhold(rt, entry);
        }
        remove_pending_int_release(rt, self.int);
        if !matches!(self.scalar, Scalar::Object(_)) {
            release_scalar_if_temporary(context, builder, rt, self.source, &self.scalar);
        }
    }
}
