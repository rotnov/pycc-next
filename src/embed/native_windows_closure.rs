//! The Windows closure's native images (#1306, Part 2 of #1297): every
//! locked payload file that is a PE image ([`is_windows_image`]) is
//! screened and parsed ([`pe`]), each import is classified by the strict
//! rules below, and a file beside an image that the closure does not lock
//! is a *native*, copied into `OUT.pycc\natives\` (which the launcher adds
//! as a DLL directory) and classified in turn. An import no rule places is
//! refused, at `pycc lock` and at the build alike, so a lock the build
//! must refuse is never written.
//!
//! A static import of an image resolves by the first matching rule:
//!
//! - R1: an API set is a system DLL;
//! - R2: a root DLL (`layout::windows_root_dlls`) is in the sidecar root;
//! - R2b: the program DLL's name is refused, since the stub already loaded
//!   the program DLL under it;
//! - R3 (closure images only): a scanned closure image in the image's own
//!   source directory is payload, found through `DLL_LOAD_DIR`;
//! - R4: any other regular file there that the closure does not lock is a
//!   native;
//! - R5 (closure images only): the one scanned closure image anywhere with
//!   that file name is payload, which the package's own
//!   `os.add_dll_directory` reaches; two or more are refused as ambiguous;
//! - R6: a file in a system directory is a system DLL;
//! - anything else is refused.
//!
//! A native gets R1, R2, R2b, R4 and R6 only. A delay import or an export
//! forwarder resolves through the standard search order, so only a system
//! DLL (R1, R6) or the interpreter's DLL is accepted. A native is refused
//! when its name is a system DLL's (it would shadow it for every image),
//! a scanned closure image's, or a file's in the interpreter's `DLLs\`.
//!
//! Every name is compared ASCII case-insensitively, never by probing a
//! joined path: a lookup in an image's own directory or in `DLLs\` goes
//! through a per-directory listing map ([`listing_map`]), and the root,
//! system and closure-image names are case-folded sets, so the derivation
//! classifies alike on a case-sensitive host. The system directories are
//! injected ([`WindowsEnv`]); an image or native a message names is shown
//! with `\`, while a directory a listing or I/O refusal names is shown as
//! the host renders it.

use super::EmbedProbe;
use super::bundle::io_error;
use super::layout;
use super::native::{DerivedNative, Natives, read_head};
use super::native_windows::{WindowsEnv, backslashed, system_names};
use super::pe;
use super::windows::is_windows_image;
use crate::lock::build::LockedClosure;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The tail of every refusal.
const EXT: &str = " -- use `pycc build --ext`";

/// Why a closure image's static import is refused when no rule places it.
const CLOSURE_WHY: &str = "is neither an API set, a DLL the sidecar bundles, a locked file of \
                           the closure, a file beside it, nor a system DLL";

/// Why a native's static import is refused when no rule places it.
const NATIVE_WHY: &str =
    "is neither an API set, a DLL the sidecar bundles, a file beside it, nor a system DLL";

