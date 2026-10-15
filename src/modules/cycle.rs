//! Circular imports between project modules (ROADMAP v0.4 accept 5).
//!
//! When `a` imports `b` and `b` imports `a` back, CPython hands `b` the
//! partially initialized `a`: the names `a` bound before its `import b`
//! exist, the rest do not yet. The program is linked to run each module
//! body once, at its first import (`pycc_hir::program::order`), so `b`'s
//! body already runs right after that prefix of `a`. What is missing is
//! `a`'s HIR, which `b` is lowered before: a [`Provisional`] stands in for
//! it, read from `a`'s source -- the qualified spelling and kind of every
//! top-level name (what `b` binds and renames), and which of them `a` has
//! bound when it reaches the waiting import.
//!
//! `b` then behaves as in CPython: a function or variable of `a` that `b`
//! reads later (in a function) is reached directly; `from a import x` of a
//! name `a` has not bound yet raises CPython's `ImportError` (with its
//! message, the "partially initialized module" one or, for a module next to
//! the entry script, the "consider renaming" one), and the code after it
//! never runs (a module-level read of the name it would have bound is
//! refused, `C0001`, as the checker cannot type a global read before its
//! binding); a module-level read of such a name through `a.x` (CPython's
//! `AttributeError`) and any class or type alias of `a` are refused
//! (`C0001`). The entry module importing itself back is still `E0108`:
//! CPython would run a second copy of it. `docs/DEVIATIONS.md` lists what
//! remains.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::Path;

use pycc_ast::namespace::{self, CELL_PREFIX, module_bindings, statement_bindings};
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ModModule, Stmt};
use pycc_hir::ProjectBindingKind;

use super::namespace::{Export, Kind};

/// Origins at or above this value name a [`Provisional`] (by its index
/// above it) rather than a loaded module.
pub(crate) const PROVISIONAL: usize = usize::MAX / 2;

/// A module still running its body, as a module it imports sees it.
#[derive(Debug, Clone)]
pub(crate) struct Provisional {
    pub(crate) qual: String,
    pub(crate) display_path: String,
    /// What each top-level name is, as [`super::namespace::exports`] would
    /// say once the module is lowered (its origin is this provisional's).
    pub(crate) exports: HashMap<String, Export>,
    /// The names its body has bound at the waiting import.
    pub(crate) bound: BTreeSet<String>,
    /// The file CPython's messages name (`__spec__.origin`).
    file: String,
    /// Whether CPython's "consider renaming" hint replaces the circular
    /// import one: the module (or its package) sits in the entry script's
    /// directory, `sys.path[0]`.
    shadowing: bool,
    /// Whether the module has finished loading: it is then reached as a
    /// loaded module, never through this.
    pub(crate) done: bool,
}

impl Provisional {
    /// `parsed` is the module's desugared source, `slot` the index of this
    /// provisional, `offset` where its body waits (the import being
    /// resolved), `file` its canonical path and `entry_dir` the entry
    /// script's canonical directory.
    pub(crate) fn new(
        parsed: &ModModule,
        qual: &str,
        display_path: &str,
        slot: usize,
        offset: u32,
        file: &Path,
        entry_dir: &Path,
    ) -> Self {
        let kinds = top_level_kinds(&parsed.body);
        let mut exports = HashMap::new();
        for name in module_bindings(parsed).defined {
            let kind = kinds.get(name.as_str()).copied().unwrap_or(Kind::Variable);
            let (exported, cell) = match name.strip_prefix(CELL_PREFIX) {
                Some(source) => (source.to_string(), true),
                None => (name.clone(), false),
            };
            exports.insert(
                exported,
                Export::Def {
                    name: namespace::module_global(qual, &name),
                    origin: PROVISIONAL + slot,
                    kind,
                    cell,
                },
            );
        }
        let mut bound = BTreeSet::new();
        for stmt in &parsed.body {
            if pycc_ast::stmt_range(stmt).end > offset {
                break;
            }
            for name in statement_bindings(stmt) {
                bound.insert(match name.strip_prefix(CELL_PREFIX) {
                    Some(source) => source.to_string(),
                    None => name,
                });
            }
        }
        let package_or_file = match file.file_name().and_then(|name| name.to_str()) {
            Some("__init__.py") => file.parent().unwrap_or(file),
            _ => file,
        };
        Provisional {
            qual: qual.to_string(),
            display_path: display_path.to_string(),
            exports,
            bound,
            file: file.to_string_lossy().into_owned(),
            shadowing: package_or_file.parent() == Some(entry_dir),
            done: false,
        }
    }

