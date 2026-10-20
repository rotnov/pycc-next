//! Union annotations: PEP 604's `X | Y` and `typing`'s `Optional[X]` and
//! `Union[X, ...]`, which denote the same types and lower to the same `Ty`.
//!
//! `T | None` is `Ty::Optional(T)` where the codegen can hold it (an `int`,
//! `float` or `bool` with a presence flag, any pointer-represented `T` as a
//! nullable pointer), `int | float` is `Ty::Num`, and every other union is a
//! CPython object in non-strict mode (CPython's value is whichever member it
//! is) and `T0048` in strict mode.

use super::annotation::is_int_float_union_names;
use super::{annotation_to_ty, is_non_strict};
use crate::Ty;
use crate::class::ClassAnnotationInfo;
use pycc_ast::Expr;
use pycc_diag::{Diagnostic, Span};
use std::ops::Range;

/// What the lowering of a union annotation needs from its scope.
pub(super) struct Scope<'a> {
    pub(super) type_param: Option<&'a str>,
    pub(super) class_name: Option<&'a str>,
    pub(super) aliases: &'a [(String, Ty)],
    pub(super) class_defs: &'a [ClassAnnotationInfo],
}

impl Scope<'_> {
    fn lower(&self, expr: &Expr) -> Result<Ty, Diagnostic> {
        annotation_to_ty(expr, self.type_param, self.class_name, self.aliases, self.class_defs)
    }
}

/// `X | Y` (an `Expr::BinOp` with `|`): `T | None` or `None | T` is
/// optional, `int | float` is `Ty::Num`, any other pair (a chain is a pair
/// whose left side is itself a union) is a general union.
pub(super) fn bitor_to_ty(
    bin_op: &pycc_ast::ExprBinOp,
    scope: &Scope<'_>,
) -> Result<Ty, Diagnostic> {
    let range = Range::<u32>::from(bin_op.range);
    match (bin_op.left.as_ref(), bin_op.right.as_ref()) {
        (Expr::NoneLiteral(_), other) | (other, Expr::NoneLiteral(_)) => {
            optional_of(other, range, scope)
        }
        (a, b) if is_int_float_union_names(a, b) => Ok(Ty::Num),
        _ => general_union(range, scope),
    }
}

/// A union of members neither `None` nor `int | float`.
fn general_union(range: Range<u32>, scope: &Scope<'_>) -> Result<Ty, Diagnostic> {
    // Non-strict mode (v0.7 accept 2): a general union is an object, as
    // CPython's value is whichever member it is.
    if is_non_strict(scope.aliases) {
        return Ok(Ty::Object);
    }
    Err(Diagnostic::error(
        "T0048",
        "general union annotations (`X | Y` where neither side is `None`) are not supported yet -- only `T | None` (PEP 604 Optional) is",
        Span::new(range.start, range.end),
    ))
}

/// The `Ty` of the union of `members`, and of `None` too when `or_none`
/// (`Optional[X]`); the annotation spans `range`.
pub(super) fn union_to_ty(
    members: &[&Expr],
    or_none: bool,
    range: Range<u32>,
    scope: &Scope<'_>,
) -> Result<Ty, Diagnostic> {
    let (nones, others): (Vec<&Expr>, Vec<&Expr>) = members
        .iter()
        .copied()
        .partition(|member| matches!(member, Expr::NoneLiteral(_)));
    match (others.as_slice(), nones.is_empty() && !or_none) {
        // `Union[X]` is `X`, as `typing` itself reduces it.
        ([one], true) => scope.lower(one),
        ([a, b], true) if is_int_float_union_names(a, b) => Ok(Ty::Num),
        ([one], false) => optional_of(one, range, scope),
        // Only `None`: `Union[None]` is `None`.
        ([], _) => Ok(Ty::None),
        _ => general_union(range, scope),
    }
}

/// `inner | None`.
fn optional_of(inner: &Expr, range: Range<u32>, scope: &Scope<'_>) -> Result<Ty, Diagnostic> {
    let non_strict = is_non_strict(scope.aliases);
    let inner = match scope.lower(inner) {
        Ok(inner) => inner,
        // Non-strict mode: `T | None` of what is an object anyway
        // (`mod.Type | None`) is an object too.
        Err(_) if non_strict => return Ok(Ty::Object),
        Err(error) => return Err(error),
    };
    // `Optional[T]` is supported for `T` in `{int, float, bool}` (D-197;
    // codegen's `{ inner, i8 }` struct) and, in strict mode, for every
    // pointer-represented `T` (`is_pointer_optional_inner`: a null pointer is
    // `None`). Gated here, at the one place a `Ty::Optional` is constructed
    // from source.
    if !matches!(inner, Ty::Int | Ty::Float | Ty::Bool) && non_strict {
        // Non-strict mode: an optional value of any other type is an object
        // (`None` or the value), as in CPython.
        return Ok(Ty::Object);
    }
    if !matches!(inner, Ty::Int | Ty::Float | Ty::Bool) && !crate::is_pointer_optional_inner(&inner) {
        return Err(Diagnostic::error(
            "T0049",
            format!(
                "`Optional[{}]` is not supported yet -- only `int`, `float`, `bool`, `str`, a list, dict or set, a class instance, \
                 or a tuple with a non-number element can be optional (`T | None`)",
                inner.name()
            ),
            Span::new(range.start, range.end),
        ));
    }
    Ok(Ty::Optional(Box::new(inner)))
}

/// The members an `Optional[X]` or `Union[X, ...]` subscript names: its
/// arguments.
pub(super) fn subscript_members(slice: &Expr) -> Vec<&Expr> {
    match slice {
        Expr::Tuple(tuple) => tuple.elts.iter().collect(),
        other => vec![other],
    }
}