/// Derives the natives the Windows closure's images need, sorted by name,
/// or refuses the first image, import or native no rule accepts. Empty
/// when the closure holds no PE image.
pub(crate) fn derive(
    probe: &EmbedProbe,
    closure: &LockedClosure,
    env: &WindowsEnv,
) -> Result<Vec<DerivedNative>, String> {
    let mut candidates = Vec::new();
    for file in &closure.files {
        if is_windows_image(&file.rel, &read_head(&file.source)?) {
            candidates.push(file);
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut ctx = Ctx::new(probe, env);
    for file in &closure.files {
        ctx.payload.insert(fold_path(&file.source));
    }
    let mut images = Vec::new();
    for file in candidates {
        let shown = format!("closure\\{}", backslashed(Path::new(&file.rel)));
        let subject = format!("`{shown}` (distribution `{}`)", file.package);
        let scanned = scan(&subject, &read_image(&file.source)?)?;
        ctx.images.insert(fold_path(&file.source));
        let by_name = ctx.images_by_name.entry(folded_name(&file.source));
        by_name.or_default().push(shown.clone());
        images.push((file, shown, subject, scanned));
    }
    let mut pending = Vec::new();
    for (file, shown, subject, scanned) in &images {
        let image = ctx.image(&file.source, true)?;
        for native in ctx.natives_of(&image, subject, scanned)? {
            pending.push((native, file.package.clone(), shown.clone()));
        }
    }
    let mut natives = Natives::with_dir(probe.ldlibrary.clone(), "natives\\");
    while let Some(((source, name), owner, needed_by)) = pending.pop() {
        if !natives.add(&source, &name, &owner)? {
            continue;
        }
        let shown = format!("natives\\{name}");
        let subject = format!(
            "the native `{shown}` (copied for distribution `{owner}`, needed by `{needed_by}`)"
        );
        ctx.check_native_name(&subject, &name)?;
        let scanned = scan(&subject, &read_image(&source)?)?;
        let image = ctx.image(&source, false)?;
        for native in ctx.natives_of(&image, &subject, &scanned)? {
            pending.push((native, owner.clone(), shown.clone()));
        }
    }
    Ok(natives.finish())
}

/// A directory's regular files by case-folded name, and the folded names
/// that more than one entry folds to (possible on a case-sensitive
/// volume).
#[derive(Debug, Default)]
pub(super) struct ListingMap {
    dir: PathBuf,
    /// Folded name to on-disk name, the first entry listed.
    names: BTreeMap<String, String>,
    /// Folded name to the first two entries that fold to it.
    duplicates: BTreeMap<String, (String, String)>,
}

/// The listing map of `names`, the regular files listed in `dir`.
pub(super) fn listing_map(dir: &Path, names: Vec<OsString>) -> ListingMap {
    let mut map = ListingMap {
        dir: dir.to_path_buf(),
        ..ListingMap::default()
    };
    for name in names {
        let name = name.to_string_lossy().into_owned();
        let folded = name.to_ascii_lowercase();
        match map.names.get(&folded) {
            Some(first) => {
                let pair = (first.clone(), name);
                map.duplicates.entry(folded).or_insert(pair);
            }
            None => {
                map.names.insert(folded, name);
            }
        }
    }
    map
}

impl ListingMap {
    /// The on-disk name `name` folds to, if any; refused when two entries
    /// fold to it, since Windows could load either.
    fn lookup(&self, name: &str) -> Result<Option<&str>, String> {
        let folded = name.to_ascii_lowercase();
        if let Some((first, second)) = self.duplicates.get(&folded) {
            return Err(format!(
                "matches both `{first}` and `{second}` in `{}`, which Windows cannot tell apart",
                self.dir.display()
            ));
        }
        Ok(self.names.get(&folded).map(String::as_str))
    }
}

/// Lists the regular files of `dir` (following symlinks). A directory
/// that cannot be listed is refused, naming it.
fn list_dir(dir: &Path) -> Result<ListingMap, String> {
    let read = std::fs::read_dir(dir).and_then(|entries| entries.collect::<Result<Vec<_>, _>>());
    let entries = read.map_err(|e| io_error("list", dir, &e))?;
    let names = entries
        .into_iter()
        .filter(|entry| std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_file()))
        .map(|entry| entry.file_name())
        .collect();
    Ok(listing_map(dir, names))
}

/// The bytes of the image read from `path`.
fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| io_error("read", path, &e))
}

/// `path` case-folded, component by component, so a path joined with `/`
/// and one joined with `\` compare equal on Windows.
fn fold_path(path: &Path) -> String {
    let parts: Vec<_> = path.iter().map(|part| part.to_string_lossy()).collect();
    parts.join("/").to_ascii_lowercase()
}

/// `path`'s file name, case-folded.
fn folded_name(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default();
    name.to_string_lossy().to_ascii_lowercase()
}

/// `path`'s directory.
fn source_dir(path: &Path) -> PathBuf {
    path.parent().unwrap_or(Path::new("")).to_path_buf()
}

