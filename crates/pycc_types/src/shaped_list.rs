//! Erasing [`Ty::ShapedList`] once the module type-checks (v0.8 accept 5).
//!
//! A list display of elements of different types (`[name, index]`) is a
//! `Ty::ShapedList`: the checker admits on it only what is valid for every
//! list of that shape (`pycc_hir::Ty::ShapedList`). Past the check it is
//! the boxed tuple object of its elements, flagged to render as a list
//! (`pycc_mir` builds the display through `__pycc_shaped_list`), so MIR and
//! codegen see a `Ty::Tuple`: every type the resolved module carries is
//! rewritten here.

use crate::empty_container::for_each_stmt_mut;
use crate::empty_container::unify::walk::{stmt_exprs_mut, walk_expr_mut};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, ProtocolMember, Ty};

/// `ty` with every shaped list replaced by its tuple.
pub(crate) fn erase_ty(ty: &mut Ty) {
    match ty {
        Ty::ShapedList(elements) => {
            elements.iter_mut().for_each(erase_ty);
            *ty = Ty::Tuple(elements.clone());
        }
        Ty::List(inner)
        | Ty::Set(inner)
        | Ty::FrozenSet(inner)
        | Ty::VarTuple(inner)
        | Ty::Optional(inner) => erase_ty(inner),
        Ty::Generator(sig) => {
            erase_ty(&mut sig.0);
            erase_ty(&mut sig.1);
            erase_ty(&mut sig.2);
        }
        Ty::Dict(pair) => {
            erase_ty(&mut pair.0);
            erase_ty(&mut pair.1);
        }
        Ty::Tuple(elements) => elements.iter_mut().for_each(erase_ty),
        Ty::Callable(sig) => {
            sig.0.iter_mut().for_each(erase_ty);
            erase_ty(&mut sig.1);
        }
        Ty::Int
        | Ty::Float
        | Ty::Num
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView
        | Ty::NdArray(_) => {}
    }
}

fn erase_expr(expr: &mut HirExpr) {
    walk_expr_mut(expr, &mut |node| {
        match node {
            HirExpr::EmptyList(ty) | HirExpr::EmptySet(ty) => erase_ty(ty),
            HirExpr::EmptyDict(pair) => {
                erase_ty(&mut pair.0);
                erase_ty(&mut pair.1);
            }
            HirExpr::GenericClassInstantiate { type_arg, .. } => erase_ty(type_arg),
            _ => {}
        }
        false
    });
}

fn erase_body(body: &mut [HirStmt]) {
    for_each_stmt_mut(body, &mut |stmt| {
        if let HirStmt::AnnAssign { annotation, .. } = stmt {
            erase_ty(annotation);
        }
        stmt_exprs_mut(stmt, &mut erase_expr);
    });
}

/// Rewrites every type `hir` carries so no `Ty::ShapedList` is left.
pub(crate) fn erase_module(hir: &mut HirModule) {
    for item in &mut hir.items {
        match item {
            HirItem::Function {
                params,
                return_ty,
                body,
                ..
            } => {
                params.iter_mut().for_each(|(_, ty)| erase_ty(ty));
                erase_ty(return_ty);
                erase_body(body);
            }
            HirItem::TopLevelStmt(stmt) => erase_body(std::slice::from_mut(stmt)),
        }
    }
    for (_, class) in &mut hir.class_defs {
        class.attrs.iter_mut().for_each(|(_, ty)| erase_ty(ty));
        class
            .dataclass_fields
            .iter_mut()
            .for_each(|(_, ty)| erase_ty(ty));
        for member in &mut class.protocol_members {
            match member {
                ProtocolMember::Method {
                    param_tys,
                    return_ty,
                    ..
                } => {
                    param_tys.iter_mut().for_each(erase_ty);
                    erase_ty(return_ty);
                }
                ProtocolMember::Attribute { ty, .. } => erase_ty(ty),
            }
        }
    }
    hir.type_aliases.iter_mut().for_each(|(_, ty)| erase_ty(ty));
}

#[cfg(test)]
mod tests {
    use super::erase_ty;
    use pycc_hir::Ty;

    #[test]
    fn a_shaped_list_becomes_its_tuple_at_any_depth() {
        let shaped = Ty::ShapedList(Box::new(vec![Ty::Str, Ty::Int]));
        let mut ty = Ty::List(Box::new(shaped));
        erase_ty(&mut ty);
        assert_eq!(
            ty,
            Ty::List(Box::new(Ty::Tuple(Box::new(vec![Ty::Str, Ty::Int]))))
        );
    }
}
