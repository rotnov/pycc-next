//! Part 3 of #1092 (#1498): the iterator of a `for` loop or a comprehension
//! over a CPython object, a comprehension's unbound result, and the unbound
//! produced operands of `print`, f-strings, `raise`, conditional expressions
//! and boolean operators are released exactly once.
//!
//! The probe follows `tests/issue_1092_object_temp_release.rs`: it compares
//! `sys.getrefcount` of every measured stub attribute before and after a
//! loop of the shapes, at two trip counts, and expects a delta of exactly
//! `0`. Every measured attribute is mortal (see that file on PEP 683).
//!
//! **Making the iterator visible.** An iterator is not a stub attribute, so
//! the stub's iterables (`SEQ`, `BOOM`) hand out iterators that keep a
//! reference to the iterable they came from: an iterator that is never
//! released keeps its iterable's count raised by one. The items are small
//! `int`s, which are immortal, so the per-trip item -- still unreleased
//! until #1499 -- moves no measured count. The positive control holds
//! iterators from the host and shows the probe sees exactly that.
//!
//! `BOOM`'s iterator yields once and then raises `ValueError`, exercising
//! `next()`'s failure edge; a body that raises exercises the loop's cleanup
//! target, nested and inside a module-level `try`; and two module bodies
//! that fail uncaught exercise the module-exec failure return.
//!
//! The hosted tests are `#[ignore]`d and contribute no line coverage; the
//! Tier-1 `native-build-test` leg runs them with `cargo test --workspace --
//! --include-ignored`. The changed lines are covered by the unit tests in
//! `crates/pycc_codegen/src/object_release_tests.rs` and
//! `crates/pycc_codegen/src/object_release_operand_tests.rs`.

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

const STUB: &str = "\
class Thing:
    def __init__(self, tag):
        self.tag = tag

    def __str__(self):
        return self.tag

    def __bool__(self):
        return True

    def fail(self):
        raise ValueError()


class Falsy:
    def __bool__(self):
        return False


class Counted:
    def __iter__(self):
        return CountedIter(self)


class CountedIter:
    def __init__(self, owner):
        self.owner = owner
        self.i = 0

    def __iter__(self):
        return self

    def __next__(self):
        if self.i >= 3:
            raise StopIteration
        self.i += 1
        return self.i


class Boom:
    def __iter__(self):
        return BoomIter(self)


class BoomIter:
    def __init__(self, owner):
        self.owner = owner
        self.done = False

    def __iter__(self):
        return self

    def __next__(self):
        if self.done:
            raise ValueError()
        self.done = True
        return 1


SEQ = Counted()
BOOM = Boom()
T = Thing('t')
U = Thing('u')
FALSY = Falsy()
EXC = ValueError()
";

/// The attributes whose reference counts the probe pins.
const MEASURED: &[&str] = &["SEQ", "BOOM", "T", "U", "FALSY", "EXC"];

/// The module under test. A `for` over an object is admitted only in a
/// module body, so its loops run on import, 100 trips of them; the
/// comprehensions and operand shapes live in functions the host calls.
const MODULE: &str = "\
import pycc_t1498_stub as s

x: float = 0.0
caught: int = 0
for i in range(100):
    for w in s.SEQ:
        x = 1.5
    for w in s.SEQ:
        for v in s.SEQ:
            x = 2.5
    try:
        for w in s.BOOM:
            x = 1.5
    except ValueError:
        caught += 1
    try:
        for w in s.SEQ:
            for v in s.SEQ:
                s.T.fail()
    except ValueError:
        caught += 1


def comps() -> int:
    [w for w in s.SEQ]
    return len([w for w in s.SEQ if w])


def comp_fails() -> int:
    caught = 0
    try:
        len([w for w in s.BOOM])
    except ValueError:
        caught += 1
    try:
        len([s.T.fail() for w in s.SEQ])
    except ValueError:
        caught += 1
    return caught


def operands() -> None:
    x: float = 0.0
    print(s.T, s.U)
    t = f\"<{s.T}>\"
    x = 1.5 if s.T else 2.5
    s.T if s.FALSY else s.U
    s.T or s.U
    s.FALSY or s.U
    if s.T and s.U:
        x = 2.5
    if s.FALSY or s.T:
        x = 3.5


def operand_fails() -> int:
    caught = 0
    try:
        raise s.EXC
    except ValueError:
        caught += 1
    try:
        print(s.T, s.U.fail())
    except ValueError:
        caught += 1
    return caught


def total() -> int:
    return caught
";

