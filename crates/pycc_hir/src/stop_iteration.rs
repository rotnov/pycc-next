//! Which generators a caught `StopIteration` can come from.
//!
//! `e.value` of `except StopIteration as e:` is the return value of the
//! generator whose `next()`, `send()` or `throw()` raised it. Its static type
//! is that generator's return type, found from the `try` body: every
//! generator operand of such a call directly in the body (not in a function
//! it calls -- a `StopIteration` leaving a generator's body is turned into
//! `RuntimeError`, PEP 479, and one leaving an ordinary function passes
//! through it unchanged, but the body cannot tell which). `pycc_types` and
//! `pycc_mir` ask the same question, each with its own way of typing an
//! operand, so the walk lives here.

use crate::{FStringPart, HirExpr, HirStmt, Ty};

/// The generator operands of every `next(g)` (without a default),
/// `g.send(v)` and `g.throw(e)` in `body`.
pub fn stop_iteration_sources(body: &[HirStmt]) -> Vec<&HirExpr> {
    let mut out = Vec::new();
    for stmt in body {
        stmt_sources(stmt, &mut out);
    }
    out
}

fn stmt_sources<'a>(stmt: &'a HirStmt, out: &mut Vec<&'a HirExpr>) {
    match stmt {
        HirStmt::ExprStmt(expr)
        | HirStmt::Assign { value: expr, .. }
        | HirStmt::AnnAssign {
            value: Some(expr), ..
        }
        | HirStmt::Return(Some(expr))
        | HirStmt::DictSet { value: expr, .. } => expr_sources(expr, out),
        HirStmt::AttrSet { base, value, .. } => {
            expr_sources(base, out);
            expr_sources(value, out);
        }
        HirStmt::If { test, body, orelse } => {
            expr_sources(test, out);
            body.iter().for_each(|stmt| stmt_sources(stmt, out));
            orelse.iter().for_each(|stmt| stmt_sources(stmt, out));
        }
        HirStmt::While { test, body } => {
            expr_sources(test, out);
            body.iter().for_each(|stmt| stmt_sources(stmt, out));
        }
        HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => {
            body.iter().for_each(|stmt| stmt_sources(stmt, out));
        }
        HirStmt::LoopElse { loop_stmt, orelse } => {
            stmt_sources(loop_stmt, out);
            orelse.iter().for_each(|stmt| stmt_sources(stmt, out));
        }
        HirStmt::Try {
            body,
            orelse,
            finalbody,
            ..
        }
        | HirStmt::TryStar {
            body,
            orelse,
            finalbody,
            ..
        } => {
            for stmt in body.iter().chain(orelse).chain(finalbody) {
                stmt_sources(stmt, out);
            }
        }
        _ => {}
    }
}

fn expr_sources<'a>(expr: &'a HirExpr, out: &mut Vec<&'a HirExpr>) {
    match expr {
        HirExpr::Call { callee, args } => {
            if callee == "next" && args.len() == 1 {
                out.push(&args[0]);
            }
            args.iter().for_each(|arg| expr_sources(arg, out));
        }
        HirExpr::MethodCall { base, method, args } => {
            if matches!(method.as_str(), "send" | "throw") {
                out.push(base);
            }
            expr_sources(base, out);
            args.iter().for_each(|arg| expr_sources(arg, out));
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            expr_sources(left, out);
            expr_sources(right, out);
        }
        HirExpr::UnaryOp { operand, .. } => expr_sources(operand, out),
        HirExpr::AttrGet { base, .. } => expr_sources(base, out),
        HirExpr::ListLiteral(elements) | HirExpr::TupleLiteral(elements) => {
            elements
                .iter()
                .for_each(|element| expr_sources(element, out));
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    expr_sources(inner, out);
                }
            }
        }
        _ => {}
    }
}

/// The type of `e.value` for a `StopIteration` caught around a body whose
/// [`stop_iteration_sources`] are typed `sources`: the one return type
/// their generators share (`None` when every one returns nothing), or an
/// error message when there is no generator to take it from or they
/// disagree.
pub fn stop_iteration_value_ty(sources: &[Ty]) -> Result<Ty, String> {
    let mut found: Option<Ty> = None;
    for ty in sources {
        let Ty::Generator(sig) = ty else { continue };
        match &found {
            None => found = Some(sig.2.clone()),
            Some(seen) if *seen == sig.2 => {}
            Some(seen) => {
                return Err(format!(
                    "`.value` of this `StopIteration` may be a `{}` or a `{}` -- the \
                     generators its `try` body resumes return different types",
                    seen.name(),
                    sig.2.name()
                ));
            }
        }
    }
    found.ok_or_else(|| {
        "`.value` of a caught `StopIteration` is supported only when its `try` body \
         itself calls `next()`, `send()` or `throw()` on a generator"
            .to_string()
    })
}
