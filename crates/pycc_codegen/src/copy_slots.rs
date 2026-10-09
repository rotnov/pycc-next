//! When an inherited-method copy's function-pointer slot is bound (#1337,
//! D-254).
//!
//! A `MirItem::Function` binds its `fnptr_` slot at its own position among
//! the module's statements, as a `def` does in CPython, and a call made
//! before that position raises `NameError`. `pycc_types`' copy pass appends
//! each receiver-exact copy of an inherited method *after* every item, so
//! binding a copy at its own position would leave its slot null for the
//! whole module body. The copy is part of its origin's `def` -- the body a
//! subclass instance runs when the base class's `def` executed -- so its slot
//! is bound together with the origin's, occurrence by occurrence: the `k`-th
//! definition of `D.m` binds the `k`-th copy of it for every receiver.

use std::collections::{HashMap, HashSet};

use pycc_mir::{MirItem, MirModule, inherited_copy_origin};

/// Function ordinals (positions among the module's `MirItem::Function`
/// items, the order `function_defs_in_order` records) whose slot store
/// moves.
#[derive(Debug, Default)]
pub(crate) struct CopySlots {
    /// Origin ordinal to the copy ordinals bound with it.
    with_origin: HashMap<usize, Vec<usize>>,
    /// Every copy ordinal: bound with its origin, never at its own position.
    copies: HashSet<usize>,
}

impl CopySlots {
    pub(crate) fn new(mir: &MirModule) -> Self {
        let class_of = |name: &str| {
            mir.class_defs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, d)| d)
        };
        let mut slots = CopySlots::default();
        let mut occurrences: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut copy_seen: HashMap<&str, usize> = HashMap::new();
        let functions = mir.items.iter().filter_map(|item| match item {
            MirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        });
        for (ordinal, name) in functions.enumerate() {
            let Some(copy) = inherited_copy_origin(name, &class_of) else {
                occurrences.entry(name).or_default().push(ordinal);
                continue;
            };
            let k = copy_seen.entry(name).or_insert(0);
            let origin = occurrences
                .get(copy.origin_name.as_str())
                .and_then(|seen| seen.get(*k))
                .copied()
                .expect("a copy follows every occurrence of the item it copies");
            *k += 1;
            slots.with_origin.entry(origin).or_default().push(ordinal);
            slots.copies.insert(ordinal);
        }
        slots
    }

    /// Whether the function at `ordinal` is a copy, bound with its origin.
    pub(crate) fn is_copy(&self, ordinal: usize) -> bool {
        self.copies.contains(&ordinal)
    }

    /// The copies bound together with the function at `ordinal`.
    pub(crate) fn bound_with(&self, ordinal: usize) -> &[usize] {
        self.with_origin.get(&ordinal).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
#[path = "copy_slots_tests.rs"]
mod tests;
