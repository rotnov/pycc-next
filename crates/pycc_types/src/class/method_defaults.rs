//! Argument checking for a regular instance-method call that may omit
//! defaulted trailing parameters (Part 1 of #1191, issue #1438).
//!
//! Only `method_call::resolve_method_call`'s regular-method arm calls
//! [`check_method_call_args`]. The other call sites keep the plain
//! [`check_call_args`] and its arity `T0021`. Those sites are the protocol
//! arm, static and class methods, `super()`, constructors and exception
//! constructors, and they belong to the parts of #1191 that are still open.
//!
//! `pycc_mir` appends the very nodes this module checks, both through
//! [`pycc_hir::HirClassDef::omitted_method_defaults`], so a call is accepted
//! here exactly when MIR can fill it.

use crate::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirClassDef, HirExpr, Ty};

use super::check_call_args;

/// Checks `arg_tys` against `param_tys`, the parameters of `owner`'s method
/// `mangled` without its receiver.
///
/// `owner` is the class whose method table the MRO walk matched, so the
/// recorded defaults are the ones the dispatched body declares.
///
/// A call that omits only defaulted parameters is checked as if each
/// default had been written at the call site. Two cases are refused:
///
/// * `C0001`: an omitted parameter is an `object` parameter whose default
///   is `None`. Boxing (#1475) happens on the call's own arguments; a
///   default filled in here is appended unboxed, so an in-module call must
///   pass that argument explicitly (an `--ext` host omitting it gets the
///   default from the export wrapper).
/// * `T0021`: the call omits a required parameter, or passes more
///   arguments than the method takes. When the method declares defaults,
///   the message gives the accepted range.
pub(super) fn check_method_call_args(
    env: &Environment,
    owner: &HirClassDef,
    mangled: &str,
    method: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
    param_tys: &[Ty],
) -> Result<(), Diagnostic> {
    let Some(omitted) = owner.omitted_method_defaults(mangled, arg_tys.len()) else {
        if arg_tys.len() != param_tys.len()
            && let Some(required) = required_count(owner, mangled)
        {
            return Err(range_arity_error(
                method,
                required,
                param_tys.len(),
                arg_tys.len(),
            ));
        }
        return check_call_args(env, method, args, arg_tys, param_tys, true);
    };
    let mut filled = arg_tys.to_vec();
    for default in omitted {
        let position = filled.len();
        if matches!(default, HirExpr::NoneLiteral) && param_tys[position] == Ty::Object {
            return Err(none_at_object(method, position + 1));
        }
        filled.push(crate::infer_expr(env, default)?);
    }
    check_call_args(env, method, args, &filled, param_tys, true)
}

/// How many leading parameters of `owner`'s method `mangled` are required,
/// excluding the receiver, or `None` when the method declares no default.
fn required_count(owner: &HirClassDef, mangled: &str) -> Option<usize> {
    let (_, defaults) = owner
        .method_defaults
        .iter()
        .find(|(held, _)| held == mangled)?;
    Some(defaults[1..].iter().take_while(|d| d.is_none()).count())
}

fn range_arity_error(method: &str, required: usize, total: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("`{method}` expects from {required} to {total} argument(s), got {got}"),
        Span::new(0, 0),
    )
    .with_help(format!(
        "pass at least {required} and at most {total} argument(s)"
    ))
}

fn none_at_object(method: &str, position: usize) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "filling the `None` default of argument {position} of `{method}`, an `object` \
             parameter, at an in-module call is not supported yet"
        ),
        Span::new(0, 0),
    )
    .with_help(format!(
        "pass argument {position} explicitly (`None` boxes into an `object` argument); an \
         in-module call does not fill an `object` parameter's `None` default yet"
    ))
}

#[cfg(test)]
#[path = "method_defaults_tests.rs"]
mod tests;
