//! End-to-end proof for the native `frozenset[int]` value type (#1326,
//! Part 1 of #1319).
//!
//! `docs/TYPE_SYSTEM.md`'s container section is the contract. A
//! `frozenset[int]` shares `set[int]`'s runtime object, and D-123 leaves a
//! set's iteration order unlike CPython's, so every program here prints only
//! order-insensitive facts: lengths, sums and single elements.
//!
//! The native differential runs against whatever `PYCC_PYTHON`/`python3` is
//! available, as `tests/issue_1262_container_slots.rs` does. The `--ext`
//! differentials import the built extension in CPython and compare it with
//! `runpy.run_path` on the same source; they are `#[ignore]`d like their
//! siblings and run under `--include-ignored` in CI.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// Every construction form, `frozenset[int]` as a parameter, a return and an
/// annotated local, `len`, a `for` sum, a set comprehension over a frozenset,
/// truthiness, and an unannotated private helper that reaches the constraint
/// path. Every helper is `_`-prefixed so the same body builds under `--ext`,
/// where a public `frozenset[int]` parameter or return is `C0003`.
const SUCCESS: &str = "\
def _total(fs: frozenset[int]) -> int:
    t = 0
    for v in fs:
        t += v
    return t


def _make(xs: list[int]) -> frozenset[int]:
    return frozenset(xs)


def _freeze(a):
    return frozenset(a)


