//! Per-function fallback in non-strict mode (#12).
//!
//! A top-level statement of the entry module that pycc cannot compile yet
//! (a `def` using a lambda, a generator, a nested function, ...) is kept as
//! Python source: the compiled module runs it with CPython's own `exec` in
//! the module's namespace dictionary, at the position it had, and binds
//! whatever it defines there. Compiled code calls a fallback function
//! through the object-call path; fallback code calls a compiled function
//! as the Python callable the module publishes.
//!
//! The mechanism is a source rewrite of the entry module, re-run through
//! the unchanged frontend until it compiles:
//!
//! 1. compile the module; on failure, map each diagnostic to the
//!    top-level statement it belongs to and mark that statement fallback;
//! 2. render the rewrite: a preamble binds the namespace
//!    (`sys.modules[__name__].__dict__`) and `builtins`; each run of
//!    consecutive fallback statements becomes one
//!    `builtins.exec(builtins.compile(<source>, <file>, "exec"), ns)`,
//!    the source padded with newlines so tracebacks keep their line
//!    numbers; and every compiled reference to a module global that
//!    fallback code reads or binds becomes `ns['name']`, so both sides see
//!    one value (a *dict global*);
//! 3. repeat until the rewrite compiles, or no diagnostic names a statement
//!    that is still compiled (then the original diagnostics are reported).
//!
//! Strict mode never falls back. Only the entry module falls back; a
//! diagnostic in another module of the program is reported as before.

mod annotations;
mod builtins;
mod function_values;
mod names;
mod render;

pub(crate) use render::Rendered;

use pycc_ast::Stmt;
use pycc_diag::Diagnostic;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The namespace dictionary's name in rewritten code.
const NS: &str = "__pycc_fallback_ns";
/// The `builtins` module's name in rewritten code.
const BUILTINS: &str = "__pycc_fallback_builtins";
/// The `sys` module's name in rewritten code.
const SYS: &str = "__pycc_fallback_sys";

static REPORT: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The rewritten entry source the loader reads instead of the file.
    static ENTRY_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Records `--report-fallbacks`.
pub(crate) fn set_report(report: bool) {
    REPORT.store(report, Ordering::Relaxed);
}

/// Whether `--report-fallbacks` was given.
pub(crate) fn reporting() -> bool {
    REPORT.load(Ordering::Relaxed)
}

/// Whether the program being built has fallback code.
pub(crate) fn active() -> bool {
    ENTRY_OVERRIDE.with(|cell| cell.borrow().is_some())
}

/// The rewritten entry source, when a fallback plan is active.
pub(crate) fn entry_override() -> Option<String> {
    ENTRY_OVERRIDE.with(|cell| cell.borrow().clone())
}

/// The entry module's source as the frontend compiles it: the fallback
/// rewrite when one is active, else the file itself.
pub(crate) fn entry_source(path: &Path) -> String {
    entry_override().unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
}

fn set_override(text: Option<String>) {
    if text.is_none() {
        TWINNED.with(|cell| cell.borrow_mut().clear());
    }
    ENTRY_OVERRIDE.with(|cell| *cell.borrow_mut() = text);
}

thread_local! {
    /// The functions of the active rewrite that also run in CPython for
    /// Python callers (`Plan::twinned`): not exported compiled.
    static TWINNED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Makes `rendered` the entry source the frontend compiles.
fn install(rendered: &Rendered) {
    set_override(Some(rendered.text.clone()));
    TWINNED.with(|cell| *cell.borrow_mut() = rendered.twinned.clone());
}

/// The compiled functions (and their `pycc_ast::ext_signatures` entries)
/// an `--ext` module does not export, because Python code calls their
/// CPython twin instead (`Plan::twinned`).
pub(crate) fn twinned_exports() -> BTreeSet<String> {
    TWINNED.with(|cell| {
        cell.borrow()
            .iter()
            .flat_map(|name| [name.clone(), pycc_ast::ext_entry_name(name)])
            .collect()
    })
}

/// Which compiled definitions an artifact publishes to Python code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Publishes {
    pub(crate) functions: bool,
    pub(crate) classes: bool,
    /// Whether an unannotated public function's signature is inferred from
    /// its compiled call sites (an embedded executable, `pycc_hir::func`):
    /// Python code may call it with anything, so fallback code that reads
    /// one makes it fall back too, like a private helper.
    pub(crate) infers_unannotated: bool,
    /// Whether every module-level variable and import binding is visible to
    /// Python code as a module attribute (an `--ext` module: `mod.X`,
    /// `from mod import X`, a package `__init__`'s re-exports). Each such
    /// name is then read by Python code as if by fallback code: a compiled
    /// global is kept native and copied into the namespace at every store
    /// (`Plan::native_globals`), an import is also run by CPython
    /// (`Plan::mirrored`) or falls back.
    pub(crate) variables: bool,
    /// Whether the entry module's relative imports bind CPython objects
    /// (`--foreign-relative-imports`): such an import is not a project
    /// import, so it may fall back.
    pub(crate) foreign_relative_imports: bool,
}

