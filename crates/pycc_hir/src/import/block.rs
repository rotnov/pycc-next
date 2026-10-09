//! A CPython-backed `import` (Part 1 of #1282, #1291) or `from ... import`
//! (#1383) nested in a module-level `if`/`try` block.
//!
//! [`lower_block_imports`] runs before a module-level `if`/`try` statement
//! is lowered. It lowers each `import` statement, and each `from ...
//! import` the driver answered foreign, nested in the block exactly as a
//! top-level one would be lowered, and keeps a statement's bindings only
//! when every one is foreign. `module::lower_top_level_item`
//! pushes those bindings, with [`ForeignImportSite::Block`] (optional when a
//! `try` whose handler catches a failed import guards it, #1290), onto the
//! module's import table before lowering the block, so the driver's lock,
//! policy and native-build gates see them. `lower_stmt` then turns the
//! statement into [`HirStmt::ForeignImport`] through
//! [`nested_foreign_import`], which finds its bindings by the statement's
//! span. Before the item loop, `module::own_foreign::binds_foreign_import`
//! replays the same call in source order, with the same import table, to
//! decide whole-module object-receiver admission (#1482), so a change to
//! the guard or `try` rules here applies to both.

use super::ResolvedImport;
use super::fallback::IMPORT_ERROR_CATCHERS;
use super::{FuturePosition, lower_import_stmt, statement_span};
use crate::stmt::is_type_checking_guard;
use crate::{ForeignImportSite, HirStmt, ImportBinding, ResolvedImports};
use pycc_ast::Stmt;
use pycc_diag::{Diagnostic, Span};

/// What [`lower_block_imports`] found in one module-level block.
#[derive(Debug, Default)]
pub(crate) struct BlockImports {
    /// The foreign bindings of every nested `import` statement whose
    /// aliases are all foreign, and of every nested foreign `from ...
    /// import` (#1383), in source order, each with
    /// [`ForeignImportSite::Block`].
    pub(crate) bindings: Vec<ImportBinding>,
    /// The span of the import statement each entry of `bindings` came
    /// from, index for index, so a refusal of one binding can point at its
    /// own statement rather than at the enclosing block.
    pub(crate) spans: Vec<Span>,
    /// The diagnostic each nested import statement that failed to lower
    /// would report at top level (a foreign from-import's alias, wildcard
    /// or spelling refusal included), keyed by that statement's span. A
    /// statement with a `pycc_std` alias is in neither list.
    pub(crate) deferred: Vec<(Span, Diagnostic)>,
}

impl BlockImports {
    /// The diagnostic to report when lowering the block failed with
    /// `error`: the deferred diagnostic of the nested `import` statement
    /// `error` is reported at, or `error` itself when none was deferred
    /// there. Only `stmt::unsupported`'s catch-all reports at a nested
    /// `import` statement's own span, so the swap never replaces any other
    /// error.
    pub(crate) fn substitute(self, error: Diagnostic) -> Diagnostic {
        self.deferred
            .into_iter()
            .find(|(span, _)| Some(*span) == error.span)
            .map_or(error, |(_, deferred)| deferred)
    }
}

/// Lowers every `import` statement nested in `stmt` when `stmt` is a
/// module-level `if` or `try` (including `except*`); any other statement
/// yields an empty result.
///
/// The walk covers the `if` body and every `elif`/`else` body, and the
/// `try` body, every handler, `else` and `finally`, recursing into a nested
/// `if`/`try`. It never enters a `for`, `while`, `with`, `match`, `def` or
/// `class` body, and it skips the body of an `if`/`elif` whose test
/// [`is_type_checking_guard`] accepts, the same bodies `lower_stmt` folds
/// away. An import it does not reach keeps `lower_stmt`'s block-body
/// diagnostic, and so does a nested `from ... import` the driver did not
/// answer foreign (#1383).
///
/// Called by `module::lower_top_level_item` for each module-level block,
/// and by `module::own_foreign::binds_foreign_import` (#1482), which
/// replays it ahead of the item loop with the import table as it stands
/// at that statement.
pub(crate) fn lower_block_imports(
    stmt: &Stmt,
    resolved: &ResolvedImports<'_>,
    imports: &[ImportBinding],
) -> BlockImports {
    let mut found = BlockImports::default();
    if matches!(stmt, Stmt::If(_) | Stmt::Try(_)) {
        walk_stmt(stmt, false, resolved, imports, &mut found);
    }
    found
}

/// Walks `body`. `guarded` is true when an enclosing `try` body has a
/// handler that catches a failed import ([`handlers_catch_import_error`]),
/// which makes every `import` reached from here optional (#1290).
fn walk_body(
    body: &[Stmt],
    guarded: bool,
    resolved: &ResolvedImports<'_>,
    imports: &[ImportBinding],
    found: &mut BlockImports,
) {
    for stmt in body {
        walk_stmt(stmt, guarded, resolved, imports, found);
    }
}

