//! The static probe's parser, the archive checks and the refusal texts of
//! the static-libpython variant (D-251), plus a negative-control link that
//! proves the whole-archive and export flags are each load-bearing.

use super::*;
use pycc_scratch::ScratchDir;

#[test]
fn the_static_probe_output_parses_into_the_archive_and_the_libs() {
    let probe = parse_static_probe("/opt/py/lib/config\nlibpython3.14.a\n-ldl  -lpthread\n-lm\n")
        .expect("four lines");
    assert_eq!(
        probe,
        StaticProbe {
            archive: PathBuf::from("/opt/py/lib/config/libpython3.14.a"),
            libs: vec!["-ldl".into(), "-lpthread".into(), "-lm".into()],
        }
    );
    // An unset `LIBS`/`SYSLIBS` prints an empty line: no tokens.
    let bare = parse_static_probe("/p\nlibpython3.14.a\n\n\n").expect("four lines");
    assert!(bare.libs.is_empty());
    assert_eq!(parse_static_probe("/p\nlibpython3.14.a\n-ldl\n"), None);
    assert_eq!(parse_static_probe(""), None);
    assert!(STATIC_PROBE_SCRIPT.starts_with("# pycc-static-probe\n"));
    assert!(!STATIC_PROBE_SCRIPT.contains("MODLIBS"));
}

/// The refusal message of `check_archive` for `archive`, which must fail.
fn refusal(archive: &Path) -> String {
    let err = check_archive("pyfake", archive, &ArchiveUse::Link).expect_err("refused");
    assert!(
        err.starts_with(
            "a static libpython was requested (`--static-libpython` or `[build] static = true` \
             in `pycc.toml`), which links the embed interpreter `pyfake`'s `LIBPL` archive into \
             the executable, but `"
        ),
        "{err}"
    );
    err
}

#[test]
fn only_a_regular_ar_archive_passes_the_archive_check() {
    let dir = ScratchDir::new("static_lib_check").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let archive = root.join("libpython3.14.a");
    std::fs::write(&archive, b"!<arch>\nmembers").expect("write");
    assert_eq!(
        check_archive("pyfake", &archive, &ArchiveUse::Link),
        Ok(archive.clone())
    );

    let missing = refusal(&root.join("missing.a"));
    assert!(missing.contains("missing.a` does not exist ("), "{missing}");
    assert!(missing.ends_with("or drop the request"), "{missing}");

    let directory = root.join("dir.a");
    std::fs::create_dir(&directory).expect("mkdir");
    assert!(refusal(&directory).ends_with("dir.a` is not a regular file"));

    let thin = root.join("thin.a");
    std::fs::write(&thin, b"!<thin>\nmembers").expect("write");
    assert!(refusal(&thin).ends_with(
        "thin.a` is a thin archive, whose members live in other files; pycc links only a \
         regular archive"
    ));

    let empty = root.join("empty.a");
    std::fs::write(&empty, b"").expect("write");
    assert!(refusal(&empty).ends_with("empty.a` is not an ar archive (its first bytes are ``)"));

    let fat = root.join("fat.a");
    std::fs::write(&fat, [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2, 9]).expect("write");
    assert!(refusal(&fat).ends_with(
        "fat.a` is not an ar archive (its first bytes are `ca fe ba be 00 00 00 02`); a \
         universal (fat) file cannot be linked statically"
    ));
}

/// `pycc lock` checks the archive of an interpreter without a shared
/// libpython too, and says why it wants one without mentioning a request.
#[test]
fn the_identity_check_words_its_refusals_for_pycc_lock() {
    let dir = ScratchDir::new("static_lib_identify").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let usage = ArchiveUse::Identify {
        described: "Py_ENABLE_SHARED=0".to_string(),
    };
    let opening = "the embed interpreter `pyfake` has no shared libpython (Py_ENABLE_SHARED=0), \
                   so `pycc.lock` identifies it by the digest of its `LIBPL` archive, but `";
    let missing = check_archive("pyfake", &root.join("missing.a"), &usage).expect_err("missing");
    assert!(missing.starts_with(opening), "{missing}");
    assert!(
        missing.ends_with(
            "; use a CPython 3.14 whose `LIBPL` holds its static library, or one built with a \
             shared libpython"
        ),
        "{missing}"
    );
    let text = root.join("text.a");
    std::fs::write(&text, b"not ar").expect("write");
    let wrong = check_archive("pyfake", &text, &usage).expect_err("not ar");
    assert!(wrong.starts_with(opening), "{wrong}");
    assert!(!wrong.contains("requested"), "{wrong}");
}