impl Publishes {
    /// Whether Python code sees the compiled definition `name`: published,
    /// public (a leading underscore is never exported), and for a function
    /// whose signature is inferred (`unannotated`), typed for any caller.
    fn sees(self, name: &str, class: bool, unannotated: bool) -> bool {
        let published = if class {
            self.classes
        } else {
            self.functions && !(unannotated && self.infers_unannotated)
        };
        published && !name.starts_with('_')
    }
}

/// What one top-level statement is, for the report and the cascade rules.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A function, and whether a parameter or its return is unannotated.
    Def(String, bool),
    Class(String),
    Import,
    Other,
}

#[derive(Debug, Clone)]
struct TopStmt {
    range: Range<u32>,
    kind: Kind,
    /// The names it binds at module scope.
    bound: BTreeSet<String>,
    /// The module globals it reads, binds or declares `global`, at any
    /// depth.
    mentions: BTreeSet<String>,
    /// The module-scope names it binds other than through a plain name
    /// target (an import, `except ... as`, a nested `def`).
    opaque: BTreeSet<String>,
    /// The names it declares `global` at any depth.
    declares: BTreeSet<String>,
    /// A function with a parameter default: the published callable cannot
    /// apply defaults yet (#1194), so Python code never calls it compiled.
    defaults: bool,
}

/// A diagnostic of the compiled rewrite, as the fallback loop sees it.
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    pub(crate) diagnostic: Diagnostic,
    /// The source offset of the top-level statement the type checker keyed
    /// the diagnostic to, for a diagnostic with no span of its own.
    pub(crate) origin: Option<u32>,
}

/// Which statements fall back, and why.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    source: String,
    display: String,
    ast: Vec<Stmt>,
    stmts: Vec<TopStmt>,
    /// Statement index -> reason.
    fallback: BTreeMap<usize, String>,
    /// Source offsets of builtin calls compiled code makes through CPython's
    /// `builtins` module (`min(a, b)` becomes `builtins.min(a, b)`): the
    /// builtin is valid Python pycc does not compile for these arguments.
    object_builtins: BTreeMap<u32, (u32, String)>,
    /// Whether the module's annotations are never evaluated (`from
    /// __future__ import annotations`), so an annotation pycc cannot lower
    /// may be erased instead (`annotations`).
    defers_annotations: bool,
    /// The annotations of the module, at any depth (`annotations::sites`).
    annotation_sites: Vec<annotations::Site>,
    /// Erased annotations: source offset -> (end, replacement).
    erased: BTreeMap<u32, (u32, String)>,
    /// Alias reads substituted in annotations (`annotations`): source
    /// offset -> (end, replacement).
    substituted: BTreeMap<u32, (u32, String)>,
    /// Unannotated public functions fallback code calls: compiled with an
    /// object for every unannotated parameter and return (the signature
    /// Python callers need), and published, instead of inferred.
    objectified: BTreeSet<usize>,
    /// Globals that compiled code could not keep native and mirror into
    /// the namespace (`native_globals`): a copy failed to compile.
    demoted: BTreeSet<String>,
    /// Compiled imports that fallback code reads a name of: they stay
    /// compiled and also run in CPython right after, so both sides see the
    /// module (`cascade`). Also every function in `twinned`.
    mirrored: BTreeSet<usize>,
    /// Functions whose signature the `--ext` boundary cannot carry (an
    /// export gap, `C0003`): compiled code calls the compiled function,
    /// and its source also runs in CPython right after, binding the name
    /// Python code reads (its *twin*). Index -> the gap.
    twinned: BTreeMap<usize, String>,
    /// The entry file's directory: an import of a module found there is the
    /// program's own and is never mirrored (CPython would load a second copy).
    entry_dir: Option<std::path::PathBuf>,
    /// Whether a compiled public function, and a compiled public class, is
    /// visible to Python code in the module namespace (an `--ext` module
    /// publishes both; an embedded executable with fallback code publishes
    /// its functions).
    publishes: Publishes,
    /// The module-level names Python code outside the module reads
    /// (`Publishes::variables`): every name a top-level statement other
    /// than a `def` or `class` binds.
    external: BTreeSet<String>,
}

