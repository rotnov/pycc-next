use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

// ===========================================================================
// Part A: isinstance — success cases (compile-time evaluation)
// ===========================================================================

/// #435: `isinstance(D(), D)` returns True — an object is an instance of
/// its own class.
#[test]
fn isinstance_true_for_same_class() {
    let dir = ScratchDir::new("435_isinstance_same").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "same.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nd = D()\nr = isinstance(d, D)\nprint(r)\n",
    );
    let out = dir.join("same");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(D(), D)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(D(), D) should be True"
    );
}

/// #435: `isinstance(D(), B)` returns True when D extends B — the base
/// class is in D's MRO.
#[test]
fn isinstance_true_for_base_class() {
    let dir = ScratchDir::new("435_isinstance_base").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "base.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nd = D()\nr = isinstance(d, B)\nprint(r)\n",
    );
    let out = dir.join("base");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(D(), B)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(D(), B) should be True"
    );
}

/// #435: `isinstance(D(), C)` returns False when D and C are unrelated.
#[test]
fn isinstance_false_for_unrelated_class() {
    let dir = ScratchDir::new("435_isinstance_unrelated").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "unrelated.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass C:\n    def __init__(self) -> None:\n        self.y = 2\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nd = D()\nr = isinstance(d, C)\nprint(r)\n",
    );
    let out = dir.join("unrelated");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(D(), C)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"False\n",
        "isinstance(D(), C) should be False"
    );
}

/// #435: `isinstance(B(), D)` returns False — a base class instance is not
/// an instance of a derived class.
#[test]
fn isinstance_false_for_subclass() {
    let dir = ScratchDir::new("435_isinstance_subclass").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "subclass.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nb = B()\nr = isinstance(b, D)\nprint(r)\n",
    );
    let out = dir.join("subclass");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(B(), D)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"False\n",
        "isinstance(B(), D) should be False"
    );
}

/// #435: `isinstance(5, int)` returns True.
#[test]
fn isinstance_with_int() {
    let dir = ScratchDir::new("435_isinstance_int").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "int.py", "r = isinstance(5, int)\nprint(r)\n");
    let out = dir.join("int");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(5, int)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(5, int) should be True"
    );
}

/// #435: `isinstance(True, int)` returns True — bool is a subtype of int.
#[test]
fn isinstance_with_bool_as_int() {
    let dir = ScratchDir::new("435_isinstance_bool_int").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "bool_int.py", "r = isinstance(True, int)\nprint(r)\n");
    let out = dir.join("bool_int");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(True, int)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(True, int) should be True"
    );
}

/// #435: `isinstance("hi", str)` returns True.
#[test]
fn isinstance_with_str() {
    let dir = ScratchDir::new("435_isinstance_str").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "str.py", "r = isinstance(\"hi\", str)\nprint(r)\n");
    let out = dir.join("str");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(\"hi\", str)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(\"hi\", str) should be True"
    );
}

/// #435: `isinstance(D(), (B, C))` returns True if D extends B — tuple of
/// classes, any match.
#[test]
fn isinstance_with_tuple_of_classes() {
    let dir = ScratchDir::new("435_isinstance_tuple").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "tuple.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass C:\n    def __init__(self) -> None:\n        self.y = 2\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nd = D()\nr = isinstance(d, (B, C))\nprint(r)\n",
    );
    let out = dir.join("tuple");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(D(), (B, C))"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "isinstance(D(), (B, C)) should be True"
    );
}

/// #435: `isinstance(D(), (C,))` returns False when D extends B, not C.
#[test]
fn isinstance_with_tuple_no_match() {
    let dir = ScratchDir::new("435_isinstance_tuple_no").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "tuple_no.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass C:\n    def __init__(self) -> None:\n        self.y = 2\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nd = D()\nr = isinstance(d, (C,))\nprint(r)\n",
    );
    let out = dir.join("tuple_no");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for isinstance(D(), (C,))"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"False\n",
        "isinstance(D(), (C,)) should be False"
    );
}

// ===========================================================================
// Part A: isinstance — error cases
// ===========================================================================

/// #435: `isinstance()` with wrong arg count is rejected with T0021.
#[test]
fn isinstance_wrong_arg_count() {
    let dir = ScratchDir::new("435_isinstance_arity").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "arity.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nd = D()\nr = isinstance(d)\n",
    );
    let out = dir.join("arity");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with wrong arg count"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for wrong arg count, got: {stderr}"
    );
}

