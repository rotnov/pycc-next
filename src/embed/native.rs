//! The native libraries a locked closure needs (the pycc.lock decision
//! entry, rule 8; #1243): libraries outside the system directories, the
//! interpreter's prefix and the bundled libpython that a closure image
//! depends on, directly or through another native. One derivation serves
//! both consumers: `pycc lock` records its result as `[[target.native]]`,
//! and an embedded build re-derives it, compares it with the section, and
//! copies what it lists into `OUT.pycc/lib/` (`OUT.pycc\natives\` on
//! Windows).
//!
//! The derivation reads the closure's *source* images, the files under the
//! site directory that `lock::build::payload` lists. On macOS it resolves
//! every dependency, absolute or relative, on the build host and
//! classifies it with `macho_host.rs`, as the build's relocation does
//! (#1259); it never refuses, and leaves the refusals to the relocation.
//! On Linux it is the ELF walk in `native_linux.rs`. On Windows it is the
//! strict PE classification in `native_windows_closure.rs` (#1306), which
//! refuses, at lock and build alike, an import it cannot place.

use super::EmbedProbe;
use super::bundle::{io_error, run_tool};
use super::closure::CLOSURE_DIR;
use super::layout::{self, EmbedPlatform, LibpythonLink};
use super::macho::{self, MachoDep};
use super::macho_host::{HostContext, HostImage, HostKind, classify_on_host};
use super::native_linux::{self, LinuxEnv};
use super::native_windows::{self, WindowsEnv};
use super::native_windows_closure;
use super::sha256::sha256_file;
use crate::lock::build::{ClosureFile, LockedClosure};
use crate::lock::schema::LockedNative;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One native library: where it is read from, and its lock entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedNative {
    /// The resolved source path.
    pub(crate) source: PathBuf,
    pub(crate) locked: LockedNative,
}

/// What an embedded build copies into `OUT.pycc/lib/` (`OUT.pycc\natives\`
/// on Windows) besides libpython and the standard library.
#[derive(Debug, Default)]
pub(crate) struct NativePlan {
    /// The natives, sorted by name.
    pub(crate) natives: Vec<DerivedNative>,
    /// Linux only: every library the build copies into `lib/` and links
    /// into the executable, prefix-vendored or native, as (the name it is
    /// needed by, its resolved source), sorted by name. macOS vendors
    /// during the relocation instead.
    pub(crate) linux_vendor: Vec<(String, PathBuf)>,
}

impl NativePlan {
    /// The `[[target.native]]` entries, sorted by name.
    pub(crate) fn locked(&self) -> Vec<LockedNative> {
        self.natives
            .iter()
            .map(|native| native.locked.clone())
            .collect()
    }

    /// The lock entry of the native read from `source`, if it is one.
    pub(crate) fn native_at(&self, source: &Path) -> Option<&LockedNative> {
        let native = self.natives.iter().find(|native| native.source == source);
        native.map(|native| &native.locked)
    }
}

/// Derives the natives of `closure` for `platform`. `interpreter` also
/// walks the interpreter's own images on Linux (libpython and
/// `lib-dynload`), which the build needs for their prefix-vendored
/// libraries and refusals, and on Windows scans the images the sidecar
/// bundles from the interpreter against `windows_env` (#1305), refusing
/// one that would not load once moved; `pycc lock` walks the closure
/// alone, as on macOS, where the relocation walks the interpreter's
/// images. On Windows the closure's images are classified and their
/// natives derived by `native_windows_closure::derive` for both callers
/// (#1306). `link` is
/// how the executable links libpython (D-251): a static build walks no
/// libpython and refuses an image that needs one.
pub(crate) fn plan_natives(
    platform: EmbedPlatform,
    probe: &EmbedProbe,
    closure: Option<&LockedClosure>,
    env: &LinuxEnv,
    windows_env: &WindowsEnv,
    interpreter: bool,
    link: LibpythonLink,
) -> Result<NativePlan, String> {
    match platform {
        EmbedPlatform::MacOs => Ok(NativePlan {
            natives: match closure {
                Some(closure) => derive_macos(probe, closure)?,
                None => Vec::new(),
            },
            linux_vendor: Vec::new(),
        }),
        EmbedPlatform::Linux => native_linux::plan(probe, closure, env, interpreter, link),
        // Windows: the interpreter's images are scanned and bundled as they
        // are; the closure's images are classified strictly, and their
        // natives are copied into `natives\` (#1306).
        EmbedPlatform::Windows => {
            if interpreter {
                native_windows::scan_interpreter(probe, windows_env)?;
            }
            let natives = match closure {
                Some(closure) => native_windows_closure::derive(probe, closure, windows_env)?,
                None => Vec::new(),
            };
            Ok(NativePlan {
                natives,
                ..NativePlan::default()
            })
        }
    }
}

