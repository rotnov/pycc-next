//! `pycc lock`: records the CPython dependency closure an embedded build
//! will carry, read offline from the `PYCC_PYTHON` interpreter's installed
//! environment, into a per-entry, per-triple `pycc.lock` (the pycc.lock
//! decision entry under `docs/decisions/`; `docs/CLI_SPEC.md` describes
//! the command).

pub(crate) mod build;
pub(crate) mod dist;
#[cfg(test)]
pub(crate) mod fixture;
pub(crate) mod marker;
pub(crate) mod probe;
pub(crate) mod requirement;
pub(crate) mod resolve;
pub(crate) mod schema;

use crate::embed::stdlib_roots::{is_embeddable_stdlib_root, is_excluded_stdlib_root};
use crate::embed::{self, EmbedToolchain};
use crate::frontend::{self, FrontendFailure};
use crate::interop_policy::InteropCli;
use pycc_hir::{ForeignImportSite, HirModule, ImportBinding};
use schema::{LOCK_FILE_NAME, LOCK_VERSION, Lock, LockTarget, LockedNative, LockedPackage};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Why `pycc lock` failed, by exit class (`docs/CLI_SPEC.md`).
pub(crate) enum LockFailure {
    /// The program itself was refused: a parse or lowering diagnostic, an
    /// `I0402` policy gap (exit 1), or an unreadable input (exit 2).
    Frontend(FrontendFailure),
    /// The host, the interpreter, its environment or the lock file cannot
    /// be used (exit 2).
    Env(String),
    /// `--check` found the lock is not what `pycc lock` would write (exit 1).
    Stale(String),
}

impl From<FrontendFailure> for LockFailure {
    fn from(failure: FrontendFailure) -> Self {
        Self::Frontend(failure)
    }
}

