//! The element type of an empty list nothing reads (see
//! [`Engine::default_unread_list_elements`]).

use super::store::Shape;
use super::{Engine, SiteKind};
use pycc_hir::Ty;
use crate::fx::HashSet;

impl Engine {
    /// An empty list bound to a name, whose element nothing ever reads,
    /// produces or compares, is a `list[int]`: its element variable's class
    /// holds only the element variables of empty lists (`L = []` rebound to
    /// another `[]` unifies the two) and has no shape, and the list itself
    /// flows only between empty-list sites and names -- at least one name,
    /// and never into a return, an attribute, a call's result or another
    /// expression. No operation of the program can then
    /// observe which element type the list has (`print([])`, `len`,
    /// truthiness and rebinding read none), and the check phase still
    /// validates every use of the name in program order.
    pub(super) fn default_unread_list_elements(&mut self) {
        let names: HashSet<usize> = self.names.values().copied().collect();
        let mut carriers = names.clone();
        carriers.extend(self.sites.iter().flatten().copied());
        let mut elements: Vec<usize> = Vec::new();
        for number in 0..self.sites.len() {
            if self.site_kinds[number] != SiteKind::List {
                continue;
            }
            let Some(var) = self.sites[number] else {
                continue;
            };
            let root = self.store.find(var);
            if self.store.conflicted[root]
                || !self.class_within(root, &carriers)
                || !self.class_meets(root, &names)
            {
                continue;
            }
            if let Some(Shape::List(element)) = self.store.shape[root].clone() {
                elements.push(element);
            }
        }
        for element in elements {
            let element_root = self.store.find(element);
            if self.store.shape[element_root].is_some() || self.store.conflicted[element_root] {
                continue;
            }
            let list_elements = self.list_elements.clone();
            if self.class_within(element_root, &list_elements) {
                self.store.shape[element_root] = Some(Shape::Known(Ty::Int));
            }
        }
    }

    /// Whether some variable of `root`'s class is in `wanted`.
    fn class_meets(&mut self, root: usize, wanted: &HashSet<usize>) -> bool {
        (0..self.store.parent.len())
            .filter(|&other| self.store.find(other) == root)
            .any(|other| wanted.contains(&other))
    }

    /// Whether every variable of `root`'s class is in `allowed`.
    fn class_within(&mut self, root: usize, allowed: &HashSet<usize>) -> bool {
        (0..self.store.parent.len())
            .filter(|&other| self.store.find(other) == root)
            .all(|other| allowed.contains(&other))
    }
}
