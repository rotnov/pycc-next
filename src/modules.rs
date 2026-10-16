//! The driver's project-module loader (#898, Part 1 of #881, D-222).
//!
//! `pycc_hir` is deliberately filesystem-free: it publishes
//! [`pycc_hir::ProjectImportRequest`]s for the imports its own `pycc_std`
//! registry does not answer, and consumes the driver's answers through
//! [`pycc_hir::ResolvedImports`]. Everything filesystem-shaped -- finding
//! the source root, mapping a dotted module name to a file, reading and
//! decoding it, ordering package `__init__.py` side effects, detecting
//! import cycles -- lives here.
//!
//! The loader is a depth-first walk from the entry file. Each module is
//! parsed, its project imports resolved (loading each dependency first),
//! and then lowered with `pycc_hir::lower_module`; the finished modules
//! come out in dependency order with the entry last, which is exactly the
//! order `pycc_hir::link` wants.
//!
//! Source-root discovery is *lazy*: a file with no project import never
//! touches the filesystem beyond its own read, which is what keeps the
//! `check_frontend_throughput.rb` gate honest.

mod cycle;
mod namespace;
mod probe;

use crate::frontend::FrontendFailure;
use crate::project_config;
use crate::source;
use cycle::{PROVISIONAL, Provisional};
use namespace::{Imported, ModuleInfo};

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    LoweredModule, ProjectImportRequest, ResolvedImport, ResolvedImports, ResolvedModule,
};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// One loaded project module: the path diagnostics render for it, its
/// decoded source (kept so a later pass can render against it), and its
/// lowering.
pub(crate) struct LoadedModule {
    pub(crate) display_path: String,
    pub(crate) source: String,
    pub(crate) module: LoweredModule,
    /// The module exactly as it was lowered (desugared, and renamed for
    /// per-module namespaces), for passes that locate items in it.
    pub(crate) parsed: pycc_ast::ModModule,
    /// Where the module's body runs: spliced into module `.0` at source
    /// offset `.1`, the import that first ran it (`None` for the entry).
    pub(crate) placement: Option<(usize, u32)>,
}

/// Every module reachable from the entry file, in dependency order with
/// the entry file last (post-order of the import walk), which is the order
/// `pycc_hir::link` expects and the order top-level statements run in.
pub(crate) struct LoadedProgram {
    pub(crate) modules: Vec<LoadedModule>,
    /// The `pycc.toml` source-root discovery found and parsed, if it ran and
    /// found one (#1224).
    pub(crate) manifest: Option<DiscoveredManifest>,
}

/// The nearest `pycc.toml` above the entry file, as source-root discovery
/// found and parsed it (#1224), kept so the interop policy reads the same
/// file -- under the same display path -- that every other manifest error
/// already renders.
///
/// `None` in [`LoadedProgram::manifest`] means "no manifest" only because
/// every `ImportBinding::Foreign` the policy can see depends on discovery
/// having run: the loader answers `Resolution::Foreign` for a non-relative
/// base, which always comes from `Loader::source_root`, and otherwise only
/// for the entry module's relative imports and, since #1382, its absolute
/// imports rooted at its own top-level package under
/// [`RelativeImports::ForeignFromEntry`] (#1366). That mode is passed only
/// by `pycc build --ext`, which discards the manifest and never runs the
/// policy, so discovery being skipped for such an import is unobservable
/// there. A future change that produced a policy-visible foreign binding
/// without discovery would make the policy fall back to `auto` silently;
/// `tests/issue_1224_interop_policy.rs`'s configured-`deny` tests pin that
/// dependency.
pub(crate) struct DiscoveredManifest {
    /// The manifest path as diagnostics render it (relative to the entry's
    /// own spelling, never the canonical absolute path).
    pub(crate) display: String,
    pub(crate) config: project_config::PyccToml,
}

/// The source root a dotted absolute module name resolves against, in both
/// spellings: the canonical path filesystem probes use, and the as-typed
/// path diagnostics render (see [`display_root`]).
#[derive(Clone)]
struct RootInfo {
    canonical: PathBuf,
    display: PathBuf,
}

