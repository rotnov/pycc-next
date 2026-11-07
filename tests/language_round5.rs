//! Core language constructs: `global` statements, `def`s in module-level
//! blocks, generators, first-class nested functions, multi-clause
//! comprehensions, starred call arguments and computed instance attributes.
//!
//! The oracle is CPython itself: each fixture is built (strict unless the
//! test says otherwise), run with the same stdin as `python3` running the
//! file, and compared byte for byte -- stdout, the uncaught exception's last
//! line and the exit status. `--ext` arms import the built module from a
//! real interpreter and compare with importing the source.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn run_with_stdin(mut command: Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program should spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Builds fixture `name` (plus `flags`) in both profiles, runs it with
/// `stdin`, and compares with CPython. Returns the build's stderr (the
/// fallback report, when asked for).
fn matches_cpython(name: &str, flags: &[&str], stdin: &str) -> String {
    let mut report = String::new();
    for release in [false, true] {
        let dir = ScratchDir::new("language_round5").expect("scratch");
        let out = dir.join("prog");
        let mut build = pycc();
        build
            .arg("build")
            .arg(fixture(name))
            .arg("-o")
            .arg(&out)
            .args(flags);
        if release {
            build.arg("--release");
        }
        let build = build.output().expect("pycc should spawn");
        assert!(
            build.status.success(),
            "{}{}",
            text(&pycc_scratch::lf(&build.stdout)),
            text(&pycc_scratch::lf(&build.stderr))
        );
        report = text(&pycc_scratch::lf(&build.stderr));
        let ours = run_with_stdin(Command::new(&out), stdin);
        let mut theirs = python();
        theirs.arg(fixture(name));
        let theirs = run_with_stdin(theirs, stdin);
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&theirs.stdout)),
            "stdout differs ({name})"
        );
        assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&theirs.stderr)), "{name}");
        assert_eq!(ours.status.code(), theirs.status.code(), "{name}");
    }
    report
}

/// Builds `source` as `--ext` module `name`, then runs `program` against
/// the built module and against the source, and compares their output.
fn ext_matches_cpython(name: &str, source: &str, program: &str) {
    let built = ScratchDir::new("language_round5_ext").expect("scratch");
    let plain = ScratchDir::new("language_round5_src").expect("scratch");
    std::fs::write(built.join(format!("{name}.py")), source).expect("write");
    std::fs::write(plain.join(format!("{name}.py")), source).expect("write");
    let build = pycc()
        .arg("build")
        .arg(built.join(format!("{name}.py")))
        .arg("-o")
        .arg(built.join(name))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&pycc_scratch::lf(&build.stdout)),
        text(&pycc_scratch::lf(&build.stderr))
    );
    std::fs::remove_file(built.join(format!("{name}.py"))).expect("remove the source");
    let run = |dir: &Path| {
        python()
            .arg("-c")
            .arg(program)
            .current_dir(dir)
            .output()
            .expect("python3 should spawn")
    };
    let ours = run(&built);
    let theirs = run(&plain);
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&theirs.stdout)),
        "stdout differs: {}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&theirs.stderr)));
    assert_eq!(ours.status.code(), theirs.status.code());
}

fn check(source: &str) -> String {
    let dir = ScratchDir::new("language_round5_check").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write");
    let out = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    format!("{}{}", text(&pycc_scratch::lf(&out.stdout)), text(&pycc_scratch::lf(&out.stderr)))
}

/// Functions rebinding module globals they declare `global` (augmented,
/// plain, `for`-target and recursive writes), functions and comprehensions
/// only reading them, a local of the same name, module code rebinding them
/// between calls, evaluation order of `cnt += f()` where `f` writes `cnt`,
/// and an exception raised through a global write.
#[test]
fn global_statements_match_cpython() {
    matches_cpython("global_statements.py", &[], "");
}