/// #435: `isinstance(D(), UnknownClass)` is rejected with T0001.
#[test]
fn isinstance_unknown_class() {
    let dir = ScratchDir::new("435_isinstance_unknown").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "unknown.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nd = D()\nr = isinstance(d, UnknownClass)\n",
    );
    let out = dir.join("unknown");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with unknown class"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0001"),
        "should report T0001 for unknown class, got: {stderr}"
    );
}

/// #435: `isinstance(D(), 5)` — second arg is not a class name or tuple.
#[test]
fn isinstance_non_class_second_arg() {
    let dir = ScratchDir::new("435_isinstance_nonclass").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "nonclass.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nd = D()\nr = isinstance(d, 5)\n",
    );
    let out = dir.join("nonclass");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with non-class second arg"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for non-class second arg, got: {stderr}"
    );
}

/// #435 review fix (P1): `isinstance(D(), D)` — a call expression as the
/// first argument is rejected with C0001, because pycc's compile-time
/// `isinstance` would silently discard the call's side effects. The user
/// must assign the call result to a variable first.
#[test]
fn isinstance_with_call_expression_is_rejected() {
    let dir = ScratchDir::new("435_isinstance_call").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "call.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nr = isinstance(D(), D)\n",
    );
    let out = dir.join("call");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with call expression"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "should report C0001 for call expression, got: {stderr}"
    );
}

/// #435 review fix (P1): a user-defined function named `isinstance` takes
/// priority over the compile-time builtin — the call is type-checked and
/// lowered as an ordinary function call, not intercepted.
#[test]
fn user_defined_isinstance_takes_priority_over_builtin() {
    let dir = ScratchDir::new("435_user_isinstance").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "user_isinstance.py",
        "def isinstance(x: int, y: int) -> int:\n    return x + y\nr = isinstance(2, 3)\nprint(r)\n",
    );
    let out = dir.join("user_isinstance");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for user-defined isinstance"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"5\n",
        "user-defined isinstance(2, 3) should return 5"
    );
}

/// #435 review fix (P1): a user-defined function named `issubclass` takes
/// priority over the compile-time builtin.
#[test]
fn user_defined_issubclass_takes_priority_over_builtin() {
    let dir = ScratchDir::new("435_user_issubclass").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "user_issubclass.py",
        "def issubclass(x: int, y: int) -> int:\n    return x * y\nr = issubclass(4, 5)\nprint(r)\n",
    );
    let out = dir.join("user_issubclass");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for user-defined issubclass"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"20\n",
        "user-defined issubclass(4, 5) should return 20"
    );
}

// ===========================================================================
// Part A: issubclass — success cases (compile-time evaluation)
// ===========================================================================

/// #435: `issubclass(D, D)` returns True.
#[test]
fn issubclass_true_for_same_class() {
    let dir = ScratchDir::new("435_issubclass_same").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "same.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nr = issubclass(D, D)\nprint(r)\n",
    );
    let out = dir.join("same");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for issubclass(D, D)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"True\n", "issubclass(D, D) should be True");
}

/// #435: `issubclass(D, B)` returns True when D extends B.
#[test]
fn issubclass_true_for_base_class() {
    let dir = ScratchDir::new("435_issubclass_base").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "base.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nr = issubclass(D, B)\nprint(r)\n",
    );
    let out = dir.join("base");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for issubclass(D, B)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"True\n", "issubclass(D, B) should be True");
}

/// #435: `issubclass(D, C)` returns False when D and C are unrelated.
#[test]
fn issubclass_false_for_unrelated() {
    let dir = ScratchDir::new("435_issubclass_unrelated").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "unrelated.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass C:\n    def __init__(self) -> None:\n        self.y = 2\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nr = issubclass(D, C)\nprint(r)\n",
    );
    let out = dir.join("unrelated");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for issubclass(D, C)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"False\n",
        "issubclass(D, C) should be False"
    );
}

/// #435: `issubclass(bool, int)` returns True — bool is a subtype of int.
#[test]
fn issubclass_bool_is_int() {
    let dir = ScratchDir::new("435_issubclass_bool_int").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "bool_int.py", "r = issubclass(bool, int)\nprint(r)\n");
    let out = dir.join("bool_int");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for issubclass(bool, int)"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "issubclass(bool, int) should be True"
    );
}