/// What the loader decided about one [`ProjectImportRequest`]. Owned (not
/// borrowing the module store) so the resolution phase can keep mutating
/// that store while the answers accumulate.
enum Resolution {
    /// The dependency loaded (or, for `from pkg import sub` inside the
    /// package being initialized, only its submodules did): `loaded` is
    /// every module this request loaded or reached, the package
    /// `__init__.py`s on the way included (issue #1188), and `imported`
    /// what the request names, for the namespace rewrite.
    Loaded {
        loaded: Vec<usize>,
        imported: Imported,
    },
    /// A bare `import m` nested in a module-level `if`/`try` block naming a
    /// real project module: recognized, not loaded (a conditionally run
    /// module body is not something static linking can express).
    Found,
    /// The import cannot be satisfied, with the exact diagnostic to report.
    NotFound { code: &'static str, message: String },
    /// Not a project import after all (an absolute module name that
    /// resolves nowhere on disk and is not [`Resolution::Foreign`]: a
    /// dotted name under a project module or package): left unanswered so
    /// `pycc_hir` reports it exactly as a single-file compilation would.
    Unanswered,
    /// A bare `import X` -- or one name `X` of `import X, Y` (#1280), or
    /// `import X as Y` (#1291), or the module of `from X import a, b`
    /// (#1278), at top level or nested in a module-level `if`/`try` body
    /// (#1383) -- whose single-segment absolute root is neither a
    /// project module nor a `pycc_std` one (Part 1 of #1026), or the dotted
    /// module of `from X.Y import a` whose root `X` is neither a project
    /// module nor a project package (Part 1 of #1138), or a plain dotted
    /// `import X.Y` / `import X.Y as Z` under such a root (#1381): the name is
    /// taken to be a CPython module the produced extension imports at
    /// module-exec time, and `pycc_hir` binds it, or each imported name of
    /// it, as an opaque object (`ImportBinding::Foreign`). Whether that module actually exists is
    /// not knowable here -- the answer lives in the `sys.path` of the
    /// interpreter that loads the artifact, so the failure is a runtime
    /// `ModuleNotFoundError`, never a compile-time diagnostic.
    ///
    /// Under [`RelativeImports::ForeignFromEntry`] (#1366) it is also the
    /// answer to every relative `from` import of the entry module, dotted or
    /// not, without any filesystem probe: the names bind attributes of the
    /// package the artifact is imported under, resolved when its `Py_mod_exec`
    /// runs. So it is, again without a probe, to every absolute import of the
    /// entry module rooted at the entry's own top-level package (#1382).
    Foreign,
}

/// How the loader answers the entry module's relative imports (#1366), and
/// since #1382 its absolute imports rooted at its own top-level package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelativeImports {
    /// D-222: a relative import is a project import, resolved against the
    /// importer's own directory. Every command but `pycc build --ext
    /// --foreign-relative-imports` uses this.
    Project,
    /// `pycc build --ext --foreign-relative-imports`: the *entry* module's
    /// relative imports are answered [`Resolution::Foreign`] without
    /// touching the disk, and bind CPython objects of the package the
    /// artifact is imported under; so are its absolute imports rooted at its
    /// own top-level package (#1382). A dependency's relative imports keep
    /// D-222: its body runs inside the entry module's `Py_mod_exec`, so a
    /// foreign relative import there would resolve against the entry's
    /// package, not its own.
    ForeignFromEntry,
}

/// Loads the whole program reachable from `entry`, with relative imports as
/// project imports (D-222). Test-only: production callers choose the mode
/// through [`load_with`], which documents the contract.
#[cfg(test)]
pub(crate) fn load(
    entry: &Path,
    entry_module_name: Option<&str>,
) -> Result<LoadedProgram, FrontendFailure> {
    load_with(
        entry,
        entry_module_name,
        RelativeImports::Project,
        false,
        false,
    )
}

