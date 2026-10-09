//! #1366: under `pycc build --ext --foreign-relative-imports`, the entry
//! module's top-level relative `from .x import a` binds the CPython object
//! of the package the artifact is imported under, resolved when its
//! `Py_mod_exec` runs, exactly as the same statement in a `.py` module of
//! that package does. Without the flag D-222 is unchanged: a relative
//! import is a project import.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. Each one compares the artifact against CPython's own
//! run of the same source installed at the same path of one package tree,
//! so no message text is transcribed. The lines this change needs covered
//! are covered by the non-ignored tests here and by the unit tests in
//! `src/modules/tests.rs`, `src/build_pipeline/ext_wiring_tests.rs`,
//! `crates/pycc_hir/src/import/tests/from_foreign.rs`,
//! `crates/pycc_hir/src/program/tests.rs` and
//! `crates/pycc_codegen/src/foreign_import/tests.rs`.

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

/// Without `--ext` the flag is a usage error: it only means anything for an
/// artifact CPython imports into a package.
#[test]
fn the_flag_requires_ext() {
    let dir = ScratchDir::new("1366_requires_ext").expect("scratch");
    let source = write(&dir, "m.py", "from .sib import x\n");
    let output = pycc()
        .args(["build", "--foreign-relative-imports"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(
        rendered.contains("the following required arguments were not provided"),
        "{rendered}"
    );
    assert!(rendered.contains("--ext"), "{rendered}");
}

/// `pycc check` has no counterpart of the flag, so a relative import there
/// keeps D-222's project-import answer: outside a package it is `T0021`.
#[test]
fn check_keeps_a_relative_import_a_project_import() {
    let dir = ScratchDir::new("1366_check_default").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", "from .sib import x\n"))
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

/// The parsed flag reaches the frontend: the relative import resolves (no
/// `T0021`, though no package and no `sib.py` exist) and compilation stops
/// at the later type error instead. A build without the flag stops at
/// `T0021`, which pins that it is the flag that made the difference.
#[test]
fn the_flag_reaches_the_ext_frontend() {
    let dir = ScratchDir::new("1366_flag_threaded").expect("scratch");
    let source = write(&dir, "m.py", "from .sib import x\nn: int = \"s\"\n");
    let build = |extra: &[&str]| {
        pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join("m"))
            .arg("--ext")
            .args(extra)
            .output()
            .expect("pycc should spawn")
    };
    let with_flag = build(&["--foreign-relative-imports"]);
    let rendered = format!("{}{}", stdout_of(&with_flag), stderr_of(&with_flag));
    assert_eq!(with_flag.status.code(), Some(1), "{rendered}");
    assert!(!rendered.contains("T0021"), "{rendered}");
    assert!(rendered.contains("error[T0025]"), "{rendered}");

    let without = build(&[]);
    let rendered = format!("{}{}", stdout_of(&without), stderr_of(&without));
    assert_eq!(without.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run, in one package tree.
// ---------------------------------------------------------------------

/// The artifact's file name for `-o <stage>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// A scratch directory holding the package tree `top` (with `top.other`,
/// `top.pkg.sib` and `top.pkg.sub.leaf`), the source under `src/` and the
/// staged artifact under `stage/`, neither of which is ever on `sys.path`.
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
            ("top/pkg/sib.py", "x = 41\n"),
            ("top/pkg/sub/__init__.py", ""),
            ("top/pkg/sub/leaf.py", "z = 5\n"),
        ] {
            write(&dir, file, text);
        }
        let source = write(&dir, "src/m.py", body);
        std::fs::create_dir_all(dir.join("stage")).expect("mkdir stage");
        // Built outside the package tree: CPython's finder tries extension
        // loaders before source loaders, so an artifact already in
        // `top/pkg` would shadow the `.py` oracle.
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join("stage").join("m"))
            .args(["--ext", "--foreign-relative-imports"])
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{}", stderr_of(&build));
        assert!(dir.join("stage").join(artifact_name()).is_file());
        Self { dir }
    }

    /// Imports `name` (`top.pkg.m`, or `m` with `top/pkg` on `sys.path`),
    /// after printing which file the finder picked.
    fn run(&self, name: &str) -> Output {
        let prelude = if name == "m" {
            "sys.path.insert(0, 'top/pkg')\n"
        } else {
            ""
        };
        let script = format!(
            "import importlib, importlib.util, os, sys\n\
             {prelude}\
             print('origin:', os.path.basename(importlib.util.find_spec({name:?}).origin))\n\
             importlib.import_module({name:?})\n"
        );
        host_python()
            .args(["-B", "-u", "-c", &script])
            .current_dir(&*self.dir)
            .output()
            .expect("python3 should spawn")
    }

    /// Runs the `.py` source, then the artifact, each installed as
    /// `top/pkg/m.*` of the same tree, and checks that each run used its own
    /// loader and that both printed and raised the same thing. Returns the
    /// shared stdout without the origin line, and the final stderr line.
    fn assert_matches_cpython(&self, name: &str) -> (String, String) {
        let installed_source = self.dir.join("top/pkg/m.py");
        std::fs::copy(self.dir.join("src/m.py"), &installed_source).expect("install m.py");
        let oracle = self.run(name);
        std::fs::remove_file(&installed_source).expect("remove m.py");
        std::fs::copy(
            self.dir.join("stage").join(artifact_name()),
            self.dir.join("top/pkg").join(artifact_name()),
        )
        .expect("install the artifact");
        let compiled = self.run(name);

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
fn relative_imports_bind_the_host_package_s_objects() {
    let fixture = Fixture::new(
        "1366_hosted_success",
        "from .sib import x\nfrom ..other import y\nfrom . import sib\n\
         print(str(x))\nprint(str(y))\nprint(str(sib.x))\n",
    );
    let (stdout, last) = fixture.assert_matches_cpython("top.pkg.m");
    assert_eq!(stdout, "41\n7\n41\n");
    assert_eq!(last, "");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dotted_relative_import_binds_the_leaf_s_object() {
    let fixture = Fixture::new(
        "1366_hosted_dotted",
        "from .sub.leaf import z\nprint(str(z))\n",
    );
    let (stdout, last) = fixture.assert_matches_cpython("top.pkg.m");
    assert_eq!(stdout, "5\n");
    assert_eq!(last, "");
}

/// Every failure raises what the `.py` form raises: a missing sibling
/// module, a missing name (through a module and through the package itself,
/// `from . import nope`), a climb beyond the top-level package, and the
/// artifact imported top-level with no parent package.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn relative_import_failures_raise_what_cpython_raises() {
    for (tag, body, name, error) in [
        (
            "1366_hosted_missing_module",
            "from .missing import z\n",
            "top.pkg.m",
            "ModuleNotFoundError",
        ),
        (
            "1366_hosted_missing_name",
            "from .sib import nope\n",
            "top.pkg.m",
            "ImportError",
        ),
        (
            "1366_hosted_missing_in_package",
            "from . import nope\n",
            "top.pkg.m",
            "ImportError",
        ),
        (
            "1366_hosted_beyond_top",
            "from ...beyond import a\n",
            "top.pkg.m",
            "ImportError",
        ),
        (
            "1366_hosted_no_parent",
            "from .sib import x\n",
            "m",
            "ImportError",
        ),
    ] {
        let fixture = Fixture::new(tag, &format!("print('before')\n{body}print('after')\n"));
        let (stdout, last) = fixture.assert_matches_cpython(name);
        assert_eq!(stdout, "before\n", "{tag}");
        assert!(last.starts_with(&format!("{error}: ")), "{tag}: {last}");
    }
}
