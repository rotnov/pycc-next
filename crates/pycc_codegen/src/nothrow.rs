//! Functions that can never raise, so a call to one needs no
//! pending-exception test after it (D-173's guard, `emit_expr`).
//!
//! A deliberately small whitelist: a function qualifies when it is defined
//! once, is not a generator, takes only `int`/`float`/`bool` parameters, and
//! its body is built only from `if`, `return`, expression statements and
//! assignments to its own parameters, over expressions that cannot raise --
//! literals, reads of its parameters (always bound), `int` `+ - * & | ^`
//! (arbitrary precision), `float` `+ - *` of two floats, comparisons of two
//! ints or two floats, `not`, `and`/`or`, conditional expressions, and calls
//! of functions that qualify. The set is the greatest fixed point, so
//! recursion qualifies (`fib`): a call can then only fail by exhausting the
//! stack or memory, which aborts the process in compiled code either way.

use pycc_mir::{BinOpKind, CmpOpKind, MirExpr, MirItem, MirModule, MirStmt, Ty};
use std::collections::{HashMap, HashSet};

/// The names of the functions of `mir` that can never raise.
pub(crate) fn nothrow_functions(mir: &MirModule) -> HashSet<String> {
    let mut defs: HashMap<&str, usize> = HashMap::new();
    for item in &mir.items {
        if let MirItem::Function { name, .. } = item {
            *defs.entry(name.as_str()).or_default() += 1;
        }
    }
    let mut candidates: Vec<(&str, HashSet<&str>, &[MirStmt])> = mir
        .items
        .iter()
        .filter_map(|item| {
            let MirItem::Function {
                name,
                params,
                return_ty,
                body,
            } = item
            else {
                return None;
            };
            let scalar = |ty: &Ty| matches!(ty, Ty::Int | Ty::Float | Ty::Bool);
            (defs[name.as_str()] == 1
                && (scalar(return_ty) || *return_ty == Ty::None)
                && params.iter().all(|(_, ty)| scalar(ty)))
            .then(|| {
                (
                    name.as_str(),
                    params.iter().map(|(p, _)| p.as_str()).collect(),
                    body.as_slice(),
                )
            })
        })
        .collect();
    loop {
        let set: HashSet<&str> = candidates.iter().map(|(name, ..)| *name).collect();
        let before = candidates.len();
        candidates.retain(|(_, params, body)| {
            let cx = Cx { params, set: &set };
            body.iter().all(|stmt| cx.stmt(stmt))
        });
        if candidates.len() == before {
            return candidates.into_iter().map(|(name, ..)| name.to_string()).collect();
        }
    }
}

struct Cx<'a> {
    params: &'a HashSet<&'a str>,
    set: &'a HashSet<&'a str>,
}

fn scalar(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool)
}

fn int_like(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Bool)
}

impl Cx<'_> {
    fn stmt(&self, stmt: &MirStmt) -> bool {
        match stmt {
            MirStmt::Return(None) | MirStmt::NoOp => true,
            MirStmt::Return(Some(value)) | MirStmt::ExprStmt(value) => self.expr(value),
            MirStmt::Assign { target, value } => {
                self.params.contains(target.as_str()) && self.expr(value)
            }
            MirStmt::If { test, body, orelse } => {
                self.expr(test) && body.iter().chain(orelse).all(|stmt| self.stmt(stmt))
            }
            _ => false,
        }
    }

    fn expr(&self, expr: &MirExpr) -> bool {
        match expr {
            MirExpr::IntLiteral(_) | MirExpr::FloatLiteral(_) | MirExpr::BoolLiteral(_) => true,
            MirExpr::Name { name, ty } => scalar(ty) && self.params.contains(name.as_str()),
            MirExpr::BinOp {
                op,
                left,
                right,
                ty,
            } => {
                let types_ok = match ty {
                    Ty::Int => {
                        matches!(
                            op,
                            BinOpKind::Add
                                | BinOpKind::Sub
                                | BinOpKind::Mul
                                | BinOpKind::BitAnd
                                | BinOpKind::BitOr
                                | BinOpKind::BitXor
                        ) && int_like(&left.ty())
                            && int_like(&right.ty())
                    }
                    Ty::Float => {
                        matches!(op, BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul)
                            && left.ty() == Ty::Float
                            && right.ty() == Ty::Float
                    }
                    _ => false,
                };
                types_ok && self.expr(left) && self.expr(right)
            }
            MirExpr::Compare { op, left, right, .. } => {
                let (l, r) = (left.ty(), right.ty());
                !matches!(op, CmpOpKind::Is | CmpOpKind::IsNot)
                    && matches!(
                        op,
                        CmpOpKind::Eq
                            | CmpOpKind::NotEq
                            | CmpOpKind::Lt
                            | CmpOpKind::LtE
                            | CmpOpKind::Gt
                            | CmpOpKind::GtE
                    )
                    && ((int_like(&l) && int_like(&r)) || (l == Ty::Float && r == Ty::Float))
                    && self.expr(left)
                    && self.expr(right)
            }
            MirExpr::Not(inner) => scalar(&inner.ty()) && self.expr(inner),
            MirExpr::BoolOp { left, right, .. } => {
                scalar(&left.ty()) && scalar(&right.ty()) && self.expr(left) && self.expr(right)
            }
            MirExpr::IfExp {
                test,
                body,
                orelse,
                ty,
            } => scalar(ty) && scalar(&test.ty()) && self.expr(test) && self.expr(body) && self.expr(orelse),
            MirExpr::Call { callee, args, .. } => {
                self.set.contains(callee.as_str())
                    && args.iter().all(|arg| scalar(&arg.ty()) && self.expr(arg))
            }
            _ => false,
        }
    }
}
