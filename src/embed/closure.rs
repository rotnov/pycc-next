//! The locked dependency closure in an embedded executable's sidecar
//! (the pycc.lock decision entry, rules 7 and 8; #1242): the copy of every
//! locked payload file into `<staging>/closure/`, checked against the lock
//! as it is written, and the file listing the macOS relocation of closure
//! images resolves `@rpath` candidates against.

use super::bundle::io_error;
use super::macho;
use super::sha256::sha256_hex;
use crate::lock::build::LockedClosure;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// The sidecar directory the closure is copied into.
pub(crate) const CLOSURE_DIR: &str = "closure";

/// Copies every planned payload file to `<staging>/closure/<rel>`: each is
/// read once, its bytes hashed and checked against the RECORD digest the
/// lock was derived from, and written to a destination that must not exist
/// yet, so two payload paths that differ only in case cannot overwrite each
/// other on a case-insensitive file system. The source's permission bits
/// are kept, plus owner-write for the relocation step. Last, each
/// distribution's copied files must add up to its locked `files` and
/// `tree-sha256`. Returns the copied Mach-O images' `rel` paths, in order.
///
/// The files are copied on every core (`super::parallel`), except those
/// whose path another payload path matches but for case: those are copied
/// afterwards, in order, so which of them is refused, and the message
/// naming the other, is what copying all of them in order gives. The first
/// failure in file order is reported.
pub(crate) fn copy_closure(closure: &LockedClosure, staging: &Path) -> Result<Vec<String>, String> {
    let root = staging.join(CLOSURE_DIR);
    let files = &closure.files;
    let folded: Vec<String> = files.iter().map(|file| file.rel.to_lowercase()).collect();
    let mut first_with: BTreeMap<&str, usize> = BTreeMap::new();
    let mut shared = vec![false; files.len()];
    for (index, name) in folded.iter().enumerate() {
        if let Some(first) = first_with.insert(name, index) {
            shared[first] = true;
            shared[index] = true;
        }
    }
    let mut copied: Vec<Option<Result<(String, bool), CopyError>>> =
        super::parallel::map_indexed(files.len(), |index| {
            (!shared[index]).then(|| copy_file(closure, &files[index], &root))
        });
    let mut owners: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for index in (0..files.len()).filter(|index| shared[*index]) {
        let file = &files[index];
        let result = copy_file(closure, file, &root).map_err(|e| match e {
            CopyError::Collided(e) => match owners.get(folded[index].as_str()) {
                Some((other, other_package)) => closure.stale(&format!(
                    "`{}` of distribution `{}` and `{other}` of distribution \
                     `{other_package}` name the same file on this case-insensitive file \
                     system",
                    file.rel, file.package
                )),
                None => e,
            },
            CopyError::Other(e) => e,
        });
        if result.is_ok() {
            owners.insert(folded[index].as_str(), (&file.rel, &file.package));
        }
        copied[index] = Some(result.map_err(CopyError::Other));
    }
    let mut digests: BTreeMap<String, String> = BTreeMap::new();
    let mut images = Vec::new();
    for (file, result) in files.iter().zip(copied) {
        let (digest, image) = result.expect("every file copied").map_err(CopyError::message)?;
        if image {
            images.push(file.rel.clone());
        }
        digests.insert(file.rel.clone(), digest);
    }
    closure.verify_copied(&digests)?;
    Ok(images)
}

/// Why one payload file was not copied.
enum CopyError {
    /// Its destination exists already (another payload path that differs
    /// only in case, on a case-insensitive file system): the message to
    /// report when no earlier file explains it.
    Collided(String),
    Other(String),
}

impl CopyError {
    fn message(self) -> String {
        match self {
            CopyError::Collided(message) | CopyError::Other(message) => message,
        }
    }
}

/// Copies one payload file (see [`copy_closure`]); its digest, and whether
/// it is a Mach-O image.
fn copy_file(
    closure: &LockedClosure,
    file: &crate::lock::build::ClosureFile,
    root: &Path,
) -> Result<(String, bool), CopyError> {
    let source = &file.source;
    let bytes = std::fs::read(source).map_err(|e| CopyError::Other(io_error("read", source, &e)))?;
    let digest = sha256_hex(&bytes);
    if digest != file.digest {
        return Err(CopyError::Other(closure.stale(&format!(
            "`{}` of distribution `{}` does not match its RECORD hash, so it changed after \
             it was installed -- reinstall the distribution",
            source.display(),
            file.package
        ))));
    }
    let dest = root.join(&file.rel);
    let parent = dest.parent().unwrap_or(root);
    std::fs::create_dir_all(parent).map_err(|e| CopyError::Other(io_error("create", parent, &e)))?;
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest);
    let mut out = created.map_err(|e| {
        let message = io_error("create", &dest, &e);
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            CopyError::Collided(message)
        } else {
            CopyError::Other(message)
        }
    })?;
    out.write_all(&bytes)
        .map_err(|e| CopyError::Other(io_error("write", &dest, &e)))?;
    keep_mode(source, &dest).map_err(CopyError::Other)?;
    Ok((digest, macho::is_macho_header(&bytes)))
}

/// Gives `dest` the permission bits of `source`, plus owner-write, so a
/// payload executable stays executable and the relocation can rewrite it.
#[cfg(unix)]
fn keep_mode(source: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(source).map_err(|e| io_error("inspect", source, &e))?;
    let mode = std::fs::Permissions::from_mode(metadata.permissions().mode() | 0o200);
    std::fs::set_permissions(dest, mode).map_err(|e| io_error("set the mode of", dest, &e))
}

/// A Windows closure file keeps the default mode: Windows has no execute
/// bit, and no image in it is relocated (#1296).
#[cfg(not(unix))]
fn keep_mode(_source: &Path, _dest: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
#[path = "closure_tests.rs"]
mod tests;
