//! Pure layout rules for an embedded executable's sidecar (Part 1 of
//! #1028): where the sidecar lives, what it is called, how the executable
//! finds its library, what the marker says, and which standard-library
//! files are copied. The embedded-executable decision entry under
//! `docs/decisions/` owns the layout; this module only computes it.

use super::EmbedProbe;
use super::stdlib_roots::EXCLUDED_STDLIB_ROOTS;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The object format an embedded build links for. Injected rather than
/// read from `cfg!` so the macOS coverage host also drives the Linux arm's
/// lines (the `ExtLinkPlatform` precedent). Windows embeds through a stub
/// `OUT` that loads a program DLL from `OUT.pycc\` (D-253), with a
/// locked pure-Python closure (#1296).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmbedPlatform {
    MacOs,
    Linux,
    Windows,
}

impl EmbedPlatform {
    /// The platform of a host whose `std::env::consts::OS` is `os`, among
    /// the hosts an embedded build or `pycc lock` runs on. Any other host
    /// folds into Linux here.
    pub(crate) fn for_os(os: &str) -> Self {
        match os {
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            _ => Self::Linux,
        }
    }

    /// The build host's own platform.
    pub(crate) const HOST: Self = if cfg!(target_os = "macos") {
        Self::MacOs
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };
}

/// The first line of every `PYCC-BUNDLE` marker this build writes. A
/// sidecar whose marker starts with anything else is treated as not
/// pycc's, so it is never deleted (risk R6 of the #1028 plan).
pub(crate) const MARKER_HEADER: &str = "pycc-bundle 1";

/// The marker's file name inside the sidecar.
pub(crate) const MARKER_NAME: &str = "PYCC-BUNDLE";

/// The sidecar directory's file name for the executable `out`: `out`'s own
/// file name plus `.pycc`.
///
/// Refuses a name the loader cannot carry in an rpath: on Linux `ld.so`
/// expands `$` tokens inside `DT_RUNPATH` and splits it on `:`, and `:` is
/// unsafe in the macOS form too. No escaping fixes either, so the build
/// stops instead. A name that is not UTF-8 is refused because it is also
/// baked into the launcher as a C string literal.
pub(crate) fn sidecar_name(out: &Path) -> Result<String, String> {
    let name = out
        .file_name()
        .ok_or_else(|| format!("output path `{}` names no file", out.display()))?
        .to_str()
        .ok_or_else(|| {
            format!(
                "output file name `{}` is not UTF-8; an embedded executable records it \
                 in its library search path",
                out.display()
            )
        })?;
    if name.contains(['$', ':']) {
        return Err(format!(
            "output file name `{name}` contains `$` or `:`, which an embedded executable \
             cannot carry in its library search path; choose another `-o` name"
        ));
    }
    Ok(format!("{name}.pycc"))
}

