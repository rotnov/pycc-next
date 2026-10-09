//! #1382: under `pycc build --ext --foreign-relative-imports`, an absolute
//! import of the entry module's own top-level package (`from top.other
//! import y` in `top/pkg/m.py`) binds CPython's module from the host
//! interpreter, exactly as the same statement in a `.py` module installed
//! at that path does, instead of linking the package's modules natively.
//! Without the flag D-222 is unchanged.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. Each builds the entry inside its package tree,
//! stages the artifact outside it, and compares the artifact against
//! CPython's own run of the same source at the same path, so no message
//! text is transcribed. The lines this change needs covered are covered by
//! the non-ignored tests here and by `src/modules/tests/entry_package.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `body` to `dir/<file>`, creating its parent directories, and
/// returns the path.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// Builds `source` as an `--ext` artifact into `stage/m`, with `extra`
/// flags, and returns the combined stdout and stderr with the exit code.
fn build(dir: &Path, source: &Path, extra: &[&str]) -> (Option<i32>, String) {
    let output = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(dir.join("stage").join("m"))
        .arg("--ext")
        .args(extra)
        .output()
        .expect("pycc should spawn");
    (
        output.status.code(),
        format!("{}{}", stdout_of(&output), stderr_of(&output)),
    )
}

/// The flag reaches the in-tree build: `top/other.py` holds a type error,
/// which the build reports only when it links that module natively. With
/// the flag the import binds CPython's module, so the build stops at the
/// entry's own later type error instead.
#[test]
fn the_flag_stops_linking_the_entry_s_own_package() {
    let dir = ScratchDir::new("1382_flag_threaded").expect("scratch");
    write(&dir, "top/__init__.py", "");
    write(&dir, "top/other.py", "y: int = \"other\"\n");
    write(&dir, "top/pkg/__init__.py", "");
    let source = write(
        &dir,
        "top/pkg/m.py",
        "from top.other import y\nn: int = \"entry\"\n",
    );

    let (code, rendered) = build(&dir, &source, &["--foreign-relative-imports"]);
    assert_eq!(code, Some(1), "{rendered}");
    assert!(!rendered.contains("other.py"), "{rendered}");
    assert!(rendered.contains("m.py"), "{rendered}");
    assert!(rendered.contains("error[T0025]"), "{rendered}");

    let (code, rendered) = build(&dir, &source, &[]);
    assert_eq!(code, Some(1), "{rendered}");
    assert!(rendered.contains("other.py"), "{rendered}");
}

