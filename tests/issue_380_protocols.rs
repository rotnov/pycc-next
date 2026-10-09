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

fn build_and_run(dir: &std::path::Path, name: &str, source: &str) -> (bool, Vec<u8>, Vec<u8>) {
    let src = write_fixture(dir, &format!("{name}.py"), source);
    let out = dir.join(name);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !status.status.success() {
        return (false, status.stdout, status.stderr);
    }
    let run = Command::new(&out).output().unwrap();
    (true, run.stdout, run.stderr)
}

// ===========================================================================
// Part A: Protocol declaration and structural conformance (success)
// ===========================================================================

/// #380: A basic protocol with a method, and a conforming class —
/// assignment to a protocol-typed variable compiles and runs.
#[test]
fn protocol_basic_conformance_compiles() {
    let dir = ScratchDir::new("380_basic").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Drawable(Protocol):
    def draw(self) -> None: ...

class Circle:
    def __init__(self) -> None:
        self.x = 0
    def draw(self) -> None:
        pass

c: Drawable = Circle()
";
    let src = write_fixture(&dir, "basic.py", source);
    let out = dir.join("basic");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for basic protocol conformance"
    );
}

/// #380: A protocol with an attribute, and a conforming class.
#[test]
fn protocol_attribute_conformance_compiles() {
    let dir = ScratchDir::new("380_attr").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Named(Protocol):
    name: str

class Person:
    def __init__(self) -> None:
        self.name = \"Alice\"

p: Named = Person()
";
    let src = write_fixture(&dir, "attr.py", source);
    let out = dir.join("attr");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for protocol attribute conformance"
    );
}

/// #380: Protocol inheritance — a sub-protocol extends a base protocol,
/// and a class conforming to the sub-protocol compiles.
#[test]
fn protocol_inheritance_compiles() {
    let dir = ScratchDir::new("380_inh").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Base(Protocol):
    def foo(self) -> int: ...

class Extended(Base):
    def bar(self) -> int: ...

class Impl:
    def __init__(self) -> None:
        self.x = 0
    def foo(self) -> int:
        return 1
    def bar(self) -> int:
        return 2

e: Extended = Impl()
";
    let src = write_fixture(&dir, "inh.py", source);
    let out = dir.join("inh");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for protocol inheritance"
    );
}

/// #380: A protocol-typed function parameter — calling the function
/// with a conforming class instance compiles and runs.
#[test]
fn protocol_typed_parameter_compiles() {
    let dir = ScratchDir::new("380_param").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Greeter(Protocol):
    def greet(self) -> str: ...

class English:
    def __init__(self) -> None:
        self.x = 0
    def greet(self) -> str:
        return \"hello\"

def say_hello(g: Greeter) -> None:
    print(g.greet())

say_hello(English())
";
    let (ok, stdout, stderr) = build_and_run(&dir, "param", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"hello\n");
}

// ===========================================================================
// Part B: Protocol non-conformance (T0046)
// ===========================================================================

/// #380: A class missing a required protocol method triggers T0046.
#[test]
fn protocol_missing_method_emits_t0046() {
    let dir = ScratchDir::new("380_missing").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Drawable(Protocol):
    def draw(self) -> None: ...

class Square:
    def __init__(self) -> None:
        self.x = 0

s: Drawable = Square()
";
    let src = write_fixture(&dir, "missing.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("missing").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for non-conforming class"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("T0046"),
        "should emit T0046, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("missing") || stderr.contains("does not conform"),
        "should mention missing method or non-conformance, got: {stderr}"
    );
}

/// #380: A class with a method with the wrong return type triggers T0046.
#[test]
fn protocol_wrong_return_type_emits_t0046() {
    let dir = ScratchDir::new("380_wr").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class IntProducer(Protocol):
    def produce(self) -> int: ...

class StrProducer:
    def __init__(self) -> None:
        self.x = 0
    def produce(self) -> str:
        return \"oops\"

s: IntProducer = StrProducer()
";
    let src = write_fixture(&dir, "wrong.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("wrong").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for wrong return type"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("T0046"),
        "should emit T0046, got stderr: {stderr}"
    );
}

/// #380: A class missing a required protocol attribute triggers T0046.
#[test]
fn protocol_missing_attribute_emits_t0046() {
    let dir = ScratchDir::new("380_miss_attr").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Named(Protocol):
    name: str

class Anon:
    def __init__(self) -> None:
        self.x = 0

n: Named = Anon()
";
    let src = write_fixture(&dir, "missattr.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("missattr").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for missing attribute"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("T0046"),
        "should emit T0046, got stderr: {stderr}"
    );
}

// ===========================================================================
// Part C: @runtime_checkable and isinstance
// ===========================================================================

