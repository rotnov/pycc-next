//! `break`, `continue`, and a loop's `else:` clause.
//!
//! A `break`/`continue` is admitted in every loop kind (`while`, `for` over
//! a `range`, a list, a dict, a set, and a CPython object) at any nesting
//! depth, including inside `if`/`elif` chains and `try`/`except` bodies and
//! handlers; a `while`/`for` loop's `else:` clause runs only when the loop
//! finished without a `break`. The one refused shape is a jump that would
//! leave a `try` with a `finally` clause (`C0001`, pinned by the
//! `c0001_*_across_finally` diagnostic fixtures).
//!
//! The native cases need no interpreter: each expected output is CPython
//! 3.13's own for the same source, recorded here. The `--ext` cases compare
//! live against the host interpreter and are `#[ignore]`d like every hosted
//! test.

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

/// Functions covering every loop kind with `break`, `continue` and `else:`,
/// nested loops, jumps inside `if`/`elif` chains and `try`/`except`, and
/// loop bodies that allocate lists and strings on every trip.
const SOURCE: &str = r#"def break_in_while(n: int) -> int:
    i: int = 0
    while True:
        i += 1
        if i >= n:
            break
    return i


def continue_in_while(n: int) -> int:
    i: int = 0
    total: int = 0
    while i < n:
        i += 1
        if i % 2 == 0:
            continue
        total += i
    return total


def break_in_range(n: int) -> str:
    out: str = ""
    for i in range(n):
        if i == 5:
            break
        out = out + f"{i},"
    return out


def continue_in_range(n: int) -> str:
    out: str = ""
    for i in range(1, n, 2):
        if i % 3 == 0:
            continue
        out = out + f"{i};"
    return out


def break_in_list(stop: int) -> int:
    xs: list[int] = [4, 8, 15, 16, 23, 42]
    seen: int = 0
    for x in xs:
        if x == stop:
            break
        seen += 1
    return seen


def continue_in_list(limit: int) -> int:
    xs: list[int] = [3, -1, 7, -9, 11, -2]
    total: int = 0
    for x in xs:
        if x < 0:
            continue
        if x > limit:
            continue
        total += x
    return total


def nested_break(n: int) -> str:
    out: str = ""
    for i in range(n):
        for j in range(n):
            if j == i:
                break
            out = out + f"{i}{j} "
        out = out + "| "
    return out


def nested_continue(n: int) -> int:
    hits: int = 0
    i: int = 0
    while i < n:
        i += 1
        for j in range(n):
            if (i + j) % 2 == 0:
                continue
            hits += j
        if i == 2:
            continue
        hits += 100
    return hits


def while_else(n: int, stop: int) -> str:
    i: int = 0
    while i < n:
        if i == stop:
            break
        i += 1
    else:
        return f"exhausted at {i}"
    return f"stopped at {i}"


def for_else(target: int) -> str:
    xs: list[int] = [2, 4, 6, 8]
    result: str = "unset"
    for x in xs:
        if x == target:
            result = f"found {x}"
            break
    else:
        result = "missing"
    return result


def for_range_else(n: int) -> int:
    total: int = 0
    for i in range(n):
        total += i
    else:
        total += 1000
    return total


def search(target: int) -> int:
    for i in range(10):
        if i * i == target:
            return i
    else:
        return -1


def break_in_if_chain(n: int) -> str:
    out: str = ""
    k: int = 0
    while k < n:
        k += 1
        if k < 3:
            out = out + "a"
        elif k == 4:
            out = out + "b"
            continue
        elif k == 6:
            out = out + "c"
            break
        else:
            out = out + "d"
    return out + f"{k}"


def risky(v: int) -> int:
    if v == 3:
        raise ValueError("three")
    return v * 2


def try_in_loop(n: int) -> str:
    out: str = ""
    r: int = 0
    for i in range(n):
        try:
            r = risky(i)
            if r == 2:
                continue
            if r > 9:
                break
        except ValueError as e:
            out = out + f"<{e}>"
            continue
        out = out + f"{r} "
    return out


def allocating_loop(n: int) -> str:
    text: str = ""
    total: int = 0
    i: int = 0
    while True:
        i += 1
        row: list[int] = [i, i * 2, i * 3]
        label: str = "row" + f"{i}"
        if i % 3 == 0:
            continue
        if i > n:
            break
        row.append(i % 4)
        total += row[3] + len(row)
        text = text + label + "."
    return f"{total} {text}"


def allocating_for(n: int) -> int:
    total: int = 0
    for i in range(n):
        chunk: list[int] = [i, i]
        words: str = f"{i}" + "-" + f"{i * i}"
        if i * i > 50:
            break
        if i % 2 == 1:
            continue
        chunk.append(i)
        total += len(chunk) + i
    return total


def dict_and_set_loops() -> str:
    d: dict[str, int] = {"a": 1, "b": 2, "c": 3, "d": 4}
    out: str = ""
    for k in d:
        if k == "b":
            continue
        if k == "d":
            break
        out = out + k
    s: set[int] = {1, 2, 3, 4}
    total: int = 0
    for v in s:
        if v == 2:
            continue
        total += v
    else:
        total += 100
    return out + f"{total}"
