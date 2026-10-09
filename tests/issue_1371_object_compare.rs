//! Part 1 of #1371: comparisons, identity tests and `isinstance` with a
//! CPython object operand, in a module body and in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. They print only `bool`s and fixed markers,
//! never an object repr. The hosted tests are `#[ignore]`d and contribute
//! no line coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_types/src/foreign/compare/`,
//! `crates/pycc_mir/src/obj_compare/` and
//! `crates/pycc_codegen/src/tests/object_compare.rs`.

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

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

/// The comparisons Part 1 admits, each at module level and in a function
/// body. `json.loads` is the producer throughout because it yields a fresh
/// CPython object of a chosen type from a literal.
const SUCCESS: &str = "import json\n\
    import collections\n\
    \n\
    a = json.loads(\"[1, 2]\")\n\
    b = a\n\
    n = json.loads(\"NaN\")\n\
    print(a is b)\n\
    print(a is not b)\n\
    print(a is json.loads(\"[1, 2]\"))\n\
    print(json.loads(\"null\") is None)\n\
    print(None is not a)\n\
    print(a == json.loads(\"[1, 2]\"))\n\
    print(a != json.loads(\"[1, 2]\"))\n\
    print(json.loads(\"2\") == 2)\n\
    print(json.loads(\"2\") < 2.5)\n\
    print(3 <= json.loads(\"2\"))\n\
    print(json.loads(\"\\\"b\\\"\") > \"a\")\n\
    print(json.loads(\"true\") >= True)\n\
    print(n == n)\n\
    print(n is n)\n\
    if a == b:\n    print(\"truth eq\")\n\
    r = a == b\n\
    print(r)\n\
    print(bool(a != b))\n\
    print(isinstance(json.loads(\"1\"), int))\n\
    print(isinstance(json.loads(\"true\"), int))\n\
    print(isinstance(json.loads(\"1\"), bool))\n\
    print(isinstance(json.loads(\"1.5\"), float))\n\
    print(isinstance(json.loads(\"\\\"s\\\"\"), str))\n\
    print(isinstance(a, collections.OrderedDict))\n\
    print(isinstance(collections.OrderedDict(), collections.OrderedDict))\n\
    \n\
    def f(s: str) -> int:\n    o = json.loads(s)\n    p = json.loads(s)\n    k = 0\n    \
    if o is not None:\n        k = k + 1\n    if o is p:\n        k = k + 10\n    \
    if o == p:\n        k = k + 100\n    if o < 5:\n        k = k + 1000\n    \
    if isinstance(o, int):\n        k = k + 10000\n    return k\n\
    \n\
    def g() -> bool:\n    o = json.loads(\"null\")\n    return o is None\n\
    \n\
    print(f(\"3\"))\n\
    print(f(\"7\"))\n\
    print(g())\n";

/// What `SUCCESS` prints under CPython.
///
/// `f`'s `o is p` holds because CPython caches small `int`s, so two
/// `json.loads("3")` results are one object -- the identity test observes
/// exactly what CPython's does.
const SUCCESS_OUT: &str = "True\nFalse\nFalse\nTrue\nTrue\nTrue\nFalse\nTrue\nTrue\nFalse\nTrue\n\
    True\nFalse\nTrue\ntruth eq\nTrue\nFalse\nTrue\nTrue\nFalse\nTrue\nTrue\nFalse\nTrue\n\
    11111\n10111\nTrue\n";

