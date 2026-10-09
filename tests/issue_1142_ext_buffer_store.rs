//! Part 1 of #1142: a bounds-checked native element **store** into a
//! buffer, exercised end to end.
//!
//! Part 2 of #1027 (#1113) admitted `b[i]` as a load and left `b[i] = v`
//! refused, which kept the `--ext` wrapper's `PyObject_GetBuffer` request
//! read-only. This part adds the store, and with it the first reason the
//! boundary ever asks for `PyBUF_WRITABLE` -- for exactly the parameters
//! whose own body stores into them, and no others.
//!
//! This file owns the source-level halves no in-crate unit test can reach:
//! the diagnostics a real program gets at the index and value positions,
//! the refusal that still covers every other use of the name, and the
//! hosted arms that prove a compiled store reaches the host's own memory
//! and that a read-only exporter is refused rather than scribbled over.
//!
//! None of these contributes line coverage: CI's coverage job runs
//! `llvm-cov` without `--include-ignored` and `scripts/check_diff_coverage.py`
//! excludes `tests/` from its denominator either way.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `source` as the entry module of a fresh scratch directory.
fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("store_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("store_probe.py"))
        .arg("-o")
        .arg(dir.join("store_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// Runs `program` under CPython with `dir` on the import path.
fn run_hosted(dir: &Path, program: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(program)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// The subject: one export that stores through its buffer parameter from
/// inside a loop, one that only reads, and one that stores at a
/// caller-supplied index.
///
/// `total` is the arm that matters most for the writability bit: it is a
/// read-only export in the same module as a storing one, so a flag applied
/// per module rather than per parameter would show up here as a read-only
/// exporter being refused.
const SUBJECT: &str = "\
def scale(b: memoryview, k: float) -> None:
    i = 0
    while i < len(b):
        b[i] = b[i] * k
        i = i + 1


def total(b: memoryview) -> float:
    s = 0.0
    i = 0
    while i < len(b):
        s = s + b[i]
        i = i + 1
    return s


def put(b: memoryview, i: int, v: float) -> None:
    b[i] = v
";

/// The index position is the load's, with the load's own diagnostic: the
/// store reuses `check_buffer_set`'s index rule rather than inventing a
/// second one, so `b["k"] = 1.0` and `b["k"]` read identically.
#[test]
fn a_non_int_store_index_is_refused_as_an_index_type_error() {
    let dir = fixture(
        "1142_index_type",
        "\
def put(b: memoryview, k: str) -> None:
    b[k] = 1.0
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("`memoryview` index must be `int`, found `str`"),
        "{err}"
    );
    // Not the "base does not support indexing" refusal a non-`dict` store
    // target gets: the base is a store target the checker now admits.
    assert!(!err.contains("error[T0033]"), "{err}");
}

/// The value position is `float` exactly, because D-086 grants no
/// int-to-float widening -- so `b[0] = 1` is a diagnostic rather than a
/// silent conversion, and codegen sees one scalar shape at that position.
#[test]
fn a_non_float_stored_value_is_refused() {
    for (source, rendered) in [
        ("def put(b: memoryview) -> None:\n    b[0] = 1\n", "`int`"),
        (
            "def put(b: memoryview) -> None:\n    b[0] = \"v\"\n",
            "`str`",
        ),
    ] {
        let dir = fixture("1142_value_type", source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains("error[T0021]"), "{err}");
        assert!(
            err.contains("to a `memoryview` element of `float`"),
            "{err}"
        );
        assert!(err.contains(rendered), "{err}");
    }
}

/// A store target that is neither a `dict` nor a buffer is untouched:
/// `T0033` is still what a `set` gets, so the surface this part opens is
/// exactly one type wide.
#[test]
fn a_store_into_another_container_is_unchanged() {
    let dir = fixture(
        "1142_other_target",
        "\
def put(xs: set[int]) -> None:
    xs[0] = 1
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    assert!(
        stderr_of(&build).contains("error[T0033]"),
        "{}",
        stderr_of(&build)
    );
}

/// Every *other* use of the name is still the Part 1 capability gap, and
/// the reworded message now names the store alongside the load.
#[test]
fn every_other_use_of_a_buffer_name_is_still_a_capability_gap() {
    let dir = fixture(
        "1142_alias",
        "\
def put(b: memoryview) -> None:
    x = b
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("using `b`, which is bound to a buffer parameter"),
        "{err}"
    );
    assert!(err.contains("store one with `b[i] = 1.0`"), "{err}");
}

/// A native build still refuses the *signature* with `I0405`, exactly as
/// it did before the store existed: admitting a second expression over a
/// buffer must not move the artifact-mode gate.
#[test]
fn a_native_build_still_refuses_the_signature_carrying_a_store() {
    let dir = fixture("1142_native", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("store_probe.py"))
        .arg("-o")
        .arg(dir.join("store_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
}

/// The whole point of the part, against a real interpreter: a compiled
/// store lands in the host's own memory, an out-of-range store raises
/// `IndexError` and writes nothing, and a read-only export in the same
/// module still accepts a read-only exporter.
///
/// Negative indices are refused rather than wrapped (D-108), the same
/// deviation from CPython the load carries, so `-1` is asserted explicitly.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's interpreter
/// is older.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_compiled_buffer_store_reaches_host_memory_and_raises_past_the_end() {
    let dir = fixture("1142_hosted_store", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import array, store_probe\n\
         data = array.array('d', [1.5, -2.25, 3.0])\n\
         store_probe.scale(memoryview(data), 2.0)\n\
         assert list(data) == [3.0, -4.5, 6.0], list(data)\n\
         store_probe.put(memoryview(data), 1, 0.25)\n\
         assert list(data) == [3.0, 0.25, 6.0], list(data)\n\
         assert store_probe.total(memoryview(data)) == 9.25\n\
         before = list(data)\n\
         for bad in (3, 4, -1, -4):\n\
         \x20   try:\n\
         \x20       store_probe.put(memoryview(data), bad, 99.0)\n\
         \x20   except IndexError as error:\n\
         \x20       assert 'out of bounds' in str(error), str(error)\n\
         \x20   else:\n\
         \x20       raise AssertionError('the store accepted %r' % (bad,))\n\
         assert list(data) == before, (list(data), before)\n\
         empty = memoryview(array.array('d', []))\n\
         try:\n\
         \x20   store_probe.put(empty, 0, 1.0)\n\
         except IndexError:\n\
         \x20   pass\n\
         else:\n\
         \x20   raise AssertionError('an empty view accepted index 0')\n\
         # The read-only export is unaffected: its parameter is still\n\
         # acquired without `PyBUF_WRITABLE`, so an immutable exporter is\n\
         # accepted exactly as it was before this part.\n\
         assert store_probe.total(memoryview(bytes(24)).cast('d')) == 0.0\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// A read-only exporter handed to a *storing* export is refused at the
/// boundary, by CPython's own `BufferError`, before any element is written.
///
/// This is the memory-safety half of the part. Without the per-parameter
/// `PyBUF_WRITABLE` request, `PyObject_GetBuffer` would hand back a
/// pointer into immutable storage and the compiled store would write
/// through it. The exception text is CPython's and is propagated verbatim,
/// so only its type and the untouched memory are asserted.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_read_only_exporter_is_refused_by_a_storing_export() {
    let dir = fixture("1142_hosted_readonly", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import store_probe\n\
         frozen = bytes(24)\n\
         for call in (\n\
         \x20   lambda: store_probe.scale(memoryview(frozen).cast('d'), 2.0),\n\
         \x20   lambda: store_probe.put(memoryview(frozen).cast('d'), 0, 1.0),\n\
         ):\n\
         \x20   try:\n\
         \x20       call()\n\
         \x20   except BufferError:\n\
         \x20       pass\n\
         \x20   else:\n\
         \x20       raise AssertionError('a read-only exporter was accepted')\n\
         assert frozen == bytes(24), frozen\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// A store through a **constructor**'s buffer parameter, which
/// `collect_exports` never sees: `__init__` is refused as an export, so
/// `Py_tp_init` builds its own slot vector and would acquire read-only
/// storage this body writes through if the bit were plumbed only for
/// exports.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_constructor_buffer_parameter_is_acquired_writable_too() {
    let dir = fixture(
        "1142_hosted_ctor",
        "\
class Grid:
    def __init__(self, b: memoryview, k: float) -> None:
        b[0] = k
        self.k = k

    def factor(self) -> float:
        return self.k
",
    );
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import array, store_probe\n\
         data = array.array('d', [1.0, 2.0])\n\
         grid = store_probe.Grid(memoryview(data), 7.5)\n\
         assert list(data) == [7.5, 2.0], list(data)\n\
         assert grid.factor() == 7.5\n\
         frozen = bytes(16)\n\
         try:\n\
         \x20   store_probe.Grid(memoryview(frozen).cast('d'), 1.0)\n\
         except BufferError:\n\
         \x20   pass\n\
         else:\n\
         \x20   raise AssertionError('a read-only exporter reached the constructor')\n\
         assert frozen == bytes(16), frozen\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// An out-of-range store as a function's **last** statement still raises:
/// the store is a void statement, so its only exception path is the
/// `guard_statement_effects` codegen emits after the runtime call. Dropping
/// that guard -- which `MirStmt::DictSet`, the arm this lowering sits
/// beside, does not emit -- would let the function return normally with a
/// pending `IndexError`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_out_of_range_store_as_the_last_statement_still_raises() {
    let dir = fixture(
        "1142_hosted_tail",
        "\
def tail(b: memoryview) -> None:
    b[len(b)] = 1.0
",
    );
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import array, store_probe\n\
         data = array.array('d', [1.0])\n\
         try:\n\
         \x20   store_probe.tail(memoryview(data))\n\
         except IndexError as error:\n\
         \x20   assert 'out of bounds' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('the tail store returned normally')\n\
         assert list(data) == [1.0], list(data)\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// CPython's assignment order, observed from a hosted run: for `b[i] = v`
/// the right-hand side is evaluated **first**, then the subscription
/// target, then the index. A `d[i()] = v()` probe on the reference
/// interpreter prints `v`'s effect before `i`'s, and the compiled store
/// must agree -- otherwise a raising or side-effecting index expression
/// suppresses a value expression CPython would already have run.
///
/// Both positions are calls whose effect is a `print`, so the order is read
/// straight off the run's stdout. Nothing else writes to it: the driver
/// prints nothing of its own, so CPython's own buffering cannot interleave
/// with the compiled writes.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_store_evaluates_its_value_before_its_index() {
    let dir = fixture(
        "1142_hosted_order",
        "\
def value() -> int:
    print(\"value\")
    return 1


def position() -> int:
    print(\"index\")
    return 0


def order(b: memoryview) -> None:
    b[position()] = float(value())
",
    );
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import array, store_probe\n\
         data = array.array('d', [0.0])\n\
         store_probe.order(memoryview(data))\n\
         assert list(data) == [1.0], list(data)\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "value\nindex\n");
}

/// A bigint index is refused at the `ext` boundary by D-141's own
/// `OverflowError`, before the store's untag ever runs -- the index
/// position is an ordinary `int` parameter, so it inherits that rule
/// rather than restating it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bigint_store_index_is_the_d141_boundary_overflow() {
    let dir = fixture("1142_hosted_bigint", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import array, store_probe\n\
         data = array.array('d', [1.0])\n\
         try:\n\
         \x20   store_probe.put(memoryview(data), 1 << 70, 2.0)\n\
         except OverflowError as error:\n\
         \x20   assert 'inline-integer range' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('a bigint index was accepted')\n\
         assert list(data) == [1.0], list(data)\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// Every *other* syntactic way to write into a buffer element is still
/// refused, which is what makes the writability walk's single
/// `HirStmt::DictSet` arm sufficient.
///
/// The walk that decides `PyBUF_WRITABLE` is syntactic and looks for that
/// one node. A store shape the checker admitted but the walk did not see
/// would acquire read-only storage the compiled body then writes through
/// -- so the shapes below are asserted to be refused *before* type
/// checking reaches them, rather than assumed.
///
/// An augmented store `b[i] += x` is not among them: since #1209 it is
/// lowered as the plain `b[i] = b[i] + x`, which *is* that one
/// `HirStmt::DictSet` node, so the walk sees it and acquires the parameter
/// writable (pinned hosted in `tests/issue_1018_aug_assign.rs`). Its slice
/// form stays refused, and is listed here. A chained store `b[0] = b[1] = v`
/// is not among them either: since #1213 it is lowered as one `DictSet` per
/// target (pinned hosted in `tests/issue_1213_chain_assign.rs`).
#[test]
fn no_other_store_syntax_reaches_a_buffer_element() {
    for source in [
        // An augmented store to a slice.
        "def f(b: memoryview) -> None:\n    b[0:2] += 1.0\n",
        // A tuple target.
        "def f(b: memoryview) -> None:\n    b[0], b[1] = 1.0, 2.0\n",
        // A slice target.
        "def f(b: memoryview) -> None:\n    b[0:2] = [1.0, 2.0]\n",
    ] {
        let dir = fixture("1142_other_store_syntax", source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{source}: {}", stdout_of(&build));
        assert!(
            stderr_of(&build).contains("error[C0001]"),
            "{source}: {}",
            stderr_of(&build)
        );
    }
}
