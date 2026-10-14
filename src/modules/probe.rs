//! Finding the file an import names (#898, D-222): the base directory a
//! dotted name resolves against, the source root, and the walk from it to
//! a module file or package `__init__.py`. Split out of `src/modules.rs`
//! when per-module namespaces (v0.3 accept 1) grew it past ~1000 lines.

use super::{
    DiscoveredManifest, Loader, ProjectImportRequest, Resolution, RootInfo, climbs_between,
    display_root, nearest_manifest,
};
use crate::frontend::FrontendFailure;
use crate::project_config;
use std::path::{Path, PathBuf};

impl Loader {
    /// The directory `request`'s dotted name resolves against: the source
    /// root for an absolute import, the importer's package (climbed
    /// `level - 1` times) for a relative one. A base that cannot be
    /// resolved at all carries its own rejection (see [`Base::rejected`])
    /// rather than being absent.
    pub(super) fn base_dir(
        &mut self,
        request: &ProjectImportRequest,
        importer_display: &str,
        importer_canonical: &Path,
    ) -> Result<Base, FrontendFailure> {
        if request.level == 0 {
            let root = self.source_root()?;
            return Ok(Base {
                path: root.canonical,
                display: root.display,
                relative: false,
                rejection: None,
            });
        }
        let mut display = PathBuf::from(importer_display);
        display.pop();
        if display.as_os_str().is_empty() {
            // The entry was named by a bare file name, so its directory
            // spelling is empty; `.` is the same directory and is what a
            // diagnostic can actually render.
            display = PathBuf::from(".");
        }
        // The importer's own canonical path is already known, so the base
        // never has to be re-derived from the display spelling: popping the
        // file name off it is exact even when the display path is relative
        // to the process working directory or spelled through a symlink.
        let mut canonical = importer_canonical.to_path_buf();
        canonical.pop();
        for climb in 0..request.level {
            if !canonical.join("__init__.py").is_file() {
                let message = if climb == 0 {
                    format!(
                        "attempted relative import with no known parent package: `{}` has no `__init__.py`",
                        pycc_diag::display_path(&display.to_string_lossy())
                    )
                } else {
                    "attempted relative import beyond the top-level package".to_string()
                };
                return Ok(Base::rejected(message));
            }
            if climb + 1 < request.level {
                canonical.pop();
                display.pop();
            }
        }
        Ok(Base {
            path: canonical,
            display,
            relative: true,
            rejection: None,
        })
    }

    /// Discovers, once per invocation, the directory absolute project
    /// module names resolve against.
    pub(super) fn source_root(&mut self) -> Result<RootInfo, FrontendFailure> {
        if let Some(root) = &self.root {
            return Ok(root.clone());
        }
        let root = self.discover_root()?;
        self.root = Some(root.clone());
        Ok(root)
    }

    fn discover_root(&mut self) -> Result<RootInfo, FrontendFailure> {
        if let Some((climbs, ancestor)) = nearest_manifest(&self.entry_dir) {
            let toml = ancestor.join("pycc.toml");
            // Rendered as-typed (relative to the entry's own spelling)
            // rather than as the canonical absolute path the walk uses.
            let display = display_root(&self.entry_display_dir, climbs)
                .join("pycc.toml")
                .to_string_lossy()
                .into_owned();
            let contents = std::fs::read_to_string(&toml)
                .map_err(|error| FrontendFailure::input(display.clone(), error.to_string()))?;
            let config = project_config::parse(&contents)
                .map_err(|message| FrontendFailure::input(display.clone(), message))?;
            let mut root = ancestor.join(&config.project.entry);
            root.pop();
            // Recorded before the source-root check below, so a manifest
            // whose `entry` locates no source root (the fall-through below) still
            // governs the interop policy (#1224).
            self.manifest = Some(DiscoveredManifest { display, config });
            let resolved = root
                .canonicalize()
                .ok()
                .and_then(|root| climbs_between(&self.entry_dir, &root).map(|c| (root, c)));
            if let Some((canonical, climbs)) = resolved {
                return Ok(RootInfo {
                    canonical,
                    display: display_root(&self.entry_display_dir, climbs),
                });
            }
        }
        let mut climbs = 0;
        let mut dir = self.entry_dir.clone();
        while dir.join("__init__.py").is_file() && dir.pop() {
            climbs += 1;
        }
        Ok(RootInfo {
            canonical: dir,
            display: display_root(&self.entry_display_dir, climbs),
        })
    }

