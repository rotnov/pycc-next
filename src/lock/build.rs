//! The build side of `pycc.lock` (the pycc.lock decision entry, rule 7;
//! #1242): an embedded build of a program with a CPython-backed import
//! outside the standard library reads the (entry, triple) section, checks
//! it against the program and the interpreter, and plans the locked
//! closure it copies into `OUT.pycc/closure/`. Every refusal is an
//! environment failure (exit 2) that names `pycc lock`.
//!
//! The order is fixed: [`plan_closure`] needs no interpreter, so a missing
//! or stale lock is reported before the interpreter is probed;
//! [`verify_interpreter`] and [`payload`] run after the probe; the copy and
//! its digest checks run in the embed bundle's staging directory.

use super::dist::{DistInfo, parse_metadata, scan_site};
use super::marker::normalize_name;
use super::probe::LockProbe;
use super::resolve::{classify_payload, read_record, scanned_sites, tree_digest};
use super::schema::{self, LockTarget};
use crate::embed::EmbedProbe;
use crate::embed::layout::EmbedPlatform;
use pycc_hir::HirModule;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A lock section the build consumes, with what its messages name.
#[derive(Debug)]
pub(crate) struct ClosureCheck {
    pub(crate) section: LockTarget,
    lock_path: PathBuf,
    entry: PathBuf,
}

impl ClosureCheck {
    /// The check of `section`, which `pycc lock` itself just derived, for
    /// the payload plan the native derivation reads.
    pub(crate) fn for_section(section: LockTarget, located: &super::Located, entry: &Path) -> Self {
        Self {
            section,
            lock_path: located.lock_path.clone(),
            entry: entry.to_path_buf(),
        }
    }

    /// A refusal naming the lock, `why`, and the command that refreshes it.
    pub(crate) fn stale(&self, why: &str) -> String {
        stale_message(&self.lock_path, &self.entry, why)
    }
}

fn stale_message(lock_path: &Path, entry: &Path, why: &str) -> String {
    format!(
        "`{}` does not match this build: {why}; run `pycc lock {}`",
        lock_path.display(),
        entry.display()
    )
}

/// One payload file the build copies to `closure/<rel>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClosureFile {
    /// The RECORD path, relative to the site directory and to `closure/`.
    pub(crate) rel: String,
    pub(crate) source: PathBuf,
    /// The RECORD sha256, lowercase hex.
    pub(crate) digest: String,
    /// The first locked distribution that claims it.
    pub(crate) package: String,
}

/// What one locked distribution's copied payload must add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedPackage {
    name: String,
    files: u64,
    tree_sha256: String,
    paths: Vec<String>,
}

/// The planned closure copy: every payload file once, plus what the copy
/// is checked against.
#[derive(Debug)]
pub(crate) struct LockedClosure {
    pub(crate) files: Vec<ClosureFile>,
    /// The section's `libpython-sha256`, checked against the digest of the
    /// file that identifies the interpreter
    /// (`embed::static_lib::identity_library`): the bundled library's
    /// bytes in a shared build, whichever file it names in a static one.
    pub(crate) libpython_sha256: String,
    /// The canonical scanned site directories (purelib, and platlib when
    /// it differs), which the native derivation classifies against
    /// (#1259); empty when the section locks no distribution.
    pub(crate) sites: Vec<PathBuf>,
    packages: Vec<ExpectedPackage>,
    lock_path: PathBuf,
    entry: PathBuf,
}

impl LockedClosure {
    /// A closure of `files` alone, with no per-distribution totals, for the
    /// copy's own tests.
    #[cfg(test)]
    pub(crate) fn of_files(files: Vec<ClosureFile>) -> Self {
        Self {
            files,
            libpython_sha256: String::new(),
            sites: Vec::new(),
            packages: Vec::new(),
            lock_path: PathBuf::from("pycc.lock"),
            entry: PathBuf::from("m.py"),
        }
    }

    /// A refusal naming the lock, `why`, and the command that refreshes it.
    pub(crate) fn stale(&self, why: &str) -> String {
        stale_message(&self.lock_path, &self.entry, why)
    }

    /// The first locked distribution that claims `rel`.
    pub(crate) fn owner_of(&self, rel: &str) -> &str {
        let owner = self.files.iter().find(|file| file.rel == rel);
        owner.map_or("", |file| file.package.as_str())
    }

