//! End-to-end coverage for #707: uncaught exceptions print a traceback frame
//! header (`Traceback (most recent call last):` / `File "<compiled>", in
//! <frame>`) naming the function the `raise` executed in, and a `raise ...
//! from ...` chain renders CPython's "The above exception was the direct
//! cause of the following exception:" separator between cause and effect,
//! oldest cause first.
//!
//! The conformance oracle in `tests/conformance.rs` only diffs `stdout` of
//! two successful (exit-0) runs, so it cannot compare an uncaught-exception
//! program's `stderr` against CPython's own traceback text (see
//! `run_conformance_fixture_with_profile`). These tests instead build and
//! run the pycc-compiled binary directly and assert on its `stderr`, the
//! same pattern `tests/issue_382_exceptions.rs`'s
//! `uncaught_exception_exits_nonzero` already uses for the pre-#707 one-line
//! format.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

fn build_and_run(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, Vec<u8>, String) {
    let src = write_fixture(dir, src_name, source);
    let out = dir.join(src_name.replace(".py", ""));
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        );
    }
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        run.stdout.clone(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

// -- A module-level uncaught raise names the `<module>` frame --

#[test]
fn a_module_level_uncaught_raise_shows_a_module_frame() {
    let dir = ScratchDir::new("707_module_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(&dir, "module_frame.py", "raise ValueError(\"boom\")\n");
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in <module>\nValueError: boom\n"
    );
}

// -- A raise inside a function names that function's frame, not `<module>` --

#[test]
fn a_raise_inside_a_function_names_that_functions_frame() {
    let dir = ScratchDir::new("707_function_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "function_frame.py",
        "def boom() -> None:\n    raise ValueError(\"bad\")\n\nboom()\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in boom\nValueError: bad\n"
    );
}

// A zero-argument builtin-exception constructor (`ValueError()`) is rejected
// by `pycc_types`' T0021 check (exactly one message argument is required),
// so the "no recorded message" rendering has no reachable source-level
// trigger; `render_single_exception_with_no_message_omits_the_colon` in
// `crates/pycc_rt/src/exception.rs` covers it directly at the runtime layer
// instead.

// -- `raise ... from ...` renders the direct-cause chain, oldest cause first.
// -- The cause here is a freshly `Constructed` value (`TypeError("cause")`)
// -- that is never itself raised, so -- matching the codegen decision to call
// -- `emit_exception_set_frame` only on the raised exception, not on an
// -- inline-constructed cause -- it carries no recorded frame and renders
// -- without a `Traceback` header, exactly like
// -- `render_exception_chain_with_no_cause_matches_render_single_exception`'s
// -- unframed case in `crates/pycc_rt/src/exception.rs`. -- A manual run of
// -- this exact program through a local `python3.14` (3.14.6) oracle during
// -- development confirmed CPython does the same: no `Traceback` header for
// -- the unraised cause, and the "direct cause" separator text below is
// -- byte-for-byte what CPython prints. This manual oracle run is not wired
// -- into the automated `tests/conformance.rs` oracle, which only diffs
// -- successful-exit stdout; the unit tests in `crates/pycc_rt/src/exception.rs`
// -- (e.g. `render_exception_chain_with_no_cause_matches_render_single_exception`)
// -- and this test file are the source of truth for the exact rendering.

#[test]
fn raise_from_renders_the_direct_cause_chain_oldest_first() {
    let dir = ScratchDir::new("707_raise_from_chain").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "raise_from_chain.py",
        "raise ValueError(\"effect\") from TypeError(\"cause\")\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "TypeError: cause\n\n\
The above exception was the direct cause of the following exception:\n\n\
Traceback (most recent call last):\n  File \"<compiled>\", in <module>\nValueError: effect\n"
    );
}

// -- A multi-level cause chain (`raise ... from ...` whose cause is itself the
// -- product of a nested `raise ... from ...`) walks every level, oldest first --

#[test]
fn a_multi_level_cause_chain_walks_every_level() {
    let dir = ScratchDir::new("707_multi_level_chain").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "multi_level_chain.py",
        "def inner() -> None:\n    try:\n        raise KeyError(\"root\")\n    except KeyError as e:\n        raise TypeError(\"middle\") from e\n\ntry:\n    inner()\nexcept TypeError as e:\n    raise ValueError(\"outer\") from e\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in inner\nKeyError: root\n\n\
The above exception was the direct cause of the following exception:\n\n\
Traceback (most recent call last):\n  File \"<compiled>\", in inner\nTypeError: middle\n\n\
The above exception was the direct cause of the following exception:\n\n\
Traceback (most recent call last):\n  File \"<compiled>\", in <module>\nValueError: outer\n"
    );
}

// -- A `raise` inside a class method, classmethod, staticmethod, or property
// -- setter names the plain Python source name of that method -- never
// -- `pycc_hir`'s mangled internal identifier (`ClassName.method_name`, with
// -- a further `.classmethod`/`.static`/`.setter` suffix), which would leak
// -- an implementation detail into a compiled program's stderr and does not
// -- match what CPython's own traceback shows for the frame (CPython prints
// -- a method's plain `co_name`, e.g. `create`, never `C.create`).

#[test]
fn a_raise_inside_a_plain_method_names_the_plain_method() {
    let dir = ScratchDir::new("707_method_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "method_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def boom(self) -> None:\n        raise ValueError(\"bad\")\n\nC().boom()\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in boom\nValueError: bad\n",
        "a method frame must show the plain method name, not the mangled `C.boom`"
    );
}

#[test]
fn a_raise_inside_a_classmethod_names_the_plain_method_not_the_mangled_name() {
    let dir = ScratchDir::new("707_classmethod_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "classmethod_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def create(cls) -> None:\n        raise ValueError(\"bad\")\n\nC.create()\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in create\nValueError: bad\n",
        "a classmethod frame must show `create`, not the mangled `C.create.classmethod`"
    );
}

#[test]
fn a_raise_inside_a_staticmethod_names_the_plain_method_not_the_mangled_name() {
    let dir = ScratchDir::new("707_staticmethod_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "staticmethod_frame.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def make() -> None:\n        raise ValueError(\"bad\")\n\nC.make()\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err,
        "Traceback (most recent call last):\n  File \"<compiled>\", in make\nValueError: bad\n",
        "a staticmethod frame must show `make`, not the mangled `C.make.static`"
    );
}

#[test]
fn a_raise_inside_a_property_setter_names_the_plain_property_not_the_mangled_name() {
    let dir = ScratchDir::new("707_setter_frame").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "setter_frame.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        raise ValueError(\"bad\")\n\nc = C()\nc.x = 1\n",
    );
    assert!(!ok, "expected non-zero exit for uncaught exception");
    assert_eq!(
        err, "Traceback (most recent call last):\n  File \"<compiled>\", in x\nValueError: bad\n",
        "a property-setter frame must show `x`, not the mangled `C.x.setter`"
    );
}
