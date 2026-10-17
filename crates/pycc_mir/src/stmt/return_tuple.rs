//! A returned tuple converted to the function's declared tuple type,
//! split out of `lower_stmt`.

use super::super::MirExpr;
use pycc_hir::Ty;

/// Non-strict mode: a tuple display with an object element is the CPython
/// tuple (`obj_arith::object_display`); returned from a function declared
/// to return a native tuple of scalars, it is the native tuple instead,
/// each object element converted with a run-time type check
/// (`MirExpr::ObjectCoerce`, admitted by `pycc_types`'
/// `non_strict::admits_tuple_display_return`).
///
/// A native tuple returned from a function declared to return a tuple with
/// `object` slots (`-> tuple[int, Any]`, v0.5) has those elements boxed: a
/// display's elements in place, any other value through
/// `MirExpr::TupleRebox` (`pycc_types::object_box::tuple_reboxes`).
pub(super) fn coerce_returned_tuple(value: MirExpr, declared: Option<Ty>) -> MirExpr {
    if let (Some(Ty::Tuple(slots)), Ty::Tuple(items)) = (&declared, value.ty())
        && items.len() == slots.len()
        && *items != **slots
        && slots.contains(&Ty::Object)
    {
        return match value {
            MirExpr::TupleLiteral(elements) => MirExpr::TupleLiteral(
                elements
                    .into_iter()
                    .zip(slots.iter())
                    .map(|(element, slot)| crate::object_box::box_into(element, slot))
                    .collect(),
            ),
            other => MirExpr::TupleRebox(Box::new(other), Box::new(Ty::Tuple(slots.clone()))),
        };
    }
    let (Some(Ty::Tuple(tys)), MirExpr::ObjDisplay { kind, elements }) = (&declared, &value) else {
        return value;
    };
    if *kind != crate::ObjDisplayKind::Tuple || elements.len() != tys.len() {
        return value;
    }
    let MirExpr::ObjDisplay { elements, .. } = value else {
        unreachable!("matched above")
    };
    let elements = elements
        .into_iter()
        .zip(tys.iter())
        .map(|(element, ty)| {
            let element = match element {
                MirExpr::ObjectBox(inner)
                    if inner.ty() == *ty
                        || matches!((inner.ty(), ty), (Ty::Tuple(items), Ty::VarTuple(e))
                            if pycc_hir::tuple_fits_var_tuple(&items, e)) =>
                {
                    *inner
                }
                other => other,
            };
            if element.ty() == Ty::Object && *ty != Ty::Object {
                MirExpr::ObjectCoerce(Box::new(element), Box::new(ty.clone()))
            } else {
                element
            }
        })
        .collect();
    MirExpr::TupleLiteral(elements)
}
