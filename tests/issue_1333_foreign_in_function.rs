//! Part 1 of #1333 (#1362): a function body may bind a CPython object to a
//! local name, alias it, return it from a pycc function, pass it to a pycc
//! function's parameter, and read a module-level name bound to one. What the
//! function body does with the value afterwards is the same operation set
//! the module level already had; a `for` over it in a function body is Part
//! 2 (#1363), and inferring an object type through a parameter or through a
//! local bound to a helper's result is Part 3 (#1364).
//!
//! Every hosted test compares against the host interpreter's own run of the
//! same source, and prints only fixed markers or `len`s -- never an object
//! repr, which carries an address. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the non-ignored tests here and by the unit tests in
//! `crates/pycc_types/src/foreign/function_local_tests.rs`,
//! `crates/pycc_types/src/foreign/in_function_tests.rs` and
//! `crates/pycc_codegen/src/tests/object_argument.rs`.

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

/// Writes `body` to `dir/<file>` and returns the path.
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

fn assert_checks(dir: &Path, body: &str) {
    let output = check_with(dir, body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{body}{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
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

/// The success program: a local binding and alias, a helper that returns
/// the object, a helper that receives it, a recursive helper that passes it
/// on, a method body, an unannotated private method that returns the object
/// to a sibling method, a module-level object global read in a function, a
/// module-level foreign-callable global called in a function, and a local
/// bound to a foreign callable and called.
const SUCCESS: &str = "import json\n\
    from itertools import product\n\
    P = product(\"ab\", \"c\")\n\
    G = json.loads\n\
    \n\
    def f(s: str) -> int:\n    y = json.loads(s)\n    z = y\n    return len(z)\n\
    \n\
    def _mk(s: str):\n    return json.loads(s)\n\
    \n\
    def _use(o) -> int:\n    return len(o)\n\
    \n\
    def g() -> int:\n    return _use(_mk(\"[1, 2, 3, 4]\"))\n\
    \n\
    def _r(o, n: int) -> int:\n    if n == 0:\n        return len(o)\n    return _r(o, n - 1)\n\
    \n\
    class C:\n    def m(self) -> int:\n        y = json.loads(\"[1]\")\n        return len(y)\n\n    \
    def _o(self):\n        return json.loads(\"[1, 2]\")\n\n    \
    def n(self) -> int:\n        return len(self._o())\n\
    \n\
    def _p() -> bool:\n    return bool(P)\n\
    \n\
    def _global_call(s: str) -> int:\n    return len(G(s))\n\
    \n\
    def _h(s: str):\n    g = json.loads\n    return g(s)\n\
    \n\
    print(f(\"[1, 2]\"))\n\
    print(g())\n\
    print(_r(json.loads(\"[1, 2, 3]\"), 3))\n\
    print(C().m())\n\
    print(C().n())\n\
    print(_p())\n\
    print(_global_call(\"[1, 2, 3, 4, 5]\"))\n\
    print(len(_h(\"[1]\")))\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "2\n4\n3\n1\n2\nTrue\n5\n1\n";

#[test]
fn check_accepts_binding_returning_and_passing_an_object_in_a_function() {
    let dir = ScratchDir::new("obj_fn_check").expect("scratch");
    for body in [
        SUCCESS,
        // A rebinding inside a loop keeps the local's `object` type.
        "import json\n\ndef f(n: int) -> int:\n    y = json.loads(\"[1]\")\n    i = 0\n    \
         while i < n:\n        y = json.loads(\"[1, 2]\")\n        i = i + 1\n    return len(y)\n",
        // An object argument passed straight from a call expression.
        "import json\n\ndef _n(o):\n    return len(o)\n\ndef f() -> int:\n    \
         return _n(json.loads(\"[1, 2, 3]\"))\n",
        // A helper reached through another helper's parameter.
        "import json\n\ndef _c():\n    return json.loads(\"[1, 2]\")\n\n\
         def _b(o) -> int:\n    return len(o)\n\ndef _a(o) -> int:\n    return _b(o)\n\n\
         print(_a(_c()))\n",
    ] {
        assert_checks(&dir, body);
    }
}

/// A sibling module's module-level object binding, imported by name, reads
/// in a function body of the importing module.
#[test]
fn check_accepts_a_dependency_module_s_object_read_in_a_function() {
    let dir = ScratchDir::new("obj_fn_dep_check").expect("scratch");
    write(&dir, "dep.py", "import json\nX = json.loads(\"[1]\")\n");
    assert_checks(
        &dir,
        "from dep import X\n\ndef f() -> int:\n    return len(X)\n\nprint(f())\n",
    );
}

/// Part 3 (#1364): the solver does not infer an object type through a local
/// bound to a helper's result, whichever order the two helpers are written
/// in, nor through a parameter.
#[test]
fn inference_through_a_helper_result_local_or_a_parameter_is_part_3() {
    for (tag, body, needle) in [
        (
            "obj_fn_part3_result_after",
            "import json\n\ndef _mk():\n    return json.loads(\"[1]\")\n\n\
             def _u():\n    y = _mk()\n    return y.copy()\n\nprint(len(_u()))\n",
            "cannot infer return type of private helper `_u`",
        ),
        (
            "obj_fn_part3_result_before",
            "import json\n\ndef _u():\n    y = _mk()\n    return y.copy()\n\n\
             def _mk():\n    return json.loads(\"[1]\")\n\nprint(len(_u()))\n",
            "cannot infer return type of private helper `_u`",
        ),
        (
            "obj_fn_part3_callable_after",
            "import json\n\ndef _getf():\n    return json.loads\n\n\
             def _v():\n    g = _getf()\n    return g(\"[1]\")\n\nprint(len(_v()))\n",
            "name `g` is bound to a non-callable value",
        ),
        (
            "obj_fn_part3_callable_before",
            "import json\n\ndef _v():\n    g = _getf()\n    return g(\"[1]\")\n\n\
             def _getf():\n    return json.loads\n\nprint(len(_v()))\n",
            "name `g` is bound to a non-callable value",
        ),
        (
            "obj_fn_part3_param_method",
            "import json\n\ndef _k(o):\n    return o.keys()\n\nprint(len(_k(json.loads(\"{}\"))))\n",
            "cannot infer return type of private helper `_k`",
        ),
        (
            "obj_fn_part3_param_call",
            "import json\n\ndef _f(g):\n    return g(\"[1]\")\n\nprint(len(_f(json.loads)))\n",
            "name `g` is bound to a non-callable value",
        ),
        (
            "obj_fn_part3_private_method_param",
            "import json\n\nclass C:\n    def _h(self, o):\n        return len(o)\n\n    \
             def m(self, s: str) -> int:\n        return self._h(json.loads(s))\n\n\
             print(C().m(\"[1]\"))\n",
            "cannot infer type of parameter `o` in private helper `C._h`",
        ),
    ] {
        assert_one_error(tag, body, "T0021", needle);
    }
}

const USING_Y: &str = "using `y`, which is bound to a CPython object";

/// The shapes Part 1 does not admit keep their diagnostics. A D-105
/// container spelling on a local object (`y.append(1)`) left this table
/// with #1095: it is the foreign method call now, pinned by
/// `a_container_spelling_on_a_function_local_object_is_admitted` below.
#[test]
fn the_shapes_outside_part_1_are_refused() {
    assert_one_error(
        "obj_fn_for_local",
        "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n    for t in y:\n        pass\n",
        "I0404",
        USING_Y,
    );
    assert_one_error(
        "obj_fn_for_call",
        "import json\n\ndef f() -> None:\n    for t in json.loads(\"[1]\"):\n        pass\n",
        "I0404",
        "is not supported inside a function body yet",
    );
    assert_one_error(
        "obj_fn_comprehension",
        "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n    zs = [t for t in y]\n",
        "I0404",
        USING_Y,
    );
    assert_one_error(
        "obj_fn_generic",
        "import json\n\ndef ident[T](x: T) -> T:\n    return x\n\n\
         def f() -> None:\n    y = json.loads(\"[1]\")\n    ident(y)\n",
        "I0404",
        "passing a CPython object to a generic function",
    );
    assert_one_error(
        "obj_fn_annotated_return",
        "import json\n\ndef f() -> str:\n    return json.loads(\"[1]\")\n",
        "T0022",
        "return type mismatch: expected `str`, found `object`",
    );
    assert_one_error(
        "obj_fn_mixed_return",
        "import json\n\ndef _m(n: int):\n    if n > 0:\n        return json.loads(\"[1]\")\n    \
         return 1\n\nprint(_m(1))\n",
        "T0022",
        "conflicting inferred types `object` and `int`",
    );
    assert_one_error(
        "obj_fn_list_argument",
        "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n    y.dumps([1])\n",
        "I0404",
        "passing a `list[int]` argument to a CPython object's method",
    );
}

/// #1095: `append` on a function-local object is the foreign method call.
#[test]
fn a_container_spelling_on_a_function_local_object_is_admitted() {
    let dir = ScratchDir::new("obj_fn_container_method").expect("scratch");
    assert_checks(
        &dir,
        "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n    y.append(1)\n",
    );
}

/// Operations on a function-local object that no Part 1 consumer admits
/// keep an ordinary check-phase diagnostic; none of them panics.
#[test]
fn unadmitted_operations_on_a_function_local_object_are_diagnosed() {
    const HEAD: &str = "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n";
    for (tag, tail, code, needle) in [
        (
            "obj_fn_add_int",
            "    print(y + 1)\n",
            "T0021",
            "operator Add is not defined for `object` and `int`",
        ),
        (
            "obj_fn_str_add",
            "    print(\"a\" + y)\n",
            "T0021",
            "operator Add is not defined for `str` and `object`",
        ),
        (
            "obj_fn_range",
            "    for i in range(y):\n        pass\n",
            "T0021",
            "range stop expects `int`, got `object`",
        ),
        // Part 1 of #1371 admits `y == 1`; a container operand stays out.
        (
            "obj_fn_eq",
            "    print(y == [1])\n",
            "I0404",
            "comparing a CPython object with a `list[int]` value",
        ),
        // Part 2b of #1371 admits `1 in y`; a container item stays out.
        (
            "obj_fn_in",
            "    print([1] in y)\n",
            "I0404",
            "testing membership of a `list[int]` value in a CPython object",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
    // `self.a = y` is refused because `y` is not an `__init__` parameter,
    // the same C0001 any non-parameter local gets. It does not reach
    // `init_slot.rs`'s type-keyed parameter arm. Since Part 1 of #1367 that
    // arm admits a parameter annotated with a class a foreign import binds
    // (`tests/issue_1367_foreign_class_annotations.rs`); a local is still no
    // parameter.
    assert_one_error(
        "obj_fn_self_attr_non_parameter",
        "import json\n\nclass C:\n    def __init__(self) -> None:\n        \
         y = json.loads(\"[1]\")\n        self.a = y\n\nC()\n",
        "C0001",
        "`self.<attr> = y` must reference one of `__init__`'s own parameters",
    );
    assert_one_error(
        "obj_fn_list_return",
        "import json\n\ndef _h():\n    y = json.loads(\"[1]\")\n    return [y]\n\n\
         print(len(_h()))\n",
        "T0021",
        "cannot infer return type of private helper `_h`",
    );
}

/// Builds `body` as the extension module `module` in `dir` (the source
/// stays at `dir/m.py`, where the oracle runs it too).
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

/// Runs `target` and prints what it raised -- any exception, by type only,
/// since a message could quote an address -- after whatever the module body
/// printed.
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
fn function_local_objects_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_fn_hosted", "pycc_obj_fn_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A raising producer in a function body is caught by the same function's
/// handler, and one frame up by the caller's `ValueError` handler
/// (`json.JSONDecodeError` derives from `ValueError`).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_producer_is_caught_in_and_above_the_function() {
    let out = assert_matches_cpython(
        "obj_fn_caught",
        "pycc_obj_fn_caught_mod",
        "import json\n\n\
         def f(s: str) -> int:\n    try:\n        y = json.loads(s)\n        return len(y)\n    \
         except Exception:\n        print(\"caught in f\")\n        return -1\n\n\
         def _mk(s: str):\n    return json.loads(s)\n\n\
         def g() -> int:\n    try:\n        o = _mk(\"{\")\n        return len(o)\n    \
         except ValueError:\n        print(\"caught in g\")\n        return -2\n\n\
         print(f(\"[1\"))\nprint(g())\n",
    );
    assert_eq!(out, "caught in f\n-1\ncaught in g\n-2\nno error\n");
}

/// A function that reads a module-level object global before the global is
/// bound raises CPython's `NameError`, which a handler catches; once bound,
/// the same read succeeds.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unbound_object_global_raises_name_error_until_it_is_bound() {
    let out = assert_matches_cpython(
        "obj_fn_unbound_global",
        "pycc_obj_fn_unbound_mod",
        "from itertools import product\n\n\
         def _read() -> bool:\n    try:\n        return bool(P)\n    except Exception:\n        \
         print(\"unbound\")\n        return False\n\n\
         print(_read())\nP = product(\"ab\", \"c\")\nprint(_read())\n",
    );
    assert_eq!(out, "unbound\nFalse\nTrue\nno error\n");
}

/// An uncaught `NameError` from a helper that uses a foreign import bound
/// only after the call escapes the module body like CPython's.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_helper_called_before_its_import_raises_name_error() {
    let out = assert_matches_cpython(
        "obj_fn_early_call",
        "pycc_obj_fn_early_mod",
        "def _f() -> int:\n    return len(json.loads(\"[1]\"))\n\nprint(_f())\nimport json\n",
    );
    assert_eq!(out, "NameError\n");
}

/// A missing attribute on a function-local object and a call of a
/// non-callable object both raise CPython's own exception.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn operations_on_a_function_local_object_raise_cpythons_exceptions() {
    let out = assert_matches_cpython(
        "obj_fn_attr_error",
        "pycc_obj_fn_attr_mod",
        "import json\n\ndef f() -> None:\n    y = json.loads(\"[1]\")\n    y.nope()\n\nf()\n",
    );
    assert_eq!(out, "AttributeError\n");
    let out = assert_matches_cpython(
        "obj_fn_type_error",
        "pycc_obj_fn_type_mod",
        "import json\n\ndef _h():\n    return json()\n\n_h()\n",
    );
    assert_eq!(out, "TypeError\n");
}

/// A dependency module's object binding reads in the importing module's
/// function body like CPython. `dep.py` is compiled into the extension: the
/// oracle runs first, then `dep.py` and any `__pycache__` are removed before
/// the extension is imported, so a run-time import of `dep` by CPython
/// would fail with `ModuleNotFoundError` instead of producing the same
/// output.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dependency_module_s_object_reads_in_a_function_in_the_host() {
    let dir = ScratchDir::new("obj_fn_two_modules").expect("scratch");
    let dep = write(&dir, "dep.py", "import json\nX = json.loads(\"[1, 2]\")\n");
    let body = "from dep import X\n\ndef f() -> int:\n    return len(X)\n\nprint(f())\n";
    build_ext(&dir, "pycc_obj_fn_two_mod", body);
    let oracle = host_python()
        .arg("m.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    std::fs::remove_file(&dep).expect("remove dep.py");
    let cache = dir.join("__pycache__");
    if cache.exists() {
        std::fs::remove_dir_all(&cache).expect("remove __pycache__");
    }
    let compiled = python(&dir, "import pycc_obj_fn_two_mod\n");
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), "2\n");
}