empty = frozenset()
from_literal = frozenset({4, 5, 4})
from_list = _make([3, 1, 3, 2])
from_frozen = frozenset(from_list)
source = {1, 2}
snapshot: frozenset[int] = frozenset(source)
source.add(9)
print(len(empty), len(from_literal), len(from_list), len(from_frozen))
print(_total(from_list), _total(from_frozen), _total(empty))
print(len(snapshot), len(source), _total(snapshot))
print(len(_freeze([6, 6, 5])))
doubled = {v * 2 for v in from_list}
print(len(doubled))
if empty:
    print(\"empty is truthy\")
else:
    print(\"empty is falsy\")
if from_literal:
    print(\"from_literal is truthy\")
while empty:
    print(\"unreachable\")
single = frozenset([7])
for v in single:
    print(v)
if source:
    print(\"source is truthy\")
else:
    print(\"source is falsy\")
drained = {v for v in empty}
if drained:
    print(\"drained is truthy\")
else:
    print(\"drained is falsy\")
";

const SUCCESS_STDOUT: &str = "0 2 3 3\n6 6 0\n2 3 3\n2\n3\nempty is falsy\n\
                              from_literal is truthy\n7\nsource is truthy\n\
                              drained is falsy\n";

/// A user `def frozenset` shadows the builtin, including for an unannotated
/// private helper defined before it, which the constraint path types.
const FUNCTION_SHADOW: &str = "\
def _h(a):
    return frozenset(a)


def frozenset(x: int) -> int:
    return x + 1


print(frozenset(2), _h(4))
";

/// A user `class frozenset:` keeps compiling as the class it is, both as a
/// call and as a bare annotation.
const CLASS_SHADOW: &str = "\
class frozenset:
    def __init__(self, x: int) -> None:
        self.x = x


def _g(a: frozenset) -> int:
    return a.x


f = frozenset(3)
print(f.x)
print(_g(frozenset(4)))
";

/// A user `class frozenset:` constructed inside a private helper. With the
/// helper's return annotated, the call is typed by `crate::expr` and runs.
const CLASS_SHADOW_ANNOTATED_HELPER: &str = "\
class frozenset:
    def __init__(self, a: int) -> None:
        self.a = a


def _f(a: int) -> frozenset:
    return frozenset(a)


def main() -> None:
    x = _f(3)
    print(x.a)


main()
";

/// The same helper left unannotated reaches the constraint solver, whose
/// frozenset arm skips a class-shadowed name and falls through to the
/// pre-existing known-callable-builtin `C0001` (CPython prints `3`). Pinned
/// so a change to that fallback is deliberate.
const CLASS_SHADOW_UNANNOTATED_HELPER: &str = "\
class frozenset:
    def __init__(self, a: int) -> None:
        self.a = a


def _f(a):
    return frozenset(a)


def main() -> None:
    x = _f(3)
    print(x.a)


main()
";

/// Builds `source` natively, runs it, and asserts its stdout equals
/// CPython's run of the same file and `expected`.
fn assert_native_matches_cpython(tag: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("a.py"))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(stdout(&run), stdout(&oracle));
    assert_eq!(stdout(&run), expected);
}

/// Builds `source` as the extension module `module`, imports it in CPython,
/// and asserts the import's stdout equals `runpy.run_path` of the same
/// source and `expected`.
fn assert_ext_matches_cpython(tag: &str, module: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("m.py"), source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("m.py"))
        .arg("-o")
        // A bare name gains the host's own suffix (`.abi3.so` or `.pyd`).
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = |script: String| {
        python()
            .arg("-c")
            .arg(script)
            .current_dir(&*dir)
            .output()
            .expect("python3 should spawn")
    };
    let compiled = run(format!("import {module}"));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    let oracle = run("import runpy\nrunpy.run_path('m.py')".to_string());
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(stdout(&compiled), stdout(&oracle));
    assert_eq!(stdout(&compiled), expected);
}

/// Runs `pycc check` on `source` and asserts it fails with exactly one
/// `code` diagnostic containing `needle`.
fn assert_one_error(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(dir.join("a.py"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{source:?} was accepted");
    let text = rendered(&output);
    assert_eq!(text.matches("error[").count(), 1, "{text}");
    assert!(text.contains(&format!("error[{code}]: {needle}")), "{text}");
}

#[test]
fn frozenset_programs_match_cpython() {
    assert_native_matches_cpython("e2e_1326_success", SUCCESS, SUCCESS_STDOUT);
}

#[test]
fn a_user_frozenset_function_or_class_shadows_the_builtin() {
    assert_native_matches_cpython("e2e_1326_fn_shadow", FUNCTION_SHADOW, "3 5\n");
    assert_native_matches_cpython("e2e_1326_class_shadow", CLASS_SHADOW, "3\n4\n");
    assert_native_matches_cpython(
        "e2e_1326_class_shadow_annotated_helper",
        CLASS_SHADOW_ANNOTATED_HELPER,
        "3\n",
    );
}

#[test]
fn a_class_shadowed_frozenset_in_an_unannotated_helper_is_still_c0001() {
    assert_one_error(
        "e2e_1326_class_shadow_unannotated_helper",
        CLASS_SHADOW_UNANNOTATED_HELPER,
        "C0001",
        "call to builtin `frozenset` is valid Python but not implemented yet",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn frozenset_module_bodies_match_cpython_as_extensions() {
    assert_ext_matches_cpython("e2e_1326_ext", "pycc_fs_mod", SUCCESS, SUCCESS_STDOUT);
    assert_ext_matches_cpython(
        "e2e_1326_ext_fn_shadow",
        "pycc_fs_fn_mod",
        FUNCTION_SHADOW,
        "3 5\n",
    );
    assert_ext_matches_cpython(
        "e2e_1326_ext_class_shadow",
        "pycc_fs_class_mod",
        CLASS_SHADOW,
        "3\n4\n",
    );
}

#[test]
fn an_unsupported_element_type_is_t0038() {
    assert_one_error(
        "e2e_1326_str_annotation",
        "x: frozenset[str] = frozenset()\n",
        "T0038",
        "frozenset[str] is not compiled yet (D-122) -- only frozenset[int] and a frozenset of a user-class instance are",
    );
    // The set literal's own gate refuses the source before `frozenset`
    // sees it.
    assert_one_error(
        "e2e_1326_str_source",
        "x = frozenset({\"a\"})\n",
        "T0038",
        "set[str] is not compiled yet (D-122) -- only set[int] and a set of a user-class instance are",
    );
}

#[test]
fn a_wrong_argument_to_frozenset_is_t0021() {
    assert_one_error(
        "e2e_1326_int_arg",
        "x = frozenset(1)\n",
        "T0021",
        "`frozenset()` argument must be a `set[int]`, `frozenset[int]` or `list[int]` \
         here, got `int`",
    );
    assert_one_error(
        "e2e_1326_str_list_arg",
        "x = frozenset([\"a\"])\n",
        "T0021",
        "`frozenset()` argument must be a `set[int]`, `frozenset[int]` or `list[int]` \
         here, got `list[str]`",
    );
    assert_one_error(
        "e2e_1326_two_args",
        "x = frozenset([1], [2])\n",
        "T0021",
        "`frozenset` expects at most 1 argument, got 2",
    );
}

#[test]
fn a_frozenset_is_immutable_and_distinct_from_a_set() {
    assert_one_error(
        "e2e_1326_add",
        "fs = frozenset([1])\nfs.add(2)\n",
        "T0033",
        "`frozenset[int]` does not support `.add()`",
    );
    assert_one_error(
        "e2e_1326_set_into_frozenset",
        "x: frozenset[int] = {1}\n",
        "T0025",
        "cannot assign `set[int]` to `x: frozenset[int]`",
    );
    assert_one_error(
        "e2e_1326_frozenset_into_set",
        "x: set[int] = frozenset([1])\n",
        "T0025",
        "cannot assign `frozenset[int]` to `x: set[int]`, initializer does not match the declared annotation",
    );
    assert_one_error(
        "e2e_1326_frozenset_reassigned_over_set",
        "x: set[int] = {1}\nx = frozenset([1])\n",
        "T0023",
        "cannot assign `frozenset[int]` to `x`, previously inferred as `set[int]`",
    );
    assert_one_error(
        "e2e_1326_frozenset_argument_for_set",
        "def f(s: set[int]) -> int:\n    return len(s)\n\n\nprint(f(frozenset([1])))\n",
        "T0021",
        "argument 1 of `f` expects `set[int]`, got `frozenset[int]`",
    );
}

#[test]
fn printing_a_set_or_frozenset_is_a_clean_c0001() {
    for (tag, source, ty) in [
        (
            "e2e_1326_print",
            "fs = frozenset([1])\nprint(fs)\n",
            "frozenset[int]",
        ),
        (
            "e2e_1326_fstring",
            "fs = frozenset([1])\nprint(f\"{fs}\")\n",
            "frozenset[int]",
        ),
        ("e2e_1326_print_set", "print({1})\n", "set[int]"),
    ] {
        assert_one_error(
            tag,
            source,
            "C0001",
            &format!("printing or formatting a `{ty}` is not supported yet"),
        );
    }
}

#[test]
fn unsupported_operators_on_a_frozenset_are_t0021() {
    assert_one_error(
        "e2e_1326_not",
        "fs = frozenset([1])\nprint(not fs)\n",
        "T0021",
        "unary operator Not is not defined for `frozenset[int]`",
    );
    assert_one_error(
        "e2e_1326_or",
        "fs = frozenset([1])\ny = fs | fs\n",
        "T0021",
        "operator `|` on `frozenset[int]` and `frozenset[int]` is not supported yet",
    );
}

#[test]
fn a_frozenset_crossing_the_extension_boundary_is_c0003() {
    for (tag, source) in [
        (
            "e2e_1326_ext_param",
            "def f(x: frozenset[int]) -> int:\n    return len(x)\n",
        ),
        (
            "e2e_1326_ext_return",
            "def f() -> frozenset[int]:\n    return frozenset()\n",
        ),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        std::fs::write(dir.join("m.py"), source).expect("write the subject");
        let build = pycc()
            .arg("build")
            .arg(dir.join("m.py"))
            .arg("-o")
            .arg(dir.join("pycc_fs_boundary"))
            .arg("--ext")
            .output()
            .expect("pycc should spawn");
        let text = rendered(&build);
        assert_eq!(build.status.code(), Some(1), "{text}");
        assert!(text.contains("error[C0003]"), "{text}");
        assert!(text.contains("frozenset"), "{text}");
    }
}
