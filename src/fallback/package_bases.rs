//! The entry module's classes that other modules of its package subclass
//! (`--ext --foreign-relative-imports`, #12).
//!
//! Since v0.9 Python code may subclass a published compiled class, and a
//! compiled call runs the subclass's overrides (`pycc_mir::dispatch`), so a
//! class some module of the same package subclasses (`class
//! H(GetCoreSchemaHandler)` in a sibling of `annotated_handlers`) stays
//! compiled -- except a compiled exception class, which runs in CPython, its
//! methods compiled (`lift`) (`crate::fallback::cascade`). The package's
//! modules are read from the source tree the artifact is built from.

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
    let mut found = BTreeMap::new();
    for (name, subclass, _, _) in package_subclasses(entry) {
        found.entry(name).or_insert(subclass);
    }
    found
}

/// v0.10 accept 5: the classes of `entry` that a class of another module
/// of its package with two bases or more, or a body off the exception
/// layout allowlist (`exc_layout::class_names`), derives from, directly or
/// through other such classes: an exception layout over one could meet
/// another base's in that class, or be bypassed by its hooks
/// (`exc_layout`), so none is made.
pub(super) fn multi_based_in_package(entry: &Path) -> BTreeSet<String> {
    package_subclasses(entry)
        .into_iter()
        .filter(|(_, _, _, multi)| *multi)
        .map(|(name, _, _, _)| name)
        .collect()
}

/// v0.10: the names (methods and class attributes) that the classes of
/// other modules of `entry`'s package deriving from each class of `entry`
/// -- directly or through other such classes, at any depth -- define in
/// their bodies: the members a compiled call on an instance of the class
/// must dispatch, since the instance may be one of those subclasses'
/// (`pycc_mir::dispatch`'s foreign arm). Syntactic: a name it lists that
/// no subclass overrides only costs that call a test.
pub(crate) fn overridden_in_package(entry: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, _, defines, _) in package_subclasses(entry) {
        out.entry(name).or_default().extend(defines);
    }
    out
}

