//! The Windows embedded build's interpreter-image scan (#1305, Part 1 of
//! #1297): every PE image the sidecar bundles from the interpreter (the
//! root DLLs and the kept `DLLs\`) is parsed ([`super::pe`]), and one whose
//! imports would not resolve once the pair is moved is refused before
//! anything is staged, as D-248 rule 5 refuses a Linux interpreter image.
//! A PE image under the kept `Lib\` is refused outright: pycc scans none
//! there.
//!
//! A static import resolves when it is an API set, a root DLL (the
//! launcher's `AddDllDirectory(<sidecar>)`), a kept image in a `DLLs\`
//! image's own directory (the loader's `DLL_LOAD_DIR`), or a file in a
//! system directory. A delay import resolves through the standard search
//! order, which reaches neither `DLLs\` nor `AddDllDirectory`, so only an
//! API set, a system file or the interpreter's DLL (which the program DLL
//! loaded first) is accepted.
//!
//! Nothing here depends on the host: the system directories are injected
//! ([`WindowsEnv`]) and every path in a message is rendered with `\`, so the
//! scan runs, and is tested, on every host.

use super::EmbedProbe;
use super::bundle::{self, Kept, io_error};
use super::layout;
use super::pe;
use super::windows::is_windows_image;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Where the scan looks for system DLLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowsEnv {
    /// The system directories: an import naming a file in one of them is a
    /// system DLL, which must also exist on the target (D-253's NEG).
    pub(crate) system_dirs: Vec<PathBuf>,
}

impl WindowsEnv {
    /// The build host's: `%SystemRoot%\System32`.
    pub(crate) fn host() -> Self {
        Self::from_system_root(std::env::var_os("SystemRoot"))
    }

    /// `<root>\System32`, else `C:\Windows\System32`, built by string
    /// concatenation with `\` (not [`Path::join`], which inserts `/` on a
    /// POSIX host), so both arms are tested on every host.
    pub(crate) fn from_system_root(root: Option<OsString>) -> Self {
        let mut dir = root.unwrap_or_else(|| OsString::from("C:\\Windows"));
        dir.push("\\System32");
        Self {
            system_dirs: vec![PathBuf::from(dir)],
        }
    }
}

/// Parses every PE image the sidecar bundles from the interpreter, in
/// sorted order, and refuses the first whose imports would not resolve
/// after the pair is moved, or that is not an x86-64 PE32+ DLL. Then
/// refuses a PE image under the kept `Lib\`.
pub(crate) fn scan_interpreter(probe: &EmbedProbe, env: &WindowsEnv) -> Result<(), String> {
    let roots = layout::windows_root_dlls(probe);
    let scan = Scan {
        probe,
        roots: roots.iter().map(|name| name.to_ascii_lowercase()).collect(),
        system: system_names(env),
    };
    for name in &roots {
        let path = probe.base_prefix.join(name);
        let bytes = std::fs::read(&path).map_err(|e| io_error("read", &path, &e))?;
        scan.image(name, &bytes, None)?;
    }
    let dlls = probe.base_prefix.join("DLLs");
    let mut images = Vec::new();
    for rel in kept_files(&dlls)? {
        let path = dlls.join(&rel);
        let bytes = std::fs::read(&path).map_err(|e| io_error("read", &path, &e))?;
        if is_windows_image(&backslashed(&rel), &bytes) {
            images.push((rel, bytes));
        }
    }
    // A sibling is a kept image in the same directory; a kept non-image
    // file of the imported name is not one.
    let mut siblings: BTreeMap<&Path, BTreeSet<String>> = BTreeMap::new();
    for (rel, _) in &images {
        let name = rel.file_name().unwrap_or_default().to_string_lossy();
        let dir = siblings.entry(parent(rel)).or_default();
        dir.insert(name.to_ascii_lowercase());
    }
    for (rel, bytes) in &images {
        let shown = format!("DLLs\\{}", backslashed(rel));
        scan.image(&shown, bytes, siblings.get(parent(rel)))?;
    }
    refuse_lib_images(probe)
}

/// The resolution sets of one scan.
struct Scan<'a> {
    probe: &'a EmbedProbe,
    /// The case-folded root DLL names.
    roots: BTreeSet<String>,
    /// The case-folded names in every system directory.
    system: BTreeSet<String>,
}