/// #380: `isinstance` with a `@runtime_checkable` protocol returns True
/// for a conforming class.
#[test]
fn runtime_checkable_isinstance_true() {
    let dir = ScratchDir::new("380_rt_true").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol, runtime_checkable

@runtime_checkable
class Drawable(Protocol):
    def draw(self) -> None: ...

class Circle:
    def __init__(self) -> None:
        self.x = 0
    def draw(self) -> None:
        pass

c = Circle()
print(isinstance(c, Drawable))
";
    let (ok, stdout, stderr) = build_and_run(&dir, "rt_true", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"True\n");
}

/// #380: `isinstance` with a `@runtime_checkable` protocol returns False
/// for a non-conforming class.
#[test]
fn runtime_checkable_isinstance_false() {
    let dir = ScratchDir::new("380_rt_false").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol, runtime_checkable

@runtime_checkable
class Drawable(Protocol):
    def draw(self) -> None: ...

class Square:
    def __init__(self) -> None:
        self.x = 0

s = Square()
print(isinstance(s, Drawable))
";
    let (ok, stdout, stderr) = build_and_run(&dir, "rt_false", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"False\n");
}

/// #380: `isinstance` against a non-`@runtime_checkable` protocol is
/// rejected at compile time.
#[test]
fn isinstance_non_runtime_checkable_rejected() {
    let dir = ScratchDir::new("380_rt_rej").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Drawable(Protocol):
    def draw(self) -> None: ...

class Circle:
    def __init__(self) -> None:
        self.x = 0
    def draw(self) -> None:
        pass

c = Circle()
print(isinstance(c, Drawable))
";
    let src = write_fixture(&dir, "rt_rej.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("rt_rej").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for isinstance against non-runtime_checkable protocol"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("runtime_checkable"),
        "should mention runtime_checkable, got: {stderr}"
    );
}

/// #380: `issubclass` with a protocol is rejected.
#[test]
fn issubclass_with_protocol_rejected() {
    let dir = ScratchDir::new("380_iss_rej").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol, runtime_checkable

@runtime_checkable
class Drawable(Protocol):
    def draw(self) -> None: ...

class Circle:
    def __init__(self) -> None:
        self.x = 0
    def draw(self) -> None:
        pass

print(issubclass(Circle, Drawable))
";
    let src = write_fixture(&dir, "iss_rej.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("iss_rej").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for issubclass with a protocol"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("issubclass") && stderr.contains("protocol"),
        "should mention issubclass and protocol, got: {stderr}"
    );
}

// ===========================================================================
// Part D: Protocol-typed variable method calls
// ===========================================================================

/// #380: Calling a method on a protocol-typed variable resolves to the
/// protocol's declared return type and compiles.
#[test]
fn protocol_typed_variable_method_call() {
    let dir = ScratchDir::new("380_mcall").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class IntProvider(Protocol):
    def get_value(self) -> int: ...

class Counter:
    def __init__(self) -> None:
        self.n = 42
    def get_value(self) -> int:
        return self.n

c: IntProvider = Counter()
print(c.get_value())
";
    let (ok, stdout, stderr) = build_and_run(&dir, "mcall", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"42\n");
}

/// #380: Calling a protocol-typed variable's method with the wrong
/// argument count is rejected — exercises `check_call_args`'s error
/// path in the protocol method-call arm of `resolve_method_call`.
#[test]
fn protocol_typed_variable_method_call_wrong_arity_rejected() {
    let dir = ScratchDir::new("380_marity").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class IntProvider(Protocol):
    def get_value(self) -> int: ...

class Counter:
    def __init__(self) -> None:
        self.n = 42
    def get_value(self) -> int:
        return self.n

c: IntProvider = Counter()
print(c.get_value(99))
";
    let src = write_fixture(&dir, "marity.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("marity").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for protocol method call with wrong arity"
    );
}

/// #380: Reading a protocol attribute on a protocol-typed variable.
#[test]
fn protocol_typed_variable_attribute_read() {
    let dir = ScratchDir::new("380_aread").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Named(Protocol):
    name: str

class Person:
    def __init__(self) -> None:
        self.name = \"Bob\"

p: Named = Person()
print(p.name)
";
    let (ok, stdout, stderr) = build_and_run(&dir, "aread", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"Bob\n");
}

// ===========================================================================
// Part E: ABC and @abstractmethod
// ===========================================================================

