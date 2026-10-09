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

/// #436: a basic `@staticmethod` called through the class name.
/// `C.create(42)` calls `C.create.static(42)` with no implicit receiver.
#[test]
fn static_method_call_through_class_builds_and_runs() {
    let dir = ScratchDir::new("436_static_class").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_class.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x + 1\nprint(C.create(42))\n",
    );
    let out = dir.join("static_class");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a static method called through a class"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"43\n", "C.create(42) should return 43");
}

/// #436: a `@staticmethod` called through an instance. The instance is
/// not passed as a receiver — only explicit arguments are forwarded.
#[test]
fn static_method_call_through_instance_builds_and_runs() {
    let dir = ScratchDir::new("436_static_inst").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_inst.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def combine(a: int, b: int) -> int:\n        return a + b\nc = C()\nprint(c.combine(10, 20))\n",
    );
    let out = dir.join("static_inst");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a static method called through an instance"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"30\n", "c.combine(10, 20) should return 30");
}

/// #436: a basic `@classmethod` called through the class name. The
/// compiler passes a null instance as `cls` (the static-dispatch model
/// does not need a real class object). The method returns a value
/// computed from its explicit arguments without dereferencing `cls`.
#[test]
fn class_method_call_through_class_builds_and_runs() {
    let dir = ScratchDir::new("436_classmethod_class").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "classmethod_class.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x * 2\nprint(C.greet(21))\n",
    );
    let out = dir.join("classmethod_class");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a class method called through a class"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n", "C.greet(21) should return 42");
}

/// #436: a `@classmethod` called through an instance. The instance is
/// passed as the implicit `cls` parameter.
#[test]
fn class_method_call_through_instance_builds_and_runs() {
    let dir = ScratchDir::new("436_classmethod_inst").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "classmethod_inst.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x * 2\nc = C()\nprint(c.greet(21))\n",
    );
    let out = dir.join("classmethod_inst");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a class method called through an instance"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n", "c.greet(21) should return 42");
}

/// #436: a `@classmethod` that uses `cls` to read an instance attribute.
/// Called through an instance, `cls` is the instance pointer, so
/// `cls.attr` reads the instance's attribute slot.
#[test]
fn class_method_using_cls_attribute_builds_and_runs() {
    let dir = ScratchDir::new("436_cls_attr").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "cls_attr.py",
        "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n    @classmethod\n    def fetch_x(cls) -> int:\n        return cls.x\nc = C(99)\nprint(c.fetch_x())\n",
    );
    let out = dir.join("cls_attr");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a class method using cls.attr"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"99\n",
        "c.fetch_x() should return 99 (the instance's x attribute via cls)"
    );
}

/// #436: a `@staticmethod` with no parameters. Called through the class
/// name, it takes no arguments and returns a constant.
#[test]
fn static_method_with_no_parameters_builds_and_runs() {
    let dir = ScratchDir::new("436_static_noargs").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_noargs.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def answer() -> int:\n        return 42\nprint(C.answer())\n",
    );
    let out = dir.join("static_noargs");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a static method with no parameters"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n", "C.answer() should return 42");
}

/// #436: an inherited `@staticmethod`. A derived class calls a static
/// method defined in its base class through the derived class name.
#[test]
fn inherited_static_method_builds_and_runs() {
    let dir = ScratchDir::new("436_inh_static").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inh_static.py",
        "class Base:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def helper(x: int) -> int:\n        return x + 100\nclass Derived(Base):\n    def __init__(self) -> None:\n        return\nprint(Derived.helper(7))\n",
    );
    let out = dir.join("inh_static");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for an inherited static method"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"107\n",
        "Derived.helper(7) should return 107 (inherited from Base)"
    );
}

/// #436: an inherited `@classmethod`. A derived class calls a class
/// method defined in its base class through the derived class name.
#[test]
fn inherited_class_method_builds_and_runs() {
    let dir = ScratchDir::new("436_inh_classmethod").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inh_classmethod.py",
        "class Base:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def helper(cls, x: int) -> int:\n        return x + 200\nclass Derived(Base):\n    def __init__(self) -> None:\n        return\nprint(Derived.helper(5))\n",
    );
    let out = dir.join("inh_classmethod");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for an inherited class method"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"205\n",
        "Derived.helper(5) should return 205 (inherited from Base)"
    );
}

/// #436: `@staticmethod` on `__init__` is rejected with C0001 — the
/// constructor must be a regular instance method.
#[test]
fn staticmethod_on_init_is_a_build_error() {
    let dir = ScratchDir::new("436_static_init").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_init.py",
        "class C:\n    @staticmethod\n    def __init__(x: int) -> None:\n        return\n",
    );
    let out = dir.join("static_init");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @staticmethod on __init__"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "should report C0001 for @staticmethod on __init__, got: {stderr}"
    );
}

