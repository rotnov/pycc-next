//! What pass 3's pruning removes as one: a region of the winner's selected
//! unit (docs/design/selected-regions.md, sections 4.1 and 4.7), or a
//! function in no region.
//!
//! A region's members are connected by calls a build compiles as calls
//! between them (`crate::fallback::regions_of`); removing one changes what
//! its neighbors call. Every set pass 3 measures is built as its own
//! selected unit, so it is a union of its own regions and single functions;
//! pruning removes a whole region of the winner at a time, and the winner's
//! key stays its members, so a build compiles exactly the measured set.

use super::FunctionKey;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The regions the selected unit of `set` forms in the package (or module)
/// at `pkg`, module by module: each a set of two or more of its functions,
/// or one that calls itself. A module whose source cannot be read forms
/// none.
pub(crate) fn static_regions(pkg: &Path, set: &BTreeSet<FunctionKey>) -> Vec<BTreeSet<FunctionKey>> {
    let mut by_module: BTreeMap<&str, BTreeSet<crate::select::Key>> = BTreeMap::new();
    for (module, qualname, line) in set {
        by_module.entry(module).or_default().insert((qualname.clone(), *line));
    }
    let files = module_files(pkg);
    let mut out = Vec::new();
    for (module, keys) in by_module {
        let Some(source) = files.get(module).and_then(|file| std::fs::read_to_string(file).ok()) else {
            continue;
        };
        for region in crate::fallback::regions_of(&source, &keys) {
            out.push(region.into_iter().map(|(q, l)| (module.to_string(), q, l)).collect());
        }
    }
    out
}

/// The Python files of `pkg` (a module, or a package directory and every
/// one below it), by the name CPython imports each under.
fn module_files(pkg: &Path) -> BTreeMap<String, PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![pkg.to_path_buf()];
    if pkg.is_file() {
        files.push(pkg.to_path_buf());
        dirs.clear();
    }
    while let Some(dir) = dirs.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() && path.join("__init__.py").is_file() => dirs.push(path),
                Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "py") => files.push(path),
                _ => {}
            }
        }
    }
    files
        .into_iter()
        .map(|file| {
            let module = if file.file_name().is_some_and(|n| n == "__init__.py") {
                crate::select::import_name(file.parent().unwrap_or(&file))
            } else {
                crate::select::import_name(&file)
            };
            (module, file)
        })
        .collect()
}

/// The unit of `key` among the regions `units`, within `within`: its
/// region's members in `within`, or `key` alone.
fn unit_of(key: &FunctionKey, units: &[BTreeSet<FunctionKey>], within: &BTreeSet<FunctionKey>) -> BTreeSet<FunctionKey> {
    match units.iter().find(|unit| unit.contains(key)) {
        Some(unit) => unit.intersection(within).cloned().collect(),
        None => BTreeSet::from([key.clone()]),
    }
}

/// The units pruning may remove from the members `removable`, given the
/// winner's regions `units` (the carried set's members are not removable,
/// so a region keeps only its removable ones), in the order it tries them:
/// least self time first, by the member with the most self time (`order`
/// comes most self time first). Members not in `order` are not tried.
pub(crate) fn removals(
    order: &[FunctionKey],
    units: &[BTreeSet<FunctionKey>],
    removable: &BTreeSet<FunctionKey>,
) -> Vec<BTreeSet<FunctionKey>> {
    let mut out: Vec<BTreeSet<FunctionKey>> = Vec::new();
    for key in order.iter().filter(|key| removable.contains(*key)) {
        if out.iter().any(|unit| unit.contains(key)) {
            continue;
        }
        out.push(unit_of(key, units, removable));
    }
    out.reverse();
    out
}

/// `key`s of `unit`, for pass 3's report: `m:a, m:b`.
pub(crate) fn names(unit: &BTreeSet<FunctionKey>) -> String {
    unit.iter().map(|key| format!("{}:{}", key.0, key.1)).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod tests;