#[test]
fn check_accepts_the_success_program() {
    let dir = ScratchDir::new("obj_cmp_check").expect("scratch");
    let output = check_with(&dir, SUCCESS);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes Part 1 does not admit are refused with `I0404` (or keep
/// their pre-existing diagnostic), never a panic.
#[test]
fn the_shapes_outside_part_1_are_refused() {
    const HEAD: &str = "import json\n\no = json.loads(\"1\")\n";
    for (tag, tail, code, needle) in [
        (
            "obj_cmp_native_identity",
            "x = 1\nprint(o is x)\n",
            "I0404",
            "testing a CPython object's identity against a `int` value",
        ),
        (
            "obj_cmp_list",
            "print(o == [1])\n",
            "I0404",
            "comparing a CPython object with a `list[int]` value",
        ),
        (
            "obj_cmp_none_eq",
            "print(o == None)\n",
            "I0404",
            "comparing a CPython object with a `None` value",
        ),
        (
            "obj_cmp_chain",
            "print(0 < o < 2)\n",
            "I0404",
            "a chained comparison with a CPython object operand",
        ),
        (
            "obj_cmp_bool_annotation",
            "b: bool = o == 1\n",
            "T0025",
            "object",
        ),
        (
            "obj_cmp_isinstance_tuple",
            "print(isinstance(o, (int, str)))\n",
            "I0404",
            "against a tuple of classes",
        ),
        (
            "obj_cmp_isinstance_pycc_class",
            "class C(Exception):\n    pass\n\nprint(isinstance(o, C))\n",
            "I0404",
            "against the pycc exception class `C`",
        ),
        (
            "obj_cmp_native_general_identity",
            "x = 1\ny = 2\nprint(x is y)\n",
            "C0001",
            "comparison operator not supported yet: Is",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
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

/// Runs `target` and prints what it raised, by type only, after whatever
/// the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, then asserts the extension's import report matches
/// CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn object_comparisons_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_cmp_hosted", "pycc_obj_cmp_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A raising `__eq__` (a signalling-NaN `Decimal` raises `InvalidOperation`
/// on `==`) and an unorderable `<` (`TypeError`) propagate out of the
/// module body like any other object-call error.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_comparison_propagates_from_the_module_body() {
    for (tag, module, tail, raised) in [
        (
            "obj_cmp_raise_eq",
            "pycc_obj_cmp_raise_eq",
            "d = decimal.Decimal(\"sNaN\")\nprint(d == 1)\n",
            "InvalidOperation",
        ),
        (
            "obj_cmp_raise_lt",
            "pycc_obj_cmp_raise_lt",
            "print(json.loads(\"[1]\") < json.loads(\"{}\"))\n",
            "TypeError",
        ),
        (
            "obj_cmp_raise_isinstance",
            "pycc_obj_cmp_raise_isinstance",
            "print(isinstance(json.loads(\"1\"), json))\n",
            "TypeError",
        ),
    ] {
        let body =
            format!("import decimal\nimport json\n\nprint(\"before\")\n{tail}print(\"after\")\n");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("before\n{raised}\n"));
    }
}

/// The same raises inside a function body are caught by the function's own
/// handler.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_comparison_is_caught_in_a_function() {
    let out = assert_matches_cpython(
        "obj_cmp_caught",
        "pycc_obj_cmp_caught_mod",
        "import decimal\nimport json\n\n\
         def eq() -> int:\n    d = decimal.Decimal(\"sNaN\")\n    try:\n        \
         if d == 1:\n            return 1\n        return 0\n    except Exception:\n        \
         return -1\n\n\
         def lt() -> int:\n    try:\n        if json.loads(\"[1]\") < json.loads(\"{}\"):\n            \
         return 1\n        return 0\n    except TypeError:\n        return -2\n\n\
         def inst() -> int:\n    try:\n        if isinstance(json.loads(\"1\"), json):\n            \
         return 1\n        return 0\n    except TypeError:\n        return -3\n\n\
         print(eq())\nprint(lt())\nprint(inst())\n",
    );
    assert_eq!(out, "-1\n-2\n-3\nno error\n");
}

/// #1419: a `-> bool` return of an object comparison stays the `T0022`
/// D-258's #1419 amendment keeps (no implicit `PyObject_IsTrue`), and its
/// JSON diagnostic carries the `bool(...)` help.
#[test]
fn a_bool_return_of_an_object_comparison_is_refused_with_a_bool_help() {
    let dir = ScratchDir::new("obj_cmp_bool_slot").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "m.py",
            "import json\n\n\ndef same() -> bool:\n    o = json.loads(\"1\")\n    return o == 1\n",
        ))
        .arg("--error-format")
        .arg("json")
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(rendered.matches("\"T0022\"").count(), 1, "{rendered}");
    assert!(
        rendered.contains("return type mismatch: expected `bool`, found `object`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("wrap the value in `bool(...)`"),
        "{rendered}"
    );
}

/// The host side of the #1419 workaround: `same` is bound by `load`, and
/// every result is printed with its type, so a non-`bool` leak would show.
fn bool_of_comparison_script(load: &str) -> String {
    format!(
        "{load}\n\
         class Yes:\n    def __eq__(self, other):\n        return 'yes'\n\
         class Zero:\n    def __eq__(self, other):\n        return 0\n\
         class Bad:\n    def __bool__(self):\n        raise ValueError('no truth')\n\
         class Raises:\n    def __eq__(self, other):\n        return Bad()\n\
         nan = float('nan')\n\
         for a, b in [(1, 1), (1, 2), (Yes(), 1), (Zero(), 1), (nan, nan)]:\n    \
         r = same(a, b)\n    print(type(r).__name__, r)\n\
         try:\n    same(Raises(), 1)\nexcept ValueError as e:\n    print('ValueError', e)\n"
    )
}

/// The explicit `bool(a == b)` the #1419 help suggests behaves as CPython's
/// own: a non-`bool` `__eq__` result is truth-tested, and a raising
/// `__bool__` propagates to the caller.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn bool_of_an_object_comparison_matches_cpython_in_the_host() {
    let dir = ScratchDir::new("obj_cmp_bool_of").expect("scratch");
    build_ext(
        &dir,
        "pycc_obj_cmp_bool_of",
        "from typing import Any\n\n\ndef same(a: Any, b: Any) -> bool:\n    return bool(a == b)\n",
    );
    let compiled = python(
        &dir,
        &bool_of_comparison_script("from pycc_obj_cmp_bool_of import same"),
    );
    assert_ok(&compiled);
    let oracle = python(
        &dir,
        &bool_of_comparison_script("import runpy\nsame = runpy.run_path('m.py')['same']"),
    );
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(
        stdout_of(&compiled),
        "bool True\nbool False\nbool True\nbool False\nbool False\nValueError no truth\n"
    );
}