/// The file `pycc.lock` digests follows the interpreter's configuration:
/// the shared library when it has one (the archive is never asked for),
/// the archive otherwise, and a missing configured library is refused
/// rather than replaced by the archive.
#[test]
fn the_identity_library_follows_the_configuration() {
    let dir = ScratchDir::new("static_lib_identity").expect("scratch");
    let layout = super::super::fake_layout::fake_layout(&dir);
    let unused = || -> Result<PathBuf, String> { panic!("the archive is not asked for") };
    assert_eq!(
        identity_library(&layout.probe, EmbedPlatform::MacOs, unused),
        Ok(layout.library())
    );
    let mut framework = layout.probe.clone();
    framework.enable_shared = false;
    framework.framework = "Python".to_string();
    let framework_binary = layout::source_library(&framework);
    std::fs::write(&framework_binary, "framework").expect("write");
    assert_eq!(
        identity_library(&framework, EmbedPlatform::MacOs, unused),
        Ok(framework_binary)
    );

    let archive = || Ok(PathBuf::from("/lib/config/libpython3.14.a"));
    let mut static_only = layout.probe.clone();
    static_only.enable_shared = false;
    assert_eq!(
        identity_library(&static_only, EmbedPlatform::MacOs, archive),
        Ok(PathBuf::from("/lib/config/libpython3.14.a"))
    );
    let failing = || Err("no archive".to_string());
    assert_eq!(
        identity_library(&static_only, EmbedPlatform::MacOs, failing),
        Err("no archive".to_string())
    );

    std::fs::remove_file(layout.library()).expect("remove the library");
    let err = identity_library(&layout.probe, EmbedPlatform::MacOs, archive)
        .expect_err("missing library");
    assert_eq!(
        err,
        format!(
            "the embed interpreter `{}` reports a shared library `{}` that does not exist ({}); \
             `pycc.lock` identifies the interpreter by that library's digest",
            layout.probe.executable.display(),
            layout.library().display(),
            layout.probe.describe()
        )
    );
}

/// On Windows the identity is always `python314.dll` beside `python.exe`
/// (#1296): the interpreter reports no `Py_ENABLE_SHARED`, yet the static
/// probe never runs, and a missing DLL is refused.
#[test]
fn the_windows_identity_library_is_the_interpreter_dll() {
    let dir = ScratchDir::new("static_lib_identity_windows").expect("scratch");
    let layout = super::super::fake_layout::fake_windows_layout(&dir);
    let windows = EmbedPlatform::Windows;
    let unused = || -> Result<PathBuf, String> { panic!("the static probe does not run") };
    let dll = layout.prefix.join("python314.dll");
    assert_eq!(
        identity_library(&layout.probe, windows, unused),
        Ok(dll.clone())
    );
    std::fs::remove_file(&dll).expect("remove the DLL");
    let err = identity_library(&layout.probe, windows, unused).expect_err("missing DLL");
    assert!(err.contains("reports a shared library"), "{err}");
    assert!(err.contains(&dll.display().to_string()), "{err}");
}

