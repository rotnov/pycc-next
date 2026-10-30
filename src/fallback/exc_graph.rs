//! v0.10 accept 5: the one graph deciding whether a native exception
//! layout (`exc_layout`) can stand against its subclasses.
//!
//! Nodes are the class statements the planner examines -- directly in a
//! module's body -- of the module being built, or of every module of its
//! package when it is built whole (the module itself included); an edge
//! goes from a class to each base it names, resolved across imports and
//! re-exports. A layout stands only if no class of its transitive subclass
//! closure lists several bases (another may carry a layout of its own, or
//! another solid base), has a body off the layout allowlist
//! (`exc_layout::class_names`), or binds one of the layout's attributes at
//! class level. A base the graph cannot resolve, and a class reached other
//! than by name, are `exc_escape`'s.

use pycc_ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// One examined class statement.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ClassNode {
    /// Its bases, each by the dotted name it resolves to (`None`: not
    /// resolved).
    bases: Vec<Option<String>>,
    /// Whether its body is off the layout allowlist.
    opaque: bool,
    /// The names its body binds at class level.
    names: BTreeSet<String>,
}

/// The examined class statements, by dotted name (`pkg.mod.C`; `.C` for a
/// module built alone).
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ClassGraph {
    nodes: BTreeMap<String, ClassNode>,
}

impl ClassGraph {
    /// Adds the top-level class statements of `body`, module `qual`, each
    /// base resolved by `resolve` (a base expression to the dotted name it
    /// reads).
    pub(super) fn add_module(
        &mut self,
        qual: &str,
        body: &[Stmt],
        resolve: &dyn Fn(&Expr) -> Option<String>,
    ) {
        for stmt in body {
            let Stmt::ClassDef(class) = stmt else {
                continue;
            };
            let names = super::exc_layout::class_names(class);
            self.nodes.insert(
                format!("{qual}.{}", class.name.as_str()),
                ClassNode {
                    bases: class.bases().iter().map(resolve).collect(),
                    opaque: names.is_none(),
                    names: names.unwrap_or_default(),
                },
            );
        }
    }

    /// The graph of a module built alone: a base resolves when it names one
    /// of the module's top-level classes.
    pub(super) fn of_module(body: &[Stmt]) -> Self {
        let classes: BTreeSet<&str> = body
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::ClassDef(class) => Some(class.name.as_str()),
                _ => None,
            })
            .collect();
        let mut graph = Self::default();
        graph.add_module("", body, &|base| match base {
            Expr::Name(name) if classes.contains(name.id.as_str()) => {
                Some(format!(".{}", name.id.as_str()))
            }
            _ => None,
        });
        graph
    }

    /// Whether a layout of class `class` with attributes `fields` may meet
    /// a subclass that does not carry it as CPython would (the module doc).
    pub(super) fn subclass_conflicts(&self, class: &str, fields: &BTreeSet<String>) -> bool {
        let mut reached: BTreeSet<&str> = BTreeSet::from([class]);
        loop {
            let before = reached.len();
            for (name, node) in &self.nodes {
                if reached.contains(name.as_str()) {
                    continue;
                }
                let derives = node
                    .bases
                    .iter()
                    .flatten()
                    .any(|base| reached.contains(base.as_str()));
                if !derives {
                    continue;
                }
                if node.bases.len() > 1
                    || node.opaque
                    || node.names.iter().any(|name| fields.contains(name))
                {
                    return true;
                }
                reached.insert(name);
            }
            if reached.len() == before {
                return false;
            }
        }
    }
}
