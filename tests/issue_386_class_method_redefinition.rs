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

/// Issue #386: redefining a non-`__init__` method within a class body
/// rebinds -- the second `def` replaces the first, and the latest
/// definition is the one dispatched to at runtime. This mirrors CPython's
/// own class-body execution order: each `def` runs in source order, and a
/// later `def` with the same name rebinds the method.
#[test]
fn method_redefinition_rebinds_to_the_latest_definition() {
    let dir = ScratchDir::new("issue386_rebind").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rebind.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def val(self) -> int:\n        return 1\n    def val(self) -> int:\n        return 2\n\nc = C()\nprint(c.val())\n",
    );
    let out = dir.join("rebind");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for method redefinition"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"2\n",
        "the latest definition should be called, printing 2"
    );
}

/// Issue #386: three (or more) redefinitions of the same method name
/// keep rebinding -- the third `def` wins. The find/replace logic in
/// `lower_class` handles any count, but this test guards against a
/// regression that special-cases only the 2× path.
#[test]
fn three_redefinitions_of_the_same_method_keep_rebinding() {
    let dir = ScratchDir::new("issue386_rebind3").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "rebind3.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def val(self) -> int:\n        return 1\n    def val(self) -> int:\n        return 2\n    def val(self) -> int:\n        return 3\n\nc = C()\nprint(c.val())\n",
    );
    let out = dir.join("rebind3");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a 3x method redefinition"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"3\n",
        "the third definition should win, printing 3"
    );
}

/// Issue #386: a method called from *another* method sees the latest
/// binding (late binding in function bodies), per PR #358's pattern. Even
/// though `caller` is defined before the second `val`, `caller`'s body is
/// evaluated at call time, by which point the second `val` has rebound the
/// method.
#[test]
fn method_called_from_another_method_sees_latest_binding() {
    let dir = ScratchDir::new("issue386_late_bind").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "late_bind.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def val(self) -> int:\n        return 1\n    def caller(self) -> int:\n        return self.val()\n    def val(self) -> int:\n        return 2\n\nc = C()\nprint(c.caller())\n",
    );
    let out = dir.join("late_bind");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for late-bound method redefinition"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"2\n",
        "caller should see the latest val binding, printing 2"
    );
}

/// Issue #386: redefining `__init__` within a class body stays C0001 --
/// the compile-time attribute-slot pre-scan cannot reconcile two different
/// `__init__` bodies. This is a `pycc check` error.
#[test]
fn init_redefinition_is_a_check_error() {
    let dir = ScratchDir::new("issue386_init_check").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "init_redef.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def __init__(self) -> None:\n        return\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject __init__ redefinition with exit code 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("redefining `__init__` in the same class body is not supported yet"),
        "stdout should mention __init__ redefinition, got: {stdout}"
    );
}

/// Issue #386: redefining `__init__` within a class body is also a
/// `pycc build` error.
#[test]
fn init_redefinition_is_a_build_error() {
    let dir = ScratchDir::new("issue386_init_build").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "init_redef.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def __init__(self) -> None:\n        return\n",
    );
    let out = dir.join("init_redef");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc build should reject __init__ redefinition with exit code 1"
    );
}

/// Issue #386: redefining a method with a *different* signature is
/// rejected by `check_incompatible_redefinitions` with T0021 -- the same
/// check that covers module-level function redefinitions (class methods
/// are in the flat `hir.items` list as `HirItem::Function`s with mangled
/// names). This is a `pycc check` error.
#[test]
fn incompatible_method_redefinition_is_a_check_error() {
    let dir = ScratchDir::new("issue386_incompat_check").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    def foo(self, x: int, y: int) -> int:\n        return x + y\n\nc = C()\nprint(c.foo(1, 2))\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject incompatible method redefinition with exit code 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cannot redefine function `C.foo` with a different signature"),
        "stdout should mention incompatible method redefinition, got: {stdout}"
    );
}

/// Issue #386: incompatible method redefinition is also a `pycc build`
/// error.
#[test]
fn incompatible_method_redefinition_is_a_build_error() {
    let dir = ScratchDir::new("issue386_incompat_build").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    def foo(self, x: int, y: int) -> int:\n        return x + y\n\nc = C()\nprint(c.foo(1, 2))\n",
    );
    let out = dir.join("incompat");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc build should reject incompatible method redefinition with exit code 1"
    );
}

/// Issue #386 (Codex review P2): redefining a method with *different
/// parameter names* but the same type shape must not produce a false
/// T0021 "not bound" error. Each definition's body is checked against
/// its own parameter names, not the last-inserted signature's names.
/// This test uses annotated parameters (the inference path for
/// unannotated private class method parameters is a separate,
/// pre-existing limitation — see #386 comment).
#[test]
fn redefinition_with_different_parameter_names_does_not_false_positive() {
    let dir = ScratchDir::new("issue386_diff_names").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "diff_names.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x + 1\n    def foo(self, y: int) -> int:\n        return y + 2\n\nc = C()\nprint(c.foo(10))\n",
    );
    let out = dir.join("diff_names");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for method redefinition with different parameter names"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"12\n",
        "the latest definition should be called, printing 12"
    );
}
