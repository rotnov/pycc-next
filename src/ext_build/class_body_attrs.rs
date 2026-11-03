//! v0.12: the entries CPython's compiler puts in a class body's namespace
//! that a published class was made without: `__firstlineno__`, the line of
//! the statement (its first decorator's when it has one), `__doc__`, the
//! body's docstring as the compiler cleans it, and `__static_attributes__`,
//! the sorted names of the attributes assigned through `self.<name>` in the
//! functions of the class body. The shim makes the class with them
//! (`pycc_ext_class_args`, the `f`, `d` and `s` lines of a class's plan), so
//! `C.__firstlineno__`, `C.__doc__` and `C.__static_attributes__` read as
//! CPython's.

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, Stmt};
use std::collections::BTreeSet;

/// `class`'s `__firstlineno__` in `source` (1-based).
pub(crate) fn first_line(class: &pycc_ast::StmtClassDef, source: &str) -> usize {
    let start = class
        .decorator_list
        .iter()
        .map(|decorator| pycc_ast::Ranged::range(decorator).start())
        .chain(std::iter::once(pycc_ast::Ranged::range(class).start()))
        .min()
        .map_or(0, |start| u32::from(start) as usize);
    source
        .get(..start)
        .map_or(0, |before| before.bytes().filter(|&b| b == b'\n').count())
        + 1
}

/// `class`'s docstring, the string literal opening its body, cleaned as
/// CPython 3.13+'s compiler cleans it (`_PyCompile_CleanDoc`: tabs
/// expanded, the first line's leading spaces and the common indentation of
/// the other non-blank lines removed); `None` when the body opens with
/// anything else.
pub(crate) fn docstring(class: &pycc_ast::StmtClassDef) -> Option<String> {
    let Some(Stmt::Expr(first)) = class.body.first() else {
        return None;
    };
    let Expr::StringLiteral(doc) = first.value.as_ref() else {
        return None;
    };
    Some(clean_doc(doc.value.to_str()))
}

/// `doc` as `_PyCompile_CleanDoc` leaves it.
fn clean_doc(doc: &str) -> String {
    let doc = expand_tabs(doc);
    let mut lines = doc.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    let rest: Vec<&str> = lines.collect();
    let margin = rest
        .iter()
        .filter_map(|line| {
            let indent = line.bytes().take_while(|&b| b == b' ').count();
            line.as_bytes()
                .get(indent)
                .is_some_and(|&b| b != b'\n')
                .then_some(indent)
        })
        .min()
        .unwrap_or(0);
    let mut out = first.trim_start_matches(' ').to_string();
    for line in rest {
        let indent = line.bytes().take(margin).take_while(|&b| b == b' ').count();
        out.push_str(&line[indent..]);
    }
    out
}

/// `str.expandtabs()`: each tab to the next multiple of 8 columns, the
/// column restarting after each `\n` or `\r`.
fn expand_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let pad = 8 - column % 8;
                out.extend(std::iter::repeat_n(' ', pad));
                column += pad;
            }
            '\n' | '\r' => {
                out.push(ch);
                column = 0;
            }
            _ => {
                out.push(ch);
                column += 1;
            }
        }
    }
    out
}

/// A class body's own binding of a name the compiler also puts in its
/// namespace (`__doc__`, `__firstlineno__`): the body's binding runs after
/// the compiler's and wins.
#[derive(Debug, PartialEq)]
pub(crate) enum Explicit {
    Str(String),
    Int(i64),
    /// Bound some other way: a non-literal value, an augmented assignment.
    Other,
}

/// The last binding of `name` by a statement of `class`'s body itself
/// (`name = ...`, `name: T = ...`, `name += ...`), or `None` when the body
/// binds it nowhere at its top level.
pub(crate) fn explicit(class: &pycc_ast::StmtClassDef, name: &str) -> Option<Explicit> {
    let names = |target: &Expr| matches!(target, Expr::Name(n) if n.id.as_str() == name);
    let literal = |value: &Expr| match value {
        Expr::StringLiteral(text) => Explicit::Str(text.value.to_str().to_string()),
        Expr::NumberLiteral(number) => match &number.value {
            pycc_ast::Number::Int(int) => int.as_i64().map_or(Explicit::Other, Explicit::Int),
            _ => Explicit::Other,
        },
        _ => Explicit::Other,
    };
    let mut found = None;
    for stmt in &class.body {
        match stmt {
            Stmt::Assign(assign) if assign.targets.iter().any(names) => {
                found = Some(literal(&assign.value));
            }
            Stmt::AnnAssign(assign) if names(&assign.target) => {
                if let Some(value) = &assign.value {
                    found = Some(literal(value));
                }
            }
            Stmt::AugAssign(assign) if names(&assign.target) => found = Some(Explicit::Other),
            _ => {}
        }
    }
    found
}

