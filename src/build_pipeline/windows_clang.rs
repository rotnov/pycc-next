//! Which `clang.exe` compiles pycc's C sources and drives the link on
//! Windows (`linker_command` in `src/build_pipeline.rs`).
//!
//! Windows has no `cc`. clang's driver accepts the GCC-style flags the rest
//! of the pipeline passes (`-c`, `-L`, `-l`, `-o`) and runs MSVC's
//! `link.exe` for an `x86_64-pc-windows-msvc` target. A release archive
//! runs on machines other than the one that built it, so the clang is
//! looked up where a user has one, in this order:
//!
//! 1. `PYCC_CLANG`, when set: that executable;
//! 2. `clang.exe` in a `PATH` directory (the LLVM installer's "add to
//!    PATH", or a developer prompt);
//! 3. Visual Studio's own clang (the "C++ Clang Compiler for Windows"
//!    component), `%VCINSTALLDIR%\Tools\Llvm\x64\bin\clang.exe`;
//! 4. the LLVM installer's default, `%ProgramFiles%\LLVM\bin\clang.exe`;
//! 5. the clang of the LLVM this pycc was built against
//!    (`LLVM_SYS_221_PREFIX` at build time), which exists for a
//!    development build.
//!
//! Pure over its inputs, so every host tests the order.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The clang to run, from the environment `var` and the file check
/// `is_file`; `build_prefix` is the LLVM prefix pycc was built against.
pub(crate) fn resolve(
    var: impl Fn(&str) -> Option<OsString>,
    is_file: impl Fn(&Path) -> bool,
    build_prefix: &Path,
) -> PathBuf {
    if let Some(clang) = var("PYCC_CLANG").filter(|v| !v.is_empty()) {
        return PathBuf::from(clang);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = var("PATH") {
        candidates.extend(split_windows_path(&path).map(|dir| dir.join("clang.exe")));
    }
    if let Some(vc) = var("VCINSTALLDIR") {
        candidates.push(PathBuf::from(vc).join(r"Tools\Llvm\x64\bin\clang.exe"));
    }
    if let Some(program_files) = var("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join(r"LLVM\bin\clang.exe"));
    }
    candidates
        .into_iter()
        .find(|candidate| is_file(candidate))
        .unwrap_or_else(|| build_prefix.join("bin").join("clang.exe"))
}

/// The directories of a Windows `PATH` value (`;`-separated, entries may
/// be quoted, empty entries skipped). Not `std::env::split_paths`, whose
/// separator is the host's, so the order is tested on every host.
fn split_windows_path(path: &OsString) -> impl Iterator<Item = PathBuf> + '_ {
    path.to_str()
        .unwrap_or("")
        .split(';')
        .map(|dir| dir.trim_matches('"'))
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    const PREFIX: &str = r"D:\build\llvm";

    #[test]
    fn pycc_clang_wins() {
        let got = resolve(
            env(&[("PYCC_CLANG", r"C:\x\clang.exe"), ("PATH", r"C:\llvm\bin")]),
            |_| true,
            Path::new(PREFIX),
        );
        assert_eq!(got, PathBuf::from(r"C:\x\clang.exe"));
    }

    #[test]
    fn the_first_path_directory_holding_clang_comes_next() {
        let path = r#"C:\a;;"C:\llvm\bin";C:\other"#;
        let got = resolve(
            env(&[
                ("PYCC_CLANG", ""),
                ("PATH", path),
                ("ProgramFiles", r"C:\PF"),
            ]),
            |p| p.starts_with(r"C:\llvm\bin") || p.starts_with(r"C:\other"),
            Path::new(PREFIX),
        );
        assert_eq!(got, Path::new(r"C:\llvm\bin").join("clang.exe"));
    }

    #[test]
    fn then_visual_studio_s_clang_then_the_llvm_installer_s() {
        let vars = [
            ("PATH", r"C:\a"),
            ("VCINSTALLDIR", r"C:\VS\VC"),
            ("ProgramFiles", r"C:\PF"),
        ];
        let vs = Path::new(r"C:\VS\VC").join(r"Tools\Llvm\x64\bin\clang.exe");
        let installer = Path::new(r"C:\PF").join(r"LLVM\bin\clang.exe");
        assert_eq!(
            resolve(env(&vars), |_| true, Path::new(PREFIX)),
            Path::new(r"C:\a").join("clang.exe")
        );
        let only = |want: PathBuf| move |p: &Path| p == want;
        assert_eq!(resolve(env(&vars), only(vs.clone()), Path::new(PREFIX)), vs);
        assert_eq!(
            resolve(env(&vars), only(installer.clone()), Path::new(PREFIX)),
            installer
        );
    }

    #[test]
    fn the_build_time_llvm_is_the_last_resort() {
        let got = resolve(env(&[]), |_| false, Path::new(PREFIX));
        assert_eq!(got, Path::new(PREFIX).join("bin").join("clang.exe"));
    }
}
