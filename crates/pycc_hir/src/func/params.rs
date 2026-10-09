//! The parameter-shape rules a `def` and a method share, plus the
//! default-value rules Part 2 of #884 (issue #1189) adds.
//!
//! Two textually duplicated copies of the same three shape checks used to
//! live in `func::lower_params` and `class::lower_method`; keeping them in
//! one place is this change's AGENTS.md file-decomposition obligation (both
//! owning files are over the ~1,000-line threshold) and it makes Parts 5 and
//! 6 of #884 — keyword-only parameters and `*args`/`**kwargs` — single-site
//! changes instead of two-site ones.
//!
//! The default-value half is the other reason this module exists.
//! [`literal_default`] is a purely *syntactic* recognizer, called from two
//! places that need different answers from it:
//!
//! * `expr::keyword_bind::signature_of`, which is infallible and runs before
//!   any item is lowered, uses `None` to mean "leave this `def` out of the
//!   signature table entirely", so no call site can ever bind against a
//!   signature this part cannot represent; and
//! * [`check_default`], reached from `func::lower_params` and, since the
//!   method part of #1140, `class::lower_method`, which turns the same
//!   `None` into the `C0001` the user sees, and additionally applies every
//!   *type* rule — assignability and the PEP 695 exclusion.
//!
//! Splitting it that way keeps the rejection at exactly one site, so a bad
//! default is reported once at its own `def` rather than once per call.

use pycc_ast::{Expr, Number, Parameters, UnaryOp};
use pycc_diag::{Diagnostic, Span};

use crate::{HirExpr, Ty, unsupported};

/// Whether a parameter list's caller can serve a default value.
///
/// Part 2 of #884 (#1189) implements defaults for a module-level `def`:
/// `func::lower_params` passes [`DefaultPolicy::Admit`]. The method part of
/// #1140 admits them on a method too: every `class::lower_method` arm passes
/// [`DefaultPolicy::AdmitMethod`]. A protocol-member caller passes
/// [`DefaultPolicy::Reject`] and keeps the pre-existing capability
/// diagnostic byte for byte.
///
/// The policy is an explicit argument rather than something derived from
/// another parameter (`class_name.is_none()` happens to discriminate the
/// module-level call sites today) precisely so a future caller cannot
/// acquire the wrong answer by accident.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DefaultPolicy {
    /// Validate each default and keep it: a module-level `def`, whose
    /// default is spliced into an in-module call's argument vector.
    Admit,
    /// [`DefaultPolicy::Admit`]'s rules plus one: `None` may default a
    /// parameter of the opaque `object` type (`Ty::Object` -- `Any` or
    /// `object` under `--ext`, D-258).
    ///
    /// Such a default is read only by the `--ext` host boundary. The
    /// boundary hands the parameter the host's own `None` object, exactly
    /// as it does when a host passes `None` explicitly.
    ///
    /// No in-module call ever materializes this default. Since Part 1 of
    /// #1191 (#1438) an in-module instance-method call may omit a defaulted
    /// parameter, but `pycc_types` refuses a call that would fill `None`
    /// into an `object` parameter.
    ///
    /// A module-level `def` gets no such relaxation: its default is spliced
    /// into a native call, where `None` at an `object` slot is refused.
    AdmitMethod,
    /// Report the unchanged `C0001` for any default at all.
    Reject,
}

/// The three parameter *kinds* no part of #884 implements yet, in the order
/// `func::lower_params` has always checked them.
///
/// Order is observable: a signature carrying both a `*args` and a
/// keyword-only parameter reports the `*args` message. Messages and spans
/// are byte-identical to the two copies this replaced.
pub(crate) fn reject_unsupported_parameter_shapes(
    parameters: &Parameters,
) -> Result<(), Diagnostic> {
    if parameters.vararg.is_some() {
        return Err(unsupported(
            "`*args` is not supported yet",
            parameters.range,
        ));
    }
    if !parameters.kwonlyargs.is_empty() {
        return Err(unsupported(
            "keyword-only parameters are not supported yet",
            parameters.range,
        ));
    }
    if parameters.kwarg.is_some() {
        return Err(unsupported(
            "`**kwargs` is not supported yet",
            parameters.range,
        ));
    }
    Ok(())
}

