//! Per-module namespaces (v0.3 accept 1): the loader's half of
//! `pycc_ast::namespace`.
//!
//! Every dependency module's own top-level names are renamed to their
//! qualified spelling (`helper.double` -> `0m6helper_double`), and every
//! importer's references to a project module -- `import m` then `m.f(...)`,
//! `m.CONST`, `m.Class(...)`, `import pkg.sub` then `pkg.sub.f`, `from m
//! import f as g` then `g(...)` -- are rewritten to the qualified name of
//! the definition they reach. The importer's project `import` statements
//! are replaced by compile-time `from <module key> import <qualified
//! names>` statements at the top of the module, which `pycc_hir` answers
//! with the defining module's HIR; a variable `from m import x` also keeps
//! a value copy, `x = <qualified x>`, where the import ran. The module
//! bodies themselves run where `program::link` splices them, at the
//! import that first ran each.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use pycc_ast::namespace::{
    self, ChainRewrite, GlobalRewrite, NameRewrite, module_bindings, rewrite_module_globals,
};
use pycc_ast::{Expr, ExprContext, ModModule, Stmt};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::HirModule;

use super::cycle::{PROVISIONAL, Provisional, module_level_names, module_level_reads};

/// What kind of top-level definition an export names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Function,
    Class,
    Alias,
    Variable,
}

/// What a module's top-level name is, as another module reaches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Export {
    /// A definition (its own, or one it imported with `from`), by its
    /// qualified spelling and the module that defines it.
    Def {
        name: String,
        origin: usize,
        kind: Kind,
        /// The variable lives in a one-element list because a function of
        /// its module rebinds it (`pycc_ast`'s `desugar::globals`): it is
        /// read as `name[0]`.
        cell: bool,
    },
    /// A project module the name is bound to (`import m`, `from . import
    /// sub`).
    Module(String),
    /// A name a CPython or standard-library import of module `origin`
    /// binds: reached under its own spelling, through a compile-time import
    /// of it from `origin` (`pycc_hir` copies a standard-library binding and
    /// refuses a CPython one).
    Other { origin: usize },
}

/// A loaded module's dotted name and the names it offers importers. The
/// entry module has neither: nothing imports it (that would be a cycle).
#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleInfo {
    pub(crate) qual: Option<String>,
    pub(crate) exports: HashMap<String, Export>,
}

/// What the loader resolved one project import request to.
#[derive(Debug, Clone)]
pub(crate) struct Imported {
    /// The dotted name of the module the request names (`m` of `import m`
    /// / `from m import x`, the package of `from . import x`).
    pub(crate) target: String,
    /// The submodules a `from X import a, b` loaded because `X` itself has
    /// no such name: name -> dotted module name.
    pub(crate) submodules: HashMap<String, String>,
}

/// Every module the program has loaded so far, as the rewrite reads it.
pub(crate) struct Program<'a> {
    pub(crate) infos: &'a [ModuleInfo],
    pub(crate) by_qual: &'a HashMap<String, usize>,
    pub(crate) display_paths: Vec<&'a str>,
    /// The modules still running their bodies that a circular import
    /// reached (`super::cycle`).
    pub(crate) provisional: &'a [Provisional],
}

impl Program<'_> {
    fn exports(&self, qual: &str) -> Option<&HashMap<String, Export>> {
        match self.by_qual.get(qual) {
            Some(index) => Some(&self.infos[*index].exports),
            None => self.provisional_of(qual).map(|module| &module.exports),
        }
    }

    /// The module `qual` when it is still running its body.
    fn provisional_of(&self, qual: &str) -> Option<&Provisional> {
        self.provisional
            .iter()
            .find(|module| !module.done && module.qual == qual)
    }

    /// The dotted name of the module `origin` names.
    fn qual_of(&self, origin: usize) -> String {
        match origin.checked_sub(PROVISIONAL) {
            Some(slot) => self.provisional[slot].qual.clone(),
            None => self.infos[origin]
                .qual
                .clone()
                .expect("a definition another module reaches is in a dependency"),
        }
    }
}

