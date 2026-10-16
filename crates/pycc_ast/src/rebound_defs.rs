//! Module-level names bound both by a `def` and by another binding (issue
//! #1).
//!
//! Compiled code calls a module-level function through the slot its `def`
//! statements set, and reads any other module-level binding of a name from
//! the name's variable. A name bound both ways (`def f` and `f = g`, `for f
//! in ...`) has two homes that disagree: a compiled
//! function calling `f` misses the rebinding, or reads a variable the
//! `def` never set (`NameError`). Until one storage serves both, such a
//! binding is refused (`C0001`); in non-strict mode it runs in CPython,
//! and so do the `def`s of its name (`src/fallback.rs`), so every compiled
//! use of the name reads the module's namespace.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, ModModule, Pattern, Stmt};
use ruff_text_size::{Ranged, TextRange};
use std::collections::BTreeSet;

/// Every module-scope binding in `module` of a name a module-level `def`
/// binds, other than the `def`s themselves: the name and the range of the
/// binding target, in source order. A `class` or an import of the same
/// name, and a `del`, are left to the checks that already own them.
pub fn rebound_def_names(module: &ModModule) -> Vec<(String, std::ops::Range<u32>)> {
    let defs: BTreeSet<&str> = module
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(def.name.as_str()),
            _ => None,
        })
        .collect();
    if defs.is_empty() {
        return Vec::new();
    }
    let mut find = Find {
        defs: &defs,
        found: Vec::new(),
    };
    for stmt in &module.body {
        if !matches!(stmt, Stmt::FunctionDef(_)) {
            find.visit_stmt(stmt);
        }
    }
    find.found
}

struct Find<'d> {
    defs: &'d BTreeSet<&'d str>,
    found: Vec<(String, std::ops::Range<u32>)>,
}

impl Find<'_> {
    fn check(&mut self, name: &str, range: TextRange) {
        if self.defs.contains(name) {
            self.found
                .push((name.to_string(), range.start().into()..range.end().into()));
        }
    }
}

impl<'a> Visitor<'a> for Find<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // Another scope: only its name binds here, and a nested `def`
            // or a `class` is checked elsewhere; so is an import of a
            // `def`'s name (a foreign import shadowing it, a project import
            // rebound).
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) | Stmt::Import(_) | Stmt::ImportFrom(_) => {}
            Stmt::Try(try_stmt) => {
                for handler in &try_stmt.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(name) = &h.name {
                        self.check(name.as_str(), name.range());
                    }
                }
                visitor::walk_stmt(self, stmt);
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) if name.ctx == ExprContext::Store => {
                self.check(name.id.as_str(), name.range);
            }
            // Their own scopes.
            Expr::Lambda(_)
            | Expr::ListComp(_)
            | Expr::SetComp(_)
            | Expr::DictComp(_)
            | Expr::Generator(_) => {}
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        match pattern {
            Pattern::MatchAs(p) => {
                if let Some(name) = &p.name {
                    self.check(name.as_str(), name.range());
                }
            }
            Pattern::MatchStar(p) => {
                if let Some(name) = &p.name {
                    self.check(name.as_str(), name.range());
                }
            }
            Pattern::MatchMapping(p) => {
                if let Some(name) = &p.rest {
                    self.check(name.as_str(), name.range());
                }
            }
            _ => {}
        }
        visitor::walk_pattern(self, pattern);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Vec<String> {
        let parsed = ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax();
        rebound_def_names(&parsed)
            .into_iter()
            .map(|(name, range)| {
                format!(
                    "{name}@{}",
                    &source[range.start as usize..range.end as usize]
                )
            })
            .collect()
    }

    #[test]
    fn every_kind_of_module_scope_binding_of_a_def_name_is_found() {
        let source = "def f(): pass\nf = 1\nfor f in []: pass\nif 1:\n    f += 1\n\
                      try:\n    pass\nexcept E as f:\n    pass\n\
                      (f := 2)\nwith a as f: pass\nmatch x:\n    case [*f]: pass\n";
        assert_eq!(found(source).len(), 7, "{:?}", found(source));
    }

    #[test]
    fn redefinitions_other_scopes_and_other_names_are_not() {
        let source = "def f(): pass\ndef f(): pass\ndef g():\n    f = 1\nclass C:\n    f = 2\n\
                      h = lambda f: f\nxs = [f for f in []]\ndel f\nother = 3\n\
                      import f\nfrom m import f\n";
        assert!(found(source).is_empty(), "{:?}", found(source));
    }

    #[test]
    fn a_module_without_a_def_has_nothing_to_find() {
        assert!(found("f = 1\nf = 2\n").is_empty());
    }
}
