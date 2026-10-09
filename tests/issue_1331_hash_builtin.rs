//! End-to-end proof for the `hash()` builtin over `int`, `bool` and tuples
//! of them (#1331, Part 1 of #1327).
//!
//! `docs/TYPE_SYSTEM.md`'s `hash()` section is the contract. Every expected
//! value is CPython 3.14's own on a 64-bit build, and every program also runs
//! under whatever `PYCC_PYTHON`/`python3` is available, as
//! `tests/issue_1326_frozenset.rs` does. The `--ext` differential imports the
//! built extension in CPython and compares it with `runpy.run_path` on the
//! same source; it is `#[ignore]`d like its siblings and runs under
//! `--include-ignored` in CI.

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

/// The pinned values: `-1 -> -2`, the `2**61 - 1` modulus wrap of both
/// signs, the smallint/bigint boundary, heap bigints built by addition,
/// bools, tuple hashes (including a heap-bigint element), a `bool` passed to
/// an `int` parameter, an unannotated private helper typed by the constraint
/// path, and a bigint hashed repeatedly without being consumed.
const SUCCESS: &str = "\
def _h(y):
    return hash(y)


def f(n: int) -> int:
    return hash(n)


def big_twice(k: int) -> None:
    b = 4611686018427387904
    for i in range(k):
        b = b + b
    print(hash(b), hash(b) == hash(b))


print(hash(-1), hash(-2), hash(0), hash(True), hash(False))
print(hash(2305843009213693951), hash(2305843009213693952), hash(-2305843009213693952))
print(hash(4611686018427387903), hash(-4611686018427387904))
p = 4611686018427387904
print(hash(p), hash(p + p), hash(p + p + p + p))
print(hash((-1,)), hash((1, 2)), hash((True, False)), hash((1, -1)))
t = (p, 3)
print(hash(t))
print(f(True), f(7), _h(9))
big_twice(3)
";

const SUCCESS_STDOUT: &str = "-2 -2 0 1 0\n0 1 -2\n1 -2\n2 4 8\n\
                              8078679518589016365 -3550055125485641917 \
                              -5164621852614943976 -6779188579744246035\n\
                              8409376899596376432\n1 7 9\n16 True\n";

/// A user `def hash` shadows the builtin, including for an unannotated
/// private helper defined before it, which the constraint path types.
const FUNCTION_SHADOW: &str = "\
def _h(a):
    return hash(a)


def hash(x: int) -> int:
    return x + 1


print(hash(2), _h(4))
";

/// A user `class hash:` keeps compiling as the class it is.
const CLASS_SHADOW: &str = "\
class hash:
    def __init__(self, x: int) -> None:
        self.x = x


h = hash(3)
print(h.x)
";

/// `class hash:` reaching the constraint path through an unannotated helper:
/// the solver yields to the user class, and the class-call is not yet typed
/// there, the pre-existing `C0001` every builtin-named class gets.
const CLASS_SHADOW_UNANNOTATED_HELPER: &str = "\
class hash:
    def __init__(self, x: int) -> None:
        self.x = x


def _g(y):
    return hash(y)


print(_g(3).x)
";

/// A stdlib module alias spelled `hash` is the module, not the builtin.
const MODULE_ALIAS: &str = "import math as hash\nprint(hash(3))\n";

const MODULE_ALIAS_IN_HELPER: &str = "\
import math as hash


def _g(y):
    return hash(y)


print(_g(3))
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
fn hash_programs_match_cpython() {
    assert_native_matches_cpython("e2e_1331_success", SUCCESS, SUCCESS_STDOUT);
}

#[test]
fn a_user_hash_function_or_class_shadows_the_builtin() {
    assert_native_matches_cpython("e2e_1331_fn_shadow", FUNCTION_SHADOW, "3 5\n");
    assert_native_matches_cpython("e2e_1331_class_shadow", CLASS_SHADOW, "3\n");
}

#[test]
fn a_class_shadowed_hash_in_an_unannotated_helper_is_still_c0001() {
    assert_one_error(
        "e2e_1331_class_shadow_unannotated_helper",
        CLASS_SHADOW_UNANNOTATED_HELPER,
        "C0001",
        "call to builtin `hash` is valid Python but not implemented yet",
    );
}

