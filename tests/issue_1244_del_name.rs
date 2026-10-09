//! End-to-end proof for `del name` ([#1244], Part 1 of [#1216]).
//!
//! `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the contract. The
//! byte-exact oracle fixture is `tests/fixtures/del_name.py` (registered in
//! `tests/conformance/classes.rs`); this file owns the refusals, the
//! multi-module rules and the generic-function arm the fixture cannot carry
//! (#1252).
//!
//! [#1244]: https://github.com/rotnov/pycc/issues/1244
//! [#1216]: https://github.com/rotnov/pycc/issues/1216

use pycc_scratch::ScratchDir;
use std::path::Path;
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

/// Builds `dir/<entry>` into `dir/app`, runs it, and returns its standard
/// output.
fn build_and_run(dir: &Path, entry: &str) -> String {
    let build = pycc()
        .arg("build")
        .arg(dir.join(entry))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    stdout(&run)
}

/// [`build_and_run`], plus CPython on the same entry module; returns both
/// standard outputs.
fn build_run_and_oracle(dir: &Path, entry: &str) -> (String, String) {
    let pycc_out = build_and_run(dir, entry);
    let oracle = python()
        .arg(entry)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    (pycc_out, stdout(&oracle))
}

/// Runs `pycc check` on the entry module `a.py` of `files` and returns the
/// rendered diagnostics, asserting that the check failed.
fn check_fails(category: &str, files: &[(&str, &str)]) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    for (name, source) in files {
        std::fs::write(dir.join(name), source).expect("write a module");
    }
    let output = pycc()
        .arg("check")
        .arg(dir.join("a.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{files:?} was accepted");
    rendered(&output)
}

/// A generic function whose body deletes a local is monomorphized and runs
/// as CPython does. It is kept out of the fixture because a generic
/// function beside the fixture's enum loop fails to build today (#1252).
/// The expected text is CPython 3.14.7's; it is not re-run here because the
/// `python3` of an unpinned CI job may predate PEP 695's `def f[T]`.
#[test]
fn a_generic_function_that_deletes_a_local_matches_cpython() {
    let dir = ScratchDir::new("e2e_1244_generic").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "def ident[T](v: T) -> T:\n    copy = v\n    del copy\n    return v\n\n\
         print(ident(4))\nprint(ident(\"s\"))\n",
    )
    .expect("write the subject");
    assert_eq!(build_and_run(&dir, "a.py"), "4\ns\n");
}

/// A module may delete and rebind a top-level name no other module of the
/// program mentions.
#[test]
fn a_module_deletes_a_name_no_other_module_mentions() {
    let dir = ScratchDir::new("e2e_1244_two_modules").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "from b import g\nx = 1\ndel x\nx = 3\nprint(x + g())\n",
    )
    .expect("write the entry module");
    std::fs::write(dir.join("b.py"), "def g() -> int:\n    return 2\n")
        .expect("write the dependency");
    let (pycc_out, cpython_out) = build_run_and_oracle(&dir, "a.py");
    assert_eq!(pycc_out, cpython_out);
    assert_eq!(pycc_out, "5\n");
}

/// The flat top-level namespace every linked module shares: a module-level
/// `del x` is refused while another module mentions `x` (whether it merely
/// defines it or reads its own `x`), and importing a name its defining
/// module deletes is refused.
#[test]
fn the_multi_module_rules_refuse_a_shared_deleted_name() {
    for (entry, dependency, fragments) in [
        (
            "from b import g\ndel x\nprint(g())\n",
            "x = 1\ndef g() -> int:\n    return 2\n",
            [
                "error[C0001]: a module-level `del x` is not supported when another module of \
                 the program (`",
                "b.py`) mentions `x`: every module shares one top-level namespace",
            ],
        ),
        (
            "from b import g\nx = 1\ndel x\nprint(g())\n",
            "x = 5\ndef g() -> int:\n    return x\n",
            [
                "error[C0001]: a module-level `del x` is not supported when another module of \
                 the program (`",
                "b.py`) mentions `x`: every module shares one top-level namespace",
            ],
        ),
        (
            "from b import x\nprint(x)\n",
            "x = 1\ndel x\nx = 2\n",
            [
                "error[C0001]: `",
                "b.py` deletes its top-level `x` with `del`, so importing `x` from it is not \
                 supported",
            ],
        ),
    ] {
        let text = check_fails("e2e_1244_modules", &[("a.py", entry), ("b.py", dependency)]);
        // The diagnostic names the other module by its full path.
        for fragment in fragments {
            assert!(text.contains(fragment), "{entry}: {text}");
        }
    }
}

