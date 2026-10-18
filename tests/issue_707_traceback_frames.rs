//! End-to-end coverage for #707 and v0.8 accept 3: an uncaught exception in
//! a native program prints CPython's traceback -- `Traceback (most recent
//! call last):`, one `File "...", line N, in <frame>` line per frame with
//! the frame's source lines and carets under it, outermost first -- and a
//! `raise ... from ...` chain renders CPython's "The above exception was the
//! direct cause of the following exception:" separator between cause and
//! effect, oldest cause first.
//!
//! Each program's `stderr` is compared with CPython's for the same file,
//! run from the same directory, so the absolute paths CPython prints for a
//! script agree too.

use pycc_scratch::ScratchDir;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> String {
    std::env::var("PYCC_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

/// Builds `source` as `src_name` in `dir`, runs it and CPython on it, and
/// asserts both fail with the same `stderr`; returns that text.
fn assert_traceback_matches_cpython(dir: &std::path::Path, src_name: &str, source: &str) -> String {
    let src = dir.join(src_name);
    std::fs::write(&src, source).unwrap();
    let out = dir.join(src_name.replace(".py", ""));
    let build = Command::new(pycc_bin())
        .current_dir(dir)
        .args(["build", src_name, "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).current_dir(dir).output().unwrap();
    let cpython = Command::new(python())
        .current_dir(dir)
        .arg(src_name)
        .output()
        .unwrap();
    assert!(
        !run.status.success(),
        "expected non-zero exit for uncaught exception"
    );
    assert_eq!(run.status.code(), cpython.status.code());
    // Line content: a native program writes `\n`, CPython on Windows `\r\n`.
    let err = String::from_utf8_lossy(&run.stderr).replace("\r\n", "\n");
    assert_eq!(
        err,
        String::from_utf8_lossy(&cpython.stderr).replace("\r\n", "\n")
    );
    err
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_module_level_uncaught_raise_shows_a_module_frame() {
    let dir = ScratchDir::new("707_module_frame").expect("failed to create scratch dir");
    let err =
        assert_traceback_matches_cpython(&dir, "module_frame.py", "raise ValueError(\"boom\")\n");
    assert!(
        err.contains(", line 1, in <module>\n    raise ValueError(\"boom\")\n"),
        "{err}"
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_raise_inside_a_function_names_that_functions_frame() {
    let dir = ScratchDir::new("707_function_frame").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "function_frame.py",
        "def boom() -> None:\n    raise ValueError(\"bad\")\n\nboom()\n",
    );
    assert!(err.contains(", line 4, in <module>\n    boom()\n"), "{err}");
    assert!(err.contains(", line 2, in boom\n"), "{err}");
}

// -- A deeper stack: every compiled frame appears, outermost first, each at
// -- the line its call or failing operation is on, with CPython's carets.

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn every_compiled_frame_appears_at_its_own_line() {
    let dir = ScratchDir::new("707_deep_stack").expect("failed to create scratch dir");
    assert_traceback_matches_cpython(
        &dir,
        "deep_stack.py",
        "def leaf(xs: list[int], i: int) -> int:\n    y = i + 1\n    return xs[i] * y\n\n\n\
         def middle(n: int) -> int:\n    total = 0\n    for i in range(n):\n        \
         total += leaf([1, 2, 3], i)\n    return total\n\n\n\
         def top() -> None:\n    r = middle(5)\n    print(r)\n\n\ntop()\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_division_shows_its_operator_carets() {
    let dir = ScratchDir::new("707_division").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "division.py",
        "def ratio(a: int, b: int) -> int:\n    return a // b\n\n\ndef run() -> None:\n    \
         x = ratio(4, 0)\n    print(x)\n\n\nrun()\n",
    );
    assert!(err.contains("~~^^~~"), "{err}");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn raise_from_renders_the_direct_cause_chain_oldest_first() {
    let dir = ScratchDir::new("707_raise_from_chain").expect("failed to create scratch dir");
    assert_traceback_matches_cpython(
        &dir,
        "raise_from_chain.py",
        "raise ValueError(\"effect\") from TypeError(\"cause\")\n",
    );
}

// -- A multi-level cause chain (`raise ... from ...` whose cause is itself the
// -- product of a nested `raise ... from ...`) walks every level, oldest
// -- first, each with the frames it passed through.

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_multi_level_cause_chain_walks_every_level() {
    let dir = ScratchDir::new("707_multi_level_chain").expect("failed to create scratch dir");
    assert_traceback_matches_cpython(
        &dir,
        "multi_level_chain.py",
        "def inner() -> None:\n    try:\n        raise KeyError(\"root\")\n    except KeyError as e:\n        raise TypeError(\"middle\") from e\n\ntry:\n    inner()\nexcept TypeError as e:\n    raise ValueError(\"outer\") from e\n",
    );
}

// -- A `raise` inside a class method, classmethod, staticmethod, or property
// -- setter names the plain Python source name of that method -- never
// -- `pycc_hir`'s mangled internal identifier (`ClassName.method_name`, with
// -- a further `.classmethod`/`.static`/`.setter` suffix).

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_raise_inside_a_plain_method_names_the_plain_method() {
    let dir = ScratchDir::new("707_method_frame").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "method_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def boom(self) -> None:\n        raise ValueError(\"bad\")\n\nC().boom()\n",
    );
    assert!(err.contains(", in boom\n"), "{err}");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_raise_inside_a_classmethod_names_the_plain_method_not_the_mangled_name() {
    let dir = ScratchDir::new("707_classmethod_frame").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "classmethod_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def create(cls) -> None:\n        raise ValueError(\"bad\")\n\nC.create()\n",
    );
    assert!(err.contains(", in create\n"), "{err}");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_raise_inside_a_staticmethod_names_the_plain_method_not_the_mangled_name() {
    let dir = ScratchDir::new("707_staticmethod_frame").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "staticmethod_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def make() -> None:\n        raise ValueError(\"bad\")\n\nC.make()\n",
    );
    assert!(err.contains(", in make\n"), "{err}");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_raise_inside_a_property_setter_names_the_plain_property_not_the_mangled_name() {
    let dir = ScratchDir::new("707_setter_frame").expect("failed to create scratch dir");
    let err = assert_traceback_matches_cpython(
        &dir,
        "setter_frame.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        raise ValueError(\"bad\")\n\nc = C()\nc.x = 1\n",
    );
    assert!(err.contains(", in x\n"), "{err}");
}

// -- With the source file gone, a frame shows no source lines, as CPython's
// -- does when it cannot read the file.

#[test]
fn a_frame_whose_source_is_gone_shows_no_source_lines() {
    let dir = ScratchDir::new("707_source_gone").expect("failed to create scratch dir");
    let src = dir.join("gone.py");
    std::fs::write(
        &src,
        "def boom() -> None:\n    raise ValueError(\"bad\")\n\nboom()\n",
    )
    .unwrap();
    let out = dir.join("gone");
    let build = Command::new(pycc_bin())
        .current_dir(&*dir)
        .args(["build", "gone.py", "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(build.status.success());
    std::fs::remove_file(&src).unwrap();
    let run = Command::new(&out).output().unwrap();
    // CPython names the script by its absolute path, symlinks resolved
    // (`/private/var` on macOS), as the build does.
    let base = if cfg!(windows) {
        dir.to_path_buf()
    } else {
        std::fs::canonicalize(&*dir).unwrap()
    };
    let path = base.join("gone.py");
    let path = path.display();
    assert_eq!(
        String::from_utf8_lossy(&run.stderr),
        format!(
            "Traceback (most recent call last):\n  File \"{path}\", line 4, in <module>\n  \
             File \"{path}\", line 2, in boom\nValueError: bad\n"
        )
    );
}
