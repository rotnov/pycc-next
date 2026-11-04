//! v0.12: the methods `dataclasses` generates for a dataclass a fallback
//! chunk defines are compiled at build time (`src/ext_build/precompile.py`'s
//! `dcs`), and importing the `--ext` module runs those code objects instead
//! of compiling the generated text (`src/ext_build/precompiled_hook.c`).
//! Everything CPython code sees is the source's: each class's fields,
//! signature, parameters and every generated method (its qualified name,
//! module, defaults, annotations, and its code object field by field, the
//! frozen dataclass's field-name set constant in the same order), the
//! `compile` and `exec` audit events for each generated text, instances'
//! behavior and errors, and `dataclasses`' namespace after the import. A
//! text the build did not foresee (a base from another module, a dataclass
//! made by `make_dataclass`) is compiled at import as before, as is every text when `builtins.exec` is
//! replaced, `dataclasses` binds an `exec` of its own, the interpreter runs
//! with `-O`, or the module was built `--exact-cpython`; an `exec` bound to
//! another builtin, or a `builtins.exec` replaced by one, gets each text as
//! in CPython. Two chunks running at once in two threads share one binding,
//! removed when the last ends.
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dataclass_texts").join(name)
}

/// The fixture's text with LF line endings (a Windows checkout has CRLF).
fn source(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).expect("read the fixture").replace("\r\n", "\n")
}

/// The modules built as `--ext` modules (with `extra` flags) in a fresh
/// directory, their sources removed, the driver beside them.
fn built(label: &str, extra: &[&str]) -> ScratchDir {
    let dir = ScratchDir::new(label).expect("scratch");
    for module in ["m", "fut", "ta", "tb"] {
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
    // Written after the builds: `sync` beside `ta` would be built as a
    // module of its program, not imported from CPython.
    for name in ["driver.py", "sync.py"] {
        std::fs::write(dir.join(name), source(name)).expect("write the driver");
    }
    dir
}

fn plain(label: &str) -> ScratchDir {
    let dir = ScratchDir::new(label).expect("scratch");
    for name in ["driver.py", "sync.py", "m.py", "fut.py", "ta.py", "tb.py"] {
        std::fs::write(dir.join(name), source(name)).expect("write the source");
    }
    dir
}

/// The driver's output in `dir` (`mode`: "", `patched`, `bound`, `static`,
/// `eval` or `threads`), with a
/// fixed hash seed so both sides build their sets alike.
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

/// The driver's count of generated texts run from a build-time code object.
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
fn dataclass_methods_compiled_ahead_match_cpython() {
    let ours = built("dataclass_texts", &[]);
    let oracle = plain("dataclass_texts_src");
    let count = compare(&ours, &oracle, "", false);
    // Every class statement under a `dataclass(...)` decorator of constant
    // options in the modules' fallback chunks (`Twin2` with code objects of
    // its own, not `Twin1`'s), but `FromElsewhere` (a base the build cannot
    // see) and the classes inside a `try` (`BadOrder`; the others raise
    // before `exec`).
    if matches!(python_version().as_str(), "3.13" | "3.14" | "3.15") {
        assert_eq!(count, 13);
    }
    // A replaced `builtins.exec` is called with each text; an `exec`
    // `dataclasses` binds itself is left alone.
    assert_eq!(compare(&ours, &oracle, "patched", false), 0);
    assert_eq!(compare(&ours, &oracle, "bound", false), 0);
    // `-O`: no chunk's code is used, every text is compiled.
    assert_eq!(compare(&ours, &oracle, "", true), 0);
}

#[test]
fn exact_cpython_compiles_every_text_at_import() {
    let ours = built("dataclass_texts_exact", &["--exact-cpython"]);
    let oracle = plain("dataclass_texts_exact_src");
    assert_eq!(compare(&ours, &oracle, "", false), 0);
}

#[test]
fn foreign_and_concurrent_bindings_match_cpython() {
    let ours = built("dataclass_texts_bindings", &[]);
    let oracle = plain("dataclass_texts_bindings_src");
    // `dataclasses.exec` bound to a builtin with no `self`, and
    // `builtins.exec` replaced by another builtin of `builtins`: each text
    // goes to it, raising what CPython raises.
    assert_eq!(compare(&ours, &oracle, "static", false), 0);
    assert_eq!(compare(&ours, &oracle, "eval", false), 0);
    // Two chunks running at once in two threads, the first to start ending
    // first: both use their texts, and no `exec` is left bound.
    let count = compare(&ours, &oracle, "threads", false);
    if matches!(python_version().as_str(), "3.13" | "3.14" | "3.15") {
        assert_eq!(count, 2);
    }
}
