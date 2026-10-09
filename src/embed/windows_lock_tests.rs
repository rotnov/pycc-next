//! `pycc lock` and `plan_embed` on the Windows-shaped fake layout (#1296):
//! a Windows lock section identifies the interpreter by its DLL, and a
//! Windows build bundles the locked pure-Python closure in
//! `OUT.pycc\closure\`, with the natives its PE images need in
//! `OUT.pycc\natives\` (#1306); an image or native whose imports no
//! strict rule places is refused by the lock and the build alike. Both run
//! on every host, so the coverage host drives every Windows arm; the real
//! Windows build is `tests/issue_1296_windows_locked_closure.rs`.

use super::super::super::fake_layout::{fake_windows_layout, write_pe_dll};
use super::super::super::pe::fixture::PeSpec;
use super::*;
use crate::lock::LockFailure;
use crate::lock::fixture::append_record;
use crate::lock::fixture::record_hash;

const WINDOWS: (&str, &str) = ("x86_64", "windows");

/// An [`Env`] whose interpreter is the Windows-shaped fake layout, with
/// `tinypkg` requiring `tinydep` in purelib, plus `extra` files in
/// `tinypkg`'s RECORD and `dep_extra` in `tinydep`'s.
fn windows_env(tag: &str, body: &str, extra: &[(&str, &[u8])], dep_extra: &[(&str, &[u8])]) -> Env {
    let dir = ScratchDir::new(tag).unwrap();
    let root = std::fs::canonicalize(&*dir).unwrap();
    let layout_root = root.join("layout");
    std::fs::create_dir_all(&layout_root).unwrap();
    let layout = fake_windows_layout(&layout_root);
    let (pure, plat) = (root.join("pure"), root.join("plat"));
    std::fs::create_dir_all(&pure).unwrap();
    std::fs::create_dir_all(&plat).unwrap();
    let mut files: Vec<(&str, &[u8])> = vec![("tinypkg/__init__.py", b"from tinydep import X\n")];
    files.extend_from_slice(extra);
    write_dist(&pure, "tinypkg", "1.0", &files, &["tinydep"]);
    let mut dep_files: Vec<(&str, &[u8])> = vec![("tinydep/__init__.py", b"X = 1\n")];
    dep_files.extend_from_slice(dep_extra);
    write_dist(&pure, "tinydep", "2.0", &dep_files, &[]);
    let lock_probe = parse_lock_probe(&probe_lines(&pure, &plat)).unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let entry = project.join("m.py");
    std::fs::write(&entry, body).unwrap();
    Env {
        root,
        _dir: dir,
        layout,
        pure,
        plat,
        lock_probe,
        entry,
    }
}

impl Env {
    fn lock_windows(&self) {
        self.lock_result()
            .expect("`pycc lock` must succeed on Windows");
    }

    /// `pycc lock` for Windows, with an environment failure's message; any
    /// other failure is an empty message, which no assertion accepts.
    fn lock_result(&self) -> Result<(), String> {
        let result = crate::lock::run_lock_on(
            &self.entry,
            false,
            InteropCli::default(),
            &self.toolchain(),
            WINDOWS,
        );
        result.map_err(|failure| match failure {
            LockFailure::Env(message) => message,
            LockFailure::Frontend(_) | LockFailure::Stale(_) => String::new(),
        })
    }

    /// Writes `bytes` at `rel` under purelib without recording it: a native
    /// beside a closure image.
    fn put(&self, rel: &str, bytes: &[u8]) {
        std::fs::write(self.pure.join(rel), bytes).unwrap();
    }

    fn lock_text(&self) -> String {
        std::fs::read_to_string(self.entry.with_file_name("pycc.lock")).unwrap()
    }

    fn embed_windows(&self) -> Result<EmbedPlan, String> {
        let hir = crate::frontend::lock_frontend(&self.entry, InteropCli::default())
            .unwrap_or_else(|_| panic!("the fixture must type-check"));
        plan_embed(
            &self.out(),
            &self.entry,
            &hir,
            &self.toolchain(),
            EmbedPlatform::Windows,
            WINDOWS,
            &self.obj(),
        )
    }
}

