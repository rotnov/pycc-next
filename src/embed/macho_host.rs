//! Pure classification of a closure image's or a native library's Mach-O
//! dependencies (the pycc.lock decision entry, rule 8; #1243 and #1259).
//!
//! Every dependency, absolute or relative, is resolved on the build host
//! from the *source* image's location, the way dyld resolves it there,
//! and then classified by where it resolved rather than by how it is
//! spelled. `@rpath` is searched over the requesting image's own
//! `LC_RPATH` entries only: the images that load a closure image are the
//! source interpreter's on the host and the pycc executable in the bundle,
//! so an rpath inherited from them differs between the two and a reference
//! that needs one is refused. The file-system probe is injected, so every
//! arm here runs without a file system; `native.rs` supplies the host's.

use super::layout;
use super::macho::{MachoDep, is_system};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Where a candidate path is found on the build host: its canonical path,
/// or `None` when dyld would not find it there.
pub(crate) type HostProbe<'a> = &'a dyn Fn(&Path) -> Option<PathBuf>;

/// What an image is, which decides whether a relative reference may stay
/// as it is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostKind {
    /// A locked payload file, copied to `closure/<rel>`.
    Closure,
    /// A native library, copied to `lib/<name>`.
    Native,
}

/// One image whose dependencies are classified, all of it computed by the
/// caller.
pub(crate) struct HostImage<'a> {
    /// Its path in the sidecar, `/`-separated: `closure/<rel>` or
    /// `lib/<name>`.
    pub(crate) sidecar_rel: &'a str,
    /// The canonical directory of its source on the build host, which
    /// dyld expands `@loader_path` from.
    pub(crate) source_dir: &'a Path,
    /// Its install name (`otool -D`), if it is a dylib.
    pub(crate) own_id: Option<&'a str>,
    /// Its own `LC_RPATH` entries, in load-command order.
    pub(crate) rpaths: &'a [String],
    pub(crate) kind: HostKind,
}

/// What the classification knows about the build host and the lock.
#[derive(Debug, Default)]
pub(crate) struct HostContext {
    /// The interpreter's resolved `sys.base_prefix`.
    pub(crate) prefix: PathBuf,
    /// The source libpython's original id and resolved path.
    pub(crate) bundle_lib: Vec<PathBuf>,
    /// The canonical scanned site directories (purelib, and platlib when
    /// it differs).
    pub(crate) scanned_sites: Vec<PathBuf>,
    /// The canonical `<stdlib>/site-packages`, which the standard-library
    /// copy drops: a non-venv interpreter keeps its distributions there,
    /// and a venv's base interpreter keeps them there unscanned.
    pub(crate) stdlib_sites: PathBuf,
    /// Every locked payload file: canonical source path to its path under
    /// `closure/`.
    pub(crate) payload: BTreeMap<PathBuf, String>,
}

/// Classifies the dependency `dep` of `image`, in precedence order:
///
/// 1. the image's own id, or an absolute system library: kept;
/// 2. the source libpython by spelling: rewritten to the bundled one;
/// 3. otherwise `dep` is resolved on the host (a failure is a refusal
///    with its reason); the source libpython by resolved path is
///    rewritten to the bundled one;
/// 4. a locked payload file is kept when the spelling resolves to the
///    same file in the sidecar (rule K: a closure image, every candidate
///    up to the match `@loader_path`-relative and spelled without ever
///    leaving the image's own site directory, the match at its payload
///    path, and no earlier
///    candidate at a payload path), and otherwise rebound to its closure
///    copy by an explicit `@loader_path` path;
/// 5. a system library named relatively is rebound to its absolute path;
/// 6. a file under a site directory (a scanned one or
///    `<stdlib>/site-packages`), which is an unlocked distribution's, is
///    a native;
/// 7. a file under the interpreter's prefix is vendored unrecorded;
/// 8. anything else is a native.
pub(crate) fn classify_on_host(
    dep: &str,
    image: &HostImage<'_>,
    context: &HostContext,
    probe: HostProbe<'_>,
) -> MachoDep {
    if image.own_id == Some(dep) || dep.starts_with('/') && is_system(dep) {
        return MachoDep::Keep;
    }
    let is_bundled = |path: &Path| context.bundle_lib.iter().any(|lib| lib == path);
    if is_bundled(Path::new(dep)) {
        return MachoDep::RewriteToBundled;
    }
    let (found, keep_as) = match resolve(dep, image, context, probe) {
        Ok(resolution) => resolution,
        Err(reason) => return MachoDep::RefuseWith(reason),
    };
    if is_bundled(&found) {
        return MachoDep::RewriteToBundled;
    }
    if let Some(rel) = context.payload.get(&found) {
        if keep_as.as_ref() == Some(rel) {
            return MachoDep::Keep;
        }
        let to = format!("closure/{rel}");
        return MachoDep::Rebind(layout::sidecar_loader_relative(image.sidecar_rel, &to));
    }
    let text = found.to_string_lossy();
    if is_system(&text) {
        return MachoDep::Rebind(text.into_owned());
    }
    let in_site = |site: &PathBuf| found.starts_with(site);
    if context.scanned_sites.iter().any(in_site) || found.starts_with(&context.stdlib_sites) {
        return MachoDep::VendorNative(found);
    }
    if found.starts_with(&context.prefix) {
        return MachoDep::Vendor(found);
    }
    MachoDep::VendorNative(found)
}

