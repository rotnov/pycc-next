//! Function-local foreign imports (v0.8 accept 2): an `import` or `from
//! ... import` inside a function or method body that the driver answered
//! foreign (`import::request::function_import_requests`).
//!
//! [`lower`] runs before a top-level `def` or `class` statement is lowered.
//! It lowers every such import in the statement's function bodies, at any
//! depth, exactly as a top-level one, with a [`ForeignImportSite::Local`]
//! site, and keeps a statement's bindings only when every one is foreign:
//! an import of a module `pycc_std` compiles natively (`math`), or of a
//! project module, keeps `lower_stmt`'s function-body diagnostic. The
//! bindings are in `ModuleState::imports` while the item lowers, where
//! `lower_stmt` finds them by span (`import::nested_foreign_import`) and
//! lowers each statement to `HirStmt::ForeignImport`, which binds local
//! names: CPython's `IMPORT_NAME`, through `sys.modules`, at the
//! statement's own position, every time it runs. `lower_module` then takes
//! them out again and appends them after the last item, so they reach the
//! driver's gates without any module-level lookup seeing them.

use crate::import::{FuturePosition, ResolvedImport, ResolvedImports, statement_span};
use crate::import::handlers_catch_import_error;
use crate::{ForeignImportSite, ImportBinding, lower_import_stmt};
use pycc_ast::Stmt;

/// The foreign bindings of the function-local imports in `stmt` (a
/// top-level `def` or `class`; any other statement has none), in source
/// order.
pub(super) fn lower(stmt: &Stmt, resolved: &ResolvedImports<'_>) -> Vec<ImportBinding> {
    let mut found = Vec::new();
    if matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
        walk(stmt, false, false, resolved, &mut found);
    }
    found
}

/// Walks `stmt`; `in_function` is whether it is in a function body, and
/// `guarded` whether a `try` of that body catching a failed import
/// encloses it (the lock's optional root, as for a module-level block).
fn walk(
    stmt: &Stmt,
    in_function: bool,
    guarded: bool,
    resolved: &ResolvedImports<'_>,
    found: &mut Vec<ImportBinding>,
) {
    match stmt {
        Stmt::Import(_) | Stmt::ImportFrom(_) if in_function => {
            lower_one(stmt, guarded, resolved, found);
        }
        Stmt::FunctionDef(def) => {
            for inner in &def.body {
                walk(inner, true, false, resolved, found);
            }
        }
        Stmt::Try(try_stmt) => {
            let body_guarded = guarded || handlers_catch_import_error(&try_stmt.handlers);
            for inner in &try_stmt.body {
                walk(inner, in_function, body_guarded, resolved, found);
            }
            for body in crate::import::child_bodies(stmt).into_iter().skip(1) {
                for inner in body {
                    walk(inner, in_function, guarded, resolved, found);
                }
            }
        }
        _ => {
            for body in crate::import::child_bodies(stmt) {
                for inner in body {
                    walk(inner, in_function, guarded, resolved, found);
                }
            }
        }
    }
}

/// Lowers one function-local import statement, keeping its bindings when
/// every one is foreign. A `from ... import` lowers only when the driver
/// answered it foreign (never a project or `pycc_std` module).
fn lower_one(
    stmt: &Stmt,
    guarded: bool,
    resolved: &ResolvedImports<'_>,
    found: &mut Vec<ImportBinding>,
) {
    if let Stmt::ImportFrom(import) = stmt
        && !matches!(
            resolved.get(statement_span(import.range)),
            Some(ResolvedImport::Foreign)
        )
    {
        return;
    }
    let site = ForeignImportSite::Local { optional: guarded };
    let Ok(Some(lowered)) = lower_import_stmt(stmt, resolved, FuturePosition::Body, site) else {
        return;
    };
    if !lowered.bindings.is_empty()
        && lowered
            .bindings
            .iter()
            .all(|binding| matches!(binding, ImportBinding::Foreign { .. }))
    {
        found.extend(lowered.bindings);
    }
}
