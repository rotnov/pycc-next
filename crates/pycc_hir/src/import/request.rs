//! The request half of the driver contract for project and foreign
//! imports (#898, D-222): the scan of a parsed module's top-level
//! statements for the imports the driver must resolve on the filesystem
//! before `module::lower_module` runs. Extracted from `import.rs` (#1291),
//! which also added the nested-block walk and aliased-name requests.

use super::{is_future_import, statement_span};
use pycc_ast::{ModModule, Stmt};
use pycc_diag::Span;

/// One project-import request -- a whole module-level `from ... import`
/// statement, or one qualifying alias of a plain `import` (#1280) -- that
/// `pycc_std`'s registry does not answer, so the driver must resolve it on
/// the filesystem before
/// `module::lower_module` runs (#898, D-222). `pycc_hir` itself never
/// touches the filesystem: this is the request half of the contract, and
/// [`ResolvedImports`] is the answer half. Under the driver's
/// `--ext --foreign-relative-imports` mode (#1366) the entry module's
/// relative requests, and since #1382 its absolute requests rooted at its
/// own top-level package, are answered as foreign without a filesystem
/// probe.
///
/// `names` is empty exactly for a bare `import m` (which binds a module
/// namespace, a shape Part 1 only recognizes) and lists every imported
/// name, in source order, for `from ... import a, b`. `module` is `None`
/// only for a relative `from . import x` with no module segment. `span` is
/// the key the driver answers under: the whole statement's span for a
/// `from ... import`, and the one alias's own span for a plain `import`,
/// which yields one request per qualifying alias so `import sys, re` gets
/// an answer for each module (#1280).
///
/// `nested` is whether the statement sits in a module-level `if`/`try`
/// body rather than at top level (#1291, #1383). The driver answers a
/// nested `from ... import` only when it is foreign and never loads the
/// module it names, so a nested from-import of a project module keeps its
/// block-body diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectImportRequest {
    pub level: u32,
    pub module: Option<String>,
    pub names: Vec<String>,
    pub span: Span,
    pub nested: bool,
}

/// Scans a parsed module's top-level statements for the imports the driver
/// must resolve: every relative `from` import, and every absolute
/// `import`/`from ... import` naming a module `pycc_std::resolve_module`
/// rejects. A plain `import` contributes one request per alias that
/// `pycc_std` does not resolve, aliased or not (#1291), so each name of
/// `import a, b` is asked about on its own (#1280). Everything else
/// (stdlib imports, non-import statements) is left to
/// [`lower_import_stmt`]'s own single-file dispatch, so a module with no
/// project import yields an empty list and lowers exactly as before.
///
/// A plain `import` (#1291) or `from ... import` (#1383) nested in a
/// module-level `if`/`try` block is requested the same way, flagged
/// `nested`. The walk recurses into nested `if`/`try` bodies and never
/// into a `for`/`while`/`with`/`match`/`def`/`class` body. It does not skip
/// an `if TYPE_CHECKING:` body, so it asks about a superset of the imports
/// `lower_block_imports` lowers; the extra answers are never read.
/// Answering a nested request never loads a file: a bare `import m` stops
/// before loading, and the driver answers a nested `from ... import` only
/// when it is foreign.
pub fn project_import_requests(module: &ModModule) -> Vec<ProjectImportRequest> {
    let mut requests = Vec::new();
    for stmt in &module.body {
        requests.extend(project_import_request(stmt, false));
        nested_import_requests(stmt, false, &mut requests);
    }
    requests
}

/// The requests of every `import`/`from ... import` nested in `stmt` when
/// it is an `if`/`try`; `nested` is whether `stmt` is itself inside such a
/// block.
fn nested_import_requests(stmt: &Stmt, nested: bool, requests: &mut Vec<ProjectImportRequest>) {
    let bodies: Vec<&[Stmt]> = match stmt {
        Stmt::Import(_) | Stmt::ImportFrom(_) if nested => {
            requests.extend(project_import_request(stmt, true));
            return;
        }
        Stmt::If(if_stmt) => std::iter::once(if_stmt.body.as_slice())
            .chain(
                if_stmt
                    .elif_else_clauses
                    .iter()
                    .map(|clause| clause.body.as_slice()),
            )
            .collect(),
        Stmt::Try(try_stmt) => std::iter::once(try_stmt.body.as_slice())
            .chain(try_stmt.handlers.iter().map(|handler| {
                let pycc_ast::ExceptHandler::ExceptHandler(handler) = handler;
                handler.body.as_slice()
            }))
            .chain([try_stmt.orelse.as_slice(), try_stmt.finalbody.as_slice()])
            .collect(),
        _ => return,
    };
    for body in bodies {
        for inner in body {
            nested_import_requests(inner, true, requests);
        }
    }
}

/// Whether `name` is a module `pycc_std` compiles natively (`math`,
/// `typing`, ...): an absolute import of it never becomes a
/// [`ProjectImportRequest`], so the driver never resolves it. The driver
/// reads the same predicate to refuse an entry package of that name under
/// `--foreign-relative-imports` (#1382).
pub fn is_native_std_module(name: &str) -> bool {
    pycc_std::resolve_module(name).is_some()
}

fn project_import_request(stmt: &Stmt, nested: bool) -> Vec<ProjectImportRequest> {
    match stmt {
        Stmt::Import(import) => import
            .names
            .iter()
            .filter(|alias| !is_native_std_module(alias.name.as_str()))
            .map(|alias| ProjectImportRequest {
                level: 0,
                module: Some(alias.name.to_string()),
                names: Vec::new(),
                span: statement_span(alias.range),
                nested,
            })
            .collect(),
        Stmt::ImportFrom(import) => {
            // A `from __future__ import ...` is a compiler directive, not a
            // module (D-229): the driver must never probe the project for a
            // sibling `__future__.py`, which CPython would only reach at
            // run time, *after* applying the directive.
            if is_future_import(import) {
                return Vec::new();
            }
            let module = import.module.as_ref().map(ToString::to_string);
            if import.level == 0 && module.as_deref().is_some_and(is_native_std_module) {
                return Vec::new();
            }
            vec![ProjectImportRequest {
                level: import.level,
                module,
                names: import
                    .names
                    .iter()
                    .map(|alias| alias.name.to_string())
                    .collect(),
                span: statement_span(import.range),
                nested,
            }]
        }
        _ => Vec::new(),
    }
}
