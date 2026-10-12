//! The solver's view of the builtins `pycc_hir::builtins` lowers to runtime
//! calls: once every argument term has a concrete type, the call's result
//! type is the table's.

use super::{TypeTerm, resolved_term};
use pycc_hir::Ty;

fn concrete_args(
    terms: &[Option<TypeTerm>],
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Option<Vec<Ty>> {
    terms
        .iter()
        .map(|term| resolved_term(term.clone()?, parents, concrete))
        .collect()
}

/// The result term of builtin `callee` over `arg_terms`, when the table
/// claims it.
pub(super) fn builtin_call_term(
    callee: &str,
    arg_terms: &[Option<TypeTerm>],
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Option<TypeTerm> {
    let tys = concrete_args(arg_terms, parents, concrete)?;
    match crate::builtin_call::resolve_builtin_call(callee, &tys)? {
        Ok(ty) => Some(Ok(ty)),
        Err(_) => None,
    }
}

/// A table builtin some argument term of which is not concrete yet: its
/// result when that does not depend on the arguments, else unanswered (the
/// check phase types the call once the arguments are known) -- never a
/// refusal.
pub(super) fn pending_builtin_call(
    callee: &str,
    arg_terms: &[Option<TypeTerm>],
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Option<Option<TypeTerm>> {
    if !pycc_hir::builtins::CALL_NAMES.contains(&callee)
        || concrete_args(arg_terms, parents, concrete).is_some()
    {
        return None;
    }
    Some(pycc_hir::builtins::fixed_return(callee).map(Ok))
}

/// The result term of `base.method(args)` on a builtin receiver type.
pub(super) fn builtin_method_term(
    base: &Option<TypeTerm>,
    method: &str,
    arg_terms: &[Option<TypeTerm>],
    parents: &mut [usize],
    concrete: &[Option<Ty>],
) -> Option<TypeTerm> {
    let base = resolved_term(base.clone()?, parents, concrete)?;
    let tys = concrete_args(arg_terms, parents, concrete)?;
    // Non-strict mode: CPython's own `str` method (`expr::attr_infer`).
    if base == Ty::Str && tys.contains(&Ty::Object) && crate::non_strict::is_non_strict() {
        return Some(Ok(Ty::Object));
    }
    match pycc_hir::builtins::resolve_method(&base, method, &tys)? {
        Ok(sig) => Some(Ok(sig.ret)),
        Err(_) => None,
    }
}