/// The same program in non-strict mode compiles without falling back.
#[test]
fn global_statements_compile_natively_in_non_strict_mode() {
    let report = matches_cpython(
        "global_statements.py",
        &["--non-strict", "--report-fallbacks"],
        "",
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// A `global` the cell rewrite does not model -- no module-level binding,
/// or a module-level binding that is not a plain assignment -- is still
/// refused with the spanned `C0001`.
#[test]
fn unmodelled_global_declarations_are_refused() {
    for source in [
        "def g() -> None:\n    global x\n    x = 1\n",
        "import os\ndef g() -> None:\n    global os\n    os = 1\n",
        "x = 1\ndef g() -> None:\n    global x\n    del x\n",
    ] {
        let text = check(source);
        assert!(
            text.contains("error[C0001]: statement kind not supported yet: a `global` declaration"),
            "{source}: {text}"
        );
    }
}

/// Module globals rebound by `global` functions of an `--ext` module, called
/// from Python.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn global_statements_cross_the_ext_boundary() {
    ext_matches_cpython(
        "glob_probe",
        "count = 0\nnames: list[str] = []\n\n\ndef bump(k: int) -> int:\n    global count\n    count += k\n    return count\n\n\ndef add(s: str) -> int:\n    global names\n    names = names + [s]\n    return len(names)\n",
        "import glob_probe\nprint(glob_probe.bump(2), glob_probe.bump(3), glob_probe.add('a'), glob_probe.add('b'))\n",
    );
}

/// Comprehensions with several `for`/`if` clauses (list, set, dict, nested,
/// inside a generator argument, evaluation order of side effects, a target
/// rebinding an outer clause's name), and starred call arguments with
/// CPython's `TypeError` messages for a wrong count.
#[test]
fn multi_clause_comprehensions_and_starred_calls_match_cpython() {
    matches_cpython("multi_clause_comprehensions.py", &[], "");
}

/// Instance attributes established in `__init__` from arbitrary
/// expressions: arithmetic, list repetition, calls, a local built from a
/// comprehension, earlier attributes and another class's instance.
#[test]
fn computed_instance_attributes_match_cpython() {
    matches_cpython("computed_instance_attrs.py", &[], "");
}

/// `def`s inside module-level `for` and `if` blocks, used only after their
/// definition in the same block, compile natively.
#[test]
fn defs_in_module_level_blocks_match_cpython() {
    let report = matches_cpython("block_defs.py", &["--non-strict", "--report-fallbacks"], "");
    assert!(
        !report.contains("blk.py:2-") && !report.contains("block_defs.py:2-"),
        "{report}"
    );
}

/// Generator functions: `for` over them, `next()` with and without a
/// default, `StopIteration`, consumers (`list`, `sum`, `max`, `sorted`,
/// `any`, `all`), `return` ending one, `try`/`finally` run when a loop
/// `break`s or a generator is abandoned, `yield from`, an exception raised
/// inside one, a generator method and an inferred private generator.
#[test]
fn generators_match_cpython() {
    matches_cpython("generators.py", &[], "");
}

/// Generator expressions: lazy, `any`/`all` stopping at the first deciding
/// element, `next()` of one, several clauses, one kept in a variable and
/// consumed later, and nested ones.
#[test]
fn generator_expressions_match_cpython() {
    matches_cpython("generator_expressions.py", &[], "");
}

/// Both fixtures compile natively in non-strict mode.
#[test]
fn generators_compile_natively_in_non_strict_mode() {
    for name in ["generators.py", "generator_expressions.py"] {
        let report = matches_cpython(name, &["--non-strict", "--report-fallbacks"], "");
        assert!(!report.contains("fallback:"), "{report}");
    }
}

/// A generator expression reading a variable its function rebinds after
/// creating it is refused rather than compiled with the earlier value.
#[test]
fn a_generator_expression_over_a_rebound_variable_is_refused() {
    let text = check(
        "def f() -> list[int]:\n    k = 1\n    g = (x + k for x in range(3))\n    k = 10\n    return list(g)\n\n\nprint(f())\n",
    );
    assert!(text.contains("error[C0001]"), "{text}");
}

/// A public generator of an `--ext` module is a CPython iterator: `list`,
/// `next`, `zip`, `StopIteration`, its `finally` run when the iterator is
/// released unfinished, and in-module consumers of the native generator.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn generators_cross_the_ext_boundary_as_iterators() {
    ext_matches_cpython(
        "gen_probe",
        "from typing import Iterator\n\n\ndef count(n: int) -> Iterator[int]:\n    for i in range(n):\n        yield i * i\n\n\ndef words(s: str) -> Iterator[str]:\n    try:\n        for w in s.split():\n            yield w.upper()\n    finally:\n        print(\"words done\")\n\n\ndef total(n: int) -> int:\n    return sum(count(n))\n\n\nclass Bag:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    def evens(self) -> Iterator[int]:\n        for x in range(self.n):\n            if x % 2 == 0:\n                yield x\n",
        "import gen_probe as m\ng = m.count(4)\nprint(type(g).__name__, iter(g) is g, list(g), list(g))\nprint(m.total(5), sum(m.count(5)))\nw = m.words('ab cd')\nprint(next(w))\ndel w\nprint('after')\ntry:\n    next(m.count(0))\nexcept StopIteration:\n    print('stop')\nprint(list(zip(m.count(3), m.words('x y z'))))\nprint(list(m.Bag(5).evens()))\n",
    );
}

/// Function values: nested functions returned and passed (capturing `int`,
/// `str`, `list`, an instance and a `nonlocal` counter), module-level
/// functions used as values, a call of a call's result, a closure raising,
/// a big-int capture and a module global holding a function.
#[test]
fn closures_match_cpython() {
    matches_cpython("closures.py", &[], "");
}

/// The same program compiles natively in non-strict mode.
#[test]
fn closures_compile_natively_in_non_strict_mode() {
    let report = matches_cpython("closures.py", &["--non-strict", "--report-fallbacks"], "");
    assert!(!report.contains("fallback:"), "{report}");
}

/// A nested function made a value while a variable it reads may still
/// change (CPython reads it when the function runs) stays refused, as does
/// one with unannotated parameters. (Printing a function is supported since
/// `language_round6`.)
#[test]
fn unsound_closures_are_refused() {
    for source in [
        "from typing import Callable\n\n\ndef f() -> int:\n    k = 1\n\n    def get() -> int:\n        return k\n    g = get\n    k = 5\n    return g()\n\n\nprint(f())\n",
        "def f() -> list[int]:\n    out: list[int] = []\n    for i in range(3):\n        def get() -> int:\n            return i\n        out.append(apply(get))\n    return out\n\n\ndef apply(g) -> int:\n    return 0\n",
        "def make(n: int):\n    def f(x):\n        return x * n\n    return f\n",
    ] {
        let text = check(source);
        assert!(text.contains("error["), "{source}: {text}");
    }
}