/// Several installers ship `LIBPL/libpython3.14.a` as a symlink to the
/// shared library: the link is resolved, and the target's bytes decide.
#[cfg(unix)]
#[test]
fn a_symlink_to_a_shared_library_is_refused_by_its_resolved_bytes() {
    let dir = ScratchDir::new("static_lib_symlink").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    let dylib = root.join("libpython3.14.dylib");
    std::fs::write(&dylib, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1, 0]).expect("write");
    let link = root.join("libpython3.14.a");
    std::os::unix::fs::symlink(&dylib, &link).expect("symlink");
    let err = refusal(&link);
    assert!(
        err.ends_with(&format!(
            "`{}` is not an ar archive (its first bytes are `cf fa ed fe 0c 00 00 01`)",
            dylib.display()
        )),
        "{err}"
    );
    let archive = root.join("real.a");
    std::fs::write(&archive, b"!<arch>\n").expect("write");
    let alias = root.join("alias.a");
    std::os::unix::fs::symlink(&archive, &alias).expect("symlink");
    assert_eq!(
        check_archive("pyfake", &alias, &ArchiveUse::Link),
        Ok(archive)
    );
}

#[test]
fn a_shared_libpython_is_recognized_by_its_name() {
    let version = (3, 14, 7);
    for dep in [
        "libpython3.14.so.1.0",
        "/usr/lib/x86_64-linux-gnu/libpython3.14.so",
        "@rpath/libpython3.14.dylib",
        "/opt/py/lib/libpython3.14d.dylib",
        "libpython3.13.so.1.0",
        "libpython3.so",
        "/usr/lib/libpython3.15.dylib",
        "/Library/Frameworks/Python.framework/Versions/3.14/Python",
        "@rpath/Python.framework/Versions/3.14/Python",
    ] {
        assert!(names_libpython(dep, version), "{dep}");
    }
    for dep in [
        "/usr/lib/libSystem.B.dylib",
        "libpythonfoo.so",
        "libpython.so",
        "libpython2.7.so.1.0",
        "/opt/Python",
        "libpythonx.so",
        "/opt/libs/libc.so.6",
    ] {
        assert!(!names_libpython(dep, version), "{dep}");
    }
}

#[test]
fn the_static_refusals_name_the_request() {
    assert_eq!(
        second_libpython("`_json.so`", "libpython3.14.so.1.0"),
        "`_json.so` depends on `libpython3.14.so.1.0`, a shared libpython; the executable \
         links libpython statically (`--static-libpython` or `[build] static = true` in \
         `pycc.toml`), and loading a second one would start a second CPython in the process; \
         pycc cannot bundle it"
    );
}

/// The negative control of the static link recipe (D-251): an executable
/// that calls one member of an archive and `dlopen`s a module needing a
/// function in another member resolves it only with the whole archive
/// loaded *and* its symbols exported. Each flag is dropped in turn and the
/// load must then fail, so neither is decorative.
#[cfg(unix)]
mod negative_control {
    use super::super::super::fake_layout::cc;
    use super::super::super::layout::{self, EmbedPlatform};
    use pycc_scratch::ScratchDir;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[cfg(target_os = "macos")]
    const HOST: EmbedPlatform = EmbedPlatform::MacOs;
    #[cfg(not(target_os = "macos"))]
    const HOST: EmbedPlatform = EmbedPlatform::Linux;

    /// How the module is built: it leaves the C-API-like symbol undefined,
    /// for the executable to provide.
    #[cfg(target_os = "macos")]
    const MODULE_ARGS: &[&str] = &["-bundle", "-undefined", "dynamic_lookup"];
    #[cfg(not(target_os = "macos"))]
    const MODULE_ARGS: &[&str] = &["-shared", "-fPIC"];

    /// What the executable links besides the archive flags.
    #[cfg(target_os = "macos")]
    const SYSTEM_LIBS: &[&str] = &[];
    #[cfg(not(target_os = "macos"))]
    const SYSTEM_LIBS: &[&str] = &["-ldl"];

    const MAIN_C: &str = "#include <dlfcn.h>\n#include <stdio.h>\n\
        int fx_used(void);\n\
        int main(int argc, char **argv) {\n\
          (void)argc;\n\
          if (fx_used() != 1) return 3;\n\
          void *handle = dlopen(argv[1], RTLD_NOW);\n\
          if (!handle) { fprintf(stderr, \"%s\\n\", dlerror()); return 1; }\n\
          int (*call)(void) = (int (*)(void))dlsym(handle, \"module_call\");\n\
          return call && call() == 2 ? 0 : 4;\n\
        }\n";