/// Prints `failure` to stderr and returns its exit code.
pub(crate) fn report_lock_failure(failure: LockFailure) -> u8 {
    match failure {
        LockFailure::Frontend(failure) => frontend::report_build_failure(failure),
        LockFailure::Env(message) => {
            eprintln!("error: {message}");
            2
        }
        LockFailure::Stale(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

/// `pycc lock PATH [--check]` on this host (the pycc.lock decision entry,
/// rule 6).
pub(crate) fn run_lock(
    path: &Path,
    check: bool,
    interop: InteropCli,
    toolchain: &EmbedToolchain,
) -> Result<(), LockFailure> {
    let host = (std::env::consts::ARCH, std::env::consts::OS);
    run_lock_on(path, check, interop, toolchain, host)
}

/// [`run_lock`] for an explicit `(arch, os)` host, so the host refusals are
/// testable on every host.
pub(crate) fn run_lock_on(
    path: &Path,
    check: bool,
    interop: InteropCli,
    toolchain: &EmbedToolchain,
    (arch, os): (&str, &str),
) -> Result<(), LockFailure> {
    let triple = schema::host_triple(arch, os).map_err(LockFailure::Env)?;
    let hir = frontend::lock_frontend(path, interop)?;
    let roots = import_roots(&hir);
    let located = locate(path).map_err(LockFailure::Env)?;
    let key = located.key().map_err(LockFailure::Env)?;
    let (lock_dir, lock_path) = (located.lock_dir.as_path(), located.lock_path.as_path());
    let existing_text = read_existing(lock_path).map_err(LockFailure::Env)?;
    let existing = match &existing_text {
        Some(text) => Some(parse_lock(text, lock_path).map_err(LockFailure::Env)?),
        None => None,
    };
    let old_section = existing
        .as_ref()
        .and_then(|lock| find_section(lock, &key, &triple))
        .cloned();
    let derived = if roots.is_empty() {
        None
    } else {
        let direct = direct_roots(&hir);
        // A standard-library-only program needs no lock (rule 7), so
        // `--check` accepts its absence without starting the interpreter.
        // An optional root (#1290) is a direct root here, so a program
        // whose only third-party imports are optional still needs one.
        if check && direct.is_empty() && old_section.is_none() {
            None
        } else {
            let platform = embed::layout::EmbedPlatform::for_os(os);
            let at = (&located, path, platform);
            Some(derive(&key, &triple, &split_roots(&hir), toolchain, at)?)
        }
    };
    let base = existing.unwrap_or(Lock {
        version: LOCK_VERSION,
        target: Vec::new(),
    });
    let sections = base.target.len();
    let pruned = schema::prune(base, lock_dir);
    let orphans = sections - pruned.target.len();
    let new = schema::replace_target(pruned, &key, &triple, derived);
    let rendered = (!new.target.is_empty()).then(|| schema::render(&new));
    if rendered == existing_text {
        return Ok(());
    }
    if check {
        let reason = match &existing_text {
            None => "it does not exist".to_string(),
            Some(_) => stale_reason(
                old_section.as_ref(),
                find_section(&new, &key, &triple),
                orphans,
            ),
        };
        return Err(LockFailure::Stale(format!(
            "`{}` is not current for `{key}` on `{triple}`: {reason}; run `pycc lock {}`",
            lock_path.display(),
            path.display()
        )));
    }
    match rendered {
        Some(text) => write_atomically(lock_dir, LOCK_FILE_NAME, &text),
        None => std::fs::remove_file(lock_path)
            .map_err(|e| format!("cannot remove `{}`: {e}", lock_path.display())),
    }
    .map_err(LockFailure::Env)
}

/// Where an entry script's lock lives: the entry, canonicalized, and the
/// directory of its nearest `pycc.toml` (else the entry's own directory).
pub(crate) struct Located {
    entry: PathBuf,
    pub(crate) lock_dir: PathBuf,
    pub(crate) lock_path: PathBuf,
}

impl Located {
    /// The lock's `entry` key: the entry's path relative to the lock
    /// directory. Computed only on demand, so a build that needs no lock
    /// never refuses a non-UTF-8 entry path.
    pub(crate) fn key(&self) -> Result<String, String> {
        entry_key(
            self.entry
                .strip_prefix(&self.lock_dir)
                .unwrap_or(&self.entry),
        )
    }
}

/// Locates the lock for the entry script at `path`.
pub(crate) fn locate(path: &Path) -> Result<Located, String> {
    let entry = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve `{}`: {e}", path.display()))?;
    let entry_dir = entry.parent().unwrap_or(&entry);
    let lock_dir = crate::modules::nearest_manifest(entry_dir)
        .map_or(entry_dir, |(_, dir)| dir)
        .to_path_buf();
    let lock_path = lock_dir.join(LOCK_FILE_NAME);
    Ok(Located {
        entry,
        lock_dir,
        lock_path,
    })
}

/// Parses the lock text read from `lock_path`, naming the file on failure.
pub(crate) fn parse_lock(text: &str, lock_path: &Path) -> Result<Lock, String> {
    schema::parse(text).map_err(|m| format!("cannot use `{}`: {m}", lock_path.display()))
}

/// The program's direct roots (rule 2): its CPython-backed import roots
/// that are neither bundled with the standard library nor excluded from it.
pub(crate) fn direct_roots(hir: &HirModule) -> BTreeSet<String> {
    import_roots(hir)
        .into_iter()
        .filter(|root| !is_embeddable_stdlib_root(root) && !is_excluded_stdlib_root(root))
        .collect()
}

/// The program's direct roots split into required and optional (#1290).
/// A root is optional when every import of it is
/// [`ForeignImportSite::Block`] with `optional` set -- inside the body of a
/// `try` whose handler catches a failed import -- and required as soon as one
/// import of it is not. Together they are [`direct_roots`].
pub(crate) struct SplitRoots {
    required: BTreeSet<String>,
    optional: BTreeSet<String>,
}

impl SplitRoots {
    /// The required roots, sorted, as `roots` records them.
    pub(crate) fn required(&self) -> Vec<String> {
        self.required.iter().cloned().collect()
    }

    /// The optional roots, sorted, as `optional-roots` records them.
    pub(crate) fn optional(&self) -> Vec<String> {
        self.optional.iter().cloned().collect()
    }
}

/// Splits the program's [`direct_roots`] into required and optional ones.
pub(crate) fn split_roots(hir: &HirModule) -> SplitRoots {
    let mut required = BTreeSet::new();
    let mut guarded = BTreeSet::new();
    let foreign = hir.imports.iter().filter_map(|binding| match binding {
        ImportBinding::Foreign {
            module_path, site, ..
        } => module_path.split('.').next().map(|root| (root, site)),
        _ => None,
    });
    for (root, site) in foreign {
        if is_embeddable_stdlib_root(root) || is_excluded_stdlib_root(root) {
            continue;
        }
        let set = match site {
            ForeignImportSite::Block { optional: true } => &mut guarded,
            _ => &mut required,
        };
        set.insert(root.to_string());
    }
    let optional = guarded.difference(&required).cloned().collect();
    SplitRoots { required, optional }
}

/// The first segment of every CPython-backed import in the linked program.
fn import_roots(hir: &HirModule) -> BTreeSet<String> {
    hir.imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign { module_path, .. } => {
                module_path.split('.').next().map(str::to_string)
            }
            _ => None,
        })
        .collect()
}

/// The lock's `entry` for the entry's path relative to the lock
/// directory: its components joined by `/`.
fn entry_key(relative: &Path) -> Result<String, String> {
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect();
    parts.map(|parts| parts.join("/")).ok_or_else(|| {
        format!(
            "the entry path `{}` is not valid UTF-8; `pycc.lock` records its entry as UTF-8 \
             text",
            relative.display()
        )
    })
}

/// The lock file's text, or `None` when it does not exist.
pub(crate) fn read_existing(lock_path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(lock_path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read `{}`: {e}", lock_path.display())),
    }
}

pub(crate) fn find_section<'a>(
    lock: &'a Lock,
    entry: &str,
    triple: &str,
) -> Option<&'a LockTarget> {
    lock.target
        .iter()
        .find(|target| target.entry == entry && target.triple == triple)
}