impl Plan {
    /// A plan with nothing falling back, or `None` when `source` does not
    /// parse (the syntax error is then reported as usual).
    pub(crate) fn new(source: &str, display: &str, publishes: Publishes) -> Option<Self> {
        let module = pycc_parser::parse_all(source).ok()?;
        // An `--ext` module publishes a function with defaults through a
        // Python wrapper of its signature (`pycc_ast::ext_signatures`), which
        // applies them: Python code may call such a function compiled.
        let wrapped: BTreeSet<String> = if publishes.variables {
            pycc_ast::ext_signatures(&module, source, true)
                .into_iter()
                .filter(|signature| signature.method().is_none())
                .map(|signature| signature.qualname)
                .collect()
        } else {
            BTreeSet::new()
        };
        let stmts = module
            .body
            .iter()
            .map(|stmt| (stmt, is_future_import(stmt)))
            .map(|(stmt, future)| TopStmt {
                range: pycc_ast::stmt_range(stmt),
                kind: match stmt {
                    Stmt::FunctionDef(def) => {
                        Kind::Def(def.name.as_str().to_string(), unannotated(def))
                    }
                    Stmt::ClassDef(class) => Kind::Class(class.name.as_str().to_string()),
                    Stmt::Import(_) | Stmt::ImportFrom(_) => Kind::Import,
                    _ => Kind::Other,
                },
                // A `from __future__` import is a compiler directive: no
                // Python code reads the feature it binds, and it can be
                // neither mirrored nor fall back (it must come first).
                bound: if future {
                    BTreeSet::new()
                } else {
                    names::bound_names(std::slice::from_ref(stmt))
                },
                mentions: names::module_references(stmt),
                opaque: if future {
                    BTreeSet::new()
                } else {
                    names::opaque_bound_names(stmt)
                },
                declares: names::all_global_declarations(stmt),
                defaults: matches!(stmt, Stmt::FunctionDef(def)
                    if def.parameters.iter_non_variadic_params().any(|p| p.default.is_some())
                        && !wrapped.contains(def.name.as_str())),
            })
            .collect::<Vec<_>>();
        let external = if publishes.variables {
            stmts
                .iter()
                .filter(|stmt| matches!(stmt.kind, Kind::Other | Kind::Import))
                .flat_map(|stmt| stmt.bound.iter())
                .filter(|name| !name.starts_with("__pycc") && name.as_str() != "*")
                .cloned()
                .collect()
        } else {
            BTreeSet::new()
        };
        let defers_annotations = annotations::defers_annotations(&module.body);
        let annotation_sites = annotations::sites(source, &module.body);
        Some(Self {
            source: source.to_string(),
            display: display.to_string(),
            ast: module.body.into_iter().collect(),
            stmts,
            fallback: BTreeMap::new(),
            object_builtins: BTreeMap::new(),
            defers_annotations,
            annotation_sites,
            erased: BTreeMap::new(),
            substituted: BTreeMap::new(),
            mirrored: BTreeSet::new(),
            twinned: BTreeMap::new(),
            demoted: BTreeSet::new(),
            objectified: BTreeSet::new(),
            entry_dir: None,
            publishes,
            external,
        })
    }