#[test]
fn a_module_alias_spelled_hash_is_not_the_builtin() {
    let needle = "call to builtin `hash` is valid Python but not implemented yet";
    assert_one_error("e2e_1331_alias", MODULE_ALIAS, "C0001", needle);
    assert_one_error(
        "e2e_1331_alias_helper",
        MODULE_ALIAS_IN_HELPER,
        "C0001",
        needle,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn hash_module_bodies_match_cpython_as_extensions() {
    assert_ext_matches_cpython("e2e_1331_ext", "pycc_hash_mod", SUCCESS, SUCCESS_STDOUT);
    assert_ext_matches_cpython(
        "e2e_1331_ext_fn_shadow",
        "pycc_hash_fn_mod",
        FUNCTION_SHADOW,
        "3 5\n",
    );
    assert_ext_matches_cpython(
        "e2e_1331_ext_class_shadow",
        "pycc_hash_class_mod",
        CLASS_SHADOW,
        "3\n",
    );
}

#[test]
fn a_hashable_but_unimplemented_argument_is_c0001() {
    for (tag, argument, name) in [
        ("e2e_1331_str", "\"s\"", "str"),
        ("e2e_1331_float", "1.5", "float"),
        ("e2e_1331_float_tuple", "(1.5, 2)", "tuple[float, int]"),
        ("e2e_1331_none", "None", "None"),
    ] {
        assert_one_error(
            tag,
            &format!("print(hash({argument}))\n"),
            "C0001",
            &format!("`hash()` of `{name}` is valid Python but not implemented yet"),
        );
    }
    assert_one_error(
        "e2e_1331_frozenset",
        "s = frozenset([1])\nprint(hash(s))\n",
        "C0001",
        "`hash()` of `frozenset[int]` is valid Python but not implemented yet",
    );
    // A class instance is hashed since #1335: `tests/issue_1335_hash_instance.rs`.
    // An unannotated helper whose parameter the constraint path infers
    // from its call site: the argument's type is resolved only after the
    // solver runs, and the final check still refuses it.
    for (tag, argument, name) in [
        ("e2e_1331_helper_str", "\"s\"", "str"),
        ("e2e_1331_helper_float", "1.5", "float"),
    ] {
        assert_one_error(
            tag,
            &format!("def _h(y):\n    return hash(y)\n\n\nprint(_h({argument}))\n"),
            "C0001",
            &format!("`hash()` of `{name}` is valid Python but not implemented yet"),
        );
    }
    assert_one_error(
        "e2e_1331_helper_literal",
        "def _h():\n    return hash(\"s\")\n\n\nprint(_h())\n",
        "C0001",
        "`hash()` of `str` is valid Python but not implemented yet",
    );
}

#[test]
fn a_list_dict_or_set_is_t0021_unhashable() {
    for (tag, argument, name) in [
        ("e2e_1331_list", "[1]", "list[int]"),
        ("e2e_1331_dict", "{\"a\": 2}", "dict[str, int]"),
        ("e2e_1331_set", "{1}", "set[int]"),
    ] {
        assert_one_error(
            tag,
            &format!("print(hash({argument}))\n"),
            "T0021",
            &format!("unhashable type: `{name}`"),
        );
    }
    assert_one_error(
        "e2e_1331_helper_list",
        "def _h(y):\n    return hash(y)\n\n\nprint(_h([1]))\n",
        "T0021",
        "unhashable type: `list[int]`",
    );
}

#[test]
fn a_wrong_argument_count_is_t0021() {
    assert_one_error(
        "e2e_1331_no_args",
        "print(hash())\n",
        "T0021",
        "`hash` expects exactly 1 argument, got 0",
    );
    assert_one_error(
        "e2e_1331_two_args",
        "print(hash(1, 2))\n",
        "T0021",
        "`hash` expects exactly 1 argument, got 2",
    );
    assert_one_error(
        "e2e_1331_helper_two_args",
        "def _h(y):\n    return hash(y, y)\n\n\nprint(_h(1))\n",
        "T0021",
        "`hash` expects exactly 1 argument, got 2",
    );
}

#[cfg(unix)]
mod peak_rss {
    use super::{ScratchDir, pycc};
    use std::process::Command;

    /// `2**62`, the smallest positive int D-061 stores as a heap bigint.
    const PROMOTED: &str = "4611686018427387904";

    /// The child's `ru_maxrss` (bytes on macOS, kilobytes on Linux, so only
    /// ever compared as a ratio). Follows `tests/issue_1211_bool_ops.rs`.
    #[allow(clippy::zombie_processes)]
    fn peak_rss(mut command: Command) -> libc::c_long {
        let child = command.spawn().expect("command must spawn");
        let pid = child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        let waited = loop {
            // SAFETY: `pid` is the live child spawned above, and both output
            // pointers are valid for writes for the duration of the call.
            let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
            if result != -1 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(error.kind(), std::io::ErrorKind::Interrupted, "{error}");
        };
        assert_eq!(waited, pid);
        // SAFETY: the successful `wait4` above initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
        usage.ru_maxrss
    }

    /// A fresh heap-bigint argument hashed and dropped each iteration, and a
    /// tuple hash that leaves the smallint range (a heap-bigint result).
    fn repro(iterations: u32) -> String {
        format!(
            "base = {PROMOTED}\nacc = 0\n\
             for i in range({iterations}):\n\
             \x20   acc = hash(base + i)\n\
             \x20   acc = hash((i, 1))\n\
             print(acc)\n"
        )
    }

    fn built_program_peak_rss(case: &str, source: &str) -> libc::c_long {
        let dir = ScratchDir::new(case).expect("scratch");
        let src = dir.join("case.py");
        std::fs::write(&src, source).expect("write the case");
        let bin = dir.join("case");
        let build = pycc()
            .arg("build")
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .status()
            .expect("pycc build must spawn");
        assert!(build.success(), "pycc build of {case} failed");
        peak_rss(Command::new(&bin))
    }

    #[test]
    fn hashing_a_bigint_temporary_does_not_grow_with_the_trip_count() {
        let single = built_program_peak_rss("rss_1331_1x", &repro(500_000));
        let double = built_program_peak_rss("rss_1331_2x", &repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }
}
