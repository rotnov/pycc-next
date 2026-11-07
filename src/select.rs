//! Which functions a non-strict `--ext` build compiles
//! (docs/design/compile-little.md, sections 2.1-2.4).
//!
//! A module of the build stays CPython bytecode; only its selected functions
//! are compiled, each patched into the function CPython makes when the
//! module's code runs (`crate::fallback`'s selected rewrite, `patch.c`'s
//! `__pycc_patch__`). The selection comes from a profile (`--profile`,
//! [`crate::profile`]) and the selection rule, or `--select all` (every
//! eligible function), adjusted by `--compile` and `--no-compile`; with
//! neither nothing is selected and the build makes no extension at all.
//! `--select module` keeps the whole-module compilation of before, for the
//! tests that still use it.

use crate::profile::{self, Function, Rule};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `--select`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub(crate) enum SelectMode {
    /// The profile's selection, or nothing without a profile.
    #[default]
    Profile,
    /// Every eligible function.
    All,
    /// Nothing, even with a profile.
    None,
    /// Transitional: whole modules compiled as before.
    Module,
}

/// A function of a module, as the selected rewrite finds it: its qualified
/// name and first line (its first decorator's).
pub(crate) type Key = (String, u32);

/// The selected functions of every module (by dotted name), and the notes
/// the build prints about them.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Selection {
    pub(crate) modules: BTreeMap<String, BTreeSet<Key>>,
    pub(crate) notes: Vec<String>,
}

/// What a build was asked to select.
pub(crate) struct Request<'a> {
    pub(crate) mode: SelectMode,
    pub(crate) profile: Option<&'a Path>,
    /// `module:qualname` added (`--compile`) and removed (`--no-compile`).
    pub(crate) compile: &'a [String],
    pub(crate) no_compile: &'a [String],
    pub(crate) rule: Rule,
    /// What a verified profile's winning set is checked against
    /// ([`profile::Profile::winner_valid`]): the whole package's sources,
    /// the interpreter's version, the build's `--exclude` (`None` for a
    /// build of one module) and that module.
    pub(crate) package_sources: &'a [Function],
    pub(crate) python: &'a str,
    pub(crate) excludes: Option<&'a [String]>,
    pub(crate) module: Option<&'a str>,
}

fn split(spec: &str) -> Option<(&str, &str)> {
    spec.split_once(':')
}

/// The selection of `sources` (every function of the build's modules) for
/// `request`.
pub(crate) fn decide(request: &Request<'_>, sources: &[Function]) -> Result<Selection, String> {
    let mut out = Selection::default();
    let mut chosen: BTreeSet<(String, String, u32)> = BTreeSet::new();
    match request.mode {
        SelectMode::None | SelectMode::Module => {}
        SelectMode::All => {
            chosen.extend(
                sources
                    .iter()
                    .filter(|f| f.eligible)
                    .map(|f| (f.module.clone(), f.qualname.clone(), f.firstlineno)),
            );
        }
        SelectMode::Profile => match request.profile {
            None => {
                if request.compile.is_empty() {
                    out.notes.push(
                        "note: no profile (`--profile`): no function is selected, so nothing is compiled"
                            .to_string(),
                    );
                }
            }
            Some(path) => {
                let loaded = profile::Profile::read(path)?;
                let (current, notes) = loaded.current(sources, request.module);
                out.notes.extend(notes);
                // A build compiles the winning set's share of its modules
                // only while the set is valid package-wide.
                let valid = loaded.winner_valid(&profile::Scope {
                    sources: request.package_sources,
                    python: request.python,
                    excludes: request.excludes,
                    module: request.module,
                });
                if let Err(why) = &valid {
                    out.notes.push(format!(
                        "note: the profile's winning set is not valid ({why}): nothing is selected; profile again to measure it"
                    ));
                }
                let mut entries: Vec<profile::Candidate> = current.iter().map(|(e, f)| f.candidate(e)).collect();
                loaded.mark(&mut entries, &valid);
                let by_entry: BTreeMap<*const profile::Entry, &Function> =
                    current.iter().map(|(e, f)| (*e as *const _, *f)).collect();
                for decision in profile::select(loaded.total_ns, &entries, loaded.rule(request.rule)) {
                    let f = by_entry[&(decision.entry as *const _)];
                    match &decision.rejected {
                        None => {
                            chosen.insert((f.module.clone(), f.qualname.clone(), f.firstlineno));
                            if crate::fallback::reporting() {
                                out.notes.push(format!(
                                    "selected: {}:{} ({:.2}% of the profile's self time, {} calls)",
                                    f.module,
                                    f.qualname,
                                    decision.share * 100.0,
                                    decision.entry.calls
                                ));
                            }
                        }
                        Some(why) if crate::fallback::reporting() => {
                            out.notes.push(format!("not selected: {}:{}: {why}", f.module, f.qualname));
                        }
                        Some(_) => {}
                    }
                }
            }
        },
    }
    for spec in request.compile {
        let (module, qualname) = split(spec)
            .ok_or_else(|| format!("--compile `{spec}`: expected MODULE:QUALNAME"))?;
        let found: Vec<&Function> = sources
            .iter()
            .filter(|f| f.module == module && f.qualname == qualname)
            .collect();
        // A module of another build (a per-module build of a package) is
        // not this one's to check.
        if found.is_empty() && sources.iter().any(|f| f.module == module) {
            return Err(format!("--compile `{spec}`: no such function in the build's sources"));
        }
        for f in found {
            if !f.eligible {
                out.notes.push(format!(
                    "note: --compile `{spec}`: not eligible (a generator, coroutine or nested function), not compiled"
                ));
                continue;
            }
            chosen.insert((f.module.clone(), f.qualname.clone(), f.firstlineno));
        }
    }
    for spec in request.no_compile {
        let (module, qualname) = split(spec)
            .ok_or_else(|| format!("--no-compile `{spec}`: expected MODULE:QUALNAME"))?;
        chosen.retain(|(m, q, _)| !(m == module && q == qualname));
    }
    for (module, qualname, line) in chosen {
        out.modules.entry(module).or_default().insert((qualname, line));
    }
    Ok(out)
}

