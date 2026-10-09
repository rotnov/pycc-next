//! The static-libpython variant of an embedded build (Part 1 of #1227;
//! the static-libpython decision entry under `docs/decisions/`): the
//! separate probe that finds the interpreter's `LIBPL` archive, the checks
//! that the archive is one the linker can force-load, the refusals the
//! variant adds, and the file that identifies an interpreter in
//! `pycc.lock` whichever way a build links it (Part 2, #1272). The link
//! arguments themselves are pure layout rules in `layout.rs`.

use super::EmbedProbe;
use super::layout::{self, EmbedPlatform};
use super::native::read_head;
use std::path::{Path, PathBuf};

/// Asks an interpreter where its static libpython is and which system
/// libraries its members need, one value per line (`LIBPL`, `LIBRARY`,
/// `LIBS`, `SYSLIBS`). The first line is a comment so a fake interpreter
/// can tell this probe from the embed probe. `MODLIBS` is left out on
/// purpose: it names build-tree paths that do not exist in an install.
pub(crate) const STATIC_PROBE_SCRIPT: &str = "# pycc-static-probe\n\
     import sysconfig\n\
     for k in ('LIBPL','LIBRARY','LIBS','SYSLIBS'):\n\
     \x20   v = sysconfig.get_config_var(k)\n\
     \x20   print('' if v is None else v)\n";

/// What the static probe reports: the archive (`LIBPL/LIBRARY`, symlinks
/// resolved once [`check_archive`] accepts it) and the system-library
/// tokens the link appends after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaticProbe {
    pub(crate) archive: PathBuf,
    pub(crate) libs: Vec<String>,
}

/// Parses [`STATIC_PROBE_SCRIPT`]'s four lines. `LIBS` and `SYSLIBS` are
/// split on whitespace, as `python3-config --ldflags` does. Pure.
pub(crate) fn parse_static_probe(stdout: &str) -> Option<StaticProbe> {
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let [libpl, library, libs, syslibs]: [&str; 4] = lines.get(..4)?.try_into().ok()?;
    let libs = libs.split_whitespace().chain(syslibs.split_whitespace());
    Some(StaticProbe {
        archive: Path::new(libpl).join(library),
        libs: libs.map(str::to_string).collect(),
    })
}

/// How every refusal of this variant names what asked for it.
pub(crate) const STATIC_REQUEST: &str =
    "`--static-libpython` or `[build] static = true` in `pycc.toml`";

/// An ar archive's first eight bytes.
const AR_MAGIC: &[u8] = b"!<arch>\n";

/// A thin archive's first eight bytes: its members stay in their own
/// files, so the archive cannot travel on its own.
const THIN_AR_MAGIC: &[u8] = b"!<thin>\n";

/// Why an archive is being checked, which decides how a refusal opens
/// and what it suggests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArchiveUse {
    /// A static build links it (the static-libpython decision entry).
    Link,
    /// `pycc lock` identifies an interpreter configured without a shared
    /// libpython (`described`, its configuration values) by its digest.
    Identify { described: String },
}

impl ArchiveUse {
    /// The opening of a refusal for the interpreter `name`.
    fn wanted(&self, name: &str) -> String {
        match self {
            Self::Link => format!(
                "a static libpython was requested ({STATIC_REQUEST}), which links the embed \
                 interpreter `{name}`'s `LIBPL` archive into the executable"
            ),
            Self::Identify { described } => format!(
                "the embed interpreter `{name}` has no shared libpython ({described}), so \
                 `pycc.lock` identifies it by the digest of its `LIBPL` archive"
            ),
        }
    }

    /// What a missing archive's refusal suggests.
    fn remedy(&self) -> &'static str {
        match self {
            Self::Link => "or drop the request",
            Self::Identify { .. } => "or one built with a shared libpython",
        }
    }
}

