//! The statement form of a comprehension that collects CPython objects,
//! split out of `lower_stmt`.

use super::super::{HirClassDef, MirExpr, MirStmt, lower_expr};
use super::lower_stmt;
use pycc_hir::{HirStmt, Ty};
use std::collections::HashMap;

/// Non-strict mode: `target = [elt for var in iter]` (or a set, or a dict
/// with an object key or value) whose element is a CPython object builds a CPython container
/// ([`crate::MirComprehension::collects_objects`]), which only the
/// expression form emits -- so the statement lowers to an assignment of
/// that expression, whose loop variable stays out of the enclosing scope as
/// in CPython.
#[allow(clippy::too_many_arguments)]
pub(super) fn object_comp_assign(
    target: &str,
    var: &str,
    iter: &pycc_hir::CompIter,
    cond: Option<&pycc_hir::HirExpr>,
    elt: pycc_hir::CompElt,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> Option<MirStmt> {
    let comp = pycc_hir::HirExpr::Comprehension(Box::new(pycc_hir::HirComprehension {
        var: var.to_string(),
        iter: iter.clone(),
        cond: cond.cloned(),
        elt,
    }));
    let lowered = lower_expr(&comp, scopes, classes, current_class);
    let MirExpr::Comprehension(c) = &lowered else {
        return None;
    };
    if !c.collects_objects() && !matches!(c.source, crate::CompSource::Object(_)) {
        return None;
    }
    let assign = HirStmt::Assign {
        target: target.to_string(),
        value: comp,
    };
    Some(lower_stmt(&assign, scopes, classes, current_class))
}