/// What `[tool.pycc]` of the `pyproject.toml` beside `path` (a module, or a
/// package directory) says to select: `profile` (relative to that file),
/// `compile` and `exclude` (`MODULE:QUALNAME` lists). A missing or
/// unreadable file says nothing; a key of the wrong type is an error.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Project {
    pub(crate) profile: Option<std::path::PathBuf>,
    pub(crate) compile: Vec<String>,
    pub(crate) exclude: Vec<String>,
}

impl Project {
    pub(crate) fn read(path: &Path) -> Result<Project, String> {
        let Some(dir) = path.parent() else {
            return Ok(Project::default());
        };
        let file = dir.join("pyproject.toml");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return Ok(Project::default());
        };
        let Ok(table) = text.parse::<toml::Table>() else {
            return Ok(Project::default());
        };
        let Some(pycc) = table.get("tool").and_then(|tool| tool.get("pycc")) else {
            return Ok(Project::default());
        };
        let list = |key: &str| -> Result<Vec<String>, String> {
            match pycc.get(key) {
                None => Ok(Vec::new()),
                Some(toml::Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str().map(str::to_string).ok_or_else(|| {
                            format!("{}: [tool.pycc] {key} must be a list of strings", file.display())
                        })
                    })
                    .collect(),
                Some(_) => Err(format!("{}: [tool.pycc] {key} must be a list of strings", file.display())),
            }
        };
        let profile = match pycc.get("profile") {
            None => None,
            Some(toml::Value::String(profile)) => Some(dir.join(profile)),
            Some(_) => return Err(format!("{}: [tool.pycc] profile must be a string", file.display())),
        };
        Ok(Project {
            profile,
            compile: list("compile")?,
            exclude: list("exclude")?,
        })
    }
}

/// The selection of a non-strict `--ext` build of `path` (a module or a
/// package directory), from its flags and `[tool.pycc]`, its sources
/// indexed by `interpreter`; `None` under `--select module`. Prints the
/// notes.
pub(crate) fn resolve(
    path: &Path,
    mode: Option<SelectMode>,
    flags: &crate::cli::SelectFlags,
    interpreter: &OsStr,
    excludes: Option<&[String]>,
) -> Result<Option<Selection>, String> {
    let mode = mode.unwrap_or_default();
    if mode == SelectMode::Module {
        return Ok(None);
    }
    let project = Project::read(path)?;
    let profile = flags.profile.clone().or(project.profile);
    let compile: Vec<String> = flags.compile.iter().cloned().chain(project.compile).collect();
    let no_compile: Vec<String> = flags.no_compile.iter().cloned().chain(project.exclude).collect();
    let defaults = Rule::default();
    // The package a verified profile's winning set is checked in: the
    // outermost package holding `path` (or `path` alone), indexed whole.
    let package_root = package_root(path);
    let checks_winner = mode == SelectMode::Profile && profile.is_some();
    let package_sources = if checks_winner {
        profile::index_excluding(interpreter, &package_root, excludes.unwrap_or(&[]))?
    } else {
        Vec::new()
    };
    let python = if checks_winner {
        profile::python_version(interpreter)?
    } else {
        String::new()
    };
    let module = (!path.is_dir()).then(|| import_name(path));
    let request = Request {
        package_sources: &package_sources,
        python: &python,
        excludes,
        module: module.as_deref(),
        mode,
        profile: profile.as_deref(),
        compile: &compile,
        no_compile: &no_compile,
        rule: Rule {
            share: flags.select_share.unwrap_or(defaults.share),
            calls: flags.select_calls.unwrap_or(defaults.calls),
            max: flags.select_max.unwrap_or(defaults.max),
            lines: flags.select_lines.unwrap_or(defaults.lines),
            crossing_ns: flags.select_crossing_ns.unwrap_or(defaults.crossing_ns),
            handled: flags.select_handled.unwrap_or(defaults.handled),
            sampled_only: false,
            measured_only: false,
        },
    };
    let needs_sources = match mode {
        SelectMode::All => true,
        SelectMode::Profile => request.profile.is_some() || !compile.is_empty(),
        SelectMode::None => !compile.is_empty(),
        SelectMode::Module => false,
    };
    let sources = if needs_sources {
        profile::index(interpreter, path)?
    } else {
        Vec::new()
    };
    let selection = decide(&request, &sources)?;
    for note in &selection.notes {
        eprintln!("{note}");
    }
    Ok(Some(selection))
}

