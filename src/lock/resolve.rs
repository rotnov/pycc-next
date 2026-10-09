//! The closure resolver: which installed distributions own the program's
//! direct roots, what they require under the lock interpreter's markers,
//! and the integrity digest of each one's payload (the pycc.lock decision
//! entry, rules 2-4). Every refusal is an environment failure (exit 2).

use super::dist::{
    DistInfo, Metadata, RecordEntry, Site, SiteKind, decode_urlsafe_b64, names_root,
    normalize_lexically, parse_metadata, parse_record, scan_site,
};
use super::marker::MarkerEnv;
use super::requirement::parse_requirement;
use crate::embed::layout::EmbedPlatform;
use crate::embed::sha256::{Sha256, sha256_file};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// One locked distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPackage {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) site: SiteKind,
    pub(crate) files: usize,
    pub(crate) tree_sha256: String,
    pub(crate) requires: Vec<String>,
}

/// The installer-written dist-info files RECORD lists but the payload
/// excludes.
const EXCLUDED_DIST_INFO_FILES: [&str; 6] = [
    "RECORD",
    "RECORD.jws",
    "RECORD.p7s",
    "INSTALLER",
    "REQUESTED",
    "direct_url.json",
];

/// Canonicalizes and deduplicates the purelib and platlib directories: a
/// `lib64 -> lib` venv makes them one directory under two spellings.
pub(crate) fn scanned_sites(purelib: &Path, platlib: &Path) -> Result<Vec<Site>, String> {
    let canonical = |path: &Path, kind: SiteKind| {
        std::fs::canonicalize(path)
            .map(|path| Site { path, kind })
            .map_err(|e| {
                format!(
                    "cannot read the {} site directory `{}`: {e}",
                    kind.as_str(),
                    path.display()
                )
            })
    };
    let pure = canonical(purelib, SiteKind::Purelib)?;
    let plat = canonical(platlib, SiteKind::Platlib)?;
    if pure.path == plat.path {
        Ok(vec![pure])
    } else {
        Ok(vec![pure, plat])
    }
}

fn describe_sites(sites: &[Site]) -> String {
    sites
        .iter()
        .map(|site| format!("`{}`", site.path.display()))
        .collect::<Vec<_>>()
        .join(" and ")
}

pub(crate) fn read_record(dist: &DistInfo) -> Result<Vec<RecordEntry>, String> {
    let path = dist.path().join("RECORD");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    parse_record(&text).map_err(|e| format!("`{}`: {e}", path.display()))
}

fn has_component(path: &str, component: &str) -> bool {
    path.split('/').any(|part| part == component)
}

struct Index {
    sites: Vec<Site>,
    /// The host the lock is for, which decides the import suffixes an
    /// import root is owned through ([`names_root`]).
    platform: EmbedPlatform,
    by_name: BTreeMap<String, Vec<DistInfo>>,
    records: BTreeMap<PathBuf, Option<Vec<RecordEntry>>>,
}

impl Index {
    fn build(sites: &[Site], platform: EmbedPlatform) -> Result<Self, String> {
        let mut by_name: BTreeMap<String, Vec<DistInfo>> = BTreeMap::new();
        let mut records = BTreeMap::new();
        for site in sites {
            for dist in scan_site(site)? {
                records.insert(dist.path(), read_record(&dist).ok());
                by_name.entry(dist.name.clone()).or_default().push(dist);
            }
        }
        Ok(Self {
            sites: sites.to_vec(),
            platform,
            by_name,
            records,
        })
    }

    fn record(&self, dist: &DistInfo) -> Option<&Vec<RecordEntry>> {
        self.records.get(&dist.path()).and_then(Option::as_ref)
    }

