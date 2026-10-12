//! The scope analysis: every binding of every name, and which names are
//! typed arrays, ints, floats, tuples of known length or class instances.

use super::*;

/// Collects every binding of every name in `body` (nested statement
/// bodies included).
pub(super) fn collect_bindings<'a>(
    body: impl Iterator<Item = &'a HirStmt>,
    out: &mut Vec<(&'a str, Binding<'a>)>,
) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, value } => out.push((target, Binding::Value(value))),
            HirStmt::AnnAssign {
                target,
                annotation,
                value,
                ..
            } => {
                out.push((target, Binding::Annotated(annotation)));
                if let Some(value) = value {
                    out.push((target, Binding::Value(value)));
                }
            }
            HirStmt::ForRange { var, body, .. } => {
                out.push((var, Binding::Int));
                collect_bindings(body.iter(), out);
            }
            HirStmt::ForList { var, list, body } => {
                out.push((var, Binding::Row(list)));
                collect_bindings(body.iter(), out);
            }
            HirStmt::ForObject { var, body, .. } => {
                out.push((var, Binding::Other));
                collect_bindings(body.iter(), out);
            }
            HirStmt::ListCompAssign { target, .. }
            | HirStmt::DictCompAssign { target, .. }
            | HirStmt::SetCompAssign { target, .. } => out.push((target, Binding::Other)),
            HirStmt::If { body, orelse, .. } => {
                collect_bindings(body.iter(), out);
                collect_bindings(orelse.iter(), out);
            }
            HirStmt::While { body, .. } => collect_bindings(body.iter(), out),
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    for name in pattern_names(&case.pattern) {
                        out.push((name, Binding::Other));
                    }
                    collect_bindings(case.body.iter(), out);
                }
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                collect_bindings(body.iter(), out);
                for handler in handlers {
                    if let Some(name) = &handler.name {
                        out.push((name, Binding::Other));
                    }
                    collect_bindings(handler.body.iter(), out);
                }
                collect_bindings(orelse.iter(), out);
                collect_bindings(finalbody.iter(), out);
            }
            HirStmt::Delete { name } => out.push((name, Binding::Other)),
            HirStmt::ForeignImport { bindings, .. } => {
                for (name, _, _) in bindings {
                    out.push((name, Binding::Other));
                }
            }
            HirStmt::LoopElse { loop_stmt, orelse } => {
                collect_bindings(std::iter::once(loop_stmt.as_ref()), out);
                collect_bindings(orelse.iter(), out);
            }
            HirStmt::ExprStmt(_)
            | HirStmt::DictSet { .. }
            | HirStmt::Return(_)
            | HirStmt::AttrSet { .. }
            | HirStmt::Raise { .. }
            | HirStmt::DeleteSlice { .. }
            | HirStmt::DeleteAttr { .. }
            | HirStmt::DeleteItem { .. }
            | HirStmt::Break
            | HirStmt::Continue => {}
        }
        // A walrus binds too; its name is never a typed array.
        for_each_stmt_expr(stmt, &mut |e| collect_walrus(e, out));
    }
}

pub(super) fn collect_walrus<'a>(expr: &'a HirExpr, out: &mut Vec<(&'a str, Binding<'a>)>) {
    if let HirExpr::NamedExpr { name, .. } = expr {
        out.push((name, Binding::Other));
    }
    for child in expr_children(expr) {
        collect_walrus(child, out);
    }
}

pub(super) fn pattern_names(pattern: &crate::HirPattern) -> Vec<&str> {
    use crate::HirPattern as P;
    let mut out = Vec::new();
    match pattern {
        P::Capture(name) => out.push(name.as_str()),
        P::As(inner, name) => {
            out.extend(pattern_names(inner));
            out.push(name.as_str());
        }
        P::Sequence(items) | P::Or(items) => {
            items.iter().for_each(|p| out.extend(pattern_names(p)))
        }
        P::SequenceStar(items, star) => {
            items.iter().for_each(|p| out.extend(pattern_names(p)));
            out.extend(star.as_deref());
        }
        P::Mapping(items, rest) => {
            items.iter().for_each(|(_, p)| out.extend(pattern_names(p)));
            out.extend(rest.as_deref());
        }
        P::Class {
            positional,
            keyword,
            ..
        } => {
            positional.iter().for_each(|p| out.extend(pattern_names(p)));
            keyword
                .iter()
                .for_each(|(_, p)| out.extend(pattern_names(p)));
        }
        P::Wildcard | P::Literal(_) | P::Singleton(_) | P::NoneSingleton => {}
    }
    out
}

