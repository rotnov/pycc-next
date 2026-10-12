//! The constraint solver's term for a method call on a user-class instance
//! (#1420): `recv.m(args)` answers the return term of the method `m` that
//! `recv`'s class resolves it to, so an unannotated method whose body is
//! `return self.copy()` infers its return from `copy`'s.
//!
//! The method is found the way the check phase's `resolve_method_call`
//! finds it -- the first class in the receiver class's MRO whose `methods`
//! table names it -- and its signature is the one `signatures` already
//! holds for the mangled name, so an annotated method contributes its
//! declared return and an unannotated one its own inference variable.
//! Nothing here walks the callee's body: the call only links two terms in
//! the union-find, so recursion (`return self.f()` inside `f`) and mutual
//! recursion cannot loop. A recursive call adds no information of its own:
//! a helper constrained only by its own recursion leaves the return
//! variable unresolved, and signature materialization reports the usual
//! `T0021`, while one with another constraining return (a base case such
//! as `if n == 0: return 1`) resolves from that return.
//!
//! The receiver term must already be a concrete `Ok(Ty::Instance(_))` --
//! `self`, an annotated parameter, or a local bound from one. A solver
//! variable that might later resolve to an instance is deliberately not
//! consulted through `resolved_term`, on `object_lift`'s reasoning: whether
//! it is resolved yet depends on the order the helpers appear in the
//! source, so the inference would too. Every receiver this module does not
//! answer for -- a protocol, `super()`, a method in no `methods` table (a
//! `@staticmethod` or `@classmethod` lives in its own table) -- keeps the
//! `MethodCall` arm's historical "no term", and the check phase stays the
//! authority on whether the call is valid at all.
//!
//! Split out of `constraints.rs` per AGENTS.md's file-decomposition rule,
//! so that file only gains the call site.

use super::*;

/// The return term of `base.method(...)` when `base_term` is a concrete
/// user-class instance whose MRO resolves `method` to a function in
/// `signatures`; `None` for every other receiver, and for a method whose
/// return type mentions a type parameter -- this solver has no
/// generic-instantiation step, so handing the raw `Ty::Param` back would
/// leak it into the caller's inferred signature.
///
/// The call's arguments are unified with the method's parameters
/// separately, by [`unify_method_args`], and only where a parameter is
/// still an inference variable.
pub(super) fn method_call_on_instance(
    signatures: &HashMap<String, SignatureTerms>,
    env: &ConstraintEnvironment<'_, '_>,
    base_term: Option<&TypeTerm>,
    method: &str,
) -> Option<TypeTerm> {
    let Some(Ok(Ty::Instance(class_name))) = base_term else {
        return None;
    };
    let mangled = resolve_method(env.class_defs, class_name, method)?;
    let (_, _, return_term) = signatures.get(mangled)?;
    if matches!(return_term, Ok(ty) if ty_contains_param(ty)) {
        return None;
    }
    Some(return_term.clone())
}

/// The mangled name of the method `class_name`'s MRO resolves `method` to:
/// the first MRO class whose `methods` table names it, CPython's own
/// method resolution order and the check phase's (`resolve_method_call`).
fn resolve_method<'hir>(
    class_defs: &'hir [(String, pycc_hir::HirClassDef)],
    class_name: &str,
    method: &str,
) -> Option<&'hir str> {
    let lookup = |name: &str| {
        class_defs
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, def)| def)
    };
    lookup(class_name)?.mro.iter().find_map(|mro_class| {
        lookup(mro_class)?
            .methods
            .iter()
            .find(|(name, _)| name == method)
            .map(|(_, mangled)| mangled.as_str())
    })
}

/// Unifies the arguments of `base.method(args)` with the parameters of the
/// method `base`'s class resolves it to, where a parameter is still an
/// inference variable: an unannotated method of a native build is inferred
/// from its call sites as a module-level helper is (`call_term`). A
/// parameter with a concrete type -- annotated, or an `--ext` parameter
/// typed from its default -- is left to the check phase, as before.
pub(super) fn unify_method_args(
    signatures: &HashMap<String, SignatureTerms>,
    env: &ConstraintEnvironment<'_, '_>,
    base_term: Option<&TypeTerm>,
    method: &str,
    arg_terms: &[Option<TypeTerm>],
    parents: &mut [usize],
    concrete: &mut [Option<Ty>],
) -> Result<(), Diagnostic> {
    // A special method is also called implicitly (`a == b`), with
    // operands an explicit call does not show.
    let Some(Ok(Ty::Instance(class_name))) = base_term else {
        return Ok(());
    };
    if method.starts_with("__") {
        return Ok(());
    }
    let Some(mangled) = resolve_method(env.class_defs, class_name, method) else {
        return Ok(());
    };
    let Some((names, params, _)) = signatures.get(mangled) else {
        return Ok(());
    };
    // The receiver is the first parameter.
    for (index, (arg, parameter)) in arg_terms.iter().zip(params.iter().skip(1)).enumerate() {
        let (Some(arg), Err(_)) = (arg, parameter) else {
            continue;
        };
        if matches!(arg, Ok(ty) if ty_contains_param(ty)) {
            continue;
        }
        unify_terms(
            parameter.clone(),
            arg.clone(),
            parents,
            concrete,
            "T0021",
            &format!("argument {} of private helper `{mangled}`", index + 1),
        )
        .map_err(|conflict| match names.get(index + 1) {
            Some(param) if super::public_inference::reports_missing_annotation(mangled) => {
                super::public_inference::parameter_conflict(mangled, param, &conflict)
            }
            _ => conflict,
        })?;
    }
    Ok(())
}
