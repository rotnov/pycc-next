//! `hash(instance)` lowering to [`MirExpr::InstanceHash`] (#1335, Part 1 of
//! #1332).
//!
//! `pycc_types` admitted the call only when
//! [`pycc_hir::resolve_instance_hash`] gave a lowerable verdict for the
//! instance's class, from the same class table this reads, so the verdict
//! here is always [`InstanceHashLowering::Identity`] or
//! [`InstanceHashLowering::Method`].

use crate::receiver_exact::exact_callee;
use crate::{HirClassDef, InstanceHashVia, MirExpr, lookup};
use pycc_hir::{InstanceHashLowering, Ty, resolve_instance_hash};
use std::collections::HashMap;

/// Lowers `hash(instance)`, where `instance` is the already-lowered argument
/// and `class` its instance type's class.
///
/// The identity hash keeps the instance as the operand. A user `__hash__`
/// becomes an ordinary call to the mangled method with the instance as
/// `self`, so the call gets the exception guard and the ownership handling
/// every other call gets; codegen then reduces its result like CPython's
/// `slot_tp_hash`.
pub(super) fn lower_instance_hash(
    instance: MirExpr,
    class: &str,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    let lowering = resolve_instance_hash(class, classes)
        .lowerable()
        .expect("pycc_types admits only a lowerable instance hash");
    match lowering {
        InstanceHashLowering::Identity => MirExpr::InstanceHash {
            operand: Box::new(instance),
            via: InstanceHashVia::Identity,
        },
        InstanceHashLowering::Method(mangled) => {
            // #1337 (D-254): an inherited `__hash__` runs its receiver-exact
            // copy when one exists. Only the callee is routed; the verdict
            // above is #1335's, unchanged.
            let owner = mangled
                .split_once('.')
                .map_or(class, |(owner, _)| owner)
                .to_string();
            let mangled = exact_callee(class, &owner, mangled, scopes, classes);
            let ty = lookup(scopes, &format!("$fn:{mangled}"));
            MirExpr::InstanceHash {
                operand: Box::new(MirExpr::Call {
                    callee: mangled,
                    args: vec![instance],
                    ty,
                }),
                via: InstanceHashVia::Method,
            }
        }
    }
}
