//! `match` statement lowering (PEP 634-636, #381, PR-21): the statement
//! itself and its patterns. Extracted from `stmt.rs` (#1291) with no logic
//! change.

use super::{ExceptStarCtx, lower_body};
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::expr::lower_expr;
use crate::{HirExpr, HirMatchCase, HirPattern, HirStmt, ImportBinding, Ty, unsupported};
use pycc_ast::{Expr, Pattern, Singleton, StmtMatch};
use pycc_diag::Diagnostic;

/// PEP 634-636 (#381, PR-21): lowers a `Stmt::Match` into `HirStmt::Match`.
/// The subject is lowered once via `lower_expr`; each case's pattern is
/// lowered via `lower_pattern`, its guard via `lower_expr`, and its body via
/// `lower_body` (which filters `Stmt::Pass`).
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_match(
    match_stmt: &StmtMatch,
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    let subject = lower_expr(
        &match_stmt.subject,
        in_function,
        class_name,
        imports,
        signatures,
    )?;
    let mut cases = Vec::with_capacity(match_stmt.cases.len());
    for case in &match_stmt.cases {
        let pattern = lower_pattern(&case.pattern, in_function, class_name, imports, signatures)?;
        let guard = case
            .guard
            .as_deref()
            .map(|g| lower_expr(g, in_function, class_name, imports, signatures))
            .transpose()?;
        let body = lower_body(
            &case.body,
            aliases,
            in_loop,
            in_function,
            in_finally,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?;
        cases.push(HirMatchCase {
            pattern,
            guard,
            body,
        });
    }
    Ok(HirStmt::Match { subject, cases })
}

/// PEP 634-636 (#381, PR-21): lowers a `ruff_python_ast::Pattern` into an
/// `HirPattern`. See `HirPattern`'s own doc comment for the per-variant
/// mapping. Unsupported pattern sub-shapes (e.g. a non-literal in
/// `MatchValue`, a non-`Expr::Name` class in `MatchClass`) produce `C0001`.
pub(super) fn lower_pattern(
    pattern: &Pattern,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirPattern, Diagnostic> {
    match pattern {
        Pattern::MatchValue(value) => {
            let expr = lower_expr(&value.value, in_function, class_name, imports, signatures)?;
            match &expr {
                HirExpr::IntLiteral(_)
                | HirExpr::FloatLiteral(_)
                | HirExpr::StringLiteral(_)
                | HirExpr::BoolLiteral(_) => Ok(HirPattern::Literal(expr)),
                _ => Err(unsupported(
                    "only a literal value pattern is supported so far",
                    pycc_ast::expr_range(&value.value),
                )),
            }
        }
        Pattern::MatchSingleton(singleton) => match singleton.value {
            Singleton::True => Ok(HirPattern::Singleton(true)),
            Singleton::False => Ok(HirPattern::Singleton(false)),
            Singleton::None => Ok(HirPattern::NoneSingleton),
        },
        Pattern::MatchSequence(seq) => {
            let has_star = seq
                .patterns
                .iter()
                .any(|p| matches!(p, Pattern::MatchStar(_)));
            if has_star {
                let mut rest: Option<String> = None;
                let mut fixed: Vec<HirPattern> = Vec::new();
                for p in &seq.patterns {
                    if let Pattern::MatchStar(star) = p {
                        rest = star.name.as_ref().map(|n| n.id.to_string());
                    } else {
                        fixed.push(lower_pattern(
                            p,
                            in_function,
                            class_name,
                            imports,
                            signatures,
                        )?);
                    }
                }
                Ok(HirPattern::SequenceStar(fixed, rest))
            } else {
                let sub_patterns = seq
                    .patterns
                    .iter()
                    .map(|p| lower_pattern(p, in_function, class_name, imports, signatures))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(HirPattern::Sequence(sub_patterns))
            }
        }
        Pattern::MatchMapping(mapping) => {
            let mut pairs = Vec::with_capacity(mapping.keys.len());
            for (key, pat) in mapping.keys.iter().zip(mapping.patterns.iter()) {
                let key_expr = lower_expr(key, in_function, class_name, imports, signatures)?;
                let val_pat = lower_pattern(pat, in_function, class_name, imports, signatures)?;
                pairs.push((key_expr, val_pat));
            }
            let rest = mapping.rest.as_ref().map(|n| n.id.to_string());
            Ok(HirPattern::Mapping(pairs, rest))
        }
        Pattern::MatchClass(class) => {
            let Expr::Name(name) = class.cls.as_ref() else {
                return Err(unsupported(
                    "only a bare-name class pattern is supported so far",
                    pycc_ast::expr_range(&class.cls),
                ));
            };
            let class_name = name.id.to_string();
            let positional = class
                .arguments
                .patterns
                .iter()
                .map(|p| lower_pattern(p, in_function, None, imports, signatures))
                .collect::<Result<Vec<_>, _>>()?;
            let keyword = class
                .arguments
                .keywords
                .iter()
                .map(|kw| {
                    lower_pattern(&kw.pattern, in_function, None, imports, signatures)
                        .map(|p| (kw.attr.to_string(), p))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(HirPattern::Class {
                class_name,
                positional,
                keyword,
            })
        }
        Pattern::MatchStar(_) => Err(unsupported(
            "a `*` pattern is only valid inside a sequence pattern",
            0..0,
        )),
        Pattern::MatchAs(as_pat) => match (&as_pat.pattern, &as_pat.name) {
            (None, None) => Ok(HirPattern::Wildcard),
            (None, Some(name)) => Ok(HirPattern::Capture(name.id.to_string())),
            (Some(inner), name) => {
                let inner_pat = lower_pattern(inner, in_function, class_name, imports, signatures)?;
                let name = name.as_ref().map(|n| n.id.to_string()).unwrap_or_default();
                Ok(HirPattern::As(Box::new(inner_pat), name))
            }
        },
        Pattern::MatchOr(or_pat) => {
            let sub = or_pat
                .patterns
                .iter()
                .map(|p| lower_pattern(p, in_function, class_name, imports, signatures))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(HirPattern::Or(sub))
        }
    }
}
