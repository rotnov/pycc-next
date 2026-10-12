//! Typed numpy arrays (ROADMAP v0.2, accept item 2): `NDArray[np.float64]`
//! and friends compiled to native element access (`pycc_hir::ndarray`).
//!
//! The oracle fixture `tests/fixtures/numpy_typed_arrays.py` exercises
//! creation calls, reads and writes through `a[i]`, `a[i, j]` and
//! `a[i][j]`, negative indices, `.shape`/`.size`/`.ndim`/`len`, iteration,
//! `int64` values beyond pycc's inline integer range, `bool` arrays, an
//! array attribute of a class, and numpy's own `IndexError`, `OverflowError`
//! and `ValueError` texts. It must print exactly what CPython prints, as an
//! `--ext` module and as an embedded executable.
//!
//! The hosted tests need numpy importable by the interpreter `pycc` builds
//! against (`PYCC_PYTHON`, default `python3`); CI installs a hash-pinned
//! numpy (`.github/numpy-requirements.txt`). Without numpy they print why
//! and return. The `pycc check` tests at the top need no interpreter.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const FIXTURE: &str = include_str!("fixtures/numpy_typed_arrays.py");

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python_bin() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    let mut out = text(&output.stdout);
    out.push_str(&text(&output.stderr));
    out
}

