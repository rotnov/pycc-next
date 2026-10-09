//! Receiver evaluation for an attribute that does not consume its receiver
//! (#1346).
//!
//! A `@staticmethod` or a `staticmethod(<foreign callable>)` class attribute
//! reached through an instance, `recv.attr` / `recv.attr(args)`, lowers to
//! an expression that has no use for `recv`'s value. CPython still evaluates
//! `recv` first, so `make().exists(p)` runs `make()` -- including its
//! `__init__` output and any exception it raises -- before the call's
//! arguments and the call itself.

use super::MirExpr;

/// `value`, preceded by `base`'s evaluation for its effects.
///
/// A plain `Name` read has no effect to preserve, so `value` comes back
/// unchanged and the lowering stays exactly what it was for a named
/// receiver; any other receiver is wrapped in [`MirExpr::Sequence`].
pub(super) fn sequence_after(base: MirExpr, value: MirExpr) -> MirExpr {
    if matches!(base, MirExpr::Name { .. }) {
        return value;
    }
    MirExpr::Sequence {
        discard: Box::new(base),
        value: Box::new(value),
    }
}
