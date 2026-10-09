//! `None` returned from a function whose declared return type is the
//! opaque CPython `object` (#1387, D-258's return-position amendment).
//!
//! `pycc_types` admits exactly two spellings into an `object` return slot
//! that would otherwise be `T0022`: a bare `return` and `return None` (the
//! literal). Both must hand the caller CPython's `None`, but the MIR carries
//! them as `MirStmt::Return(None)` and `MirStmt::Return(Some(NoneLiteral))`,
//! and neither generic path produces a `PyObject *`: the bare form emits the
//! type's zero default (a null pointer, which the export wrapper reports as
//! `SystemError`), and `NoneLiteral` emits the native `Optional` carrier.
//!
//! [`object_return_value`] folds the bare form into the literal one, and
//! since Part 2 of #1387 the `return` path boxes every native returned value
//! -- the `None` literal included -- through `object_box`, which turns the
//! literal into [`crate::foreign_pack::none_pointer`]'s borrowed `Py_None`.
//! Borrowed is the leak-only ownership model's convention for a compiled
//! body (`docs/RUNTIME.md`): the export wrapper's `pycc_ext_pack_object`
//! takes the new reference the host receives, exactly as it does for a
//! borrowed object parameter returned by name.

use super::*;

/// The value a `return` statement hands back: `value` itself, except that a
/// bare `return` in an `object`-returning function is `return None`, spelled
/// as `bare_none` (a caller-owned `MirExpr::NoneLiteral`).
pub(crate) fn object_return_value<'a>(
    expected_return_ty: &pycc_mir::Ty,
    value: &'a Option<MirExpr>,
    bare_none: &'a MirExpr,
) -> Option<&'a MirExpr> {
    match value {
        None if *expected_return_ty == pycc_mir::Ty::Object => Some(bare_none),
        other => other.as_ref(),
    }
}
