//! MIR's lowering of a regular instance-method call: the `MethodCall`
//! arm's tail once the receiver is a user class and no foreign static
//! attribute, `@staticmethod` or `@classmethod` matched.
//!
//! The method is found by walking the receiver class's MRO (#432) and
//! called through the receiver-exact copy when one exists (#1337, D-254).
//! Since Part 1 of #1191 (#1438) the call's omitted trailing defaults are
//! appended after its supplied arguments.

use super::lower_expr;
use crate::receiver_exact::exact_callee;
use crate::{HirClassDef, MirExpr, lookup, mro_class_def};
use pycc_hir::{HirExpr, Ty};
use std::collections::HashMap;

/// Lowers `base.method(args)`, where `base` is the already-lowered receiver
/// of the user class `class_def`.
pub(super) fn lower_regular_method_call(
    base: MirExpr,
    class_def: &HirClassDef,
    method: &str,
    args: &[HirExpr],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    // #432: walk the MRO to find the method's mangled name.
    let (owner, mangled) = class_def
        .mro
        .iter()
        .find_map(|mro_class| {
            let mro_def = mro_class_def(mro_class, classes);
            mro_def
                .methods
                .iter()
                .find(|(name, _)| name == method)
                .map(|(_, mangled)| (mro_class, mangled.clone()))
        })
        .unwrap_or_else(|| {
            panic!(
                "pycc_mir: internal error: method `{method}` not declared on class `{}` or \
             any base in its MRO -- pycc_types::check should have rejected this HIR \
             before it reached pycc_mir",
                class_def.name
            )
        });
    // Part 1 of #1191 (#1438): the defaults of the trailing
    // parameters the call omits, looked up under the owner's own
    // mangled name -- the key `method_defaults` is stored under --
    // before the receiver-exact rename below. `pycc_types` accepted
    // a short call only when this fill covers it.
    let omitted: Vec<HirExpr> = mro_class_def(owner, classes)
        .omitted_method_defaults(&mangled, args.len())
        .unwrap_or_default()
        .into_iter()
        .cloned()
        .collect();
    // #1337 (D-254): the receiver-exact copy when one exists.
    let mangled = exact_callee(&class_def.name, owner, mangled, scopes, classes);
    let ty = lookup(scopes, &format!("$fn:{mangled}"));
    let mut call_args = Vec::with_capacity(args.len() + omitted.len() + 1);
    call_args.push(base);
    call_args.extend(
        args.iter()
            .chain(omitted.iter())
            .map(|a| lower_expr(a, scopes, classes, current_class)),
    );
    MirExpr::Call {
        callee: mangled,
        args: call_args,
        ty,
    }
}