    /// The kind of every qualified name, for `pycc_hir`'s synthesized
    /// import of this module.
    pub(crate) fn binding_kinds(&self) -> Vec<(String, ProjectBindingKind)> {
        self.exports
            .values()
            .filter_map(|export| match export {
                Export::Def { name, kind, .. } => Some((
                    name.clone(),
                    match kind {
                        Kind::Function => ProjectBindingKind::Function,
                        Kind::Class => ProjectBindingKind::Class,
                        Kind::Alias => ProjectBindingKind::TypeAlias,
                        Kind::Variable => ProjectBindingKind::Variable,
                    },
                )),
                _ => None,
            })
            .collect()
    }

    /// CPython's message for `from <module> import <name>` while the
    /// module has not bound `name` (`_PyEval_ImportFrom`, 3.13).
    pub(crate) fn import_error(&self, name: &str) -> String {
        if self.shadowing {
            format!(
                "cannot import name {} from {} (consider renaming {} if it has the same name \
                 as a library you intended to import)",
                repr(name),
                repr(&self.qual),
                repr(&self.file)
            )
        } else {
            format!(
                "cannot import name {} from partially initialized module {} (most likely due \
                 to a circular import) ({})",
                repr(name),
                repr(&self.qual),
                self.file
            )
        }
    }
}

impl super::Loader {
    /// Makes the module waiting at `in_progress[position]` reachable as
    /// partially initialized: `false` for the entry module.
    pub(super) fn reach_partially_initialized(&mut self, position: usize) -> bool {
        let waiting = &self.in_progress[position];
        let (Some(qual), Some(parsed)) = (&waiting.qual, &waiting.parsed) else {
            return false;
        };
        let slot = self
            .provisional
            .iter()
            .position(|module| !module.done && module.qual == *qual)
            .unwrap_or(self.provisional.len());
        let module = Provisional::new(
            parsed,
            qual,
            &waiting.display,
            slot,
            waiting.offset,
            &waiting.path,
            &self.entry_dir,
        );
        if slot == self.provisional.len() {
            self.provisional.push(module);
        } else {
            self.provisional[slot] = module;
        }
        true
    }

    /// Module `qual` finished loading as `index`: every export a module
    /// lowered meanwhile recorded as reaching it while partially
    /// initialized now reaches it as loaded.
    pub(super) fn finish_provisional(&mut self, qual: &str, index: usize) {
        let Some(slot) = self
            .provisional
            .iter()
            .position(|module| !module.done && module.qual == qual)
        else {
            return;
        };
        self.provisional[slot].done = true;
        for info in &mut self.infos {
            for export in info.exports.values_mut() {
                if let Export::Def { origin, .. } = export
                    && *origin == PROVISIONAL + slot
                {
                    *origin = index;
                }
            }
        }
    }
}

/// The kind of each top-level `def`, `class` and type alias.
fn top_level_kinds(body: &[Stmt]) -> HashMap<&str, Kind> {
    let mut kinds = HashMap::new();
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => {
                kinds.entry(def.name.as_str()).or_insert(Kind::Function);
            }
            Stmt::ClassDef(class) => {
                kinds.insert(class.name.as_str(), Kind::Class);
            }
            Stmt::TypeAlias(alias) => {
                if let Expr::Name(name) = &*alias.name {
                    kinds.insert(name.id.as_str(), Kind::Alias);
                }
            }
            Stmt::AnnAssign(assign) => {
                let type_alias = match &*assign.annotation {
                    Expr::Name(name) => name.id.as_str() == "TypeAlias",
                    Expr::Attribute(attr) => attr.attr.as_str() == "TypeAlias",
                    _ => false,
                };
                if type_alias && let Expr::Name(name) = &*assign.target {
                    kinds.insert(name.id.as_str(), Kind::Alias);
                }
            }
            _ => {}
        }
    }
    kinds
}

/// Python's `repr()` of a `str`.
fn repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch == quote => {
                out.push('\\');
                out.push(ch);
            }
            ch => out.push(ch),
        }
    }
    out.push(quote);
    out
}