/// The natives found so far, and the names they and every other vendored
/// library claim in the bundle's native directory (`lib/`, or `natives\`
/// on Windows).
pub(crate) struct Natives {
    bundled_name: String,
    /// The native directory as a message names it: `lib/` or `natives\`.
    dir_label: &'static str,
    /// Source path to (name, sha256, the distributions that need it).
    by_source: BTreeMap<PathBuf, (String, String, BTreeSet<String>)>,
    /// Case-folded name to (name, source).
    names: BTreeMap<String, (String, PathBuf)>,
}

impl Natives {
    pub(crate) fn new(bundled_name: String) -> Self {
        Self::with_dir(bundled_name, "lib/")
    }

    /// Natives copied into the directory a message names `dir_label`.
    pub(crate) fn with_dir(bundled_name: String, dir_label: &'static str) -> Self {
        Self {
            bundled_name,
            dir_label,
            by_source: BTreeMap::new(),
            names: BTreeMap::new(),
        }
    }

    /// Claims `name` in the native directory for the library read from `source`, and
    /// returns whether that library already claimed it. Refused when it is
    /// the bundled libpython's name, or when a different source already
    /// claims it; names are compared case-folded, because the macOS default
    /// file system does.
    pub(crate) fn claim_name(&mut self, name: &str, source: &Path) -> Result<bool, String> {
        if name.eq_ignore_ascii_case(&self.bundled_name) {
            return Err(format!(
                "the library `{}` needs the name `{name}` in the bundle's `{}`, which the \
                 bundled libpython already has; pycc cannot bundle both",
                source.display(),
                self.dir_label
            ));
        }
        let folded = name.to_ascii_lowercase();
        match self.names.get(&folded) {
            Some((other, other_source)) if other_source != source => Err(format!(
                "the libraries `{}` and `{}` both need the name `{name}` in the bundle's \
                 `{}` (as `{other}` and `{name}`); pycc cannot bundle both",
                other_source.display(),
                source.display(),
                self.dir_label
            )),
            Some(_) => Ok(true),
            None => {
                let entry = (name.to_string(), source.to_path_buf());
                self.names.insert(folded, entry);
                Ok(false)
            }
        }
    }

    /// Records the native read from `source`, named `name`, as needed by
    /// the distribution `owner`. Returns whether the pair is new, so the
    /// caller walks each native once per distribution. The source is
    /// hashed when it is first seen, so a missing or unreadable one is
    /// refused by name.
    pub(crate) fn add(&mut self, source: &Path, name: &str, owner: &str) -> Result<bool, String> {
        if let Some((_, _, owners)) = self.by_source.get_mut(source) {
            return Ok(owners.insert(owner.to_string()));
        }
        self.claim_name(name, source)?;
        let sha256 = sha256_file(source).map_err(|e| {
            format!(
                "the locked closure needs the native library `{}` (for distribution \
                 `{owner}`), which cannot be read: {e}",
                source.display()
            )
        })?;
        let owners = BTreeSet::from([owner.to_string()]);
        let entry = (name.to_string(), sha256, owners);
        self.by_source.insert(source.to_path_buf(), entry);
        Ok(true)
    }

