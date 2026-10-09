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

/// #377: a `@property` getter is transparently rewritten to a method call
/// at the HIR/MIR level. `obj.x` becomes `obj.x()` (a call to the getter),
/// producing the same output as CPython.
#[test]
fn property_getter_builds_and_runs_correctly() {
    let dir = ScratchDir::new("issue377_getter").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "getter.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 42\n    @property\n    def x(self) -> int:\n        return self._x\n\nc = C()\nprint(c.x)\n",
    );
    let out = dir.join("getter");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property getter"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"42\n",
        "property getter should return the backing slot value"
    );
}

/// #377: a `@property` getter + `@<name>.setter` pair transparently rewrites
/// both reads and writes to method calls. `obj.x` becomes `obj.x()` and
/// `obj.x = v` becomes `obj.x.setter(v)`.
#[test]
fn property_setter_builds_and_runs_correctly() {
    let dir = ScratchDir::new("issue377_setter").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "setter.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n\nc = C()\nprint(c.x)\nc.x = 99\nprint(c.x)\n",
    );
    let out = dir.join("setter");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property getter+setter"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"0\n99\n",
        "property setter should update the backing slot via the setter method"
    );
}

/// #377: a read-only property (getter only, no setter) correctly rejects
/// assignment at type-check time with a T0044 diagnostic.
#[test]
fn read_only_property_assignment_is_a_check_error() {
    let dir = ScratchDir::new("issue377_ro").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "readonly.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n\nc = C()\nc.x = 1\n",
    );
    let out = dir.join("readonly");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for assigning to a read-only property"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0044") && stderr.contains("read-only"),
        "expected T0044 read-only diagnostic, got stderr: {stderr}"
    );
}

/// #377: a property setter with a type mismatch (assigning a `str` to an
/// `int`-typed setter) is rejected at type-check time with a T0021
/// diagnostic.
#[test]
fn property_setter_type_mismatch_is_a_check_error() {
    let dir = ScratchDir::new("issue377_mismatch").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "mismatch.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n\nc = C()\nc.x = \"hello\"\n",
    );
    let out = dir.join("mismatch");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for a property setter type mismatch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "expected T0021 type mismatch diagnostic, got stderr: {stderr}"
    );
}

/// #377: a property getter can be read from within another method of the
/// same class via `self.x`, exercising `resolve_attr_get`'s property check
/// from within a function body (pass 3), not just top-level (pass 2).
#[test]
fn property_getter_readable_from_another_method() {
    let dir = ScratchDir::new("issue377_method").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "method_read.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 7\n    @property\n    def x(self) -> int:\n        return self._x\n    def show(self) -> None:\n        print(self.x)\n\nc = C()\nc.show()\n",
    );
    let out = dir.join("method_read");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property read from a method body"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"7\n",
        "property read from a method body should return the backing slot value"
    );
}

/// #377: a property setter can be written from within another method of
/// the same class via `self.x = v`, exercising `check_attr_set`'s property
/// check from within a function body.
#[test]
fn property_setter_writable_from_another_method() {
    let dir = ScratchDir::new("issue377_method_write").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "method_write.py",
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n    def set_to_5(self) -> None:\n        self.x = 5\n\nc = C()\nc.set_to_5()\nprint(c.x)\n",
    );
    let out = dir.join("method_write");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property write from a method body"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"5\n",
        "property write from a method body should update the backing slot"
    );
}

/// #377: a `@property` on a generic class (PEP 695 `class C[T]:`) is
/// monomorphized alongside the class's methods. The property getter on
/// `Box[int]` should return the backing slot value after monomorphization.
#[test]
fn property_on_a_generic_class_builds_and_runs_correctly() {
    let dir = ScratchDir::new("issue377_generic").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "generic_property.py",
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self._v = v\n    @property\n    def v(self) -> T:\n        return self._v\n\nb = Box[int](42)\nprint(b.v)\n",
    );
    let out = dir.join("generic_property");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property on a generic class"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"42\n",
        "property on a generic class should return the backing slot value"
    );
}

/// #377: a `@property` getter and setter on a generic class (PEP 695)
/// are both monomorphized. The setter should update the backing slot and
/// the getter should return the updated value.
#[test]
fn property_setter_on_a_generic_class_builds_and_runs_correctly() {
    let dir = ScratchDir::new("issue377_generic_setter").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "generic_property_setter.py",
        "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self._v = v\n    @property\n    def v(self) -> T:\n        return self._v\n    @v.setter\n    def v(self, new_v: T) -> None:\n        self._v = new_v\n\nb = Box[int](42)\nprint(b.v)\nb.v = 99\nprint(b.v)\n",
    );
    let out = dir.join("generic_property_setter");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a property setter on a generic class"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"42\n99\n",
        "property setter on a generic class should update the backing slot"
    );
}
