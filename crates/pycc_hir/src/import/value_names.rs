//! The names a module reads as values -- anywhere but in an annotation.
//!
//! A `typing` form pycc models (`Union`, `Any`, `Dict`, `Optional`, ...) is
//! resolved by its spelling in annotations and has no value of its own. A
//! non-strict module that also uses one as a value (`V = Union[A, B]`,
//! `R = dict[str, Any]`, `isinstance(x, Dict)`) needs the CPython object, so
//! its `from typing import ...` statement binds every name it imports as the
//! CPython object instead ([`super::lower_import_stmt`]), as a module whose
//! import statement fell back to CPython did.

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ModModule};
use std::collections::HashSet;

/// Every bare name `module` reads outside an annotation.
pub fn names_used_as_values(module: &ModModule) -> HashSet<String> {
    struct Names(HashSet<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_annotation(&mut self, _expr: &'a Expr) {}
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut names = Names(HashSet::new());
    for stmt in &module.body {
        names.visit_stmt(stmt);
    }
    names.0
}

/// Whether a `typing` symbol of `kind` named `name` is one a value use
/// needs as the CPython object: a typing form or annotation wrapper with no
/// compiled value. `TypeVar` and `Generic` declare compile-time generics
/// through their binding, and the decorator and cast markers are compiled
/// where they are used, so their uses never ask for the object.
pub(super) fn needs_object_for_value(kind: pycc_std::StdSymbolKind, name: &str) -> bool {
    matches!(
        kind,
        pycc_std::StdSymbolKind::TypingFormMarker | pycc_std::StdSymbolKind::AnnotationMarker
    ) && !matches!(name, "TypeVar" | "Generic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotations_are_not_value_uses() {
        let source = "from typing import Any, Union\nV = Union[int, str]\n\
                      def f(x: Any) -> Union[int, None]:\n    y: Any = x\n    return len([x])\n";
        let module = pycc_parser::parse_all(source).unwrap();
        let names = names_used_as_values(&module);
        assert!(names.contains("Union"));
        assert!(!names.contains("Any"));
        assert!(names.contains("len") && names.contains("x"));
    }
}
