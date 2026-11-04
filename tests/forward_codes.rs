//! v0.12: the forward references `typing` makes for the string annotations
//! of a `TypedDict` or `NamedTuple` class a fallback chunk defines are
//! compiled at build time (`src/ext_build/precompile.py`'s `fwds`), and
//! importing the `--ext` module uses those code objects instead of compiling
//! each text (`src/ext_build/precompiled_hook.c`). Everything CPython code
//! sees is the source's: each class's keys and fields, every annotation's
//! `ForwardRef` (its text, flags, module and code object field by field),
//! each text's `compile` audit event in order, the types they evaluate to,
//! a distinct code object per reference, and `annotationlib`'s (`typing`'s
//! on 3.13) namespace after the import. A text the build did not foresee
//! (an invalid one, a `ForwardRef` made by hand) is compiled at import as
//! before, as is every text when `builtins.compile` is replaced, the module
//! binds a `compile` of its own, the interpreter runs with `-O`, or the
//! module was built `--exact-cpython`.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the sources.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forward_codes").join(name)
}

/// The fixture's text with LF line endings (a Windows checkout has CRLF).
fn source(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).expect("read the fixture").replace("\r\n", "\n")
}

/// The modules built as `--ext` modules (with `extra` flags) in a fresh
/// directory, their sources removed, the driver beside them.
fn built(label: &str, extra: &[&str]) -> ScratchDir {
    let dir = ScratchDir::new(label).expect("scratch");
    std::fs::write(dir.join("driver.py"), source("driver.py")).expect("write the driver");
    for module in ["fm", "fe"] {
        let path = dir.join(format!("{module}.py"));
        std::fs::write(&path, source(&format!("{module}.py"))).expect("write the module");
        let build = pycc()
            .args(["build", "--ext", "--non-strict"])
            .args(extra)
            .arg(&path)
            .arg("-o")
            .arg(dir.join(module))
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{}", text(&build.stderr));
        std::fs::remove_file(&path).expect("import the artifact, not the source");
    }
    dir
}

fn plain(label: &str) -> ScratchDir {
    let dir = ScratchDir::new(label).expect("scratch");
    for name in ["driver.py", "fm.py", "fe.py"] {
        std::fs::write(dir.join(name), source(name)).expect("write the source");
    }
    dir
}

/// The driver's output in `dir` (`mode`: "", `patched`, `bound` or
/// `static`), with a fixed hash seed.
fn run(dir: &Path, mode: &str, optimize: bool) -> Output {
    let mut cmd = python();
    if optimize {
        cmd.arg("-O");
    }
    cmd.args(["-B", "driver.py"]);
    if !mode.is_empty() {
        cmd.arg(mode);
    }
    cmd.env("PYTHONHASHSEED", "0").current_dir(dir).output().expect("python should spawn")
}

/// The driver's count of references given a build-time code object.
fn ahead(out: &Output) -> usize {
    let err = text(&out.stderr);
    err.lines()
        .find_map(|line| line.strip_prefix("ahead-of-time: "))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no count in: {err}"))
}

/// The oracle interpreter's `major.minor`.
fn python_version() -> String {
    python()
        .args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
        .output()
        .map(|out| text(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

fn compare(ours: &Path, oracle: &Path, mode: &str, optimize: bool) -> usize {
    let (got, want) = (run(ours, mode, optimize), run(oracle, mode, optimize));
    assert_eq!(want.status.code(), Some(0), "{}", text(&want.stderr));
    assert_eq!(got.status.code(), Some(0), "{}", text(&got.stderr));
    assert_eq!(text(&got.stdout), text(&want.stdout), "mode {mode:?} -O {optimize}");
    ahead(&got)
}

#[test]
fn forward_reference_codes_compiled_ahead_match_cpython() {
    let ours = built("forward_codes", &[]);
    let oracle = plain("forward_codes_src");
    let count = compare(&ours, &oracle, "", false);
    // Every annotation of the modules' `TypedDict` and `NamedTuple` classes
    // but `Bad`'s (not an expression) and `b` of `Eager` (`list["str"]`: the
    // reference is made inside `list[...]`, from a text the build does not
    // see as an annotation).
    if matches!(python_version().as_str(), "3.13" | "3.14" | "3.15") {
        assert_eq!(count, 21);
    }
    // A replaced `builtins.compile` is called with each text; a `compile`
    // the module binds itself is left alone.
    assert_eq!(compare(&ours, &oracle, "patched", false), 0);
    assert_eq!(compare(&ours, &oracle, "bound", false), 0);
    // `compile` bound to a builtin with no `self`: each text goes to it,
    // raising what CPython raises.
    assert_eq!(compare(&ours, &oracle, "static", false), 0);
    // `-O`: no chunk's code is used, every text is compiled.
    assert_eq!(compare(&ours, &oracle, "", true), 0);
}

#[test]
fn exact_cpython_compiles_every_reference_at_import() {
    let ours = built("forward_codes_exact", &["--exact-cpython"]);
    let oracle = plain("forward_codes_exact_src");
    assert_eq!(compare(&ours, &oracle, "", false), 0);
}