/// What a project-import name of the module being rewritten is bound to.
#[derive(Debug, Clone)]
enum Binding {
    Module(String),
    Def {
        name: String,
        origin: usize,
    },
    /// A name of a CPython or standard-library import module `origin`
    /// made.
    Other {
        name: String,
        origin: usize,
    },
}

/// A value copy `from m import x` makes of a variable of `m`.
struct Snapshot {
    local: String,
    source: String,
    origin: usize,
    cell: bool,
}

/// The outcome of rewriting one module.
pub(crate) struct Rewritten {
    /// The modules the synthesized imports name (index, module key).
    pub(crate) synthesized: Vec<(usize, String)>,
}

/// Rewrites `parsed` for per-module namespaces. `qual` is the module's
/// dotted name, `None` for the entry module (whose own names keep their
/// spelling). `answers` maps the start offset of each project-import
/// request span the loader answered to what it loaded.
pub(crate) fn rewrite(
    parsed: &mut ModModule,
    qual: Option<&str>,
    answers: &HashMap<Span, Imported>,
    program: &Program<'_>,
) -> Result<Rewritten, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    // The project-import statements at the top level: their bindings, and
    // the value copies their variables need.
    let mut bindings: HashMap<String, Binding> = HashMap::new();
    let mut snapshots: HashMap<usize, Vec<Snapshot>> = HashMap::new();
    let mut project_statements: BTreeSet<usize> = BTreeSet::new();
    // A from-import of a name a partially initialized module (a circular
    // import) has not bound yet: how many of its value copies run first,
    // and CPython's `ImportError` message.
    let mut raises: HashMap<usize, (usize, String)> = HashMap::new();
    // The variables such an import names from the point it raises.
    let mut failed: BTreeSet<String> = BTreeSet::new();
    for (position, stmt) in parsed.body.iter().enumerate() {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let Some(answer) = answers.get(&span_of(alias.range)) else {
                        continue;
                    };
                    project_statements.insert(position);
                    let (local, target) = match &alias.asname {
                        Some(asname) => (asname.to_string(), answer.target.clone()),
                        None => {
                            let root = alias.name.split('.').next().unwrap_or_default();
                            (root.to_string(), root.to_string())
                        }
                    };
                    bindings.insert(local, Binding::Module(target));
                }
            }
            Stmt::ImportFrom(import) => {
                let Some(answer) = answers.get(&span_of(import.range)) else {
                    continue;
                };
                project_statements.insert(position);
                for alias in &import.names {
                    let name = alias.name.as_str();
                    if name == "*" {
                        diagnostics.push(Diagnostic::error(
                            "C0001",
                            "`from ... import *` (wildcard import) is not supported yet",
                            span_of(import.range),
                        ));
                        continue;
                    }
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    let export = program
                        .exports(&answer.target)
                        .and_then(|exports| exports.get(name));
                    if let Some(module) = program.provisional_of(&answer.target)
                        && export.is_some()
                        && !module.bound.contains(name)
                        && !raises.contains_key(&position)
                    {
                        let copies = snapshots.get(&position).map_or(0, Vec::len);
                        raises.insert(position, (copies, module.import_error(name)));
                    }
                    match (export, answer.submodules.get(name)) {
                        (
                            Some(Export::Def {
                                name,
                                origin,
                                kind,
                                cell,
                            }),
                            _,
                        ) => {
                            if *kind == Kind::Variable && raises.contains_key(&position) {
                                // Never copied: the import raises first.
                                // Functions reading the name (never run
                                // either) reach the partial module's own.
                                failed.insert(local.clone());
                                if !*cell {
                                    bindings.insert(
                                        local,
                                        Binding::Def {
                                            name: name.clone(),
                                            origin: *origin,
                                        },
                                    );
                                }
                            } else if *kind == Kind::Variable {
                                snapshots.entry(position).or_default().push(Snapshot {
                                    local: local.clone(),
                                    source: name.clone(),
                                    origin: *origin,
                                    cell: *cell,
                                });
                            } else {
                                bindings.insert(
                                    local,
                                    Binding::Def {
                                        name: name.clone(),
                                        origin: *origin,
                                    },
                                );
                            }
                        }
                        (Some(Export::Module(sub)), _) | (None, Some(sub)) => {
                            bindings.insert(local, Binding::Module(sub.clone()));
                        }
                        (Some(Export::Other { origin }), _) => {
                            bindings.insert(
                                local,
                                Binding::Other {
                                    name: name.to_string(),
                                    origin: *origin,
                                },
                            );
                        }
                        (None, None) => diagnostics.push(Diagnostic::error(
                            "T0021",
                            format!(
                                "module `{}` (`{}`) has no top-level name `{name}`",
                                answer.target,
                                program
                                    .by_qual
                                    .get(&answer.target)
                                    .map_or("?", |index| program.display_paths[*index])
                            ),
                            span_of(import.range),
                        )),
                    }
                }
            }
            _ => {}
        }
    }
    // `a.x` read while the module body runs, `a` partially initialized and
    // `x` not bound yet: CPython's `AttributeError`, not supported.
    let partial: BTreeSet<String> = bindings
        .iter()
        .filter_map(|(local, binding)| match binding {
            Binding::Module(module) if program.provisional_of(module).is_some() => {
                Some(local.clone())
            }
            _ => None,
        })
        .collect();
    // A module-level read of a variable such an import names: never run
    // (the import raised), and it reads a global the partial module has
    // not bound, which the checker refuses.
    for (range, name) in module_level_names(&parsed.body, &failed) {
        diagnostics.push(Diagnostic::error(
            "C0001",
            format!(
                "`{name}` is read after the circular import that binds it, which raises \
                 `ImportError` (the module it comes from is still running its body and has not \
                 bound `{name}` yet); a module-level read of it is not supported"
            ),
            Span::new(range.start, range.end),
        ));
    }
    if !partial.is_empty() {
        for (range, root, attr) in module_level_reads(&parsed.body, &partial) {
            let Some(Binding::Module(module)) = bindings.get(&root) else {
                continue;
            };
            let module = program
                .provisional_of(module)
                .expect("a partial root names a provisional module");
            if module.exports.contains_key(&attr) && !module.bound.contains(&attr) {
                diagnostics.push(Diagnostic::error(
                    "C0001",
                    format!(
                        "`{root}.{attr}` is read while `{}` is still running its body (a \
                         circular import) and has not bound `{attr}` yet: CPython raises \
                         `AttributeError` here, which is not supported",
                        module.qual
                    ),
                    Span::new(range.start, range.end),
                ));
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    // The module's own names: renamed in a dependency, kept in the entry.
    let scanned = module_bindings(parsed);
    let mut own: BTreeSet<String> = scanned.defined;
    for copies in snapshots.values() {
        own.extend(copies.iter().map(|copy| copy.local.clone()));
    }
    let mut dunder_name = false;
    if qual.is_some()
        && scanned.mentions_dunder_name
        && !own.contains("__name__")
        && !bindings.contains_key("__name__")
    {
        own.insert("__name__".to_string());
        dunder_name = true;
    }
    let rewriter = Rewriter {
        qual,
        own: &own,
        bindings: &bindings,
        program,
        uses: RefCell::new(BTreeMap::new()),
    };
    for copies in snapshots.values() {
        for copy in copies {
            rewriter.record(copy.origin, &copy.source);
        }
    }
    // A from-imported name another module binds by an import of its own is
    // looked up through that module whether or not it is used, so
    // `pycc_hir` reports a re-exported CPython import at the statement (and
    // copies a standard-library binding).
    for binding in bindings.values() {
        if let Binding::Other { name, origin } = binding {
            rewriter.record(*origin, name);
        }
    }
    let errors = rewrite_module_globals(parsed, &rewriter);
    if !errors.is_empty() {
        return Err(errors
            .into_iter()
            .map(|error| {
                Diagnostic::error(
                    error.code,
                    error.message,
                    Span::new(error.range.start, error.range.end),
                )
            })
            .collect());
    }
    let uses = rewriter.uses.into_inner();
    // Replace each project-import statement by its value copies (or
    // nothing), keeping its range for the splice position.
    let anchor = project_statements
        .first()
        .map(|position| pycc_ast::stmt_range(&parsed.body[*position]));
    let body = std::mem::take(&mut parsed.body);
    let mut rebuilt = Vec::with_capacity(body.len());
    for (position, stmt) in body.into_iter().enumerate() {
        if !project_statements.contains(&position) {
            rebuilt.push(stmt);
            continue;
        }
        let range = ruff_range(pycc_ast::stmt_range(&stmt));
        let copies = snapshots.remove(&position).unwrap_or_default();
        // The copies of the names before the one not bound yet run, then
        // the `ImportError`; the rest would never run, and would read
        // globals the partial module has not bound: they are left out.
        let raise = raises.remove(&position);
        let before = raise.as_ref().map_or(copies.len(), |(before, _)| *before);
        for copy in copies.into_iter().take(before) {
            let target = match qual {
                Some(qual) => namespace::module_global(qual, &copy.local),
                None => copy.local,
            };
            rebuilt.push(namespace::snapshot_assign(
                &target,
                &copy.source,
                copy.cell,
                range,
            ));
        }
        if let Some((_, message)) = raise {
            rebuilt.push(namespace::raise_with_message(
                "ImportError",
                &message,
                range,
            ));
        }
    }
    let prologue = prologue_len(&rebuilt);
    let anchor = ruff_range(anchor.unwrap_or(0..0));
    let mut synthesized = Vec::new();
    let mut header = Vec::new();
    if dunder_name && let Some(qual) = qual {
        header.push(namespace::string_assign(
            &namespace::module_global(qual, "__name__"),
            qual,
            ruff_range(0..0),
        ));
    }
    let mut imports = Vec::new();
    for (origin, names) in uses {
        let origin_qual = program.qual_of(origin);
        let key = namespace::module_key(&origin_qual);
        let names: Vec<String> = names.into_iter().collect();
        imports.push(namespace::synthesized_import(&key, &names, anchor));
        synthesized.push((origin, key));
    }
    // Compile-time bindings first, then the `__name__` copy, then the body.
    rebuilt.splice(prologue..prologue, imports.into_iter().chain(header));
    parsed.body = rebuilt.into();
    Ok(Rewritten { synthesized })
}

/// The exports of a lowered dependency: its own names by their qualified
/// spelling and kind, and the project-import names it binds.
pub(crate) fn exports(
    parsed_before: &ExportScan,
    qual: &str,
    index: usize,
    hir: &HirModule,
) -> HashMap<String, Export> {
    let mut exports = HashMap::new();
    for name in &parsed_before.own {
        let qualified = namespace::module_global(qual, name);
        let kind = if hir.class_defs.iter().any(|(class, _)| *class == qualified) {
            Kind::Class
        } else if hir.items.iter().any(
            |item| matches!(item, pycc_hir::HirItem::Function { name, .. } if *name == qualified),
        ) {
            Kind::Function
        } else if hir
            .type_aliases
            .iter()
            .any(|(alias, _)| *alias == qualified)
        {
            Kind::Alias
        } else {
            Kind::Variable
        };
        // A module global a function rebinds lives in a cell: importers
        // reach it under its source name, as the cell's element.
        let (exported, cell) = match name.strip_prefix(namespace::CELL_PREFIX) {
            Some(source) => (source.to_string(), true),
            None => (name.clone(), false),
        };
        exports.insert(
            exported,
            Export::Def {
                name: qualified,
                origin: index,
                kind,
                cell,
            },
        );
    }
    for (local, export) in &parsed_before.imported {
        exports
            .entry(local.clone())
            .or_insert_with(|| export.clone());
    }
    exports
}

/// What a module binds at top level, scanned before the rewrite renames
/// it: its own names, and its import-bound names with what each exports.
#[derive(Debug, Default)]
pub(crate) struct ExportScan {
    own: BTreeSet<String>,
    imported: Vec<(String, Export)>,
}

/// Scans `parsed` (before [`rewrite`]) for what it will export.
pub(crate) fn scan_exports(
    parsed: &ModModule,
    index: usize,
    answers: &HashMap<Span, Imported>,
    program: &Program<'_>,
) -> ExportScan {
    let scanned = module_bindings(parsed);
    let mut scan = ExportScan {
        own: scanned.defined,
        imported: Vec::new(),
    };
    let mut project_names = BTreeSet::new();
    for stmt in &parsed.body {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    if let Some(answer) = answers.get(&span_of(alias.range)) {
                        let (local, target) = match &alias.asname {
                            Some(asname) => (asname.to_string(), answer.target.clone()),
                            None => {
                                let root = alias.name.split('.').next().unwrap_or_default();
                                (root.to_string(), root.to_string())
                            }
                        };
                        project_names.insert(local.clone());
                        scan.imported.push((local, Export::Module(target)));
                    }
                }
            }
            Stmt::ImportFrom(import) => {
                let Some(answer) = answers.get(&span_of(import.range)) else {
                    continue;
                };
                for alias in &import.names {
                    let name = alias.name.as_str();
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    project_names.insert(local.clone());
                    let export = program
                        .exports(&answer.target)
                        .and_then(|exports| exports.get(name))
                        .cloned()
                        .or_else(|| answer.submodules.get(name).cloned().map(Export::Module));
                    match export {
                        // A variable is copied: the copy is this module's own.
                        Some(Export::Def {
                            kind: Kind::Variable,
                            ..
                        }) => {
                            scan.own.insert(local);
                        }
                        Some(export) => scan.imported.push((local, export)),
                        None => {}
                    }
                }
            }
            _ => {}
        }
    }
    for name in scanned.imported {
        if !project_names.contains(&name) {
            scan.imported.push((name, Export::Other { origin: index }));
        }
    }
    scan
}

