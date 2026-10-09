//! Exported `@staticmethod`s and `@classmethod`s against a real CPython
//! (PR 1 of #1143).
//!
//! Every test here is `#[ignore]`d for the same reason
//! `tests/issue_1050_ext_tuple.rs`'s are: each builds an artifact and asks an
//! installed CPython 3.13+ to import it, which is a property of the machine
//! rather than of the change under test. CI runs them on every Tier-1
//! `native-build-test` leg through that job's
//! `cargo test --workspace -- --include-ignored`.
//!
//! They are deliberately not where line coverage comes from -- the coverage
//! job runs `llvm-cov` without `--include-ignored`.
//! `src/ext_build_tests/generated_c.rs` covers the emitted text and
//! `crates/pycc_codegen/src/ext_thunk_tests.rs` covers the emitted symbols.
//! This file covers the one thing neither can: that the type object the
//! generated `PyType_FromSpec` call builds really publishes
//! `mod.Class.method` and really passes the null receiver a
//! `@classmethod`'s compiled body expects. Instantiation of that type
//! object was refused in PR 1 and is not any more -- see
//! `the_published_type_is_immutable_and_now_constructible` below and
//! `tests/issue_1145_ext_instance_methods.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn run_python(dir: &Path, script: &str) {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}

const GRID: &str = "\
class Grid:
    @staticmethod
    def scale(n: int) -> int:
        return n * 3

    @classmethod
    def make(cls, n: int) -> int:
        return n + 1
";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_static_and_a_class_method_are_callable_through_the_class_attribute() {
    let dir = ScratchDir::new("ext_1143_call").expect("scratch");
    build_ext(&dir, "grid", GRID);
    run_python(
        &dir,
        "\
import grid
assert grid.Grid.scale(3) == 9, grid.Grid.scale(3)
assert grid.Grid.make(3) == 4, grid.Grid.make(3)
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn no_flat_dotted_module_attribute_is_published() {
    // The host surface is `mod.Grid.scale`, and only that. A flat
    // `mod."Grid.scale"` attribute would be a second, unsupported spelling of
    // the same export.
    let dir = ScratchDir::new("ext_1143_flat").expect("scratch");
    build_ext(&dir, "grid", GRID);
    run_python(
        &dir,
        "\
import grid
assert not hasattr(grid, 'Grid.scale'), dir(grid)
assert not hasattr(grid, 'scale'), dir(grid)
assert not hasattr(grid, 'make'), dir(grid)
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_published_type_is_immutable_and_now_constructible() {
    // `Py_TPFLAGS_IMMUTABLETYPE` is unchanged since PR 1: a mutable type
    // would let a host add a method the compiler never saw.
    //
    // `Py_TPFLAGS_DISALLOW_INSTANTIATION` is *not* unchanged. PR 1 set it
    // because instance methods were unimplemented, so an instance would have
    // had no behaviour. #1145 implements them, and `Grid` -- which declares
    // no `__init__` at all -- is constructible through the D-225 implicit
    // zero-argument constructor, so the flag is gone and `grid.Grid()`
    // succeeds. `tests/issue_1145_ext_instance_methods.rs` owns the
    // still-refused direction: a class the constructibility predicate
    // rejects keeps the flag.
    let dir = ScratchDir::new("ext_1143_flags").expect("scratch");
    build_ext(&dir, "grid", GRID);
    run_python(
        &dir,
        "\
import grid
instance = grid.Grid()
assert type(instance) is grid.Grid, type(instance)
try:
    grid.Grid.other = 1
except TypeError:
    pass
else:
    raise AssertionError('the type should be immutable')
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn both_method_kinds_bind_a_keyword_and_refuse_an_unknown_one() {
    // D-244 rule 7 as amended by #1461: a keyword naming a parameter is
    // bound as CPython binds it, on a type object exactly as on the module,
    // and one naming no parameter raises CPython's own wording.
    let dir = ScratchDir::new("ext_1143_kw").expect("scratch");
    build_ext(&dir, "grid", GRID);
    run_python(
        &dir,
        "\
import grid
assert grid.Grid.scale(n=1) == 3, grid.Grid.scale(n=1)
assert grid.Grid.make(n=1) == 2, grid.Grid.make(n=1)
for call, name in (
    (lambda: grid.Grid.scale(k=1), 'scale'),
    (lambda: grid.Grid.make(k=1), 'make'),
):
    try:
        call()
    except TypeError as e:
        want = f\"Grid.{name}() got an unexpected keyword argument 'k'\"
        assert str(e) == want, str(e)
    else:
        raise AssertionError('an unknown keyword should be refused')
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_arity_message_names_the_source_level_spelling() {
    // Never `Grid.scale.static`, which is a compiler-internal mangling, and
    // never the mangled C symbol.
    let dir = ScratchDir::new("ext_1143_arity").expect("scratch");
    build_ext(&dir, "grid", GRID);
    run_python(
        &dir,
        "\
import grid
try:
    grid.Grid.scale(1, 2)
except TypeError as exc:
    message = str(exc)
else:
    raise AssertionError('a wrong arity should be refused')
assert 'Grid.scale() takes exactly 1 argument' in message, message
assert '.static' not in message, message
assert '0m' not in message, message
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_tuple_carrying_class_method_round_trips_through_its_thunk() {
    let dir = ScratchDir::new("ext_1143_tuple").expect("scratch");
    build_ext(
        &dir,
        "pairs",
        "\
class Pairs:
    @classmethod
    def of(cls, n: int) -> tuple[int, int]:
        return (n, n + 1)
",
    );
    run_python(
        &dir,
        "\
import pairs
assert pairs.Pairs.of(4) == (4, 5), pairs.Pairs.of(4)
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_user_exception_class_publishes_its_class_but_not_its_static_method() {
    // The exception class itself is registered by #1066's path and stays
    // instantiable; its methods are outside the export set.
    let dir = ScratchDir::new("ext_1143_exc").expect("scratch");
    build_ext(
        &dir,
        "failing",
        "\
class Failure(Exception):
    @staticmethod
    def of(n: int) -> int:
        return n
",
    );
    run_python(
        &dir,
        "\
import failing
assert issubclass(failing.Failure, Exception)
assert not hasattr(failing.Failure, 'of'), dir(failing.Failure)
",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
// A PEP 654 group class is excluded from the registered exception table by
// `GROUP_EXCEPTION_CLASSES` -- the limited C API exposes no
// `PyExc_ExceptionGroup` -- so the class itself is not published, a residual
// `docs/RUNTIME.md` already records and #1143 does not change. What #1143
// adds is that its `@staticmethod` is not published either, by any spelling:
// the export set excludes every method of a user exception class.
fn an_exception_group_subclass_publishes_neither_the_class_nor_its_static_method() {
    let dir = ScratchDir::new("ext_1143_excgroup").expect("scratch");
    build_ext(
        &dir,
        "grouping",
        "\
class Grouped(ExceptionGroup):
    @staticmethod
    def of(n: int) -> int:
        return n
",
    );
    run_python(
        &dir,
        "\
import grouping
assert not hasattr(grouping, 'Grouped'), dir(grouping)
assert not hasattr(grouping, 'of'), dir(grouping)
assert not hasattr(grouping, 'Grouped.of'), dir(grouping)
",
    );
}
