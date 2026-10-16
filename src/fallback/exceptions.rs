//! Exception classes an `--ext` module would publish without their members.
//!
//! The artifact publishes a compiled exception class to Python as a bare
//! `PyErr_NewException` class (`ext_build::collect_user_exception_classes`):
//! its bases and name, none of its own `__init__`, methods or class
//! attributes. Python code (fallback code included) that reads such a class
//! would build and inspect a different object from the one the program
//! defines, so in non-strict mode the class runs in CPython instead.

use pycc_ast::{Expr, Stmt};
use std::collections::BTreeSet;

/// The module-level exception classes (a builtin exception base, or another
/// such class) whose body defines anything: what the published class lacks.
pub(super) fn with_hidden_members(body: &[Stmt]) -> BTreeSet<String> {
    let mut exceptions: BTreeSet<String> = BTreeSet::new();
    let mut hidden = BTreeSet::new();
    for stmt in body {
        let Stmt::ClassDef(class) = stmt else {
            continue;
        };
        let is_exception = class.bases().iter().any(|base| match base {
            Expr::Name(name) => {
                let name = name.id.as_str();
                pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&name) || exceptions.contains(name)
            }
            _ => false,
        });
        if !is_exception {
            continue;
        }
        let name = class.name.as_str().to_string();
        if class.body.iter().any(|s| !is_inert(s)) {
            hidden.insert(name.clone());
        }
        exceptions.insert(name);
    }
    hidden
}

/// `pass`, `...` or a docstring: nothing the published class would miss.
fn is_inert(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Pass(_) => true,
        Stmt::Expr(expr) => matches!(
            &*expr.value,
            Expr::StringLiteral(_) | Expr::EllipsisLiteral(_)
        ),
        _ => false,
    }
}