/// A Windows lock section records the interpreter DLL's digest and no
/// native library; the build bundles the closure beside `Lib\` and
/// `DLLs\` and defines `PYCC_EMBED_CLOSURE` for the launcher.
#[test]
fn a_windows_build_bundles_the_locked_closure() {
    let env = windows_env("embed_windows_lock_closure", "import tinypkg\n", &[], &[]);
    env.lock_windows();
    let text = env.lock_text();
    assert!(text.contains("x86_64-pc-windows-msvc"), "{text}");
    let dll = std::fs::read(env.layout.prefix.join("python314.dll")).unwrap();
    let digest = sha256::sha256_hex(&dll);
    assert!(
        text.contains(&format!("libpython-sha256 = \"{digest}\"")),
        "{text}"
    );
    assert!(!text.contains("[[target.native]]"), "{text}");
    env.embed_windows().expect("embedded");
    let files = relative_files(&env.sidecar());
    for rel in [
        "closure/tinypkg/__init__.py",
        "closure/tinydep/__init__.py",
        "closure/tinypkg-1.0.dist-info/METADATA",
        "Lib/os.py",
        "DLLs/_ssl.pyd",
        "python314.dll",
    ] {
        assert!(files.iter().any(|file| file == rel), "{rel}: {files:?}");
    }
    assert_eq!(
        std::fs::read(env.sidecar().join("closure/tinypkg/__init__.py")).unwrap(),
        b"from tinydep import X\n"
    );
    assert!(env.config().contains("#define PYCC_EMBED_CLOSURE"));
}

/// A standard-library-only program's section locks no package: the build
/// copies no closure and defines no `PYCC_EMBED_CLOSURE`.
#[test]
fn a_windows_standard_library_program_bundles_no_closure() {
    let env = windows_env("embed_windows_lock_stdlib", "import json\n", &[], &[]);
    env.lock_windows();
    env.embed_windows().expect("embedded");
    let files = relative_files(&env.sidecar());
    assert!(
        !files.iter().any(|file| file.starts_with("closure/")),
        "{files:?}"
    );
    assert!(!env.config().contains("PYCC_EMBED_CLOSURE"));
}

const FAST: &str = "tinydep/_fast.pyd";
const NATIVE: &str = "tinydep/pyccnative.dll";

/// An [`Env`] whose transitive `tinydep` locks `_fast.pyd`, importing the
/// unrecorded `pyccnative.dll` beside it, which imports `native_imports`.
fn native_env(tag: &str, body: &str, native_imports: &[&str]) -> Env {
    let fast = PeSpec::dll(&["pyccnative.dll", "python314.dll", "KERNEL32.dll"]).bytes();
    let env = windows_env(tag, body, &[], &[(FAST, &fast)]);
    write_pe_dll(&env.pure.join(NATIVE), native_imports);
    env
}

/// A PE image added to a distribution's RECORD after a clean lock that
/// cannot be scanned (a truncated `MZ`) is refused by `pycc lock`, leaving
/// the earlier lock byte-identical, and by the build against the earlier
/// lock before staging, leaving the earlier sidecar intact.
#[test]
fn an_unscannable_closure_image_is_refused_by_lock_and_build() {
    let env = windows_env("embed_windows_lock_image", "import tinypkg\n", &[], &[]);
    env.lock_windows();
    let before = env.lock_text();
    let bytes = b"MZ\x90\x00";
    env.put(FAST, bytes);
    let line = format!("{FAST},{},{}", record_hash(bytes), bytes.len());
    append_record(&env.pure.join("tinydep-2.0.dist-info"), &line);
    let err = env.lock_result().expect_err("refused by lock");
    for part in [
        "cannot scan `closure\\tinydep\\_fast.pyd`",
        "`tinydep`",
        "pycc build --ext",
    ] {
        assert!(err.contains(part), "{part}: {err}");
    }
    assert_eq!(env.lock_text(), before);
    env.previous_sidecar();
    let err = env.embed_windows().expect_err("refused by the build");
    assert!(
        err.contains("cannot scan `closure\\tinydep\\_fast.pyd`"),
        "{err}"
    );
    env.assert_previous_sidecar_intact();
}

/// Any other failure of `pycc lock` than an environment one carries no
/// message the refusal assertions accept.
#[test]
fn a_frontend_lock_failure_has_no_environment_message() {
    let env = windows_env("embed_windows_lock_frontend", "def (:\n", &[], &[]);
    assert_eq!(env.lock_result(), Err(String::new()));
}

