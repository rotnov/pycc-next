//! MIR's reading of a [`HirExpr::ReceiverDispatchedCall`] (issue #1188).
//!
//! `pycc_types` has already picked the reading and accepted the program;
//! this module picks the same one from the same facts, so the call is
//! lowered as the call that was checked:
//!
//! - a [`ContainerFallback::Refused`] call was accepted only through its
//!   method reading, since the container reading's own diagnostic would
//!   have rejected it;
//! - an [`ContainerFallback::Admitted`] call has a bare-name receiver or
//!   (#1263) an attribute-read receiver. A class name takes the method
//!   reading, as it does in the `MethodCall` arm; any other name's recorded
//!   type, or the attribute read's lowered type, decides through
//!   [`receiver_takes_method_path`], the rule `pycc_types` also uses.

use super::lower_expr;
use crate::{HirClassDef, MirExpr, lookup, narrowed_ty};
use pycc_hir::{ContainerFallback, HirExpr, Ty, receiver_takes_method_path};
use std::collections::HashMap;

/// Whether `name` is a class no value binding shadows, so that
/// `name.m(...)` is a `ClassName.static_method(...)` or
/// `ClassName.class_method(...)` call (#436). Shared by the `MethodCall` arm
/// and [`lower_receiver_dispatched_call`] so both route such a receiver the
/// same way, and before any `lookup`, which panics on a class name.
pub(super) fn is_unshadowed_class_name(
    name: &str,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> bool {
    !scopes.iter().any(|scope| scope.contains_key(name)) && classes.contains_key(name)
}

/// Lowers `call` (always a `MethodCall`) under the reading `pycc_types`
/// chose for it.
pub(super) fn lower_receiver_dispatched_call(
    call: &HirExpr,
    container: &ContainerFallback,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    if let ContainerFallback::Admitted = container {
        let (receiver, _) = call
            .method_receiver()
            .expect("a receiver-dispatched call is always a method call");
        let takes_method_path = match receiver {
            HirExpr::Name(name) => {
                is_unshadowed_class_name(name, scopes, classes)
                    || receiver_takes_method_path(
                        &narrowed_ty(scopes, name).unwrap_or_else(|| lookup(scopes, name)),
                    )
            }
            // #1263: an admitted attribute receiver (`self.xs.append(v)`)
            // is typed by its own lowered read -- a slot's declared type or
            // a `@property` getter's return type -- the same type
            // `pycc_types` inferred for it. Lowering it here is pure.
            attr => {
                receiver_takes_method_path(&lower_expr(attr, scopes, classes, current_class).ty())
            }
        };
        if !takes_method_path {
            let form = call
                .container_form()
                .expect("an admitted container reading has a container form");
            return lower_expr(&form, scopes, classes, current_class);
        }
    }
    lower_expr(call, scopes, classes, current_class)
}
