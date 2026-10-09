//! Which module-level names are bound more than once.
//!
//! [`super::SignatureTable`] is one static, module-wide table, but a
//! module-level name that is bound twice does not have one signature: pycc's
//! runtime dispatch of a redefined `def` is source-order sensitive (issue
//! #22's function-pointer slot), so a call made before the second binding
//! runs the first `def` and a call made after it runs the second. No call
//! site can be matched to "its" binding statically in general -- a call
//! inside a function body executes at a time unrelated to its source
//! position -- so a keyword argument or a default filled from either
//! signature could silently bind to the wrong one. The table therefore
//! excludes every name this module finds bound more than once
//! (`docs/TYPE_SYSTEM.md`, "Keyword arguments and default parameter values
//! on a redefined name").
//!
//! The scan is deliberately conservative: it may count a binding CPython
//! would not (a walrus inside a `lambda` body), which only costs a keyword
//! call or a default fill its binding; it must never miss one. It does not
//! scan function bodies, which is complete only while pycc rejects a
//! `global` declaration: supporting `global` must add its targets here. It
//! also counts `from ... import *` as binding the literal `*` rather than
//! the names it imports, which is complete only while pycc rejects a
//! wildcard import (`import.rs`'s `check_from_import_shape`): accepting one
//! must expand it here.

use std::collections::HashMap;

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Alias, Comprehension, ExceptHandler, Expr, ExprContext, Pattern, Stmt};

/// Counts, for every name, how many binding sites module scope holds for it.
///
/// Module scope is the module body plus the bodies of every compound
/// statement nested in it (`if`, `while`, `for`, `try`, `with`, `match`),
/// which execute in the module's own namespace. A `def` or `class` binds its
/// own name here, and its decorators, defaults and bases are evaluated here
/// too, but its body is a scope of its own and is not scanned.
pub(super) fn binding_counts(body: &[Stmt]) -> HashMap<String, usize> {
    let mut scan = BindingScan::default();
    scan.visit_body(body);
    scan.counts
}

#[derive(Default)]
struct BindingScan {
    counts: HashMap<String, usize>,
}

impl BindingScan {
    fn bind(&mut self, name: &str) {
        *self.counts.entry(name.to_string()).or_default() += 1;
    }

    /// The name an `import` alias binds: `as` name when present, otherwise
    /// the first dotted component (`import a.b` binds `a`).
    fn bind_alias(&mut self, alias: &Alias) {
        let bound = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
        let first = bound.split('.').next().unwrap_or(bound);
        self.bind(first);
    }
}

impl<'a> Visitor<'a> for BindingScan {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                self.bind(def.name.as_str());
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&def.parameters);
            }
            Stmt::ClassDef(def) => {
                self.bind(def.name.as_str());
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &def.arguments {
                    self.visit_arguments(arguments);
                }
            }
            // An annotation without a value (`foo: int`) declares the name
            // but does not bind it.
            Stmt::AnnAssign(ann) if ann.value.is_none() => {}
            Stmt::Import(import) => import.names.iter().for_each(|a| self.bind_alias(a)),
            Stmt::ImportFrom(import) => import.names.iter().for_each(|a| self.bind_alias(a)),
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        // Every assignment-shaped target -- `=`, annotated `=`, `+=`, a
        // `for`/`with` target, a starred or tuple target, `type X = ...`,
        // `del`, and a walrus -- reaches here as a `Name` in store or delete
        // context.
        if let Expr::Name(name) = expr
            && matches!(name.ctx, ExprContext::Store | ExprContext::Del)
        {
            self.bind(name.id.as_str());
        }
        visitor::walk_expr(self, expr);
    }

    fn visit_comprehension(&mut self, comprehension: &'a Comprehension) {
        // A comprehension's own `for` target is local to the comprehension,
        // but a walrus in its iterable or condition binds in module scope.
        self.visit_expr(&comprehension.iter);
        for condition in &comprehension.ifs {
            self.visit_expr(condition);
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(inner) = handler;
        if let Some(name) = &inner.name {
            self.bind(name.as_str());
        }
        visitor::walk_except_handler(self, handler);
    }

    fn visit_pattern(&mut self, pattern: &'a Pattern) {
        // A capture is an `Identifier`, not an `Expr::Name`, so
        // `visit_expr` never sees it.
        let captured = match pattern {
            Pattern::MatchAs(as_pattern) => as_pattern.name.as_ref(),
            Pattern::MatchStar(star_pattern) => star_pattern.name.as_ref(),
            Pattern::MatchMapping(mapping_pattern) => mapping_pattern.rest.as_ref(),
            _ => None,
        };
        if let Some(name) = captured {
            self.bind(name.as_str());
        }
        visitor::walk_pattern(self, pattern);
    }
}