/// Loads the whole program reachable from `entry`.
///
/// `entry_module_name` is the `__name__` value the *entry* module is compiled
/// with (W0 of #882, #1156); every module loaded recursively from it is a
/// dependency and receives `None` here: its own `__name__` is a renamed
/// global of its own namespace that the rewrite binds to its dotted module
/// name (`namespace`, v0.3 accept 1).
///
/// `relative_imports` is how the entry module's relative imports (#1366),
/// and since #1382 its absolute imports rooted at its own top-level package,
/// resolve: [`RelativeImports::Project`] everywhere but a `pycc build --ext
/// --foreign-relative-imports`.
///
/// `ext_module` is whether the program is compiled into an `ext` artifact
/// (D-258 rule 1, #1397). It reaches every module the program loads, the
/// entry and each dependency alike, through
/// [`ResolvedImports::set_ext_module`]: `Any`, `object` and the
/// object-carrying container annotations lower to the CPython object in
/// every module of an `ext` artifact and keep their refusals everywhere else.
pub(crate) fn load_with(
    entry: &Path,
    entry_module_name: Option<&str>,
    relative_imports: RelativeImports,
    ext_module: bool,
    non_strict: bool,
) -> Result<LoadedProgram, FrontendFailure> {
    let display = entry.to_string_lossy().into_owned();
    let canonical = canonicalize(entry, &display)?;
    let mut entry_dir = canonical.clone();
    entry_dir.pop();
    let mut entry_display_dir = PathBuf::from(&display);
    entry_display_dir.pop();
    let mut loader = Loader {
        modules: Vec::new(),
        infos: Vec::new(),
        by_qual: HashMap::new(),
        current_import: None,
        placements: Vec::new(),
        memo: HashMap::new(),
        in_progress: Vec::new(),
        provisional: Vec::new(),
        entry_dir,
        entry_display_dir,
        root: None,
        manifest: None,
        // An artifact built to live in its package is imported under its
        // dotted name, which is what its `__name__` reads in CPython
        // (`pkg/m.abi3.so` is `pkg.m`).
        entry_module_name: match relative_imports {
            RelativeImports::ForeignFromEntry => entry_module_name.map(|name| {
                dotted_entry_name(&spelled_path(entry, &canonical))
                    .unwrap_or_else(|| name.to_string())
            }),
            RelativeImports::Project => entry_module_name.map(str::to_string),
        },
        entry_package: match relative_imports {
            RelativeImports::ForeignFromEntry => {
                top_level_package(&spelled_path(entry, &canonical))
            }
            RelativeImports::Project => None,
        },
        relative_imports,
        ext_module,
        non_strict,
    };
    loader.load_module(&canonical, display, true, None)?;
    let mut modules = loader.modules;
    for (module, placement) in modules.iter_mut().zip(loader.placements) {
        module.placement =
            placement.and_then(|(parent, offset)| Some((*loader.memo.get(&parent)?, offset)));
    }
    Ok(LoadedProgram {
        modules,
        manifest: loader.manifest,
    })
}

struct Loader {
    modules: Vec<LoadedModule>,
    /// Each loaded module's dotted name and exports, parallel to `modules`.
    infos: Vec<ModuleInfo>,
    /// Dotted module name -> index in `modules`.
    by_qual: HashMap<String, usize>,
    /// The importer and source offset of the import request being
    /// resolved, which a module loaded for it is placed at.
    current_import: Option<(PathBuf, u32)>,
    /// Each loaded module's placement, by the importer's canonical path,
    /// parallel to `modules`; resolved to indices once loading ends.
    placements: Vec<Option<(PathBuf, u32)>>,
    /// Canonical path -> index in `modules`, so two spellings of one file
    /// (a symlink, a `./` prefix) are one module.
    memo: HashMap<PathBuf, usize>,
    /// The modules currently being loaded, innermost last: the import
    /// chain a circular import closes.
    in_progress: Vec<InProgress>,
    /// The modules still running their bodies that a circular import
    /// reached, in the order first reached (`cycle`).
    provisional: Vec<Provisional>,
    entry_dir: PathBuf,
    entry_display_dir: PathBuf,
    root: Option<RootInfo>,
    /// The manifest [`Self::discover_root`] parsed, recorded for the interop
    /// policy (#1224).
    manifest: Option<DiscoveredManifest>,
    /// The `__name__` value the entry module is compiled with, or `None` when
    /// the caller supplied none (#1156). Only the entry module ever sees it.
    entry_module_name: Option<String>,
    /// How the entry module's relative imports (#1366), and since #1382 its
    /// absolute imports rooted at its own top-level package, are answered.
    relative_imports: RelativeImports,
    /// The name of the entry module's own top-level package (#1382), set
    /// only under [`RelativeImports::ForeignFromEntry`] and only when the
    /// entry's directory is a package: see [`top_level_package`].
    entry_package: Option<String>,
    /// Whether every module is lowered for an `ext` artifact (D-258, #1397).
    ext_module: bool,
    /// Whether every module is lowered in non-strict mode (#3).
    non_strict: bool,
}

/// A module being loaded: waiting at one of its imports while that import
/// loads.
struct InProgress {
    path: PathBuf,
    display: String,
    /// Its dotted name; `None` for the entry module.
    qual: Option<String>,
    /// Its desugared source (a dependency's only), which a circular import
    /// reads as its [`Provisional`].
    parsed: Option<pycc_ast::ModModule>,
    /// The start of the import it waits at.
    offset: u32,
}

