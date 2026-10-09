//! The type checker's reading of a [`HirExpr::ReceiverDispatchedCall`]
//! (issue #1188).
//!
//! HIR lowering keeps both readings of `recv.<append|pop|get|add>(...)` in a
//! module that can see a user class defining that method name. This module
//! picks one, in the order `docs/TYPE_SYSTEM.md` states:
//!
//! 1. a receiver that names a class with a static or class method of that
//!    name takes the method reading, exactly as the `MethodCall` arm would;
//! 2. otherwise the receiver's static type decides, through the one rule
//!    `pycc_mir` also uses, [`receiver_takes_method_path`]: a user class, a
//!    protocol or (issue #1095) a CPython object takes the method reading;
//! 3. anything else -- a container, a scalar, or a receiver whose type
//!    cannot be inferred -- takes the container reading, whose diagnostics
//!    are the ones such a call has always produced.

use super::{class_name_dispatch, infer_expr_in};
use crate::{BindingState, Environment, class, lookup_bound_name};
use pycc_diag::Diagnostic;
use pycc_hir::{ContainerFallback, HirExpr, Ty, receiver_takes_method_path};

/// The class a `base.method(...)` call names directly, when `base` is a bare
/// class name the checker resolves as the class (not a shadowing value) and
/// that class has a static or class method called `method` (#436).
///
/// Shared by the `MethodCall` arm and [`infer_receiver_dispatched_call`] so
/// the two cannot disagree about which calls go to the class's own table.
pub(super) fn static_or_class_method_receiver<'a>(
    env: &Environment,
    local_names: &[&str],
    base: &'a HirExpr,
    method: &str,
) -> Result<Option<&'a str>, Diagnostic> {
    if let HirExpr::Name(class_name) = base
        && class_name_dispatch(env, local_names, class_name)?
        && class::has_static_or_class_method(env, class_name, method)
    {
        return Ok(Some(class_name.as_str()));
    }
    Ok(None)
}

/// Infers `call` (always a `MethodCall`) under the reading its receiver
/// selects; `container` is what HIR's container lowering made of the same
/// call.
pub(super) fn infer_receiver_dispatched_call(
    env: &Environment,
    local_names: &[&str],
    call: &HirExpr,
    container: &ContainerFallback,
) -> Result<Ty, Diagnostic> {
    let (base, method) = call
        .method_receiver()
        .expect("a receiver-dispatched call always wraps a MethodCall");
    if static_or_class_method_receiver(env, local_names, base, method)?.is_some() {
        return infer_expr_in(env, local_names, call);
    }
    let receiver_ty = match base {
        // Issue #1095: `lookup_bound_name` refuses a read of a name bound to
        // a CPython object (`reject_object_read`), so that receiver is
        // recognized before it; its call is the foreign method call.
        HirExpr::Name(name)
            if matches!(
                env.binding_state(name),
                Some(BindingState::Definitely(Ty::Object))
            ) =>
        {
            Ok(Ty::Object)
        }
        HirExpr::Name(name) => lookup_bound_name(env, local_names, name),
        other => infer_expr_in(env, local_names, other),
    };
    if receiver_ty.is_ok_and(|ty| receiver_takes_method_path(&ty)) {
        return infer_expr_in(env, local_names, call);
    }
    match container {
        ContainerFallback::Admitted => {
            let form = call
                .container_form()
                .expect("an admitted container reading has a container form");
            infer_expr_in(env, local_names, &form)
        }
        ContainerFallback::Refused(diagnostic) => Err((**diagnostic).clone()),
    }
}
