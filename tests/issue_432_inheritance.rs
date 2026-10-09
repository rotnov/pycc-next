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

/// #432: a derived class with its own `__init__` and a method override.
/// The derived method is called, not the base method (static dispatch via
/// the MRO, which lists the derived class first).
#[test]
fn method_override_resolves_to_derived_class() {
    let dir = ScratchDir::new("432_override").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "override.py",
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 2\nb = B()\nprint(b.f())\n",
    );
    let out = dir.join("override");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a method override"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"2\n",
        "the derived class's override should be called"
    );
}

/// #432: a derived class inherits a method from its base class (no override
/// in the derived class). The base class's method is called. The derived
/// class also inherits the base's `__init__`, so the attribute is properly
/// initialized.
#[test]
fn inherited_method_is_callable_on_derived_instance() {
    let dir = ScratchDir::new("432_inherited").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inherited.py",
        "class A:\n    def __init__(self) -> None:\n        self.x = 10\n    def fetch_x(self) -> int:\n        return self.x\nclass B(A):\n    def bump(self) -> int:\n        return self.x + 1\nb = B()\nprint(b.fetch_x())\nprint(b.bump())\n",
    );
    let out = dir.join("inherited");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for an inherited method call"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"10\n11\n",
        "the inherited method should return the base class's attribute, and the derived method should use it too"
    );
}

/// #432: a derived class without its own `__init__` inherits the base
/// class's constructor. `Derived(42)` calls `Base.__init__(self, 42)`.
#[test]
fn inherited_init_is_used_for_instantiation() {
    let dir = ScratchDir::new("432_inhinit").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inhinit.py",
        "class Base:\n    def __init__(self, x: int) -> None:\n        self.x = x\n    def fetch(self) -> int:\n        return self.x\nclass Derived(Base):\n    def double(self) -> int:\n        return self.x + self.x\nd = Derived(42)\nprint(d.fetch())\nprint(d.double())\n",
    );
    let out = dir.join("inhinit");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for inherited __init__"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"42\n84\n",
        "inherited __init__ should initialize the base attribute, and both inherited and derived methods should work"
    );
}

/// #432: a derived class inherits an attribute from its base class. The
/// attribute is written and read via the inherited `__init__` and from an
/// external instance access.
#[test]
fn inherited_attribute_is_accessible_on_derived_instance() {
    let dir = ScratchDir::new("432_inhattr").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "inhattr.py",
        "class Base:\n    def __init__(self, x: int) -> None:\n        self.x = x\nclass Derived(Base):\n    def bump(self) -> int:\n        return self.x + 1\nd = Derived(10)\nprint(d.x)\nd.x = 99\nprint(d.x)\nprint(d.bump())\n",
    );
    let out = dir.join("inhattr");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for inherited attribute access"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"10\n99\n100\n",
        "inherited attribute should be writable and readable on the derived instance"
    );
}

/// #432: `@override` on a method that does override a base class method
/// builds and runs correctly.
#[test]
fn override_decorator_on_valid_override_builds_and_runs() {
    let dir = ScratchDir::new("432_valid_override").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "valid_override.py",
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def f(self) -> int:\n        return 2\nb = B()\nprint(b.f())\n",
    );
    let out = dir.join("valid_override");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a valid @override"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"2\n",
        "the @override-decorated method should be called"
    );
}

/// #432: `@override` on a method that does NOT override any base class
/// method is rejected with T0031 at compile time.
#[test]
fn override_decorator_on_nonexistent_method_is_a_check_error() {
    let dir = ScratchDir::new("432_bad_override").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "bad_override.py",
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def g(self) -> int:\n        return 2\n",
    );
    let out = dir.join("bad_override");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for @override on a non-existent method"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("T0031"),
        "stderr should contain T0031, got: {stderr}"
    );
}

/// #432: a class inheriting from an unknown base class is rejected.
#[test]
fn unknown_base_class_is_a_build_error() {
    let dir = ScratchDir::new("432_unknown_base").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "unknown_base.py",
        "class C(Base):\n    def __init__(self) -> None:\n        return\n",
    );
    let out = dir.join("unknown_base");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc build should fail for an unknown base class"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("C0001"),
        "stderr should contain C0001, got: {stderr}"
    );
}

/// #432 / #912: a class with no `__init__` and no base class providing one
/// used to be rejected with a `C0001`. Since D-225 it gets an implicit
/// zero-argument constructor synthesized instead, so it builds, instantiates
/// and dispatches its own methods.
#[test]
fn class_without_init_and_no_base_init_builds_and_runs() {
    let dir = ScratchDir::new("432_no_init").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "no_init.py",
        "class C:\n    def f(self) -> int:\n        return 1\nc = C()\nprint(c.f())\n",
    );
    let out = dir.join("no_init");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc build should succeed for a class without __init__, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let run = Command::new(&out).output().unwrap();
    assert_eq!(
        run.stdout, b"1\n",
        "the implicitly-constructed instance should dispatch its own method"
    );
}

/// #432: multiple inheritance with a diamond pattern. The C3 MRO determines
/// which method is called.
#[test]
fn diamond_inheritance_resolves_via_c3_mro() {
    let dir = ScratchDir::new("432_diamond").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "diamond.py",
        "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 2\nclass C(A):\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 3\nclass D(B, C):\n    def __init__(self) -> None:\n        return\nd = D()\nprint(d.f())\n",
    );
    let out = dir.join("diamond");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for diamond inheritance"
    );

    let output = Command::new(&out).output().unwrap();
    // C3 MRO for D is [D, B, C, A]. B.f is found first (before C.f).
    assert_eq!(
        output.stdout, b"2\n",
        "D's MRO is [D, B, C, A], so B.f should be called"
    );
}

/// #432: a derived class that adds new attributes while inheriting a
/// method from the base that accesses base attributes. The slot layout
/// must be most-base-first so that the inherited method reads the correct
/// slot (not a slot that shifted due to the derived class's new attrs).
#[test]
fn inherited_method_reads_correct_slot_with_derived_attrs() {
    let dir = ScratchDir::new("issue432_slot").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "slot.py",
        "class Animal:\n    def __init__(self, name: str) -> None:\n        self.name = name\n    def speak(self) -> str:\n        return self.name\nclass Dog(Animal):\n    def __init__(self, name: str, breed: str) -> None:\n        self.breed = breed\n        self.name = name\nd = Dog(\"Rex\", \"Labrador\")\nprint(d.speak())\nprint(d.name)\nprint(d.breed)\n",
    );
    let out = dir.join("slot");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for inherited method with derived attrs"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"Rex\nRex\nLabrador\n",
        "inherited method speak() should read the correct slot (name, not breed)"
    );
}

/// #432: a generic class with base classes is rejected with a clear error.
#[test]
fn generic_class_with_bases_is_rejected() {
    let dir = ScratchDir::new("issue432_generic_base").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "generic_base.py",
        "class Base:\n    def __init__(self) -> None:\n        return\nclass C[T](Base):\n    def __init__(self, x: T) -> None:\n        self.x = x\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc check should fail for a generic class with base classes"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("generic class") && stdout.contains("base classes"),
        "error should mention generic class with base classes, got: {stdout}"
    );
}