/// Checks that `archive` (the interpreter `name`'s `LIBPL/LIBRARY`, used
/// as `usage` says) is a regular file, after resolving symlinks, whose
/// first bytes are an ar archive's, and returns its resolved path. A path
/// that exists is not enough: several installers ship
/// `LIBPL/libpython3.14.a` as a symlink to the shared library. The message
/// for a wrong magic never guesses what the file is, because a universal
/// (fat) archive and a universal dylib share their `0xcafebabe` header; a
/// fat archive is refused too.
pub(crate) fn check_archive(
    name: &str,
    archive: &Path,
    usage: &ArchiveUse,
) -> Result<PathBuf, String> {
    let wanted = usage.wanted(name);
    let resolved = std::fs::canonicalize(archive).map_err(|e| {
        format!(
            "{wanted}, but `{}` does not exist ({e}); use a CPython 3.14 whose `LIBPL` holds \
             its static library, {}",
            archive.display(),
            usage.remedy()
        )
    })?;
    if !resolved.is_file() {
        return Err(format!(
            "{wanted}, but `{}` is not a regular file",
            resolved.display()
        ));
    }
    let head = read_head(&resolved)?;
    if head == AR_MAGIC {
        return Ok(resolved);
    }
    if head == THIN_AR_MAGIC {
        return Err(format!(
            "{wanted}, but `{}` is a thin archive, whose members live in other files; pycc \
             links only a regular archive",
            resolved.display()
        ));
    }
    let hex: Vec<String> = head.iter().map(|byte| format!("{byte:02x}")).collect();
    let fat = if head.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        "; a universal (fat) file cannot be linked statically"
    } else {
        ""
    };
    Err(format!(
        "{wanted}, but `{}` is not an ar archive (its first bytes are `{}`){fat}",
        resolved.display(),
        hex.join(" ")
    ))
}

/// The file whose sha256 is `pycc.lock`'s `libpython-sha256` for the
/// interpreter `probe` (Part 2 of #1227, #1272). On a Windows `platform`
/// (#1296) it is always the interpreter's DLL
/// ([`layout::windows_interpreter_dll`], `python314.dll`), whatever
/// `Py_ENABLE_SHARED` reports, and the archive is never asked for.
/// Elsewhere it is its shared library when
/// it is configured with one ([`super::check_shared`]), otherwise its
/// `LIBPL` archive, which `archive` finds and checks. The arm follows the
/// configuration, never which files exist, so `pycc lock` and either kind
/// of build pick the same one; a configured shared library that is missing
/// is refused, not replaced by the archive. The file identifies the
/// interpreter whichever way a build links it, so one lock serves a shared
/// and a static build alike. That refusal names the interpreter by the
/// executable its probe reports, since a build that populates the sidecar
/// holds only the probe, not the `PYCC_PYTHON` spelling the shared probe's
/// own refusal names.
pub(crate) fn identity_library(
    probe: &EmbedProbe,
    platform: EmbedPlatform,
    archive: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let windows = platform == EmbedPlatform::Windows;
    if !windows && !super::check_shared(probe.enable_shared, &probe.framework) {
        return archive();
    }
    let library = if windows {
        layout::windows_interpreter_dll(probe)
    } else {
        layout::source_library(probe)
    };
    if library.is_file() {
        return Ok(library);
    }
    Err(format!(
        "the embed interpreter `{}` reports a shared library `{}` that does not exist ({}); \
         `pycc.lock` identifies the interpreter by that library's digest",
        probe.executable.display(),
        library.display(),
        probe.describe()
    ))
}

/// Whether the dependency `dep` names a shared libpython of the `version`'s
/// major line: a file name that starts with `libpython<major>.` (any minor
/// and suffix, so `.so.1.0`, `.dylib`, a debug `d` build, another minor's
/// library and Linux's stable-ABI shim `libpython3.so`, which itself needs
/// the full library, all count), or a framework's `Python` binary. By
/// name, not by the shared build's bundled name: a static-only interpreter
/// may have no shared library to compare with.
pub(crate) fn names_libpython(dep: &str, version: (u32, u32, u32)) -> bool {
    let file = dep.rsplit('/').next().unwrap_or(dep);
    let stem = format!("libpython{}.", version.0);
    file.starts_with(&stem) || (file == "Python" && dep.contains("Python.framework/"))
}

/// The refusal of a bundled image (`image`, as a refusal describes it)
/// whose dependency `dep` is a shared libpython, in a static build.
pub(crate) fn second_libpython(image: &str, dep: &str) -> String {
    format!(
        "{image} depends on `{dep}`, a shared libpython; the executable links libpython \
         statically ({STATIC_REQUEST}), and loading a second one would start a second CPython \
         in the process; pycc cannot bundle it"
    )
}

#[cfg(test)]
#[path = "static_lib_tests.rs"]
mod tests;