"#;

/// Module-level loops, compiled only in the native build (an `--ext`
/// module's body runs at import, before the script can print).
const TOP_LEVEL: &str = r#"count: int = 0
while count < 10:
    count += 1
    if count == 2:
        continue
    if count == 5:
        break
    print("while", count)
else:
    print("while else skipped")

values: list[int] = [1, 2, 3]
for v in values:
    print("list", v)
else:
    print("list else ran")

for a in range(3):
    for b in range(2):
        if b > a:
            break
        print("pair", a, b)
    else:
        print("inner else", a)
    if a == 1:
        continue
    print("outer", a)

n: int = 0
while n < 3:
    n += 1
else:
    print("while else ran", n)
"#;

/// Calls every function in [`SOURCE`] and prints the results.
const SCRIPT: &str = r#"print(break_in_while(4), break_in_while(0))
print(continue_in_while(9))
print(break_in_range(9), break_in_range(3))
print(continue_in_range(16))
print(break_in_list(16), break_in_list(99))
print(continue_in_list(10))
print(nested_break(4))
print(nested_continue(4))
print(while_else(5, 2), "/", while_else(5, 9))
print(for_else(6), "/", for_else(5))
print(for_range_else(4), for_range_else(0))
print(search(49), search(50))
print(break_in_if_chain(10), break_in_if_chain(3))
print(try_in_loop(8))
print(allocating_loop(20))
print(allocating_for(1000))
print(dict_and_set_loops())
"#;

/// CPython's output for `SOURCE` + `SCRIPT`.
const EXPECTED_FUNCTIONS: &str = r#"4 1
25
0,1,2,3,4, 0,1,2,
1;5;7;11;13;
3 6
10
| 10 | 20 21 | 30 31 32 | 
312
stopped at 2 / exhausted at 5
found 6 / missing
1006 1000
7 -1
aadbdc6 aad3
0 4 <three>8 
75 row1.row2.row4.row5.row7.row8.row10.row11.row13.row14.row16.row17.row19.row20.
24
ac108
"#;

/// CPython's output for `TOP_LEVEL`.
const EXPECTED_TOP_LEVEL: &str = r#"while 1
while 3
while 4
list 1
list 2
list 3
list else ran
pair 0 0
outer 0
pair 1 0
pair 1 1
inner else 1
pair 2 0
pair 2 1
inner else 2
outer 2
while else ran 3
"#;

fn build_native(dir: &Path, source: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    binary
}

fn check(dir: &Path, source: &str) -> Output {
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn")
}

/// Every function and every module-level loop builds natively and prints
/// the recorded CPython output, which the live oracle reproduces too.
#[test]
fn break_continue_and_loop_else_run_like_cpython() {
    let dir = ScratchDir::new("loop_control_native").expect("scratch");
    let program = format!("{SOURCE}{TOP_LEVEL}{SCRIPT}");
    let binary = build_native(&dir, &program);
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", stderr_of(&run));
    let expected = format!("{EXPECTED_TOP_LEVEL}{EXPECTED_FUNCTIONS}");
    assert_eq!(stdout_of(&run), expected);

    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let cpython = Command::new(oracle)
        .arg(dir.join("m.py"))
        .output()
        .expect("python3 should spawn");
    assert!(cpython.status.success(), "{}", stderr_of(&cpython));
    assert_eq!(stdout_of(&cpython), expected);
}

/// The same program also builds with `--release`, whose optimizer sees the
/// loop latches and exit blocks the jumps target.
#[test]
fn break_continue_and_loop_else_run_like_cpython_optimized() {
    let dir = ScratchDir::new("loop_control_release").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, format!("{SOURCE}{TOP_LEVEL}{SCRIPT}")).expect("write the fixture source");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(
        stdout_of(&run),
        format!("{EXPECTED_TOP_LEVEL}{EXPECTED_FUNCTIONS}")
    );
}

/// A `range` loop over bigint bounds owns a heap word per trip; leaving it
/// by `break` releases the current word in the loop's exit block and a
/// `continue` advances through the same latch as a normal trip, so neither
/// frees a word still in use.
#[test]
fn a_bigint_range_loop_survives_break_and_continue() {
    let dir = ScratchDir::new("loop_control_bigint").expect("scratch");
    let binary = build_native(
        &dir,
        "big: int = 10 ** 30\nc: int = 0\nfor z in range(big, big + 6):\n    c += 1\n    if c == 2:\n        continue\n    if c == 5:\n        break\n    print(z)\nprint(c)\n",
    );
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(
        stdout_of(&run),
        "1000000000000000000000000000000\n1000000000000000000000000000002\n1000000000000000000000000000003\n5\n"
    );
}