/// The imports of a screened image.
#[derive(Debug, Default)]
struct Scanned {
    imports: Vec<String>,
    delay_imports: Vec<String>,
    forwarders: Vec<String>,
}

/// Refuses `bytes` (the image `subject` names) unless it is an x86-64
/// PE32+ DLL whose export directory reads, and returns its imports.
fn scan(subject: &str, bytes: &[u8]) -> Result<Scanned, String> {
    let cannot = |e: String| format!("cannot scan {subject}: {e}{EXT}");
    let image = pe::parse_pe(bytes).map_err(cannot)?;
    let image = image.ok_or_else(|| format!("{subject} is not a PE image{EXT}"))?;
    if !image.pe32_plus || image.machine != pe::MACHINE_AMD64 {
        return Err(format!("{subject} is not an x86-64 PE32+ image{EXT}"));
    }
    if !image.dll {
        return Err(format!("{subject} is not a DLL{EXT}"));
    }
    let forwarders = pe::parse_forwarders(bytes).map_err(cannot)?;
    Ok(Scanned {
        imports: image.imports,
        delay_imports: image.delay_imports,
        forwarders,
    })
}

/// The refusal of `subject`'s `verb` (`imports`, `delay-loads`, `forwards
/// to`) of `name`, because it `why`.
fn refusal(subject: &str, verb: &str, name: &str, why: &str) -> String {
    format!("{subject} {verb} `{name}`, which {why}{EXT}")
}

/// Where an import resolves.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// An API set or a system-directory file (R1, R6).
    System,
    /// A root DLL (R2).
    Root,
    /// A scanned closure image (R3, R5).
    Payload,
    /// A native at the source path, by its on-disk name (R4).
    Native(PathBuf, String),
}

/// The image an import is classified for.
struct ImageCtx {
    /// The directory the image is read from.
    source_dir: PathBuf,
    /// A closure image (R3 and R5 apply), not a native.
    closure: bool,
}

/// The resolution sets of one derivation.
#[derive(Default)]
struct Ctx {
    /// The interpreter's DLL, the one delay import or forwarder target
    /// beyond the system.
    ldlibrary: String,
    /// `<base_prefix>\DLLs`, listed when a native is first checked.
    dlls_dir: PathBuf,
    dlls: Option<ListingMap>,
    /// The case-folded root DLL names.
    roots: BTreeSet<String>,
    /// The case-folded names in every system directory.
    system: BTreeSet<String>,
    /// Source directory to its listing, filled as images are classified.
    listings: BTreeMap<PathBuf, ListingMap>,
    /// Every payload file's folded source path.
    payload: BTreeSet<String>,
    /// Every scanned closure image's folded source path.
    images: BTreeSet<String>,
    /// Scanned closure images by folded file name, as messages show them.
    images_by_name: BTreeMap<String, Vec<String>>,
}

impl Ctx {
    fn new(probe: &EmbedProbe, env: &WindowsEnv) -> Self {
        let roots = layout::windows_root_dlls(probe);
        Self {
            ldlibrary: probe.ldlibrary.clone(),
            dlls_dir: probe.base_prefix.join("DLLs"),
            roots: roots.iter().map(|name| name.to_ascii_lowercase()).collect(),
            system: system_names(env),
            ..Self::default()
        }
    }

    /// The classification context of the image read from `source`, its
    /// directory listed first.
    fn image(&mut self, source: &Path, closure: bool) -> Result<ImageCtx, String> {
        let source_dir = source_dir(source);
        if !self.listings.contains_key(&source_dir) {
            let listing = list_dir(&source_dir)?;
            self.listings.insert(source_dir.clone(), listing);
        }
        Ok(ImageCtx {
            source_dir,
            closure,
        })
    }

    /// Whether `name` is an API set or a file in a system directory.
    fn system(&self, name: &str) -> bool {
        pe::is_api_set(name) || self.system.contains(&name.to_ascii_lowercase())
    }

