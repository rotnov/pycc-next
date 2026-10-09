//! Which inherited bodies need a receiver-exact copy (#1337, D-254): the
//! differing set `Δ(C, D)` and the copy-needed least fixpoint.
//!
//! For a receiver class `C` and a body `D.m` that runs with a `C` receiver,
//! the body needs a copy for `C` when any arm holds:
//!
//! - **reach** -- it names, through `self.x`/`cls.x`, a member of `Δ(C, D)`,
//!   or a body that itself needs a copy for `C` (directly or through
//!   `super()`);
//! - **super** -- a `super().x` in it selects a different class for `C` than
//!   for `D` (the cooperative diamond);
//! - **identity** -- it tests `isinstance(self, T)` for a `T` in `C`'s MRO but
//!   not `D`'s, constructs through `cls(...)` or `type(self)(...)`, or is a
//!   dataclass-synthesized
//!   `__repr__`/`__eq__` (both observe the exact class);
//! - **escape** -- `Δ(C, D)` is non-empty and the body uses its receiver
//!   bare (returns it, passes it on, prints it, compares it).
//!
//! `Δ(C, D)` is the set of members visible from `D` whose resolution for
//! `C` differs from their resolution for `D`, plus the `<identity>`
//! pseudo-member when the class of a `D`-typed value is observable: `D`'s
//! resolved `__repr__`/`__eq__` is dataclass-synthesized, or some identity
//! observer in the program (`isinstance`/`issubclass` target, class pattern,
//! `except` handler) names a class in `MRO(C) \ MRO(D)`.
//!
//! The arms only ever turn `false` into `true`, so the iteration is a
//! monotone least fixpoint over a finite set and terminates.

use std::collections::{BTreeSet, HashMap};

use pycc_hir::{HirClassDef, first_definer};

use super::facts::BodyFacts;

/// Class-level hooks that are never copied: they run at class-creation time
/// or on the class object, never with an instance receiver.
const NEVER_COPIED: [&str; 3] = ["__init_subclass__", "__class_getitem__", "__set_name__"];

/// One copy the pass must materialize: the body `origin_name`, defined by
/// `origin_class`, compiled for `receiver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedCopy {
    pub(crate) receiver: String,
    pub(crate) origin_class: String,
    pub(crate) origin_name: String,
}

/// The class-table view the planner reads.
pub(crate) struct Tables<'a> {
    /// Every class, by name.
    pub(crate) classes: HashMap<&'a str, &'a HirClassDef>,
    /// The user classes (not seeded builtin exceptions), in program order.
    pub(crate) user_classes: Vec<&'a HirClassDef>,
    /// Per method item name, the receiver facts of its body (merged across
    /// redefinitions).
    pub(crate) facts: &'a HashMap<String, BodyFacts>,
    /// Every class an identity observer names anywhere in the program.
    pub(crate) observers: &'a BTreeSet<String>,
}

/// A candidate body: an own, receiver-taking item of an ancestor class.
struct Candidate<'a> {
    class: &'a HirClassDef,
    member: &'a str,
    name: &'a str,
}