/// A function whose loop `else:` clause returns, with no `break` bound to
/// the loop, does not fall off its end; one whose loop can `break` does.
#[test]
fn a_returning_loop_else_without_break_satisfies_return_coverage() {
    let dir = ScratchDir::new("loop_control_t0022").expect("scratch");
    let ok = check(
        &dir,
        "def f(n: int) -> int:\n    for i in range(n):\n        if i == 3:\n            return i\n    else:\n        return -1\n",
    );
    assert!(ok.status.success(), "{}", stdout_of(&ok));
    let missing = check(
        &dir,
        "def f(n: int) -> int:\n    for i in range(n):\n        if i == 3:\n            break\n    else:\n        return -1\n",
    );
    assert!(!missing.status.success());
    assert!(
        stdout_of(&missing).contains("error[T0022]: function `f` can exit without returning `int`"),
        "{}",
        stdout_of(&missing)
    );
}

/// A loop over an enum class is unrolled into one copy of its body per
/// member, so a `break` or `continue` bound to it is refused with `C0001`.
#[test]
fn a_jump_in_an_enum_loop_is_not_supported_yet() {
    let dir = ScratchDir::new("loop_control_enum").expect("scratch");
    let out = check(
        &dir,
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n    GREEN = 2\n\n\nfor c in Color:\n    break\n",
    );
    assert!(!out.status.success());
    assert!(
        stdout_of(&out).contains(
            "error[C0001]: statement kind not supported yet: `break` or `continue` in a loop over the enum class `Color`"
        ),
        "{}",
        stdout_of(&out)
    );
}

/// Builds `source` as the extension module `module` and runs `script`
/// against it, then runs the same script against CPython importing the
/// same source, and returns both outputs.
fn hosted_and_cpython(tag: &str, module: &str, source: &str, script: &str) -> (String, String) {
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let hosted = ScratchDir::new(&format!("{tag}_ext")).expect("scratch");
    let src = hosted.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(hosted.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    let run = Command::new(&oracle)
        .arg("-c")
        .arg(script)
        .current_dir(&*hosted)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));

    let reference = ScratchDir::new(&format!("{tag}_cpython")).expect("scratch");
    std::fs::write(reference.join(format!("{module}.py")), source)
        .expect("write the oracle source");
    let cpython = Command::new(&oracle)
        .arg("-c")
        .arg(script)
        .current_dir(&*reference)
        .output()
        .expect("python3 should spawn");
    assert!(cpython.status.success(), "{}", stderr_of(&cpython));
    (stdout_of(&run), stdout_of(&cpython))
}

/// The functions built as an extension module and called from the host,
/// compared with CPython importing the same source.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn break_continue_and_loop_else_run_like_cpython_as_an_extension() {
    let script = format!("from pycc_loopctl import *\n{SCRIPT}");
    let (hosted, cpython) = hosted_and_cpython("loop_control", "pycc_loopctl", SOURCE, &script);
    assert_eq!(hosted, EXPECTED_FUNCTIONS);
    assert_eq!(cpython, EXPECTED_FUNCTIONS);
}

/// A module-body loop over a CPython object (`for x in o.method():`)
/// with `break`, `continue` and `else:`.
const OBJECT_LOOPS: &str = r#"import string

seen: int = 0
skipped: int = 0
for ch in string.digits.upper():
    if ch == "3":
        skipped += 1
        continue
    if ch == "7":
        break
    seen += 1
else:
    seen = -1

completed: int = 0
for ch in string.ascii_lowercase.upper():
    completed += 1
else:
    completed += 1000


def report() -> str:
    return f"{seen} {skipped} {completed}"
"#;

/// `break` and `continue` out of a CPython-object loop release its
/// iterator, so the iterated object's reference count is unchanged.
const OBJECT_REFCOUNT: &str = r#"import string
import sys

parts = string.digits.partition("5")
before = sys.getrefcount(parts)
for p in parts:
    break
after_break = sys.getrefcount(parts)
for p in parts:
    continue
after_continue = sys.getrefcount(parts)


def report() -> str:
    return f"{before} {after_break} {after_continue}"
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_loops_break_and_continue_like_cpython_as_an_extension() {
    let (hosted, cpython) = hosted_and_cpython(
        "loop_control_object",
        "pycc_objloop",
        OBJECT_LOOPS,
        "import pycc_objloop\nprint(pycc_objloop.report())\n",
    );
    assert_eq!(hosted, "6 1 1026\n");
    assert_eq!(cpython, hosted);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_object_loop_jump_releases_its_iterator() {
    let (hosted, cpython) = hosted_and_cpython(
        "loop_control_refcount",
        "pycc_rc",
        OBJECT_REFCOUNT,
        "import pycc_rc\nprint(pycc_rc.report())\n",
    );
    let counts: Vec<&str> = hosted.split_whitespace().collect();
    assert_eq!(counts.len(), 3, "{hosted}");
    assert!(counts.iter().all(|c| *c == counts[0]), "{hosted}");
    let reference: Vec<&str> = cpython.split_whitespace().collect();
    assert!(reference.iter().all(|c| *c == reference[0]), "{cpython}");
}
