//! Locals that may hold a *borrowed* reference (`heap_rc`): no count
//! traffic on their stores and no release on the way out.
//!
//! `c = bodies[j]` in a hot loop would otherwise increment the new element
//! and decrement the old one on every trip. That is unnecessary exactly when
//! the list the element came from keeps owning it for as long as `c` can be
//! read, which this module proves conservatively for a function:
//!
//! - every binding of the local is `v = p[i]` or `for v in p`, where `p` is a
//!   parameter (the frame owns it for the whole call) the body never
//!   rebinds;
//! - nothing in the body can remove a list element: no item store, `pop`,
//!   `insert` or mutating list builtin, and no call into user code, a
//!   constructor or a CPython object (any of which could reach the same list
//!   through an alias). `len` is the one call admitted.
//!
//! A borrowed local's value is still a borrowed read when used, so storing
//! it anywhere else takes a reference as usual. The check reads the MIR's
//! `Debug` rendering: anything it does not recognize disqualifies, so an
//! unexpected shape costs speed, never safety.

use super::*;
use std::collections::HashSet;

/// MIR shapes that can drop a list element, or run code that might.
const DISQUALIFYING: &[&str] = &[
    "ListSet",
    "ListPop",
    "ListInsert",
    "ListAppend",
    "ListCompAssign",
    "Instantiate",
    "ObjCall",
    "ObjMethodCall",
    "ObjKeywordCall",
    "ObjAttrSet",
    "ObjSetItem",
    "ObjDel",
    "pycc_rt_list_clear",
    "pycc_rt_list_remove",
    "pycc_rt_list_extend",
    "pycc_rt_list_sort",
    "pycc_rt_list_reverse",
    "Raise",
    "Try",
];

/// The locals of a function with parameters `params` and body `body` that
/// may hold borrowed references; `candidates` are its counted locals.
pub(super) fn borrowed_locals<'a>(
    params: &[(String, pycc_mir::Ty)],
    body: &[MirStmt],
    candidates: impl Iterator<Item = &'a String>,
) -> HashSet<String> {
    let text = format!("{body:?}");
    if DISQUALIFYING.iter().any(|shape| text.contains(shape)) || has_call_other_than_len(&text) {
        return HashSet::new();
    }
    let stable_params: Vec<&str> = params
        .iter()
        .filter(|(name, ty)| {
            matches!(ty, pycc_mir::Ty::List(_)) && !text.contains(&format!("target: {name:?}"))
        })
        .map(|(name, _)| name.as_str())
        .collect();
    if stable_params.is_empty() {
        return HashSet::new();
    }
    candidates
        .filter(|name| every_mention_is_admitted(&text, name, &stable_params))
        .cloned()
        .collect()
}

fn has_call_other_than_len(text: &str) -> bool {
    text.match_indices("Call {")
        .any(|(at, _)| !text[at..].starts_with("Call { callee: \"len\""))
}

/// Whether every mention of `name` in `text` is a read (`Name { name: .. }`)
/// or an admitted binding from a stable parameter.
fn every_mention_is_admitted(text: &str, name: &str, stable_params: &[&str]) -> bool {
    let quoted = format!("{name:?}");
    let from_param = |rest: &str, prefix: &str| {
        stable_params
            .iter()
            .any(|param| rest.starts_with(&format!("{prefix}{param:?}")))
    };
    text.match_indices(&quoted).all(|(at, _)| {
        let before = &text[..at];
        let rest = &text[at + quoted.len()..];
        if before.ends_with("Name { name: ") {
            return true;
        }
        if before.ends_with("Assign { target: ") {
            return from_param(rest, ", value: Subscript { base: Name { name: ");
        }
        if before.ends_with("ForList { var: ") {
            // `ForList { var: "v", var_ty: <ty>, list: "p", .. }`
            return rest
                .find(", list: ")
                .is_some_and(|list_at| from_param(&rest[list_at..], ", list: "));
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_mir::{MirExpr, MirStmt, Ty};

    fn name(name: &str, ty: Ty) -> MirExpr {
        MirExpr::Name {
            name: name.to_string(),
            ty,
        }
    }

    fn element_read(var: &str, list: &str) -> MirStmt {
        let body_ty = Ty::Instance(Box::new("Body".to_string()));
        MirStmt::Assign {
            target: var.to_string(),
            value: MirExpr::Subscript {
                base: Box::new(name(list, Ty::List(Box::new(body_ty)))),
                index: Box::new(MirExpr::IntLiteral(0)),
            },
        }
    }

    fn params() -> Vec<(String, Ty)> {
        let body_ty = Ty::Instance(Box::new("Body".to_string()));
        vec![("bodies".to_string(), Ty::List(Box::new(body_ty)))]
    }

    fn candidates() -> Vec<String> {
        vec!["c".to_string()]
    }

    #[test]
    fn an_element_read_from_a_stable_parameter_is_borrowed() {
        let body = vec![element_read("c", "bodies")];
        let found = borrowed_locals(&params(), &body, candidates().iter());
        assert!(found.contains("c"));
    }

    #[test]
    fn a_rebound_parameter_or_a_mutation_or_a_call_disqualifies() {
        let rebinding = vec![
            element_read("c", "bodies"),
            MirStmt::Assign {
                target: "bodies".to_string(),
                value: MirExpr::EmptyList(Ty::Int),
            },
        ];
        assert!(borrowed_locals(&params(), &rebinding, candidates().iter()).is_empty());
        let call = vec![
            element_read("c", "bodies"),
            MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            }),
        ];
        assert!(borrowed_locals(&params(), &call, candidates().iter()).is_empty());
        let other_source = vec![element_read("c", "local_list")];
        assert!(borrowed_locals(&params(), &other_source, candidates().iter()).is_empty());
    }
}