/// The expressions a statement evaluates itself (not those of its nested
/// statement bodies).
pub(super) fn for_each_stmt_expr<'a>(stmt: &'a HirStmt, f: &mut dyn FnMut(&'a HirExpr)) {
    match stmt {
        HirStmt::ExprStmt(e) | HirStmt::Assign { value: e, .. } => f(e),
        HirStmt::AnnAssign { value, .. } | HirStmt::Return(value) => {
            if let Some(e) = value {
                f(e);
            }
        }
        HirStmt::If { test, .. } | HirStmt::While { test, .. } => f(test),
        HirStmt::ForRange {
            start, stop, step, ..
        } => {
            f(start);
            f(stop);
            f(step);
        }
        HirStmt::ForObject { iter, .. } => f(iter),
        HirStmt::DictSet { key, value, .. } => {
            f(key);
            f(value);
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        }
        | HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_exprs(iter, f);
            if let Some(c) = cond {
                f(c);
            }
            f(elt);
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_exprs(iter, f);
            if let Some(c) = cond {
                f(c);
            }
            f(key);
            f(value);
        }
        HirStmt::AttrSet { base, value, .. } => {
            f(base);
            f(value);
        }
        HirStmt::Match { subject, cases } => {
            f(subject);
            for case in cases {
                if let Some(g) = &case.guard {
                    f(g);
                }
            }
        }
        HirStmt::Raise { exc, cause } => {
            if let Some(e) = exc {
                f(e);
            }
            if let Some(e) = cause {
                f(e);
            }
        }
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            f(base);
            for e in [start, stop, step].into_iter().flatten() {
                f(e);
            }
        }
        HirStmt::DeleteAttr { base, .. } => f(base),
        HirStmt::DeleteItem { base, key, .. } => {
            f(base);
            f(key);
        }
        HirStmt::ForList { .. }
        | HirStmt::Try { .. }
        | HirStmt::TryStar { .. }
        | HirStmt::Delete { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Break
        | HirStmt::Continue
        | HirStmt::LoopElse { .. } => {}
    }
}

pub(super) fn comp_iter_exprs<'a>(iter: &'a crate::CompIter, f: &mut dyn FnMut(&'a HirExpr)) {
    match iter {
        crate::CompIter::Range { start, stop, step } => {
            f(start);
            f(stop);
            f(step);
        }
        crate::CompIter::Iterable(e) => f(e),
        crate::CompIter::Name(_) => {}
    }
}

