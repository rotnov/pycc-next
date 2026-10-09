//! String-literal type annotations (`def f(self) -> "C[T]"`, Part 1 of
//! #889).
//!
//! CPython never evaluates a string annotation, so a quoted annotation
//! means exactly what its unquoted spelling means. pycc honours that in two
//! places:
//!
//! - [`normalize_string_annotations`] runs once, right after a module
//!   parses (`pycc_parser::parse_all`). It replaces every *top-level* string
//!   annotation with the expression the string contains. Every later reader
//!   of an annotation -- `Final`/`ClassVar` detection, `__slots__` checks,
//!   the type resolver -- therefore sees the unquoted expression at the
//!   top.
//! - [`parse_string_annotation`] parses a string that is still in the tree,
//!   i.e. one nested inside an annotation (`list["C"]`) or one the
//!   normalizer left in place because it does not parse. The type resolver
//!   calls it from its own string-literal arm, and
//!   [`unquote_nested_string_annotations`] applies it throughout an
//!   annotation for a reader that matches by shape (`__slots__`).
//!
//! The rewrite touches nothing but the annotation itself. Strings inside
//! an annotation stay strings, so `Literal["a"]` keeps its literal and
//! `Annotated[int, "doc"]` its metadata. Type-parameter bounds and
//! `type X = ...` values are expressions, not annotations, and stay as
//! written.

use crate::{Expr, ExprStringLiteral, ModModule};
use ruff_python_ast::relocate::relocate_expr;
use ruff_python_ast::visitor::transformer::{Transformer, walk_body, walk_expr};

/// Parses the contents of a string type annotation as a Python expression,
/// with every node of the result placed at the literal's own span.
///
/// The `Err` text is ruff's error kind only (for example "Expected an
/// expression"), never its full message. The full message names a byte
/// range relative to the string's *contents*, which would mislead a reader
/// who looks it up in the module.
pub fn parse_string_annotation(literal: &ExprStringLiteral) -> Result<Expr, String> {
    let parsed = ruff_python_parser::parse_expression(literal.value.to_str())
        .map_err(|error| error.error.to_string())?;
    let mut expr = parsed.into_expr();
    relocate_expr(&mut expr, literal.range);
    Ok(expr)
}

/// A copy of `annotation` with every string inside it that parses replaced
/// by the expression it contains, recursively (`ClassVar["list['str']"]`
/// becomes `ClassVar[list[str]]`). For a reader that matches an annotation
/// by shape rather than through the type resolver, so that a nested string
/// matches exactly where its unquoted spelling does. A string that does not
/// parse stays a string.
pub fn unquote_nested_string_annotations(annotation: &Expr) -> Expr {
    let mut copy = annotation.clone();
    Unquoter.visit_expr(&mut copy);
    copy
}

struct Unquoter;

impl Transformer for Unquoter {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::StringLiteral(literal) = expr
            && let Ok(parsed) = parse_string_annotation(literal)
        {
            // Each parse strips one level of quoting, so this terminates.
            *expr = parsed;
            self.visit_expr(expr);
            return;
        }
        walk_expr(self, expr);
    }
}

/// Replaces each top-level string annotation in `module` with the
/// expression it contains. `source` is the full text `module` was parsed
/// from.
///
/// A simple string (one part, no escapes) keeps the exact span of each
/// name inside it. A concatenated or escaped string puts every node at the
/// whole literal's span, ruff's own rule. A string that does not parse is
/// left in place, so the type resolver reports it on the literal.
pub fn normalize_string_annotations(module: &mut ModModule, source: &str) {
    walk_body(&Normalizer { source }, &mut module.body);
}

struct Normalizer<'s> {
    source: &'s str,
}

