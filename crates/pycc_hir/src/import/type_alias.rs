//! The two type-alias statement lowerings `module::lower_top_level_item`
//! tries before its import arm: PEP 695 `type X = ...` and the legacy PEP
//! 613 `X: TypeAlias = ...` spelling (D-135). Extracted from `import.rs`
//! (#1291) with no logic change; both call `annotation_to_ty`, which lives
//! in the sibling `func` module.

use crate::class::ClassAnnotationInfo;
use crate::{Ty, annotation_to_ty};
use pycc_ast::{Expr, ExprName, Stmt};
use pycc_diag::{Diagnostic, Span};

/// Recognizes a PEP 695 `type X = <expr>` statement and evaluates its RHS as
/// a type expression, reusing `annotation_to_ty` (D-135) -- the same
/// resolver used for parameter/return/variable annotations, since a type
/// alias's RHS is syntactically just another type expression. Returns
/// `Ok(None)` for any other statement kind, leaving it to the caller's own
/// dispatch.
///
/// A generic alias (`type X[T] = ...`) is rejected with `T0042`, not the
/// generic `unsupported`/`C0001` catch-all: D-134/D-135 explicitly scope a
/// generic alias out of this PR, but -- unlike, say, `async def`, which is
/// simply unrecognized syntax -- this shape *is* recognized and type-checked
/// far enough to name precisely why it is rejected, the same reasoning
/// `check_generic_function`'s own `T0042` diagnostics already use for a
/// generic function's out-of-scope shapes.
pub(crate) fn lower_type_alias_stmt(
    stmt: &Stmt,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Option<(String, Ty)>, Diagnostic> {
    let Stmt::TypeAlias(type_alias) = stmt else {
        return Ok(None);
    };
    // `type_alias.type_params` being `Some(_)` at all is enough to reject:
    // `ruff_python_parser`'s own `parse_type_params` reports a parse error
    // (`EmptyTypeParams`, surfaced by this crate's own `pycc_parser::parse`
    // as `L0001` before this function ever runs) for an empty `[]`, so a
    // `Some(type_params)` reaching this point always has at least one entry
    // -- there is no valid parsed input where an extra `.type_params.is_empty()`
    // check here would ever be reached with a `false` result to skip on
    // (confirmed against the pinned `ruff_python_parser = "0.0.6"` registry
    // source, the same way this function's own name-target extraction below
    // documents its own unreachable shape).
    if type_alias.type_params.is_some() {
        let range = std::ops::Range::<u32>::from(type_alias.range);
        return Err(Diagnostic::error(
            "T0042",
            "a generic type alias (`type X[T] = ...`) is not supported yet".to_string(),
            Span::new(range.start, range.end),
        ));
    }
    // Unlike the legacy `AnnAssign` form's target (which can be an
    // `Attribute`/`Subscript`, see `lower_legacy_type_alias_ann_assign`
    // below), `ruff_python_parser`'s own `parse_type_alias_statement`
    // unconditionally builds this field as `Expr::Name(self.parse_name(...))`
    // -- there is no valid source text that parses a `type` statement with a
    // non-name target, so there is no `unsupported`/unreachable fallback
    // branch to write or cover here (confirmed against the pinned
    // `ruff_python_parser = "0.0.6"` registry source). `.expect(...)`, not a
    // hand-rolled panic arm, per this crate's own documented coverage
    // convention (`pycc_ast::re_exported_grammar_types_resolve_and_have_the_expected_shape`'s
    // comment): the panic path lives in libcore, invisible to instrumented
    // regions, the same way `.unwrap()`'s does.
    let name = type_alias
        .name
        .as_name_expr()
        .expect("ruff always parses a `type` statement's name as Expr::Name");
    let ty = alias_value_to_ty(&type_alias.value, aliases, class_defs)?;
    Ok(Some((name.id.to_string(), ty)))
}

/// Recognizes the legacy `X: TypeAlias = <expr>` annotated-assignment form
/// of a type alias (PEP 613). Real Python requires `from typing import
/// TypeAlias` before this annotation is meaningful, but requiring that
/// import here is not merely inconsistent with existing precedent -- it is
/// currently infeasible: `pycc_hir` has no `Stmt::Import`/`Stmt::ImportFrom`
/// handling anywhere in this crate, so `from typing import TypeAlias` would
/// itself be unconditionally rejected with the generic `C0001` ("statement
/// kind not supported yet") diagnostic if pycc tried to require it first.
/// There is no accepted-bare-typing-name precedent to lean on either --
/// `Any` is the only other typing-shaped bare name `annotation_to_ty`
/// currently recognizes, and it is rejected with `T0002`, not accepted
/// (outside an `ext` module, where D-258 lowers it to the CPython object
/// since #1397 -- still with no import verification). So
/// this function accepts the bare annotation name `TypeAlias`
/// unconditionally, not by analogy to an existing precedent, but because
/// real import verification cannot be expressed with this crate's current
/// statement coverage (plan-deviation note, since the design doc leaves
/// this specific question open; import support is PR-14's).
///
/// Returns `Ok(None)` for any statement that is not this exact shape --
/// including an ordinary `X: TypeAlias` with no value, which is invalid as a
/// type alias and instead falls through to the ordinary `AnnAssign` lowering
/// path, where `annotation_to_ty` rejects the bare name `TypeAlias` with the
/// same `C0001` catch-all as any other unrecognized annotation name.
pub(crate) fn lower_legacy_type_alias_ann_assign(
    stmt: &Stmt,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Option<(String, Ty)>, Diagnostic> {
    let Some((target, value)) = legacy_type_alias_parts(stmt) else {
        return Ok(None);
    };
    let ty = alias_value_to_ty(value, aliases, class_defs)?;
    Ok(Some((target.id.to_string(), ty)))
}

/// The target and value of a legacy `X: TypeAlias = <value>` statement, or
/// `None` for any other shape: the annotation must be the bare name
/// `TypeAlias`, a value must be present, and the target must be a name. The
/// pure shape half of [`lower_legacy_type_alias_ann_assign`], shared with
/// `carrier::reject_carrier_misuse` (#1380), which must treat exactly the
/// values that lowering reads as a type as annotation positions.
pub(crate) fn legacy_type_alias_parts(stmt: &Stmt) -> Option<(&ExprName, &Expr)> {
    let Stmt::AnnAssign(ann) = stmt else {
        return None;
    };
    let Expr::Name(annotation_name) = ann.annotation.as_ref() else {
        return None;
    };
    if annotation_name.id.as_str() != "TypeAlias" {
        return None;
    }
    let value = ann.value.as_deref()?;
    let Expr::Name(target) = ann.target.as_ref() else {
        return None;
    };
    Some((target, value))
}

/// Resolves a type alias's value (`type X = <value>`, `X: TypeAlias =
/// <value>`) as a type. The value is an expression, not an annotation, so
/// `pycc_parser::parse_all` leaves a quoted one in place (Part 1 of #889
/// unquotes annotations only). A quoted value (`type X = "int"`) is
/// therefore refused with the `C0001` it had before #889, rather than
/// resolving through `annotation_to_ty`'s nested-string arm. A string
/// nested inside the value (`type X = list["int"]`) is an annotation
/// argument, and resolves like one.
fn alias_value_to_ty(
    value: &Expr,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Ty, Diagnostic> {
    if value.is_string_literal_expr() {
        // No bare-container advice: it would see through the quotes and
        // suggest `list[int]` for `type X = "list"`, a spelling that is
        // refused here as well.
        return Err(crate::unsupported(
            format!(
                "only a bare name type annotation is supported so far, got {}",
                pycc_ast::expr_kind_name(value)
            ),
            pycc_ast::expr_range(value),
        ));
    }
    annotation_to_ty(value, None, None, aliases, class_defs)
        .map_err(|error| crate::with_bare_container_advice(error, value))
}