impl Scan<'_> {
    /// Refuses the image `rel` (as a message shows it) unless it is an
    /// x86-64 PE32+ DLL whose every import resolves. `siblings` holds the
    /// case-folded kept images of a `DLLs\` image's own directory; `None`
    /// for a root image.
    fn image(
        &self,
        rel: &str,
        bytes: &[u8],
        siblings: Option<&BTreeSet<String>>,
    ) -> Result<(), String> {
        let image = pe::parse_pe(bytes).map_err(|e| format!("cannot scan `{rel}`: {e}"))?;
        let image = image.ok_or_else(|| format!("cannot scan `{rel}`: not a PE image"))?;
        if !image.pe32_plus || image.machine != pe::MACHINE_AMD64 {
            return Err(format!("`{rel}` is not an x86-64 PE32+ image"));
        }
        if !image.dll {
            return Err(format!("`{rel}` is not a DLL"));
        }
        for name in &image.imports {
            let lower = name.to_ascii_lowercase();
            let sibling = siblings.is_some_and(|siblings| siblings.contains(&lower));
            if !(self.system(name) || self.roots.contains(&lower) || sibling) {
                let why =
                    "neither a system DLL, a DLL the sidecar bundles, nor a sibling in `DLLs\\`";
                return Err(self.unresolved(rel, "imports", name, why));
            }
        }
        for name in &image.delay_imports {
            if !(self.system(name) || name.eq_ignore_ascii_case(&self.probe.ldlibrary)) {
                let why = format!("neither a system DLL nor `{}`", self.probe.ldlibrary);
                return Err(self.unresolved(rel, "delay-loads", name, &why));
            }
        }
        Ok(())
    }

    /// Whether `name` is an API set or a file in a system directory.
    fn system(&self, name: &str) -> bool {
        pe::is_api_set(name) || self.system.contains(&name.to_ascii_lowercase())
    }

    fn unresolved(&self, rel: &str, verb: &str, name: &str, why: &str) -> String {
        format!(
            "the embed interpreter `{}` is not relocatable: `{rel}` {verb} `{name}`, which is \
             {why}; pycc cannot bundle it",
            self.probe.executable.display()
        )
    }
}

/// The case-folded file names in every system directory. A directory that
/// is missing or unreadable counts as empty, so its imports fall to the
/// refusal.
pub(super) fn system_names(env: &WindowsEnv) -> BTreeSet<String> {
    let listings = env
        .system_dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok());
    let entries = listings.flatten().flatten();
    entries
        .map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase())
        .collect()
}

/// Refuses a kept file under `Lib\` that is a PE image by name or by its
/// `MZ` magic.
fn refuse_lib_images(probe: &EmbedProbe) -> Result<(), String> {
    for rel in kept_files(&probe.stdlib)? {
        let path = probe.stdlib.join(&rel);
        let file = std::fs::File::open(&path).map_err(|e| io_error("read", &path, &e))?;
        let mut head = Vec::new();
        file.take(2)
            .read_to_end(&mut head)
            .map_err(|e| io_error("read", &path, &e))?;
        let shown = backslashed(&rel);
        if is_windows_image(&shown, &head) {
            return Err(format!(
                "`Lib\\{shown}` is a PE image outside `DLLs\\`; pycc cannot scan it"
            ));
        }
    }
    Ok(())
}

/// The files the Windows copy keeps under `root`, sorted.
fn kept_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let kept = bundle::kept_entries(root, layout::skip_in_windows_stdlib_copy)?;
    let mut files: Vec<PathBuf> = kept
        .into_iter()
        .filter_map(|entry| match entry {
            Kept::File(rel) => Some(rel),
            Kept::Dir(_) => None,
        })
        .collect();
    files.sort();
    Ok(files)
}

/// `rel`'s directory; `""` for a file at the root.
fn parent(rel: &Path) -> &Path {
    rel.parent().unwrap_or(Path::new(""))
}

/// `rel`'s components joined with `\`, whatever the host's separator.
pub(super) fn backslashed(rel: &Path) -> String {
    let parts: Vec<_> = rel.iter().map(|part| part.to_string_lossy()).collect();
    parts.join("\\")
}

#[cfg(test)]
#[path = "native_windows_tests.rs"]
mod tests;
