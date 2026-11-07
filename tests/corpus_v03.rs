//! v0.3 accept 3 (the real-world corpus): what the corpus needed natively.
//!
//! * The solver types the result of an operator over two concrete operands
//!   at once, so a builtin method or call over it (`a.count(m - 1)`) types
//!   the parameter it is passed to; a list display of such results is a
//!   concrete list.
//! * An empty list whose producer reads an unannotated function's parameter
//!   or return is typed once the solver has pinned that signature.
//! * An empty list bound to a name whose element nothing reads, produces or
//!   compares is a `list[int]`: no operation can observe its element type.
//! * A strict-mode return the solver cannot type (a returned tuple display)
//!   is typed from the function's body once its parameters are, and the
//!   solver runs again with it (`constraints::return_fallback`).
//! * `exit`/`quit` and `from sys import exit` are `sys.exit`; a module-level
//!   `X = sys.stdin` (or `sys.stdout`) reads `X` as the stream itself.
//! * A module-level `if` on a flag bound once to a literal, or on
//!   `sys.gettrace()`, is the branch that runs; `next(sys.stdin)` reads a
//!   line; a parameter whose default is a builtin conversion no call passes
//!   is that builtin; `x in c` of a constant tuple, `a, b = x, y` and
//!   `g[i].sort(reverse=True)` compile natively; a `def` returning
//!   `sys.stdin.buffer.readline()` read only as numbers is text.
//! * Non-strict mode solves again with what the solver settled written in,
//!   so a use met before its operand was typed is native too.
//! * A tuple whose elements share one type takes a computed index and
//!   iterates; a generator expression reads a module-level loop variable;
//!   `any`/`all` of a generator of CPython objects is native.
//! * Item access on an instance of a class defining the item dunders calls
//!   them; a scalar local rebound to an empty list is a separate variable;
//!   `list()`/`dict()` are `[]`/`{}`; `set.update`/`list.extend` take a
//!   generator, and a generator crosses into a CPython method.
//! * Strict mode, like non-strict mode, does not compile an unannotated
//!   definition nothing in the entry module mentions
//!   (`corpus_round7::strict_mode_does_not_compile_dead_unannotated_definitions`).
//!
//! The oracle is CPython: each fixture is built, run and compared with
//! `python3` running the file -- stdout, the uncaught exception's last line
//! and the exit status.

use pycc_scratch::ScratchDir;
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

/// Runs `command` with fixture `name`'s `.stdin` file (if any) as stdin.
fn run_with_stdin(mut command: Command, name: &str) -> Output {
    let stdin = fixture(&name.replace(".py", ".stdin"));
    let input = std::fs::File::open(&stdin).map_or_else(|_| Stdio::null(), Stdio::from);
    command.stdin(input).output().expect("the program runs")
}

/// Builds fixture `name` (plus `flags`) with `--release`, runs it and
/// compares with CPython. Returns the build's stderr.
fn matches_cpython(name: &str, flags: &[&str]) -> String {
    let dir = ScratchDir::new("corpus_v03").expect("scratch");
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(fixture(name))
        .arg("-o")
        .arg(&out)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&pycc_scratch::lf(&build.stdout)),
        text(&pycc_scratch::lf(&build.stderr))
    );
    let ours = run_with_stdin(Command::new(&out), name);
    let mut oracle = python();
    oracle.arg(fixture(name));
    let theirs = run_with_stdin(oracle, name);
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&theirs.stdout)),
        "stdout differs ({name})"
    );
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&theirs.stderr)), "{name}");
    assert_eq!(ours.status.code(), theirs.status.code(), "{name}");
    text(&pycc_scratch::lf(&build.stderr))
}

#[test]
fn inference_strict() {
    matches_cpython("corpus_v03_inference.py", &[]);
}

#[test]
fn inference_non_strict_without_fallback() {
    let report = matches_cpython(
        "corpus_v03_inference.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// `exit`/`quit` and `from sys import exit` are CPython's `sys.exit`, run
/// through the embedded interpreter.
#[test]
#[ignore = "needs CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn exit_is_sys_exit() {
    for flags in [&[][..], &["--non-strict"][..]] {
        matches_cpython("corpus_v03_exit.py", flags);
        matches_cpython("corpus_v03_exit_from_sys.py", flags);
    }
}

#[test]
fn a_stream_alias_is_the_stream() {
    matches_cpython("corpus_v03_stream_alias.py", &[]);
    let report = matches_cpython(
        "corpus_v03_stream_alias.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// Template idioms: module-level `if`s on a literal flag or
/// `sys.gettrace()`, `next(sys.stdin)`, a builtin default no call passes,
/// membership in a constant tuple, unpacked tuple displays and
/// `.sort(reverse=True)` of a subscripted list.
#[test]
fn template_idioms_compile_natively() {
    for fixture in [
        "corpus_v03_idioms.py",
        "corpus_v03_buffer_def.py",
        "corpus_v03_item_producers.py",
    ] {
        matches_cpython(fixture, &[]);
        let report = matches_cpython(fixture, &["--non-strict", "--report-fallbacks"]);
        assert!(!report.contains("fallback:"), "{fixture}: {report}");
    }
}

/// A generator expression reading a module-level loop variable is typed
/// (the variable is bound where the expression runs; an unbound one raises
/// `NameError`), and `any`/`all` of a generator of CPython objects runs
/// natively in non-strict mode.
#[test]
#[ignore = "needs CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn generator_expressions_over_globals_and_objects() {
    let report = matches_cpython(
        "corpus_v03_genexpr_global.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
    for flags in [&[][..], &["--non-strict"][..]] {
        matches_cpython("corpus_v03_genexpr_unbound.py", flags);
    }
}

/// Item dunders through `self`, a local and a global; a scalar local
/// rebound to an empty list; `list()`/`dict()`; `set.update`/`list.extend`
/// of a generator.
#[test]
fn item_dunders_rebinding_and_generator_updates() {
    for fixture in ["corpus_v03_item_dunders.py", "corpus_v03_rebinding.py"] {
        matches_cpython(fixture, &[]);
        let report = matches_cpython(fixture, &["--non-strict", "--report-fallbacks"]);
        assert!(!report.contains("fallback:"), "{fixture}: {report}");
    }
}

/// Non-strict mode: a generator argument of a CPython object's method.
#[test]
#[ignore = "needs CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn a_generator_crosses_into_a_cpython_method() {
    let report = matches_cpython(
        "corpus_v03_object_generator_arg.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// A tuple whose elements share one type: a computed index (`t[k]`, an
/// out-of-range one raising `IndexError`) and iteration (`for x in t`, a
/// comprehension over it).
#[test]
fn homogeneous_tuples_index_and_iterate() {
    for fixture in ["corpus_v03_tuple_index.py", "corpus_v03_tuple_iter.py"] {
        matches_cpython(fixture, &[]);
        let report = matches_cpython(fixture, &["--non-strict", "--report-fallbacks"]);
        assert!(!report.contains("fallback:"), "{fixture}: {report}");
    }
}