/// A plain (embedded) build runs the same function bodies, matching
/// CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_function_local_objects_like_cpython() {
    let dir = ScratchDir::new("obj_fn_embedded").expect("scratch");
    let source = write(&dir, "m.py", SUCCESS);
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
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(stdout_of(&embedded), SUCCESS_OUT);
}

/// A plain (embedded) build catches the `NameError` of a function-body
/// read of an object global before its binding runs, like CPython. The
/// handler names `Exception`, as in the `--ext` twin above: `except
/// NameError` is not a recognized handler class in the current subset.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_catches_an_unbound_object_global_like_cpython() {
    let dir = ScratchDir::new("obj_fn_embedded_unbound").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "from itertools import product\n\n\
         def _read() -> bool:\n    try:\n        return bool(P)\n    except Exception:\n        \
         print(\"unbound\")\n        return False\n\n\
         print(_read())\nP = product(\"ab\", \"c\")\nprint(_read())\n",
    );
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
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    let normalized = |output: &Output| stdout_of(output).replace("\r\n", "\n");
    assert_eq!(normalized(&embedded), normalized(&oracle));
    assert_eq!(normalized(&embedded), "unbound\nFalse\nTrue\n");
}

/// A mortal object the probe reads through a foreign module attribute; see
/// `tests/issue_1084_refcount_probe.rs` for why it must not be a small int,
/// and `tests/issue_1084_loop_shape.rs` for why the stub must not sit beside
/// the entry module.
const STUB: &str = "class _M:\n    def __len__(self):\n        return 3\n\n\nMESH = _M()\n";