/// #380: An ABC with an @abstractmethod — a concrete subclass that
/// overrides the abstract method compiles.
#[test]
fn abc_abstract_method_override_compiles() {
    let dir = ScratchDir::new("380_abc").expect("failed to create scratch dir");
    let source = "\
from abc import ABC, abstractmethod

class Animal(ABC):
    def __init__(self) -> None:
        self.x = 0
    @abstractmethod
    def sound(self) -> str: ...

class Dog(Animal):
    def __init__(self) -> None:
        super().__init__()
    def sound(self) -> str:
        return \"woof\"

d = Dog()
print(d.sound())
";
    let (ok, stdout, stderr) = build_and_run(&dir, "abc", source);
    assert!(
        ok,
        "build should succeed, stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"woof\n");
}

/// #380: An ABC with an @abstractmethod — a subclass that does NOT
/// override the abstract method is rejected.
#[test]
fn abc_abstract_method_not_overridden_rejected() {
    let dir = ScratchDir::new("380_abc_no").expect("failed to create scratch dir");
    let source = "\
from abc import ABC, abstractmethod

class Animal(ABC):
    def __init__(self) -> None:
        self.x = 0
    @abstractmethod
    def sound(self) -> str: ...

class Cat(Animal):
    def __init__(self) -> None:
        super().__init__()

c = Cat()
";
    let src = write_fixture(&dir, "abc_no.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("abc_no").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for unoverridden abstract method"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("abstract"),
        "should mention abstract, got: {stderr}"
    );
}

/// #380: An abstract class (with unoverridden abstract methods) can be
/// subclassed but not instantiated directly.
#[test]
fn abc_cannot_instantiate_abstract_class() {
    let dir = ScratchDir::new("380_abc_inst").expect("failed to create scratch dir");
    let source = "\
from abc import ABC, abstractmethod

class Animal(ABC):
    def __init__(self) -> None:
        self.x = 0
    @abstractmethod
    def sound(self) -> str: ...

a = Animal()
";
    let src = write_fixture(&dir, "abc_inst.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("abc_inst").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for instantiating an abstract class"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("abstract"),
        "should mention abstract, got: {stderr}"
    );
}

// ===========================================================================
// Part F: Protocol body validation
// ===========================================================================

/// #380: A protocol method with an implementation body (not `...` or
/// `pass`) is rejected.
#[test]
fn protocol_method_with_body_rejected() {
    let dir = ScratchDir::new("380_body").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class P(Protocol):
    def foo(self) -> int:
        return 1
";
    let src = write_fixture(&dir, "body.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("body").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for protocol method with body"
    );
}

/// #380: A protocol method with a docstring body (a non-ellipsis
/// expression statement) is rejected — `is_declaration_body` must
/// return `false` for `Stmt::Expr` variants other than `EllipsisLiteral`.
#[test]
fn protocol_method_with_docstring_body_rejected() {
    let dir = ScratchDir::new("380_doc").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class P(Protocol):
    def foo(self) -> int:
        \"docstring\"
";
    let src = write_fixture(&dir, "doc.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("doc").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for protocol method with docstring body"
    );
}

/// #380: A protocol attribute with a default value is rejected.
#[test]
fn protocol_attribute_with_default_rejected() {
    let dir = ScratchDir::new("380_def").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class P(Protocol):
    x: int = 42
";
    let src = write_fixture(&dir, "def.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("def").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for protocol attribute with default"
    );
}

/// #380: `@runtime_checkable` on a non-protocol class is rejected.
#[test]
fn runtime_checkable_on_non_protocol_rejected() {
    let dir = ScratchDir::new("380_rt_np").expect("failed to create scratch dir");
    let source = "\
from typing import runtime_checkable

@runtime_checkable
class C:
    def __init__(self) -> None:
        self.x = 1
";
    let src = write_fixture(&dir, "rt_np.py", source);
    let out = Command::new(pycc_bin())
        .args([
            "build",
            src.to_str().unwrap(),
            "-o",
            dir.join("rt_np").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "pycc build should fail for @runtime_checkable on non-protocol"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("runtime_checkable") && stderr.contains("protocol"),
        "should mention runtime_checkable and protocol, got: {stderr}"
    );
}

/// #380: Protocol-to-protocol assignability — assigning a sub-protocol
/// typed value to a base-protocol typed variable compiles.
#[test]
fn protocol_to_protocol_assignability() {
    let dir = ScratchDir::new("380_p2p").expect("failed to create scratch dir");
    let source = "\
from typing import Protocol

class Base(Protocol):
    def foo(self) -> int: ...

class Sub(Base):
    def bar(self) -> int: ...

class Impl:
    def __init__(self) -> None:
        self.x = 0
    def foo(self) -> int:
        return 1
    def bar(self) -> int:
        return 2

s: Sub = Impl()
b: Base = s
";
    let src = write_fixture(&dir, "p2p.py", source);
    let out = dir.join("p2p");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for protocol-to-protocol assignment"
    );
}