impl Loader {
    /// Parses, resolves and lowers one module, loading every dependency it
    /// imports first. Returns its index in `modules`.
    ///
    /// `is_entry` selects the module that receives the program's `__name__`
    /// value (#1156); `resolve` always loads dependencies with `false`.
    fn load_module(
        &mut self,
        canonical: &Path,
        display: String,
        is_entry: bool,
        qual: Option<String>,
    ) -> Result<usize, FrontendFailure> {
        if let Some(index) = self.memo.get(canonical) {
            return Ok(*index);
        }
        let placement = self.current_import.take();
        let bytes = std::fs::read(canonical)
            .map_err(|error| FrontendFailure::input(display.clone(), error.to_string()))?;
        let source = source::decode_python_source(&bytes)
            .map_err(|message| FrontendFailure::input(display.clone(), message))?;
        // The non-strict fallback rewrite of the entry module (#12).
        let source = match is_entry.then(crate::fallback::entry_override).flatten() {
            Some(rewritten) => rewritten,
            None => source,
        };
        let mut parsed = pycc_parser::parse_all(&source)
            .map_err(|diagnostics| FrontendFailure::compile(&display, &source, diagnostics))?;
        // Names the compiler's own rewrites introduce are not the
        // program's to bind or read (E0109).
        let reserved: Vec<Diagnostic> = pycc_ast::reserved_name_uses(&parsed)
            .into_iter()
            .map(|(name, range)| {
                Diagnostic::error(
                    "E0109",
                    format!("`{name}` is reserved for names the compiler introduces -- rename it"),
                    Span::new(range.start, range.end),
                )
            })
            .collect();
        if !reserved.is_empty() {
            return Err(FrontendFailure::compile(&display, &source, reserved));
        }
        // Issue #1: a module-level name bound both by a `def` and by another
        // binding has two homes compiled code does not keep in step
        // (`pycc_ast::rebound_def_names`); refused before anything else
        // looks at the module, so in non-strict mode those bindings, and
        // then the `def`s, run in CPython.
        let rebound: Vec<Diagnostic> = pycc_ast::rebound_def_names(&parsed)
            .into_iter()
            .map(|(name, range)| {
                Diagnostic::error(
                    "C0001",
                    format!(
                        "rebinding `{name}`, which a module-level `def` also binds, is not \
                         supported yet"
                    ),
                    Span::new(range.start, range.end),
                )
            })
            .collect();
        if !rebound.is_empty() {
            return Err(FrontendFailure::compile(&display, &source, rebound));
        }
        // An `--ext` module's public signatures the boundary cannot bind
        // alone get a compiled entry for a Python wrapper (#1194).
        if is_entry && self.ext_module {
            pycc_ast::split_ext_signatures(&mut parsed, &source, self.non_strict);
            // Untyped functions get guarded native specializations.
            if self.non_strict {
                crate::specialize::specialize_entry(&mut parsed);
            }
        }
        // A native program's entry module is closed, so a dead unannotated
        // definition there can never run and is not compiled: no call site
        // could type it, in strict mode as in non-strict mode.
        if is_entry && !self.ext_module {
            pycc_ast::fold_static_branches(&mut parsed);
            pycc_ast::drop_unused_defs(&mut parsed);
        }
        pycc_ast::desugar(&mut parsed);
        if is_entry && let Some(message) = self.entry_package_collision() {
            let diagnostic = Diagnostic::error("C0001", message, Span::new(0, 0));
            return Err(FrontendFailure::compile(
                &display,
                &source,
                vec![diagnostic],
            ));
        }

        self.in_progress.push(InProgress {
            path: canonical.to_path_buf(),
            display: display.clone(),
            qual: qual.clone(),
            parsed: qual.is_some().then(|| parsed.clone()),
            offset: 0,
        });
        let requests = pycc_hir::project_import_requests(&parsed);
        let mut answers = Vec::with_capacity(requests.len());
        for request in &requests {
            if let Some(waiting) = self.in_progress.last_mut() {
                waiting.offset = request.span.start;
            }
            answers.push((
                request.span,
                self.resolve(request, &display, canonical, is_entry)?,
            ));
        }
        self.in_progress.pop();

        let mut resolved = ResolvedImports::default();
        if let Some(qual) = &qual {
            resolved.set_module_key(pycc_ast::namespace::module_key(qual));
        }
        resolved.set_ext_module(self.ext_module);
        resolved.set_non_strict(self.non_strict);
        for loaded in &self.modules {
            resolved.add_module(
                loaded.display_path.clone(),
                &loaded.module.hir,
                &loaded.module.class_slots,
            );
        }
        let mut imported: HashMap<Span, Imported> = HashMap::new();
        for (span, answer) in answers {
            match answer {
                Resolution::Loaded {
                    loaded,
                    imported: names,
                } => {
                    // Issue #1188: a direct dependency's container method
                    // names reach this module, and through them the whole
                    // transitive import closure's. Issue #1425: so does
                    // whether it can hold a CPython object -- its final
                    // state, which already includes what it inherited.
                    for dependency in loaded {
                        let dependency = &self.modules[dependency].module;
                        resolved.inherit_container_method_names(
                            dependency.container_method_names.iter().copied(),
                        );
                        resolved.inherit_object_receivers(dependency.object_receivers);
                    }
                    imported.insert(span, names);
                }
                Resolution::Found => resolved.insert(span, ResolvedImport::Found),
                Resolution::NotFound { code, message } => {
                    resolved.insert(span, ResolvedImport::NotFound { code, message });
                }
                Resolution::Foreign => resolved.insert(span, ResolvedImport::Foreign),
                Resolution::Unanswered => {}
            }
        }
        // Per-module namespaces (v0.3 accept 1): a dependency's own names
        // are renamed to their qualified spelling, and every reference to
        // another project module's name reaches it directly.
        let index = self.modules.len();
        let program = namespace::Program {
            infos: &self.infos,
            by_qual: &self.by_qual,
            display_paths: self
                .modules
                .iter()
                .map(|loaded| loaded.display_path.as_str())
                .collect(),
            provisional: &self.provisional,
        };
        let scan = namespace::scan_exports(&parsed, index, &imported, &program);
        let rewritten = namespace::rewrite(&mut parsed, qual.as_deref(), &imported, &program)
            .map_err(|diagnostics| FrontendFailure::compile(&display, &source, diagnostics))?;
        for (origin, key) in rewritten.synthesized {
            if let Some(slot) = origin.checked_sub(PROVISIONAL) {
                let module = &self.provisional[slot];
                resolved.insert_synthesized(
                    key,
                    ResolvedImport::Provisional {
                        display_path: module.display_path.clone(),
                        kinds: module.binding_kinds(),
                    },
                );
                continue;
            }
            let origin = &self.modules[origin];
            resolved.insert_synthesized(
                key,
                ResolvedImport::Module(ResolvedModule {
                    display_path: origin.display_path.clone(),
                    hir: &origin.module.hir,
                    submodule_names: Vec::new(),
                }),
            );
        }
        // The entry module's `__name__` (#1156); a dependency's own
        // `__name__` is a renamed global the rewrite binds.
        let module_name = is_entry
            .then_some(self.entry_module_name.as_deref())
            .flatten();
        let module = pycc_hir::lower_module(&parsed, &resolved, module_name)
            .map_err(|diagnostics| FrontendFailure::compile(&display, &source, diagnostics))?;
        drop(resolved);
        let exports = match &qual {
            Some(qual) => namespace::exports(&scan, qual, index, &module.hir),
            None => HashMap::new(),
        };

        self.modules.push(LoadedModule {
            display_path: display,
            source,
            module,
            parsed,
            placement: None,
        });
        if let Some(qual) = &qual {
            self.by_qual.insert(qual.clone(), index);
            self.finish_provisional(qual, index);
        }
        self.infos.push(ModuleInfo { qual, exports });
        self.placements.push(placement);
        self.memo.insert(canonical.to_path_buf(), index);
        Ok(index)
    }