    /// The natives, sorted by name.
    pub(crate) fn finish(self) -> Vec<DerivedNative> {
        let mut natives: Vec<DerivedNative> = self
            .by_source
            .into_iter()
            .map(|(source, (name, sha256, owners))| DerivedNative {
                source,
                locked: LockedNative {
                    name,
                    sha256,
                    required_by: owners.into_iter().collect(),
                },
            })
            .collect();
        natives.sort_by(|a, b| a.locked.name.cmp(&b.locked.name));
        natives
    }
}

/// The first bytes of `path`, enough to recognize an image's header.
pub(crate) fn read_head(path: &Path) -> Result<Vec<u8>, String> {
    let mut head = Vec::with_capacity(8);
    let file = std::fs::File::open(path).map_err(|e| io_error("read", path, &e))?;
    file.take(8)
        .read_to_end(&mut head)
        .map_err(|e| io_error("read", path, &e))?;
    Ok(head)
}

/// `path` with symlinks resolved, or `path` itself when it does not
/// resolve.
pub(crate) fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The names a dependency can reach the source libpython by: the original
/// id of `library` (the source or its bundled copy, whose bytes are the
/// same) and the source's resolved path. Shared by the derivation and the
/// relocation, so the two never disagree about which dependency is the
/// bundled library.
pub(crate) fn macho_bundle_lib(library: &Path, source: &Path) -> Result<[PathBuf; 2], String> {
    let listing = run_tool("otool", &[OsString::from("-L"), library.into()])?;
    let original_id = macho::parse_otool_l(&listing)
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok([PathBuf::from(original_id), resolved(source)])
}

/// [`macho_bundle_lib`]'s names for the interpreter's own shared library
/// `source`, or none when it has no Mach-O image there: a static-only
/// interpreter (the static-libpython decision entry) ships no shared
/// libpython, and asking `otool` about a missing file fails the build.
/// Only a static build and `pycc lock` meet such an interpreter; a shared
/// build's probe has already required the library.
pub(crate) fn source_bundle_lib(source: &Path) -> Result<Vec<PathBuf>, String> {
    if source.is_file() && macho::is_macho_header(&read_head(source)?) {
        return Ok(macho_bundle_lib(source, source)?.to_vec());
    }
    Ok(Vec::new())
}

/// The interpreter's resolved `sys.base_prefix`.
pub(crate) fn canonical_prefix(probe: &EmbedProbe) -> PathBuf {
    resolved(&probe.base_prefix)
}

/// What both macOS consumers classify a closure's and its natives'
/// dependencies against: the prefix, the source libpython's names, the
/// closure's scanned site directories and `<stdlib>/site-packages`, and
/// every locked payload file by its canonical source path.
pub(crate) fn host_context(
    probe: &EmbedProbe,
    closure: &LockedClosure,
    bundle_lib: &[PathBuf],
) -> HostContext {
    let payload = closure.files.iter();
    HostContext {
        prefix: canonical_prefix(probe),
        bundle_lib: bundle_lib.to_vec(),
        scanned_sites: closure.sites.clone(),
        stdlib_sites: resolved(&probe.stdlib.join("site-packages")),
        payload: payload
            .map(|file| (resolved(&file.source), file.rel.clone()))
            .collect(),
    }
}

/// Where dyld finds `path` on the build host: the canonical path of an
/// existing regular file, or a system path the dyld shared cache serves,
/// where most system libraries live only since macOS 11.
pub(crate) fn on_host(path: &Path) -> Option<PathBuf> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return std::fs::canonicalize(path).ok();
    }
    let system = macho::is_system(&path.to_string_lossy());
    (system && in_shared_cache(path)).then(|| path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn in_shared_cache(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        // `<mach-o/dyld.h>`, macOS 11 and later.
        fn _dyld_shared_cache_contains_path(path: *const std::ffi::c_char) -> bool;
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes());
    // SAFETY: the argument is a NUL-terminated string that outlives the call.
    path.is_ok_and(|path| unsafe { _dyld_shared_cache_contains_path(path.as_ptr()) })
}

