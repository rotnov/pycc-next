//! Protocol-parameter method-call specialization (#953), extracted from
//! `monomorphize.rs` under `AGENTS.md`'s decomposability rule when #1337
//! routed its callee through the receiver-exact resolver (D-254).

use std::collections::{HashMap, HashSet};

use pycc_hir::{HirExpr, HirItem, Ty, inherited_copy_name};

use super::{mangle_protocol_instantiation, substitute_body_protocols, substitute_ty_protocols};
use crate::{Environment, infer_expr_in};

/// #953: Specializes one `base.method(args)` call whose resolved method
/// has protocol-typed parameters, returning the `HirExpr::Call` that
/// replaces it — the same shape `pycc_mir`'s own method-call lowering
/// produces (`MirExpr::Call { callee: <mangled>, args: [base] ++ args }`),
/// so the receiver simply becomes the first argument and no class method
/// table has to be rewritten.
///
/// Returns `None` — leaving the `MethodCall` untouched — whenever the
/// receiver's type is not a concrete class instance (a bare class name or
/// a `super()` receiver infers as an error; a non-`Instance` type has no
/// method table to walk), the method resolves to no entry in the
/// receiver's MRO, the resolved method is not one of the dropped
/// protocol-parameter functions, or no argument supplies a concrete type
/// for a protocol-typed parameter.
///
/// The specialization keeps `mangle_protocol_instantiation`'s literal
/// `0gen_` prefix, which `pycc_codegen` keys on
/// (`is_monomorphized = name.starts_with("0gen_")`) to dispatch a
/// compiler-generated function directly rather than through an indirect
/// function-pointer slot populated in item order. `pycc_mir` recovers the
/// owning class from the resulting `0gen_<Class>.<method>__<P>_<C>` name
/// by lookup rather than by naive prefix — see `pycc_mir`'s `lower_item`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn specialize_protocol_method_call(
    base: &HirExpr,
    method: &str,
    args: &[HirExpr],
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) -> Option<HirExpr> {
    let Ok(Ty::Instance(class_name)) = infer_expr_in(env, local_names, base) else {
        return None;
    };
    let class_def = env.lookup_class(class_name.as_ref())?;
    // Mirrors `pycc_mir`'s own `#432` MRO walk for a method call.
    let (owner, mangled) = class_def.mro.iter().find_map(|mro_class| {
        env.lookup_class(mro_class).and_then(|mro_def| {
            mro_def
                .methods
                .iter()
                .find(|(name, _)| name == method)
                .map(|(_, mangled)| (mro_class, mangled.clone()))
        })
    })?;
    // #1337 (D-254): an inherited method compiled for this receiver class
    // (a receiver-exact copy) is the body the call runs, exactly as
    // `pycc_mir`'s method-call lowering routes it.
    let mangled = inherited_copy_name(class_def, owner, &mangled, &|n| env.lookup_class(n))
        .filter(|copy| *owner != class_def.name && protocol_funcs.contains_key(copy))
        .unwrap_or(mangled);
    let Some(HirItem::Function {
        name,
        params,
        return_ty,
        body,
    }) = protocol_funcs.get(&mangled)
    else {
        return None;
    };
    // `params[0]` is `self`, which the call's own argument list does not
    // carry -- index the arguments against the remaining parameters.
    let mut substitutions: Vec<(String, Ty)> = Vec::new();
    for (i, (_, param_ty)) in params.iter().skip(1).enumerate() {
        if let Ty::Protocol(proto_name) = param_ty
            && i < args.len()
            && let Ok(Ty::Instance(concrete_name)) = infer_expr_in(env, local_names, &args[i])
        {
            substitutions.push((proto_name.as_ref().clone(), Ty::Instance(concrete_name)));
        }
    }
    if substitutions.is_empty() {
        return None;
    }
    let specialized_name = mangle_protocol_instantiation(name, &substitutions);
    if seen.insert(specialized_name.clone()) {
        specializations.push(HirItem::Function {
            name: specialized_name.clone(),
            params: params
                .iter()
                .map(|(n, ty)| (n.clone(), substitute_ty_protocols(ty, &substitutions)))
                .collect(),
            return_ty: substitute_ty_protocols(return_ty, &substitutions),
            body: substitute_body_protocols(body, &substitutions),
        });
    }
    let mut call_args = Vec::with_capacity(args.len() + 1);
    call_args.push(base.clone());
    call_args.extend(args.iter().cloned());
    Some(HirExpr::Call {
        callee: specialized_name,
        args: call_args,
    })
}
