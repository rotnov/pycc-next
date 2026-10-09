//! String-literal type annotations that reach the resolver (Part 1 of
//! #889).
//!
//! `pycc_parser::parse_all` has already replaced every top-level string
//! annotation with the expression it contains
//! ([`pycc_ast::normalize_string_annotations`]). Two kinds of string still
//! arrive here: one nested inside an annotation (`list["C"]`,
//! `Optional["C"]`), and a top-level one the normalizer left in place
//! because it does not parse. Both are handled by
//! [`string_annotation_to_ty`].

use super::annotation_to_ty;
use crate::class::ClassAnnotationInfo;
use crate::{Ty, unsupported};
use pycc_ast::ExprStringLiteral;
use pycc_diag::Diagnostic;

/// Resolves a string annotation exactly as its unquoted spelling would be
/// resolved, in the same scope.
///
/// A string that is not a valid Python expression is `C0001` on the
/// literal, naming its text and ruff's error kind. A string that parses
/// but names something unsupported gets that spelling's own diagnostic,
/// placed on the literal.
pub(super) fn string_annotation_to_ty(
    literal: &ExprStringLiteral,
    type_param: Option<&str>,
    class_name: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Ty, Diagnostic> {
    match pycc_ast::parse_string_annotation(literal) {
        Ok(expr) => annotation_to_ty(&expr, type_param, class_name, aliases, class_defs),
        Err(kind) => Err(unsupported(
            format!(
                "string type annotation `{}` is not a valid Python expression: {kind}",
                literal.value.to_str()
            ),
            std::ops::Range::<u32>::from(literal.range),
        )),
    }
}
