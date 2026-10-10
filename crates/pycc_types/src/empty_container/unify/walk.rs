//! Mutable walks over HIR statements and expressions, used to number the
//! empty-container sites and to write their resolved types back.

use crate::empty_container::for_each_stmt_mut;
use pycc_hir::{FStringPart, HirExpr, HirStmt};

/// Calls `visit` on every expression of `stmt` (not recursing into an
/// expression's children, nor into nested statement bodies).
pub(crate) fn stmt_exprs_mut(stmt: &mut HirStmt, visit: &mut dyn FnMut(&mut HirExpr)) {
    match stmt {
        HirStmt::Assign { value, .. } => visit(value),
        HirStmt::AnnAssign {
            value: Some(value), ..
        } => visit(value),
        HirStmt::ExprStmt(expr) => visit(expr),
        HirStmt::If { test, .. } | HirStmt::While { test, .. } => visit(test),
        HirStmt::DictSet { key, value, .. } => {
            visit(key);
            visit(value);
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        }
        | HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_mut(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(elt);
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_mut(iter, visit);
            if let Some(cond) = cond {
                visit(cond);
            }
            visit(key);
            visit(value);
        }
        HirStmt::Return(Some(value)) => visit(value),
        HirStmt::AttrSet { value, .. } => visit(value),
        HirStmt::ForObject { iter, .. } => visit(iter),
        _ => {}
    }
}

fn comp_iter_mut(iter: &mut pycc_hir::CompIter, visit: &mut dyn FnMut(&mut HirExpr)) {
    match iter {
        pycc_hir::CompIter::Range { start, stop, step } => {
            visit(start);
            visit(stop);
            visit(step);
        }
        pycc_hir::CompIter::Iterable(iterable) => visit(iterable),
        pycc_hir::CompIter::Name(_) => {}
    }
}

/// Calls `visit` on `expr` and then on every sub-expression, depth first.
/// `visit` returning `true` stops the descent below that node.
pub(crate) fn walk_expr_mut(expr: &mut HirExpr, visit: &mut dyn FnMut(&mut HirExpr) -> bool) {
    if visit(expr) {
        return;
    }
    let mut each = |child: &mut HirExpr| walk_expr_mut(child, visit);
    match expr {
        HirExpr::Call { args, .. }
        | HirExpr::ListLiteral(args)
        | HirExpr::SetLiteral(args)
        | HirExpr::TupleLiteral(args)
        | HirExpr::ObjectList(args)
        | HirExpr::ReceiverClassCall { args }
        | HirExpr::GenericClassInstantiate { args, .. } => args.iter_mut().for_each(&mut each),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            each(left);
            each(right);
        }
        HirExpr::CompareChain { first, links } => {
            each(first);
            for link in links {
                each(&mut link.right);
            }
        }
        HirExpr::UnaryOp { operand, .. } => each(operand),
        HirExpr::IfExp { test, body, orelse } => {
            each(test);
            each(body);
            each(orelse);
        }
        HirExpr::Unpack { value, .. } | HirExpr::NamedExpr { value, .. } => each(value),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    each(inner);
                }
            }
        }
        HirExpr::Subscript { base, index } => {
            each(base);
            each(index);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            each(base);
            for bound in [start, stop, step].into_iter().flatten() {
                each(bound);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(attr) = list.attr_expr_mut() {
                each(attr);
            }
            each(value);
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs {
                each(key);
                each(value);
            }
        }
        HirExpr::DictGetOrDefault { key, default, .. } => {
            each(key);
            each(default);
        }
        HirExpr::SetAdd { value, .. } => each(value),
        HirExpr::AttrGet { base, .. } => each(base),
        HirExpr::MethodCall { base, args, .. } => {
            each(base);
            args.iter_mut().for_each(&mut each);
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => each(call),
        HirExpr::ExprCall { callee, args } => {
            each(callee);
            args.iter_mut().for_each(&mut each);
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            each(call);
            for (_, value) in keywords {
                each(value);
            }
        }
        HirExpr::Comprehension(comp) => {
            comp_iter_mut(&mut comp.iter, &mut each);
            for body_expr in comp.body_exprs_mut() {
                each(body_expr);
            }
        }
        _ => {}
    }
}

/// Every statement of `body` and of its nested bodies, mutably.
pub(super) fn all_stmts_mut(body: &mut [HirStmt], visit: &mut dyn FnMut(&mut HirStmt)) {
    for_each_stmt_mut(body, visit);
}