/// The directory `out` is written into. `-o app` has an empty parent,
/// which means the current directory.
pub(crate) fn sidecar_parent(out: &Path) -> &Path {
    match out.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// The linker arguments that make the executable find `<sidecar>/lib`
/// relative to itself. `-Xlinker` rather than `-Wl,`, which splits on
/// commas; `$ORIGIN` is a literal argument because no shell is involved.
/// Empty on Windows, which has no rpath: the stub loads the program DLL
/// by its full path instead (D-253), and the launcher adds the sidecar and
/// its `natives\` as DLL directories (#1305, #1306).
pub(crate) fn rpath_args(platform: EmbedPlatform, sidecar: &str) -> Vec<OsString> {
    let origin = match platform {
        EmbedPlatform::MacOs => "@executable_path",
        EmbedPlatform::Linux => "$ORIGIN",
        EmbedPlatform::Windows => return Vec::new(),
    };
    ["-Xlinker", "-rpath", "-Xlinker"]
        .into_iter()
        .map(OsString::from)
        .chain([OsString::from(format!("{origin}/{sidecar}/lib"))])
        .collect()
}

/// Linux only: links every library the bundle copies into `lib/` besides
/// libpython (`names`, from `dir`) into the executable as `DT_NEEDED`
/// entries, so the loader has them loaded, by their `DT_SONAME`, before an
/// extension module that needs one is opened (#1243). `--no-as-needed`
/// keeps an entry nothing in the executable itself references, and
/// `-rpath-link` lets the link resolve their own dependencies among them.
/// Empty on macOS, which rewrites the images' install names instead, and
/// on Windows, which has no `lib\`: its natives go in `natives\`
/// ([`WINDOWS_NATIVES_DIR`]), which the launcher adds as a DLL directory
/// (#1306).
pub(crate) fn preload_args(platform: EmbedPlatform, dir: &Path, names: &[String]) -> Vec<OsString> {
    if platform != EmbedPlatform::Linux || names.is_empty() {
        return Vec::new();
    }
    let mut args: Vec<OsString> = ["-Xlinker", "--push-state", "-Xlinker", "--no-as-needed"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.push(OsString::from("-L"));
    args.push(dir.as_os_str().to_os_string());
    args.extend(
        names
            .iter()
            .map(|name| OsString::from(format!("-l:{name}"))),
    );
    let tail = [
        "-Xlinker",
        "--pop-state",
        "-Xlinker",
        "-rpath-link",
        "-Xlinker",
    ];
    args.extend(tail.into_iter().map(OsString::from));
    args.push(dir.as_os_str().to_os_string());
    args
}

/// The interpreter's shared library as the probe describes it: on a
/// framework build the framework binary beside `LIBDIR`
/// (`.../Versions/3.14/Python`), otherwise `LIBDIR/LDLIBRARY` with symlinks
/// resolved. An unresolvable path is returned as is, for the caller's
/// existence check to report.
///
/// This is the macOS and Linux interpretation of the probe: on Windows
/// `LIBDIR` is the import-library directory, and the DLL is
/// [`windows_interpreter_dll`] instead. No Windows build reaches this.
pub(crate) fn source_library(probe: &EmbedProbe) -> PathBuf {
    if !probe.framework.is_empty() {
        let root = probe.libdir.parent().unwrap_or(&probe.libdir);
        return root.join(&probe.framework);
    }
    let path = probe.libdir.join(&probe.ldlibrary);
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// The interpreter's DLL on a Windows host: `<base_prefix>\<LDLIBRARY>`,
/// beside `python.exe` (`python314.dll`). The Windows interpretation of
/// the probe, which [`source_library`] is not.
pub(crate) fn windows_interpreter_dll(probe: &EmbedProbe) -> PathBuf {
    probe.base_prefix.join(&probe.ldlibrary)
}

/// The program DLL's file name in a Windows sidecar: it holds the
/// launcher, the shim, the compiled module and `pycc_rt`, and the stub
/// `OUT` loads it by this fixed name (D-253). The stub's C source spells
/// the same name.
pub(crate) const PROGRAM_DLL_NAME: &str = "pycc_program.dll";

/// The directory of a Windows sidecar that holds the closure's natives
/// (#1306), created only when the lock records one. Not `lib`: that is
/// the standard library's `Lib` on a case-insensitive volume.
pub(crate) const WINDOWS_NATIVES_DIR: &str = "natives";

/// The Visual C++ runtime DLLs a Windows sidecar carries when the
/// interpreter's `base_prefix` has them, each copied only if it exists
/// (D-253). `python3.dll` is not listed: the probe requires it.
pub(crate) fn windows_runtime_dlls() -> [&'static str; 2] {
    ["vcruntime140.dll", "vcruntime140_1.dll"]
}

/// The DLLs at a Windows sidecar's root, in copy order: the interpreter's
/// DLL (`probe.ldlibrary`), `python{major}.dll`, and each of
/// [`windows_runtime_dlls`] that exists in `base_prefix`. The copy and the
/// import scan both read this list, so the scan's root set is exactly what
/// the sidecar carries.
pub(crate) fn windows_root_dlls(probe: &EmbedProbe) -> Vec<String> {
    let mut names = vec![
        probe.ldlibrary.clone(),
        format!("python{}.dll", probe.version.0),
    ];
    let present = windows_runtime_dlls()
        .into_iter()
        .filter(|name| probe.base_prefix.join(name).is_file());
    names.extend(present.map(str::to_string));
    names
}

/// The library's file name inside `<sidecar>/lib`. macOS renames it to a
/// plain dylib whose id the build rewrites to `@rpath/<name>`; Linux keeps
/// the SONAME the executable will record as `DT_NEEDED`. Windows keeps the
/// DLL's own name, in the sidecar root ([`bundled_library_path`]); its
/// natives go in `natives\` ([`WINDOWS_NATIVES_DIR`]), never `lib\`.
pub(crate) fn bundled_library_name(platform: EmbedPlatform, probe: &EmbedProbe) -> String {
    match platform {
        EmbedPlatform::MacOs => {
            format!("libpython{}.{}.dylib", probe.version.0, probe.version.1)
        }
        EmbedPlatform::Linux if probe.instsoname.is_empty() => probe.ldlibrary.clone(),
        EmbedPlatform::Linux => probe.instsoname.clone(),
        EmbedPlatform::Windows => probe.ldlibrary.clone(),
    }
}

/// The bundled library's final path in `sidecar`: `<sidecar>\python314.dll`
/// on Windows, which has no `lib\`, and `<sidecar>/lib/<name>` elsewhere.
pub(crate) fn bundled_library_path(
    platform: EmbedPlatform,
    sidecar: &Path,
    probe: &EmbedProbe,
) -> PathBuf {
    let name = bundled_library_name(platform, probe);
    match platform {
        EmbedPlatform::Windows => sidecar.join(name),
        EmbedPlatform::MacOs | EmbedPlatform::Linux => sidecar.join("lib").join(name),
    }
}

/// The position-independence flag the C sources compile with: `-fPIC` for
/// an ELF or Mach-O image, nothing on Windows, where a PE/COFF image is
/// position-independent by construction (the same fact
/// `ext_build::ext_compile_args` encodes for `--ext`).
pub(crate) fn pic_args(platform: EmbedPlatform) -> Vec<OsString> {
    match platform {
        EmbedPlatform::MacOs | EmbedPlatform::Linux => vec![OsString::from("-fPIC")],
        EmbedPlatform::Windows => Vec::new(),
    }
}

/// The standard library's directory name under `<sidecar>/lib`, which is
/// where CPython looks for it once `home` is the sidecar.
pub(crate) fn stdlib_dir_name(probe: &EmbedProbe) -> String {
    format!("python{}.{}", probe.version.0, probe.version.1)
}

/// The `PYCC-BUNDLE` marker's text. A static build's `library_sha256` is
/// the archive's digest, and one line naming the mode follows it (D-251);
/// a shared build's text is the one D-248 defines, unchanged.
pub(crate) fn marker_text(probe: &EmbedProbe, library_sha256: &str, link: LibpythonLink) -> String {
    let (major, minor, micro) = probe.version;
    let mode = match link {
        LibpythonLink::Shared => "",
        LibpythonLink::Static => "libpython-link static\n",
    };
    format!(
        "{MARKER_HEADER}\npython {major}.{minor}.{micro}\nexecutable {}\nlibpython-sha256 {library_sha256}\n{mode}",
        probe.executable.display()
    )
}

/// How an embedded executable links libpython (D-251): against the bundled
/// shared library (D-248, the default), or statically, from the
/// interpreter's `LIBPL` archive, with nothing bundled in its place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LibpythonLink {
    #[default]
    Shared,
    Static,
}

/// The linker arguments that pull every member of the static `archive`
/// into the executable, so an extension module's C-API reference resolves
/// even to a function the executable itself never calls. Empty on
/// Windows, which never links statically: `plan_embed` refuses a static
/// libpython there first (D-253).
pub(crate) fn archive_load_args(platform: EmbedPlatform, archive: &Path) -> Vec<OsString> {
    let archive = archive.as_os_str().to_os_string();
    match platform {
        EmbedPlatform::MacOs => vec![
            "-Xlinker".into(),
            "-force_load".into(),
            "-Xlinker".into(),
            archive,
        ],
        EmbedPlatform::Linux => vec![
            "-Xlinker".into(),
            "--whole-archive".into(),
            archive,
            "-Xlinker".into(),
            "--no-whole-archive".into(),
        ],
        EmbedPlatform::Windows => Vec::new(),
    }
}

/// The linker arguments that export the executable's global symbols to
/// the modules it opens: on Linux an executable exports none by default,
/// and on macOS `-export_dynamic` keeps them through dead stripping.
/// Empty on Windows, which never links statically (D-253).
pub(crate) fn export_args(platform: EmbedPlatform) -> Vec<OsString> {
    let flag = match platform {
        EmbedPlatform::MacOs => "-export_dynamic",
        EmbedPlatform::Linux => "--export-dynamic",
        EmbedPlatform::Windows => return Vec::new(),
    };
    vec!["-Xlinker".into(), flag.into()]
}

/// A static build's link arguments in place of the bundled library: the
/// whole archive, the export flag, then the interpreter's own `LIBS` and
/// `SYSLIBS` tokens (`libs`), which the archive's members need (D-251).
pub(crate) fn static_link_args(
    platform: EmbedPlatform,
    archive: &Path,
    libs: &[String],
) -> Vec<OsString> {
    let mut args = archive_load_args(platform, archive);
    args.extend(export_args(platform));
    args.extend(libs.iter().map(OsString::from));
    args
}

/// Whether `text` is a marker this build knows how to replace.
pub(crate) fn marker_is_current(text: &str) -> bool {
    text.lines().next() == Some(MARKER_HEADER)
}

/// Whether the standard-library copy skips `rel` (a path relative to the
/// interpreter's stdlib directory): `site-packages`, every `__pycache__`,
/// the `test` package, and each excluded root as a package, as a top-level
/// module, and as a `lib-dynload` extension.
pub(crate) fn skip_in_stdlib_copy(rel: &Path) -> bool {
    let parts: Vec<&str> = rel
        .components()
        .map(|part| part.as_os_str().to_str().unwrap_or(""))
        .collect();
    if parts.contains(&"__pycache__") {
        return true;
    }
    let Some(first) = parts.first() else {
        return false;
    };
    if matches!(*first, "site-packages" | "test") {
        return true;
    }
    EXCLUDED_STDLIB_ROOTS.iter().any(|root| {
        *first == *root
            || *first == format!("{root}.py")
            || (*first == "lib-dynload"
                && parts
                    .get(1)
                    .is_some_and(|file| file.starts_with(&format!("{root}."))))
    })
}

/// The Windows twin of [`skip_in_stdlib_copy`], applied to both copies a
/// Windows sidecar makes (`Lib` and `DLLs`), with `rel` relative to the
/// copy's root: every `__pycache__`, a first component of `site-packages`
/// or `test`, each excluded root as a package, as `<root>.py` or as a
/// first component starting with `<root>.` (`_tkinter.pyd`), the
/// Tcl/Tk DLLs (`tcl86t.dll`, `tk86t.dll`), and the extension images of
/// another ABI ([`is_other_abi_image`]). Case-insensitive, as NTFS is.
pub(crate) fn skip_in_windows_stdlib_copy(rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    if parts.iter().any(|part| part == "__pycache__") {
        return true;
    }
    if parts.last().is_some_and(|name| is_other_abi_image(name)) {
        return true;
    }
    let Some(first) = parts.first() else {
        return false;
    };
    if matches!(first.as_str(), "site-packages" | "test") || is_tcl_tk_dll(first) {
        return true;
    }
    EXCLUDED_STDLIB_ROOTS
        .iter()
        .any(|root| first == root || first.starts_with(&format!("{root}.")))
}

/// Whether the lowercased file name `name` is an extension image the
/// release, non-free-threaded interpreter pycc embeds never imports (#1305):
/// a free-threaded one (`.cp3NNt-` in the name, importing `python3NNt.dll`)
/// or a debug one (`*_d.pyd`, `*_d.dll`, importing `python3NN_d.dll`). The
/// python.org installer's optional components put both in the same `DLLs\`.
fn is_other_abi_image(name: &str) -> bool {
    let free_threaded = name.match_indices(".cp3").any(|(at, tag)| {
        let rest = &name[at + tag.len()..];
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        digits > 0 && rest[digits..].starts_with("t-")
    });
    free_threaded || name.ends_with("_d.pyd") || name.ends_with("_d.dll")
}

/// Whether the lowercased file name `name` is a Tcl/Tk DLL:
/// `(tcl|tk)<digit>...dll`.
fn is_tcl_tk_dll(name: &str) -> bool {
    let rest = name.strip_prefix("tcl").or_else(|| name.strip_prefix("tk"));
    rest.is_some_and(|rest| {
        rest.starts_with(|c: char| c.is_ascii_digit()) && rest.ends_with(".dll")
    })
}

/// `text` as a C string literal. Every byte outside printable ASCII, and
/// the quote and backslash, is written as a three-digit octal escape, which
/// (unlike `\x`) cannot run into a following hex digit.
pub(crate) fn c_string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        if byte.is_ascii_graphic() && byte != b'"' && byte != b'\\' || byte == b' ' {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("\\{byte:03o}"));
        }
    }
    out.push('"');
    out
}