/// The probe: each exported function runs its shape `n` times.
const PROBE: &str = "import pycc_1333_stub\n\
    \n\
    G = pycc_1333_stub.MESH\n\
    \n\
    def _get():\n    return pycc_1333_stub.MESH\n\
    \n\
    def _keep(o) -> int:\n    return len(o)\n\
    \n\
    def bind_alias(n: int) -> int:\n    t = 0\n    i = 0\n    while i < n:\n        \
    y = pycc_1333_stub.MESH\n        z = y\n        t = t + len(z)\n        i = i + 1\n    \
    return t\n\
    \n\
    def returned(n: int) -> int:\n    t = 0\n    i = 0\n    while i < n:\n        \
    t = t + len(_get())\n        i = i + 1\n    return t\n\
    \n\
    def passed(n: int) -> int:\n    y = pycc_1333_stub.MESH\n    t = 0\n    i = 0\n    \
    while i < n:\n        t = t + _keep(y)\n        i = i + 1\n    return t\n\
    \n\
    def global_read(n: int) -> int:\n    t = 0\n    i = 0\n    while i < n:\n        \
    w = G\n        t = t + len(w)\n        i = i + 1\n    return t\n";

/// The #1092 leak-only rule extends to function locals unchanged: only the
/// attribute load that produces the object leaks one reference. Binding,
/// aliasing, returning, passing and reading a module-level global add
/// nothing. Pinned at two trip counts so the slope is the claim, not a
/// constant. **When #1092 lands, these numbers become `0` and the pull
/// request that closes it is expected to edit this test.**
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn function_local_objects_leak_only_their_producing_load() {
    let dir = ScratchDir::new("obj_fn_refcount").expect("scratch");
    std::fs::write(dir.join("pycc_1333_stub.py"), STUB).expect("write the stub");
    std::fs::create_dir_all(dir.join("src")).expect("create the entry directory");
    let source = write(&dir.join("src"), "pycc_1333_probe.py", PROBE);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("pycc_1333_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, '.')\n\
         import pycc_1333_stub as stub\n\
         import pycc_1333_probe as probe\n\
         for name in ['bind_alias', 'returned', 'passed', 'global_read']:\n\
         \x20   for n in (1000, 2000):\n\
         \x20       before = sys.getrefcount(stub.MESH)\n\
         \x20       total = getattr(probe, name)(n)\n\
         \x20       print(name, n, sys.getrefcount(stub.MESH) - before, total)\n",
    );
    assert_ok(&run);
    assert_eq!(
        stdout_of(&run),
        "bind_alias 1000 1000 3000\nbind_alias 2000 2000 6000\n\
         returned 1000 1000 3000\nreturned 2000 2000 6000\n\
         passed 1000 1 3000\npassed 2000 1 6000\n\
         global_read 1000 0 3000\nglobal_read 2000 0 6000\n"
    );
}
