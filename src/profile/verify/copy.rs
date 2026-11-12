//! Pass 3's copy of the project a check builds into (`copy_package`).

use std::path::{Path, PathBuf};

/// The files that mark a project's root directory.
const PROJECT_MARKERS: [&str; 3] = ["pyproject.toml", "setup.cfg", "setup.py"];

/// Whether `dir` is an installation directory (`site-packages`,
/// `dist-packages`), which is never a project.
fn installed_dir(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|name| name == "site-packages" || name == "dist-packages")
}

/// Copies what pass 3 times `pkg` (a package or single module) in under
/// `root`, and returns the copy of `pkg`, at the same path below the copy
/// of its import root (the parent of its top-level package, or of the
/// module):
///
/// - the project: the nearest directory at or above the import root that
///   holds a `pyproject.toml`, `setup.cfg` or `setup.py`, searched up to
///   the first `site-packages` or `dist-packages` directory, so the
///   workload's tests, data and `pyproject.toml` are found at the same
///   relative paths;
/// - else (no marker, or the package is installed) the top-level package or
///   the module alone;
/// - and each of `include` (`--verify-include`), which must lie under the
///   import root, at its own path.
///
/// Hidden entries, `__pycache__` and `root` itself are left out. An entry
/// outside the package that cannot be read is skipped, with a note; one
/// inside it is an error.
pub(crate) fn copy_package(pkg: &Path, root: &Path, include: &[PathBuf]) -> Result<PathBuf, String> {
    let mut base = pkg.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut top = pkg.to_path_buf();
    while base.join("__init__.py").is_file() {
        top = base.clone();
        match base.parent() {
            Some(parent) => base = parent.to_path_buf(),
            None => break,
        }
    }
    if base.as_os_str().is_empty() {
        base = PathBuf::from(".");
        top = Path::new(".").join(&top);
    }
    let base = crate::canonical::canonicalize(&base).map_err(|e| format!("{}: {e}", base.display()))?;
    let top = crate::canonical::canonicalize(&top).map_err(|e| format!("{}: {e}", top.display()))?;
    let pkg = crate::canonical::canonicalize(pkg).map_err(|e| format!("{}: {e}", pkg.display()))?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    // Canonical, as the paths copied from are (a temporary directory is a
    // link on macOS: `/var` is `/private/var`), so the path returned is the
    // copy's own whatever form `root` was given in, relative included. Never
    // a Windows verbatim path (`crate::canonical`): the workload's Python
    // joins `/`-separated names to it, which a verbatim path refuses.
    let skip = crate::canonical::canonicalize(root).map_err(|e| e.to_string())?;
    let root = skip.as_path();
    let installed = base.ancestors().any(installed_dir);
    let project = if installed {
        None
    } else {
        base.ancestors()
            .take_while(|dir| !installed_dir(dir))
            .find(|dir| PROJECT_MARKERS.iter().any(|marker| dir.join(marker).is_file()))
            .map(Path::to_path_buf)
    };
    // The copy of the import root.
    let below = |path: &Path, from: &Path| -> Result<PathBuf, String> {
        path.strip_prefix(from)
            .map(Path::to_path_buf)
            .map_err(|_| format!("{} is not under {}", path.display(), from.display()))
    };
    let copy_base = match &project {
        Some(project) => root.join(below(&base, project)?),
        None => root.to_path_buf(),
    };
    let copy = Copy { package: &top, skip: &skip };
    match &project {
        Some(project) => copy.dir(project, root, false)?,
        None => copy.entry(&top, &copy_base.join(below(&top, &base)?), true)?,
    }
    for path in include {
        let path = crate::canonical::canonicalize(path).map_err(|e| format!("--verify-include {}: {e}", path.display()))?;
        let rel = below(&path, &base).map_err(|_| {
            format!("--verify-include {}: not under the package's import root {}", path.display(), base.display())
        })?;
        copy.entry(&path, &copy_base.join(rel), false)?;
    }
    Ok(copy_base.join(below(&pkg, &base)?))
}

/// One copy of [`copy_package`]: the package's own directory or module
/// (whose entries must all be read), and the destination to leave out.
struct Copy<'a> {
    package: &'a Path,
    skip: &'a Path,
}

impl Copy<'_> {
    /// Copies the file or directory `from` to `to`; `inside` when `from` is
    /// part of the package.
    fn entry(&self, from: &Path, to: &Path, inside: bool) -> Result<(), String> {
        let inside = inside || from.starts_with(self.package);
        // A link is not followed (it may leave the project, or loop).
        let meta = match std::fs::symlink_metadata(from) {
            Ok(meta) => meta,
            Err(e) => return self.unreadable(from, inside, e),
        };
        if meta.is_dir() {
            if from == self.skip {
                return Ok(());
            }
            return self.dir(from, to, inside);
        }
        if meta.is_file() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if let Err(e) = std::fs::copy(from, to) {
                return self.unreadable(from, inside, e);
            }
        }
        Ok(())
    }

    fn dir(&self, from: &Path, to: &Path, inside: bool) -> Result<(), String> {
        let entries = match std::fs::read_dir(from) {
            Ok(entries) => entries,
            Err(e) => return self.unreadable(from, inside, e),
        };
        std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "__pycache__" {
                continue;
            }
            self.entry(&entry.path(), &to.join(&name), inside)?;
        }
        Ok(())
    }

    /// An entry `from` that cannot be read: an error inside the package,
    /// else skipped with a note.
    fn unreadable(&self, from: &Path, inside: bool, e: std::io::Error) -> Result<(), String> {
        if inside {
            return Err(format!("pass 3 cannot copy {}: {e}", from.display()));
        }
        eprintln!("pycc profile: pass 3: {} left out of the copy: {e}", from.display());
        Ok(())
    }
}