/// #435: `issubclass(D, (B, C))` returns True if D extends B.
#[test]
fn issubclass_with_tuple() {
    let dir = ScratchDir::new("435_issubclass_tuple").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "tuple.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\nclass C:\n    def __init__(self) -> None:\n        self.y = 2\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nr = issubclass(D, (B, C))\nprint(r)\n",
    );
    let out = dir.join("tuple");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for issubclass(D, (B, C))"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"True\n",
        "issubclass(D, (B, C)) should be True"
    );
}

// ===========================================================================
// Part A: issubclass — error cases
// ===========================================================================

/// #435: `issubclass()` with wrong arg count is rejected with T0021.
#[test]
fn issubclass_wrong_arg_count() {
    let dir = ScratchDir::new("435_issubclass_arity").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "arity.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nr = issubclass(D)\n",
    );
    let out = dir.join("arity");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for issubclass with wrong arg count"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for wrong arg count, got: {stderr}"
    );
}

/// #435: `issubclass(5, int)` — first arg is not a class name.
#[test]
fn issubclass_non_class_first_arg() {
    let dir = ScratchDir::new("435_issubclass_nonclass").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "nonclass.py", "r = issubclass(5, int)\n");
    let out = dir.join("nonclass");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for issubclass with non-class first arg"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for non-class first arg, got: {stderr}"
    );
}

/// #435: `issubclass(D, UnknownClass)` is rejected with T0001.
#[test]
fn issubclass_unknown_class() {
    let dir = ScratchDir::new("435_issubclass_unknown").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "unknown.py",
        "class D:\n    def __init__(self) -> None:\n        self.x = 1\nr = issubclass(D, UnknownClass)\n",
    );
    let out = dir.join("unknown");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for issubclass with unknown class"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0001"),
        "should report T0001 for unknown class, got: {stderr}"
    );
}

// ===========================================================================
// Part B: __init_subclass__ (compile-time hook)
// ===========================================================================

/// #435 (Part B): a class with `__init_subclass__` that just has `pass`
/// compiles and runs. In pycc's static-dispatch model, `__init_subclass__`
/// is a regular method (the `cls` parameter from CPython's implicit
/// classmethod protocol is not modeled — `self` is used instead). `D`'s own
/// creation is validated against its nearest ancestor's (`B`'s) body, which
/// is trivial here (see #854/D-214), so this stays accepted.
#[test]
fn init_subclass_with_pass_body_is_accepted() {
    let dir = ScratchDir::new("435_init_subclass_pass").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "pass.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nprint(D().x)\n",
    );
    let out = dir.join("pass");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for __init_subclass__ with pass body"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

/// #435 (Part B): `__init_subclass__` with a non-trivial (side-effecting)
/// body on the *ancestor* is rejected with C0001 at the subclass's own
/// creation, regardless of what the subclass itself overrides the hook
/// with. Per #854/D-214, `D`'s own creation is always validated against its
/// nearest ancestor's body (`B`'s, here side-effecting) — `D`'s own trivial
/// override is never the body checked at `D`'s own creation site (only a
/// further subclass of `D` would see it).
#[test]
fn init_subclass_with_non_trivial_body_is_rejected() {
    let dir = ScratchDir::new("435_init_subclass_reject").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "reject.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\n    def __init_subclass__(self) -> None:\n        print(\"hello\")\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
    );
    let out = dir.join("reject");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for __init_subclass__ with non-trivial body"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "should report C0001 for non-trivial __init_subclass__, got: {stderr}"
    );
    assert!(
        stderr.contains("__init_subclass__"),
        "error should mention __init_subclass__, got: {stderr}"
    );
}

/// #435 (Part B): a subclass of a class with `__init_subclass__` that does
/// NOT redefine `__init_subclass__` compiles without re-defining it. `D`
/// has no override at all, so its own creation is validated directly
/// against `B`'s trivial body.
#[test]
fn init_subclass_inherited_from_base() {
    let dir = ScratchDir::new("435_init_subclass_inherit").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inherit.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nprint(D().x)\n",
    );
    let out = dir.join("inherit");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for inherited __init_subclass__"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

