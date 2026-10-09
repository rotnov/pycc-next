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
pub(crate) fn copy_closure(closure: &LockedClosure, staging: &Path) -> Result<Vec<String>, String> {
    let root = staging.join(CLOSURE_DIR);
    let mut copied: BTreeMap<String, String> = BTreeMap::new();
    let mut owners: BTreeMap<String, (&str, &str)> = BTreeMap::new();
    let mut images = Vec::new();
    for file in &closure.files {
        let source = &file.source;
        let bytes = std::fs::read(source).map_err(|e| io_error("read", source, &e))?;
        let digest = sha256_hex(&bytes);
        if digest != file.digest {
            return Err(closure.stale(&format!(
                "`{}` of distribution `{}` does not match its RECORD hash, so it changed after \
                 it was installed -- reinstall the distribution",
                source.display(),
                file.package
            )));
        }
        let dest = root.join(&file.rel);
        let parent = dest.parent().unwrap_or(&root);
        std::fs::create_dir_all(parent).map_err(|e| io_error("create", parent, &e))?;
        let folded = file.rel.to_lowercase();
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest);
        let mut out = created.map_err(|e| {
            let collided = e.kind() == std::io::ErrorKind::AlreadyExists;
            let other = owners.get(&folded).filter(|_| collided);
            other.map_or_else(
                || io_error("create", &dest, &e),
                |(other, other_package)| {
                    closure.stale(&format!(
                        "`{}` of distribution `{}` and `{other}` of distribution \
                         `{other_package}` name the same file on this case-insensitive file \
                         system",
                        file.rel, file.package
                    ))
                },
            )
        })?;
        out.write_all(&bytes)
            .map_err(|e| io_error("write", &dest, &e))?;
        keep_mode(source, &dest)?;
        owners.insert(folded, (&file.rel, &file.package));
        if macho::is_macho_header(&bytes) {
            images.push(file.rel.clone());
        }
        copied.insert(file.rel.clone(), digest);
    }
    closure.verify_copied(&copied)?;
    Ok(images)
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