    /// Records the entry file's directory (see `entry_dir`).
    pub(crate) fn set_entry_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.entry_dir = dir;
        self.substituted.clear();
        self.substitute_aliases();
    }

    /// Whether the compiled import `index` can run in CPython too: an
    /// absolute import of no module of the program's own, other than `*`,
    /// binding names no other top-level statement binds.
    fn mirrorable(&self, index: usize) -> bool {
        let modules: Vec<&str> = match &self.ast[index] {
            // An alias of a stream method (`input = sys.stdin.readline`),
            // which compiled code desugars into a function (evaluating
            // nothing), so CPython evaluating it too has no other effect. A
            // bare stream (`input = sys.stdin`) is not one: compiled code
            // may read it.
            // A lambda (`mi = lambda: map(int, input().split())`), which
            // compiled code desugars into a function too: creating it has
            // no effect.
            Stmt::Assign(assign)
                if matches!(assign.targets.as_slice(), [pycc_ast::Expr::Name(_)])
                    && matches!(&*assign.value, pycc_ast::Expr::Lambda(_)) =>
            {
                return self.binds_alone(index);
            }
            Stmt::Assign(assign) => {
                return matches!(assign.targets.as_slice(), [pycc_ast::Expr::Name(_)])
                    && matches!(&*assign.value, pycc_ast::Expr::Attribute(attr)
                        if matches!(attr.attr.as_str(), "readline" | "readlines" | "read" | "write" | "flush")
                            && is_attribute_chain(&attr.value)
                            // Compiled code reads `sys.stdin.buffer` through
                            // the text stream, whose read-ahead CPython's
                            // binary reads would skip.
                            && !matches!(&*attr.value, pycc_ast::Expr::Attribute(inner)
                                if inner.attr.as_str() == "buffer"))
                    && self.binds_alone(index);
            }
            Stmt::Import(import) => import.names.iter().map(|a| a.name.as_str()).collect(),
            Stmt::ImportFrom(import) if import.level == 0 => match &import.module {
                Some(module) => vec![module.as_str()],
                None => return false,
            },
            _ => return false,
        };
        let stmt = &self.stmts[index];
        if stmt.opaque.contains("*") {
            return false;
        }
        let local = |module: &str| {
            let top = module.split('.').next().unwrap_or(module);
            self.entry_dir.as_ref().is_some_and(|dir| {
                dir.join(format!("{top}.py")).exists() || dir.join(top).is_dir()
            })
        };
        if modules.iter().any(|m| local(m)) {
            return false;
        }
        self.binds_alone(index)
    }

    /// Whether statement `index` imports a project module (a relative
    /// import, or an absolute one naming a module beside the entry file).
    /// Such a module is compiled into the program, not importable by
    /// CPython, so the statement cannot fall back (v0.3 accept 1).
    fn is_project_import(&self, index: usize) -> bool {
        let modules: Vec<&str> = match &self.ast[index] {
            Stmt::Import(import) => import.names.iter().map(|a| a.name.as_str()).collect(),
            Stmt::ImportFrom(import) if import.level > 0 => {
                return !self.publishes.foreign_relative_imports;
            }
            Stmt::ImportFrom(import) => import.module.iter().map(|m| m.as_str()).collect(),
            _ => return false,
        };
        modules.iter().any(|module| {
            let top = module.split('.').next().unwrap_or(module);
            self.entry_dir.as_ref().is_some_and(|dir| {
                dir.join(format!("{top}.py")).exists() || dir.join(top).is_dir()
            })
        })
    }

    /// Whether the plan makes a project import fall back, which no plan
    /// can do ([`Self::is_project_import`]).
    fn falls_back_a_project_import(&self) -> bool {
        self.fallback
            .keys()
            .any(|index| self.is_project_import(*index))
    }

    /// Whether no other top-level statement binds a name statement `index`
    /// binds.
    fn binds_alone(&self, index: usize) -> bool {
        let bound = &self.stmts[index].bound;
        self.stmts
            .iter()
            .enumerate()
            .all(|(other, s)| other == index || s.bound.intersection(bound).next().is_none())
    }

    /// Marks the statements `failures` point at as fallback, returning
    /// whether any statement was newly marked. A failure no statement owns
    /// (a span-less whole-module diagnostic, the preamble) marks every
    /// statement.
    pub(crate) fn absorb(&mut self, rendered: Option<&Rendered>, failures: &[Failure]) -> bool {
        let before = (
            self.fallback.len(),
            self.object_builtins.len(),
            self.demoted.len(),
            self.erased.len(),
            self.twinned.len(),
        );
        // The docstring and `from __future__` imports stay compiled: the
        // preamble goes after them.
        let first = self.preamble_position();
        for failure in failures {
            if self.erase_annotation(rendered, failure)
                || self.functions_as_objects(rendered, failure)
                || self.rewrite_builtin_use(rendered, failure)
                || self.demote(rendered, failure)
            {
                continue;
            }
            let reason = describe(&failure.diagnostic);
            if self.twin(rendered, failure, &reason) {
                continue;
            }
            match self.owner_of(rendered, failure) {
                Some(index) if index >= first => {
                    // Name the line within a multi-line statement.
                    let line = self.source_line(rendered, failure);
                    let stmt = &self.stmts[index].range;
                    let reason = match line {
                        Some(line)
                            if line_of(&self.source, stmt.start)
                                != line_of(&self.source, stmt.end.saturating_sub(1)) =>
                        {
                            format!("{reason} (line {line})")
                        }
                        _ => reason,
                    };
                    self.fallback.entry(index).or_insert(reason);
                }
                Some(_) => {}
                None => {
                    // A diagnostic no statement owns is usually the
                    // module-level solver's: the module-level statements
                    // fall back first, every statement only once they have.
                    let module_level: Vec<usize> = (first..self.stmts.len())
                        .filter(|index| {
                            matches!(self.stmts[*index].kind, Kind::Other | Kind::Import)
                                && !self.fallback.contains_key(index)
                        })
                        .collect();
                    let marked: Vec<usize> = if module_level.is_empty() {
                        (first..self.stmts.len()).collect()
                    } else {
                        module_level
                    };
                    for index in marked {
                        self.fallback.entry(index).or_insert_with(|| reason.clone());
                    }
                }
            }
        }
        self.cascade();
        (
            self.fallback.len(),
            self.object_builtins.len(),
            self.demoted.len(),
            self.erased.len(),
            self.twinned.len(),
        ) != before
    }

    /// Twins the function an export gap (`C0003`) of an `--ext` module
    /// refuses, returning whether it did.
    fn twin(&mut self, rendered: Option<&Rendered>, failure: &Failure, reason: &str) -> bool {
        if failure.diagnostic.code != "C0003" || !self.publishes.variables {
            return false;
        }
        let Some(index) = self.owner_of(rendered, failure) else {
            return false;
        };
        if !matches!(self.stmts[index].kind, Kind::Def(..)) || self.fallback.contains_key(&index) {
            return false;
        }
        self.mirrored.insert(index);
        self.twinned.entry(index).or_insert_with(|| reason.to_string());
        true
    }

    /// The source line a failure's span starts on, when it has one in
    /// compiled source.
    fn source_line(&self, rendered: Option<&Rendered>, failure: &Failure) -> Option<usize> {
        let span = failure.diagnostic.span.filter(|s| (s.start, s.end) != (0, 0))?;
        let at = match rendered {
            Some(rendered) => rendered.source_offset(span.start)?,
            None => span.start,
        };
        Some(line_of(&self.source, at))
    }

    /// The statement a failure belongs to.
    fn owner_of(&self, rendered: Option<&Rendered>, failure: &Failure) -> Option<usize> {
        let span = failure
            .diagnostic
            .span
            .filter(|s| (s.start, s.end) != (0, 0));
        let at = span.map(|s| s.start).or(failure.origin);
        if let Some(start) = at {
            let ranges: Vec<Range<u32>> = match rendered {
                Some(rendered) => {
                    if rendered.preamble.contains(&start) {
                        return None;
                    }
                    rendered.ranges.clone()
                }
                None => self.stmts.iter().map(|s| s.range.clone()).collect(),
            };
            return ranges
                .iter()
                .position(|r| r.contains(&start) || r.start == start);
        }
        None
    }

    /// Adds the statements whose compiled form Python code could not see:
    /// a compiled import, function or class that fallback code reads
    /// (unless the module publishes the function or class).
    fn cascade(&mut self) {
        loop {
            let globals = self.dict_globals();
            let read: BTreeSet<&String> = self
                .fallback
                .keys()
                .chain(&self.mirrored)
                .flat_map(|index| &self.stmts[*index].mentions)
                .chain(&self.external)
                .collect();
            // A name fallback code reads that no top-level statement binds
            // may come from a `from m import *`, whose names are not known
            // here: such an import falls back with the first one.
            let all_bound: BTreeSet<&String> =
                self.stmts.iter().flat_map(|stmt| &stmt.bound).collect();
            let unbound_read = read.iter().find(|name| !all_bound.contains(*name)).copied();
            let mirror: Vec<usize> = (0..self.stmts.len())
                .filter(|index| {
                    !self.fallback.contains_key(index)
                        && !self.mirrored.contains(index)
                        && matches!(self.stmts[*index].kind, Kind::Import | Kind::Other)
                        && self.stmts[*index].bound.iter().any(|n| read.contains(n))
                        && self.mirrorable(*index)
                })
                .collect();
            if !mirror.is_empty() {
                self.mirrored.extend(mirror);
                continue;
            }
            let objectify: Vec<usize> = (0..self.stmts.len())
                .filter(|index| {
                    !self.fallback.contains_key(index)
                        && !self.objectified.contains(index)
                        && matches!(&self.stmts[*index].kind,
                            Kind::Def(name, true) if read.contains(name)
                                && !self.stmts[*index].defaults
                                && self.publishes.sees(name, false, false)
                                && self.publishes.infers_unannotated)
                })
                .collect();
            if !objectify.is_empty() {
                self.objectified.extend(objectify);
                continue;
            }
            let forced: Vec<(usize, String)> = self
                .stmts
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    !self.fallback.contains_key(index) && !self.mirrored.contains(index)
                })
                .filter_map(|(index, stmt)| {
                    let name = match &stmt.kind {
                        Kind::Import if stmt.opaque.contains("*") => unbound_read?,
                        Kind::Import | Kind::Other => {
                            stmt.opaque.iter().find(|n| globals.contains(*n))?
                        }
                        Kind::Def(name, unannotated)
                            if read.contains(name)
                                && (stmt.defaults || !self.publishes.sees(
                                    name,
                                    false,
                                    *unannotated && !self.objectified.contains(&index),
                                )) =>
                        {
                            name
                        }
                        Kind::Class(name)
                            if read.contains(name) && !self.publishes.sees(name, true, false) =>
                        {
                            name
                        }
                        // An instance of a compiled class may reach fallback
                        // code through any value without its name being
                        // read, and an unpublished class's instances have no
                        // attributes Python code can see.
                        Kind::Class(name)
                            if !self.publishes.classes && !self.fallback.is_empty() =>
                        {
                            name
                        }
                        Kind::Def(..) | Kind::Class(_) => return None,
                    };
                    Some((
                        index,
                        format!(
                            "fallback code uses `{name}`, which Python code cannot see compiled"
                        ),
                    ))
                })
                .collect();
            if forced.is_empty() {
                return;
            }
            self.fallback.extend(forced);
        }
    }

    /// Demotes the native global whose namespace copy `failure` points at
    /// (a list cannot be stored into the namespace without a copy that
    /// would break aliasing), returning whether it did.
    fn demote(&mut self, rendered: Option<&Rendered>, failure: &Failure) -> bool {
        let (Some(rendered), Some(span)) = (rendered, failure.diagnostic.span) else {
            return false;
        };
        let Some((_, name)) = rendered
            .mirror_spans
            .iter()
            .find(|(range, _)| range.contains(&span.start))
        else {
            return false;
        };
        self.demoted.insert(name.clone())
    }

    /// The dict globals (`dict_globals`) that compiled code keeps native
    /// instead, copying every compiled store into the namespace: fallback
    /// code reads them but never binds them, so compiled code is their one
    /// writer, and every compiled store is a plain `=`/`+=` a copy can
    /// follow. The copy only compiles for a value the namespace can hold
    /// without breaking aliasing (a scalar or an object); any other is
    /// demoted by the fallback loop.
    pub(super) fn native_globals(&self) -> BTreeSet<String> {
        let mut fallback_binds = BTreeSet::new();
        for (index, stmt) in self.stmts.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                fallback_binds.extend(stmt.bound.iter().cloned());
                fallback_binds.extend(stmt.declares.iter().cloned());
            } else if !matches!(stmt.kind, Kind::Other) {
                fallback_binds.extend(stmt.opaque.iter().cloned());
            }
        }
        let mut native: BTreeSet<String> = self
            .dict_globals()
            .into_iter()
            .filter(|name| !fallback_binds.contains(name) && !self.demoted.contains(name))
            .collect();
        let none = BTreeSet::new();
        for (index, stmt) in self.ast.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                continue;
            }
            let mut rewriter = names::Rewriter::new(&none, NS).with_mirrors(native.clone());
            pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, stmt);
            native.retain(|name| !rewriter.unmirrorable.contains(name));
        }
        native
    }

    /// The module globals that live in the namespace dictionary.
    fn dict_globals(&self) -> BTreeSet<String> {
        let mut fallback_bound = BTreeSet::new();
        let mut fallback_read = self.external.clone();
        let mut compiled_bound = BTreeSet::new();
        for (index, stmt) in self.stmts.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                fallback_bound.extend(stmt.bound.iter().cloned());
                fallback_read.extend(stmt.mentions.iter().cloned());
            } else if self.twinned.contains_key(&index) {
                fallback_bound.extend(stmt.declares.iter().cloned());
                fallback_read.extend(stmt.mentions.iter().cloned());
            } else if matches!(stmt.kind, Kind::Other | Kind::Import)
                && !self.mirrored.contains(&index)
            {
                compiled_bound.extend(stmt.bound.iter().cloned());
            }
        }
        let mut globals: BTreeSet<String> = fallback_read
            .intersection(&compiled_bound)
            .cloned()
            .collect();
        globals.extend(fallback_bound);
        // A compiled `def`/`class` name stays native: it is published, or
        // fallback code reading it made it fall back too (`cascade`).
        for (index, stmt) in self.stmts.iter().enumerate() {
            if let (false, Kind::Def(name, _) | Kind::Class(name)) =
                (self.fallback.contains_key(&index), &stmt.kind)
                && !compiled_bound.contains(name)
            {
                globals.remove(name);
            }
        }
        globals
    }

    /// The statement the preamble goes before: the first one that is not
    /// the module docstring or a `from __future__` import.
    fn preamble_position(&self) -> usize {
        let body = &self.ast;
        let mut index = 0;
        if matches!(body.first(), Some(Stmt::Expr(e)) if e.value.is_string_literal_expr()) {
            index = 1;
        }
        while matches!(body.get(index), Some(Stmt::ImportFrom(import))
            if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
        {
            index += 1;
        }
        index
    }

    /// One line per fallback statement, for `--report-fallbacks`.
    pub(crate) fn report_lines(&self) -> Vec<String> {
        let twins = self.twinned.iter().map(|(index, reason)| {
            let stmt = &self.stmts[*index];
            let first = line_of(&self.source, stmt.range.start);
            let last = line_of(&self.source, stmt.range.end.saturating_sub(1));
            let name = match &stmt.kind {
                Kind::Def(name, _) => name.as_str(),
                _ => "?",
            };
            format!(
                "twin: function `{name}` at {}:{first}-{last} is compiled, and runs in CPython \
                 when Python code calls it: {reason}",
                self.display
            )
        });
        self.fallback
            .iter()
            .map(|(index, reason)| {
                let stmt = &self.stmts[*index];
                let first = line_of(&self.source, stmt.range.start);
                let last = line_of(&self.source, stmt.range.end.saturating_sub(1));
                let what = match &stmt.kind {
                    Kind::Def(name, _) => format!("function `{name}`"),
                    Kind::Class(name) => format!("class `{name}`"),
                    Kind::Import | Kind::Other if first == last => "statement".to_string(),
                    Kind::Import | Kind::Other => "statements".to_string(),
                };
                let lines = if first == last {
                    format!("{first}")
                } else {
                    format!("{first}-{last}")
                };
                format!(
                    "fallback: {what} at {}:{lines} runs in CPython: {reason}",
                    self.display
                )
            })
            .chain(twins)
            .collect()
    }
}