fn numpy_is_importable() -> bool {
    let ok = Command::new(python_bin())
        .args(["-c", "import numpy"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: numpy is not importable by the hosted interpreter");
    }
    ok
}

fn python_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(python_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// Runs `pycc check` on `source`, returning the rendered output and status.
fn check(source: &str) -> (bool, String) {
    let dir = ScratchDir::new("numpy-typed-check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    (output.status.success(), rendered(&output))
}

const HEADER: &str = "import numpy as np\nfrom numpy.typing import NDArray\n\n\n";

#[test]
fn the_dimension_count_is_inferred_from_two_index_subscripts() {
    let (ok, out) = check(&format!(
        "{HEADER}def f(a: NDArray[np.float64]) -> float:\n    return a[0, 1] + a[1][0]\n"
    ));
    assert!(ok, "{out}");
}

#[test]
fn an_index_must_be_an_int_and_not_a_bool() {
    for index in ["1.5", "True", "\"x\""] {
        let (ok, out) = check(&format!(
            "{HEADER}def f(a: NDArray[np.float64]) -> float:\n    return a[{index}]\n"
        ));
        assert!(!ok, "{index}: {out}");
        assert!(
            out.contains("`ndarray[float64, 1-D]` index must be `int`"),
            "{index}: {out}"
        );
    }
}

#[test]
fn a_store_must_fit_the_element_type() {
    for (dtype, value, ok_expected) in [
        ("np.int64", "1.5", false),
        ("np.int64", "True", true),
        ("np.float64", "3", true),
        ("np.bool_", "1", false),
        ("np.bool_", "False", true),
    ] {
        let (ok, out) = check(&format!(
            "{HEADER}def f(a: NDArray[{dtype}]) -> None:\n    a[0] = {value}\n"
        ));
        assert_eq!(ok, ok_expected, "{dtype} <- {value}: {out}");
        if !ok_expected {
            assert!(out.contains("cannot store"), "{out}");
        }
    }
}

#[test]
fn a_three_dimensional_annotation_is_refused() {
    let (ok, out) = check(&format!(
        "{HEADER}def f(a: np.ndarray[tuple[int, int, int], np.dtype[np.float64]]) -> None:\n    pass\n"
    ));
    assert!(!ok, "{out}");
    assert!(
        out.contains("typed numpy arrays have 1 or 2 dimensions, not 3"),
        "{out}"
    );
}

#[test]
fn an_attribute_base_names_its_dtype() {
    let (ok, out) = check(&format!(
        "{HEADER}def f(a: np.ndarray[int]) -> None:\n    pass\n"
    ));
    assert!(!ok, "{out}");
    assert!(
        out.contains("a typed numpy array annotation names its dtype"),
        "{out}"
    );
}

/// The fixture as an `--ext` module: importing it runs `main()`, whose
/// output must be CPython's for the same source.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers and numpy"]
fn the_oracle_fixture_matches_cpython_as_an_ext_module() {
    if !numpy_is_importable() {
        return;
    }
    let dir = ScratchDir::new("numpy-typed-ext").expect("scratch");
    let source = dir.join("nta.py");
    std::fs::write(&source, FIXTURE).expect("write fixture");
    let out_dir = dir.join("ext");
    std::fs::create_dir_all(&out_dir).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(out_dir.join("nta"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let expected = python_in(&dir, &["nta.py"]);
    assert!(expected.status.success(), "{}", rendered(&expected));
    let actual = python_in(&out_dir, &["-c", "import nta"]);
    assert!(actual.status.success(), "{}", rendered(&actual));
    assert_eq!(text(&actual.stdout), text(&expected.stdout));
}

/// What the `--ext` boundary does with arrays from Python: a write is
/// visible to the caller, a strided view is indexed through its strides,
/// and a value that is not an array of the declared dtype and dimension
/// count is refused at the call with `TypeError` (the check happens at the
/// boundary, where CPython would fail only at the first operation, if at
/// all -- `docs/DEVIATIONS.md`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers and numpy"]
fn the_ext_boundary_checks_dtype_and_dimensions_and_shares_the_array() {
    if !numpy_is_importable() {
        return;
    }
    let dir = ScratchDir::new("numpy-typed-boundary").expect("scratch");
    std::fs::write(
        dir.join("ndb.py"),
        format!(
            "{HEADER}def fill(g: NDArray[np.float64]) -> None:\n    n, m = g.shape\n    \
             for i in range(n):\n        for j in range(m):\n            g[i, j] = i * 10.0 + j\n\n\n\
             def total(a: NDArray[np.int64]) -> int:\n    t = 0\n    for x in a:\n        t += x\n    \
             return t\n\n\ndef doubled(a: NDArray[np.int64]) -> NDArray[np.int64]:\n    \
             out = np.zeros(len(a), dtype=np.int64)\n    for i in range(len(a)):\n        \
             out[i] = 2 * a[i]\n    return out\n\n\nclass Holder:\n    cells: NDArray[np.float64]\n\n    \
             def __init__(self, n: int) -> None:\n        self.cells = np.ones((n, n))\n\n    \
             def corner(self) -> float:\n        return self.cells[0, 0] + self.cells[-1][-1]\n"
        ),
    )
    .expect("write subject");
    std::fs::create_dir_all(dir.join("ext")).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(dir.join("ndb.py"))
        .arg("-o")
        .arg(dir.join("ext").join("ndb"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let script = "import numpy as np, ndb\n\
        g = np.zeros((2, 3)); ndb.fill(g); print(g.tolist())\n\
        v = np.zeros((4, 6))[::2, 1::2]; ndb.fill(v); print(v.tolist())\n\
        print(ndb.total(np.arange(5)), ndb.doubled(np.arange(4)).tolist())\n\
        for bad in [np.arange(5.0), [1, 2], np.zeros((2, 2), dtype=np.int64), np.zeros(3, dtype=np.int32)]:\n\
        \x20   try:\n\
        \x20       ndb.total(bad)\n\
        \x20   except TypeError as e:\n\
        \x20       print('TypeError:', e)\n\
        h = ndb.Holder(2); print(h.corner())\n\
        h.cells = np.full((3, 3), 2.0); print(h.corner())\n\
        for bad in [[1.0], np.zeros(3)]:\n\
        \x20   try:\n\
        \x20       h.cells = bad\n\
        \x20   except TypeError as e:\n\
        \x20       print('TypeError:', e)\n\
        print(h.corner())\n";
    let output = python_in(&dir.join("ext"), &["-c", script]);
    assert!(output.status.success(), "{}", rendered(&output));
    assert_eq!(
        text(&output.stdout),
        "[[0.0, 1.0, 2.0], [10.0, 11.0, 12.0]]\n\
         [[0.0, 1.0, 2.0], [10.0, 11.0, 12.0]]\n\
         10 [0, 2, 4, 6]\n\
         TypeError: expected numpy.ndarray[int64, 1-D], got an array of dtype float64\n\
         TypeError: expected numpy.ndarray[int64, 1-D], got list\n\
         TypeError: expected numpy.ndarray[int64, 1-D], got a 2-D array\n\
         TypeError: expected numpy.ndarray[int64, 1-D], got an array of dtype int32\n\
         2.0\n\
         4.0\n\
         TypeError: expected numpy.ndarray[float64, 2-D], got list\n\
         TypeError: expected numpy.ndarray[float64, 2-D], got a 1-D array\n\
         4.0\n"
    );
}

/// The fixture as an embedded executable (numpy bundled from `pycc.lock`).
/// Linux only: the bundle of numpy's native closure is exercised on the
/// platform CI builds it on.
#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires a CPython 3.13+ with a shared libpython and numpy"]
fn the_oracle_fixture_matches_cpython_as_an_embedded_executable() {
    if !numpy_is_importable() {
        return;
    }
    let dir = ScratchDir::new("numpy-typed-native").expect("scratch");
    let source = dir.join("nta.py");
    std::fs::write(&source, FIXTURE).expect("write fixture");
    let lock = pycc()
        .arg("lock")
        .arg(&source)
        .output()
        .expect("pycc should spawn");
    assert!(lock.status.success(), "{}", rendered(&lock));
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("nta"))
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let expected = python_in(&dir, &["nta.py"]);
    let actual = Command::new(dir.join("nta"))
        .output()
        .expect("program should spawn");
    assert!(actual.status.success(), "{}", rendered(&actual));
    assert_eq!(text(&actual.stdout), text(&expected.stdout));
}

/// An uncaught out-of-range index ends the program with numpy's message
/// and exit status 1, as CPython does.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers and numpy"]
fn an_uncaught_index_error_carries_numpys_message() {
    if !numpy_is_importable() {
        return;
    }
    let dir = ScratchDir::new("numpy-typed-uncaught").expect("scratch");
    std::fs::write(
        dir.join("ndu.py"),
        format!("{HEADER}def get(a: NDArray[np.float64], i: int) -> float:\n    return a[i]\n"),
    )
    .expect("write subject");
    std::fs::create_dir_all(dir.join("ext")).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(dir.join("ndu.py"))
        .arg("-o")
        .arg(dir.join("ext").join("ndu"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let output = python_in(
        &dir.join("ext"),
        &["-c", "import numpy as np, ndu; ndu.get(np.zeros(3), -4)"],
    );
    assert_eq!(output.status.code(), Some(1));
    let err = text(&output.stderr);
    assert!(
        err.trim_end()
            .ends_with("IndexError: index -4 is out of bounds for axis 0 with size 3"),
        "{err}"
    );
}

/// An array that CPython code changes while compiled code holds it is
/// re-checked before the next element access, and a loop variable typed as
/// an array is checked at each binding (`tests/fixtures/
/// numpy_array_safety.py`). Each case once read through a stale or
/// never-checked header: freed memory after `resize(refcheck=False)`, a
/// bool buffer as `float64` after `__setstate__`, a float or a row as an
/// array. Run under `PYTHONMALLOC=debug` so a read of freed memory shows.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers and numpy"]
fn an_array_changed_by_cpython_code_is_rechecked() {
    if !numpy_is_importable() {
        return;
    }
    let dir = ScratchDir::new("numpy-typed-safety").expect("scratch");
    std::fs::write(
        dir.join("ndsafe.py"),
        include_str!("fixtures/numpy_array_safety.py"),
    )
    .expect("write subject");
    std::fs::create_dir_all(dir.join("ext")).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(dir.join("ndsafe.py"))
        .arg("-o")
        .arg(dir.join("ext").join("ndsafe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let script = "import numpy as np, ndsafe\n\
        def shrink(a):\n\
        \x20   a.resize(1, refcheck=False)\n\
        def to_bool(a):\n\
        \x20   a.__setstate__(np.zeros(2, dtype=np.bool_).__reduce__()[2])\n\
        def grow(a):\n\
        \x20   a.resize(8, refcheck=False)\n\
        for call in [lambda: ndsafe.after_hook(np.arange(4.0), shrink),\n\
        \x20            lambda: ndsafe.after_hook(np.arange(4.0), to_bool),\n\
        \x20            lambda: ndsafe.after_hook(np.arange(4.0), grow),\n\
        \x20            lambda: ndsafe.element_as_array(np.arange(3.0)),\n\
        \x20            lambda: ndsafe.row_as_matrix(np.ones((2, 2)))]:\n\
        \x20   try:\n\
        \x20       print(call())\n\
        \x20   except (IndexError, TypeError) as e:\n\
        \x20       print(type(e).__name__ + ':', e)\n";
    let output = Command::new(python_bin())
        .args(["-c", script])
        .current_dir(dir.join("ext"))
        .env("PYTHONMALLOC", "debug")
        .output()
        .expect("python should spawn");
    assert!(output.status.success(), "{}", rendered(&output));
    assert_eq!(
        text(&output.stdout),
        "IndexError: index 3 is out of bounds for axis 0 with size 1\n\
         TypeError: expected numpy.ndarray[float64, 1-D], got an array of dtype bool\n\
         3.0\n\
         TypeError: expected numpy.ndarray[float64, 1-D], got float\n\
         TypeError: expected numpy.ndarray[float64, 2-D], got a 1-D array\n"
    );
}