/// `class`'s `__static_attributes__`: every `self.<name>` store (an
/// assignment, augmented or annotated assignment, `for` or `with` target,
/// at any depth) in a function of its body, a nested function's or
/// comprehension's included and a nested class's excluded, sorted, as
/// CPython's compiler records them.
pub(crate) fn static_attributes(class: &pycc_ast::StmtClassDef) -> Vec<String> {
    struct Collect {
        in_function: usize,
        names: BTreeSet<String>,
    }
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                // A nested class records its own.
                Stmt::ClassDef(_) => {}
                Stmt::FunctionDef(_) => {
                    self.in_function += 1;
                    visitor::walk_stmt(self, stmt);
                    self.in_function -= 1;
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Attribute(attribute) = expr
                && self.in_function > 0
                && attribute.ctx == pycc_ast::ExprContext::Store
                && matches!(attribute.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self")
            {
                self.names.insert(attribute.attr.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect {
        in_function: 0,
        names: BTreeSet::new(),
    };
    for stmt in &class.body {
        collect.visit_stmt(stmt);
    }
    collect.names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(source: &str) -> pycc_ast::StmtClassDef {
        let parsed = pycc_parser::parse_all(source).expect("parse");
        parsed
            .body
            .into_iter()
            .find_map(|stmt| match stmt {
                Stmt::ClassDef(class) => Some(class),
                _ => None,
            })
            .expect("a class")
    }

    #[test]
    fn the_first_line_is_the_first_decorators() {
        let source = "x = 1\n\n@a\n@b\nclass C:\n    pass\n";
        assert_eq!(first_line(&class(source), source), 3);
        let source = "x = 1\nclass C:\n    pass\n";
        assert_eq!(first_line(&class(source), source), 2);
    }

    #[test]
    fn the_docstring_is_cleaned_as_cpythons_compiler_cleans_it() {
        let doc = |body: &str| docstring(&class(&format!("class C:\n{body}")));
        assert_eq!(doc("    \"\"\"One.\"\"\"\n").as_deref(), Some("One."));
        assert_eq!(
            doc("    \"\"\"  One.\n\n      Two.\n        Three.\n    \"\"\"\n").as_deref(),
            Some("One.\n\nTwo.\n  Three.\n")
        );
        assert_eq!(doc("    'a\\tb\\n\\tc'\n").as_deref(), Some("a       b\nc"));
        assert_eq!(doc("    x = 1\n"), None);
        assert_eq!(doc("    b'no'\n"), None);
    }

    #[test]
    fn the_bodys_own_binding_is_its_last() {
        let body = class(
            "class C:\n    \"Doc.\"\n    __doc__ = 'one'\n    __doc__ = 'two'\n    \
             __firstlineno__: int = 9\n    x = __doc__\n",
        );
        assert_eq!(explicit(&body, "__doc__"), Some(Explicit::Str("two".to_string())));
        assert_eq!(explicit(&body, "__firstlineno__"), Some(Explicit::Int(9)));
        assert_eq!(explicit(&body, "x"), Some(Explicit::Other));
        assert_eq!(explicit(&body, "y"), None);
        let aug = class("class C:\n    __firstlineno__ = 1\n    __firstlineno__ += 1\n");
        assert_eq!(explicit(&aug, "__firstlineno__"), Some(Explicit::Other));
    }

    #[test]
    fn static_attributes_are_self_stores_in_functions_sorted() {
        let source = "class C:\n    self = None\n    \
                      def __init__(self):\n        self.b = 1\n        self.a: int = 2\n        \
                      self.a += 1\n        del self.gone\n        self.c.d = 3\n    \
                      def f(this):\n        this.z = 1\n        def inner():\n            \
                      self.q = 1\n        for self.w in []:\n            pass\n        \
                      [0 for self.cv in []]\n    \
                      class Inner:\n        def m(self):\n            self.inner = 1\n";
        assert_eq!(
            static_attributes(&class(source)),
            ["a", "b", "cv", "q", "w"]
        );
        assert!(static_attributes(&class("class C:\n    pass\n")).is_empty());
    }
}