/// Every `del` refusal names its reason: the HIR target kinds and scopes,
/// the checker's binding-state rule (a later read, a double delete, an
/// unbound name, a builtin) and its allowlist.
#[test]
fn every_del_refusal_is_named() {
    for (source, header) in [
        // Binding state.
        (
            "x = 1\ndel x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "del x\n",
            "error[T0021]: local name `x` is not bound before this use",
        ),
        (
            "a = 1\ndel a, a\n",
            "error[T0041]: local name `a` may not be bound",
        ),
        (
            "del len\n",
            "error[T0021]: local name `len` is not bound before this use",
        ),
        (
            "def f() -> None:\n    del x\n\nf()\n",
            "error[T0021]: local name `x` is not bound before this use",
        ),
        // A function-scope `del` makes `x` local: the module's `x` is not it.
        (
            "x = 1\n\n\ndef f() -> None:\n    del x\n\n\nf()\n",
            "error[T0021]: local name `x` is not bound before this use",
        ),
        // One arm, a loop, a handler, `finally`, `except*`.
        (
            "x = 1\nif len([1]) > 0:\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "def f(c: bool) -> None:\n    x = 1\n    if c:\n        del x\n        return\n    print(x)\n\nf(False)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\ni = 0\nwhile i < 2:\n    i += 1\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\ni = 0\nwhile i < 2:\n    i += 1\n    del x\n    x = 2\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "def f() -> None:\n    x = 1\n    i = 0\n    while i < 1:\n        i += 1\n        del x\n    print(x)\n\nf()\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\nfor i in range(2):\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "def f() -> None:\n    x = 1\n    for i in range(2):\n        del x\n    print(x)\n\nf()\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\nys = [1, 2]\nfor i in ys:\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "def f() -> None:\n    x = 1\n    ys = [1, 2]\n    for i in ys:\n        del x\n    print(x)\n\nf()\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nx = 1\nfor c in Color:\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\ndef f() -> None:\n    x = 1\n    for c in Color:\n        del x\n    print(x)\n\n\nf()\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "import gc\n\nx = 1\nfor o in gc.garbage:\n    del x\nprint(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\ntry:\n    del x\n    raise ValueError(\"e\")\nexcept ValueError:\n    print(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\ntry:\n    raise ValueError(\"e\")\nexcept ValueError:\n    del x\nfinally:\n    print(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "def f(n: int) -> int:\n    x = n\n    try:\n        del x\n        x = 2\n    finally:\n        print(0)\n    return x\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        (
            "x = 1\ntry:\n    raise ValueError(\"a\")\nexcept* ValueError:\n    del x\nexcept* TypeError:\n    print(x)\n",
            "error[T0041]: local name `x` may not be bound",
        ),
        // The sticky representation survives a deletion.
        (
            "x = 1\ndel x\nx = \"s\"\n",
            "error[T0023]: cannot assign `str` to `x`, previously inferred as `int`",
        ),
        // The checker's allowlist.
        (
            "def f() -> int:\n    return 1\n\n\ndel f\n",
            "error[C0001]: a `del` of `f` is not supported yet: it names a function",
        ),
        (
            "def helper() -> int:\n    return 1\n\n\nhelper = 2\ndel helper\n",
            "error[C0001]: a `del` of `helper` is not supported yet: it names a function",
        ),
        (
            "class C:\n    pass\n\n\ndel C\n",
            "error[C0001]: a `del` of `C` is not supported yet: it names a class",
        ),
        (
            "del ValueError\n",
            "error[C0001]: a `del` of `ValueError` is not supported yet: it names a class",
        ),
        (
            "from typing import Final\nX: Final[int] = 1\ndel X\n",
            "error[C0001]: a `del` of `X` is not supported yet: it is declared `Final`",
        ),
        (
            "def f() -> int:\n    a = ndarray(4)\n    del a\n    return 0\n",
            "error[C0001]: a `del` of `a` is not supported yet: it owns a buffer",
        ),
        (
            "import gc\n\nfor o in gc.garbage:\n    del o\n",
            "error[C0001]: a `del` of `o` is not supported yet: it holds a CPython object",
        ),
        // Types: a slice target of a non-`object` base (HIR lowers every
        // slice target since Part 2c of #1371; the type stage refuses it).
        (
            "xs = [1, 2]\ndel xs[0:1]\n",
            "error[C0001]: a `del` of a slice (`del xs[a:b]`) is not supported yet",
        ),
        // Types: an attribute target of a non-`object` base (HIR lowers
        // every attribute target since #1457; the type stage refuses it).
        (
            "class C:\n    def __init__(self) -> None:\n        self.a = 1\n\n\nc = C()\ndel c.a\n",
            "error[C0001]: a `del` of an attribute (`del obj.attr`) is supported only on a \
             CPython object, not on `C`",
        ),
        // HIR: target kinds, scopes and the module-level rules.
        (
            "d = {1: 2}\ndel d[1]\n",
            "error[C0001]: a `del` of a subscript (`del d[k]`, `del xs[i]`) is not supported yet \
             (#1245 for `dict`, #1246 for `list`)",
        ),
        (
            "del __name__\n",
            "error[C0001]: a `del` of the module's `__name__` is not supported",
        ),
        (
            "class C:\n    x = 1\n    del x\n",
            "error[C0001]: a `del` statement in a class body is not supported yet",
        ),
        (
            "class C:\n    def m(self) -> None:\n        del self\n",
            "error[C0001]: a `del` of a method's receiver (`self`) is not supported",
        ),
        (
            "class C:\n    def m(this) -> None:\n        del this\n",
            "error[C0001]: a `del` of a method's receiver (`this`) is not supported",
        ),
        (
            "class C:\n    @classmethod\n    def m(cls) -> None:\n        del cls\n",
            "error[C0001]: a `del` of a method's receiver (`cls`) is not supported",
        ),
        (
            "import math\ndel math\n",
            "error[C0001]: a `del` of the imported name `math` is not supported yet",
        ),
        (
            "from math import sqrt\ndel sqrt\n",
            "error[C0001]: a `del` of the imported name `sqrt` is not supported yet",
        ),
        (
            "x = 1\n\n\ndef f() -> None:\n    print(x)\n\n\ndel x\nf()\n",
            "error[C0001]: a module-level `del x` is not supported when a function or class of \
             this module mentions `x`: its body is checked after all top-level code, so it could \
             read `x` after the deletion",
        ),
    ] {
        let text = check_fails("e2e_1244_refusals", &[("a.py", source)]);
        assert!(text.contains(header), "{source}: {text}");
        assert!(!text.contains("panicked"), "{source}: {text}");
    }
}

/// The last `except*` handler may read a name and then delete it: no later
/// handler runs after it. As above, the expected text is CPython 3.14.7's:
/// an unpinned `python3` may predate PEP 654's `except*`.
#[test]
fn the_last_except_star_handler_may_delete_what_it_read() {
    let dir = ScratchDir::new("e2e_1244_except_star").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "x = 1\ntry:\n    raise ValueError(\"a\")\nexcept* ValueError:\n    pass\n\
         except* TypeError:\n    print(x)\n    del x\nprint(1)\n",
    )
    .expect("write the subject");
    assert_eq!(build_and_run(&dir, "a.py"), "1\n");
}
