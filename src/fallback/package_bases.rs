//! The entry module's classes that other modules of its package subclass
//! (`--ext --foreign-relative-imports`, #12).
//!
//! An `--ext` module publishes a compiled class as a type CPython does not
//! accept as a base: every `self.m()` in a compiled body is bound to the
//! compiled method at compile time, so an override in a Python subclass
//! would be ignored. A class some module of the same package subclasses
//! (`class H(GetCoreSchemaHandler)` in a sibling of `annotated_handlers`)
//! therefore runs in CPython, its methods compiled (`lift`), and is a real
//! Python class. The package's modules are read from the source tree the
//! artifact is built from. A subclass outside the package, or one whose
//! base is chosen at run time, is not seen: it still raises CPython's
//! `TypeError` (not an acceptable base type) at class creation, never runs
//! wrongly (`docs/DEVIATIONS.md`).

use pycc_ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The classes of `entry` (a module of a package) that a class statement of
/// another module of that package, at any depth of the package, lists as a
/// base -- imported by name (`from .m import C as D`, `from pkg.m import
/// C`, through a package `__init__.py`'s re-export), or read as an attribute
/// (`import pkg.m` then `pkg.m.C`, `from . import m` then `m.C`) -- each
/// with the first subclass found (`pkg.n.D`), for the report.
pub(super) fn subclassed_in_package(entry: &Path) -> BTreeMap<String, String> {
    let Some(package) = Package::read(entry) else {
        return BTreeMap::new();
    };
    let (entry, target, parsed) = (&package.entry, &package.target, &package.parsed);
    // Each module's module-level re-exports: `(module, name)` -> the
    // `(module, name)` it imports, so `from pkg import C` resolves to the
    // module defining `C`, through a package `__init__.py` or any other
    // module re-exporting it (`pkg/__init__.py` importing from `pkg.init`,
    // which imports from `pkg.spec`).
    let mut exports: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    for (_, qual, is_package, module) in parsed {
        let scan = Scan::new(qual, *is_package);
        for stmt in &module.body {
            if let Stmt::ImportFrom(import) = stmt
                && let Some(from) =
                    scan.resolve(import.level, import.module.as_ref().map(|m| m.as_str()))
            {
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    exports.insert((qual.clone(), local), (from.clone(), alias.name.to_string()));
                }
            }
        }
    }
    let mut found = BTreeMap::new();
    for (file, qual, is_package, module) in parsed {
        if file == entry {
            continue;
        }
        let mut scan = Scan::new(qual, *is_package);
        scan.body(&module.body);
        for (base, subclass) in scan.bases {
            let Some((module, name)) = base.rsplit_once('.') else {
                continue;
            };
            let (mut module, mut name) = (module.to_string(), name.to_string());
            // Through a few re-exports at most.
            for _ in 0..4 {
                match exports.get(&(module.clone(), name.clone())) {
                    Some((m, n)) if (m, n) != (&module, &name) => {
                        module = m.clone();
                        name = n.clone();
                    }
                    _ => break,
                }
            }
            if module == *target {
                found.entry(name).or_insert(subclass);
            }
        }
    }
    found
}

/// The entry module's package, its modules parsed.
struct Package {
    /// The entry file, canonical.
    entry: PathBuf,
    /// The entry module's dotted name.
    target: String,
    /// Every module of the package: its file, dotted name, whether it is a
    /// package `__init__.py`, and its syntax tree.
    parsed: Vec<(PathBuf, String, bool, pycc_ast::ModModule)>,
}

impl Package {
    /// The package of `entry`, or `None` when it is not in one; read once
    /// per entry by a build, which asks for it from every fallback round.
    fn read(entry: &Path) -> Option<std::sync::Arc<Self>> {
        type Cache = Vec<(PathBuf, Option<std::sync::Arc<Package>>)>;
        static READ: std::sync::Mutex<Cache> = std::sync::Mutex::new(Vec::new());
        let mut cache = READ.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, package)) = cache.iter().find(|(path, _)| path == entry) {
            return package.clone();
        }
        let package = Self::parse(entry).map(std::sync::Arc::new);
        cache.push((entry.to_path_buf(), package.clone()));
        package
    }

    fn parse(entry: &Path) -> Option<Self> {
        let entry = entry.canonicalize().ok()?;
        let (root, top) = package_root(&entry)?;
        let target = qualname(&root, &entry)?;
        let mut files = Vec::new();
        collect_py(&root.join(&top), &mut files);
        let mut parsed = Vec::new();
        for file in files {
            let Some(qual) = qualname(&root, &file) else {
                continue;
            };
            let Some(source) = std::fs::read(&file)
                .ok()
                .and_then(|bytes| crate::source::decode_python_source(&bytes).ok())
            else {
                continue;
            };
            let Ok(module) = pycc_parser::parse_all(&source) else {
                continue;
            };
            let is_package = file.file_name().is_some_and(|n| n == "__init__.py");
            parsed.push((file, qual, is_package, module));
        }
        Some(Self { entry, target, parsed })
    }
}