    /// Answers one import request: finds the target file, reports every
    /// CPython-rejected or not-yet-supported shape, and otherwise loads the
    /// dependency (plus the package `__init__.py`s on the way to it).
    ///
    /// `importer_is_entry` is whether the importer is the entry module: under
    /// [`RelativeImports::ForeignFromEntry`] its relative imports are foreign
    /// before any base directory is resolved (#1366), and so are its absolute
    /// imports rooted at its own top-level package (#1382).
    ///
    /// A nested `from ... import` request (#1383) is answered only
    /// [`Resolution::Foreign`] or [`Resolution::Unanswered`]: it never
    /// reaches the cycle check or loads a module.
    fn resolve(
        &mut self,
        request: &ProjectImportRequest,
        importer_display: &str,
        importer_canonical: &Path,
        importer_is_entry: bool,
    ) -> Result<Resolution, FrontendFailure> {
        if importer_is_entry
            && self.relative_imports == RelativeImports::ForeignFromEntry
            && (request.level > 0 || self.names_entry_package(request))
        {
            return Ok(Resolution::Foreign);
        }
        let base = self.base_dir(request, importer_display, importer_canonical)?;
        let segments: Vec<&str> = match &request.module {
            Some(module) => module.split('.').collect(),
            None => Vec::new(),
        };
        if request.nested && !request.names.is_empty() {
            // A `from ... import` nested in a module-level `if`/`try` body
            // (#1383) is admitted only as a foreign import. Every other
            // outcome -- a project module, a relative miss, a namespace
            // package -- is left unanswered, so `pycc_hir` keeps its
            // block-body `C0001` and the module is never loaded: a
            // conditionally run module body is not something static
            // module linking can express.
            return Ok(match self.probe(&base, &segments, request) {
                Err(Resolution::Foreign) => Resolution::Foreign,
                _ => Resolution::Unanswered,
            });
        }
        let target = match self.probe(&base, &segments, request) {
            Ok(target) => target,
            Err(resolution) => return Ok(resolution),
        };
        if request.names.is_empty() && request.nested {
            // A bare `import m` nested in a block: recognized, refused by
            // `pycc_hir` (a conditionally run module body).
            return Ok(Resolution::Found);
        }
        let canonical = identity_path(&target.path);
        let offset = request.span.start;
        let in_progress = self
            .in_progress
            .iter()
            .position(|waiting| waiting.path == canonical);
        if let Some(position) = in_progress {
            // `from . import sub` inside the package being initialized (or a
            // module it is importing) loads the submodules alone; anything
            // else reads the partially initialized module: a circular
            // import (`cycle`). The entry module importing itself back
            // would be a second copy of it in CPython.
            let only_submodules = !request.names.is_empty()
                && request
                    .names
                    .iter()
                    .all(|name| target.submodules.iter().any(|sub| sub == name));
            if !only_submodules && !self.reach_partially_initialized(position) {
                let mut chain: Vec<String> = self.in_progress[position..]
                    .iter()
                    .map(|waiting| format!("`{}`", pycc_diag::display_path(&waiting.display)))
                    .collect();
                chain.push(format!("`{}`", pycc_diag::display_path(&target.display)));
                return Ok(Resolution::NotFound {
                    code: "E0108",
                    message: format!(
                        "import cycle through the entry module: {}",
                        chain.join(" -> ")
                    ),
                });
            }
        }
        let mut loaded = Vec::new();
        for (init_path, init_display) in &target.package_inits {
            let init_canonical = identity_path(init_path);
            // An `__init__.py` already on the in-progress stack is the
            // importer itself (a package initializer importing its own
            // submodule); loading it again would be a cycle. Re-loading an
            // *already finished* one is harmless -- `load_module` memoizes.
            if self
                .in_progress
                .iter()
                .any(|waiting| waiting.path == init_canonical)
            {
                continue;
            }
            let qual = self.qual_of(&init_canonical)?;
            self.current_import = Some((importer_canonical.to_path_buf(), offset));
            loaded.push(self.load_module(
                &init_canonical,
                init_display.clone(),
                false,
                Some(qual),
            )?);
        }
        let target_qual = self.qual_of(&canonical)?;
        let index = match in_progress {
            Some(_) => None,
            None => {
                self.current_import = Some((importer_canonical.to_path_buf(), offset));
                let index = self.load_module(
                    &canonical,
                    target.display.clone(),
                    false,
                    Some(target_qual.clone()),
                )?;
                loaded.push(index);
                Some(index)
            }
        };
        // `from pkg import sub`: a name the package itself does not bind but
        // a submodule file provides is that submodule, imported now.
        let mut submodules = HashMap::new();
        let package_dir = target
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let display_dir = Path::new(&target.display)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for name in &request.names {
            let bound = index.is_some_and(|index| self.infos[index].exports.contains_key(name));
            if bound || !target.submodules.iter().any(|sub| sub == name) {
                continue;
            }
            let (file, file_display) = if package_dir.join(format!("{name}.py")).is_file() {
                (
                    package_dir.join(format!("{name}.py")),
                    display_dir.join(format!("{name}.py")),
                )
            } else if package_dir.join(name).join("__init__.py").is_file() {
                (
                    package_dir.join(name).join("__init__.py"),
                    display_dir.join(name).join("__init__.py"),
                )
            } else {
                continue;
            };
            let sub_canonical = identity_path(&file);
            if self
                .in_progress
                .iter()
                .any(|waiting| waiting.path == sub_canonical)
            {
                continue;
            }
            let qual = self.qual_of(&sub_canonical)?;
            self.current_import = Some((importer_canonical.to_path_buf(), offset));
            loaded.push(self.load_module(
                &sub_canonical,
                file_display.to_string_lossy().into_owned(),
                false,
                Some(qual.clone()),
            )?);
            submodules.insert(name.clone(), qual);
        }
        Ok(Resolution::Loaded {
            loaded,
            imported: Imported {
                target: target_qual,
                submodules,
            },
        })
    }