struct Rewriter<'a> {
    qual: Option<&'a str>,
    own: &'a BTreeSet<String>,
    bindings: &'a HashMap<String, Binding>,
    program: &'a Program<'a>,
    /// The qualified names this module reaches in each other module.
    uses: RefCell<BTreeMap<usize, BTreeSet<String>>>,
}

impl Rewriter<'_> {
    fn record(&self, origin: usize, name: &str) {
        self.uses
            .borrow_mut()
            .entry(origin)
            .or_default()
            .insert(name.to_string());
    }

    fn own_name(&self, name: &str) -> NameRewrite {
        match self.qual {
            Some(qual) if self.own.contains(name) => {
                NameRewrite::Rename(namespace::module_global(qual, name))
            }
            _ => NameRewrite::Keep,
        }
    }

    /// Follows `attrs` from module `qual`; see [`GlobalRewrite::chain`].
    fn resolve_chain(
        &self,
        root: &str,
        qual: &str,
        attrs: &[&str],
        ctx: ExprContext,
    ) -> ChainRewrite {
        let mut module = qual.to_string();
        for (position, attr) in attrs.iter().enumerate() {
            let last = position + 1 == attrs.len();
            let export = self
                .program
                .exports(&module)
                .and_then(|exports| exports.get(*attr));
            match export {
                Some(Export::Def {
                    name, origin, cell, ..
                }) => {
                    if last && ctx != ExprContext::Load {
                        return ChainRewrite::Refuse {
                            code: "C0001",
                            message: format!(
                                "assigning to `{root}.{}`, an attribute of the module `{module}`, \
                                 is not supported yet",
                                attrs[..=position].join(".")
                            ),
                        };
                    }
                    self.record(*origin, name);
                    return ChainRewrite::Name {
                        consumed: position + 1,
                        name: name.clone(),
                        cell: *cell,
                    };
                }
                Some(Export::Module(sub)) => module = sub.clone(),
                Some(Export::Other { origin }) => {
                    self.record(*origin, attr);
                    return ChainRewrite::Name {
                        consumed: position + 1,
                        name: (*attr).to_string(),
                        cell: false,
                    };
                }
                None => {
                    let sub = format!("{module}.{attr}");
                    if self.program.by_qual.contains_key(&sub) {
                        module = sub;
                    } else if self.program.exports(&module).is_none() {
                        return ChainRewrite::Refuse {
                            code: "C0001",
                            message: format!(
                                "`{module}.{attr}` reads the partially initialized module \
                                 `{module}`, which is not supported (a circular import)"
                            ),
                        };
                    } else {
                        return ChainRewrite::Refuse {
                            code: "T0021",
                            message: format!("module `{module}` has no attribute `{attr}`"),
                        };
                    }
                }
            }
        }
        ChainRewrite::Refuse {
            code: "C0001",
            message: format!(
                "the module `{module}` used as a value is not supported yet (only `{root}.<name>` \
                 is)"
            ),
        }
    }
}