impl Transformer for Normalizer<'_> {
    fn visit_annotation(&self, annotation: &mut Expr) {
        let Expr::StringLiteral(literal) = annotation else {
            return;
        };
        if let Ok(parsed) = ruff_python_parser::typing::parse_type_annotation(literal, self.source)
        {
            *annotation = parsed.expression().clone();
        }
    }

    /// Expressions hold no statements and no annotations, so the walk
    /// never needs to enter one. Skipping them also keeps type-parameter
    /// bounds and `type X = ...` values, which ruff routes here rather than
    /// through `visit_annotation`, untouched.
    fn visit_expr(&self, _expr: &mut Expr) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Stmt, expr_range};

    fn normalized(source: &str) -> ModModule {
        let mut module = ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax();
        normalize_string_annotations(&mut module, source);
        module
    }

    fn function(module: &ModModule, index: usize) -> &crate::StmtFunctionDef {
        module.body[index].as_function_def_stmt().unwrap()
    }

    fn ann_assign(stmt: &Stmt) -> &crate::StmtAnnAssign {
        stmt.as_ann_assign_stmt().unwrap()
    }

    #[test]
    fn a_simple_string_keeps_the_exact_span_of_each_name() {
        let source = "def f(a: \"C[T]\") -> 'D':\n    pass\n";
        let module = normalized(source);
        let def = function(&module, 0);
        let param = def.parameters.args[0]
            .parameter
            .annotation
            .as_deref()
            .unwrap();
        let subscript = param.as_subscript_expr().unwrap();
        let base = subscript.value.as_name_expr().unwrap();
        assert_eq!(base.id.as_str(), "C");
        assert_eq!(
            &source[expr_range(&subscript.value).start as usize..][..1],
            "C"
        );
        assert_eq!(expr_range(param), 10..14);
        let returns = def.returns.as_deref().unwrap();
        assert_eq!(returns.as_name_expr().unwrap().id.as_str(), "D");
        assert_eq!(expr_range(returns), 21..22);
    }

    #[test]
    fn a_concatenated_string_takes_the_whole_literal_span() {
        let source = "x: \"list\" \"[int]\"\n";
        let module = normalized(source);
        let annotation = &ann_assign(&module.body[0]).annotation;
        let subscript = annotation.as_subscript_expr().unwrap();
        assert_eq!(subscript.value.as_name_expr().unwrap().id.as_str(), "list");
        assert_eq!(expr_range(annotation), 3..17);
        assert_eq!(expr_range(&subscript.slice), 3..17);
    }

    #[test]
    fn a_triple_quoted_string_parses() {
        let source = "x: \"\"\"int\"\"\"\n";
        let module = normalized(source);
        let annotation = &ann_assign(&module.body[0]).annotation;
        assert_eq!(annotation.as_name_expr().unwrap().id.as_str(), "int");
    }

    #[test]
    fn an_unparsable_string_is_left_in_place() {
        let source = "x: \"not a type\"\n";
        let module = normalized(source);
        assert!(
            ann_assign(&module.body[0])
                .annotation
                .is_string_literal_expr()
        );
    }

    #[test]
    fn strings_inside_an_annotation_are_untouched() {
        let module = normalized("x: Literal[\"a\"]\ny: list[\"C\"]\n");
        for stmt in &module.body {
            let subscript = ann_assign(stmt).annotation.as_subscript_expr().unwrap();
            assert!(subscript.slice.is_string_literal_expr());
        }
    }

    #[test]
    fn type_parameter_bounds_and_type_alias_values_stay_strings() {
        let module = normalized("def f[T: \"C\"](a: T) -> T:\n    return a\ntype X = \"int\"\n");
        let def = function(&module, 0);
        let type_params = def.type_params.as_deref().unwrap();
        let bound = type_params.type_params[0]
            .as_type_var()
            .unwrap()
            .bound
            .as_deref()
            .unwrap();
        assert!(bound.is_string_literal_expr());
        let alias = module.body[1].as_type_alias_stmt().unwrap();
        assert!(alias.value.is_string_literal_expr());
    }

    #[test]
    fn annotations_in_nested_bodies_are_rewritten() {
        let source = "class C:\n    a: \"int\"\n    def m(self) -> \"C\":\n        b: \"int\" = 1\n        return self\n";
        let module = normalized(source);
        let class = module.body[0].as_class_def_stmt().unwrap();
        assert!(ann_assign(&class.body[0]).annotation.is_name_expr());
        let method = class.body[1].as_function_def_stmt().unwrap();
        assert!(method.returns.as_deref().unwrap().is_name_expr());
        assert!(ann_assign(&method.body[0]).annotation.is_name_expr());
    }

    #[test]
    fn parse_string_annotation_relocates_to_the_literal() {
        let source = "x = \"list[C]\"\n";
        let module = ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax();
        let value = &module.body[0].as_assign_stmt().unwrap().value;
        let literal = value.as_string_literal_expr().unwrap();
        let expr = parse_string_annotation(literal).unwrap();
        let subscript = expr.as_subscript_expr().unwrap();
        assert_eq!(expr_range(&expr), 4..13);
        assert_eq!(expr_range(&subscript.slice), 4..13);
    }

    #[test]
    fn parse_string_annotation_reports_only_the_error_kind() {
        let source = "x = \"a b\"\n";
        let module = ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax();
        let value = &module.body[0].as_assign_stmt().unwrap().value;
        let literal = value.as_string_literal_expr().unwrap();
        let error = parse_string_annotation(literal).unwrap_err();
        assert!(!error.is_empty());
        assert!(!error.contains("byte range"), "{error}");
    }

    #[test]
    fn unquote_nested_string_annotations_strips_every_parsable_level() {
        let source = "x = ClassVar[\"list['str']\"]\ny = list[\"a b\"]\n";
        let module = ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax();
        let value = |index: usize| &module.body[index].as_assign_stmt().unwrap().value;
        let unquoted = unquote_nested_string_annotations(value(0));
        let class_var = unquoted.as_subscript_expr().unwrap();
        let list = class_var.slice.as_subscript_expr().unwrap();
        assert_eq!(list.value.as_name_expr().unwrap().id.as_str(), "list");
        assert_eq!(list.slice.as_name_expr().unwrap().id.as_str(), "str");
        // A string that does not parse stays a string.
        let kept = unquote_nested_string_annotations(value(1));
        assert!(
            kept.as_subscript_expr()
                .unwrap()
                .slice
                .is_string_literal_expr()
        );
    }
}