/// One place dyld looks for a relative reference.
enum Candidate {
    /// A directory relative to the image's own.
    Loader(String),
    /// An absolute directory.
    Absolute(PathBuf),
}

const EXECUTABLE_PATH: &str = "names the program that loads the image, which is the \
     embedded executable in the bundle";

/// Resolves `dep` on the host: its canonical path, and, when rule K's
/// conditions on the search hold, the site-relative path it was found at.
fn resolve(
    dep: &str,
    image: &HostImage<'_>,
    context: &HostContext,
    probe: HostProbe<'_>,
) -> Result<(PathBuf, Option<String>), String> {
    if dep.starts_with('/') {
        let path = PathBuf::from(dep);
        return Ok((probe(&path).unwrap_or(path), None));
    }
    // An `@rpath` search converts each `LC_RPATH` entry only when it is
    // reached, so a bad entry after the match refuses nothing (R1, R3).
    let loader = [Ok(Candidate::Loader(String::new()))];
    let (tail, candidates): (_, Box<dyn Iterator<Item = Result<Candidate, String>>>) =
        if let Some(tail) = dep.strip_prefix("@loader_path/") {
            (tail, Box::new(loader.into_iter()))
        } else if let Some(tail) = dep.strip_prefix("@rpath/") {
            let candidates = image.rpaths.iter().map(|rpath| rpath_candidate(rpath));
            (tail, Box::new(candidates))
        } else if dep.starts_with("@executable_path/") {
            return Err(format!(
                "is `@executable_path`-relative, which {EXECUTABLE_PATH}"
            ));
        } else {
            return Err(
                "is neither absolute nor an `@rpath` or `@loader_path` reference".to_string(),
            );
        };
    let site = context
        .scanned_sites
        .iter()
        .find(|site| image.source_dir.starts_with(site))
        .filter(|_| image.kind == HostKind::Closure);
    let mut keep = site.is_some();
    let mut searched = Vec::new();
    for candidate in candidates {
        let (path, spelled) = match candidate? {
            Candidate::Loader(dir) => {
                let spelled = Path::new(&dir).join(tail);
                (normalize(&image.source_dir.join(&spelled)), Some(spelled))
            }
            Candidate::Absolute(dir) => (normalize(&dir.join(tail)), None),
        };
        let site_rel = site.zip(spelled).and_then(|(site, spelled)| {
            let start = image.source_dir.strip_prefix(site).ok()?;
            within_site(start, &spelled)
        });
        keep = keep && site_rel.is_some();
        if let Some(found) = probe(&path) {
            return Ok((found, site_rel.filter(|_| keep)));
        }
        let payload_path = |rel: &String| context.payload.values().any(|path| path == rel);
        keep = keep && !site_rel.as_ref().is_some_and(payload_path);
        searched.push(format!("`{}`", path.display()));
    }
    let searched = if searched.is_empty() {
        "it has no `LC_RPATH` entry".to_string()
    } else {
        format!("searched {}", searched.join(", "))
    };
    Err(format!(
        "resolves nowhere on the build host from the image's own location and `LC_RPATH` \
         entries ({searched}); an rpath inherited from the program that loads it is not \
         followed, because that is a different program in the bundle"
    ))
}

/// Where the `LC_RPATH` entry `rpath` sends an `@rpath` search.
fn rpath_candidate(rpath: &str) -> Result<Candidate, String> {
    if rpath.starts_with('/') {
        return Ok(Candidate::Absolute(PathBuf::from(rpath)));
    }
    if let Some(tail) = rpath.strip_prefix("@loader_path")
        && (tail.is_empty() || tail.starts_with('/'))
    {
        return Ok(Candidate::Loader(tail.trim_start_matches('/').to_string()));
    }
    let why = if rpath.starts_with("@executable_path") {
        EXECUTABLE_PATH
    } else {
        "is neither absolute nor `@loader_path`-relative"
    };
    Err(format!(
        "is searched through the `LC_RPATH` entry `{rpath}`, which {why}"
    ))
}

/// The site-relative path that `spelled`, walked from the site-relative
/// directory `start`, reaches, or `None` when the walk climbs above the
/// site directory at any step: the sidecar renames that directory to
/// `closure/`, so a spelling that leaves it and comes back in would not
/// resolve there as it does on the host (rule K).
fn within_site(start: &Path, spelled: &Path) -> Option<String> {
    let mut walk: Vec<&std::ffi::OsStr> = Vec::new();
    for component in start.components().chain(spelled.components()) {
        match component {
            Component::ParentDir => {
                walk.pop()?;
            }
            Component::Normal(name) => walk.push(name),
            _ => {}
        }
    }
    let rel: PathBuf = walk.iter().collect();
    Some(rel.to_string_lossy().into_owned())
}

/// `path` with `.` and `..` removed lexically.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

// Unix only: the tests spell host paths with `/`, and a Windows host
// never reaches this walk (it scans PE images with `native_windows`
// instead).
#[cfg(all(test, unix))]
#[path = "macho_host_tests.rs"]
mod tests;
