//! The element ops of a set literal or `.add(...)` (#1343, Part 1 of
//! #1336) and of a set comprehension's element (#1344, Part 2), resolved
//! from the class table codegen does not see.
//!
//! `pycc_types::set_element` admitted the insertion only when both verdicts
//! below were compilable, from the same resolvers, so every `expect` here is
//! an internal invariant.

use crate::receiver_exact::exact_callee;
use crate::{HirClassDef, SetElementOps, SetEqOp, SetHashOp, lookup};
use pycc_hir::{InstanceEq, InstanceHashLowering, Ty, resolve_instance_eq, resolve_instance_hash};
use std::collections::HashMap;

/// The ops for a set whose element type is `elem_ty`: `None` for `int`,
/// otherwise the instance class's hash and equality.
pub(crate) fn lower_set_element_ops(
    elem_ty: &Ty,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> Option<SetElementOps> {
    let Ty::Instance(class) = elem_ty else {
        return None;
    };
    let hash = match resolve_instance_hash(class, classes)
        .lowerable()
        .expect("pycc_types admits only a hashable set element")
    {
        InstanceHashLowering::Identity => SetHashOp::Identity,
        InstanceHashLowering::Method(mangled) => {
            let mangled = receiver_exact(class, mangled, scopes, classes);
            let ret = lookup(scopes, &format!("$fn:{mangled}"));
            SetHashOp::Method {
                callee: mangled,
                ret,
            }
        }
    };
    let eq = match resolve_instance_eq(class, classes) {
        InstanceEq::Identity => SetEqOp::Identity,
        InstanceEq::Method(callee) => SetEqOp::Method {
            callee: receiver_exact(class, callee, scopes, classes),
        },
        InstanceEq::Unsupported(refusal) => {
            panic!("pycc_types admits only a compiled set element `__eq__`: {refusal:?}")
        }
    };
    Some(SetElementOps { hash, eq })
}

/// Routes an inherited dunder to the element class's receiver-exact copy
/// (D-254), as `hash(instance)` does, so a leaf override of a method the
/// dunder calls is honoured inside the set.
fn receiver_exact(
    class: &str,
    mangled: String,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> String {
    let owner = mangled
        .split_once('.')
        .map_or(class, |(owner, _)| owner)
        .to_string();
    exact_callee(class, &owner, mangled, scopes, classes)
}