    fn write(dir: &Path, name: &str, text: &str) -> String {
        std::fs::write(dir.join(name), text).expect("write C");
        name.to_string()
    }

    /// Builds the archive (`fx_used` and `fx_unused` in separate members)
    /// and the module, and returns their paths.
    fn fixture(dir: &Path) -> (PathBuf, PathBuf) {
        for (name, body) in [("fx_used", "return 1;"), ("fx_unused", "return 2;")] {
            let c = write(
                dir,
                &format!("{name}.c"),
                &format!("int {name}(void) {{ {body} }}\n"),
            );
            cc(dir, &["-c", "-fPIC", &c, "-o", &format!("{name}.o")]);
        }
        let status = Command::new("ar")
            .current_dir(dir)
            .args(["rcs", "libfx.a", "fx_used.o", "fx_unused.o"])
            .status()
            .expect("spawn ar");
        assert!(status.success(), "ar failed");
        let module_c = write(
            dir,
            "module.c",
            "int fx_unused(void);\nint module_call(void) { return fx_unused(); }\n",
        );
        let mut args = MODULE_ARGS.to_vec();
        args.extend([module_c.as_str(), "-o", "module.so"]);
        cc(dir, &args);
        (dir.join("libfx.a"), dir.join("module.so"))
    }

    /// Links the executable with `link`, runs it on `module`, and returns
    /// whether the module loaded and its call resolved.
    fn loads(dir: &Path, tag: &str, link: &[OsString], module: &Path) -> bool {
        let exe = dir.join(format!("main_{tag}"));
        let output = Command::new("cc")
            .current_dir(dir)
            .arg("main.c")
            .arg("-o")
            .arg(&exe)
            .args(link)
            .args(SYSTEM_LIBS)
            .output()
            .expect("spawn cc");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "link {tag} failed: {stderr}");
        let run = Command::new(&exe).arg(module).output().expect("run");
        let code = run.status.code();
        assert!(
            matches!(code, Some(0 | 1)),
            "{tag}: unexpected exit {code:?}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        code == Some(0)
    }

    #[test]
    fn the_module_resolves_only_with_the_whole_archive_and_the_export() {
        let dir = ScratchDir::new("static_lib_negative").expect("scratch");
        let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
        write(&dir, "main.c", MAIN_C);
        let (archive, module) = fixture(&dir);
        let full = layout::static_link_args(HOST, &archive, &[]);
        assert!(
            loads(&dir, "full", &full, &module),
            "the recipe must resolve"
        );
        // The archive named plainly: only the member `main` calls is kept.
        let bare = [archive.clone().into_os_string()];
        assert!(!loads(&dir, "bare", &bare, &module), "an unloaded member");
        dropped_export(&dir, &archive, &module, &full);
    }

    /// Linux: an executable exports nothing without `--export-dynamic`.
    #[cfg(not(target_os = "macos"))]
    fn dropped_export(dir: &Path, archive: &Path, module: &Path, _full: &[OsString]) {
        let load_only = layout::archive_load_args(HOST, archive);
        assert!(!loads(dir, "load_only", &load_only, module), "no export");
    }

    /// macOS exports an executable's globals by default, so the export's
    /// job shows under `-dead_strip`: without it the unreferenced member is
    /// stripped away even though it was force-loaded.
    #[cfg(target_os = "macos")]
    fn dropped_export(dir: &Path, archive: &Path, module: &Path, full: &[OsString]) {
        let dead_strip = [OsString::from("-Xlinker"), OsString::from("-dead_strip")];
        let mut load_only = layout::archive_load_args(HOST, archive);
        load_only.extend(dead_strip.clone());
        assert!(!loads(dir, "load_only", &load_only, module), "stripped");
        let mut stripped_full = full.to_vec();
        stripped_full.extend(dead_strip);
        assert!(
            loads(dir, "full_stripped", &stripped_full, module),
            "exported"
        );
    }
}