/// Writes the stub into the scratch root and each `(module, source)` as
/// `src/<module>.py` (the stub must not sit beside an entry module; see
/// `tests/issue_1084_loop_shape.rs`), then builds each extension into the
/// scratch root.
fn build(tag: &str, modules: &[(&str, &str)]) -> ScratchDir {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("pycc_t1498_stub.py"), STUB).expect("write the stub");
    std::fs::create_dir_all(dir.join("src")).expect("create the entry directory");
    for (module, source) in modules {
        let entry = dir.join("src").join(format!("{module}.py"));
        std::fs::write(&entry, source).expect("write the module");
        let build = pycc()
            .arg("build")
            .arg(&entry)
            .arg("-o")
            .arg(dir.join(module))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{}", stderr_of(&build));
    }
    dir
}

/// Runs `script` under the host interpreter. The host reports on stderr,
/// so stdout carries only what the compiled `print` wrote.
fn python(dir: &Path, script: &str) -> (String, String) {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    (stdout_of(&run), stderr_of(&run))
}

/// A host prelude defining `deltas(step)` and `report(*fields)`.
fn prelude() -> String {
    format!(
        "import sys\n\
         sys.path.insert(0, '.')\n\
         import pycc_t1498_stub as s\n\
         names = {MEASURED:?}\n\
         def counts():\n\
         \x20   return [sys.getrefcount(getattr(s, k)) for k in names]\n\
         def deltas(step):\n\
         \x20   before = counts()\n\
         \x20   result = step()\n\
         \x20   after = counts()\n\
         \x20   return ' '.join(str(a - b) for a, b in zip(after, before)), result\n\
         def report(*fields):\n\
         \x20   print(*fields, file=sys.stderr, flush=True)\n"
    )
}

/// One line of zeros, one per measured attribute.
fn zeros() -> String {
    vec!["0"; MEASURED.len()].join(" ")
}

/// The loops, comprehensions and operand shapes -- including every failure
/// edge -- leave every measured count where they found it, at two trip
/// counts; holding iterators from the host moves `SEQ` by one per iterator.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn iteration_and_operand_temporaries_are_released_exactly_once() {
    let dir = build("t1498_release", &[("pycc_t1498_mod", MODULE)]);
    let script = format!(
        "{prelude}\
         d, _ = deltas(lambda: __import__('pycc_t1498_mod'))\n\
         import pycc_t1498_mod as mod\n\
         report('import', d, mod.total())\n\
         def repeat(fn, trips):\n\
         \x20   return sum(fn() or 0 for _ in range(trips))\n\
         for trips in (50, 150):\n\
         \x20   for fn in (mod.comps, mod.comp_fails, mod.operands, mod.operand_fails):\n\
         \x20       d, r = deltas(lambda: repeat(fn, trips))\n\
         \x20       report(fn.__name__, trips, d, r)\n\
         \x20   held = []\n\
         \x20   d, _ = deltas(lambda: held.extend(iter(s.SEQ) for _ in range(trips)))\n\
         \x20   report('control', trips, d)\n",
        prelude = prelude()
    );
    let (out, err) = python(&dir, &script);
    let z = zeros();
    let mut expected = format!("import {z} 200\n");
    for trips in [50, 150] {
        expected.push_str(&format!(
            "comps {trips} {z} {}\n\
             comp_fails {trips} {z} {}\n\
             operands {trips} {z} 0\n\
             operand_fails {trips} {z} {}\n\
             control {trips} {trips} 0 0 0 0 0\n",
            3 * trips,
            2 * trips,
            2 * trips,
        ));
    }
    assert_eq!(err, expected);
    assert_eq!(out, "t u\n".repeat(200));
}

/// An uncaught failure in a module-level loop -- `next()` raising, and the
/// body raising inside two nested loops -- leaves the module body through
/// the module-exec failure return, which releases every open iterator: a
/// failed import moves no count, however many times it is retried.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_module_loop_releases_its_iterators() {
    let dir = build(
        "t1498_exec_fail",
        &[
            (
                "pycc_t1498_next",
                "import pycc_t1498_stub as s\n\nfor w in s.BOOM:\n    x = 1.5\n",
            ),
            (
                "pycc_t1498_body",
                "import pycc_t1498_stub as s\n\n\
                 for w in s.SEQ:\n    for v in s.SEQ:\n        s.T.fail()\n",
            ),
        ],
    );
    let script = format!(
        "{prelude}\
         def attempt(name):\n\
         \x20   caught = 0\n\
         \x20   for _ in range(50):\n\
         \x20       try:\n\
         \x20           __import__(name)\n\
         \x20       except ValueError:\n\
         \x20           caught += 1\n\
         \x20       sys.modules.pop(name, None)\n\
         \x20   return caught\n\
         for name in ('pycc_t1498_next', 'pycc_t1498_body'):\n\
         \x20   d, caught = deltas(lambda: attempt(name))\n\
         \x20   report(name, d, caught)\n",
        prelude = prelude()
    );
    let (_, err) = python(&dir, &script);
    let z = zeros();
    assert_eq!(
        err,
        format!("pycc_t1498_next {z} 50\npycc_t1498_body {z} 50\n")
    );
}
