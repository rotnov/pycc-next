//! Lowering of an annotated assignment statement (`Stmt::AnnAssign`),
//! extracted from `crates/pycc_hir/src/stmt.rs`'s `lower_stmt` per AGENTS.md's
//! file-decomposition rule when #1264 (Part 3 of #1218) added its attribute
//! target. The bare-name arm is moved verbatim; the only edits it received
//! are the ones the function boundary forces (the `use` lines and the
//! `Ok(..)` around the arm's value).
//!
//! #1264 admits exactly one attribute-target shape, inside a function body:
//! `<base>.<attr>: list[int] = []` and `<base>.<attr>: dict[str, int] = {}`.
//! It lowers to an ordinary `HirStmt::AttrSet` whose value is a
//! `HirExpr::EmptyList`/`HirExpr::EmptyDict` built from the written
//! annotation, so the annotation is carried by the value's own type and
//! `pycc_types`' `check_attr_set` checks it against the attribute's slot
//! exactly as it checks any other stored value. `docs/TYPE_SYSTEM.md`'s class
//! "Current state" paragraph is the contract; D-245's 2026-09-24 amendment
//! records why this is the one `pycc_hir` construction site of those two
//! variants. That paragraph is also the canonical statement of how every
//! other annotated attribute target is refused, including the order of the
//! checks. The general form is #891's.

use super::lower_expr;
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExpr, HirStmt, ImportBinding, Ty, annotation_to_ty, unsupported};
use pycc_ast::{Expr, ExprAttribute, StmtAnnAssign};
use pycc_diag::Diagnostic;

/// The admitted-form sentence every refused annotated attribute target
/// ends with, so the refusals stay in step with each other.
const ADMITTED_FORM: &str = "only `<obj>.<attr>: list[int] = []` or `<obj>.<attr>: dict[str, int] = {}` \
     inside a function body is supported so far (the general annotated attribute \
     target is issue #891)";

/// Lowers `ann` (`Stmt::AnnAssign`) to `HirStmt::AnnAssign` for a bare-name
/// target, or to `HirStmt::AttrSet` for the one admitted attribute-target
/// shape (see the module doc comment).
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_ann_assign(
    ann: &StmtAnnAssign,
    aliases: &[(String, Ty)],
    in_function: bool,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    let name = match ann.target.as_ref() {
        Expr::Name(name) => name,
        Expr::Attribute(attr) => {
            return lower_attribute_target(
                ann,
                attr,
                aliases,
                in_function,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            );
        }
        other => {
            return Err(unsupported(
                format!(
                    "only assigning to a bare name is supported so far, got {}",
                    pycc_ast::expr_kind_name(other)
                ),
                pycc_ast::expr_range(other),
            ));
        }
    };
    // `ann.simple` is false either when the target isn't a bare name
    // (already handled above: lowered as an attribute target, or rejected
    // as any other non-name shape) or when a bare name target is itself
    // parenthesized, e.g. `(x): int = 1` -- upstream's own parser
    // sets `simple = target.is_name_expr() && !target.is_parenthesized`
    // (verified against the pinned ruff_python_parser = "0.0.6"
    // registry source). CPython treats a parenthesized target as not
    // "simple" (it doesn't record a `__annotations__` entry the same
    // way), a real semantic difference this compiler doesn't model
    // yet -- reject explicitly instead of silently treating it the
    // same as the unparenthesized form.
    if !ann.simple {
        return Err(unsupported(
            "a parenthesized annotated-assignment target is not supported yet",
            pycc_ast::expr_range(&ann.target),
        ));
    }
    let annotation = annotation_to_ty(&ann.annotation, type_param, class_name, aliases, class_defs)
        .map_err(|error| crate::with_bare_container_advice(error, &ann.annotation))?;
    let value = ann
        .value
        .as_deref()
        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
        .transpose()?;
    Ok(HirStmt::AnnAssign {
        target: name.id.as_str().to_string(),
        annotation,
        value,
        is_final: is_final_annotation(&ann.annotation),
    })
}