impl GlobalRewrite for Rewriter<'_> {
    fn name(&self, name: &str, ctx: ExprContext) -> NameRewrite {
        let Some(binding) = self.bindings.get(name) else {
            return self.own_name(name);
        };
        if ctx == ExprContext::Invalid {
            return NameRewrite::Keep;
        }
        if ctx != ExprContext::Load {
            return NameRewrite::Refuse {
                code: "C0001",
                message: format!(
                    "rebinding `{name}`, which a project import binds, is not supported yet"
                ),
            };
        }
        match binding {
            Binding::Module(module) => NameRewrite::Refuse {
                code: "C0001",
                message: format!(
                    "the module `{module}` used as a value is not supported yet (only \
                     `{name}.<name>` is)"
                ),
            },
            Binding::Def { name, origin } => {
                self.record(*origin, name);
                NameRewrite::Rename(name.clone())
            }
            Binding::Other {
                name: imported,
                origin,
            } => {
                if imported != name {
                    return NameRewrite::Refuse {
                        code: "C0001",
                        message: format!(
                            "`from ... import {imported} as {name}` of a name a project module \
                             imports from a CPython or standard module is not supported yet"
                        ),
                    };
                }
                self.record(*origin, imported);
                NameRewrite::Keep
            }
        }
    }

    fn chain(&self, root: &str, attrs: &[&str], ctx: ExprContext) -> Option<ChainRewrite> {
        match self.bindings.get(root)? {
            Binding::Module(qual) => Some(self.resolve_chain(root, qual, attrs, ctx)),
            _ => None,
        }
    }
}

/// The number of leading statements that must stay first: a docstring and
/// the `from __future__` imports after it.
fn prologue_len(body: &[Stmt]) -> usize {
    body.iter()
        .enumerate()
        .position(|(index, stmt)| match stmt {
            Stmt::Expr(expr) => !(index == 0 && matches!(*expr.value, Expr::StringLiteral(_))),
            Stmt::ImportFrom(import) => {
                !(import.level == 0
                    && import
                        .module
                        .as_ref()
                        .is_some_and(|module| module.as_str() == "__future__"))
            }
            _ => true,
        })
        .unwrap_or(body.len())
}

fn span_of<R>(range: R) -> Span
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    Span::new(range.start, range.end)
}

fn ruff_range(range: std::ops::Range<u32>) -> pycc_ast::TextRange {
    pycc_ast::TextRange::new(range.start.into(), range.end.into())
}
