//! Type rules for the builtins `pycc_hir::builtins` lowers to runtime calls.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// A builtin call claimed by `pycc_hir::builtins` (see its module doc),
/// with `min`/`max` over three or more arguments folded pairwise.
pub(crate) fn resolve_builtin_call(callee: &str, arg_tys: &[Ty]) -> Option<Result<Ty, Diagnostic>> {
    let to_diag = |message: String| Diagnostic::error("T0021", message, Span::new(0, 0));
    if pycc_hir::builtins::is_pairwise_variadic(callee) && arg_tys.len() > 2 {
        let mut acc = arg_tys[0].clone();
        for next in &arg_tys[1..] {
            match pycc_hir::builtins::resolve_call(callee, &[acc.clone(), next.clone()])? {
                Ok(sig) => acc = sig.ret,
                Err(message) => return Some(Err(to_diag(message))),
            }
        }
        return Some(Ok(acc));
    }
    // Non-strict mode: `chr` runs in CPython (the fallback loop calls it
    // through `builtins`), whose `str` holds a lone surrogate (`chr(0xD800)`)
    // that pycc's UTF-8 `str` cannot (`docs/DEVIATIONS.md`).
    if callee == "chr" && crate::non_strict::is_non_strict() {
        return Some(Err(crate::name_diagnostics::unsupported_callable_builtin(
            callee,
        )));
    }
    // Non-strict mode: a CPython object for the one scalar parameter is
    // converted at run time (`pycc_hir::builtins::object_param`).
    if let ([Ty::Object], Some((param, _))) = (arg_tys, pycc_hir::builtins::object_param(callee))
        && crate::non_strict::is_non_strict()
    {
        return pycc_hir::builtins::resolve_call(callee, &[param])
            .map(|r| r.map(|sig| sig.ret).map_err(to_diag));
    }
    pycc_hir::builtins::resolve_call(callee, arg_tys).map(|r| r.map(|sig| sig.ret).map_err(to_diag))
}