/// No dyld shared cache off macOS, where no Mach-O image is resolved.
#[cfg(not(target_os = "macos"))]
fn in_shared_cache(_path: &Path) -> bool {
    false
}

/// One Mach-O image's own id, its dependencies and its `LC_RPATH`
/// entries, read with `otool`.
pub(crate) struct MachoImage {
    pub(crate) own_id: Option<String>,
    pub(crate) deps: Vec<String>,
    pub(crate) rpaths: Vec<String>,
}

impl MachoImage {
    pub(crate) fn read(image: &Path) -> Result<Self, String> {
        let otool = |flag: &str| run_tool("otool", &[OsString::from(flag), image.into()]);
        Ok(Self {
            own_id: macho::parse_otool_d(&otool("-D")?),
            deps: macho::parse_otool_l(&otool("-L")?),
            rpaths: macho::parse_otool_rpaths(&otool("-l")?),
        })
    }
}

/// The canonical directory of `source`, which dyld expands `@loader_path`
/// from.
pub(crate) fn source_dir(source: &Path) -> PathBuf {
    let source = resolved(source);
    source.parent().unwrap_or(&source).to_path_buf()
}

/// The macOS derivation: every Mach-O image in the closure, and every
/// native it reaches, directly or through another native.
fn derive_macos(probe: &EmbedProbe, closure: &LockedClosure) -> Result<Vec<DerivedNative>, String> {
    let mut images: Vec<&ClosureFile> = Vec::new();
    for file in &closure.files {
        if macho::is_macho_header(&read_head(&file.source)?) {
            images.push(file);
        }
    }
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let source = layout::source_library(probe);
    let context = host_context(probe, closure, &source_bundle_lib(&source)?);
    let bundled_name = layout::bundled_library_name(EmbedPlatform::MacOs, probe);
    let mut natives = Natives::new(bundled_name);
    let mut pending: Vec<(PathBuf, String)> = Vec::new();
    for file in images {
        let sidecar_rel = format!("{CLOSURE_DIR}/{}", file.rel);
        let image = (sidecar_rel.as_str(), HostKind::Closure);
        for native in natives_of(&file.source, image, &context)? {
            pending.push((native, file.package.clone()));
        }
    }
    while let Some((source, owner)) = pending.pop() {
        let name = file_name(&source);
        if natives.add(&source, &name, &owner)? {
            let sidecar_rel = format!("lib/{name}");
            let image = (sidecar_rel.as_str(), HostKind::Native);
            for native in natives_of(&source, image, &context)? {
                pending.push((native, owner.clone()));
            }
        }
    }
    Ok(natives.finish())
}

/// The natives among the dependencies of the image read from `source`,
/// which the sidecar holds at `sidecar_rel`.
fn natives_of(
    source: &Path,
    (sidecar_rel, kind): (&str, HostKind),
    context: &HostContext,
) -> Result<Vec<PathBuf>, String> {
    let facts = MachoImage::read(source)?;
    let source_dir = source_dir(source);
    let image = HostImage {
        sidecar_rel,
        source_dir: &source_dir,
        own_id: facts.own_id.as_deref(),
        rpaths: &facts.rpaths,
        kind,
    };
    let natives = facts
        .deps
        .iter()
        .filter_map(
            |dep| match classify_on_host(dep, &image, context, &on_host) {
                MachoDep::VendorNative(source) => Some(source),
                _ => None,
            },
        )
        .collect();
    Ok(natives)
}

/// A library's file name, which is its name in `lib/` on macOS.
pub(crate) fn file_name(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default();
    name.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
