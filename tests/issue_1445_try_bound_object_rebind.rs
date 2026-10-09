//! #1445: `s = []` beside an object binding `s = value_stack[-size:]` whose
//! slice bound derives from a name bound inside a `try` suite -- lark
//! `lalr_parser_state.py`'s `feed_token`, lines 74-99 (`action, arg = ...`
//! inside `try`/`except KeyError`, `size = len(rule.expansion)`, then the
//! two bindings of `s`).
//!
//! Part 2d of #1371 already types such an `s = []` as a fresh CPython
//! `list` (D-258's Part 2d amendment, source (b)). What was missing is the
//! evidence: the flat whole-function binder the empty-container pass reads
//! had no `try` arm, so `arg`, `size` and the slice never typed and `s`
//! was never bound to the object. Every hosted test builds the source with
//! `pycc build --ext`, imports the artifact into the host CPython, and
//! compares its output with CPython's own run of the same source. The
//! hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered in-process by
//! `crates/pycc_types/src/tests/generic_monomorphization_arms.rs` and
//! `crates/pycc_types/src/foreign/list_display/tests.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    command.env("PYTHONIOENCODING", "utf-8");
    command
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn build(dir: &Path, output: &str, body: &str, ext: bool) {
    let mut command = pycc();
    command
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(output));
    if ext {
        command.arg("--ext");
    }
    let build = command.output().expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Lark's shape with the parse table reduced to a mapping from a state to
/// its rule: `states[state]` raising `KeyError` is re-raised as a
/// `ValueError` from the handler, `size` is derived from the try-bound
/// `arg`, and `s` is either the object slice or the empty display. The
/// callback gets `s` exactly as lark's `callbacks[rule](s)` does.
const SUBJECT: &str = "def reduce(value_stack: object, states: object, state: int, callbacks: object) -> object:\n    \
    try:\n        arg = states[state]\n    except KeyError:\n        \
    raise ValueError('no rule')\n    \
    size = len(arg)\n    \
    if size:\n        s = value_stack[-size:]\n        del value_stack[-size:]\n    \
    else:\n        s = []\n    \
    value = callbacks(s) if callbacks else s\n    \
    return value\n";

/// Both branches, the callback and the no-callback spelling, a fresh list
/// per evaluation of the empty display, and the handler's re-raise.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_try_bound_lark_shape_behaves_like_cpython_in_the_host() {
    let dir = ScratchDir::new("issue_1445_hosted").expect("scratch");
    build(&dir, "pycc_issue_1445", SUBJECT, true);
    let driver = |module: &str| {
        format!(
            "{module}\n\
             states = {{0: 'ab', 1: ''}}\n\
             stack = list('wxyz')\n\
             print(m.reduce(stack, states, 0, None), stack)\n\
             print(m.reduce(stack, states, 1, None), stack)\n\
             print(m.reduce(stack, states, 0, len), stack)\n\
             print(m.reduce(stack, states, 1, tuple), stack)\n\
             a = m.reduce(stack, states, 1, None)\n\
             b = m.reduce(stack, states, 1, None)\n\
             print(type(a).__name__, a is b)\n\
             try:\n    m.reduce(stack, states, 7, None)\n\
             except ValueError as e:\n    print('ValueError', e)\n"
        )
    };
    let compiled = python(&dir, &driver("import pycc_issue_1445 as m"));
    assert_ok(&compiled);
    let oracle = python(
        &dir,
        &driver("import runpy, types\nm = types.SimpleNamespace(**runpy.run_path('m.py'))"),
    );
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(
        stdout_of(&compiled),
        "['y', 'z'] ['w', 'x']\n[] ['w', 'x']\n2 []\n() []\nlist False\nValueError no rule\n"
    );
}

/// Reference counts, measured on a mortal `object()` held three times in a
/// fresh stack list and on the `states` mapping, across 100 calls of each
/// branch with the result dropped by the host each time. CPython's own run
/// prints `0` everywhere.
///
/// The extension's deltas are the #1092 leak-only rule and nothing else
/// (`docs/RUNTIME.md`, "A function body adds no reference traffic of its
/// own": the only reference that leaks is the one each producer returns,
/// plus the export wrapper's one reference per object argument from the
/// `object` parameter row). Per call:
///
/// - `states` column, both branches: `100`, the argument reference
///   `pycc_ext_unpack_object` takes and never releases.
/// - probe column, state 0 (`size == 2`): `300`. The leaked stack argument
///   keeps the one probe the `del value_stack[-2:]` leaves (`100`), and the
///   slice `value_stack[-2:]` is a producer whose new list is leaked with
///   its two probes (`200`). Measured apart on the same build: a function
///   that only slices leaks `500` (the stack's three probes plus the
///   slice's two), one that only deletes leaks `100`.
/// - probe column, state 1 (`size == 0`): `300`, the leaked stack argument
///   keeping all three probes; the fresh list of `s = []` is leaked too but
///   holds no probe.
///
/// A doubly-leaked slice or an unbalanced packer would move one of the
/// columns.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_try_bound_lark_shape_keeps_reference_counts_like_cpython() {
    let dir = ScratchDir::new("issue_1445_refcount").expect("scratch");
    build(&dir, "pycc_issue_1445_rc", SUBJECT, true);
    let driver = |module: &str| {
        format!(
            "{module}\n\
             import sys\n\
             probe = object()\n\
             states = {{0: 'ab', 1: ''}}\n\
             for state in (0, 1):\n    \
             p0, s0 = sys.getrefcount(probe), sys.getrefcount(states)\n    \
             for _ in range(100):\n        \
             stack = [probe, probe, probe]\n        \
             m.reduce(stack, states, state, None)\n        \
             del stack\n    \
             print(state, sys.getrefcount(probe) - p0, sys.getrefcount(states) - s0)\n"
        )
    };
    let compiled = python(&dir, &driver("import pycc_issue_1445_rc as m"));
    assert_ok(&compiled);
    let oracle = python(
        &dir,
        &driver("import runpy, types\nm = types.SimpleNamespace(**runpy.run_path('m.py'))"),
    );
    assert_ok(&oracle);
    assert_eq!(stdout_of(&oracle), "0 0 0\n1 0 0\n");
    let compiled = stdout_of(&compiled);
    let lines: Vec<&str> = compiled.lines().collect();
    assert_eq!(lines[0], "0 300 100", "state 0: 100 stack + 200 slice");
    assert_eq!(lines[1], "1 300 100", "state 1: 300 stack, empty list");
    assert_eq!(lines.len(), 2, "{compiled}");
}

/// The native face of the same binder change: a list bound inside a `try`
/// suite now types an empty display of the same name in its handler
/// (`T0003` before). Expected output verified against CPython on this
/// exact source.
#[test]
fn a_native_empty_display_typed_from_a_try_bound_list_runs() {
    let dir = ScratchDir::new("issue_1445_native").expect("scratch");
    let body = "def f(n: int) -> int:\n    \
        try:\n        xs = [n, n + 1]\n        if n < 0:\n            raise ValueError('neg')\n    \
        except ValueError:\n        xs = []\n    return len(xs)\n\
        \n\
        \n\
        print(f(3))\n\
        print(f(-1))\n";
    build(&dir, "native", body, false);
    let run = Command::new(dir.join("native"))
        .output()
        .expect("the native binary should spawn");
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "2\n0\n");
}