fn line_of(source: &str, offset: u32) -> usize {
    source[..(offset as usize).min(source.len())]
        .matches('\n')
        .count()
        + 1
}

fn describe(diagnostic: &Diagnostic) -> String {
    let message: String = diagnostic.message.chars().take(160).collect();
    format!("{} {message}", diagnostic.code)
}

/// Whether `stmt` is a `from __future__ import ...`.
fn is_future_import(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::ImportFrom(import)
        if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
}

/// Whether `expr` is a name or an attribute chain on one (`sys.stdin.readline`).
fn is_attribute_chain(expr: &pycc_ast::Expr) -> bool {
    match expr {
        pycc_ast::Expr::Name(_) => true,
        pycc_ast::Expr::Attribute(attr) => is_attribute_chain(&attr.value),
        _ => false,
    }
}

/// The module-level functions of `source` with an unannotated parameter
/// or return, whose signature a native non-strict build infers
/// (`pycc_hir::func`): an embedded build publishes none of them.
pub(crate) fn inferred_functions(source: &str) -> BTreeSet<String> {
    let Ok(module) = pycc_parser::parse_all(source) else {
        return BTreeSet::new();
    };
    module
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) if unannotated(def) => Some(def.name.as_str().to_string()),
            // A lambda alias, which compiled code desugars into an inferred
            // function (and CPython code sees as its own lambda, mirrored).
            Stmt::Assign(assign) if matches!(&*assign.value, pycc_ast::Expr::Lambda(_)) => {
                match assign.targets.as_slice() {
                    [pycc_ast::Expr::Name(name)] => Some(name.id.as_str().to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn unannotated(def: &pycc_ast::StmtFunctionDef) -> bool {
    def.returns.is_none() || def.parameters.iter().any(|p| p.annotation().is_none())
}

/// A Python string literal for `text`.
fn py_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether a diagnostic is one a fallback can resolve: anything but a
/// syntax error, an interop-policy or embedding refusal.
pub(crate) fn can_fall_back(diagnostic: &Diagnostic) -> bool {
    !matches!(
        diagnostic.code,
        "L0001" | "L0002" | "I0402" | "I0403" | "I0405"
    )
}

/// Runs `attempt` (one frontend pass) with the fallback loop around it,
/// in non-strict mode. `split` turns a failure into the entry file's
/// fallback-resolvable diagnostics, or `None` when the failure cannot be
/// resolved by falling back (another file, a syntax error, an input
/// error).
pub(crate) fn with_fallback<T, E>(
    path: &Path,
    display: &str,
    publishes: Publishes,
    attempt: impl Fn() -> Result<T, E>,
    split: impl Fn(&E) -> Option<Vec<Failure>>,
) -> Result<T, E> {
    set_override(None);
    let first = attempt();
    let failures = match &first {
        Ok(_) if publishes.variables && mode_allows(path) => {
            // Nothing falls back, but Python code reads the module's
            // variables: compile the rewrite that publishes them.
            let Some(mut plan) =
                read_entry(path).and_then(|s| Plan::new(&s, display, publishes))
            else {
                return first;
            };
            if plan.external.is_empty() {
                return first;
            }
            plan.set_entry_dir(path.parent().map(Path::to_path_buf));
            plan.cascade();
            if plan.falls_back_a_project_import() {
                return first;
            }
            let rendered = plan.render();
            if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
                eprintln!("--- fallback rewrite ---\n{}", rendered.text);
            }
            install(&rendered);
            let value = match attempt() {
                Ok(value) => Some(value),
                Err(failure) => split(&failure).and_then(|failures| {
                    converge(&mut plan, Some(rendered), failures, &attempt, &split)
                }),
            };
            return match value {
                Some(value) if !plan.falls_back_a_project_import() => {
                    if REPORT.load(Ordering::Relaxed) {
                        for line in plan.report_lines() {
                            eprintln!("{line}");
                        }
                    }
                    Ok(value)
                }
                _ => {
                    set_override(None);
                    attempt()
                }
            };
        }
        // Strict mode: Python code reads the module's variables, which a
        // rewrite copies onto the module object (`crate::ext_globals`).
        // Not under `--foreign-relative-imports`: the artifact is imported
        // under a dotted name its `__name__` may not spell.
        Ok(_)
            if publishes.variables
                && !publishes.foreign_relative_imports
                && !crate::mode::non_strict_for(path) =>
        {
            return publish_strict(path, first, &attempt, &split);
        }
        Ok(_) => return first,
        Err(failure) => match split(failure) {
            Some(failures) if mode_allows(path) =>
            {
                failures
            }
            _ => return first,
        },
    };
    let source = crate::fallback::read_entry(path);
    let Some(mut plan) = source.and_then(|s| Plan::new(&s, display, publishes)) else {
        return first;
    };
    plan.set_entry_dir(path.parent().map(Path::to_path_buf));
    // With annotation aliases substituted (`annotations`), the failures of
    // the source as written may be ones the substitution removes: the loop
    // starts from the rewrite's own.
    let (rendered, failures) = if plan.substituted.is_empty() {
        (None, failures)
    } else {
        let rendered = plan.render();
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- fallback rewrite ---\n{}", rendered.text);
        }
        install(&rendered);
        match attempt() {
            Ok(value) => {
                if REPORT.load(Ordering::Relaxed) {
                    for line in plan.report_lines() {
                        eprintln!("{line}");
                    }
                }
                return Ok(value);
            }
            Err(failure) => match split(&failure) {
                Some(failures) => (Some(rendered), failures),
                None => {
                    set_override(None);
                    return first;
                }
            },
        }
    };
    let Some(mut value) = converge(&mut plan, rendered, failures, &attempt, &split) else {
        set_override(None);
        return first;
    };
    // Fallback code that reaches a project module would need CPython to
    // import it: the program is refused with its original diagnostics.
    if plan.falls_back_a_project_import() {
        set_override(None);
        return first;
    }
    // A statement that fell back for a type error early on may compile now
    // that later rounds changed what it sees (a global kept native, a
    // function compiled with objects, a builtin called through CPython):
    // run the loop once more from a plan without those, and keep whichever
    // result falls back less.
    let mut retry = plan.clone();
    retry.fallback.retain(|_, reason| !retryable(reason));
    // So is a function compiled with objects only for fallback code that
    // may be gone: a compiled class instance it then received as an object
    // would have no attributes Python code can see.
    retry.objectified.clear();
    // What fallback code that stays forces is forced again.
    retry.cascade();
    if retry.fallback.len() < plan.fallback.len() {
        let kept = entry_override();
        let rendered = retry.render();
        install(&rendered);
        let retried = match attempt() {
            Ok(value) => Some(value),
            Err(failure) => split(&failure)
                .and_then(|failures| converge(&mut retry, Some(rendered), failures, &attempt, &split)),
        };
        match retried {
            Some(better)
                if retry.fallback.len() < plan.fallback.len()
                    && !retry.falls_back_a_project_import() =>
            {
                plan = retry;
                value = better;
            }
            _ => set_override(kept),
        }
    }
    if REPORT.load(Ordering::Relaxed) {
        for line in plan.report_lines() {
            eprintln!("{line}");
        }
    }
    Ok(value)
}

/// Compiles the strict `--ext` rewrite publishing the entry module's
/// variables (`crate::ext_globals`), dropping each name whose copy fails to
/// compile; `first` (nothing published) when a failure is not one of the
/// copies.
fn publish_strict<T, E>(
    path: &Path,
    first: Result<T, E>,
    attempt: &impl Fn() -> Result<T, E>,
    split: &impl Fn(&E) -> Option<Vec<Failure>>,
) -> Result<T, E> {
    let Some(source) = read_entry(path) else {
        return first;
    };
    let mut dropped = BTreeSet::new();
    loop {
        let Some(rewrite) = crate::ext_globals::rewrite(&source, &dropped) else {
            set_override(None);
            return first;
        };
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- ext globals rewrite ---\n{}", rewrite.text);
        }
        set_override(Some(rewrite.text.clone()));
        let failure = match attempt() {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        let failures = split(&failure).unwrap_or_default();
        let mut progress = false;
        let mut attributed = !failures.is_empty();
        for failure in &failures {
            let at = failure.diagnostic.span.map(|s| s.start).or(failure.origin);
            match at.and_then(|at| rewrite.copies.iter().find(|(range, _)| range.contains(&at))) {
                Some((_, name)) => progress |= dropped.insert(name.clone()),
                None => attributed = false,
            }
        }
        if !attributed || !progress {
            set_override(None);
            return first;
        }
    }
}

/// Whether the fallback loop runs for `path`: non-strict mode, and not
/// disabled with `PYCC_NO_FALLBACK`.
fn mode_allows(path: &Path) -> bool {
    crate::mode::non_strict_for(path) && std::env::var_os("PYCC_NO_FALLBACK").is_none()
}

/// Whether a statement that fell back for `reason` may compile in a later
/// plan: a type error may, an unsupported construct (`C0001`) never does.
fn retryable(reason: &str) -> bool {
    !reason.starts_with("C0001")
}

/// The fallback loop: absorbs `failures` into `plan`, renders and compiles
/// until an attempt succeeds (its value, with the rewrite as the entry
/// override) or no failure marks anything new (`None`).
fn converge<T, E>(
    plan: &mut Plan,
    mut rendered: Option<Rendered>,
    mut failures: Vec<Failure>,
    attempt: &impl Fn() -> Result<T, E>,
    split: &impl Fn(&E) -> Option<Vec<Failure>>,
) -> Option<T> {
    let dump = std::env::var_os("PYCC_FALLBACK_DUMP").is_some();
    loop {
        if !plan.absorb(rendered.as_ref(), &failures) {
            if dump {
                eprintln!("--- fallback loop stuck on ---");
                for failure in &failures {
                    eprintln!("{} {:?}", describe(&failure.diagnostic), failure.diagnostic.span);
                }
            }
            return None;
        }
        let next = plan.render();
        if dump {
            eprintln!("--- fallback rewrite ---\n{}", next.text);
        }
        install(&next);
        rendered = Some(next);
        match attempt() {
            Ok(value) => return Some(value),
            Err(failure) => match split(&failure) {
                Some(next) => failures = next,
                None => {
                    if dump {
                        eprintln!("--- fallback loop cannot place a failure ---");
                    }
                    return None;
                }
            },
        }
    }
}

/// The entry file's decoded source.
fn read_entry(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    crate::source::decode_python_source(&bytes).ok()
}

#[cfg(test)]
#[path = "fallback/tests.rs"]
mod tests;