#[cfg(test)]
mod tests {
    use super::binding_counts;

    const DEF: &str = "def foo(a: int = 1) -> None:\n    print(a)\n";

    fn count_of(source: &str, name: &str) -> usize {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        binding_counts(&module.body)
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    #[test]
    fn every_module_scope_binding_form_counts() {
        for (other, name) in [
            (DEF, "foo"),
            ("foo = 3\n", "foo"),
            ("foo: int = 3\n", "foo"),
            ("foo += 1\n", "foo"),
            ("x, *foo = 1, 2\n", "foo"),
            ("import foo\n", "foo"),
            ("import foo.bar\n", "foo"),
            ("import math as foo\n", "foo"),
            ("from math import foo\n", "foo"),
            ("from math import sqrt as foo\n", "foo"),
            ("type foo = int\n", "foo"),
            ("class foo:\n    pass\n", "foo"),
            ("if (foo := 3) > 0:\n    pass\n", "foo"),
            ("xs = [x for x in range(2) if (foo := x)]\n", "foo"),
            ("for foo in range(2):\n    pass\n", "foo"),
            ("with open(\"x\") as foo:\n    pass\n", "foo"),
            (
                "try:\n    pass\nexcept ValueError as foo:\n    pass\n",
                "foo",
            ),
            ("match 1:\n    case foo:\n        pass\n", "foo"),
            ("match [1]:\n    case [*foo]:\n        pass\n", "foo"),
            ("match {}:\n    case {**foo}:\n        pass\n", "foo"),
            ("del foo\n", "foo"),
            ("if True:\n    foo = 3\n", "foo"),
            ("while True:\n    foo = 3\n", "foo"),
            ("@deco(foo := 1)\ndef bar() -> None:\n    return\n", "foo"),
            ("def bar(a: int = (foo := 1)) -> None:\n    return\n", "foo"),
            ("class Bar(Base[(foo := 1)]):\n    pass\n", "foo"),
            ("@deco(foo := 1)\nclass Bar:\n    pass\n", "foo"),
        ] {
            assert_eq!(count_of(&format!("{DEF}{other}"), name), 2, "{other}");
        }
    }

    #[test]
    fn a_def_that_the_table_cannot_represent_still_counts() {
        assert_eq!(
            count_of(
                &format!("def foo(*args) -> None:\n    return\n{DEF}"),
                "foo"
            ),
            2
        );
    }

    #[test]
    fn a_binding_outside_module_scope_does_not_count() {
        for other in [
            // A function-local binding.
            "def bar() -> None:\n    foo = 3\n",
            // A class-body binding.
            "class Bar:\n    foo = 3\n",
            // A comprehension's own target.
            "xs = [foo for foo in range(2)]\n",
            // An annotation without a value.
            "foo: int\n",
            // A read, and an attribute or subscript store.
            "print(foo)\nx.foo = 1\nxs[foo] = 1\n",
        ] {
            assert_eq!(count_of(&format!("{DEF}{other}"), "foo"), 1, "{other}");
        }
    }
}
