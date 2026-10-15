//! `sorted(xs)`, `xs.sort()`, `min(...)`, `max(...)` over class instances
//! -- or with `key=`/`reverse=` (`pycc_ast::desugar::keys`'s
//! `__pycc_*_keyed` pairs) over instance keys -- (`MirExpr::InstanceOrder`),
//! and `x in s` for a `set[C]` (`MirExpr::InstanceSetContains`), as
//! `pycc_types::instance_order` and `pycc_types::set_element` admitted
//! them.

use super::*;
use crate::{InstanceOrderExpr, OrderKind};

/// The method `a OP b` calls for instances of `class` (`OP` is `<` unless
/// `greater`), and whether it is the reflected one (`b.__gt__(a)` for
/// `a < b`), as CPython's rich comparison falls back.
fn order_callee(
    class: &str,
    greater: bool,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> Option<(String, bool)> {
    let class_def = classes.get(class)?;
    let (own, reflected) = if greater {
        ("__gt__", "__lt__")
    } else {
        ("__lt__", "__gt__")
    };
    for (method, swap) in [(own, false), (reflected, true)] {
        let found = class_def.mro.iter().find_map(|mro_class| {
            classes.get(mro_class.as_str()).and_then(|mro_def| {
                mro_def
                    .methods
                    .iter()
                    .find(|(name, _)| name == method)
                    .map(|(_, mangled)| (mro_class, mangled.clone()))
            })
        });
        if let Some((owner, mangled)) = found {
            let callee = crate::receiver_exact::exact_callee(class, owner, mangled, scopes, classes);
            return Some((callee, swap));
        }
    }
    None
}

/// The ordering node; `key` is the type compared (an instance).
#[allow(clippy::too_many_arguments)]
fn order(
    kind: OrderKind,
    list: MirExpr,
    target: Option<MirExpr>,
    reverse: Option<MirExpr>,
    key: &Ty,
    elem: Ty,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> MirExpr {
    let Ty::Instance(class) = key else {
        unreachable!("an ordering compares instances")
    };
    let (callee, swap) = order_callee(class, kind == OrderKind::Max, classes, scopes)
        .expect("pycc_types admits an ordering only of instances whose class defines `__lt__` or `__gt__`");
    let keyed = *key != elem || list.ty() != Ty::List(Box::new(elem.clone()));
    MirExpr::InstanceOrder(Box::new(InstanceOrderExpr {
        kind,
        list,
        target,
        reverse,
        keyed,
        callee,
        swap,
        elem,
    }))
}

/// The `(key, element)` types of a list of pairs whose key is an instance.
fn instance_pairs(ty: &Ty) -> Option<(Ty, Ty)> {
    let Ty::List(pair) = ty else {
        return None;
    };
    match pair.as_ref() {
        Ty::Tuple(items) if items.len() == 2 && matches!(items[0], Ty::Instance(_)) => {
            Some((items[0].clone(), items[1].clone()))
        }
        _ => None,
    }
}

/// `sorted(xs)`, `min(xs)`, `max(xs)` of a list of instances,
/// `min(a, b, ...)`/`max(a, b, ...)` of instances, or the keyed forms over
/// instance keys; `Err(args)` for any other call.
pub(crate) fn lower_order_call(
    callee: &str,
    mut args: Vec<MirExpr>,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> Result<MirExpr, Vec<MirExpr>> {
    let tys: Vec<Ty> = args.iter().map(MirExpr::ty).collect();
    let keyed_kind = match callee {
        "__pycc_sorted_keyed" => Some(OrderKind::Sorted),
        "__pycc_sort_keyed" => Some(OrderKind::Sort),
        "__pycc_min_keyed" => Some(OrderKind::Min),
        "__pycc_max_keyed" => Some(OrderKind::Max),
        _ => None,
    };
    if let Some(kind) = keyed_kind {
        let pairs_at = usize::from(kind == OrderKind::Sort);
        let Some((key, elem)) = tys.get(pairs_at).and_then(instance_pairs) else {
            return Err(args);
        };
        let reverse = matches!(kind, OrderKind::Sorted | OrderKind::Sort).then(|| {
            args.pop().expect("a keyed sort's reverse flag")
        });
        let list = args.pop().expect("the pairs");
        let target = args.pop();
        return Ok(order(kind, list, target, reverse, &key, elem, classes, scopes));
    }
    let kind = match callee {
        "sorted" => OrderKind::Sorted,
        "min" => OrderKind::Min,
        "max" => OrderKind::Max,
        _ => return Err(args),
    };
    let elem = match tys.as_slice() {
        [Ty::List(elem)] if matches!(**elem, Ty::Instance(_)) => (**elem).clone(),
        [first @ Ty::Instance(_), rest @ ..]
            if kind != OrderKind::Sorted && !rest.is_empty() && rest.iter().all(|t| t == first) =>
        {
            first.clone()
        }
        _ => return Err(args),
    };
    let list = if args.len() == 1 {
        args.into_iter().next().expect("one argument")
    } else {
        MirExpr::ListLiteral(args)
    };
    Ok(order(kind, list, None, None, &elem.clone(), elem, classes, scopes))
}

/// `xs.sort()` of a list of instances; `Err(base)` for any other call.
pub(crate) fn lower_sort_method(
    base: MirExpr,
    method: &str,
    args: &[HirExpr],
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> Result<MirExpr, MirExpr> {
    match base.ty() {
        Ty::List(elem) if method == "sort" && args.is_empty() && matches!(*elem, Ty::Instance(_)) => {
            Ok(order(OrderKind::Sort, base, None, None, &elem.clone(), *elem, classes, scopes))
        }
        _ => Err(base),
    }
}

/// `element in set` / `not in` for a `set[C]` of instances; `Err` with the
/// operands for any other test.
pub(crate) fn lower_set_contains(
    op: CmpOpKind,
    element: MirExpr,
    set: MirExpr,
    classes: &HashMap<String, HirClassDef>,
    scopes: &[HashMap<String, Ty>],
) -> Result<MirExpr, (MirExpr, MirExpr)> {
    let negate = match op {
        CmpOpKind::In => false,
        CmpOpKind::NotIn => true,
        _ => return Err((element, set)),
    };
    let elem_ty = match set.ty() {
        Ty::Set(elem) | Ty::FrozenSet(elem) if matches!(*elem, Ty::Instance(_)) => *elem,
        _ => return Err((element, set)),
    };
    if element.ty() != elem_ty {
        return Err((element, set));
    }
    let Some(ops) = super::set_ops::lower_set_element_ops(&elem_ty, scopes, classes) else {
        return Err((element, set));
    };
    Ok(MirExpr::InstanceSetContains {
        element: Box::new(element),
        set: Box::new(set),
        ops: Box::new(ops),
        negate,
    })
}
