//! Receiver-exact member resolution (#1337, D-254).
//!
//! Every MIR site that resolves a user method by walking a class's MRO finds
//! the item that *defines* the member. When the receiver's static class `C`
//! inherits that item from `D` and `pycc_types`' copy pass compiled a
//! receiver-exact copy of it for `C`, the call must run the copy: it is the
//! body whose `self.m()`, `super()`, class attributes, and type tests were
//! resolved for `C`. This module is the one place that maps "the item an MRO
//! walk found" to "the item to call"; every site routes through it.

use std::collections::HashMap;

use crate::Ty;
use pycc_hir::{HirClassDef, inherited_copy_name};

/// The item a call on a `receiver`-class value runs, given the item
/// `found` that the MRO walk resolved the member to, defined by
/// `owner`. `found` itself unless a copy of it for `receiver` exists in
/// `scopes` (the copy pass only materializes the copies that run).
pub(crate) fn exact_callee(
    receiver: &str,
    owner: &str,
    found: String,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> String {
    if receiver == owner {
        return found;
    }
    let Some(receiver_def) = classes.get(receiver) else {
        return found;
    };
    match inherited_copy_name(receiver_def, owner, &found, &|n| classes.get(n)) {
        Some(copy)
            if scopes
                .iter()
                .any(|s| s.contains_key(&format!("$fn:{copy}"))) =>
        {
            copy
        }
        _ => found,
    }
}

/// The class a `self`-typed receiver expression statically names, for the
/// `super()` arms: a copy's `self` is typed as its receiver, an original
/// body's as its own class.
pub(crate) fn receiver_class(ty: &Ty) -> Option<&str> {
    match ty {
        Ty::Instance(class) => Some(class.as_str()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "receiver_exact_tests.rs"]
mod tests;
