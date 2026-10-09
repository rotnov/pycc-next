//! Part 11 of #1371: `type(o)` on a CPython object, and `is` chains over
//! one.
//!
//! `type(o)` is CPython's `PyObject_Type` (`pycc_ext_obj_type`), whose
//! result is one more opaque object: printed, bound, compared by identity,
//! nested and read for an attribute. An `is`/`is not` chain with an object
//! operand is refused with the object chain's `I0404` instead of the
//! `C0001` the HIR lowering used to report for any non-`None` identity
//! link.
//!
//! The hosted tests build the source with `pycc build --ext`, import the
//! artifact into the host CPython, and compare its output with CPython's
//! own run of the same source. They are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_types/src/foreign/`,
//! `crates/pycc_mir/src/tests/import/object_type.rs` and
//! `crates/pycc_codegen/src/tests/object_type.rs`; the refusals below stop
//! at `pycc check` and run everywhere.
//!
//! The helper module `pycc_type_helper` lives in a `host_only/`
//! subdirectory of the scratch dir rather than beside the fixture source: a
//! `.py` next to the source is resolved as a *project* import, so it would
//! never reach the foreign-object path.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
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

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// The host-only helper: an instance of a plain class, a subclass
/// instance, an `int`, an `int` subclass instance and a class object itself.
const HELPER: &str = "class K:\n    pass\n\
    \n\
    \n\
    class Sub(K):\n    pass\n\
    \n\
    \n\
    k = K()\n\
    class MyInt(int):\n    pass\n\
    \n\
    \n\
    k2 = K()\n\
    s = Sub()\n\
    j = 3\n\
    mi = MyInt(4)\n";

const PRELUDE: &str = "import pycc_type_helper\n\nH = pycc_type_helper\n";

/// The shapes Part 11 admits, in a function body and the module body:
/// printing the class, binding it, identity between two classes, a
/// subclass's distinct class, `type(type(o))`, an attribute of the class,
/// `type(o)` returned from an unannotated private helper, and `type(o)` on
/// a name an `isinstance(o, int)` guard narrowed, which keeps an `int`
/// subclass's own class (#1476).
const SUCCESS: &str = "import pycc_type_helper\n\
    \n\
    H = pycc_type_helper\n\
    \n\
    \n\
    def _cls(n: int):\n    \
    if n > 0:\n        return type(H.k)\n    \
    return type(H.j)\n\
    \n\
    \n\
    def body() -> None:\n    \
    t = type(H.k)\n    \
    print(t)\n    \
    print(type(H.k) is type(H.k2))\n    \
    print(type(H.k) is not type(H.s))\n    \
    print(type(H.s).__name__, type(H.s).__mro__[1].__name__)\n    \
    print(type(type(H.k)))\n    \
    print(_cls(1) is t, _cls(0))\n\
    \n\
    \n\
    def _narrowed(o):\n    \
    if isinstance(o, int):\n        \
    print(type(o).__name__)\n\
    \n\
    \n\
    body()\n\
    _narrowed(H.mi)\n\
    _narrowed(H.j)\n\
    print(type(H.j))\n\
    u = type(H.k)\n\
    print(u.__name__)\n\
    print(type(H.K))\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "<class 'pycc_type_helper.K'>\nTrue\nTrue\nSub K\n<class 'type'>\n\
    True <class 'int'>\nMyInt\nint\n<class 'int'>\nK\n<class 'type'>\n";

/// `SUCCESS` passes `pycc check`: every object in it comes from a foreign
/// import.
#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_type_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{tag}: {}",
        stderr_of(&output)
    );
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
    assert!(
        rendered.contains(&format!("error[{code}]")),
        "{tag}: {rendered}"
    );
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

/// An `is`/`is not` chain with an object operand gets the object chain's
/// `I0404`, in a module body and a function body.
#[test]
fn an_identity_chain_over_an_object_is_refused_with_i0404() {
    for (tag, tail) in [
        ("obj_type_is_chain", "print(H.k is H.k is H.k)\n"),
        ("obj_type_is_not_chain", "print(H.k is not H.j is H.k)\n"),
        (
            "obj_type_is_chain_fn",
            "def f(a: int) -> bool:\n    return a is H.k is a\n",
        ),
    ] {
        assert_one_error(
            tag,
            &format!("{PRELUDE}{tail}"),
            "I0404",
            "a chained comparison with a CPython object operand",
        );
    }
}

/// Every other `type(...)` shape keeps its `C0001`: a native argument, the
/// three-argument constructor's arity, and calling the class.
#[test]
fn the_other_type_shapes_stay_refused() {
    for (tag, tail, needle) in [
        (
            "obj_type_native",
            "print(type(5))\n",
            "call to builtin `type` is valid Python but not implemented yet",
        ),
        (
            "obj_type_two_args",
            "print(type(H.k, H.j))\n",
            "call to builtin `type` is valid Python but not implemented yet",
        ),
        (
            "obj_type_call_result",
            "print(type(H.k)())\n",
            "calling `type(...)` is supported only as `type(self)(...)`",
        ),
    ] {
        assert_one_error(tag, &format!("{PRELUDE}{tail}"), "C0001", needle);
    }
}

/// Writes the helper module into `dir/host_only`.
fn write_helper(dir: &Path) {
    let helper_dir = dir.join("host_only");
    std::fs::create_dir_all(&helper_dir).expect("create the helper directory");
    std::fs::write(helper_dir.join("pycc_type_helper.py"), HELPER).expect("write the helper");
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PYTHONPATH", dir.join("host_only"))
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn type_of_objects_behaves_like_cpython_in_the_host() {
    let dir = ScratchDir::new("obj_type_hosted").expect("scratch");
    write_helper(&dir);
    let module = "pycc_obj_type_mod";
    let build = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", SUCCESS))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let compiled = python(&dir, &format!("import {module}"));
    assert_ok(&compiled);
    let oracle = python(&dir, "import runpy\nrunpy.run_path('m.py')");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), SUCCESS_OUT);
}