    /// The dotted module name of the project file at `canonical`: its path
    /// below the source root, `__init__.py` naming its package. A file
    /// outside the root (reached by a relative import) is named by its own
    /// stem; a name another file already has is refused, as is a segment
    /// that is not a Python identifier.
    fn qual_of(&mut self, canonical: &Path) -> Result<String, FrontendFailure> {
        let root = self.source_root()?.canonical;
        let mut segments: Vec<String> = match canonical.strip_prefix(&root) {
            Ok(relative) => relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect(),
            Err(_) => canonical
                .file_name()
                .map(|name| vec![name.to_string_lossy().into_owned()])
                .unwrap_or_default(),
        };
        if let Some(last) = segments.last_mut()
            && let Some(stem) = last.strip_suffix(".py")
        {
            *last = stem.to_string();
        }
        if segments.last().is_some_and(|last| last == "__init__") {
            segments.pop();
        }
        if segments.is_empty() {
            segments.push(canonical.parent().and_then(Path::file_name).map_or_else(
                || "module".to_string(),
                |name| name.to_string_lossy().into_owned(),
            ));
        }
        // A module name reaches generated C (type names) and the renamer's
        // encoding; CPython can only import identifier segments anyway.
        if let Some(bad) = segments.iter().find(|segment| !is_identifier(segment)) {
            return Err(FrontendFailure::input(
                canonical.display().to_string(),
                format!(
                    "cannot compile module file {}: `{bad}` is not a valid Python module name",
                    canonical.display()
                ),
            ));
        }
        let qual = segments.join(".");
        if let Some(&other) = self.by_qual.get(&qual)
            && self.memo.get(canonical) != Some(&other)
        {
            return Err(FrontendFailure::input(
                canonical.display().to_string(),
                format!(
                    "module name `{qual}` of {} is already used by {}; rename one of the files",
                    canonical.display(),
                    self.modules[other].display_path
                ),
            ));
        }
        Ok(qual)
    }