/// Derives the (entry, triple) section for a program with at least one
/// CPython-backed import; `direct` holds no root for a standard-library-only
/// program, whose section carries only the interpreter fields. `at` is the
/// located lock, the entry as given, and the host's platform, which the
/// native libraries (rule 8, #1243) are derived for.
fn derive(
    entry: &str,
    triple: &str,
    direct: &SplitRoots,
    toolchain: &EmbedToolchain,
    (located, entry_path, platform): (&Located, &Path, embed::layout::EmbedPlatform),
) -> Result<LockTarget, LockFailure> {
    // A lock serves a shared and a static build alike (#1272), so it needs
    // no shared library: it records the one that identifies the
    // interpreter, which is the `LIBPL` archive when there is none.
    let probe = toolchain
        .probe_as(embed::LibpythonLink::Static, platform)
        .map_err(LockFailure::Env)?;
    let env = toolchain.lock_probe().map_err(LockFailure::Env)?;
    let library = toolchain
        .identity_library(&probe, platform)
        .map_err(LockFailure::Env)?;
    let libpython_sha256 = embed::sha256::sha256_file(&library)
        .map_err(|e| LockFailure::Env(format!("cannot read `{}`: {e}", library.display())))?;
    // An optional root is scanned like a required one (#1290): when it is
    // installed its closure is bundled, and an unowned one is still checked
    // for unrecorded files.
    let packages = if direct.required.is_empty() && direct.optional.is_empty() {
        Vec::new()
    } else {
        let sites = resolve::scanned_sites(&env.purelib, &env.platlib).map_err(LockFailure::Env)?;
        let (required, optional) = (&direct.required, &direct.optional);
        resolve::resolve(&sites, required, optional, &env.markers, platform)
            .map_err(LockFailure::Env)?
    };
    let (major, minor, micro) = probe.version;
    let mut section = LockTarget {
        entry: entry.to_string(),
        triple: triple.to_string(),
        python: format!("{major}.{minor}.{micro}"),
        cache_tag: env.cache_tag.clone(),
        platform: env.platform.clone(),
        libpython_sha256,
        roots: direct.required(),
        optional_roots: direct.optional(),
        package: packages
            .into_iter()
            .map(|package| LockedPackage {
                name: package.name,
                version: package.version,
                site: package.site.as_str().to_string(),
                files: package.files as u64,
                tree_sha256: package.tree_sha256,
                requires: package.requires,
            })
            .collect(),
        native: Vec::new(),
    };
    if !section.package.is_empty() {
        // The natives are derived from the payload the build will copy, by
        // the derivation the build repeats, so the two cannot disagree.
        let check = build::ClosureCheck::for_section(section.clone(), located, entry_path);
        let closure = build::payload(&check, &env, platform).map_err(LockFailure::Env)?;
        let (linux_env, windows_env) = (toolchain.linux_env(), toolchain.windows_env());
        let natives = embed::plan_natives(
            platform,
            &probe,
            Some(&closure),
            &linux_env,
            &windows_env,
            false,
            embed::LibpythonLink::Shared,
        );
        section.native = natives.map_err(LockFailure::Env)?.locked();
    }
    Ok(section)
}