fn walk_stmt(
    stmt: &Stmt,
    guarded: bool,
    resolved: &ResolvedImports<'_>,
    imports: &[ImportBinding],
    found: &mut BlockImports,
) {
    match stmt {
        Stmt::Import(import) => {
            lower_nested_import(stmt, statement_span(import.range), guarded, resolved, found);
        }
        // #1383: a nested `from ... import` lowers only when the driver
        // answered it foreign. Any other answer -- a project or `pycc_std`
        // module, a relative miss, none at all -- leaves the statement to
        // `lower_stmt`'s block-body diagnostic.
        Stmt::ImportFrom(import)
            if matches!(
                resolved.get(statement_span(import.range)),
                Some(ResolvedImport::Foreign)
            ) =>
        {
            lower_nested_import(stmt, statement_span(import.range), guarded, resolved, found);
        }
        Stmt::If(if_stmt) => {
            if !is_type_checking_guard(&if_stmt.test, imports) {
                walk_body(&if_stmt.body, guarded, resolved, imports, found);
            }
            for clause in &if_stmt.elif_else_clauses {
                if clause
                    .test
                    .as_ref()
                    .is_some_and(|test| is_type_checking_guard(test, imports))
                {
                    continue;
                }
                walk_body(&clause.body, guarded, resolved, imports, found);
            }
        }
        Stmt::Try(try_stmt) => {
            // Only the `try` body is guarded by its own handlers; a handler,
            // `else` or `finally` body runs outside them and inherits the
            // enclosing guard.
            let body_guarded = guarded || handlers_catch_import_error(&try_stmt.handlers);
            walk_body(&try_stmt.body, body_guarded, resolved, imports, found);
            for handler in &try_stmt.handlers {
                let pycc_ast::ExceptHandler::ExceptHandler(handler) = handler;
                walk_body(&handler.body, guarded, resolved, imports, found);
            }
            walk_body(&try_stmt.orelse, guarded, resolved, imports, found);
            walk_body(&try_stmt.finalbody, guarded, resolved, imports, found);
        }
        _ => {}
    }
}

/// Lowers one nested `import` or foreign `from ... import` statement `stmt`
/// spanning `span` exactly as a top-level one, with a
/// [`ForeignImportSite::Block`] site. The bindings are kept only when every
/// one is foreign; a failure is deferred under `span`.
fn lower_nested_import(
    stmt: &Stmt,
    span: Span,
    guarded: bool,
    resolved: &ResolvedImports<'_>,
    found: &mut BlockImports,
) {
    // `position` only matters for a `from __future__` import, which is
    // never answered foreign and so never reaches here.
    match lower_import_stmt(
        stmt,
        resolved,
        FuturePosition::Body,
        ForeignImportSite::Block { optional: guarded },
    ) {
        Ok(lowered) => {
            let bindings = lowered.map(|l| l.bindings).unwrap_or_default();
            if bindings
                .iter()
                .all(|binding| matches!(binding, ImportBinding::Foreign { .. }))
            {
                found
                    .spans
                    .extend(std::iter::repeat_n(span, bindings.len()));
                found.bindings.extend(bindings);
            }
        }
        Err(diagnostic) => found.deferred.push((span, diagnostic)),
    }
}

/// Whether any of `handlers` catches a failed `import` (#1290): a bare
/// `except:`, or a handler whose type is one of [`IMPORT_ERROR_CATCHERS`]
/// by name, alone or as an element of a tuple. Any other type expression
/// (an attribute such as `builtins.ImportError`, a call) answers false,
/// which keeps the import required.
pub(crate) fn handlers_catch_import_error(handlers: &[pycc_ast::ExceptHandler]) -> bool {
    fn catches(expr: &pycc_ast::Expr) -> bool {
        match expr {
            pycc_ast::Expr::Name(name) => IMPORT_ERROR_CATCHERS.contains(&name.id.as_str()),
            pycc_ast::Expr::Tuple(tuple) => tuple.elts.iter().any(catches),
            _ => false,
        }
    }
    handlers.iter().any(|handler| {
        let pycc_ast::ExceptHandler::ExceptHandler(handler) = handler;
        handler.type_.as_deref().is_none_or(catches)
    })
}

/// The [`HirStmt::ForeignImport`] for `stmt` when it is an `import` or
/// `from ... import` statement whose bindings [`lower_block_imports`]
/// recorded in `imports` (matched by the statement's own span), or `None`.
/// A from-import's bindings carry their [`crate::FromImport`], in source
/// order.
pub(crate) fn nested_foreign_import(stmt: &Stmt, imports: &[ImportBinding]) -> Option<HirStmt> {
    let span = match stmt {
        Stmt::Import(import) => statement_span(import.range),
        Stmt::ImportFrom(import) => statement_span(import.range),
        _ => return None,
    };
    let bindings: Vec<(String, String, Option<crate::FromImport>)> = imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from,
                site: ForeignImportSite::Block { .. },
                span: binding_span,
            } if *binding_span == span => {
                Some((local_name.clone(), module_path.clone(), from.clone()))
            }
            _ => None,
        })
        .collect();
    (!bindings.is_empty()).then_some(HirStmt::ForeignImport { bindings, span })
}