/// Every `root.attr` read that runs while the module body runs (outside
/// `def` bodies and lambdas) whose `root` is one of `roots`: its range,
/// root and attribute.
pub(crate) fn module_level_reads(
    body: &[Stmt],
    roots: &BTreeSet<String>,
) -> Vec<(Range<u32>, String, String)> {
    struct Reads<'r> {
        roots: &'r BTreeSet<String>,
        found: Vec<(Range<u32>, String, String)>,
    }
    impl<'a> Visitor<'a> for Reads<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    for decorator in &def.decorator_list {
                        self.visit_expr(&decorator.expression);
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::Attribute(attr) => match &*attr.value {
                    Expr::Name(root) if self.roots.contains(root.id.as_str()) => {
                        let range = std::ops::Range::<u32>::from(attr.range);
                        self.found
                            .push((range, root.id.to_string(), attr.attr.to_string()));
                    }
                    _ => visitor::walk_expr(self, expr),
                },
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut reads = Reads {
        roots,
        found: Vec::new(),
    };
    for stmt in body {
        reads.visit_stmt(stmt);
    }
    reads.found
}

/// Every read of one of `names` that runs while the module body runs
/// (outside `def` bodies and lambdas): its range and name.
pub(crate) fn module_level_names(
    body: &[Stmt],
    names: &BTreeSet<String>,
) -> Vec<(Range<u32>, String)> {
    struct Reads<'r> {
        names: &'r BTreeSet<String>,
        found: Vec<(Range<u32>, String)>,
    }
    impl<'a> Visitor<'a> for Reads<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    for decorator in &def.decorator_list {
                        self.visit_expr(&decorator.expression);
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::Name(name)
                    if name.ctx == pycc_ast::ExprContext::Load
                        && self.names.contains(name.id.as_str()) =>
                {
                    let range = std::ops::Range::<u32>::from(name.range);
                    self.found.push((range, name.id.to_string()));
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    if names.is_empty() {
        return Vec::new();
    }
    let mut reads = Reads {
        names,
        found: Vec::new(),
    };
    for stmt in body {
        reads.visit_stmt(stmt);
    }
    reads.found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_quotes_like_python() {
        assert_eq!(repr("a"), "'a'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a'\"b\\"), "'a\\'\"b\\\\'");
    }

    #[test]
    fn a_provisional_module_knows_what_it_bound_before_the_import() {
        let source = "X = 1\n\ndef f() -> int:\n    return 1\n\nimport b\nY = 2\n\n\
                      class C:\n    pass\n";
        let parsed = pycc_parser::parse_all(source).expect("parses");
        let offset = source.find("import b").expect("present") as u32;
        let module = Provisional::new(
            &parsed,
            "a",
            "a.py",
            0,
            offset,
            Path::new("/src/a.py"),
            Path::new("/src"),
        );
        let bound: Vec<&str> = module.bound.iter().map(String::as_str).collect();
        assert_eq!(bound, ["X", "f"]);
        assert!(matches!(
            module.exports.get("C"),
            Some(Export::Def { kind: Kind::Class, origin, .. }) if *origin == PROVISIONAL
        ));
        assert_eq!(
            module.import_error("Y"),
            "cannot import name 'Y' from 'a' (consider renaming '/src/a.py' if it has the same \
             name as a library you intended to import)"
        );
        let nested = Provisional::new(
            &parsed,
            "pkg.a",
            "pkg/a.py",
            1,
            offset,
            Path::new("/src/pkg/a.py"),
            Path::new("/src"),
        );
        assert_eq!(
            nested.import_error("Y"),
            "cannot import name 'Y' from partially initialized module 'pkg.a' (most likely due \
             to a circular import) (/src/pkg/a.py)"
        );
    }

    #[test]
    fn module_level_reads_skip_function_bodies() {
        let source = "x = a.B\n\nclass K:\n    y = a.C\n\ndef f() -> int:\n    return a.D\n\n\
                      g = lambda: a.E\n";
        let parsed = pycc_parser::parse_all(source).expect("parses");
        let roots: BTreeSet<String> = ["a".to_string()].into();
        let attrs: Vec<String> = module_level_reads(&parsed.body, &roots)
            .into_iter()
            .map(|(_, _, attr)| attr)
            .collect();
        assert_eq!(attrs, ["B", "C"]);
    }
}