/// #436: `@classmethod` on `__init__` is rejected with C0001 — the
/// constructor must be a regular instance method.
#[test]
fn classmethod_on_init_is_a_build_error() {
    let dir = ScratchDir::new("436_classmethod_init").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "classmethod_init.py",
        "class C:\n    @classmethod\n    def __init__(cls) -> None:\n        return\n",
    );
    let out = dir.join("classmethod_init");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @classmethod on __init__"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "should report C0001 for @classmethod on __init__, got: {stderr}"
    );
}

/// #436: wrong argument types for a `@staticmethod` are rejected at
/// type-check time with T0021.
#[test]
fn static_method_wrong_argument_types_is_a_check_error() {
    let dir = ScratchDir::new("436_static_wrongtype").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_wrongtype.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def combine(a: int, b: int) -> int:\n        return a + b\nprint(C.combine(\"hello\", 3))\n",
    );
    let out = dir.join("static_wrongtype");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for wrong static method argument types"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for wrong static method argument types, got: {stderr}"
    );
}

/// #436: wrong argument types for a `@classmethod` are rejected at
/// type-check time with T0021. The implicit `cls` parameter is excluded
/// from the argument count/type check.
#[test]
fn class_method_wrong_argument_types_is_a_check_error() {
    let dir = ScratchDir::new("436_classmethod_wrongtype").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "classmethod_wrongtype.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\nprint(C.greet(\"hello\"))\n",
    );
    let out = dir.join("classmethod_wrongtype");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for wrong class method argument types"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0021"),
        "should report T0021 for wrong class method argument types, got: {stderr}"
    );
}

/// #436: a `@staticmethod` that declares `self` as its first parameter
/// is treated as a regular parameter. Since the method is public and
/// `self` has no type annotation, this fails with T0001 (missing
/// annotation) — the user has incorrectly used `self` in a context
/// where no implicit receiver exists.
#[test]
fn static_method_declaring_self_is_a_check_error() {
    let dir = ScratchDir::new("436_static_self").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "static_self.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def f(self) -> int:\n        return 1\n",
    );
    let out = dir.join("static_self");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for a static method declaring self"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0001"),
        "should report T0001 for an unannotated self in a public static method, got: {stderr}"
    );
}

/// #436: a `@classmethod` without a `cls` parameter is rejected with
/// C0001 at HIR-lowering time.
#[test]
fn class_method_without_cls_is_a_build_error() {
    let dir = ScratchDir::new("436_no_cls").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "no_cls.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(x: int) -> int:\n        return x\n",
    );
    let out = dir.join("no_cls");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for a class method without cls"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "should report C0001 for a class method without cls, got: {stderr}"
    );
}

/// #436 review fix: a @staticmethod cannot share a name with a regular
/// method in the same class.
#[test]
fn static_method_collides_with_regular_method_is_a_build_error() {
    let dir = ScratchDir::new("issue436_collide_sm").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "collide_sm.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    @staticmethod\n    def foo(x: int) -> int:\n        return x + 1\n",
    );
    let out = dir.join("collide_sm");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @staticmethod colliding with a regular method"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001") && stderr.contains("cannot share a name"),
        "should report C0001 with collision message, got: {stderr}"
    );
}

/// #436 review fix: a @classmethod cannot share a name with a regular
/// method in the same class.
#[test]
fn class_method_collides_with_regular_method_is_a_build_error() {
    let dir = ScratchDir::new("issue436_collide_cm").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "collide_cm.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x + 1\n",
    );
    let out = dir.join("collide_cm");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @classmethod colliding with a regular method"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001") && stderr.contains("cannot share a name"),
        "should report C0001 with collision message, got: {stderr}"
    );
}

/// #436 review fix: a @staticmethod cannot share a name with a
/// @classmethod in the same class.
#[test]
fn static_method_collides_with_class_method_is_a_build_error() {
    let dir = ScratchDir::new("issue436_collide_both").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "collide_both.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def foo(x: int) -> int:\n        return x\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x + 1\n",
    );
    let out = dir.join("collide_both");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @staticmethod colliding with @classmethod"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001") && stderr.contains("cannot share a name"),
        "should report C0001 with collision message, got: {stderr}"
    );
}

/// #436 review fix: a regular method defined after a @staticmethod with
/// the same name is also rejected (reverse direction collision).
#[test]
fn regular_method_collides_with_static_method_is_a_build_error() {
    let dir = ScratchDir::new("issue436_collide_rev").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "collide_rev.py",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def foo(x: int) -> int:\n        return x\n    def foo(self, x: int) -> int:\n        return x + 1\n",
    );
    let out = dir.join("collide_rev");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for a regular method colliding with @staticmethod"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001") && stderr.contains("cannot share a name"),
        "should report C0001 with collision message, got: {stderr}"
    );
}