    /// Checks the copied bytes' digests, `copied` (RECORD path to sha256
    /// hex), add up to each locked distribution's `files` and
    /// `tree-sha256`. A shared path counts for every claimant, as in the
    /// lock.
    pub(crate) fn verify_copied(&self, copied: &BTreeMap<String, String>) -> Result<(), String> {
        for package in &self.packages {
            let payload: BTreeMap<String, (PathBuf, String)> = package
                .paths
                .iter()
                .map(|path| {
                    let digest = copied.get(path).cloned().unwrap_or_default();
                    (path.clone(), (PathBuf::new(), digest))
                })
                .collect();
            let files = payload.len().to_string();
            let tree = tree_digest(&payload);
            let locked_files = package.files.to_string();
            let fields = [
                ("files", locked_files.as_str(), files.as_str()),
                ("tree-sha256", package.tree_sha256.as_str(), tree.as_str()),
            ];
            if let Some(difference) = super::field_difference(&fields) {
                return Err(self.stale(&format!(
                    "the copied payload of distribution `{}` differs: {difference}",
                    package.name
                )));
            }
        }
        Ok(())
    }
}

/// The lock checks that need no interpreter (step 1 of the #1242 plan).
///
/// `Ok(None)`: no lock work, because the program is standard-library-only
/// and either no `pycc.lock` exists, the host has no Tier-1 triple, or the
/// lock has no section for it. `Ok(Some)`: the section the build consumes.
/// A standard-library-only program's section is still consumed, so its
/// `libpython-sha256` is checked.
pub(crate) fn plan_closure(
    entry: &Path,
    hir: &HirModule,
    (arch, os): (&str, &str),
) -> Result<Option<ClosureCheck>, String> {
    let direct = super::direct_roots(hir);
    let located = super::locate(entry)?;
    let text = super::read_existing(&located.lock_path)?;
    if direct.is_empty() && text.is_none() {
        return Ok(None);
    }
    // A malformed lock is refused before the host is consulted, so it fails
    // on every host, including one with no Tier-1 triple.
    let lock = match &text {
        Some(text) => Some(super::parse_lock(text, &located.lock_path)?),
        None => None,
    };
    let triple = match schema::host_triple(arch, os) {
        Ok(triple) => triple,
        Err(_) if direct.is_empty() => return Ok(None),
        Err(e) => {
            return Err(format!(
                "an embedded build cannot use `pycc.lock` here: {e}"
            ));
        }
    };
    let roots = direct.iter().cloned().collect::<Vec<_>>();
    let Some(lock) = lock else {
        return Err(format!(
            "the program imports {} from outside the standard library, so an embedded build \
             bundles its locked closure from `{}`, which does not exist; run `pycc lock {}`",
            describe_roots(&roots),
            located.lock_path.display(),
            entry.display()
        ));
    };
    let key = located.key()?;
    let Some(section) = super::find_section(&lock, &key, &triple) else {
        if direct.is_empty() {
            return Ok(None);
        }
        return Err(format!(
            "no `pycc.lock` section for `{key}` on `{triple}` in `{}`; run `pycc lock {}`",
            located.lock_path.display(),
            entry.display()
        ));
    };
    let check = ClosureCheck {
        section: section.clone(),
        lock_path: located.lock_path,
        entry: entry.to_path_buf(),
    };
    // `roots` holds the required roots and `optional-roots` the ones only a
    // `try` whose handler catches a failed import imports (#1290); each is
    // compared on its own, so the refusal names the field that differs.
    let split = super::split_roots(hir);
    let (required, optional) = (split.required(), split.optional());
    if section.roots != required {
        return Err(check.stale(&format!(
            "its section's `roots` lists {} but the program requires {} from outside the \
             standard library",
            describe_roots(&section.roots),
            describe_roots(&required)
        )));
    }
    if section.optional_roots != optional {
        return Err(check.stale(&format!(
            "its section's `optional-roots` lists {} but the program imports {} from outside \
             the standard library only under a handler that catches a failed import",
            describe_roots(&section.optional_roots),
            describe_roots(&optional)
        )));
    }
    Ok(Some(check))
}