/// The attribute names `entry` and, in a package built one extension per
/// module, the package's other modules use on an object other than a
/// method's own `self` (`state.tokens`, `m.f`), loaded, stored or deleted:
/// compiled code holding an instance of a class of `entry` as a CPython
/// object (fallback code, a lifted body, another module) reaches it
/// through CPython's attribute protocol, so its slots of these names must
/// be ones CPython can see (`pycc_hir::set_external_attribute_reads`).
pub(crate) fn attributes_used_in_package(entry: &Path, package: bool) -> BTreeSet<String> {
    use pycc_ast::visitor::{self, Visitor};
    struct Names(BTreeSet<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Attribute(attr) = expr
                && !matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == "self")
            {
                self.0.insert(attr.attr.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut names = Names(BTreeSet::new());
    let own = std::fs::read(entry)
        .ok()
        .and_then(|bytes| crate::source::decode_python_source(&bytes).ok())
        .and_then(|source| pycc_parser::parse_all(&source).ok());
    for stmt in own.iter().flat_map(|module| module.body.iter()) {
        names.visit_stmt(stmt);
    }
    if package && let Some(package) = Package::read(entry) {
        for (file, _, _, module) in &package.parsed {
            if *file == package.entry {
                continue;
            }
            for stmt in &module.body {
                names.visit_stmt(stmt);
            }
        }
    }
    names.0
}

/// The directory holding the entry's top-level package, and that package's
/// name: the entry's directory climbed past every `__init__.py`.
fn package_root(entry: &Path) -> Option<(PathBuf, String)> {
    let mut dir = entry.parent()?.to_path_buf();
    let mut top = None;
    while dir.join("__init__.py").is_file() {
        top = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        if !dir.pop() {
            break;
        }
    }
    Some((dir, top?))
}

/// `pkg.sub.m` for `root/pkg/sub/m.py` (`pkg.sub` for its `__init__.py`).
fn qualname(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let last = parts.pop()?;
    let stem = last.strip_suffix(".py")?;
    if stem != "__init__" {
        parts.push(stem.to_string());
    }
    Some(parts.join("."))
}

fn collect_py(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_py(&path, out);
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "py") {
            out.push(path.canonicalize().unwrap_or(path));
        }
    }
}

/// One module's imports and class bases.
struct Scan<'a> {
    /// The scanned module's dotted name.
    qual: &'a str,
    is_package: bool,
    /// Local name -> the dotted name it binds (`pkg.m` for a module,
    /// `pkg.m.C` for a name imported from one).
    bound: BTreeMap<String, String>,
    /// Every base of a class statement as a dotted name, with the class
    /// listing it.
    bases: Vec<(String, String)>,
}

impl<'a> Scan<'a> {
    fn new(qual: &'a str, is_package: bool) -> Self {
        Self {
            qual,
            is_package,
            bound: BTreeMap::new(),
            bases: Vec::new(),
        }
    }

    /// The absolute module a `from` import at `level` of `module` names.
    fn resolve(&self, level: u32, module: Option<&str>) -> Option<String> {
        if level == 0 {
            return module.map(str::to_string);
        }
        let mut parts: Vec<&str> = self.qual.split('.').collect();
        if !self.is_package {
            parts.pop();
        }
        for _ in 1..level {
            parts.pop()?;
        }
        if let Some(module) = module {
            parts.extend(module.split('.'));
        }
        Some(parts.join("."))
    }

    /// The dotted name `expr` (a name or an attribute chain) reads.
    fn dotted(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) => Some(
                self.bound
                    .get(name.id.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("{}.{}", self.qual, name.id.as_str())),
            ),
            Expr::Attribute(attr) => Some(format!("{}.{}", self.dotted(&attr.value)?, attr.attr)),
            _ => None,
        }
    }

    fn body(&mut self, body: &[Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ImportFrom(import) => {
                let Some(module) =
                    self.resolve(import.level, import.module.as_ref().map(|m| m.as_str()))
                else {
                    return;
                };
                for alias in &import.names {
                    let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    self.bound
                        .insert(local, format!("{module}.{}", alias.name.as_str()));
                }
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let name = alias.name.as_str();
                    match &alias.asname {
                        Some(asname) => {
                            self.bound.insert(asname.to_string(), name.to_string());
                        }
                        None => {
                            let root = name.split('.').next().unwrap_or(name);
                            self.bound.insert(root.to_string(), root.to_string());
                        }
                    }
                }
            }
            Stmt::ClassDef(class) => {
                for base in class.bases() {
                    if let Some(dotted) = self.dotted(base) {
                        self.bases
                            .push((dotted, format!("{}.{}", self.qual, class.name.as_str())));
                    }
                }
                self.body(&class.body);
            }
            Stmt::FunctionDef(def) => self.body(&def.body),
            Stmt::If(s) => {
                self.body(&s.body);
                for clause in &s.elif_else_clauses {
                    self.body(&clause.body);
                }
            }
            Stmt::For(s) => {
                self.body(&s.body);
                self.body(&s.orelse);
            }
            Stmt::While(s) => {
                self.body(&s.body);
                self.body(&s.orelse);
            }
            Stmt::With(s) => self.body(&s.body),
            Stmt::Try(s) => {
                self.body(&s.body);
                for handler in &s.handlers {
                    let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                    self.body(&h.body);
                }
                self.body(&s.orelse);
                self.body(&s.finalbody);
            }
            _ => {}
        }
    }
}
