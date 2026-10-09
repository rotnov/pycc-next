//! Issue #914 (follow-up to #911, Part 1 of #885): a class-level attribute
//! satisfies a PEP 544 `Protocol` attribute member, end to end through the
//! public `pycc` CLI.
//!
//! Before this change `check_protocol_conformance`'s attribute arm consulted
//! only the instance-attribute and `@property` tables, so the issue's own
//! program was rejected with `T0046 ... missing attribute`. Two seams had to
//! move together: the conformance check itself, and
//! `pycc_mir::class::eval_isinstance_protocol`, whose presence check must
//! agree with it (#380 W2) or a `@runtime_checkable` `isinstance` folds to
//! `False` for a class the checker just certified as conforming.
//!
//! Every accepting test asserts the program's *stdout*, never a bare exit 0:
//! a class attribute has no runtime storage at all, so "it compiled" says
//! nothing about the value the read folded to.

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

/// Builds and runs `source`, asserting the program's stdout.
fn assert_runs(tag: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = dir.join("main.bin");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for {tag}:\n{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "compiled program {tag} should exit 0");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "stdout for {tag}"
    );
}

/// Asserts that `pycc check` rejects `source` with a diagnostic containing
/// both `code` and `needle`. Deliberately matches on the code and a message
/// substring rather than on a rendered path: the renderer prints
/// forward-slash paths on Windows CI.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded"
    );
    assert!(
        rendered.contains(code) && rendered.contains(needle),
        "diagnostic for {tag} should contain {code:?} and {needle:?}, got:\n{rendered}"
    );
}

/// The issue's own program: a class attribute satisfies the protocol's
/// attribute member, and the read through the monomorphized protocol-typed
/// parameter folds to the recorded constant.
#[test]
fn class_attribute_satisfies_a_protocol_attribute_member() {
    assert_runs(
        "issue914_param",
        "\
from typing import Protocol

class HasLimit(Protocol):
    limit: int

class C:
    limit: int = 1

    def __init__(self) -> None:
        self.n = 0

def read(p: HasLimit) -> int:
    return p.limit

print(read(C()))
",
        "1\n",
    );
}

/// The fallback walks the full MRO, so a class attribute declared on a base
/// satisfies the member for a derived class that declares none of its own.
#[test]
fn inherited_class_attribute_satisfies_a_protocol_attribute_member() {
    assert_runs(
        "issue914_inherited",
        "\
from typing import Protocol

class HasLimit(Protocol):
    limit: int

class Base:
    limit: int = 7

class C(Base):
    def __init__(self) -> None:
        self.n = 0

def read(p: HasLimit) -> int:
    return p.limit

print(read(C()))
",
        "7\n",
    );
}

/// A protocol carrying both a method and an attribute member, satisfied by a
/// method plus a class attribute, and read through a protocol-typed
/// module-level receiver as well as through a parameter.
#[test]
fn class_attribute_and_method_satisfy_a_mixed_protocol() {
    assert_runs(
        "issue914_mixed",
        "\
from typing import Protocol

class HasLimit(Protocol):
    limit: int
    def bump(self, n: int) -> int: ...

class Base:
    limit: int = 7

class C(Base):
    def __init__(self) -> None:
        self.n = 0

    def bump(self, n: int) -> int:
        return n + 1

def read(p: HasLimit) -> int:
    return p.limit + p.bump(1)

v: HasLimit = C()
print(read(C()))
print(v.limit)
",
        "9\n7\n",
    );
}

/// #380 W2: the conformance check and `isinstance` must agree. Before the
/// `pycc_mir` half of this change, `pycc check` accepted the class while
/// `isinstance` folded to `False`.
#[test]
fn isinstance_agrees_with_conformance_for_a_class_attribute() {
    assert_runs(
        "issue914_isinstance",
        "\
from typing import Protocol, runtime_checkable

@runtime_checkable
class HasLimit(Protocol):
    limit: int

class C:
    limit: int = 1

    def __init__(self) -> None:
        self.n = 0

class D:
    def __init__(self) -> None:
        self.n = 0

c = C()
d = D()
print(isinstance(c, HasLimit))
print(isinstance(d, HasLimit))
",
        "True\nFalse\n",
    );
}

/// A class attribute whose type does not match the member is still rejected,
/// with the same "has type" wording an instance attribute produces.
#[test]
fn wrong_typed_class_attribute_is_still_t0046() {
    assert_rejected(
        "issue914_mismatch",
        "\
from typing import Protocol

class HasLimit(Protocol):
    limit: int

class C:
    limit: str = \"x\"

    def __init__(self) -> None:
        self.n = 0

def read(p: HasLimit) -> int:
    return p.limit

print(read(C()))
",
        "T0046",
        "attribute `limit` has type `str`",
    );
}

/// A class with neither an instance nor a class attribute of that name is
/// still rejected as missing the member.
#[test]
fn absent_attribute_is_still_t0046_missing() {
    assert_rejected(
        "issue914_missing",
        "\
from typing import Protocol

class HasLimit(Protocol):
    limit: int

class C:
    other: int = 1

    def __init__(self) -> None:
        self.n = 0

def read(p: HasLimit) -> int:
    return p.limit

print(read(C()))
",
        "T0046",
        "missing attribute `limit`",
    );
}
