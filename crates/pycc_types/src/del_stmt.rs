//! Type checking for the `del` statement (#1244, Part 1 of #1216).
//!
//! `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the canonical
//! statement of the rule. `del name` is static-only: it lowers to nothing at
//! runtime (D-124's leak-only model has no reference to release), so the
//! whole of its CPython semantics -- a later read raises `NameError` /
//! `UnboundLocalError` -- is enforced here, by demoting the name's binding
//! state to `Maybe` so that any later read reports `T0041`. The re-entrant
//! points that can observe a deletion out of source order (loop bodies,
//! handlers, `finally`) are covered by `narrow::apply_delete_prescan`.

use crate::env::{BindingState, Environment};
use crate::{possibly_unbound, unbound_local};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// Checks `del name` against `env` (module or function scope) and, when it
/// is accepted, leaves `name` maybe-bound.
///
/// The accepted shape is an allowlist: `name` must be a definitely bound
/// value binding that is not a function or class name (those resolve
/// through their own tables, not `bindings`, so demoting the binding would
/// leave `del f; f()` callable), not `Final`, not an owned buffer, and not a
/// CPython object (whose release timing could run a foreign finalizer).
/// Everything else is refused, so no deletion can be silently ignored.
pub(crate) fn check_delete(env: &mut Environment, name: &str) -> Result<(), Diagnostic> {
    if env.def_rebound.contains(name) || env.lookup_function(name).is_some() {
        return Err(refused(name, "it names a function"));
    }
    if env.lookup_class(name).is_some() {
        return Err(refused(name, "it names a class"));
    }
    // Read the raw binding state, never the narrowed type: storing a
    // narrowed `int` back as `Maybe(int)` would break D-040's sticky
    // representation for an `int | None` binding.
    let ty = match env.bindings.get(name) {
        Some(BindingState::Definitely(ty)) => ty.clone(),
        Some(BindingState::Maybe(_)) => return Err(possibly_unbound(name)),
        None => return Err(unbound_local(name)),
    };
    if env.finals.contains(name) {
        return Err(refused(name, "it is declared `Final`"));
    }
    if env.owned_buffers.contains(name) {
        return Err(refused(
            name,
            "it owns a buffer the function releases on return",
        ));
    }
    if ty == Ty::Object {
        return Err(refused(
            name,
            "it holds a CPython object, whose release could run a foreign finalizer",
        ));
    }
    env.bindings
        .insert(name.to_string(), BindingState::Maybe(ty));
    env.narrowed.remove(name);
    Ok(())
}

fn refused(name: &str, reason: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!("a `del` of `{name}` is not supported yet: {reason}"),
        Span::new(0, 0),
    )
}
