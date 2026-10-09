//! End-to-end proof for comprehensions in any expression position ([#1254],
//! Part 1 of [#1214]; D-250).
//!
//! `docs/TYPE_SYSTEM.md`'s comprehension rule is the contract. The
//! byte-exact oracle fixture is `tests/fixtures/comprehension_expr.py`
//! (registered in `tests/conformance/classes.rs`); this file owns the
//! uncaught-exception exits, which CPython reports with a traceback pycc
//! does not print, a comprehension re-run from a loop in a function, the
//! set and dict expression forms without the oracle, and the two-module
//! programs of [#1237].
//!
//! [#1254]: https://github.com/rotnov/pycc/issues/1254
//! [#1214]: https://github.com/rotnov/pycc/issues/1214
//! [#1237]: https://github.com/rotnov/pycc/issues/1237

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

/// Builds `source` as a standalone executable and runs it.
fn build_and_run(category: &str, source: &str) -> Output {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(dir.join("subject"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{source}: {}", rendered(&build));
    Command::new(dir.join("subject"))
        .output()
        .expect("the built program should spawn")
}

/// Asserts the program printed exactly `stdout`, then stopped with exit
/// status 1 and `exception` on stderr, as CPython does.
fn assert_uncaught(run: &Output, stdout: &str, exception: &str) {
    assert_eq!(run.status.code(), Some(1), "{}", rendered(run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        stdout
    );
    assert!(
        String::from_utf8_lossy(&run.stderr).contains(exception),
        "{}",
        rendered(run)
    );
}

/// A `range` step of `0` in a comprehension argument raises `ValueError`
/// before the enclosing call runs, and nothing after it prints.
#[test]
fn a_zero_range_step_in_a_comprehension_argument_exits_with_value_error() {
    let run = build_and_run(
        "e2e_1254_step_zero",
        "print(\"before\")\nprint(len([x for x in range(0, 3, 0)]))\nprint(\"after\")\n",
    );
    assert_uncaught(
        &run,
        "before\n",
        "ValueError: range() arg 3 must not be zero",
    );
}

/// A raising element stops the loop at that element: the element raises on
/// the second of three iterations, and the third element's side effect never
/// runs.
#[test]
fn a_raising_element_in_a_comprehension_argument_exits_with_zero_division_error() {
    let run = build_and_run(
        "e2e_1254_elt_raises",
        "def f(x: int) -> int:\n    print(x)\n    return 1 // (x - 1)\n\n\
         print(\"before\")\nprint(len([f(x) for x in range(3)]))\nprint(\"after\")\n",
    );
    assert_uncaught(&run, "before\n0\n1\n", "ZeroDivisionError");
}

/// A raising element over a dict source stops at the raising key, and the
/// program exits with the element's own exception.
#[test]
fn a_raising_element_over_a_dict_source_exits_with_zero_division_error() {
    let run = build_and_run(
        "e2e_1254_dict_elt_raises",
        "def f(d: dict[str, int], k: str) -> int:\n    print(k)\n    return 10 // d[k]\n\n\
         d = {\"a\": 1, \"b\": 0, \"c\": 2}\nprint(\"before\")\n\
         print(len([f(d, k) for k in d]))\nprint(\"after\")\n",
    );
    assert_uncaught(&run, "before\na\nb\n", "ZeroDivisionError");
}

/// The same comprehension, run many times from one call and from many
/// calls, rebuilds its container each time and never grows the stack: the
/// loop-variable slot is hoisted to the entry block.
#[test]
fn a_comprehension_rerun_in_a_loop_rebuilds_its_container_each_time() {
    let run = build_and_run(
        "e2e_1254_rerun",
        "def count(n: int) -> int:\n    t = 0\n    for r in range(n):\n        \
         t = t + len([i for i in range(r) if i % 2 == 0])\n    return t\n\n\
         print(count(2000))\nprint(count(3))\n",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "1000000\n2\n"
    );
}

/// Set and dict comprehensions in expression position, including a dict
/// source whose `str` loop variable lives in the entry-block slot, run
/// without the pinned oracle (`3 2` and `3 3` match CPython 3.14.7).
#[test]
fn set_and_dict_comprehension_arguments_run_to_the_cpython_output() {
    let run = build_and_run(
        "e2e_1254_set_dict",
        "def keys(d: dict[str, int]) -> int:\n    return len({k: d[k] + 1 for k in d if d[k] > 1})\n\n\nd = {\"a\": 1, \"b\": 2, \"c\": 3}\nprint(len({i % 3 for i in range(10)}), keys(d))\nprint(len({k: 0 for k in d}), len({d[k] for k in d}))\n",
    );
    assert_eq!(run.status.code(), Some(0), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "3 2\n3 3\n"
    );
}

/// Writes `modules` (the first is the entry) into one directory, builds the
/// entry and returns the build output and, when it succeeded, the run.
fn build_program(category: &str, modules: &[(&str, &str)]) -> (Output, Option<Output>) {
    let dir = ScratchDir::new(category).expect("scratch");
    for (name, source) in modules {
        std::fs::write(dir.join(name), source).expect("write a module");
    }
    let build = pycc()
        .arg("build")
        .arg(dir.join(modules[0].0))
        .arg("-o")
        .arg(dir.join("program"))
        .output()
        .expect("pycc should spawn");
    let run = build.status.success().then(|| {
        Command::new(dir.join("program"))
            .output()
            .expect("the built program should spawn")
    });
    (build, run)
}

/// #1237: two modules whose statement-form comprehensions share a target
/// name and a byte offset used to fail to link on the synthesized loop
/// variable the user never wrote. CPython prints `2`.
#[test]
fn same_offset_comprehensions_in_two_modules_link_and_run() {
    let (build, run) = build_program(
        "e2e_1237_two_modules",
        &[
            (
                "a.py",
                "ws = [1]\nxs = [y for y in ws]\nfrom b import zs\nprint(len(xs) + len(zs))\n",
            ),
            ("b.py", "vs = [1]\nzs = [y for y in vs]\n"),
        ],
    );
    let run = run.unwrap_or_else(|| panic!("{}", rendered(&build)));
    assert_eq!(run.status.code(), Some(0), "{}", rendered(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "2\n"
    );
}

/// A known gap under #1237, not intended behavior: the linked program is
/// one flat namespace, so same-offset, same-name loop variables of
/// *different* types (a `range` `int` and a `dict` `str` key) still share
/// one module-global slot and the checker refuses the second binding.
/// CPython prints `4`. The refusal is fail-closed (nothing is emitted).
#[test]
fn same_offset_comprehension_variables_of_different_types_are_still_refused() {
    let (build, run) = build_program(
        "e2e_1237_two_types",
        &[
            (
                "a.py",
                "ws = [1, 2]\nxs = [y for y in range(3)]\nfrom b import zs\nprint(len(xs) + len(zs))\n",
            ),
            ("b.py", "v={\"\":1}\nzs = {y: 1 for y in v}\n"),
        ],
    );
    assert!(run.is_none(), "the build is expected to be refused");
    assert!(
        rendered(&build).contains(
            "error[T0023]: cannot assign `int` to `0comp_24_y`, previously inferred as `str`"
        ),
        "{}",
        rendered(&build)
    );
}
