//! Part 7 of #1371: `isinstance(o, C)` with a CPython object `o` and a
//! class `C` compiled in the same module -- lark `lalr_parser_state.py`
//! line 52, `if not isinstance(other, ParserState)` in `__eq__` -- plus
//! `list`, `dict` and `tuple` as builtin class arguments.
//!
//! A compiled instance reaches the CPython side only as a carrier of its
//! published class's type object (`mod.Base(1)`), so the hosted tests drive
//! the extension from a host script and run the very same script against
//! the source imported as a plain Python module: CPython is the oracle for
//! every line. The hosted tests are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in
//! `crates/pycc_types/src/foreign/compare/tests.rs`,
//! `crates/pycc_mir/src/obj_compare/tests.rs`,
//! `crates/pycc_codegen/src/tests/object_compare.rs` and
//! `src/ext_build_tests/generated_c.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    command.env("PYTHONIOENCODING", "utf-8");
    command
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

/// The compiled classes with no exported type object of their own kind are
/// refused, each naming its kind, and a tuple of classes stays refused.
#[test]
fn isinstance_against_a_special_compiled_class_or_a_tuple_is_refused() {
    const HEAD: &str = "import builtins\nfrom enum import Enum\nfrom typing import Protocol\n\n\n";
    for (tag, classes, class_arg, needle) in [
        (
            "obj_isinstance_exception",
            "class E(Exception):\n    pass\n",
            "E",
            "against the pycc exception class `E`",
        ),
        (
            "obj_isinstance_protocol",
            "class P(Protocol):\n    def m(self) -> int: ...\n",
            "P",
            "against the pycc protocol `P`",
        ),
        (
            "obj_isinstance_enum",
            "class K(Enum):\n    A = 1\n",
            "K",
            "against the pycc enum `K`",
        ),
        (
            "obj_isinstance_generic",
            "class G[T]:\n    pass\n",
            "G",
            "against the pycc generic class `G`",
        ),
        (
            "obj_isinstance_tuple",
            "class C:\n    pass\n",
            "(C, int)",
            "against a tuple of classes",
        ),
    ] {
        let body = format!("{HEAD}{classes}\n\nb = isinstance(builtins.len, {class_arg})\n");
        assert_one_error(tag, &body, "I0404", needle);
    }
}

/// The module the hosted carrier test builds. `Base`/`Derived` are a
/// published family, `ParserState.same` is the lark `__eq__` shape on an
/// `Any` parameter, `_Hidden` is private and so has no published type, and
/// `shadowed` and `shadowed_list` take a parameter spelled like `Base` and
/// `list`, which shadows the class as in CPython, and `pin` repeats the test 200 times on one object inside compiled code,
/// returning `1000 * refcount drift + hits`: a reference the shim failed to
/// release would show as a drift.
const MODULE: &str = "import builtins\n\
    import sys\n\
    from typing import Any\n\
    \n\
    \n\
    class Base:\n    \
    def __init__(self, x: int) -> None:\n        self.x = x\n\n    \
    def get(self) -> int:\n        return self.x\n\
    \n\
    \n\
    class Derived(Base):\n    \
    def twice(self) -> int:\n        return self.x * 2\n\
    \n\
    \n\
    class ParserState:\n    \
    def __init__(self, position: int) -> None:\n        self.position = position\n\n    \
    def same(self, other: Any) -> bool:\n        \
    if not isinstance(other, ParserState):\n            return False\n        \
    return True\n\
    \n\
    \n\
    class _Hidden:\n    \
    def get(self) -> int:\n        return 0\n\
    \n\
    \n\
    def is_base(o: object) -> bool:\n    return isinstance(o, Base)\n\
    \n\
    \n\
    def is_derived(o: object) -> bool:\n    return isinstance(o, Derived)\n\
    \n\
    \n\
    def is_hidden(o: object) -> bool:\n    return isinstance(o, _Hidden)\n\
    \n\
    \n\
    def is_list(o: object) -> bool:\n    return isinstance(o, list)\n\
    \n\
    \n\
    def is_dict(o: object) -> bool:\n    return isinstance(o, dict)\n\
    \n\
    \n\
    def is_tuple(o: object) -> bool:\n    return isinstance(o, tuple)\n\
    \n\
    \n\
    def len_is_base() -> bool:\n    return isinstance(builtins.len, Base)\n\
    \n\
    \n\
    def shadowed(o: object, Base: object) -> bool:\n    return isinstance(o, Base)\n\
    \n\
    \n\
    def shadowed_list(o: object, list: object) -> bool:\n    return isinstance(o, list)\n\
    \n\
    \n\
    def pin(o: object) -> int:\n    \
    before = int(sys.getrefcount(o))\n    \
    hits = 0\n    \
    i = 0\n    \
    while i < 200:\n        \
    if isinstance(o, Base):\n            hits += 1\n        \
    i += 1\n    \
    return (int(sys.getrefcount(o)) - before) * 1000 + hits\n";