    /// The natives among the static imports of the image `subject` names,
    /// as (source, on-disk name), after refusing an import, delay import
    /// or forwarder that does not resolve.
    fn natives_of(
        &self,
        image: &ImageCtx,
        subject: &str,
        scanned: &Scanned,
    ) -> Result<Vec<(PathBuf, String)>, String> {
        let mut natives = Vec::new();
        for name in &scanned.imports {
            let resolution = classify(name, image, self);
            match resolution.map_err(|why| refusal(subject, "imports", name, &why))? {
                Resolution::Native(source, on_disk) => natives.push((source, on_disk)),
                Resolution::System | Resolution::Root | Resolution::Payload => {}
            }
        }
        let restricted = [
            ("delay-loads", &scanned.delay_imports),
            ("forwards to", &scanned.forwarders),
        ];
        for (verb, names) in restricted {
            for name in names {
                if !(self.system(name) || name.eq_ignore_ascii_case(&self.ldlibrary)) {
                    let why = format!(
                        "is neither an API set, a system DLL, nor `{}`",
                        self.ldlibrary
                    );
                    return Err(refusal(subject, verb, name, &why));
                }
            }
        }
        Ok(natives)
    }

    /// Refuses the native `subject` names, `name`, when it would shadow a
    /// system DLL, or shares its name with a closure image or a file in the
    /// interpreter's `DLLs\`.
    fn check_native_name(&mut self, subject: &str, name: &str) -> Result<(), String> {
        let folded = name.to_ascii_lowercase();
        if self.system.contains(&folded) {
            return Err(format!(
                "{subject} has the name of a system DLL, which it would shadow for every \
                 image{EXT}"
            ));
        }
        if let Some(image) = self.images_by_name.get(&folded).and_then(|v| v.first()) {
            return Err(format!(
                "{subject} has the name of the closure image `{image}`; pycc cannot bundle \
                 both{EXT}"
            ));
        }
        if self.dlls.is_none() {
            self.dlls = Some(list_dir(&self.dlls_dir)?);
        }
        let dlls = self.dlls.as_ref().expect("listed above");
        let clash = |why: String| format!("{subject} {why}{EXT}");
        if let Some(file) = dlls.lookup(name).map_err(clash)? {
            return Err(format!(
                "{subject} has the name of the interpreter's `DLLs\\{file}`, which would bind \
                 in its place; pycc cannot bundle both{EXT}"
            ));
        }
        Ok(())
    }
}

/// Classifies the static import `import` of `image` by the first matching
/// rule of the module doc, or returns why it is refused. Pure: `image`'s
/// directory must already be listed in `ctx` (an unlisted one places
/// nothing by R3 or R4).
fn classify(import: &str, image: &ImageCtx, ctx: &Ctx) -> Result<Resolution, String> {
    let folded = import.to_ascii_lowercase();
    if pe::is_api_set(import) {
        return Ok(Resolution::System);
    }
    if ctx.roots.contains(&folded) {
        return Ok(Resolution::Root);
    }
    if folded == layout::PROGRAM_DLL_NAME {
        return Err("is the name the program DLL is already loaded under".to_string());
    }
    let listing = ctx.listings.get(&image.source_dir);
    if let Some(on_disk) = listing.map(|l| l.lookup(import)).transpose()?.flatten() {
        let source = image.source_dir.join(on_disk);
        let key = fold_path(&source);
        if image.closure && ctx.images.contains(&key) {
            return Ok(Resolution::Payload);
        }
        if !ctx.payload.contains(&key) {
            return Ok(Resolution::Native(source, on_disk.to_string()));
        }
    }
    if image.closure {
        match ctx.images_by_name.get(&folded).map(Vec::as_slice) {
            Some([_]) => return Ok(Resolution::Payload),
            Some([first, second, ..]) => {
                return Err(format!(
                    "matches more than one locked image of the closure (`{first}` and \
                     `{second}`)"
                ));
            }
            _ => {}
        }
    }
    if ctx.system.contains(&folded) {
        return Ok(Resolution::System);
    }
    Err(if image.closure {
        CLOSURE_WHY
    } else {
        NATIVE_WHY
    }
    .to_string())
}

#[cfg(test)]
#[path = "native_windows_closure_tests.rs"]
mod tests;