impl<'a> Tables<'a> {
    fn class(&self, name: &str) -> Option<&'a HirClassDef> {
        self.classes.get(name).copied()
    }

    fn is_user(&self, name: &str) -> bool {
        self.user_classes.iter().any(|c| c.name == name)
    }

    fn resolve(&self, receiver: &'a HirClassDef, member: &str) -> Option<&'a str> {
        first_definer(receiver, member, &|n| self.class(n)).map(|c| c.name.as_str())
    }

    /// Whether instances of `class` can exist, so copies for it can run.
    fn is_instantiable(&self, class: &HirClassDef) -> bool {
        if class.is_abstract || class.is_protocol || class.type_param.is_some() {
            return false;
        }
        // A user exception class that inherits `Exception`'s constructor is
        // never materialized as a plain object (Part 3 of #541); one with its
        // own constructor is.
        if class.exception_type_tag.is_some() {
            let init_owner = self.resolve(class, "__init__");
            return init_owner.is_some_and(|owner| self.is_user(owner));
        }
        true
    }

    /// Every own item of `class` implementing `member` with a receiver.
    fn bodies_for(&self, class: &'a HirClassDef, member: &str) -> Vec<&'a str> {
        let mut out: Vec<&'a str> = Vec::new();
        out.extend(
            class
                .methods
                .iter()
                .filter(|(n, _)| n == member)
                .map(|(_, m)| m.as_str()),
        );
        for prop in class.properties.iter().filter(|p| p.name == member) {
            out.push(prop.getter.as_str());
            out.extend(prop.setter.as_deref());
        }
        out.extend(
            class
                .class_methods
                .iter()
                .filter(|(n, _)| n == member)
                .map(|(_, m)| m.as_str()),
        );
        out
    }

    /// Every own receiver-taking body of `class`, in declaration order per
    /// kind, excluding abstract stubs and the class-level hooks.
    fn candidates_of(&self, class: &'a HirClassDef) -> Vec<Candidate<'a>> {
        let mut out = Vec::new();
        let mut push = |member: &'a str, name: &'a str| {
            if NEVER_COPIED.contains(&member)
                || class.abstract_methods.iter().any(|a| a == member)
                || !self.facts.contains_key(name)
            {
                return;
            }
            out.push(Candidate {
                class,
                member,
                name,
            });
        };
        for (member, name) in &class.methods {
            push(member, name);
        }
        for prop in &class.properties {
            push(&prop.name, &prop.getter);
            if let Some(setter) = &prop.setter {
                push(&prop.name, setter);
            }
        }
        for (member, name) in &class.class_methods {
            push(member, name);
        }
        out
    }

    /// The class `super().member` selects in a body compiled for `receiver`
    /// whose defining class is `anchor` (WI-4): the first class after
    /// `anchor` in `receiver`'s MRO that binds `member`. `__init__` skips a
    /// D-225 implicit constructor on the first pass (#966).
    pub(crate) fn super_target(
        &self,
        receiver: &'a HirClassDef,
        anchor: &str,
        member: &str,
    ) -> Option<&'a HirClassDef> {
        let pos = receiver.mro.iter().position(|c| c == anchor)?;
        let after: Vec<&'a HirClassDef> = receiver.mro[pos + 1..]
            .iter()
            .filter_map(|n| self.class(n))
            .collect();
        let find = |skip_implicit: bool| {
            after.iter().copied().find(|c| {
                !(skip_implicit && c.implicit_object_init) && pycc_hir::binds_member(c, member)
            })
        };
        find(member == "__init__").or_else(|| find(false))
    }

    /// `Δ(C, D)`'s member part, and whether it contains `<identity>`.
    fn differing(&self, c: &'a HirClassDef, d: &'a HirClassDef) -> (BTreeSet<String>, bool) {
        let mut visible: BTreeSet<&str> = BTreeSet::new();
        for class in d.mro.iter().filter_map(|n| self.class(n)) {
            visible.extend(class.methods.iter().map(|(n, _)| n.as_str()));
            visible.extend(class.properties.iter().map(|p| p.name.as_str()));
            visible.extend(class.static_methods.iter().map(|(n, _)| n.as_str()));
            visible.extend(class.class_methods.iter().map(|(n, _)| n.as_str()));
            visible.extend(class.class_attrs.iter().map(|(n, _, _)| n.as_str()));
        }
        let members: BTreeSet<String> = visible
            .into_iter()
            .filter(|x| self.resolve(c, x) != self.resolve(d, x))
            .map(String::from)
            .collect();
        let synthesized_observer = ["__repr__", "__eq__"].iter().any(|dunder| {
            self.resolve(d, dunder)
                .and_then(|owner| self.class(owner))
                .is_some_and(|owner| owner.is_dataclass)
        });
        let observed = c
            .mro
            .iter()
            .filter(|n| !d.mro.contains(n))
            .any(|n| self.observers.contains(n));
        (
            members,
            c.name != d.name && (synthesized_observer || observed),
        )
    }

    /// The copies needed for every instantiable user class, in program
    /// order of receivers and declaration order of bodies.
    pub(crate) fn plan(&self) -> Vec<PlannedCopy> {
        let mut planned = Vec::new();
        for &c in &self.user_classes {
            if self.is_instantiable(c) {
                planned.extend(self.plan_receiver(c));
            }
        }
        planned
    }

    fn plan_receiver(&self, c: &'a HirClassDef) -> Vec<PlannedCopy> {
        let ancestors: Vec<&'a HirClassDef> = c.mro[1..]
            .iter()
            .filter(|n| self.is_user(n))
            .filter_map(|n| self.class(n))
            .filter(|d| !d.is_protocol && d.type_param.is_none())
            .collect();
        let candidates: Vec<Candidate<'a>> = ancestors
            .iter()
            .flat_map(|d| self.candidates_of(d))
            .collect();
        if candidates.is_empty() {
            return Vec::new();
        }
        let deltas: HashMap<&str, (BTreeSet<String>, bool)> = ancestors
            .iter()
            .map(|d| (d.name.as_str(), self.differing(c, d)))
            .collect();
        let mut need: HashMap<&str, bool> = candidates.iter().map(|b| (b.name, false)).collect();
        let needs =
            |name: &str, need: &HashMap<&str, bool>| need.get(name).copied().unwrap_or(false);
        loop {
            let mut changed = false;
            for b in &candidates {
                if need[b.name] {
                    continue;
                }
                let facts = &self.facts[b.name];
                let (delta, identity) = &deltas[b.class.name.as_str()];
                let reach = facts.self_refs.iter().any(|x| {
                    delta.contains(x)
                        || self
                            .resolve(c, x)
                            .and_then(|owner| self.class(owner))
                            .filter(|owner| owner.name != c.name)
                            .is_some_and(|owner| {
                                self.bodies_for(owner, x).iter().any(|e| needs(e, &need))
                            })
                });
                let via_super = facts.super_refs.iter().any(|x| {
                    let for_c = self.super_target(c, &b.class.name, x);
                    let for_d = self.super_target(b.class, &b.class.name, x);
                    for_c.map(|t| &t.name) != for_d.map(|t| &t.name)
                        || for_c
                            .is_some_and(|t| self.bodies_for(t, x).iter().any(|e| needs(e, &need)))
                });
                let identity_arm = facts
                    .receiver_type_tests
                    .iter()
                    .any(|t| c.mro.contains(t) && !b.class.mro.contains(t))
                    || facts.constructs_via_receiver
                    || (b.class.is_dataclass && matches!(b.member, "__repr__" | "__eq__"));
                let escape = (!delta.is_empty() || *identity) && facts.bare_use;
                if reach || via_super || identity_arm || escape {
                    need.insert(b.name, true);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Only a body that actually runs compiled for `C` is materialized:
        // `C`'s resolved member (the primary copy), or a `super()` target
        // reached from `C`'s own bodies or from another materialized copy.
        let mut runs: BTreeSet<&str> = candidates
            .iter()
            .filter(|b| need[b.name] && self.resolve(c, b.member) == Some(b.class.name.as_str()))
            .map(|b| b.name)
            .collect();
        let own: Vec<Candidate<'a>> = self.candidates_of(c);
        let mut worklist: Vec<(&'a HirClassDef, &'a str)> = own
            .iter()
            .map(|b| (b.class, b.name))
            .chain(
                candidates
                    .iter()
                    .filter(|b| runs.contains(b.name))
                    .map(|b| (b.class, b.name)),
            )
            .collect();
        while let Some((anchor, name)) = worklist.pop() {
            for x in &self.facts[name].super_refs {
                let Some(target) = self.super_target(c, &anchor.name, x) else {
                    continue;
                };
                for e in self.bodies_for(target, x) {
                    if needs(e, &need) && runs.insert(e) {
                        worklist.push((target, e));
                    }
                }
            }
        }
        candidates
            .iter()
            .filter(|b| runs.contains(b.name))
            .map(|b| PlannedCopy {
                receiver: c.name.clone(),
                origin_class: b.class.name.clone(),
                origin_name: b.name.to_string(),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
