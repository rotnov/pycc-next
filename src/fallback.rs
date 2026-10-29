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
//!    numbers (in an `--ext` module `builtins.exec(ns["__pycc_code__"](k),
//!    ns)`: chunk `k`'s code object, compiled when the module was built,
//!    `render::CodeChunk`); and every compiled reference to a module global that
//!    fallback code reads or binds becomes `ns['name']`, so both sides see
//!    one value (a *dict global*);
//! 3. repeat until the rewrite compiles, or no diagnostic names a statement
//!    that is still compiled (then the original diagnostics are reported).
//!
//! The unit of fallback is the smallest enclosing function or method, not
//! the top-level statement: every `def` of a class body or block is a
//! *lift* (`lift.rs`), a unit of its own after the top-level statements.
//! A class (or block) statement CPython runs keeps its lifts compiled, each
//! `def` forwarding to a compiled module function holding its body; a lift
//! that fails runs in CPython inside its statement. In an `--ext` module a
//! compiled class keeps its native type and a failing method of it is
//! *attached* (`attach.rs`): a CPython function the class calls. The
//! statement-level steps above apply to these units unchanged; `cascade.rs`
//! adds what fallback code needs (a class it subclasses, a compiled name it
//! cannot see) and `driver.rs` runs the loop.
//!
//! Strict mode never falls back. Only the entry module falls back; a
//! diagnostic in another module of the program is reported as before.

mod annotations;
mod attach;
mod builtins;
mod cascade;
mod driver;
mod exc_layout;
mod exc_names;
mod exceptions;
mod function_values;
mod lift;
mod members;
mod names;
mod nested;
mod package_bases;
pub(crate) use package_bases::overridden_in_package;
mod render;
mod report;

pub(crate) use exc_layout::LAYOUT_PREFIX;
pub(crate) use lift::LIFT_PREFIX;
pub(crate) use driver::{can_fall_back, with_fallback};
pub(crate) use render::{CodeChunk, Rendered};

use pycc_ast::Stmt;
use pycc_diag::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::Ordering;

/// The namespace dictionary's name in rewritten code.
pub(crate) const NS: &str = "__pycc_fallback_ns";
/// The `builtins` module's name in rewritten code.
const BUILTINS: &str = pycc_ast::FALLBACK_BUILTINS;
/// The `sys` module's name in rewritten code.
const SYS: &str = "__pycc_fallback_sys";

mod state;