/// The generated `pycc_embed_config.inc` the launcher includes. With
/// `has_closure` it also defines `PYCC_EMBED_CLOSURE`, which makes the
/// launcher append `<sidecar>/closure` to `sys.path` (#1242); with
/// `has_natives` (a Windows build whose closure needs natives, #1306) it
/// defines `PYCC_EMBED_NATIVES`, which makes the launcher add
/// `<sidecar>\natives` ([`WINDOWS_NATIVES_DIR`]) as a DLL directory. A
/// build with no locked package emits exactly the pre-#1242 text.
pub(crate) fn embed_config_inc(sidecar: &str, has_closure: bool, has_natives: bool) -> String {
    let closure = if has_closure {
        "#define PYCC_EMBED_CLOSURE 1\n"
    } else {
        ""
    };
    let natives = if has_natives {
        "#define PYCC_EMBED_NATIVES 1\n"
    } else {
        ""
    };
    format!(
        "/* Generated by pycc for an embedded executable. Do not edit: see src/embed/layout.rs. */\n\
         #define PYCC_EMBED_SIDECAR {}\n{closure}{natives}",
        c_string_literal(sidecar)
    )
}

/// The `@loader_path` reference from a closure image at `rel` (relative to
/// `<sidecar>/closure`, `/`-separated) to a vendored library `name` in
/// `<sidecar>/lib`: one `../` per component of `rel`, which climbs from the
/// image's directory to the sidecar root.
pub(crate) fn closure_loader_relative(rel: &str, name: &str) -> String {
    let mut path = String::from("@loader_path/");
    for _ in rel.split('/') {
        path.push_str("../");
    }
    path.push_str("lib/");
    path.push_str(name);
    path
}