    /// The one dist-info named `name`, refusing a duplicate.
    fn unique(&self, name: &str) -> Result<Option<&DistInfo>, String> {
        match self.by_name.get(name).map(Vec::as_slice) {
            None => Ok(None),
            Some([dist]) => Ok(Some(dist)),
            Some(many) => Err(format!(
                "distribution `{name}` is installed more than once: {}; remove all but one",
                many.iter()
                    .map(|dist| format!("`{}`", dist.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The distributions owning import root `root`: those whose RECORD
    /// lists a path naming it.
    fn owners(&self, root: &str) -> Vec<&DistInfo> {
        self.by_name
            .values()
            .flatten()
            .filter(|dist| {
                self.record(dist).is_some_and(|record| {
                    record.iter().any(|entry| {
                        names_root(&entry.path, root, self.platform)
                            && !has_component(&entry.path, "__pycache__")
                            && !has_component(&entry.path, "..")
                    })
                })
            })
            .collect()
    }
}

/// Resolves the closure of the program's `required` and `optional` direct
/// roots over the installed environment in `sites`, evaluated under `env`,
/// for a lock on `platform`.
///
/// An optional root (#1290) with an owner is resolved exactly like a
/// required one. An optional root no installed distribution owns is not a
/// refusal -- the program's handler for the failed import takes over at run time
/// -- but its site entries are still checked, so an unrecorded file or
/// link under it (which CPython would import) refuses the lock.
pub(crate) fn resolve(
    sites: &[Site],
    required: &BTreeSet<String>,
    optional: &BTreeSet<String>,
    env: &MarkerEnv,
    platform: EmbedPlatform,
) -> Result<Vec<ResolvedPackage>, String> {
    let index = Index::build(sites, platform)?;
    let mut requested: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let roots = required
        .iter()
        .map(|root| (root, false))
        .chain(optional.iter().map(|root| (root, true)));
    for (root, is_optional) in roots {
        let owners = index.owners(root);
        if owners.is_empty() && is_optional {
            check_coverage(&index, root, &owners)?;
            continue;
        }
        if owners.is_empty() {
            return Err(format!(
                "no installed distribution owns import root `{root}`: no `*.dist-info` RECORD \
                 in {} lists it; install the distribution that provides `{root}` into the \
                 PYCC_PYTHON environment",
                describe_sites(&index.sites)
            ));
        }
        for owner in &owners {
            index.unique(&owner.name)?;
        }
        check_coverage(&index, root, &owners)?;
        for owner in owners {
            requested.entry(owner.name.clone()).or_default();
        }
    }
    let members = closure(&index, requested, env)?;
    let mut claimed: BTreeMap<String, (String, PathBuf, String)> = BTreeMap::new();
    let mut packages = Vec::new();
    for (name, member) in &members {
        let payload =
            classify_payload(&index.sites, member.dist, &member.record, (true, platform))?;
        for (path, (location, digest)) in &payload {
            if let Some((other, other_location, other_digest)) = claimed.get(path) {
                if other_location != location || other_digest != digest {
                    return Err(format!(
                        "distributions `{other}` and `{name}` both install `{path}` with \
                         different contents; an embedded build copies both into one \
                         directory -- uninstall one of them"
                    ));
                }
            } else {
                claimed.insert(
                    path.clone(),
                    (name.clone(), location.clone(), digest.clone()),
                );
            }
        }
        packages.push(ResolvedPackage {
            name: name.clone(),
            version: member.metadata.version.clone(),
            site: member.dist.site.kind,
            files: payload.len(),
            tree_sha256: tree_digest(&payload),
            requires: member.requires.iter().cloned().collect(),
        });
    }
    Ok(packages)
}

/// Checks every on-disk file under `root` in every scanned site is a
/// location one of its owners' RECORDs lists. `owners` is empty only for an
/// optional root no distribution owns (#1290), where any file is
/// unrecorded and any site entry at all -- even a directory holding no
/// file, which CPython imports as a namespace package -- is refused.
fn check_coverage(index: &Index, root: &str, owners: &[&DistInfo]) -> Result<(), String> {
    let mut recorded = BTreeSet::new();
    for owner in owners {
        for entry in index.record(owner).into_iter().flatten() {
            recorded.insert(normalize_lexically(&owner.site.path.join(&entry.path)));
        }
    }
    let mut found = Vec::new();
    let mut entries_on_disk = Vec::new();
    for site in &index.sites {
        let entries = std::fs::read_dir(&site.path)
            .map_err(|e| format!("cannot read `{}`: {e}", site.path.display()))?;
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| name == root || names_root(name, root, index.platform))
            .collect();
        names.sort();
        for name in names {
            let entry = site.path.join(name);
            walk(&entry, root, &mut found)?;
            entries_on_disk.push(entry);
        }
    }
    found.sort();
    if owners.is_empty()
        && found.is_empty()
        && let Some(entry) = entries_on_disk.first()
    {
        // A directory holding no file is still a namespace package to
        // CPython, so the guarded import would succeed on this host.
        return Err(format!(
            "`{}` is a directory for optional import root `{root}`, which no installed \
             distribution owns; CPython would import it as a namespace package, but an \
             embedded build bundles only files a RECORD lists -- remove the directory",
            entry.display()
        ));
    }
    match found.into_iter().find(|path| !recorded.contains(path)) {
        None => Ok(()),
        Some(path) if owners.is_empty() => Err(format!(
            "`{}` is under optional import root `{root}`, which no installed distribution \
             owns; CPython would import it, but an embedded build bundles only files a \
             RECORD lists -- install it through its distribution or remove the file",
            path.display()
        )),
        Some(path) => Err(format!(
            "`{}` is under import root `{root}` but no RECORD of its owning distributions ({}) \
             lists it; reinstall them or remove the file",
            path.display(),
            owners
                .iter()
                .map(|owner| format!("`{}`", owner.dir_name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn walk(path: &Path, root: &str, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "`{}` under import root `{root}` is a symbolic link; the lock refuses links under \
             an import root",
            path.display()
        ));
    }
    if !meta.is_dir() {
        found.push(path.to_path_buf());
        return Ok(());
    }
    if path.file_name().is_some_and(|name| name == "__pycache__") {
        return Ok(());
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    for entry in entries.flatten() {
        walk(&entry.path(), root, found)?;
    }
    Ok(())
}

struct Member<'a> {
    dist: &'a DistInfo,
    metadata: Metadata,
    record: Vec<RecordEntry>,
    requires: BTreeSet<String>,
}

fn load_member<'a>(index: &'a Index, name: &str, why: &str) -> Result<Member<'a>, String> {
    let dist = index.unique(name)?.ok_or_else(|| {
        format!(
            "{why}, but distribution `{name}` is not installed as a `*.dist-info` in {}; \
             install it into the PYCC_PYTHON environment",
            describe_sites(&index.sites)
        )
    })?;
    let venv_hint = "a distribution- or Homebrew-managed site-packages often lacks it; build \
                     from a venv (PYCC_PYTHON=<venv>/bin/python)";
    let metadata_path = dist.path().join("METADATA");
    let metadata = std::fs::read_to_string(&metadata_path)
        .map_err(|e| e.to_string())
        .and_then(|text| parse_metadata(&text))
        .map_err(|e| {
            format!(
                "cannot read `{}`: {e}; {venv_hint}",
                metadata_path.display()
            )
        })?;
    if super::marker::normalize_name(&metadata.name) != dist.name {
        return Err(format!(
            "`{}` declares `Name: {}`, which does not match its directory name",
            metadata_path.display(),
            metadata.name
        ));
    }
    let direct_url = dist.path().join("direct_url.json");
    if let Ok(text) = std::fs::read_to_string(&direct_url) {
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("cannot parse `{}`: {e}", direct_url.display()))?;
        if json["dir_info"]["editable"] == serde_json::Value::Bool(true) {
            return Err(format!(
                "distribution `{name}` is an editable install (`{}`); an embedded build \
                 copies installed files, so install it normally (without `-e`)",
                direct_url.display()
            ));
        }
    }
    let record = read_record(dist).map_err(|e| format!("{e}; {venv_hint}"))?;
    Ok(Member {
        dist,
        metadata,
        record,
        requires: BTreeSet::new(),
    })
}

/// The fixed-point closure over (distribution, requested extras).
fn closure<'a>(
    index: &'a Index,
    mut requested: BTreeMap<String, BTreeSet<String>>,
    env: &MarkerEnv,
) -> Result<BTreeMap<String, Member<'a>>, String> {
    let mut queue: VecDeque<(String, String)> = requested
        .keys()
        .map(|name| {
            (
                name.clone(),
                format!("an owner of a direct import root is `{name}`"),
            )
        })
        .collect();
    let mut members: BTreeMap<String, Member<'a>> = BTreeMap::new();
    while let Some((name, why)) = queue.pop_front() {
        if !members.contains_key(&name) {
            let member = load_member(index, &name, &why)?;
            members.insert(name.clone(), member);
        }
        let member = &members[&name];
        let extras = requested[&name].clone();
        let mut reached = Vec::new();
        for text in &member.metadata.requires_dist {
            let requirement =
                parse_requirement(text).map_err(|e| format!("distribution `{name}`: {e}"))?;
            let included = match &requirement.marker {
                None => true,
                Some((marker, source)) => {
                    let mut holds = false;
                    for extra in std::iter::once("").chain(extras.iter().map(String::as_str)) {
                        holds |= marker
                            .evaluate(env, extra, source)
                            .map_err(|e| format!("distribution `{name}`: {e}"))?;
                    }
                    holds
                }
            };
            if included {
                reached.push((requirement, text.clone()));
            }
        }
        for (requirement, text) in reached {
            if requirement.name != name {
                members
                    .get_mut(&name)
                    .expect("the member was inserted above")
                    .requires
                    .insert(requirement.name.clone());
            }
            let is_new = !requested.contains_key(&requirement.name);
            let slot = requested.entry(requirement.name.clone()).or_default();
            let before = slot.len();
            slot.extend(requirement.extras);
            if is_new || slot.len() != before {
                queue.push_back((
                    requirement.name,
                    format!("distribution `{name}` requires `{text}`"),
                ));
            }
        }
    }
    Ok(members)
}

/// Classifies one distribution's RECORD entries (rule 4): RECORD path to
/// (location, sha256 hex). `sites` are every scanned site directory.
///
/// On a Windows `platform` a path inside the distribution's site must also
/// pass [`windows_record_path_defect`] (#1296), so every `/`-based screen
/// here sees the name Windows actually opens.
///
/// With `verify`, each payload file is hashed and must match its RECORD
/// digest, and the value is that digest (`pycc lock`). Without it nothing
/// is read beyond each path's `symlink_metadata`, and the value is the
/// RECORD digest itself: an embedded build hashes the bytes it copies
/// instead (#1242), so the two share every path rule and cannot drift.
pub(crate) fn classify_payload(
    sites: &[Site],
    dist: &DistInfo,
    record: &[RecordEntry],
    (verify, platform): (bool, EmbedPlatform),
) -> Result<BTreeMap<String, (PathBuf, String)>, String> {
    let own = &dist.site.path;
    let refuse = |path: &str, why: &str| {
        format!(
            "distribution `{}` ({}) lists `{path}` in RECORD, which {why}",
            dist.name,
            dist.display()
        )
    };
    let mut payload = BTreeMap::new();
    for entry in record {
        let path = entry.path.as_str();
        let installer_file = path
            .strip_prefix(dist.dir_name.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|file| EXCLUDED_DIST_INFO_FILES.contains(&file));
        if installer_file || has_component(path, "__pycache__") {
            continue;
        }
        if Path::new(path).is_absolute() || path.starts_with('/') {
            return Err(refuse(path, "is an absolute path"));
        }
        let target = normalize_lexically(&own.join(path));
        if !target.starts_with(own) {
            if sites.iter().any(|site| target.starts_with(&site.path)) {
                return Err(refuse(
                    path,
                    "lies in the other scanned site directory; a distribution split across \
                     purelib and platlib cannot be locked",
                ));
            }
            continue;
        }
        if has_component(path, "..") {
            return Err(refuse(path, "contains `..`"));
        }
        if platform == EmbedPlatform::Windows
            && let Some(defect) = windows_record_path_defect(path)
        {
            return Err(refuse(path, defect));
        }
        let Some(encoded) = entry.hash.strip_prefix("sha256=") else {
            return Err(refuse(path, "has no sha256 hash"));
        };
        let expected = decode_urlsafe_b64(encoded)
            .filter(|bytes| bytes.len() == 32)
            .ok_or_else(|| refuse(path, "has a malformed sha256 hash"))?;
        if payload.contains_key(path) {
            return Err(refuse(path, "is listed twice"));
        }
        if !path.contains('/') && path.ends_with(".pth") {
            return Err(refuse(
                path,
                "is a top-level `.pth` file; the embedded launcher never processes `.pth` \
                 files (an editable install? install the distribution normally)",
            ));
        }
        let mut prefix = own.clone();
        for component in path.split('/') {
            prefix.push(component);
            match std::fs::symlink_metadata(&prefix) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(refuse(path, "is or passes through a symbolic link"));
                }
                Ok(_) => {}
                Err(_) => {
                    return Err(refuse(
                        path,
                        "is missing on disk; reinstall the distribution",
                    ));
                }
            }
        }
        let expected: String = expected.iter().map(|byte| format!("{byte:02x}")).collect();
        if !verify {
            payload.insert(path.to_string(), (target, expected));
            continue;
        }
        let actual =
            sha256_file(&target).map_err(|e| refuse(path, &format!("cannot be read: {e}")))?;
        if actual != expected {
            return Err(refuse(
                path,
                "does not match its RECORD hash; the file changed after install -- \
                 reinstall the distribution",
            ));
        }
        payload.insert(path.to_string(), (target, actual));
    }
    Ok(payload)
}

/// The device names Win32 reserves in every directory, whatever the
/// extension: `NUL.py` opens the null device, not a file.
const WINDOWS_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why a `/`-separated RECORD path is unsafe to lock on a Windows host
/// (#1296), or `None`: a `\`, which Windows also splits on, so the
/// `/`-based screens would not see its components; a `:`, a drive or an
/// alternate data stream (`x.pyd::$DATA` writes `x.pyd`); a component
/// other than `.` and `..` ending in `.` or a space, which Win32 strips
/// (`x.pyd.` is `x.pyd`); or a component whose stem before its first `.`
/// is a reserved device name. Pure.
pub(crate) fn windows_record_path_defect(path: &str) -> Option<&'static str> {
    if path.contains('\\') {
        return Some("contains `\\`, which a Windows host also splits paths on");
    }
    if path.contains(':') {
        return Some("contains `:`, a drive or an alternate data stream on a Windows host");
    }
    for component in path.split('/') {
        if matches!(component, "." | "..") {
            continue;
        }
        if component.ends_with(['.', ' ']) {
            return Some("has a component ending in `.` or a space, which a Windows host strips");
        }
        let stem = component.split('.').next().unwrap_or(component);
        if WINDOWS_DEVICE_NAMES
            .iter()
            .any(|device| device.eq_ignore_ascii_case(stem))
        {
            return Some("names a device a Windows host reserves (`CON`, `NUL`, `COM1`, ...)");
        }
    }
    None
}

/// The tree digest: sha256 over `"<path>\0<sha256-hex>\n"` per payload
/// file, in path byte order.
pub(crate) fn tree_digest(payload: &BTreeMap<String, (PathBuf, String)>) -> String {
    let mut hasher = Sha256::new();
    for (path, (_, digest)) in payload {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finish()
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