/// An unrecorded DLL under the direct import root `tinypkg\` is refused by
/// the lock's coverage rule (D-249 rule 2) before any image is classified,
/// so it never becomes a native.
#[test]
fn an_unrecorded_dll_under_a_direct_root_is_refused_by_coverage() {
    let env = windows_env("embed_windows_lock_coverage", "import tinypkg\n", &[], &[]);
    write_pe_dll(&env.pure.join("tinypkg/pyccnative.dll"), &["KERNEL32.dll"]);
    let err = env.lock_result().expect_err("refused");
    assert!(err.contains("pyccnative.dll"), "{err}");
    assert!(err.contains("RECORD"), "{err}");
    assert!(!err.contains("natives"), "{err}");
}

/// A transitive distribution's `.pyd` importing an unrecorded DLL beside
/// it locks that DLL as one `[[target.native]]` under its on-disk name,
/// required by `tinydep`; the build copies it into `natives\`, never into
/// `closure\`, and defines `PYCC_EMBED_NATIVES` for the launcher.
#[test]
fn a_windows_closure_native_is_locked_and_copied_into_natives() {
    let env = native_env(
        "embed_windows_lock_native",
        "import tinypkg\n",
        &["KERNEL32.dll"],
    );
    env.lock_windows();
    let text = env.lock_text();
    assert!(text.contains("[[target.native]]"), "{text}");
    assert!(text.contains("name = \"pyccnative.dll\""), "{text}");
    assert!(text.contains("required-by = [\"tinydep\"]"), "{text}");
    env.embed_windows().expect("embedded");
    let files = relative_files(&env.sidecar());
    assert!(
        files.iter().any(|f| f == "natives/pyccnative.dll"),
        "{files:?}"
    );
    assert!(
        files.iter().any(|f| f == "closure/tinydep/_fast.pyd"),
        "{files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "closure/tinydep/pyccnative.dll"),
        "{files:?}"
    );
    assert_eq!(
        std::fs::read(env.sidecar().join("natives/pyccnative.dll")).unwrap(),
        std::fs::read(env.pure.join(NATIVE)).unwrap()
    );
    let config = env.config();
    assert!(config.contains("#define PYCC_EMBED_NATIVES 1"), "{config}");
    assert!(config.contains("#define PYCC_EMBED_CLOSURE"), "{config}");
}

