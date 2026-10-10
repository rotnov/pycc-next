//! Whether a program can hold a CPython object without importing one
//! (non-strict mode, #3), so a native build embeds an interpreter for it.
//!
//! Outside a foreign import, `Ty::Object` enters a program only through a
//! *declared* slot: a function or method signature (a missing annotation or
//! `Any` in non-strict mode), a class attribute, or an annotated assignment.
//! A private helper's inferred `object` parameter or a display built from
//! objects needs an object from one of those first. So the walk reads every
//! declared type and every annotated assignment, at any depth.

use crate::{HirItem, HirModule, HirStmt, Ty};

/// Whether `ty` is or contains the opaque CPython object.
fn mentions_object(ty: &Ty) -> bool {
    match ty {
        Ty::Object => true,
        Ty::List(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) | Ty::Optional(inner) => {
            mentions_object(inner)
        }
        Ty::Dict(pair) => mentions_object(&pair.0) || mentions_object(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(mentions_object),
        _ => false,
    }
}

/// Whether some declared slot of `hir` is (or contains) `object`.
pub fn declares_object_slot(hir: &HirModule) -> bool {
    hir.items.iter().any(|item| match item {
        HirItem::Function {
            params,
            return_ty,
            body,
            ..
        } => {
            params.iter().any(|(_, ty)| mentions_object(ty))
                || mentions_object(return_ty)
                || body_annotates_object(body)
        }
        HirItem::TopLevelStmt(stmt) => body_annotates_object(std::slice::from_ref(stmt)),
    }) || hir
        .class_defs
        .iter()
        .any(|(_, class)| class.attrs.iter().any(|(_, ty)| mentions_object(ty)))
}

fn body_annotates_object(body: &[HirStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        HirStmt::AnnAssign { annotation, .. } => mentions_object(annotation),
        HirStmt::If { body, orelse, .. } => {
            body_annotates_object(body) || body_annotates_object(orelse)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_annotates_object(body),
        HirStmt::LoopElse { loop_stmt, orelse } => {
            body_annotates_object(std::slice::from_ref(loop_stmt.as_ref()))
                || body_annotates_object(orelse)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|case| body_annotates_object(&case.body)),
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
            body_annotates_object(body)
                || handlers.iter().any(|h| body_annotates_object(&h.body))
                || body_annotates_object(orelse)
                || body_annotates_object(finalbody)
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_inside_containers_is_found() {
        assert!(mentions_object(&Ty::Object));
        assert!(mentions_object(&Ty::List(Box::new(Ty::Object))));
        assert!(mentions_object(&Ty::Dict(Box::new((Ty::Str, Ty::Object)))));
        assert!(mentions_object(&Ty::Tuple(Box::new(vec![
            Ty::Int,
            Ty::Object
        ]))));
        assert!(!mentions_object(&Ty::Dict(Box::new((Ty::Str, Ty::Int)))));
        assert!(!mentions_object(&Ty::Int));
    }
}
