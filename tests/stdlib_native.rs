//! `itertools` and `collections` compiled natively (no embedded CPython):
//! each fixture `tests/fixtures/<name>.py` is built in strict mode, in
//! `--non-strict` mode and as an `--ext` module imported by CPython, and
//! each run is compared with the live CPython oracle (`PYCC_PYTHON`,
//! default `python3`): stdout byte-for-byte, the uncaught exception's last
//! stderr line and the exit status.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_stderr_line(output: &Output) -> String {
    text(&output.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

fn oracle() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.py"))
}

fn pycc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .output()
        .expect("pycc should spawn")
}

fn assert_same(name: &str, mode: &str, run: &Output, cpython: &Output) {
    assert_eq!(
        text(&run.stdout),
        text(&cpython.stdout),
        "{name} ({mode}): stdout"
    );
    assert_eq!(
        last_stderr_line(run),
        last_stderr_line(cpython),
        "{name} ({mode}): last stderr line"
    );
    assert_eq!(
        run.status.code(),
        cpython.status.code(),
        "{name} ({mode}): exit status"
    );
}

/// Builds the fixture natively (strict, then `--non-strict`) and as an
/// `--ext` module, and compares every run with CPython's.
fn assert_matches_cpython(name: &str) {
    let source = fixture(name);
    let cpython = Command::new(oracle())
        .arg(&source)
        .output()
        .expect("python3 spawns");
    let dir = ScratchDir::new(name).expect("scratch");
    for (mode, flags) in [("strict", &[][..]), ("non-strict", &["--non-strict"][..])] {
        let binary = dir.join(format!("app-{mode}"));
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "build".as_ref(),
            source.as_os_str(),
            "-o".as_ref(),
            binary.as_os_str(),
            "--release".as_ref(),
        ];
        args.extend(flags.iter().map(|f| std::ffi::OsStr::new(*f)));
        let build = pycc(&args);
        assert!(
            build.status.success(),
            "pycc build ({mode}) of {name} failed:\n{}{}",
            text(&build.stdout),
            text(&build.stderr)
        );
        let run = Command::new(&binary).output().expect("binary runs");
        assert_same(name, mode, &run, &cpython);
    }
    let ext_dir = dir.join("ext");
    std::fs::create_dir_all(&ext_dir).expect("ext dir");
    let module = ext_dir.join(name);
    let build = pycc(&[
        "build".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        module.as_os_str(),
        "--ext".as_ref(),
        "--release".as_ref(),
    ]);
    assert!(
        build.status.success(),
        "pycc build --ext of {name} failed:\n{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let run = Command::new(oracle())
        .current_dir(&ext_dir)
        .arg("-c")
        .arg(format!("import {name}"))
        .output()
        .expect("python3 spawns");
    assert_same(name, "--ext", &run, &cpython);
}

/// `pycc check` of `source` fails with a diagnostic containing `expected`.
fn assert_check_fails(name: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(name).expect("scratch");
    let path = dir.join(format!("{name}.py"));
    std::fs::write(&path, source).expect("write source");
    let check = pycc(&["check".as_ref(), path.as_os_str()]);
    let stderr = text(&check.stderr) + &text(&check.stdout);
    assert!(!check.status.success(), "{name}: pycc check should fail");
    assert!(
        stderr.contains(expected),
        "{name}: expected `{expected}` in:\n{stderr}"
    );
}

/// `permutations`, `combinations`, `combinations_with_replacement`,
/// `product` (with `repeat=`), `islice`, `count`, `chain` (and
/// `chain.from_iterable`), `accumulate` (operators, `max`/`min`, lambdas,
/// `initial=`), `pairwise` and `groupby` (with a key), consumed by `list`,
/// `for` and comprehensions.
#[test]
fn itertools_functions_match_cpython() {
    assert_matches_cpython("itertools_basic");
}

/// Laziness (an iterator pulls only the values it needs from a generator),
/// infinite `count`, bigints, tuples of every element kind, instances,
/// early exit, empty inputs, every consumer and `islice`'s `ValueError`s.
#[test]
fn itertools_laziness_edges_and_errors_match_cpython() {
    assert_matches_cpython("itertools_lazy");
}

/// The tuples `permutations` yields have the iterable's length when `r` is
/// omitted, which the type of a list known only at run time cannot carry.
#[test]
fn permutations_without_a_literal_r_is_refused() {
    assert_check_fails(
        "it_perm_r",
        "from itertools import permutations\n\ndef f(xs: list[int]) -> None:\n    for p in permutations(xs):\n        print(p)\n",
        "compiled only with `r` given as an int literal",
    );
}

/// `collections.deque`: construction (empty, from an iterable, `maxlen`),
/// both ends, `extend`/`extendleft`, `rotate`, `index`/`count`/`in`,
/// iteration, `copy`, `+`/`*`, equality, `repr`, a BFS queue, and the
/// errors CPython raises (`pop from an empty deque`, `deque index out of
/// range`, a list's missing `appendleft`).
#[test]
fn collections_deque_matches_cpython() {
    assert_matches_cpython("collections_deque");
}

/// `collections.Counter` (from a `str`, a list, empty; missing keys reading
/// `0`, `most_common`, `elements`, `total`, `update`, `subtract`, `+`/`-`/
/// `|`/`&`, equality, `repr` in `most_common` order) and
/// `collections.defaultdict` with `int`, `list`, `set`, `float`, `str`, a
/// nested `defaultdict` and a lambda as factories.
#[test]
fn collections_counter_and_defaultdict_match_cpython() {
    assert_matches_cpython("collections_counter");
}
