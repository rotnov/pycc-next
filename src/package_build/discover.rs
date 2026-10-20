//! Which modules a package build compiles: every `.py` module of the package
//! directory and of each subpackage below it (a directory with an
//! `__init__.py`), but those `--exclude` keeps in Python.

use std::path::{Path, PathBuf};

/// One module of the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageModule {
    /// Its dotted name, the package's own name first (`pkg.sub.mod`).
    pub(crate) dotted: String,
    /// Its name below the package (`sub.mod`; `""` for the package itself).
    pub(crate) relative: String,
    /// Its source file.
    pub(crate) path: PathBuf,
    /// An `__init__.py`: the module of a (sub)package.
    pub(crate) is_package: bool,
}

/// A name CPython can import: an ASCII identifier, as `ext_output` requires
/// of a module name. A file or directory named otherwise is not a module.
fn is_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `relative` (`sub.mod`) is kept in Python by an `--exclude`
/// entry: a module or a whole subpackage, named relative to the package or
/// with the package's name first.
fn excluded(package: &str, relative: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|entry| {
        let entry = if entry == package {
            ""
        } else {
            entry
                .strip_prefix(package)
                .and_then(|rest| rest.strip_prefix('.'))
                .unwrap_or(entry)
        };
        relative == entry || (!entry.is_empty() && relative.starts_with(&format!("{entry}.")))
    })
}

/// The modules of the package directory `root`, sorted by relative name (the
/// package itself first). `Err` names what is wrong with `root` or an
/// `--exclude`.
pub(crate) fn discover(root: &Path, excludes: &[String]) -> Result<Vec<PackageModule>, String> {
    let package = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !is_module_name(&package) {
        return Err(format!(
            "`{}` is not a package directory CPython can import: its name `{package}` is not a \
             module name",
            root.display()
        ));
    }
    if !root.join("__init__.py").is_file() {
        return Err(format!(
            "`{}` is not a package: it has no `__init__.py`",
            root.display()
        ));
    }
    if excluded(&package, "", excludes) {
        return Err(format!(
            "the package's own `__init__` cannot be excluded: it is the extension `{package}` \
             imports as"
        ));
    }
    let mut modules = Vec::new();
    walk(root, &package, "", excludes, &mut modules)?;
    modules.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(modules)
}

/// Adds the modules of the package directory `dir` (whose name below the
/// package is `prefix`) and of its subpackages.
fn walk(
    dir: &Path,
    package: &str,
    prefix: &str,
    excludes: &[String],
    out: &mut Vec<PackageModule>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
        names.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
        ));
    }
    names.sort();
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    for (name, path) in names {
        // `symlink_metadata`: a link is neither followed nor compiled.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if is_module_name(&name) && path.join("__init__.py").is_file() {
                let relative = join(&name);
                if !excluded(package, &relative, excludes) {
                    walk(&path, package, &relative, excludes, out)?;
                }
            }
            continue;
        }
        let Some(stem) = name.strip_suffix(".py") else {
            continue;
        };
        if !meta.is_file() || !is_module_name(stem) {
            continue;
        }
        let is_package = stem == "__init__";
        let relative = if is_package {
            prefix.to_string()
        } else {
            join(stem)
        };
        if !is_package && excluded(package, &relative, excludes) {
            continue;
        }
        let dotted = if relative.is_empty() {
            package.to_string()
        } else {
            format!("{package}.{relative}")
        };
        out.push(PackageModule {
            dotted,
            relative,
            path,
            is_package,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> pycc_scratch::ScratchDir {
        let scratch = pycc_scratch::ScratchDir::new("package-discover").unwrap();
        let root = scratch.join("pkg");
        for file in [
            "__init__.py",
            "a.py",
            "sub/__init__.py",
            "sub/b.py",
            "tests/__init__.py",
            "tests/t.py",
            "data/c.py",
            "not-a-module.py",
            "notes.txt",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x = 1\n").unwrap();
        }
        scratch
    }

    #[test]
    fn every_module_of_the_package_and_its_subpackages_is_found_sorted() {
        let scratch = tree();
        let modules = discover(&scratch.join("pkg"), &[]).unwrap();
        let names: Vec<_> = modules.iter().map(|m| m.dotted.as_str()).collect();
        assert_eq!(
            names,
            [
                "pkg",
                "pkg.a",
                "pkg.sub",
                "pkg.sub.b",
                "pkg.tests",
                "pkg.tests.t"
            ]
        );
        assert!(modules[0].is_package && modules[2].is_package && !modules[1].is_package);
        assert_eq!(modules[3].relative, "sub.b");
    }

    #[test]
    fn an_exclude_keeps_a_module_or_a_subpackage_in_python() {
        let scratch = tree();
        let excludes = ["tests".to_string(), "pkg.sub.b".to_string()];
        let modules = discover(&scratch.join("pkg"), &excludes).unwrap();
        let names: Vec<_> = modules.iter().map(|m| m.dotted.as_str()).collect();
        assert_eq!(names, ["pkg", "pkg.a", "pkg.sub"]);
    }

    #[test]
    fn a_directory_that_is_not_a_package_is_refused() {
        let scratch = tree();
        let err = discover(&scratch.join("pkg").join("data"), &[]).unwrap_err();
        assert!(err.contains("has no `__init__.py`"), "{err}");
        let err = discover(&scratch.join("pkg"), &["pkg".to_string()]).unwrap_err();
        assert!(err.contains("cannot be excluded"), "{err}");
        assert!(!is_module_name("not-a-module") && is_module_name("_a1"));
    }
}