fn describe_roots(roots: &[String]) -> String {
    if roots.is_empty() {
        return "no roots".to_string();
    }
    roots
        .iter()
        .map(|root| format!("`{root}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compares the section's interpreter fields with the interpreter being
/// bundled (step 2). `libpython-sha256` is compared by the bundle against
/// the file [`crate::embed::static_lib::identity_library`] names, which it
/// already reads.
pub(crate) fn verify_interpreter(
    check: &ClosureCheck,
    probe: &EmbedProbe,
    lock_probe: &LockProbe,
) -> Result<(), String> {
    let (major, minor, micro) = probe.version;
    let python = format!("{major}.{minor}.{micro}");
    let section = &check.section;
    let fields = [
        ("python", section.python.as_str(), python.as_str()),
        ("cache-tag", &section.cache_tag, &lock_probe.cache_tag),
        ("platform", &section.platform, &lock_probe.platform),
    ];
    match super::field_difference(&fields) {
        None => Ok(()),
        Some(difference) => Err(check.stale(&difference)),
    }
}

/// Plans the closure copy (step 3): each locked distribution's payload,
/// classified from its recorded site's RECORD by the same rules `pycc lock`
/// applies on the build host's `platform` (#1296). Hashes nothing; the
/// copy hashes the bytes it writes.
pub(crate) fn payload(
    check: &ClosureCheck,
    lock_probe: &LockProbe,
    platform: EmbedPlatform,
) -> Result<LockedClosure, String> {
    let section = &check.section;
    let mut closure = LockedClosure {
        files: Vec::new(),
        libpython_sha256: section.libpython_sha256.clone(),
        sites: Vec::new(),
        packages: Vec::new(),
        lock_path: check.lock_path.clone(),
        entry: check.entry.clone(),
    };
    if section.package.is_empty() {
        return Ok(closure);
    }
    let sites = scanned_sites(&lock_probe.purelib, &lock_probe.platlib)?;
    closure.sites = sites.iter().map(|site| site.path.clone()).collect();
    let mut claims: BTreeMap<String, (String, PathBuf, String)> = BTreeMap::new();
    for package in &section.package {
        let site = sites
            .iter()
            .find(|site| site.kind.as_str() == package.site)
            .ok_or_else(|| {
                check.stale(&format!(
                    "distribution `{}` is locked in the {} site directory, which this \
                     interpreter does not have as a separate directory",
                    package.name, package.site
                ))
            })?;
        let name = normalize_name(&package.name);
        let matches: Vec<DistInfo> = scan_site(site)?
            .into_iter()
            .filter(|dist| dist.name == name)
            .collect();
        let dist = match matches.as_slice() {
            [dist] => dist,
            [] => {
                return Err(check.stale(&format!(
                    "distribution `{}` is not installed in `{}`",
                    package.name,
                    site.path.display()
                )));
            }
            many => {
                return Err(check.stale(&format!(
                    "distribution `{}` is installed more than once: {}",
                    package.name,
                    many.iter()
                        .map(|dist| format!("`{}`", dist.display()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        let metadata_path = dist.path().join("METADATA");
        let metadata = std::fs::read_to_string(&metadata_path)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_metadata(&text))
            .map_err(|e| format!("cannot read `{}`: {e}", metadata_path.display()))?;
        if metadata.version != package.version {
            return Err(check.stale(&format!(
                "distribution `{}` is locked at version `{}` but `{}` is installed",
                package.name, package.version, metadata.version
            )));
        }
        let record = read_record(dist)?;
        let classified = classify_payload(&sites, dist, &record, (false, platform))?;
        for (rel, (source, digest)) in &classified {
            match claims.get(rel) {
                Some((other, other_source, other_digest)) => {
                    if other_source != source || other_digest != digest {
                        return Err(check.stale(&format!(
                            "distributions `{other}` and `{}` both install `{rel}` with \
                             different contents",
                            package.name
                        )));
                    }
                }
                None => {
                    claims.insert(
                        rel.clone(),
                        (package.name.clone(), source.clone(), digest.clone()),
                    );
                    closure.files.push(ClosureFile {
                        rel: rel.clone(),
                        source: source.clone(),
                        digest: digest.clone(),
                        package: package.name.clone(),
                    });
                }
            }
        }
        closure.packages.push(ExpectedPackage {
            name: package.name.clone(),
            files: package.files,
            tree_sha256: package.tree_sha256.clone(),
            paths: classified.into_keys().collect(),
        });
    }
    Ok(closure)
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