/// The message [`check_default`] reports for a default expression outside the
/// admitted literal subset. Shared with the doc-comment-level statement of
/// the subset in `docs/TYPE_SYSTEM.md`.
pub(crate) const UNADMITTED_DEFAULT: &str = "only a literal `int`, `float`, `bool`, `str`, or `None` default parameter value is \
     supported yet (a unary `-`/`+` may be applied to a numeric literal)";

/// Recognizes a default expression this part can materialize at the call
/// site, returning **exactly** the `HirExpr` the same literal written
/// explicitly as an argument lowers to.
///
/// Returning the lowered node rather than a description is what makes the
/// design's load-bearing invariant mechanical: a filled default is
/// byte-identical to its explicit twin, so a default is never checked more
/// strictly — or more loosely — than the same value at the call site.
///
/// Recognition is purely syntactic. No annotation is consulted, and nothing
/// here can fail with a diagnostic: `signature_of` needs an infallible,
/// context-free answer, and [`check_default`] owns every type rule.
///
/// The arms mirror `expr::lower_expr`'s own literal arms one for one,
/// including the two *different* integer gates. An unsigned `Number::Int`
/// is admitted only when `Int::as_i64()` succeeds, matching `lower_expr`'s
/// unsigned arm; a *negated* one goes through `expr::fold_int_literal_sign`,
/// which applies the sign before the range check, because
/// `-9223372036854775808` parses as `USub` over a magnitude whose own
/// `as_i64()` fails even though the negation is exactly `i64::MIN` — and
/// `f(-9223372036854775808)` compiles today. `Number::Complex` is admitted
/// in neither position, matching `lower_expr`'s own rejection of it.
pub(crate) fn literal_default(expr: &Expr) -> Option<HirExpr> {
    match expr {
        Expr::NumberLiteral(lit) => match &lit.value {
            Number::Int(i) => i.as_i64().map(HirExpr::IntLiteral),
            Number::Float(f) => Some(HirExpr::FloatLiteral(*f)),
            _ => None,
        },
        Expr::BooleanLiteral(lit) => Some(HirExpr::BoolLiteral(lit.value)),
        Expr::StringLiteral(lit) => Some(HirExpr::StringLiteral(lit.value.to_str().to_string())),
        Expr::NoneLiteral(_) => Some(HirExpr::NoneLiteral),
        Expr::UnaryOp(unary) => match (unary.op, unary.operand.as_ref()) {
            (UnaryOp::USub | UnaryOp::UAdd, Expr::NumberLiteral(lit)) => {
                let negate = matches!(unary.op, UnaryOp::USub);
                match &lit.value {
                    Number::Int(i) => {
                        crate::expr::fold_int_literal_sign(i, negate, pycc_ast::expr_range(expr))
                            .ok()
                            .map(HirExpr::IntLiteral)
                    }
                    Number::Float(f) => Some(HirExpr::FloatLiteral(if negate { -*f } else { *f })),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// The `Ty` a recognized literal default carries, for the assignability
/// rule below.
fn default_ty(default: &HirExpr) -> Ty {
    match default {
        HirExpr::IntLiteral(_) => Ty::Int,
        HirExpr::FloatLiteral(_) => Ty::Float,
        HirExpr::BoolLiteral(_) => Ty::Bool,
        HirExpr::StringLiteral(_) => Ty::Str,
        // `literal_default` produces no other variant.
        _ => Ty::None,
    }
}

/// The type an **unannotated** defaulted parameter takes in an `--ext`
/// module (#1409), or `None` when the default gives it none and the
/// parameter keeps its `T0001`.
///
/// The parameter is typed exactly as if it carried the annotation its
/// default implies (D-258's #1409 amendment), so it then goes through
/// [`check_default`] and every later pass unchanged:
///
/// * a literal `int`, `float`, `bool` or `str` default gives that scalar
///   type -- the annotation `x: bool = True` is what the parameter would
///   have been written with, and a body that branches on it
///   (`if deepcopy_values:`) stays native;
/// * a `None` default gives the opaque CPython object (`Ty::Object`, the
///   type `Any` lowers to in an `--ext` module) under
///   [`DefaultPolicy::AdmitMethod`] only. A module-level `def` splices its
///   default into in-module calls, where `None` at an object slot is still
///   refused (#1387), so there a `None` default implies no type;
/// * any other default (`[]`, a name, a call) implies no type.
pub(crate) fn unannotated_default_ty(default: &Expr, policy: DefaultPolicy) -> Option<Ty> {
    match default_ty(&literal_default(default)?) {
        Ty::None => (policy == DefaultPolicy::AdmitMethod).then_some(Ty::Object),
        scalar => Some(scalar),
    }
}

/// Whether a literal of type `from` may fill a parameter annotated `to`.
///
/// The subset of `pycc_types::is_assignable` reachable from a literal
/// default: identity, `bool` as a subtype of `int` (`docs/TYPE_SYSTEM.md`
/// rule 4 / D-086's representation table), and widening a bare value or a
/// bare `None` into a `T | None`. D-086 grants no implicit numeric widening,
/// so `def f(x: float = 1)` is rejected exactly as `f(1)` at a `float`
/// parameter is. `Ty::Param` never reaches here: a default on a PEP 695
/// type-parameter-annotated parameter is refused before this call.
fn literal_is_assignable(from: &Ty, to: &Ty) -> bool {
    from == to
        || (*from == Ty::Bool && *to == Ty::Int)
        || matches!(to, Ty::Optional(inner)
            if *from == Ty::None || literal_is_assignable(from, inner))
}

/// Validates one defaulted parameter of a module-level `def` or a method,
/// returning the lowered default on success.
///
/// `policy` is [`DefaultPolicy::Admit`] or [`DefaultPolicy::AdmitMethod`];
/// the latter additionally admits `None` at `Ty::Object` (see its own doc
/// comment for why that is sound only for a method).
///
/// This is the **single** site that rejects a default. It runs *after* the
/// parameter's annotation has been resolved, so an annotation that is itself
/// unsupported (a wider `Optional`, say) keeps reporting its own diagnostic
/// first. That ordering is deliberate and applies under both admitting
/// policies; the `Reject` arm still runs before annotation resolution,
/// preserving the pre-existing order for a protocol member.
///
/// `ty` is the parameter's resolved annotation, the type
/// [`unannotated_default_ty`] gives an unannotated public parameter of an
/// `--ext` module (#1409), or `Ty::Infer` for an unannotated parameter of a
/// private `def`. Outside that `--ext` case **no type is inferred from a
/// default**: an unannotated defaulted parameter of a private `def` keeps
/// `Ty::Infer` and its default is accepted whatever it is, exactly as the
/// same helper's unannotated non-defaulted parameter is today.
///
/// A mismatch is `T0021`, not the `T0025` an annotated *assignment* reports,
/// even though the def-site syntax resembles one: by this part's design the
/// default **is** a call-site argument, spliced into the positional vector
/// during lowering, and the existing call-argument mismatch spelling is
/// `T0021`.
pub(crate) fn check_default(
    default: &Expr,
    param_name: &str,
    fn_name: &str,
    ty: &Ty,
    policy: DefaultPolicy,
) -> Result<HirExpr, Diagnostic> {
    let range = pycc_ast::expr_range(default);
    if matches!(ty, Ty::Param(_)) {
        return Err(unsupported(
            format!(
                "a default value on parameter `{param_name}` of `{fn_name}`, whose annotation is \
                 a type parameter, is not supported yet"
            ),
            range,
        ));
    }
    let Some(lowered) = literal_default(default) else {
        return Err(unsupported(UNADMITTED_DEFAULT, range));
    };
    let from = default_ty(&lowered);
    let object_none = policy == DefaultPolicy::AdmitMethod && from == Ty::None && *ty == Ty::Object;
    if *ty != Ty::Infer && !object_none && !literal_is_assignable(&from, ty) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "default value of parameter `{param_name}` of `{fn_name}` expects `{}`, got `{}`",
                ty.name(),
                from.name()
            ),
            Span::new(range.start, range.end),
        )
        .with_help(format!(
            "a default value is passed as an argument, so it must already be `{}`",
            ty.name()
        )));
    }
    Ok(lowered)
}