/// The `@loader_path` reference from an image at `image` (relative to
/// `<sidecar>/lib`) to a vendored library `name` in `<sidecar>/lib`.
pub(crate) fn loader_relative(image: &Path, name: &str) -> String {
    let depth = image.components().count().saturating_sub(1);
    let mut path = String::from("@loader_path/");
    for _ in 0..depth {
        path.push_str("../");
    }
    path.push_str(name);
    path
}

/// The `@loader_path` reference from the image at `from` to the file at
/// `to`, both relative to the sidecar root and `/`-separated (#1259): one
/// `../` per directory of `from` that `to` does not share, then the rest of
/// `to`. It serves a closure image rebound to another payload file and a
/// native in `lib/` rebound to one in `closure/`.
pub(crate) fn sidecar_loader_relative(from: &str, to: &str) -> String {
    let from_dirs: Vec<&str> = from.split('/').collect();
    let from_dirs = &from_dirs[..from_dirs.len() - 1];
    let to_parts: Vec<&str> = to.split('/').collect();
    let shared = from_dirs
        .iter()
        .zip(&to_parts[..to_parts.len() - 1])
        .take_while(|(a, b)| a == b)
        .count();
    let mut path = String::from("@loader_path/");
    for _ in shared..from_dirs.len() {
        path.push_str("../");
    }
    path.push_str(&to_parts[shared..].join("/"));
    path
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
