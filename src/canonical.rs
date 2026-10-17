//! Canonical paths a user reads.
//!
//! On Windows `std::fs::canonicalize` returns a verbatim path
//! (`\\?\D:\src\m.py`, `\\?\UNC\server\share\m.py`). Those reach CPython as
//! a module's `__file__` and diagnostics as the file a message names, where
//! CPython itself would show `D:\src\m.py`. [`canonicalize`] drops the
//! verbatim prefix when the path means the same without it; elsewhere it is
//! `std::fs::canonicalize`.

use std::path::{Path, PathBuf};

/// `path` resolved like [`std::fs::canonicalize`], without a Windows
/// verbatim prefix.
pub(crate) fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    if cfg!(windows) {
        Ok(without_verbatim(resolved))
    } else {
        Ok(resolved)
    }
}

/// `path` without its `\\?\` (drive) or `\\?\UNC\` (share) prefix, when
/// what follows is an ordinary absolute path: no component is `.` or `..`
/// and it fits `MAX_PATH`, the two things a verbatim path may hold that its
/// plain spelling cannot. Pure, so every host tests it.
fn without_verbatim(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let drive = rest.as_bytes();
        if drive.len() < 3 || !drive[0].is_ascii_alphabetic() || &drive[1..3] != br":\" {
            return path;
        }
        rest.to_string()
    } else {
        return path;
    };
    const MAX_PATH: usize = 260;
    let dotted = plain.split('\\').any(|part| part == "." || part == "..");
    if dotted || plain.len() >= MAX_PATH {
        return path;
    }
    PathBuf::from(plain)
}

#[cfg(test)]
mod tests {
    use super::without_verbatim;
    use std::path::PathBuf;

    fn plain(text: &str) -> String {
        without_verbatim(PathBuf::from(text))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn a_verbatim_drive_or_share_path_loses_its_prefix() {
        assert_eq!(plain(r"\\?\D:\src\m.py"), r"D:\src\m.py");
        assert_eq!(plain(r"\\?\UNC\server\share\m.py"), r"\\server\share\m.py");
    }

    #[test]
    fn other_paths_are_kept() {
        for kept in [
            "/home/u/m.py",
            r"D:\src\m.py",
            r"\\?\Volume{0f}\m.py",
            r"\\?\D:\a\..\m.py",
            r"\\?\D:",
        ] {
            assert_eq!(plain(kept), kept);
        }
        let long = format!(r"\\?\D:\{}", "x".repeat(300));
        assert_eq!(plain(&long), long);
    }
}