/// #435 (Part B): `__init_subclass__` with a docstring body is accepted
/// (a docstring is a bare string-literal expression statement, which has
/// no side effects). The docstring lives on `B`'s hook, since `D` has no
/// override and its own creation is validated against `B`'s body directly.
#[test]
fn init_subclass_with_docstring_body_is_accepted() {
    let dir = ScratchDir::new("435_init_subclass_doc").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "doc.py",
        "class B:\n    def __init__(self) -> None:\n        self.x = 1\n    def __init_subclass__(self) -> None:\n        \"docstring\"\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\nprint(D().x)\n",
    );
    let out = dir.join("doc");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for __init_subclass__ with docstring body"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

// ===========================================================================
// Part C: __set_name__ (recognition)
// ===========================================================================

/// #435 (Part C): a class with a `__set_name__` method compiles — it is
/// accepted as a regular method (pycc does not support class-level
/// attribute assignments, so `__set_name__` can never be triggered, but
/// the method name is recognized as valid).
#[test]
fn set_name_method_is_accepted() {
    let dir = ScratchDir::new("435_set_name").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "set_name.py",
        "class C:\n    def __init__(self) -> None:\n        self.x = 1\n    def __set_name__(self, owner: int, name: int) -> None:\n        pass\nprint(C().x)\n",
    );
    let out = dir.join("set_name");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for __set_name__ method"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

// ===========================================================================
// Part D: __class_getitem__ (recognition)
// ===========================================================================

/// #435 (Part D): a class with a `__class_getitem__` static method compiles.
#[test]
fn class_getitem_method_is_accepted() {
    let dir = ScratchDir::new("435_class_getitem").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "class_getitem.py",
        "class C:\n    def __init__(self) -> None:\n        self.x = 1\n    @staticmethod\n    def __class_getitem__(key: int) -> int:\n        return 42\nprint(C().x)\n",
    );
    let out = dir.join("class_getitem");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for __class_getitem__ method"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

/// #435 (Part D): `ClassName[int]` as a type annotation works — it resolves
/// to `Ty::Instance(ClassName)`, ignoring the type argument. Used inside
/// the class body where the class name is self-referential (the existing
/// `annotation_to_ty` self-referential-class-name rule applies).
#[test]
fn class_getitem_allows_subscript_syntax() {
    let dir = ScratchDir::new("435_class_getitem_sub").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "subscript.py",
        "class C:\n    def __init__(self) -> None:\n        self.x = 1\n    @staticmethod\n    def __class_getitem__(key: int) -> int:\n        return 42\n    def get_self(self) -> C[int]:\n        return self\nprint(C().get_self().x)\n",
    );
    let out = dir.join("subscript");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for ClassName[int] annotation"
    );
    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"1\n", "program should run correctly");
}

// ===========================================================================
// Part E: edge cases — empty tuple, container types
// ===========================================================================

/// #435: `isinstance(1, ())` — an empty tuple is rejected with T0021.
/// `extract_class_names` rejects empty tuples because at least one class
/// is required.
#[test]
fn isinstance_with_empty_tuple_is_rejected() {
    let dir = ScratchDir::new("435_isinstance_empty_tuple").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "empty_tuple.py", "r = isinstance(1, ())\nprint(r)\n");
    let out = dir.join("empty_tuple");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with empty tuple"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for empty tuple, got: {stderr}"
    );
}

/// #435: `issubclass(int, ())` — an empty tuple is rejected with T0021.
#[test]
fn issubclass_with_empty_tuple_is_rejected() {
    let dir = ScratchDir::new("435_issubclass_empty_tuple").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "empty_tuple_sub.py",
        "r = issubclass(int, ())\nprint(r)\n",
    );
    let out = dir.join("empty_tuple_sub");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for issubclass with empty tuple"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for empty tuple, got: {stderr}"
    );
}

/// #435: `isinstance(1, list)` — `list` is not a valid class name for
/// isinstance (pycc only supports `int`, `str`, `float`, `bool`, and
/// user-defined classes), so this is rejected with T0001.
#[test]
fn isinstance_with_list_builtin_is_rejected() {
    let dir = ScratchDir::new("435_isinstance_list").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "list_builtin.py",
        "r = isinstance(1, list)\nprint(r)\n",
    );
    let out = dir.join("list_builtin");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for isinstance with list builtin"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0001"),
        "should report T0001 for unknown class 'list', got: {stderr}"
    );
}
