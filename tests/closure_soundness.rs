//! Function values against CPython's late binding and object lifetimes.
//!
//! A nested function used as a value binds its captured variables when the
//! value is made; CPython reads them when it runs. Each program here
//! rebinds a captured variable afterwards (by every binding form), uses
//! the value before its `def`, or rebinds the variable holding a function
//! value while its call's arguments run. Strict mode must refuse it or
//! match CPython; non-strict mode must match CPython (falling back where
//! it refuses). A program naming the compiler's internal names is refused
//! in strict mode (E0109) and runs that code in CPython in non-strict mode.

use pycc_scratch::ScratchDir;
use std::process::Command;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

/// Builds `source` with `flags`; `None` when the build refuses it (with a
/// diagnostic, never a crash), else the run compared with CPython's.
fn build_and_compare(source: &str, flags: &[&str]) -> Option<()> {
    let dir = ScratchDir::new("closure_soundness").expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write");
    let out = dir.join("subject");
    let build = pycc()
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .args(flags)
        .output()
        .expect("pycc should spawn");
    if !build.status.success() {
        let report = format!("{}{}", text(&build.stdout), text(&build.stderr));
        assert!(
            report.contains("error[") && build.status.code() == Some(1),
            "{source}\n{report}"
        );
        return None;
    }
    let ours = Command::new(&out).output().expect("the program should run");
    let theirs = python().arg(&path).output().expect("python3 should spawn");
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{source}");
    assert_eq!(
        last_line(&ours.stderr),
        last_line(&theirs.stderr),
        "{source}"
    );
    assert_eq!(ours.status.code(), theirs.status.code(), "{source}");
    Some(())
}

/// Strict mode refuses or matches CPython; non-strict mode matches it.
fn sound(source: &str) {
    build_and_compare(source, &[]);
    assert!(
        build_and_compare(source, &["--non-strict"]).is_some(),
        "non-strict mode refused:\n{source}"
    );
}

const PRELUDE: &str = "from typing import Callable\n\n\n\
def apply(g: Callable[[], int]) -> int:\n    return g()\n\n\n\
class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n";

/// `get` captures `k`; `g = get` makes the value; `rebind` then rebinds
/// `k`, and the call must see the new binding (or its deletion).
fn rebound_after_value(rebind: &str) -> String {
    let rebind: String = rebind.lines().map(|line| format!("    {line}\n")).collect();
    format!(
        "{PRELUDE}def f() -> int:\n    k = 1\n\n    def get() -> int:\n        return k\n\n    \
         g = get\n{rebind}    return apply(g)\n\n\nprint(f())\n"
    )
}

#[test]
fn a_captured_variable_rebound_by_any_binding_form_is_seen() {
    for rebind in [
        "k = 5",
        "for k in [5]:\n    pass",
        "print([(k := x) for x in [5]])",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as k:\n    pass",
        "match 5:\n    case k:\n        pass",
        "match 5:\n    case 5 as k:\n        pass\n    case _:\n        pass",
        "match [4, 5]:\n    case [x, *k]:\n        pass\n    case _:\n        pass",
        "match {1: 2}:\n    case {**k}:\n        pass\n    case _:\n        pass",
        "match P(5):\n    case P(x=k):\n        pass\n    case _:\n        pass",
        "type k = int",
    ] {
        sound(&rebound_after_value(rebind));
    }
}

/// A function value made before its `def` runs: CPython raises (an
/// `UnboundLocalError`, or a `NameError` for a use inside a function
/// defined earlier), before any captured variable it reads is assigned.
#[test]
fn a_function_value_used_before_its_def_raises_like_cpython() {
    sound(&format!(
        "{PRELUDE}def f() -> int:\n    k = 1\n    r = apply(get)\n\n    def get() -> int:\n        \
         return k\n\n    return r\n\n\nprint(f())\n"
    ));
    sound(&format!(
        "{PRELUDE}def f() -> int:\n    def h() -> Callable[[], int]:\n        return get\n\n    \
         r = h()\n    k = 1\n\n    def get() -> int:\n        return k\n\n    return r()\n\n\n\
         print(f())\n"
    ));
}

/// The variable holding the called function value is rebound by an
/// argument: the call still runs the value it read, which stays alive
/// until the call returns.
#[test]
fn rebinding_the_called_variable_in_an_argument_matches_cpython() {
    let make = "from typing import Callable\n\n\n\
def make_adder(n: int) -> Callable[[int], int]:\n    def add(x: int) -> int:\n        \
return x + n\n\n    return add\n\n\n\
def ident(x: int) -> int:\n    return x\n\n\n";
    sound(&format!(
        "{make}f: Callable[[int], int] = make_adder(10**30)\n\n\n\
def rebind() -> int:\n    global f\n    f = ident\n    return 2\n\n\n\
print(f(rebind()))\nprint(f(3))\n"
    ));
    sound(&format!(
        "{make}def outer() -> None:\n    g = make_adder(10**30)\n\n    def rb() -> int:\n        \
nonlocal g\n        g = ident\n        return 5\n\n    print(g(rb()))\n    print(g(7))\n\n\n\
outer()\n"
    ));
}

/// The compiler's own names cannot be bound or read by compiled code: no
/// forged function value, no aliased temporary. Strict mode refuses them;
/// non-strict mode runs the statements naming them in CPython.
#[test]
fn reserved_names_are_refused_or_run_in_cpython() {
    for source in [
        "def add(a: int, x: int) -> int:\n    return a + x\n\n\n\
         f = __pycc_closure__(add, 1, 2, 3)\n",
        "def _pycc_f_get(k: int) -> int:\n    return k\n\n\nprint(_pycc_f_get(1))\n",
        "_pycc_0 = 1\nprint(_pycc_0)\n",
    ] {
        let dir = ScratchDir::new("closure_soundness_reserved").expect("scratch");
        let path = dir.join("subject.py");
        std::fs::write(&path, source).expect("write");
        let build = pycc()
            .arg("build")
            .arg(&path)
            .arg("-o")
            .arg(dir.join("subject"))
            .output()
            .expect("pycc should spawn");
        let report = format!("{}{}", text(&build.stdout), text(&build.stderr));
        assert_eq!(build.status.code(), Some(1), "{source}\n{report}");
        assert!(report.contains("error[E0109]"), "{source}\n{report}");
        assert!(
            build_and_compare(source, &["--non-strict"]).is_some(),
            "non-strict mode refused:\n{source}"
        );
    }
}
