//! Where `pycc` finds the files it needs at run time.
//!
//! A release archive unpacks to
//!
//! ```text
//! pycc-vX.Y.Z-<arch>-<os>/
//!   bin/pycc
//!   lib/pycc/libpycc_rt.a
//!   README.md  LICENSE  CHANGELOG.md
//! ```
//!
//! (`scripts/package.sh` builds it.) The runtime archive is the only file
//! the driver reads from its own installation: the C sources of the `--ext`
//! shim and the embedded-executable launcher are compiled into the binary
//! with `include_str!`. Everything is resolved relative to the running
//! executable (`std::env::current_exe`, which resolves symlinks on Linux and
//! macOS, so a `/usr/local/bin/pycc` link into an unpacked archive works),
//! never relative to the checkout `pycc` was compiled in.
//!
//! Lookup order for the runtime directory:
//!
//! 1. installed: `<exe>/../../lib/pycc/` (host target only; that archive is
//!    the one built with this binary and serves both profiles);
//! 2. development: a Cargo target directory -- `CARGO_TARGET_DIR` or
//!    `CARGO_BUILD_TARGET_DIR` when set, else the one this executable sits
//!    in (`<root>/<profile>/pycc`, or `<root>/<profile>/deps/` for a test
//!    harness), searched by
//!    [`pycc_codegen::artifact_layout::find_pycc_rt_lib_dir_in`].

use pycc_codegen::artifact_layout::{
    cargo_target_dir_from_env, find_pycc_rt_lib_dir_in, pycc_rt_lib_filename, target_dir_from_env,
};
use std::path::{Path, PathBuf};

/// Locates the directory holding `pycc_rt`'s static library for this
/// process. See the module docs for the order.
pub(crate) fn find_runtime_dir(target: Option<&str>, release: bool) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().ok();
    find_runtime_dir_for(
        exe.as_deref(),
        target_dir_from_env(cargo_target_dir_from_env),
        target,
        release,
        Path::is_file,
    )
}

/// [`find_runtime_dir`] with the executable path, the environment's target
/// directory and the existence check injected.
pub(crate) fn find_runtime_dir_for(
    exe: Option<&Path>,
    env_target_root: Option<PathBuf>,
    target: Option<&str>,
    release: bool,
    exists: fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let prefix = exe.and_then(Path::parent).and_then(Path::parent);
    let installed = prefix.map(|p| p.join("lib").join("pycc"));
    if target.is_none()
        && let Some(dir) = &installed
        && exists(&dir.join(pycc_rt_lib_filename(None)))
    {
        return Ok(dir.clone());
    }
    let env_target_root_is_unset = env_target_root.is_none();
    let Some(target_root) = env_target_root.or_else(|| prefix.map(Path::to_path_buf)) else {
        return Err(
            "cannot locate pycc's runtime library: the path of the running pycc is unknown"
                .to_string(),
        );
    };
    // A test harness binary sits one level deeper, in `<root>/<profile>/deps/`.
    if env_target_root_is_unset
        && let Some(root) = target_root.parent()
        && exe.and_then(Path::parent).and_then(Path::file_name) == Some("deps".as_ref())
        && let Ok(dir) = find_pycc_rt_lib_dir_in(root, target, release, exists)
    {
        return Ok(dir);
    }
    find_pycc_rt_lib_dir_in(&target_root, target, release, exists).map_err(|dev| match installed {
        // An installed pycc missing its runtime is the likelier story for a
        // host build, so name the installed path first.
        Some(dir) if target.is_none() => format!(
            "pycc's runtime library is missing: expected {} beside this pycc (a release \
             archive ships it there; keep the unpacked directory together). For a \
             development build: {dev}",
            dir.join(pycc_rt_lib_filename(None)).display()
        ),
        _ => dev,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &Path) -> bool {
        true
    }

    fn never(_: &Path) -> bool {
        false
    }

    fn only_dev_release(path: &Path) -> bool {
        path == Path::new("/w/tgt/release/libpycc_rt.a")
    }

    #[test]
    fn an_installed_runtime_sits_in_lib_pycc_beside_bin() {
        let dir =
            find_runtime_dir_for(Some(Path::new("/opt/p/bin/pycc")), None, None, true, always);
        assert_eq!(dir, Ok(PathBuf::from("/opt/p/lib/pycc")));
        let debug = find_runtime_dir_for(
            Some(Path::new("/opt/p/bin/pycc")),
            None,
            None,
            false,
            always,
        );
        assert_eq!(debug, Ok(PathBuf::from("/opt/p/lib/pycc")));
    }

    #[test]
    fn a_development_binary_searches_its_own_target_directory() {
        let exe = Path::new("/w/tgt/release/pycc");
        let dir = find_runtime_dir_for(Some(exe), None, None, true, only_dev_release);
        assert_eq!(dir, Ok(PathBuf::from("/w/tgt/release")));
    }

    #[test]
    fn a_test_harness_binary_searches_the_target_directory_above_deps() {
        let exe = Path::new("/w/tgt/release/deps/pycc-0123");
        let dir = find_runtime_dir_for(Some(exe), None, None, true, only_dev_release);
        assert_eq!(dir, Ok(PathBuf::from("/w/tgt/release")));
    }

    #[test]
    fn the_environment_target_directory_wins_over_the_executable_s() {
        let exe = Path::new("/elsewhere/release/pycc");
        let env = Some(PathBuf::from("/w/tgt"));
        let dir = find_runtime_dir_for(Some(exe), env, None, true, only_dev_release);
        assert_eq!(dir, Ok(PathBuf::from("/w/tgt/release")));
    }

    #[test]
    fn a_cross_target_never_uses_the_installed_host_runtime() {
        let exe = Path::new("/opt/p/bin/pycc");
        let err = find_runtime_dir_for(
            Some(exe),
            None,
            Some("riscv64-unknown-linux-gnu"),
            true,
            never,
        )
        .unwrap_err();
        assert!(
            err.starts_with("no pycc_rt build found for target"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_runtime_names_the_installed_path_first() {
        let exe = Path::new("/opt/p/bin/pycc");
        let err = find_runtime_dir_for(Some(exe), None, None, true, never).unwrap_err();
        assert!(
            err.contains("expected /opt/p/lib/pycc/libpycc_rt.a beside this pycc"),
            "{err}"
        );
        assert!(err.contains("no pycc_rt build found"), "{err}");
    }

    #[test]
    fn an_unknown_executable_path_is_reported() {
        let err = find_runtime_dir_for(None, None, None, true, always).unwrap_err();
        assert!(err.contains("path of the running pycc is unknown"), "{err}");
        let bare = find_runtime_dir_for(Some(Path::new("pycc")), None, None, true, always);
        assert!(bare.is_err());
    }
}