/// Why `--check` found the lock stale, given the locked and the derived
/// (entry, triple) sections and how many orphaned sections `prune` drops.
fn stale_reason(old: Option<&LockTarget>, new: Option<&LockTarget>, orphans: usize) -> String {
    match (old, new) {
        (None, Some(_)) => "it has no section for this entry and host".to_string(),
        (Some(_), None) => {
            "its section for this entry is not needed: the program has no CPython-backed import"
                .to_string()
        }
        (Some(old), Some(new)) if old != new => first_difference(old, new),
        _ if orphans > 0 => {
            format!("{orphans} section(s) name an entry script that no longer exists")
        }
        _ => "it is not in the form `pycc lock` writes".to_string(),
    }
}

/// The first `(field, locked, now)` whose two values differ, worded for
/// both `pycc lock --check` and an embedded build.
pub(crate) fn field_difference(fields: &[(&str, &str, &str)]) -> Option<String> {
    fields
        .iter()
        .find(|(_, locked, now)| locked != now)
        .map(|(field, locked, now)| {
            format!("`{field}` is `{locked}` in the lock but `{now}` in the environment")
        })
}

/// The first field in which a locked section differs from the derived one.
fn first_difference(old: &LockTarget, new: &LockTarget) -> String {
    let scalars = [
        ("python", old.python.as_str(), new.python.as_str()),
        ("cache-tag", &old.cache_tag, &new.cache_tag),
        ("platform", &old.platform, &new.platform),
        (
            "libpython-sha256",
            &old.libpython_sha256,
            &new.libpython_sha256,
        ),
    ];
    if let Some(difference) = field_difference(&scalars) {
        return difference;
    }
    if old.roots != new.roots {
        return format!(
            "`roots` is {:?} in the lock but {:?} for the program",
            old.roots, new.roots
        );
    }
    if old.optional_roots != new.optional_roots {
        return format!(
            "`optional-roots` is {:?} in the lock but {:?} for the program",
            old.optional_roots, new.optional_roots
        );
    }
    let names: BTreeSet<&str> = old
        .package
        .iter()
        .chain(&new.package)
        .map(|package| package.name.as_str())
        .collect();
    for name in names {
        let locked = old.package.iter().find(|package| package.name == name);
        let now = new.package.iter().find(|package| package.name == name);
        match (locked, now) {
            (None, _) => return format!("package `{name}` is in the closure but not locked"),
            (_, None) => return format!("package `{name}` is locked but not in the closure"),
            (Some(a), Some(b)) if a != b => {
                return format!("package `{name}` differs from the installed distribution");
            }
            _ => {}
        }
    }
    // The same packages and natives in another order, as a hand-edited
    // lock can hold them.
    native_difference(&old.native, &new.native)
        .unwrap_or_else(|| "its entries are in a different order".to_string())
}

/// The first native library in which the locked `[[target.native]]`
/// entries differ from the derived ones, compared by name, or `None` when
/// they hold the same entries (in any order). Worded for both `pycc lock
/// --check` and an embedded build.
pub(crate) fn native_difference(locked: &[LockedNative], now: &[LockedNative]) -> Option<String> {
    let names: BTreeSet<&str> = locked
        .iter()
        .chain(now)
        .map(|native| native.name.as_str())
        .collect();
    names.into_iter().find_map(|name| {
        let old = locked.iter().find(|native| native.name == name);
        let new = now.iter().find(|native| native.name == name);
        match (old, new) {
            (None, _) => Some(format!(
                "`[[target.native]]` library `{name}` is needed but not locked"
            )),
            (_, None) => Some(format!(
                "`[[target.native]]` library `{name}` is locked but not needed"
            )),
            (Some(a), Some(b)) if a != b => Some(format!(
                "`[[target.native]]` library `{name}` differs from the library the closure \
                 needs"
            )),
            _ => None,
        }
    })
}

/// Writes `text` to `dir/name` through `name.tmp-<pid>` and a rename,
/// removing the temporary file on any failure.
fn write_atomically(dir: &Path, name: &str, text: &str) -> Result<(), String> {
    let tmp = dir.join(format!("{name}.tmp-{}", std::process::id()));
    let target = dir.join(name);
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &target))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write `{}`: {e}", target.display())
        })
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;