/// The scope analysis: which names are typed arrays, ints, floats, tuples
/// of known length and class instances. A fixpoint over "every binding of
/// the name has the property".
pub(super) fn analyze<'a>(
    ctx: &Ctx,
    params: &[(String, Ty)],
    body: impl Iterator<Item = &'a HirStmt>,
    is_module: bool,
) -> Facts {
    let mut bindings: Vec<(&str, Binding<'_>)> = Vec::new();
    collect_bindings(body, &mut bindings);
    let mut by_name: HashMap<&str, Vec<&Binding<'_>>> = HashMap::new();
    for (name, binding) in &bindings {
        by_name.entry(name).or_default().push(binding);
    }
    let mut facts = Facts::default();
    for (name, ty) in params {
        facts.locals.insert(name.clone());
        match ty {
            Ty::NdArray(nd) => {
                facts.nd.insert(name.clone(), *nd);
            }
            Ty::Int => {
                facts.ints.insert(name.clone());
            }
            Ty::Float => {
                facts.floats.insert(name.clone());
            }
            Ty::Tuple(elements) => {
                facts.tuples.insert(name.clone(), elements.len());
            }
            Ty::Instance(class) => {
                facts.instances.insert(name.clone(), class.to_string());
            }
            _ => {}
        }
    }
    let param_names: HashSet<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    for name in by_name.keys() {
        facts.locals.insert((*name).to_string());
    }
    for _round in 0..6 {
        let mut changed = false;
        for (name, list) in &by_name {
            let name = *name;
            // A parameter rebound in the body keeps its declared facts only
            // if every rebinding agrees; handled like any other name, with
            // the parameter's own declaration as one more binding.
            let param_ty = params.iter().find(|(n, _)| n == name).map(|(_, t)| t);
            let annotated_nd = list.iter().find_map(|b| match b {
                Binding::Annotated(Ty::NdArray(nd)) => Some(*nd),
                _ => None,
            });
            let nd = match (annotated_nd, param_ty) {
                (Some(nd), _) => Some(nd),
                (None, Some(Ty::NdArray(nd))) => Some(*nd),
                _ => {
                    let mut agreed: Option<NdTy> = None;
                    let mut ok = param_ty.is_none() || !param_names.contains(name);
                    for b in list {
                        let this = match b {
                            Binding::Value(v) => nd_of_value(ctx, &facts, v),
                            Binding::Row(list) => nd_of_name(ctx, &facts, list)
                                .filter(|nd| nd.ndim == 2)
                                .map(|nd| NdTy {
                                    dtype: nd.dtype,
                                    ndim: 1,
                                }),
                            _ => None,
                        };
                        match (this, agreed) {
                            (None, _) => {
                                if !matches!(b, Binding::Annotated(_)) {
                                    ok = false;
                                }
                            }
                            (Some(t), None) => agreed = Some(t),
                            (Some(t), Some(a)) if t == a => {}
                            _ => ok = false,
                        }
                    }
                    agreed.filter(|_| ok)
                }
            };
            if let Some(nd) = nd
                && facts.nd.get(name) != Some(&nd)
            {
                facts.nd.insert(name.to_string(), nd);
                changed = true;
            }
            let all = |pred: &dyn Fn(&Binding<'_>) -> bool| list.iter().all(|b| pred(b));
            if !facts.ints.contains(name)
                && param_ty.is_none_or(|t| *t == Ty::Int)
                && all(&|b| match b {
                    Binding::Int => true,
                    Binding::Annotated(t) => **t == Ty::Int,
                    Binding::Value(v) => is_int(ctx, &facts, v),
                    Binding::Row(_) | Binding::Other => false,
                })
            {
                facts.ints.insert(name.to_string());
                changed = true;
            }
            if !facts.floats.contains(name)
                && param_ty.is_none_or(|t| *t == Ty::Float)
                && all(&|b| match b {
                    Binding::Annotated(t) => **t == Ty::Float,
                    Binding::Value(v) => is_float(ctx, &facts, v),
                    _ => false,
                })
            {
                facts.floats.insert(name.to_string());
                changed = true;
            }
            if !facts.tuples.contains_key(name) && param_ty.is_none() {
                let lens: Vec<Option<usize>> = list
                    .iter()
                    .map(|b| match b {
                        Binding::Value(v) => tuple_len(ctx, &facts, v),
                        Binding::Annotated(Ty::Tuple(t)) => Some(t.len()),
                        _ => None,
                    })
                    .collect();
                if let Some(Some(first)) = lens.first()
                    && lens.iter().all(|l| *l == Some(*first))
                {
                    facts.tuples.insert(name.to_string(), *first);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    if !is_module {
        for name in facts.nd.keys() {
            let checked = by_name.get(name.as_str()).is_none_or(|list| {
                list.iter().all(|b| match b {
                    Binding::Value(_) | Binding::Annotated(_) => true,
                    Binding::Row(list) => nd_of_name(ctx, &facts, list).is_some(),
                    Binding::Int | Binding::Other => false,
                })
            });
            if checked && facts.locals.contains(name) {
                facts.cache_safe.insert(name.clone());
            }
        }
    }
    facts
}
