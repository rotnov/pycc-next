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
//!
//! Every class statement is a node of its own, two of one name included:
//! a name a module binds more than once (two class statements, or a class
//! statement and any other binding) is ambiguous -- which statement a base
//! naming it reads depends on run-time order -- so a layout named so, or
//! whose closure reaches such a name, does not stand.

use pycc_ast::{Expr, Stmt};
use std::collections::BTreeSet;

/// One examined class statement.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ClassNode {
    /// The dotted name it binds (`pkg.mod.C`).
    name: String,
    /// Its bases, each by the dotted name it resolves to (`None`: not
    /// resolved).
    bases: Vec<Option<String>>,
    /// Whether its body is off the layout allowlist.
    opaque: bool,
    /// The names its body binds at class level.
    names: BTreeSet<String>,
}

/// The examined class statements, each a node (dotted names: `pkg.mod.C`;
/// `.C` for a module built alone).
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ClassGraph {
    nodes: Vec<ClassNode>,
    /// The dotted names a module binds more than once, a class statement
    /// among them (the module doc).
    ambiguous: BTreeSet<String>,
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
        for name in super::exc_escape::ambiguous_names(body) {
            self.ambiguous.insert(format!("{qual}.{name}"));
        }
        for stmt in body {
            let Stmt::ClassDef(class) = stmt else {
                continue;
            };
            let names = super::exc_layout::class_names(class);
            self.nodes.push(ClassNode {
                name: format!("{qual}.{}", class.name.as_str()),
                bases: class.bases().iter().map(resolve).collect(),
                opaque: names.is_none(),
                names: names.unwrap_or_default(),
            });
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
        if self.ambiguous.contains(class) {
            return true;
        }
        // The names reached, and the statements reached through them.
        let mut reached: BTreeSet<&str> = BTreeSet::from([class]);
        let mut done: Vec<bool> = vec![false; self.nodes.len()];
        loop {
            let mut grew = false;
            for (at, node) in self.nodes.iter().enumerate() {
                if done[at] {
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
                    || self.ambiguous.contains(&node.name)
                {
                    return true;
                }
                done[at] = true;
                grew |= reached.insert(&node.name);
            }
            if !grew {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClassGraph;
    use std::collections::BTreeSet;

    fn graph(source: &str) -> ClassGraph {
        let module = pycc_parser::parse_all(source).expect("parses");
        ClassGraph::of_module(&module.body)
    }

    fn fields(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// A plain subclass leaves the layout standing; one of several bases,
    /// a body off the allowlist or a field binding withdraws it.
    #[test]
    fn the_closure_tests_each_subclass() {
        let code = fields(&["code"]);
        let base = "class A(Exception):\n    pass\n";
        let ok = graph(&format!(
            "{base}class B(A):\n    pass\nclass C(B):\n    x = 1\n"
        ));
        assert!(!ok.subclass_conflicts(".A", &code));
        for sub in [
            "class B(A):\n    pass\nclass C(B, OSError):\n    pass\n",
            "class B(A):\n    def __getattr__(self, n):\n        return n\n",
            "class B(A):\n    code = 1\n",
        ] {
            assert!(
                graph(&format!("{base}{sub}")).subclass_conflicts(".A", &code),
                "{sub}"
            );
        }
    }

    /// Every class statement is a node, and a name bound more than once
    /// (another class statement, or any other binding, a nested `global`
    /// one included) withdraws a layout reached through it or named so.
    #[test]
    fn a_name_bound_twice_is_ambiguous() {
        let code = fields(&["code"]);
        let base = "class A(Exception):\n    pass\n";
        for rebind in [
            "class B(A, OSError):\n    pass\nclass B(Exception):\n    pass\n",
            "class B(A):\n    pass\nB = 1\n",
            "class B(A):\n    pass\ndef f():\n    global B\n    B = 2\n",
            "class B(A):\n    pass\nfor B in ():\n    pass\n",
            "class B(A):\n    pass\nfrom m import B\n",
            "class B(A):\n    pass\ntry:\n    pass\nexcept ImportError as B:\n    pass\n",
        ] {
            assert!(
                graph(&format!("{base}{rebind}")).subclass_conflicts(".A", &code),
                "{rebind}"
            );
        }
        assert!(graph(&format!("{base}A = 3\n")).subclass_conflicts(".A", &code));
    }
}