/// In the skeleton tree (only the `__init__.py` files) the dotted import
/// was a `C0001` without the flag, and is foreign with it.
#[test]
fn the_skeleton_tree_import_resolves_under_the_flag() {
    let dir = ScratchDir::new("1382_skeleton").expect("scratch");
    write(&dir, "top/__init__.py", "");
    write(&dir, "top/pkg/__init__.py", "");
    let source = write(
        &dir,
        "top/pkg/m.py",
        "from top.exceptions import Bad\nn: int = \"entry\"\n",
    );

    let (code, rendered) = build(&dir, &source, &["--foreign-relative-imports"]);
    assert_eq!(code, Some(1), "{rendered}");
    assert!(!rendered.contains("C0001"), "{rendered}");
    assert!(rendered.contains("error[T0025]"), "{rendered}");

    let (code, rendered) = build(&dir, &source, &[]);
    assert_eq!(code, Some(1), "{rendered}");
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(rendered.contains("top.exceptions"), "{rendered}");
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run, in one package tree.
// ---------------------------------------------------------------------

/// The artifact's file name for `-o <stage>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// The package tree `top` (with `top.other` and `top.pkg`), the entry built
/// in place as `top/pkg/m.py`, and the artifact staged under `stage/`,
/// which is never on `sys.path`.
struct Fixture {
    dir: ScratchDir,
}

impl Fixture {
    fn new(tag: &str, body: &str) -> Self {
        let dir = ScratchDir::new(tag).expect("scratch");
        for (file, text) in [
            ("top/__init__.py", ""),
            ("top/other.py", "y = 7\n"),
            ("top/pkg/__init__.py", ""),
        ] {
            write(&dir, file, text);
        }
        let source = write(&dir, "top/pkg/m.py", body);
        std::fs::create_dir_all(dir.join("stage")).expect("mkdir stage");
        let (code, rendered) = build(&dir, &source, &["--foreign-relative-imports"]);
        assert_eq!(code, Some(0), "{rendered}");
        assert!(dir.join("stage").join(artifact_name()).is_file());
        Self { dir }
    }

    /// Imports `top.pkg.m` after printing which file the finder picked.
    fn run(&self) -> Output {
        let script = "import importlib, importlib.util, os\n\
             print('origin:', os.path.basename(importlib.util.find_spec('top.pkg.m').origin))\n\
             importlib.import_module('top.pkg.m')\n";
        host_python()
            .args(["-B", "-u", "-c", script])
            .current_dir(&*self.dir)
            .output()
            .expect("python3 should spawn")
    }

    /// Runs the in-tree `.py` source, then replaces it with the artifact and
    /// runs that, checking that each run used its own loader and that both
    /// printed and raised the same thing. Returns the shared stdout without
    /// the origin line, and the final stderr line.
    fn assert_matches_cpython(&self) -> (String, String) {
        let oracle = self.run();
        std::fs::remove_file(self.dir.join("top/pkg/m.py")).expect("remove m.py");
        std::fs::copy(
            self.dir.join("stage").join(artifact_name()),
            self.dir.join("top/pkg").join(artifact_name()),
        )
        .expect("install the artifact");
        let compiled = self.run();
        let split = |output: &Output, origin: &str| {
            let stdout = stdout_of(output);
            let (first, rest) = stdout.split_once('\n').unwrap_or((&stdout, ""));
            assert_eq!(
                first,
                format!("origin: {origin}"),
                "stdout: {stdout}\nstderr: {}",
                stderr_of(output)
            );
            let last = stderr_of(output)
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .to_string();
            (rest.to_string(), last, output.status.success())
        };
        let oracle = split(&oracle, "m.py");
        let compiled = split(&compiled, artifact_name());
        assert_eq!(compiled, oracle);
        (compiled.0, compiled.1)
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn own_package_imports_bind_the_host_package_s_objects() {
    let fixture = Fixture::new(
        "1382_hosted_success",
        "from top.other import y\nfrom top import other\nimport top.other as o\n\
         print(str(y))\nprint(str(other.y))\nprint(str(o.y))\n",
    );
    let (stdout, last) = fixture.assert_matches_cpython();
    assert_eq!(stdout, "7\n7\n7\n");
    assert_eq!(last, "");
}

/// A nested import of the own package takes its `except ImportError`
/// fallback when the module is missing, as the `.py` form does (#1383).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_nested_own_package_import_falls_back_like_cpython() {
    let fixture = Fixture::new(
        "1382_hosted_nested",
        "try:\n    from top.missing import a\nexcept ImportError:\n    print('fallback')\n",
    );
    let (stdout, last) = fixture.assert_matches_cpython();
    assert_eq!(stdout, "fallback\n");
    assert_eq!(last, "");
}

/// A missing module and a missing name raise what the `.py` form raises.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn own_package_import_failures_raise_what_cpython_raises() {
    for (tag, body, error) in [
        (
            "1382_hosted_missing_module",
            "from top.missing import z\n",
            "ModuleNotFoundError",
        ),
        (
            "1382_hosted_missing_name",
            "from top.other import nope\n",
            "ImportError",
        ),
    ] {
        let fixture = Fixture::new(tag, &format!("print('before')\n{body}print('after')\n"));
        let (stdout, last) = fixture.assert_matches_cpython();
        assert_eq!(stdout, "before\n", "{tag}");
        assert!(last.starts_with(&format!("{error}: ")), "{tag}: {last}");
    }
}