/// PEP 591 (#383): detect `Final[X]` at the AST level (before
/// `annotation_to_ty` unwrapped it to `X`) so the type checker can track
/// this binding as non-reassignable. `Final` is recognized as a bare name
/// without requiring `from typing import Final`, matching the existing
/// `TypeAlias`/`Any` precedent.
fn is_final_annotation(annotation: &Expr) -> bool {
    matches!(
        annotation,
        Expr::Subscript(sub) if matches!(
            sub.value.as_ref(),
            Expr::Name(base) if base.id.as_str() == "Final"
        )
    )
}

/// #1264: `<base>.<attr>: list[int] = []` / `<base>.<attr>: dict[str, int]
/// = {}` inside a function body lowers to `HirStmt::AttrSet` with a typed
/// empty-container value built from the annotation. The refusals, and the
/// order in which they are checked, are specified by `docs/TYPE_SYSTEM.md`'s
/// class "Current state" paragraph; the shape refusals end in
/// [`ADMITTED_FORM`].
#[allow(clippy::too_many_arguments)]
fn lower_attribute_target(
    ann: &StmtAnnAssign,
    attr: &ExprAttribute,
    aliases: &[(String, Ty)],
    in_function: bool,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    let target_range = pycc_ast::expr_range(&ann.target);
    // D-245 item 8 scopes typed empty containers to function bodies; a
    // module-level `obj.xs: list[int] = []` stays refused.
    if !in_function {
        return Err(unsupported(
            format!(
                "an annotated attribute target at module level is not supported yet -- {ADMITTED_FORM}"
            ),
            target_range,
        ));
    }
    // Checked before any annotation or value shape, so `super().x: <any> =
    // ...` always gets #448's message rather than a shape refusal.
    super::assign::reject_super_attr_base(attr)?;
    // `Final` on an attribute would need per-attribute reassignment
    // tracking this compiler does not model.
    if is_final_annotation(&ann.annotation) {
        return Err(unsupported(
            format!(
                "a `Final` annotation on an attribute target is not supported yet -- {ADMITTED_FORM}"
            ),
            target_range,
        ));
    }
    // The annotation is validated exactly as a local's is, so `list[str]`
    // is the same `T0034` and `list[T]` the same refusal. A bare `list`
    // or `dict` gets the parameterized-form advice -- the two forms this
    // position lowers -- while a bare `set`/`tuple` keeps the generic
    // message, since `docs/TYPE_SYSTEM.md` promises the advice never names
    // a form that fails too.
    let annotation = annotation_to_ty(&ann.annotation, type_param, class_name, aliases, class_defs)
        .map_err(|error| crate::func::with_bare_list_or_dict_advice(error, &ann.annotation))?;
    let Some(value) = ann.value.as_deref() else {
        return Err(unsupported(
            format!(
                "an annotated attribute target without a value is not supported yet -- {ADMITTED_FORM}"
            ),
            target_range,
        ));
    };
    let typed_empty = match (&annotation, value) {
        (Ty::List(element), Expr::List(list)) if list.elts.is_empty() => {
            HirExpr::EmptyList((**element).clone())
        }
        (Ty::Dict(pair), Expr::Dict(dict)) if dict.items.is_empty() => {
            HirExpr::EmptyDict(pair.clone())
        }
        _ => {
            return Err(unsupported(
                format!(
                    "an annotated attribute target annotated `{}` with this value is not \
                     supported yet -- {ADMITTED_FORM}",
                    annotation.name()
                ),
                target_range,
            ));
        }
    };
    Ok(HirStmt::AttrSet {
        base: lower_expr(&attr.value, in_function, class_name, imports, signatures)?,
        attr: attr.attr.to_string(),
        value: typed_empty,
    })
}

#[cfg(test)]
#[path = "ann_assign_tests.rs"]
mod tests;
