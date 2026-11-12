//! Which native parameters of a specialization a loop computes with
//! natively ([`natively_used`]).
//!
//! A specialization (`pycc_ast::specialize_observed`) converts its
//! specialized arguments at every call, and the generic body it replaces
//! already takes an exact `int` or `str` operand's common case inline
//! (`pycc_codegen`'s fast paths, tagged locals). So a native parameter
//! pays only where an operation repeated by a loop has nothing but native
//! operands: `text1[x1] == text2[y1]` with `x1` and `y1` native `int`s, or
//! `result[ai]` with `ai = -filter_unit` a native `int` because the
//! parameter is. Where its other operand is an object (`src[pos]` for an
//! object `pos`, a character of it `in` an object set), the operation
//! converts that operand to a native value, or the native value to an
//! object, on every iteration, which the generic body does not do at all:
//! tomli's `skip_chars` ran 1,325 more instructions per call specialized
//! for `src: str` than its generic lift.

use crate::{MirExpr, MirItem, MirModule, MirStmt};
use pycc_hir::Ty;
use std::collections::BTreeSet;

/// The parameters among `params` of `function` (a module-level function of
/// `module`) whose native value a loop of its body computes with: an
/// operation inside a loop reads the parameter, or a native local computed
/// from it (`ai = -filter_unit`, then `result[ai]`), with only native
/// operands beside it. Such an operation is arithmetic, a comparison, an
/// index or a slice, a builtin's argument (`len`, a `str` method), a
/// comprehension's source, or the sequence a `for` walks. A read that is
/// boxed, passed to a call, stored in a container or meets an object
/// operand does not count, and neither does a read outside a loop that
/// feeds no such local: it runs once per call, as the conversion does.
/// Empty when `module` has no such function.
#[must_use]
pub fn natively_used(module: &MirModule, function: &str, params: &[String]) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for item in &module.items {
        if let MirItem::Function {
            name,
            params: declared,
            body,
            ..
        } = item
            && name == function
        {
            let mut walk = Walk::default();
            walk.stmts(body, false);
            // A local computed natively from other natives is as good as
            // they are: what feeds a used local is used too.
            loop {
                let before = walk.used.len();
                for (target, sources) in &walk.feeds {
                    if walk.used.contains(target) {
                        walk.used.extend(sources.iter().cloned());
                    }
                }
                if walk.used.len() == before {
                    break;
                }
            }
            used.extend(
                params
                    .iter()
                    .filter(|param| {
                        walk.used.contains(*param)
                            && declared
                                .iter()
                                .any(|(name, ty)| name == *param && is_native(ty))
                    })
                    .cloned(),
            );
        }
    }
    used
}

/// Whether a value of type `ty` is held natively.
fn is_native(ty: &Ty) -> bool {
    !matches!(ty, Ty::Object)
}

#[derive(Default)]
struct Walk {
    /// The native names an operation inside a loop computes with.
    used: BTreeSet<String>,
    /// Each native local assigned a native computation, with the native
    /// names that computation reads.
    feeds: Vec<(String, BTreeSet<String>)>,
}

impl Walk {
    fn stmts(&mut self, body: &[MirStmt], in_loop: bool) {
        for stmt in body {
            self.stmt(stmt, in_loop);
        }
    }

    fn stmt(&mut self, stmt: &MirStmt, in_loop: bool) {
        // A `for` over a native sequence walks it natively, item by item.
        match stmt {
            MirStmt::ForList { list: walked, .. }
            | MirStmt::ForSet { set: walked, .. }
            | MirStmt::ForDict { dict: walked, .. } => {
                self.used.insert(walked.clone());
            }
            MirStmt::Assign { target, value } if is_native_operand(value) => {
                let mut sources = BTreeSet::new();
                reads(value, &mut sources);
                self.feeds.push((target.clone(), sources));
            }
            _ => {}
        }
        let loops = matches!(
            stmt,
            MirStmt::While { .. }
                | MirStmt::ForRange { .. }
                | MirStmt::ForList { .. }
                | MirStmt::ForObject { .. }
                | MirStmt::ForDict { .. }
                | MirStmt::ForSet { .. }
                | MirStmt::ForGen { .. }
        );
        // A `while` test runs on every iteration; a `for`'s iterable and a
        // `range`'s bounds once.
        let repeated = in_loop || matches!(stmt, MirStmt::While { .. });
        let comprehension = matches!(
            stmt,
            MirStmt::ListCompAssign { .. }
                | MirStmt::SetCompAssign { .. }
                | MirStmt::DictCompAssign { .. }
        );
        for expr in stmt.exprs() {
            if comprehension
                && let MirExpr::Name { name, ty } = expr
                && is_native(ty)
            {
                // The comprehension's source.
                self.used.insert(name.clone());
            }
            self.expr(expr, repeated || comprehension);
        }
        for body in stmt.bodies() {
            self.stmts(body, in_loop || loops);
        }
    }

    fn expr(&mut self, expr: &MirExpr, in_loop: bool) {
        let in_loop = in_loop || matches!(expr, MirExpr::Comprehension(_));
        let children = expr.children();
        if in_loop && computes(expr) && children.iter().all(|child| is_native_operand(child)) {
            for child in &children {
                if let MirExpr::Name { name, .. } = child {
                    self.used.insert(name.clone());
                }
            }
        }
        for child in children {
            self.expr(child, in_loop);
        }
    }
}

/// The native names `expr`, a native value, computes from: itself if it
/// is a name, else the names among the operands of its native operations.
fn reads(expr: &MirExpr, out: &mut BTreeSet<String>) {
    match expr {
        MirExpr::Name { name, ty } if is_native(ty) => {
            out.insert(name.clone());
        }
        _ if computes(expr) => {
            let children = expr.children();
            if children.iter().all(|child| is_native_operand(child)) {
                for child in children {
                    reads(child, out);
                }
            }
        }
        _ => {}
    }
}

/// Whether `expr` computes with its operands in place, rather than boxing,
/// storing or passing them on.
fn computes(expr: &MirExpr) -> bool {
    matches!(
        expr,
        MirExpr::BinOp { .. }
            | MirExpr::Compare { .. }
            | MirExpr::CompareChain { .. }
            | MirExpr::BoolOp { .. }
            | MirExpr::Not(_)
            | MirExpr::IfExp { .. }
            | MirExpr::Subscript { .. }
            | MirExpr::BufferGet { .. }
            | MirExpr::Slice { .. }
            | MirExpr::RtCall(_)
            | MirExpr::Comprehension(_)
    )
}

/// Whether `expr` is a native value no conversion produced: not an object,
/// and not an object converted for this use.
fn is_native_operand(expr: &MirExpr) -> bool {
    !matches!(
        expr,
        MirExpr::ObjectIndex(..) | MirExpr::ObjectUnbox(..) | MirExpr::ObjectCoerce(..)
    ) && is_native(&expr.ty())
}

#[cfg(test)]
mod tests;