    /// Walks `segments` down from `base`, returning the module file they
    /// name plus the package `__init__.py`s passed on the way. `Err`
    /// carries the [`Resolution`] to report instead.
    pub(super) fn probe(
        &self,
        base: &Base,
        segments: &[&str],
        request: &ProjectImportRequest,
    ) -> Result<Target, Resolution> {
        if let Some(rejection) = &base.rejection {
            return Err(Resolution::NotFound {
                code: "T0021",
                message: rejection.clone(),
            });
        }
        let mut path = base.path.clone();
        let mut display = base.display.clone();
        let mut package_inits = Vec::new();
        for (position, segment) in segments.iter().enumerate() {
            let last = position + 1 == segments.len();
            let file = path.join(format!("{segment}.py"));
            let file_display = display.join(format!("{segment}.py"));
            if last && file.is_file() {
                return Ok(Target::new(file, file_display, package_inits, Vec::new()));
            }
            let init = path.join(segment).join("__init__.py");
            let init_display = display.join(segment).join("__init__.py");
            if !path.join(segment).is_dir() {
                return Err(self.missing(base, request, position));
            }
            if last {
                if !init.is_file() {
                    return Err(Resolution::NotFound {
                        code: "C0001",
                        message: format!(
                            "namespace package `{}` (a directory without `__init__.py`) is not supported yet",
                            pycc_diag::display_path(&display.join(segment).to_string_lossy())
                        ),
                    });
                }
                let submodules = submodule_names(&path.join(segment));
                return Ok(Target::new(init, init_display, package_inits, submodules));
            }
            if init.is_file() {
                package_inits.push((init, init_display));
            }
            path.push(segment);
            display.push(segment);
        }
        // A relative `from . import x`: the base package itself.
        let submodules = submodule_names(&path);
        Ok(Target::new(
            path.join("__init__.py"),
            display.join("__init__.py"),
            package_inits,
            submodules,
        ))
    }

    /// The diagnostic for a dotted name that resolves to nothing: a
    /// CPython-rejected relative import (`T0021`), a foreign CPython
    /// module (Part 1 of #1026), or -- for every other absolute name --
    /// no answer at all, so `pycc_hir` keeps its own
    /// "import of module `x` is not supported yet" `C0001`. `position` is
    /// the index of the segment whose directory [`Loader::probe`] did not
    /// find.
    fn missing(&self, base: &Base, request: &ProjectImportRequest, position: usize) -> Resolution {
        if !base.relative {
            // Part 1 of #1026 admits a foreign import of a single, undotted
            // root: a bare `import X`, one such name of a multi-name
            // `import X, Y` (which `pycc_hir` requests alias by alias under
            // each alias's own span, #1280), an aliased `import X as Y`
            // (#1291; `pycc_hir` binds `Y`), and -- when `request.names` is
            // non-empty -- a `from X import a, b` at top level (#1278) or
            // nested in a module-level `if`/`try` body (#1383), whose
            // names `pycc_hir` binds to the module's attributes. A non-relative
            // base means `request.level` is `0` and `base.path` is the
            // source root the probe walked.
            let module = request.module.as_deref().unwrap_or_default();
            if !module.is_empty() && !module.contains('.') {
                return Resolution::Foreign;
            }
            // Part 1 of #1138: the from form's module may be dotted
            // (`from X.Y import a`) when its root `X` is neither a project
            // package (the probe missed at the first segment) nor a project
            // module (`X.py` beside the root). So may a plain `import X.Y`
            // or `import X.Y as Z` (empty `names`, #1381), at top level or
            // nested, under the same two conditions; `pycc_hir` binds the
            // root `X` for the first and the leaf `X.Y` for the second. A
            // dotted name under a project root keeps the `C0001`: that root
            // is compiled into the artifact, so a runtime lookup of `X.Y` in
            // the host would name a different `X` than the one pycc
            // compiled. A non-package `X/` directory that shadows a CPython
            // root (`json/` without an `__init__.py`) is walked into, and
            // keeps the `C0001` too -- conservative, as for the undotted
            // form's namespace-package refusal.
            let root = module.split('.').next().unwrap_or_default();
            if position == 0 && !base.path.join(format!("{root}.py")).is_file() {
                return Resolution::Foreign;
            }
            return Resolution::Unanswered;
        }
        let spec = format!(
            "{}{}",
            ".".repeat(request.level as usize),
            request.module.as_deref().unwrap_or_default()
        );
        Resolution::NotFound {
            code: "T0021",
            message: format!(
                "no module named `{spec}` in `{}`",
                pycc_diag::display_path(&base.display.to_string_lossy())
            ),
        }
    }
}

/// The directory a dotted name resolves against, or the reason it cannot
/// be resolved at all (a relative import outside a package).
pub(super) struct Base {
    pub(super) path: PathBuf,
    pub(super) display: PathBuf,
    pub(super) relative: bool,
    pub(super) rejection: Option<String>,
}

impl Base {
    fn rejected(message: String) -> Self {
        Self {
            path: PathBuf::new(),
            display: PathBuf::new(),
            relative: true,
            rejection: Some(message),
        }
    }
}

pub(super) struct Target {
    pub(super) path: PathBuf,
    pub(super) display: String,
    pub(super) package_inits: Vec<(PathBuf, String)>,
    pub(super) submodules: Vec<String>,
}

impl Target {
    fn new(
        path: PathBuf,
        display: PathBuf,
        package_inits: Vec<(PathBuf, PathBuf)>,
        submodules: Vec<String>,
    ) -> Self {
        Self {
            path,
            display: display.to_string_lossy().into_owned(),
            package_inits: package_inits
                .into_iter()
                .map(|(path, display)| (path, display.to_string_lossy().into_owned()))
                .collect(),
            submodules,
        }
    }
}

/// The submodule names a package directory offers: every `name.py` other
/// than `__init__.py`, and every subdirectory. Sorted so a diagnostic that
/// mentions them is deterministic across filesystems.
pub(super) fn submodule_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            match name.strip_suffix(".py") {
                Some(stem) => (stem != "__init__").then(|| stem.to_string()),
                None => entry.path().is_dir().then_some(name),
            }
        })
        .collect();
    names.sort();
    names
}