    /// The `C0001` refusal for an entry whose own top-level package (#1382)
    /// has the name of a module `pycc_std` compiles natively (`math`,
    /// `typing`, ...), or `None` when there is no such collision.
    ///
    /// Request generation answers `import math` and `from math import x`
    /// from `pycc_std` before the loader sees them, so under the flag such a
    /// statement would silently compile against pycc's model of the standard
    /// module instead of binding the package the artifact is imported under.
    /// CPython's own answer for the same name depends on `sys.path` order
    /// and on what `sys.modules` already holds, so the build refuses the
    /// collision instead of picking a side.
    fn entry_package_collision(&self) -> Option<String> {
        let package = self.entry_package.as_deref()?;
        if !pycc_hir::is_native_std_module(package) {
            return None;
        }
        Some(format!(
            "the entry module's top-level package `{package}` has the name of a standard \
             module pycc compiles natively, which `--foreign-relative-imports` does not \
             support yet -- `import {package}` would bind pycc's `{package}` instead of the \
             package; rename the package"
        ))
    }

    /// Whether `request` is an absolute import whose first segment is the
    /// entry module's own top-level package (#1382). Such an import names
    /// the package the artifact is installed in, which the host interpreter
    /// imports by that same absolute name, so under
    /// [`RelativeImports::ForeignFromEntry`] it binds CPython's module and
    /// is never linked natively, exactly as an absolute import of a
    /// non-project root (Part 1 of #1026, Part 1 of #1138).
    fn names_entry_package(&self, request: &ProjectImportRequest) -> bool {
        let root = request
            .module
            .as_deref()
            .and_then(|module| module.split('.').next());
        request.level == 0 && root.is_some() && root == self.entry_package.as_deref()
    }
}

