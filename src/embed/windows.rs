//! The Windows embedded executable (D-253, Part 1 of #1226): a stub `OUT`
//! that loads a program DLL from `OUT.pycc\`, with a locked pure-Python
//! closure in `OUT.pycc\closure\` (#1296), whose PE images are scanned
//! and whose natives are copied into `OUT.pycc\natives\`
//! (`native_windows_closure.rs`, #1306). Pure functions, compiled and
//! unit-tested on every host; only the stub's link spawn in
//! `src/build_pipeline.rs` is `cfg(windows)`.

use super::layout::{self, EmbedPlatform, LibpythonLink};
use super::{EmbedProbe, write_source};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The stub's C source: it loads `<exe dir>\<sidecar>\pycc_program.dll`
/// and calls its `pycc_embed_main`. It holds no Python code (D-253).
pub(crate) const STUB_C: &str = include_str!("pycc_embed_stub_windows.c");

/// The stub's file name in the build's scratch directory.
pub(crate) const STUB_C_NAME: &str = "pycc_embed_stub_windows.c";

/// How the stub `OUT` is compiled and linked, after the program DLL.
#[derive(Debug)]
#[cfg_attr(not(any(windows, test)), expect(dead_code))]
pub(crate) struct StubLink {
    /// The static-CRT flag, the stub source, and `-I` naming the scratch
    /// directory that holds `pycc_embed_config.inc`.
    pub(crate) compile_args: Vec<OsString>,
    /// The executable the user asked for, `OUT`.
    pub(crate) output: PathBuf,
}

/// Refuses what a Windows embedded build cannot do, before the
/// interpreter is probed: a static libpython (CPython for Windows ships no
/// static library, D-251). `Ok` on every other platform.
pub(crate) fn check_windows_request(
    platform: EmbedPlatform,
    link: LibpythonLink,
) -> Result<(), String> {
    if platform == EmbedPlatform::Windows && link == LibpythonLink::Static {
        return Err(STATIC_REFUSAL.to_string());
    }
    Ok(())
}

/// Why a Windows embedded build refuses a static libpython (D-251).
pub(crate) const STATIC_REFUSAL: &str = "a static libpython is not available for a Windows \
     embed interpreter: CPython for Windows ships no static library (D-251, D-253)";

/// Whether the closure file `rel`, whose first bytes are `head`, is a PE
/// image a Windows build would load (#1296): its name ends in `.pyd` or
/// `.dll`, which the import system and `LoadLibrary` load by name, or it
/// starts with the PE `MZ` magic, which `ctypes` loads under any name,
/// unless its name ends in `.exe` (a launcher Python never loads). Every
/// suffix is compared ASCII case-insensitively. Pure.
pub(crate) fn is_windows_image(rel: &str, head: &[u8]) -> bool {
    let lower = rel.to_ascii_lowercase();
    if lower.ends_with(".pyd") || lower.ends_with(".dll") {
        return true;
    }
    head.starts_with(b"MZ") && !lower.ends_with(".exe")
}

/// The Windows half of the embed probe (D-253): the import libraries in
/// `LIBDIR` (`libs\python314.lib`, and `libs\python3.lib`, which the
/// shim's limited-API `pyconfig.h` pragma names), then the interpreter's
/// DLL, `python3.dll` and `DLLs\` beside `python.exe`. Windows CPython
/// always ships a DLL, so there is no shared-library check.
pub(crate) fn check_windows_layout(probe: &EmbedProbe, name: &str) -> Result<(), String> {
    let (major, minor, _) = probe.version;
    for lib in [
        format!("python{major}{minor}.lib"),
        format!("python{major}.lib"),
    ] {
        let path = probe.libdir.join(lib);
        if !path.is_file() {
            return Err(format!(
                "the embed interpreter `{name}` has no import library `{}`; an embedded \
                 Windows build links its program DLL against it -- install CPython with the \
                 full python.org installer, which ships `libs\\`",
                path.display()
            ));
        }
    }
    let dlls = [
        layout::windows_interpreter_dll(probe),
        probe.base_prefix.join(format!("python{major}.dll")),
    ];
    for dll in dlls {
        if !dll.is_file() {
            return Err(format!(
                "the embed interpreter `{name}` has no DLL `{}` ({}); an embedded Windows \
                 build bundles it",
                dll.display(),
                probe.describe()
            ));
        }
    }
    let extensions = probe.base_prefix.join("DLLs");
    if !extensions.is_dir() {
        return Err(format!(
            "the embed interpreter `{name}` has no extension-module directory `{}`; an \
             embedded Windows build bundles it",
            extensions.display()
        ));
    }
    Ok(())
}

/// The program DLL's link arguments after `pycc_rt`: a DLL, linked against
/// the interpreter's import library, with no import library of its own.
pub(crate) fn program_link_args(probe: &EmbedProbe) -> Vec<OsString> {
    let (major, minor, _) = probe.version;
    vec![
        OsString::from("-shared"),
        OsString::from("-L"),
        probe.libdir.clone().into_os_string(),
        OsString::from(format!("-lpython{major}{minor}")),
        OsString::from("-Wl,/NOIMPLIB"),
    ]
}

/// Writes the stub's C source beside `obj_path` and returns how the stub
/// `out` is compiled from it.
pub(crate) fn stub_link(obj_path: &Path, out: &Path) -> Result<StubLink, String> {
    let source = obj_path.with_file_name(STUB_C_NAME);
    write_source(&source, STUB_C)?;
    let scratch = source.parent().unwrap_or(Path::new("."));
    let compile_args = vec![
        OsString::from("-fms-runtime-lib=static"),
        source.clone().into_os_string(),
        OsString::from("-I"),
        scratch.as_os_str().to_os_string(),
    ];
    Ok(StubLink {
        compile_args,
        output: out.to_path_buf(),
    })
}

/// The stub's arguments to the linker driver, after `-target`: its compile
/// arguments, then `-o OUT`.
#[cfg_attr(not(any(windows, test)), expect(dead_code))]
pub(crate) fn stub_link_args(stub: &StubLink) -> Vec<OsString> {
    let mut args = stub.compile_args.clone();
    args.push(OsString::from("-o"));
    args.push(stub.output.clone().into_os_string());
    args
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