/// A native changed after the lock makes the build stale, before staging.
#[test]
fn a_windows_native_changed_after_the_lock_is_stale() {
    let env = native_env("embed_windows_lock_native_stale", "import tinypkg\n", &[]);
    env.lock_windows();
    write_pe_dll(&env.pure.join(NATIVE), &["KERNEL32.dll"]);
    env.previous_sidecar();
    let err = env.embed_windows().expect_err("stale");
    assert!(err.contains("pyccnative.dll"), "{err}");
    assert!(err.contains("pycc lock"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// A native named like a file in the interpreter's `DLLs\` is refused by
/// the lock.
#[test]
fn a_windows_native_named_like_an_interpreter_dll_is_refused() {
    let fast = PeSpec::dll(&["tcl86t.dll"]).bytes();
    let env = windows_env(
        "embed_windows_lock_native_dlls",
        "import tinypkg\n",
        &[],
        &[(FAST, &fast)],
    );
    write_pe_dll(&env.pure.join("tinydep/tcl86t.dll"), &[]);
    let err = env.lock_result().expect_err("refused");
    assert!(err.contains("the native `natives\\tcl86t.dll`"), "{err}");
    assert!(err.contains("`DLLs\\tcl86t.dll`"), "{err}");
}

/// A native whose import no rule places, after a clean lock, is refused by
/// `pycc lock`, leaving the earlier lock byte-identical, and by the build,
/// which writes nothing.
#[test]
fn a_windows_native_with_an_unresolvable_import_is_refused_by_lock_and_build() {
    let env = native_env("embed_windows_lock_native_bad", "import tinypkg\n", &[]);
    env.lock_windows();
    let before = env.lock_text();
    write_pe_dll(&env.pure.join(NATIVE), &["pycc1306missing.dll"]);
    let err = env.lock_result().expect_err("refused by lock");
    for part in [
        "the native `natives\\pyccnative.dll` (copied for distribution `tinydep`",
        "imports `pycc1306missing.dll`",
        "pycc build --ext",
    ] {
        assert!(err.contains(part), "{part}: {err}");
    }
    assert_eq!(env.lock_text(), before);
    let err = env.embed_windows().expect_err("refused by the build");
    assert!(err.contains("imports `pycc1306missing.dll`"), "{err}");
    assert!(!env.sidecar().exists());
    assert!(!env.out().exists());
}

/// A closure `.pyd` whose import no rule places is refused by `pycc lock`,
/// naming the image, its distribution and the import; no lock is written.
#[test]
fn a_windows_closure_image_with_an_unresolvable_import_is_refused_by_lock() {
    let fast = PeSpec::dll(&["pycc1306missing.dll"]).bytes();
    let env = windows_env(
        "embed_windows_lock_image_bad",
        "import tinypkg\n",
        &[],
        &[(FAST, &fast)],
    );
    let err = env.lock_result().expect_err("refused");
    assert!(
        err.starts_with(
            "`closure\\tinydep\\_fast.pyd` (distribution `tinydep`) imports \
             `pycc1306missing.dll`, which is neither"
        ),
        "{err}"
    );
    assert!(!env.entry.with_file_name("pycc.lock").exists());
}

/// An `.exe` launcher with the `MZ` magic is not an image: it locks and is
/// bundled as data (#1296).
#[test]
fn a_windows_exe_in_the_closure_is_bundled_as_data() {
    let env = windows_env(
        "embed_windows_lock_exe",
        "import tinypkg\n",
        &[
            ("tinypkg/cli.exe", b"MZ\x90\x00"),
            ("tinypkg/CLI2.EXE", b"MZ"),
        ],
        &[],
    );
    env.lock_windows();
    assert!(!env.lock_text().contains("[[target.native]]"));
    env.embed_windows().expect("an `.exe` is bundled as data");
    assert_eq!(
        std::fs::read(env.sidecar().join("closure/tinypkg/cli.exe")).unwrap(),
        b"MZ\x90\x00"
    );
    assert!(!env.sidecar().join("natives").exists());
    assert!(!env.config().contains("PYCC_EMBED_NATIVES"));
}

/// An installed optional root (#1290) is resolved like a required one: the
/// native its transitive distribution's `.pyd` needs is locked the same
/// way, under `optional-roots`, and vendored by the build.
#[test]
fn an_optional_root_locks_and_vendors_its_native() {
    let body = "try:\n    import tinypkg\nexcept ImportError:\n    pass\n";
    let env = native_env(
        "embed_windows_lock_native_optional",
        body,
        &["KERNEL32.dll"],
    );
    env.lock_windows();
    let text = env.lock_text();
    assert!(text.contains("optional-roots = [\"tinypkg\"]"), "{text}");
    assert!(text.contains("name = \"pyccnative.dll\""), "{text}");
    assert!(text.contains("required-by = [\"tinydep\"]"), "{text}");
    env.embed_windows().expect("embedded");
    assert!(env.sidecar().join("natives/pyccnative.dll").is_file());
}

/// A locked file that vanished before the build is refused by the
/// payload's own missing-file check, which runs ahead of the image scan
/// and leaves the previous sidecar untouched.
#[test]
fn a_windows_build_reports_a_vanished_closure_file() {
    let env = windows_env("embed_windows_lock_vanished", "import tinypkg\n", &[], &[]);
    env.lock_windows();
    std::fs::remove_file(env.pure.join("tinydep/__init__.py")).unwrap();
    env.previous_sidecar();
    let err = env.embed_windows().expect_err("vanished");
    assert!(err.contains("tinydep"), "{err}");
    env.assert_previous_sidecar_intact();
}

/// A closure file changed after the lock is refused by its RECORD digest,
/// and an interpreter DLL changed after the lock by `libpython-sha256`,
/// which is the digest of the very file the sidecar bundles; both leave
/// the previous sidecar untouched.
#[test]
fn a_windows_build_refuses_a_stale_closure_or_interpreter() {
    let env = windows_env("embed_windows_lock_stale", "import tinypkg\n", &[], &[]);
    env.lock_windows();
    std::fs::write(env.pure.join("tinypkg/__init__.py"), "changed\n").unwrap();
    env.previous_sidecar();
    let err = env.embed_windows().expect_err("stale file");
    assert!(err.contains("does not match its RECORD hash"), "{err}");
    env.assert_previous_sidecar_intact();

    let env = windows_env("embed_windows_lock_stale_dll", "import tinypkg\n", &[], &[]);
    env.lock_windows();
    // Still an image the scan accepts, so the digest check refuses it.
    let dll = env.layout.prefix.join("python314.dll");
    let mut rebuilt = std::fs::read(&dll).unwrap();
    rebuilt.push(0);
    std::fs::write(&dll, rebuilt).unwrap();
    env.previous_sidecar();
    let err = env.embed_windows().expect_err("stale DLL");
    assert!(err.contains("libpython-sha256"), "{err}");
    env.assert_previous_sidecar_intact();
}