/// Each class of `entry` that a class statement of another module of its
/// package lists as a base, with that subclass (`pkg.n.D`), the names the
/// subclass and its own subclasses (in any module but `entry`) define, and
/// whether one of them lists two bases or more.
fn package_subclasses(entry: &Path) -> Vec<(String, String, BTreeSet<String>, bool)> {
    let Some(package) = Package::read(entry) else {
        return Vec::new();
    };
    let (entry, target, parsed) = (&package.entry, &package.target, &package.parsed);
    let exports = re_exports(parsed);
    // Every class statement of the other modules: its resolved bases (as
    // `module.name`) and the names its body defines.
    let resolve = |base: &str| resolve_export(&exports, base);
    let mut classes: Vec<(String, Vec<String>, BTreeSet<String>, bool)> = Vec::new();
    for (file, qual, is_package, module) in parsed {
        if file == entry {
            continue;
        }
        let mut scan = Scan::new(qual, *is_package);
        scan.body(&module.body);
        let mut bases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut listed: BTreeMap<&str, usize> = BTreeMap::new();
        for (base, subclass) in &scan.bases {
            *listed.entry(subclass.as_str()).or_default() += 1;
            if let Some(base) = resolve(base) {
                bases.entry(subclass.clone()).or_default().push(base);
            }
        }
        for (class, defines) in scan.defines {
            let multi = listed.get(class.as_str()).is_some_and(|n| *n > 1)
                || scan.opaque.contains(&class);
            let bases = bases.remove(&class).unwrap_or_default();
            classes.push((class, bases, defines, multi));
        }
    }
    let prefix = format!("{target}.");
    let mut found = Vec::new();
    for (class, bases, defines, multi) in &classes {
        for base in bases {
            let Some(name) = base.strip_prefix(&prefix).filter(|n| !n.contains('.')) else {
                continue;
            };
            // The subclass's names, and those of every class deriving from
            // it, to a fixpoint.
            let mut reached: BTreeSet<&str> = BTreeSet::from([class.as_str()]);
            let mut names = defines.clone();
            let mut any_multi = *multi;
            loop {
                let before = reached.len();
                for (other, other_bases, other_defines, other_multi) in &classes {
                    if !reached.contains(other.as_str())
                        && other_bases.iter().any(|b| reached.contains(b.as_str()))
                    {
                        reached.insert(other);
                        names.extend(other_defines.iter().cloned());
                        any_multi |= *other_multi;
                    }
                }
                if reached.len() == before {
                    break;
                }
            }
            found.push((name.to_string(), class.clone(), names, any_multi));
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
    /// The names each class statement's body binds (its methods and class
    /// attributes), by the class's dotted name.
    defines: BTreeMap<String, BTreeSet<String>>,
    /// The class statements whose bodies are off the exception layout
    /// allowlist (`exc_layout::class_names`), by dotted name.
    opaque: BTreeSet<String>,
}

impl<'a> Scan<'a> {
    fn new(qual: &'a str, is_package: bool) -> Self {
        Self {
            qual,
            is_package,
            bound: BTreeMap::new(),
            bases: Vec::new(),
            defines: BTreeMap::new(),
            opaque: BTreeSet::new(),
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
                    // `Box[int]` subclasses `Box` (its `__mro_entries__`).
                    let base = match base {
                        Expr::Subscript(subscript) => &*subscript.value,
                        base => base,
                    };
                    if let Some(dotted) = self.dotted(base) {
                        self.bases
                            .push((dotted, format!("{}.{}", self.qual, class.name.as_str())));
                    }
                }
                if super::exc_layout::class_names(class).is_none() {
                    self.opaque
                        .insert(format!("{}.{}", self.qual, class.name.as_str()));
                }
                let defined = self
                    .defines
                    .entry(format!("{}.{}", self.qual, class.name.as_str()))
                    .or_default();
                for member in &class.body {
                    match member {
                        Stmt::FunctionDef(def) => {
                            defined.insert(def.name.to_string());
                        }
                        Stmt::Assign(assign) => {
                            for target in &assign.targets {
                                if let Expr::Name(name) = target {
                                    defined.insert(name.id.to_string());
                                }
                            }
                        }
                        Stmt::AnnAssign(assign) => {
                            if let Expr::Name(name) = &*assign.target {
                                defined.insert(name.id.to_string());
                            }
                        }
                        _ => {}
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

/// Each module's module-level re-exports: `(module, name)` -> the
/// `(module, name)` it imports, so `from pkg import C` resolves to the
/// module defining `C`, through a package `__init__.py` or any other module
/// re-exporting it (`pkg/__init__.py` importing from `pkg.init`, which
/// imports from `pkg.spec`).
type Exports = BTreeMap<(String, String), (String, String)>;

fn re_exports(parsed: &[(PathBuf, String, bool, pycc_ast::ModModule)]) -> Exports {
    let mut exports: Exports = BTreeMap::new();
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
    exports
}

/// The `module.name` the dotted name `base` reads, through a few
/// re-exports at most.
fn resolve_export(exports: &Exports, base: &str) -> Option<String> {
    let (module, name) = base.rsplit_once('.')?;
    let (mut module, mut name) = (module.to_string(), name.to_string());
    for _ in 0..4 {
        match exports.get(&(module.clone(), name.clone())) {
            Some((m, n)) if (m, n) != (&module, &name) => {
                module = m.clone();
                name = n.clone();
            }
            _ => break,
        }
    }
    Some(format!("{module}.{name}"))
}

/// v0.10 accept 5: an exception class of another module of the package, as
/// a class of the entry deriving from it sees it (`exc_layout`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PackageException {
    /// Whether it or an ancestor may have a native layout: a class deriving
    /// from it gets no layout of its own, since its instances must stay its
    /// base's.
    pub(super) may_layout: bool,
    /// The names bound at class level along its MRO (`None`: a class in it
    /// is of unknown body), which a layout over it leaves to CPython.
    pub(super) names: Option<BTreeSet<String>>,
}

/// v0.10 accept 5: the names `entry` binds to an exception class of another
/// module of its package (`from .common import PystacheError`), each with
/// what a class of `entry` deriving from it sees ([`PackageException`]).
/// Syntactic, like the rest of this module.
pub(super) fn exceptions_from_package(entry: &Path) -> BTreeMap<String, PackageException> {
    let mut out = BTreeMap::new();
    let Some(package) = Package::read(entry) else {
        return out;
    };
    let exports = re_exports(&package.parsed);
    let mut scans: BTreeMap<&str, (Scan<'_>, &pycc_ast::ModModule)> = BTreeMap::new();
    for (_, qual, is_package, module) in &package.parsed {
        let mut scan = Scan::new(qual, *is_package);
        scan.body(&module.body);
        scans.insert(qual.as_str(), (scan, module));
    }
    // What `module.name` is as an exception class (`None`: none).
    fn classify(
        scans: &BTreeMap<&str, (Scan<'_>, &pycc_ast::ModModule)>,
        exports: &Exports,
        dotted: &str,
        depth: usize,
    ) -> Option<PackageException> {
        let dotted = resolve_export(exports, dotted)?;
        let (module, name) = dotted.rsplit_once('.')?;
        if depth > 8 {
            return None;
        }
        let (scan, parsed) = scans.get(module)?;
        let class = parsed.body.iter().find_map(|stmt| match stmt {
            Stmt::ClassDef(class) if class.name.as_str() == name => Some(class),
            _ => None,
        })?;
        let mut exception = false;
        // Conservative: a store of a name a base binds is no layout
        // attribute, which only makes `may_layout` true more often.
        let mut may_layout =
            !super::exc_layout::fields_of(class, &BTreeSet::new()).is_empty();
        // `None` for a body off the layout allowlist.
        let mut names = super::exc_layout::class_names(class);
        for base in class.bases() {
            if let Expr::Name(base) = base
                && pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&base.id.as_str())
                && !scan.bound.contains_key(base.id.as_str())
            {
                exception = true;
                if super::exc_names::builtin_reduces_itself(base.id.as_str()) {
                    names = None;
                } else if let Some(names) = &mut names {
                    names.extend(
                        super::exc_names::builtin_exception_names(base.id.as_str())
                            .map(str::to_string),
                    );
                }
                continue;
            }
            match scan.dotted(base).and_then(|b| classify(scans, exports, &b, depth + 1)) {
                Some(base) => {
                    exception = true;
                    may_layout |= base.may_layout;
                    match (&mut names, base.names) {
                        (Some(names), Some(more)) => names.extend(more),
                        _ => names = None,
                    }
                }
                // A base of unknown kind may be a layout class.
                None => {
                    may_layout = true;
                    names = None;
                }
            }
        }
        exception.then_some(PackageException { may_layout, names })
    }
    let Some((scan, _)) = scans.get(package.target.as_str()) else {
        return out;
    };
    for (local, dotted) in &scan.bound {
        if dotted.starts_with(&format!("{}.", package.target)) {
            continue;
        }
        if let Some(class) = classify(&scans, &exports, dotted, 0) {
            out.insert(local.clone(), class);
        }
    }
    out
}

#[cfg(test)]
#[path = "package_bases_tests.rs"]
mod tests;
