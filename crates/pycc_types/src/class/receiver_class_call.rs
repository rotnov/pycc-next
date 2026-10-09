//! Checking `type(self)(args)` (`HirExpr::ReceiverClassCall`, #1411).
//!
//! The call constructs the class `self` is statically typed as in the body
//! being checked, through the same [`super::resolve_instantiation`] an
//! explicit `C(args)` uses, so every constructor rule (the `__init__` MRO
//! walk, the argument check, the abstract/protocol/enum refusals) applies
//! unchanged. pycc dispatches statically (D-006), so that class *is* the
//! receiver's run-time class: an inherited body that constructs through its
//! receiver is compiled again for each subclass with `self` retyped
//! (D-254, `crate::inherited_copies`), and each compilation constructs its
//! own receiver's class, as CPython's `type(self)` does.
//!
//! Two shapes are refused here because lowering cannot see them: a body
//! where `self` is not bound to an instance of a class of this program (a
//! `@classmethod`, or a `@staticmethod` whose `self` parameter has another
//! type), and a program that rebinds the name `type`, where `type(self)`
//! would call that binding rather than the builtin. A `@staticmethod` whose
//! `self` parameter is typed as a class `C` constructs `C`, which is exact:
//! an instance is assignable only to its own class.

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

use super::resolve_instantiation;

/// Infers `type(self)(args)` (module doc).
pub(crate) fn infer_receiver_class_call(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    if env.lookup("type").is_some()
        || local_names.contains(&"type")
        || env.lookup_function("type").is_some()
        || env.lookup_class("type").is_some()
    {
        return Err(Diagnostic::error(
            "C0001",
            "`type(self)(...)` is supported only while `type` is the builtin -- this program \
             rebinds the name `type`",
            Span::new(0, 0),
        ));
    }
    let class = match env.lookup("self") {
        Some(Ty::Instance(class)) if env.lookup_class(&class).is_some() => class,
        _ => {
            return Err(Diagnostic::error(
                "C0001",
                "`type(self)(...)` is supported only in an instance method, or wherever `self` \
                 is typed as an instance of a class of this program -- a `@classmethod` has no \
                 `self` receiver"
                    .to_string(),
                Span::new(0, 0),
            ));
        }
    };
    let arg_tys = args
        .iter()
        .map(|arg| infer_expr_in(env, local_names, arg))
        .collect::<Result<Vec<_>, _>>()?;
    resolve_instantiation(env, class.as_str(), args, &arg_tys)
}
