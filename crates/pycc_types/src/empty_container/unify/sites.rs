//! The empty containers of a module as numbered sites: each is replaced by
//! a placeholder name while [`super::infer`] runs, then restored rewritten
//! to the type its site variable resolved to.

use super::*;

/// The prefix of a numbered site's placeholder name: it starts with a
/// digit, so no source name can equal it.
const SITE_PREFIX: &str = "0site";

/// The site number a placeholder name carries.
pub(super) fn site_number(name: &str) -> Option<usize> {
    name.strip_prefix(SITE_PREFIX)?.parse().ok()
}

/// Replaces every empty container of `hir` with a numbered placeholder
/// name, returning the originals by number.
pub(super) fn number_sites(
    hir: &mut HirModule,
    functions: &HashMap<String, (Vec<Ty>, Ty)>,
) -> Vec<(SiteKind, HirExpr)> {
    let mut sites: Vec<(SiteKind, HirExpr)> = Vec::new();
    let mut visit_stmt = |stmt: &mut HirStmt| {
        stmt_exprs_mut(stmt, &mut |expr| {
            walk_expr_mut(expr, &mut |node| {
                let Some(kind) = site_kind_in(node, functions) else {
                    return false;
                };
                let placeholder = HirExpr::Name(format!("{SITE_PREFIX}{}", sites.len()));
                sites.push((kind, std::mem::replace(node, placeholder)));
                true
            });
        });
    };
    for item in &mut hir.items {
        match item {
            HirItem::Function { body, .. } => all_stmts_mut(body, &mut visit_stmt),
            HirItem::TopLevelStmt(stmt) => {
                all_stmts_mut(std::slice::from_mut(stmt), &mut visit_stmt)
            }
        }
    }
    sites
}

/// Puts each numbered site back: its resolved typed node, or the original.
pub(super) fn restore_sites(hir: &mut HirModule, sites: Vec<(SiteKind, HirExpr)>, types: &[Option<Ty>]) {
    let mut originals: Vec<Option<(SiteKind, HirExpr)>> = sites.into_iter().map(Some).collect();
    let mut visit_stmt = |stmt: &mut HirStmt| {
        stmt_exprs_mut(stmt, &mut |expr| {
            walk_expr_mut(expr, &mut |node| {
                let HirExpr::Name(name) = node else {
                    return false;
                };
                let Some(number) = site_number(name) else {
                    return false;
                };
                let Some((kind, original)) = originals.get_mut(number).and_then(Option::take)
                else {
                    return true;
                };
                *node = match (kind, types.get(number).cloned().flatten()) {
                    // Non-strict mode (#3): a list of objects is a CPython list.
                    (SiteKind::List, Some(Ty::List(elem)))
                        if crate::non_strict::is_non_strict()
                            && crate::empty_container::holds_objects(&elem) =>
                    {
                        HirExpr::ObjectList(Vec::new())
                    }
                    (SiteKind::List, Some(Ty::List(elem))) => HirExpr::EmptyList(*elem),
                    // ... and a dict or set of objects a CPython one
                    // (`pycc_hir::empty_object_constructor`).
                    (SiteKind::Dict, Some(Ty::Dict(pair)))
                        if (pair.0 == Ty::Object || pair.1 == Ty::Object)
                            && crate::non_strict::is_non_strict() =>
                    {
                        HirExpr::Call {
                            callee: "dict".to_string(),
                            args: Vec::new(),
                        }
                    }
                    (SiteKind::Set, Some(Ty::Set(elem)))
                        if *elem == Ty::Object && crate::non_strict::is_non_strict() =>
                    {
                        HirExpr::Call {
                            callee: "set".to_string(),
                            args: Vec::new(),
                        }
                    }
                    (SiteKind::Dict, Some(Ty::Dict(pair))) => HirExpr::EmptyDict(pair),
                    (SiteKind::Set, Some(Ty::Set(elem))) => HirExpr::EmptySet(*elem),
                    // `frozenset()` of the resolved element type: the
                    // frozenset of an empty set of it.
                    (SiteKind::FrozenSet, Some(Ty::FrozenSet(elem))) => HirExpr::Call {
                        callee: "frozenset".to_string(),
                        args: vec![HirExpr::EmptySet(*elem)],
                    },
                    _ => original,
                };
                true
            });
        });
    };
    for item in &mut hir.items {
        match item {
            HirItem::Function { body, .. } => all_stmts_mut(body, &mut visit_stmt),
            HirItem::TopLevelStmt(stmt) => {
                all_stmts_mut(std::slice::from_mut(stmt), &mut visit_stmt)
            }
        }
    }
}

/// The empty container `expr` is, when it is one: a display, or a
/// zero-argument `list()`/`dict()`/`set()` call of the builtin (not of a
/// module function of that name).
fn site_kind_in(expr: &HirExpr, functions: &HashMap<String, (Vec<Ty>, Ty)>) -> Option<SiteKind> {
    match expr {
        HirExpr::ListLiteral(elements) if elements.is_empty() => Some(SiteKind::List),
        HirExpr::DictLiteral(pairs) if pairs.is_empty() => Some(SiteKind::Dict),
        HirExpr::SetLiteral(elements) if elements.is_empty() => Some(SiteKind::Set),
        HirExpr::Call { callee, args } if args.is_empty() && !functions.contains_key(callee) => {
            match callee.as_str() {
                "list" => Some(SiteKind::List),
                "dict" => Some(SiteKind::Dict),
                "set" => Some(SiteKind::Set),
                "frozenset" => Some(SiteKind::FrozenSet),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether `hir` has an empty container anywhere this pass looks.
pub(in crate::empty_container) fn has_sites(hir: &HirModule) -> bool {
    let mut copy = hir.clone();
    !number_sites(&mut copy, &HashMap::default()).is_empty()
}