/// The host script, run once against the extension and once against
/// `MODULE` as plain Python. `Raising` has a raising `__class__`, which
/// CPython's `isinstance` consults once the type check fails -- for a
/// published family and for the unpublished `_Hidden` alike. `Posing`
/// claims to be a `Derived` through `__class__`, which makes it an instance
/// of `Base` and `Derived` in CPython and in the extension.
const DRIVER: &str = "import pycc_obj_isinstance_mod as m\n\
    b = m.Base(1)\n\
    d = m.Derived(2)\n\
    s = m.ParserState(3)\n\
    print(m.is_base(b), m.is_base(d), m.is_base(s), m.is_derived(b), m.is_derived(d))\n\
    print(m.is_base(3), m.is_base('x'), m.is_base(None), m.is_base(m.Base), m.is_hidden(b))\n\
    print(s.same(m.ParserState(4)), s.same(b), s.same(1), m.len_is_base())\n\
    print(m.is_list([1]), m.is_list((1,)), m.is_dict({}), m.is_tuple((1,)), m.is_tuple([1]))\n\
    class Raising:\n\
    \x20   @property\n\
    \x20   def __class__(self):\n\
    \x20       raise ValueError('boom')\n\
    class Posing:\n\
    \x20   @property\n\
    \x20   def __class__(self):\n\
    \x20       return m.Derived\n\
    for test in (m.is_base, m.is_hidden):\n\
    \x20   try:\n\
    \x20       test(Raising())\n\
    \x20   except ValueError as e:\n\
    \x20       print('ValueError', e)\n\
    print(m.is_base(Posing()), m.is_derived(Posing()), m.is_hidden(Posing()), m.is_base(object()))\n\
    print(m.pin(b), m.pin(d), m.pin([1]))\n\
    print(m.shadowed(b, int), m.shadowed(3, int), m.shadowed(b, m.Base))\n\
    print(m.shadowed_list([1], tuple), m.shadowed_list((1,), tuple))\n";

const DRIVER_OUT: &str = "True True False False True\n\
    False False False False False\n\
    True False False False\n\
    True False True True False\n\
    ValueError boom\n\
    ValueError boom\n\
    True True False False\n\
    200 200 0\n\
    False True True\n\
    False True\n";

fn run_driver(path_entry: &Path, cwd: &Path) -> Output {
    host_python()
        .arg("-c")
        .arg(DRIVER)
        .env("PYTHONPATH", path_entry)
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn")
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn isinstance_against_a_compiled_class_behaves_like_cpython_in_the_host() {
    let dir = ScratchDir::new("obj_isinstance_hosted").expect("scratch");
    let ext = dir.join("ext");
    let oracle = dir.join("oracle");
    std::fs::create_dir_all(&ext).expect("ext dir");
    std::fs::create_dir_all(&oracle).expect("oracle dir");
    let source = write(&oracle, "pycc_obj_isinstance_mod.py", MODULE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join("pycc_obj_isinstance_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    // Each run's working directory is the *other* side's directory's
    // parent, so neither can import the module from where it runs.
    let compiled = run_driver(&ext, &dir);
    assert_ok(&compiled);
    let cpython = run_driver(&oracle, &dir);
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), DRIVER_OUT);
}

/// A module-body test against a compiled class, with no carrier anywhere:
/// every object is a foreign value, so every answer is False, and the
/// container classes answer like CPython.
const BODY: &str = "import builtins\n\
    \n\
    \n\
    class C:\n    \
    def __init__(self) -> None:\n        self.x = 1\n\n    \
    def get(self) -> int:\n        return self.x\n\
    \n\
    \n\
    print(isinstance(builtins.len, C), isinstance(builtins.list('ab'), C))\n\
    print(isinstance(builtins.list('ab'), list), isinstance(builtins.dict(), dict))\n\
    print(isinstance(builtins.tuple('ab'), tuple), isinstance(builtins.tuple('ab'), list))\n";

const BODY_OUT: &str = "False False\nTrue True\nTrue False\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_module_body_test_behaves_like_cpython_in_the_host() {
    let dir = ScratchDir::new("obj_isinstance_body").expect("scratch");
    let source = write(&dir, "m.py", BODY);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("pycc_obj_isinstance_body"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let compiled = host_python()
        .arg("-c")
        .arg("import pycc_obj_isinstance_body")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&compiled);
    let cpython = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&cpython);
    assert_eq!(stdout_of(&compiled), stdout_of(&cpython));
    assert_eq!(stdout_of(&compiled), BODY_OUT);
}

/// A plain (embedded) build publishes no class, so a compiled class is
/// answered by the shim's unpublished-class path -- which is right, since no
/// CPython object there is a compiled instance -- and matches CPython.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_answers_like_cpython() {
    let dir = ScratchDir::new("obj_isinstance_embedded").expect("scratch");
    let source = write(&dir, "m.py", BODY);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_ok(&embedded);
    let cpython = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&cpython);
    assert_eq!(stdout_of(&embedded), stdout_of(&cpython));
    assert_eq!(stdout_of(&embedded), BODY_OUT);
}
