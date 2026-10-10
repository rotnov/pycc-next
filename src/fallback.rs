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
    ENTRY_OVERRIDE.with(|cell| *cell.borrow_mut() = text);
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
#[derive(Debug)]
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
    object_builtins: BTreeMap<u32, String>,
    /// Unannotated public functions fallback code calls: compiled with an
    /// object for every unannotated parameter and return (the signature
    /// Python callers need), and published, instead of inferred.
    objectified: BTreeSet<usize>,
    /// Globals that compiled code could not keep native and mirror into
    /// the namespace (`native_globals`): a copy failed to compile.
    demoted: BTreeSet<String>,
    /// Compiled imports that fallback code reads a name of: they stay
    /// compiled and also run in CPython right after, so both sides see the
    /// module (`cascade`).
    mirrored: BTreeSet<usize>,
    /// The entry file's directory: an import of a module found there is the
    /// program's own and is never mirrored (CPython would load a second copy).
    entry_dir: Option<std::path::PathBuf>,
    /// Whether a compiled public function, and a compiled public class, is
    /// visible to Python code in the module namespace (an `--ext` module
    /// publishes both; an embedded executable with fallback code publishes
    /// its functions).
    publishes: Publishes,
}

impl Plan {
    /// A plan with nothing falling back, or `None` when `source` does not
    /// parse (the syntax error is then reported as usual).
    pub(crate) fn new(source: &str, display: &str, publishes: Publishes) -> Option<Self> {
        let module = pycc_parser::parse_all(source).ok()?;
        let stmts = module
            .body
            .iter()
            .map(|stmt| TopStmt {
                range: pycc_ast::stmt_range(stmt),
                kind: match stmt {
                    Stmt::FunctionDef(def) => {
                        Kind::Def(def.name.as_str().to_string(), unannotated(def))
                    }
                    Stmt::ClassDef(class) => Kind::Class(class.name.as_str().to_string()),
                    Stmt::Import(_) | Stmt::ImportFrom(_) => Kind::Import,
                    _ => Kind::Other,
                },
                bound: names::bound_names(std::slice::from_ref(stmt)),
                mentions: names::module_references(stmt),
                opaque: names::opaque_bound_names(stmt),
                declares: names::all_global_declarations(stmt),
                defaults: matches!(stmt, Stmt::FunctionDef(def)
                    if def.parameters.iter_non_variadic_params().any(|p| p.default.is_some())),
            })
            .collect::<Vec<_>>();
        Some(Self {
            source: source.to_string(),
            display: display.to_string(),
            ast: module.body.into_iter().collect(),
            stmts,
            fallback: BTreeMap::new(),
            object_builtins: BTreeMap::new(),
            mirrored: BTreeSet::new(),
            demoted: BTreeSet::new(),
            objectified: BTreeSet::new(),
            entry_dir: None,
            publishes,
        })
    }

    /// Records the entry file's directory (see `entry_dir`).
    pub(crate) fn set_entry_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.entry_dir = dir;
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
        );
        // The docstring and `from __future__` imports stay compiled: the
        // preamble goes after them.
        let first = self.preamble_position();
        for failure in failures {
            if self.call_through_builtins(rendered, failure) || self.demote(rendered, failure) {
                continue;
            }
            let reason = describe(&failure.diagnostic);
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
        ) != before
    }

    /// Records a compiled call of a builtin pycc does not compile for its
    /// arguments (`C0001` "call to builtin") as a call through CPython's
    /// `builtins` module, returning whether it did. A builtin that reads
    /// its caller's frame, an exception class, or a call already made
    /// through `builtins` is left to fall back.
    fn call_through_builtins(&mut self, rendered: Option<&Rendered>, failure: &Failure) -> bool {
        let message = &failure.diagnostic.message;
        let Some(name) = message
            .strip_prefix("call to builtin `")
            .and_then(|rest| rest.strip_suffix("` is valid Python but not implemented yet"))
        else {
            return false;
        };
        if !name.starts_with(|c: char| c.is_ascii_lowercase())
            || matches!(
                name,
                "super" | "locals" | "globals" | "vars" | "eval" | "exec" | "dir" | "breakpoint"
            )
        {
            return false;
        }
        let Some(span) = failure.diagnostic.span.filter(|s| (s.start, s.end) != (0, 0)) else {
            return false;
        };
        let at = match rendered {
            Some(rendered) => rendered.source_offset(span.start),
            None => Some(span.start),
        };
        let Some(at) = at else {
            return false;
        };
        let rest = &self.source[at as usize..];
        let is_call = rest
            .strip_prefix(name)
            .is_some_and(|after| after.trim_start().starts_with('('));
        let starts_name = at == 0
            || !self.source[..at as usize]
                .ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
        if !is_call || !starts_name || self.object_builtins.contains_key(&at) {
            return false;
        }
        self.object_builtins.insert(at, name.to_string());
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
        let mut fallback_read = BTreeSet::new();
        let mut compiled_bound = BTreeSet::new();
        for (index, stmt) in self.stmts.iter().enumerate() {
            if self.fallback.contains_key(&index) {
                fallback_bound.extend(stmt.bound.iter().cloned());
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
        Ok(_) => return first,
        Err(failure) => match split(failure) {
            Some(failures)
                if crate::mode::non_strict_for(path)
                    && std::env::var_os("PYCC_NO_FALLBACK").is_none() =>
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
    let mut rendered: Option<Rendered> = None;
    let mut failures = failures;
    loop {
        if !plan.absorb(rendered.as_ref(), &failures) {
            set_override(None);
            return first;
        }
        let next = plan.render();
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- fallback rewrite ---\n{}", next.text);
        }
        set_override(Some(next.text.clone()));
        rendered = Some(next);
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
                Some(next_failures) => failures = next_failures,
                None => {
                    set_override(None);
                    return first;
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