/// The selected functions of the module this process compiles, when the
/// build selects functions (not set under `--select module`, or in strict
/// mode).
static MODULE: OnceLock<BTreeSet<Key>> = OnceLock::new();

/// The outermost package directory holding `path` (a module or a package
/// directory), or `path` itself when no package holds it.
fn package_root(path: &Path) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut root = path.clone();
    let mut dir = if path.is_dir() { Some(path.as_path()) } else { path.parent() };
    while let Some(d) = dir.filter(|d| d.join("__init__.py").is_file()) {
        root = d.to_path_buf();
        dir = d.parent();
    }
    root
}

/// Records this build's module's selection.
pub(crate) fn set_module(selected: BTreeSet<Key>) {
    let _ = MODULE.set(selected);
}

/// This build's module's selection, when it selects functions.
pub(crate) fn module() -> Option<&'static BTreeSet<Key>> {
    MODULE.get()
}

/// The name CPython imports the module at `path` (a `.py` file, or a
/// package directory) under: its dotted path below its import root, the
/// directory above every package that holds it (`outer/inner/m.py` with
/// `outer/__init__.py` and `outer/inner/__init__.py` is `outer.inner.m`; a
/// package's `__init__.py` is the package). The one name every part of
/// selection uses for a module: the profile's index (`import_root` and
/// `dotted` in `src/profile/driver.py`, the same walk), `--compile`, a package
/// build's lookup of its members' selections, and the build's report of
/// what it compiled.
pub(crate) fn import_name(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut parts: Vec<String> = Vec::new();
    let mut dir = if path.is_dir() {
        Some(path.as_path())
    } else {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if stem != "__init__" {
            parts.push(stem);
        }
        path.parent()
    };
    while let Some(d) = dir.filter(|d| d.join("__init__.py").is_file()) {
        parts.push(d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        dir = d.parent();
    }
    parts.reverse();
    parts.join(".")
}

/// A member's selection as the package build passes it (`--select-functions
/// FILE`): one `firstlineno qualname` per line.
pub(crate) fn to_text(selected: &BTreeSet<Key>) -> String {
    selected
        .iter()
        .map(|(qualname, line)| format!("{line} {qualname}\n"))
        .collect()
}

/// Reads [`to_text`]'s form.
pub(crate) fn from_text(text: &str) -> Result<BTreeSet<Key>, String> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (number, qualname) = line
                .split_once(' ')
                .ok_or_else(|| format!("bad selection line `{line}`"))?;
            let number = number.parse().map_err(|_| format!("bad selection line `{line}`"))?;
            Ok((qualname.to_string(), number))
        })
        .collect()
}

#[cfg(test)]
mod tests;

/// What `pycc build PATH -o OUT --ext` writes for a module with nothing
/// selected: no extension, but PATH's source as OUT's module
/// ([`crate::ext_output::python_source`]), which CPython imports as it is.
/// An extension a previous build left at OUT is removed, as it would shadow
/// the source. Returns the source file, which is PATH itself when OUT names
/// it.
pub(crate) fn ship_source(path: &Path, out: &Path, target: Option<&str>) -> Result<PathBuf, String> {
    let platform = crate::ext_build::ExtLinkPlatform::resolve(target).suffix_platform();
    let output = crate::ext_output::resolve(out, &platform).map_err(|e| e.message())?;
    let dest = crate::ext_output::python_source(out, &platform).map_err(|e| e.message())?;
    remove_stale(&output.artifact)?;
    let same = match (std::fs::canonicalize(path), std::fs::canonicalize(&dest)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        if let Some(dir) = dest.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create `{}`: {e}", dir.display()))?;
        }
        std::fs::copy(path, &dest)
            .map_err(|e| format!("cannot write `{}`: {e}", dest.display()))?;
    }
    Ok(dest)
}

/// Removes the extension `artifact` an earlier build left, if any: with
/// nothing selected, CPython must import the source, which an extension at
/// the same name shadows.
pub(crate) fn remove_stale(artifact: &Path) -> Result<(), String> {
    match std::fs::remove_file(artifact) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove `{}`: {e}", artifact.display())),
    }
}
