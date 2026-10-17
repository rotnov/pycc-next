//! The build cache of the `--ext` shim's runtime unit (v0.8 accept 4,
//! `crate::ext_build`'s `shim_split`).
//!
//! The runtime unit is the same text for every module a pycc version builds,
//! so its object depends only on that text, the compile flags (the CPython
//! header directory among them), the CPython headers and the C compiler.
//! The first build compiles it as before and stores the object under a key
//! of all four; every later build with the same key links the stored object
//! instead of compiling it again.
//!
//! The cache directory is `PYCC_CACHE_DIR` when set (an empty value turns the
//! cache off), else the platform's user cache directory (`$XDG_CACHE_HOME`
//! or `~/.cache` on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on
//! Windows) with `pycc/c-objects` below it. A directory that cannot be
//! created, read or written only disables the cache: the object is compiled.
//! An object is stored by writing a file of its own and renaming it into
//! place, so concurrent builds never read a partial one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where cached objects live, or `None` when the cache is off or there is
/// no user cache directory.
pub(super) fn cache_dir() -> Option<PathBuf> {
    dir_from(|name| std::env::var_os(name))
}

fn dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = var("PYCC_CACHE_DIR") {
        return (!dir.is_empty()).then(|| PathBuf::from(dir));
    }
    let base = if cfg!(windows) {
        PathBuf::from(var("LOCALAPPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(var("HOME")?).join("Library").join("Caches")
    } else {
        match var("XDG_CACHE_HOME").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(var("HOME")?).join(".cache"),
        }
    };
    Some(base.join("pycc").join("c-objects"))
}

/// The key of compiling `source` with `flags` by `compiler`: a digest of
/// pycc's version, the compiler's own `--version` text, the flags, the
/// headers' version (`patchlevel.h` of every `-I` directory) and the source.
/// `None` when the compiler does not answer `--version`.
pub(super) fn key(compiler: Command, flags: &[&OsString], source: &Path) -> Option<String> {
    let mut version = compiler;
    let output = version.arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut digest = crate::embed::sha256::Sha256::new();
    let mut part = |bytes: &[u8]| {
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    };
    part(env!("CARGO_PKG_VERSION").as_bytes());
    part(version.get_program().as_encoded_bytes());
    part(&output.stdout);
    let mut include = false;
    for flag in flags {
        part(flag.as_encoded_bytes());
        if include {
            let patchlevel = Path::new(flag).join("patchlevel.h");
            part(&std::fs::read(patchlevel).unwrap_or_default());
        }
        include = flag.as_encoded_bytes() == b"-I";
    }
    part(&std::fs::read(source).ok()?);
    Some(digest.finish())
}

/// The stored object of `key` in `dir`, if there is one.
pub(super) fn lookup(dir: &Path, key: &str, extension: &str) -> Option<PathBuf> {
    let path = dir.join(format!("{key}.{extension}"));
    path.is_file().then_some(path)
}

/// Stores `object` as the object of `key` in `dir`, best effort: a failure
/// leaves the cache as it was.
pub(super) fn store(dir: &Path, key: &str, extension: &str, object: &Path) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temporary = dir.join(format!(
        "{key}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    if std::fs::copy(object, &temporary).is_ok()
        && std::fs::rename(&temporary, dir.join(format!("{key}.{extension}"))).is_err()
    {
        let _ = std::fs::remove_file(&temporary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_cache_directory_follows_pycc_cache_dir_and_an_empty_one_turns_it_off() {
        assert_eq!(dir_from(vars(&[("PYCC_CACHE_DIR", "/c")])), Some(PathBuf::from("/c")));
        assert_eq!(dir_from(vars(&[("PYCC_CACHE_DIR", ""), ("HOME", "/h")])), None);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_linux_cache_directory_is_xdg_cache_home_or_dot_cache() {
        assert_eq!(
            dir_from(vars(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x/pycc/c-objects"))
        );
        assert_eq!(
            dir_from(vars(&[("HOME", "/h")])),
            Some(PathBuf::from("/h/.cache/pycc/c-objects"))
        );
        assert_eq!(dir_from(vars(&[])), None);
    }

    #[test]
    fn a_stored_object_is_found_by_its_key_and_the_key_follows_the_source() {
        let scratch = pycc_scratch::ScratchDir::new("object-cache").unwrap();
        let object = scratch.join("o.o");
        std::fs::write(&object, b"object bytes").unwrap();
        let dir = scratch.join("cache");
        assert_eq!(lookup(&dir, "k", "o"), None);
        store(&dir, "k", "o", &object);
        let found = lookup(&dir, "k", "o").expect("stored");
        assert_eq!(std::fs::read(found).unwrap(), b"object bytes");
        let source = scratch.join("s.c");
        std::fs::write(&source, "int a;\n").unwrap();
        let flags = [OsString::from("-O2")];
        let flags: Vec<&OsString> = flags.iter().collect();
        let Some(first) = key(Command::new("cc"), &flags, &source) else {
            return; // no C compiler on this host
        };
        assert_eq!(key(Command::new("cc"), &flags, &source), Some(first.clone()));
        std::fs::write(&source, "int b;\n").unwrap();
        assert_ne!(key(Command::new("cc"), &flags, &source), Some(first.clone()));
        let other = [OsString::from("-O1")];
        let other: Vec<&OsString> = other.iter().collect();
        std::fs::write(&source, "int a;\n").unwrap();
        assert_ne!(key(Command::new("cc"), &other, &source), Some(first));
    }
}