pub(crate) use state::{
    Snapshot, active, attached_methods, code_chunks, entry_copied, entry_override, entry_source, reporting, set_report,
    twinned_exports,
};
use state::{REPORT, install, set_override};

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
        // A lifted method body (`lift`): published for its forwarder.
        if name.starts_with(lift::LIFT_PREFIX) {
            return self.functions;
        }
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
    /// A function with a parameter default, `*args`, `**kwargs` or a
    /// keyword-only or positional-only parameter and no published wrapper:
    /// the published callable cannot bind it (#1194), so Python code never
    /// calls it compiled.
    defaults: bool,
    /// An exception class the artifact would publish without its own
    /// members (`exceptions`): Python code never sees it compiled.
    hidden_members: bool,
    /// A class an embedded executable would publish without some of its
    /// members (`members`): it runs in CPython once the module has any
    /// fallback code (`cascade`).
    unpublished_members: bool,
    /// The names its class statements' bases read (`class C(B)`, at any
    /// depth outside a function): a compiled class Python code subclasses
    /// runs in CPython (`cascade`), since a published class is not a base
    /// CPython accepts.
    bases: BTreeSet<String>,
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
    /// How many of `stmts` (and `ast`) are top-level statements; the rest
    /// are the `def`s lifted out of them (`lift`), unit `top + i` being
    /// `lifts[i]`.
    top: usize,
    lifts: Vec<lift::Lift>,
    /// The classes other modules of the entry's package subclass
    /// (`package_bases`): they run in CPython (`cascade`).
    package_bases: BTreeMap<String, String>,
    /// v0.10: the names the entry binds to an exception class of another
    /// module of its package, and whether that class's chain may have a
    /// native layout, and the names its MRO binds at class level
    /// (`package_bases::exceptions_from_package`).
    package_exceptions: BTreeMap<String, package_bases::PackageException>,
    /// v0.10: each class of the entry and the names the classes of other
    /// modules of its package deriving from it define
    /// (`package_bases::overridden_in_package`): a layout attribute one of
    /// them binds at class level is no layout's (`exc_layout`).
    package_overrides: BTreeMap<String, BTreeSet<String>>,
    /// v0.10: the classes of the entry a class of another module of its
    /// package with several bases derives from
    /// (`package_bases::multi_based_in_package`): none has a layout.
    package_multi: BTreeSet<String>,
    /// The nested `def`s and `lambda`s of compiled functions that CPython
    /// runs, their enclosing functions compiled (`nested`).
    nested: Vec<nested::Nested>,
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
        let hidden_members = if publishes.classes {
            exceptions::with_hidden_members(&module.body)
        } else {
            BTreeSet::new()
        };
        // An embedded build publishes its classes but not every member.
        let unpublished_members = if publishes.classes && publishes.infers_unannotated {
            members::with_unpublished_members(&module.body)
        } else {
            BTreeSet::new()
        };
        // v0.9: an `--ext` module publishes the function compiled code
        // makes of a module-level `name = lambda ...`, as it publishes a
        // `def`: Python code calls it compiled, with no Python frame.
        let lambda_aliases = if publishes.variables && !publishes.infers_unannotated {
            pycc_ast::module_lambda_aliases(&module.body)
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
                    Stmt::Assign(assign)
                        if matches!(assign.targets.as_slice(), [pycc_ast::Expr::Name(n)]
                            if lambda_aliases.contains(n.id.as_str())) =>
                    {
                        let [pycc_ast::Expr::Name(n)] = assign.targets.as_slice() else {
                            unreachable!("matched above")
                        };
                        Kind::Def(n.id.to_string(), true)
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
                // Any signature the published callable cannot bind alone:
                // a default, `*args`, `**kwargs`, a keyword-only or
                // positional-only parameter.
                defaults: matches!(stmt, Stmt::FunctionDef(def)
                    if pycc_ast::ext_needs_wrapper(&def.parameters)
                        && !wrapped.contains(def.name.as_str())),
                hidden_members: matches!(stmt, Stmt::ClassDef(class)
                    if hidden_members.contains(class.name.as_str())),
                unpublished_members: matches!(stmt, Stmt::ClassDef(class)
                    if unpublished_members.contains(class.name.as_str())),
                bases: names::base_names(stmt),
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
        let mut stmts = stmts;
        let mut ast: Vec<Stmt> = module.body.into_iter().collect();
        let top = ast.len();
        let mut lifts = Vec::new();
        let mut lifted_asts = Vec::new();
        let mut next = 0;
        for (index, stmt) in ast.iter().enumerate() {
            let mut found = lift::find(index, stmt, source, &mut next);
            if found.is_empty() {
                continue;
            }
            // Only an `--ext` module attaches a method to its compiled class
            // (an embedded build's fallback rewrite has no forwarders for it).
            if !publishes.infers_unannotated && let Stmt::ClassDef(class) = stmt {
                attach::analyze(class, &mut found);
            }
            // Once split, CPython runs the statement without the lifted
            // bodies, and reads the lifted functions instead.
            let starts: BTreeSet<u32> = found.iter().map(|(l, _)| l.range.start).collect();
            let stripped = lift::without_bodies(stmt, &starts);
            let owner = &mut stmts[index];
            owner.mentions = names::module_references(&stripped);
            owner.mentions.extend(found.iter().map(|(l, _)| l.name.clone()));
            owner.declares = names::all_global_declarations(&stripped);
            for (lift, analyzed) in found {
                let name = lift.name.clone();
                stmts.push(TopStmt {
                    range: lift.keyword..lift.range.end,
                    kind: Kind::Def(name.clone(), false),
                    bound: BTreeSet::from([name.clone()]),
                    mentions: names::module_references(&analyzed),
                    opaque: BTreeSet::from([name]),
                    declares: names::all_global_declarations(&analyzed),
                    defaults: false,
                    hidden_members: false,
                    unpublished_members: false,
                    bases: BTreeSet::new(),
                });
                lifts.push(lift);
                lifted_asts.push(analyzed);
            }
        }
        ast.extend(lifted_asts);
        Some(Self {
            source: source.to_string(),
            display: display.to_string(),
            ast,
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
            top,
            lifts,
            package_bases: BTreeMap::new(),
            package_exceptions: BTreeMap::new(),
            package_overrides: BTreeMap::new(),
            package_multi: BTreeSet::new(),
            nested: Vec::new(),
        })
    }

    /// Records the entry file's directory (see `entry_dir`).
    /// Records the entry file (`set_entry_dir`, and under
    /// `--foreign-relative-imports` the classes its package subclasses).
    pub(crate) fn set_entry(&mut self, path: &Path) {
        if self.publishes.foreign_relative_imports {
            self.package_bases = package_bases::subclassed_in_package(path);
        }
        if self.publishes.classes {
            self.package_exceptions = package_bases::exceptions_from_package(path);
            self.package_overrides = package_bases::overridden_in_package(path);
            self.package_multi = package_bases::multi_based_in_package(path);
        }
        self.set_entry_dir(path.parent().map(Path::to_path_buf));
    }

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
            // A type variable (`T = TypeVar("T")`), which compiled code only
            // records at compile time (`pycc_hir`'s `type_var`): CPython
            // makes the module's `T`, which `Generic[T]` and Python code read.
            Stmt::Assign(assign)
                if matches!(assign.targets.as_slice(), [pycc_ast::Expr::Name(_)])
                    && matches!(&*assign.value, pycc_ast::Expr::Call(call)
                        if matches!(&*call.func, pycc_ast::Expr::Name(f) if f.id.as_str() == "TypeVar")
                            || matches!(&*call.func, pycc_ast::Expr::Attribute(a) if a.attr.as_str() == "TypeVar")) =>
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
        if modules.iter().any(|m| self.is_local_module(m)) {
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
        modules.iter().any(|module| self.is_local_module(module))
    }

    /// Whether the absolute import of `module` names a module of the
    /// program's own: one found from its source root (the entry's directory,
    /// climbed past every package `__init__.py`, as the loader finds it), but
    /// not, under `--foreign-relative-imports`, one of the entry's own
    /// top-level package, which binds the CPython module (#1382).
    fn is_local_module(&self, module: &str) -> bool {
        let top = module.split('.').next().unwrap_or(module);
        let Some(dir) = &self.entry_dir else {
            return false;
        };
        let mut root = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        let mut package = None;
        while root.join("__init__.py").is_file() {
            package = root.file_name().map(|n| n.to_string_lossy().into_owned());
            if !root.pop() {
                break;
            }
        }
        if self.publishes.foreign_relative_imports && package.as_deref() == Some(top) {
            return false;
        }
        root.join(format!("{top}.py")).exists() || root.join(top).is_dir()
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
            self.nested.len(),
        );
        // The docstring and `from __future__` imports stay compiled: the
        // preamble goes after them.
        let first = self.preamble_position();
        // A nested `def` or `lambda` runs in CPython alone, its enclosing
        // function compiled (`nested`); the unit's other failures this
        // round may be consequences of it, so the rewrite is compiled
        // before any of them is absorbed.
        let mut nested_units = BTreeSet::new();
        // The exception classes this round makes run in CPython
        // (`exception_with_constructor`): every failure naming one is its.
        let mut constructed = BTreeSet::new();
        for failure in failures {
            if self.nest(rendered, failure)
                && let Some(unit) = self.owner_of(rendered, failure)
            {
                nested_units.insert(unit);
            }
        }
        for failure in failures {
            if self
                .owner_of(rendered, failure)
                .is_some_and(|unit| nested_units.contains(&unit))
            {
                continue;
            }
            if self.class_without_attribute(failure)
                || self.receiving_generators(failure)
                || self.exception_with_constructor(failure, &mut constructed)
            {
                continue;
            }
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
                    // An unsupported construct in the body of a `def` the
                    // statement lifts fails the lifted body too: that lift
                    // falls back at once (`lift`).
                    // A method of a compiled class that can attach to it
                    // (`attach`) runs in CPython alone, the class compiled.
                    if index < self.top
                        && failure.diagnostic.code == "C0001"
                        && let Some(at) = self.source_start(rendered, failure)
                        && let Some(unit) = self
                            .lifts_of(index)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .find(|unit| {
                                self.lift(*unit).is_some_and(|lift| lift.body.contains(&at))
                            })
                    {
                        self.fallback.entry(unit).or_insert_with(|| reason.clone());
                        if self.publishes.classes
                            && matches!(self.stmts[index].kind, Kind::Class(_))
                            && !self.fallback.contains_key(&index)
                            && self.lift(unit).is_some_and(|lift| lift.attach.is_some())
                        {
                            continue;
                        }
                    }
                    self.fallback.entry(index).or_insert(reason);
                }
                Some(_) => {}
                None => {
                    // A diagnostic no statement owns is usually the
                    // module-level solver's: the module-level statements
                    // fall back first, every statement only once they have.
                    let module_level: Vec<usize> = (first..self.top)
                        .filter(|index| {
                            matches!(self.stmts[*index].kind, Kind::Other | Kind::Import)
                                && !self.fallback.contains_key(index)
                        })
                        .collect();
                    let marked: Vec<usize> = if module_level.is_empty() {
                        (first..self.top).collect()
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
            self.nested.len(),
        ) != before
    }

    /// Makes the compiled class a `T0044` (`class `C` has no attribute
    /// named `x``) names run in CPython, returning whether it did: code
    /// reading or setting an attribute the class does not declare is code
    /// CPython would run on a Python instance (`o.extra = 1` from module
    /// code), which a compiled instance, with no `__dict__`, refuses.
    fn class_without_attribute(&mut self, failure: &Failure) -> bool {
        let message = failure.diagnostic.message.as_str();
        if failure.diagnostic.code != "T0044" {
            return false;
        }
        let Some(rest) = message.strip_prefix("class `") else {
            return false;
        };
        let Some((class, rest)) = rest.split_once('`') else {
            return false;
        };
        let Some(attr) = rest
            .strip_prefix(" has no attribute named `")
            .and_then(|r| r.split('`').next())
        else {
            return false;
        };
        let Some(index) = (0..self.top).find(|index| {
            matches!(&self.stmts[*index].kind, Kind::Class(name) if name == class)
        }) else {
            return false;
        };
        if self.fallback.contains_key(&index) {
            return false;
        }
        self.fallback.insert(
            index,
            format!("code outside the class uses an attribute `{attr}` it does not declare"),
        );
        true
    }

    /// Makes an exception class with a constructor of its own run in CPython
    /// when compiled code raises or constructs it (`C0001`: pycc raises an
    /// exception by its type and message only, so the class's own state
    /// would be lost), returning whether it did (this round, `constructed`).
    /// The code raising it stays compiled and raises the CPython class's
    /// instance, as CPython does.
    fn exception_with_constructor(
        &mut self,
        failure: &Failure,
        constructed: &mut BTreeSet<usize>,
    ) -> bool {
        if failure.diagnostic.code != "C0001" {
            return false;
        }
        // The class's own `__init__` calling `super().__init__(...)` on the
        // builtin exception (`pycc_types`' `super_call`) is the same
        // capability gap, met in its body rather than at a `raise`.
        let message = &failure.diagnostic.message;
        let class = if let Some(rest) = message.strip_prefix("exception class `") {
            let Some((class, rest)) = rest.split_once('`') else {
                return false;
            };
            if !rest.starts_with(" declares or inherits an `__init__`") {
                return false;
            }
            class
        } else if let Some(rest) =
            message.strip_prefix("`super().__init__(...)` in exception class `")
        {
            let Some((class, _)) = rest.split_once('`') else {
                return false;
            };
            class
        } else {
            return false;
        };
        let Some(index) = (0..self.top).find(|index| {
            matches!(&self.stmts[*index].kind, Kind::Class(name) if name == class)
        }) else {
            return false;
        };
        if constructed.contains(&index) {
            return true;
        }
        if self.fallback.contains_key(&index) {
            return false;
        }
        self.fallback.insert(
            index,
            "an exception class with a constructor of its own, which compiled code raises"
                .to_string(),
        );
        constructed.insert(index);
        true
    }

    /// Makes the compiled generator functions that receive sent values run
    /// in CPython on a `send()` of a value their send type excludes
    /// (`T0021`), returning whether any newly did. Sending from CPython
    /// instead (the statement falling back) would reach the same compiled
    /// generator, which receives only `None` from Python code; a generator
    /// CPython creates takes any value. A *receiving* generator is one whose
    /// `yield` is not a statement of its own (`x = (yield total)`); the
    /// diagnostic does not say which generator was sent to, so every one
    /// falls back.
    fn receiving_generators(&mut self, failure: &Failure) -> bool {
        if failure.diagnostic.code != "T0021"
            || !failure.diagnostic.message.starts_with("`send()` to a generator taking")
        {
            return false;
        }
        let receiving: Vec<usize> = (0..self.top)
            .filter(|index| !self.fallback.contains_key(index))
            .filter(|index| {
                let Stmt::FunctionDef(def) = &self.ast[*index] else {
                    return false;
                };
                let range = &self.stmts[*index].range;
                pycc_ast::body_yields(&def.body)
                    && self.source[range.start as usize..range.end as usize]
                        .lines()
                        .any(|line| line.contains("yield") && !line.trim_start().starts_with("yield"))
            })
            .collect();
        for &index in &receiving {
            self.fallback.insert(
                index,
                "code sends values to its generators, which a compiled generator receives only \
                 from compiled code"
                    .to_string(),
            );
        }
        !receiving.is_empty()
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
        if !matches!(self.stmts[index].kind, Kind::Def(..))
            || self.fallback.contains_key(&index)
            || index >= self.top
        {
            return false;
        }
        self.mirrored.insert(index);
        self.twinned.entry(index).or_insert_with(|| reason.to_string());
        true
    }

    /// The source line a failure's span starts on, when it has one in
    /// compiled source.
    fn source_line(&self, rendered: Option<&Rendered>, failure: &Failure) -> Option<usize> {
        Some(line_of(&self.source, self.source_start(rendered, failure)?))
    }

    /// The source offset a failure's span starts at, when it has one in
    /// compiled source.
    fn source_start(&self, rendered: Option<&Rendered>, failure: &Failure) -> Option<u32> {
        let span = failure.diagnostic.span.filter(|s| (s.start, s.end) != (0, 0))?;
        match rendered {
            Some(rendered) => rendered.source_offset(span.start),
            None => Some(span.start),
        }
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

#[cfg(test)]
#[path = "fallback/tests.rs"]
mod tests;
