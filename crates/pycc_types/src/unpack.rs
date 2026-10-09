//! Typing of a tuple-unpacking assignment's value, `HirExpr::Unpack`
//! (Part 1 of #891).
//!
//! `pycc_hir` lowers `t1, ..., tn = value` into
//! `tmp = Unpack(value, n)` followed by `ti = tmp[i - 1]`; the canonical
//! rule is `docs/TYPE_SYSTEM.md`'s "Tuple-unpacking assignment" section.
//! This module types the first statement's value:
//!
//! - a native `tuple[...]` of exactly `n` elements is itself, so each
//!   `tmp[i]` read is the ordinary in-range literal tuple index (D-116);
//! - a native tuple of any other length is `T0055`. CPython raises
//!   `ValueError` at run time, but the count is known statically, the
//!   same reasoning that makes an out-of-range literal tuple index `T0040`
//!   rather than a run-time `IndexError`, and the same check mypy and
//!   pyright report;
//! - a CPython object (`Ty::Object`) is an object: the run-time unpack
//!   yields a `tuple` of exactly `n` items or raises CPython's own
//!   `ValueError`/`TypeError`, and each `tmp[i]` read is an object
//!   subscript load;
//! - every other type -- a `list`, a `str`, a `dict`, an instance -- is
//!   refused with `C0001` until a later part of #891 admits it.

use crate::Environment;
use crate::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// Types `Unpack(value, arity)`.
pub(crate) fn infer_unpack(
    env: &Environment,
    local_names: &[&str],
    value: &HirExpr,
    arity: usize,
) -> Result<Ty, Diagnostic> {
    match infer_expr_in(env, local_names, value)? {
        Ty::Tuple(elems) if elems.len() == arity => Ok(Ty::Tuple(elems)),
        Ty::Tuple(elems) => Err(arity_mismatch(elems.len(), arity)),
        Ty::Object => Ok(Ty::Object),
        other => Err(Diagnostic::error(
            "C0001",
            format!(
                "unpacking a `{}` value into names is not supported yet",
                other.name()
            ),
            Span::new(0, 0),
        )
        .with_help(
            "only a fixed-length `tuple[...]` or, in an `--ext` build, a CPython object \
             can be unpacked so far; index the value element by element instead",
        )),
    }
}

/// `T0055`: CPython's own `ValueError` wording for a tuple of `found`
/// values unpacked into `expected` targets.
fn arity_mismatch(found: usize, expected: usize) -> Diagnostic {
    let message = if found > expected {
        format!("too many values to unpack (expected {expected}, got {found})")
    } else {
        format!("not enough values to unpack (expected {expected}, got {found})")
    };
    Diagnostic::error("T0055", message, Span::new(0, 0)).with_help(format!(
        "the tuple has {found} element(s); write exactly {found} target name(s)"
    ))
}

#[cfg(test)]
mod tests;