/// `entry` made absolute without resolving the symlinks it spells, the
/// spelling the package name is read from (#1382): a package directory
/// reached through a symlink `alias -> real` is imported as `alias`, so
/// climbing the canonical path would name `real`.
///
/// A `..` cannot be folded lexically, since after a symlink the OS resolves
/// it against the link's target. So the prefix up to and including the last
/// `..` is canonicalized, which gives it the OS's meaning, and the
/// components after it are appended as spelled; they hold no `..`, and
/// [`std::path::absolute`] already dropped every `.`. A path with no `..`
/// is kept whole. `canonical` stands in when the working directory or that
/// prefix cannot be resolved.
fn spelled_path(entry: &Path, canonical: &Path) -> PathBuf {
    let absolute = std::path::absolute(entry).unwrap_or_else(|_| canonical.to_path_buf());
    let components: Vec<Component> = absolute.components().collect();
    let last_parent = components
        .iter()
        .rposition(|component| matches!(component, Component::ParentDir));
    let Some(last_parent) = last_parent else {
        return absolute;
    };
    let prefix: PathBuf = components[..=last_parent].iter().collect();
    match prefix.canonicalize() {
        Ok(mut resolved) => {
            resolved.extend(&components[last_parent + 1..]);
            resolved
        }
        Err(_) => canonical.to_path_buf(),
    }
}

/// The name of the top-level package the module file `entry` belongs to
/// (#1382): the outermost directory reached by climbing from `entry`'s own
/// directory while each directory holds an `__init__.py`, or `None` when
/// `entry`'s directory is not a package. Read from the source tree alone,
/// never from `pycc.toml`: the package a module sits in is a property of
/// the tree CPython imports it from, not of pycc's source root.
/// The dotted name CPython imports `entry` under when its directory is a
/// package: the package directories above it (each holding an
/// `__init__.py`), then the module's own name, which a package
/// `__init__.py` does not add (`pkg/sub/m.py` is `pkg.sub.m`,
/// `pkg/__init__.py` is `pkg`). `None` outside a package, or when a part is
/// not valid UTF-8.
fn dotted_entry_name(entry: &Path) -> Option<String> {
    let stem = entry.file_stem()?.to_str()?;
    let mut parts = Vec::new();
    if stem != "__init__" {
        parts.push(stem.to_string());
    }
    let mut dir = entry.parent();
    while let Some(current) = dir.filter(|dir| dir.join("__init__.py").is_file()) {
        parts.push(current.file_name()?.to_str()?.to_string());
        dir = current.parent();
    }
    if parts.is_empty() || (stem != "__init__" && parts.len() == 1) {
        return None;
    }
    parts.reverse();
    Some(parts.join("."))
}

fn top_level_package(entry: &Path) -> Option<String> {
    let mut package = None;
    let mut dir = entry.parent();
    while let Some(current) = dir.filter(|dir| dir.join("__init__.py").is_file()) {
        package = current.file_name();
        dir = current.parent();
    }
    package.map(|name| name.to_string_lossy().into_owned())
}

/// How many directory components separate `dir` from its ancestor `root`,
/// or `None` when `root` is not an ancestor of `dir` at all.
/// The nearest directory at or above `dir` holding a `pycc.toml` file,
/// paired with how many levels up it is: the walk source-root discovery
/// makes, which `pycc lock` repeats to place `pycc.lock` beside the
/// manifest (a program with no import never runs discovery).
pub(crate) fn nearest_manifest(dir: &Path) -> Option<(usize, &Path)> {
    dir.ancestors()
        .enumerate()
        .find(|(_, ancestor)| ancestor.join("pycc.toml").is_file())
}

fn climbs_between(dir: &Path, root: &Path) -> Option<usize> {
    dir.strip_prefix(root)
        .ok()
        .map(|rest| rest.components().count())
}

/// The as-typed spelling of a source root reached by climbing `climbs`
/// directories out of `dir`. A total function: when `dir` has fewer
/// components than `climbs` (the entry was named by a bare file name, so
/// the root is above the working directory), the remainder is padded with
/// `..` so the rendered path still points at the right place.
fn display_root(dir: &Path, climbs: usize) -> PathBuf {
    let components: Vec<_> = dir.components().collect();
    let keep = components.len().saturating_sub(climbs);
    let mut root = PathBuf::new();
    for component in &components[..keep] {
        root.push(component);
    }
    for _ in 0..climbs.saturating_sub(components.len()) {
        root.push("..");
    }
    root
}

fn canonicalize(path: &Path, display: &str) -> Result<PathBuf, FrontendFailure> {
    path.canonicalize()
        .map_err(|error| FrontendFailure::input(display.to_string(), error.to_string()))
}

/// The identity a module is memoized under: its canonical form when the
/// filesystem can produce one, and the path as given otherwise. Both
/// callers have already found the file with `is_file`, and each is about to
/// read it, so a canonicalization failure here is not the right place to
/// report: the read that follows raises the real diagnostic, and the only
/// cost of the fallback is that two spellings of one unreadable file
/// memoize separately.
fn identity_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;

/// Whether `segment` is a Python identifier (letters, digits, `_`, not
/// starting with a digit): a module name CPython could import.
fn is_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}
