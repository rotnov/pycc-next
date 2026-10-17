//! Non-strict mode: instance attributes a class's methods assign outside
//! `__init__`'s own top-level statements (`self.width = ...` in a method
//! that parses a header, or under an `if` in `__init__`).
//!
//! `super::init_slot::collect_init_attrs` types a slot from the first
//! assignment in `__init__`'s top-level body. Untyped classes commonly
//! create attributes anywhere else, which CPython allows, so each such
//! attribute gets an `object` slot: any value stores into it (boxed), and a
//! read before any store raises CPython's `AttributeError`
//! (`pycc_rt::instance`). A name the class already uses for a slot, a
//! method, a property or a class attribute is left alone.

use crate::Ty;
use pycc_ast::{Expr, Stmt};
use pycc_ast::visitor::{self, Visitor};

/// The attribute names assigned through the receiver `receiver` anywhere in
/// `body` (any nesting; not inside a nested `def` or class, whose own scope
/// the receiver is not), in source order.
fn stored_attrs(body: &[Stmt], receiver: &str, out: &mut Vec<String>) {
    struct Stores<'a> {
        receiver: &'a str,
        out: &'a mut Vec<String>,
    }
    impl Stores<'_> {
        fn target(&mut self, target: &Expr) {
            match target {
                Expr::Attribute(attr)
                    if matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == self.receiver) =>
                {
                    let name = attr.attr.to_string();
                    if !self.out.contains(&name) {
                        self.out.push(name);
                    }
                }
                Expr::Tuple(t) => t.elts.iter().for_each(|e| self.target(e)),
                Expr::List(l) => l.elts.iter().for_each(|e| self.target(e)),
                Expr::Starred(s) => self.target(&s.value),
                _ => {}
            }
        }
    }
    impl<'a> Visitor<'a> for Stores<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
                Stmt::Assign(a) => a.targets.iter().for_each(|t| self.target(t)),
                Stmt::AugAssign(a) => self.target(&a.target),
                Stmt::AnnAssign(a) if a.value.is_some() => self.target(&a.target),
                Stmt::For(f) => self.target(&f.target),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut stores = Stores { receiver, out };
    for stmt in body {
        stores.visit_stmt(stmt);
    }
}

/// Adds an `object` slot to `attrs` for every attribute a method of the
/// class body `body` assigns that `taken` does not already name (module
/// doc).
pub(super) fn add_method_attrs(
    body: &[Stmt],
    attrs: &mut Vec<(String, Ty)>,
    taken: &dyn Fn(&str) -> bool,
) {
    let mut names = Vec::new();
    for stmt in body {
        let Stmt::FunctionDef(def) = stmt else {
            continue;
        };
        // A static or class method has no instance receiver.
        if !def.decorator_list.is_empty() {
            continue;
        }
        let Some(receiver) = def
            .parameters
            .posonlyargs
            .iter()
            .chain(def.parameters.args.iter())
            .next()
        else {
            continue;
        };
        stored_attrs(&def.body, receiver.parameter.name.as_str(), &mut names);
    }
    for name in names {
        if !attrs.iter().any(|(held, _)| *held == name) && !taken(&name) {
            attrs.push((name, Ty::Object));
        }
    }
}
