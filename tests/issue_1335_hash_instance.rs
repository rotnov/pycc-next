//! End-to-end proof for `hash()` of a user-class instance (#1335, Part 1 of
//! #1332).
//!
//! `docs/TYPE_SYSTEM.md`'s `hash()` section is the contract. Every program
//! also runs under whatever `PYCC_PYTHON`/`python3` is available, and its
//! stdout must equal both CPython's and the pinned text; the harness is
//! `tests/issue_1331_hash_builtin.rs`'s. An identity hash is an address, so
//! only its properties are printed: two distinct objects are both bound to
//! names, because CPython may reuse a freed temporary's address. The `--ext`
//! differential is `#[ignore]`d like its siblings and runs under
//! `--include-ignored` in CI.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// A user `__hash__` returning every row of the `slot_tp_hash` table (a heap
/// bigint that fits 64 bits passes through; a wider one is reduced; `-1`
/// becomes `-2`), a `-> bool` method, a `-> int` method returning `True`,
/// `hash((self.x, self.y))`, a typed parameter and an unannotated private
/// helper; the identity hash's properties; inheritance, a redefining
/// subclass, a dataclass with its own `__hash__`, `hash(self)` in a method;
/// one call per `hash()`, left to right; a raising `__hash__` mid-expression
/// and inside a private helper.
const SUCCESS: &str = "\
from dataclasses import dataclass


class V:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v


class T:
    def __init__(self, t: bool) -> None:
        self.t = t

    def __hash__(self) -> bool:
        return self.t


class Tr:
    def __init__(self) -> None:
        self.x = 0

    def __hash__(self) -> int:
        return True


class P:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def __hash__(self) -> int:
        return hash((self.x, self.y))


class R:
    def __init__(self, x: int) -> None:
        self.x = x


class Base:
    def __init__(self, x: int) -> None:
        self.x = x

    def __hash__(self) -> int:
        return self.x + 100


class Child(Base):
    pass


class Base2:
    def __init__(self, x: int) -> None:
        self.x = x


class Child2(Base2):
    def __hash__(self) -> int:
        return self.x + 200


@dataclass
class D:
    x: int

    def __hash__(self) -> int:
        return self.x * 3


class S:
    def __init__(self, x: int) -> None:
        self.x = x

    def __hash__(self) -> int:
        return self.x

    def key(self) -> int:
        return hash(self) + 1


order: list[int] = [0]


class C:
    def __init__(self, k: int) -> None:
        self.k = k

    def __hash__(self) -> int:
        order.append(self.k)
        return self.k


class Bad:
    def __init__(self) -> None:
        self.x = 0

    def __hash__(self) -> int:
        raise ValueError(\"no\")


def _h():
    r = V(5)
    return hash(r)


def _raise_h():
    b = Bad()
    return hash(b)


def _f(v: V) -> int:
    return hash(v)


p61 = 2305843009213693952
p62 = p61 + p61
p63 = p62 + p62
p64 = p63 + p63
p70 = p64
for _ in range(6):
    p70 = p70 + p70
print(hash(V(p61)), hash(V(p62)), hash(V(p63 - 1)))
print(hash(V(0 - p63)), hash(V(0 - p62 - 1)))
print(hash(V(p63)), hash(V(0 - p63 - 1)), hash(V(p64 + 5)), hash(V(0 - p70)))
print(hash(V(-1)), hash(V(-2)), hash(V(7)), _f(V(8)), _h())
print(hash(T(True)), hash(T(False)), hash(Tr()))
print(hash(P(1, 2)) == hash((1, 2)))
a = R(1)
b = R(2)
h = hash(a)
print(h == hash(a), hash(a) != hash(b), h != -1)
a.x = 5
print(hash(a) == h)
print(hash(Base(1)), hash(Child(2)), hash(Child2(3)), hash(D(4)), S(9).key())
c1 = C(1)
c2 = C(2)
print(hash(c1) + hash(c2), len(order) - 1, order[1], order[2])
try:
    x = 1 + hash(Bad())
    print(x)
except ValueError as e:
    print(\"caught\", e)
try:
    print(_raise_h())
except ValueError as e:
    print(\"helper caught\", e)
";

const SUCCESS_STDOUT: &str = "2305843009213693952 4611686018427387904 9223372036854775807\n\
                              -9223372036854775808 -4611686018427387905\n\
                              4 -5 13 -512\n\
                              -2 -2 7 8 5\n\
                              1 0 1\n\
                              True\n\
                              True True True\n\
                              True\n\
                              101 102 203 12 10\n\
                              3 2 1 2\n\
                              caught no\n\
                              helper caught no\n";

/// Builds `source` natively, runs it, and asserts its stdout equals
/// CPython's run of the same file and `expected`.
fn assert_native_matches_cpython(tag: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("a.py"))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(stdout(&run), stdout(&oracle));
    assert_eq!(stdout(&run), expected);
}

/// Builds `source` as the extension module `module`, imports it in CPython,
/// and asserts the import's stdout equals `runpy.run_path` of the same
/// source and `expected`.
fn assert_ext_matches_cpython(tag: &str, module: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("m.py"), source).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("m.py"))
        .arg("-o")
        // A bare name gains the host's own suffix (`.abi3.so` or `.pyd`).
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = |script: String| {
        python()
            .arg("-c")
            .arg(script)
            .current_dir(&*dir)
            .output()
            .expect("python3 should spawn")
    };
    let compiled = run(format!("import {module}"));
    assert!(compiled.status.success(), "{}", rendered(&compiled));
    let oracle = run("import runpy\nrunpy.run_path('m.py')".to_string());
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(stdout(&compiled), stdout(&oracle));
    assert_eq!(stdout(&compiled), expected);
}

/// Runs `pycc check --error-format json` on `source` and asserts it fails
/// with exactly one `code` diagnostic whose message contains `needle` and
/// whose help contains `help` (the human format prints no help line).
fn assert_one_error(tag: &str, source: &str, code: &str, needle: &str, help: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg(dir.join("a.py"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{source:?} was accepted");
    let text = rendered(&output);
    let [diagnostic] = text.lines().collect::<Vec<_>>()[..] else {
        panic!("expected exactly one diagnostic: {text}");
    };
    assert!(
        diagnostic.contains(&format!("\"code\":\"{code}\"")),
        "{text}"
    );
    assert!(
        diagnostic.contains(&format!("\"message\":\"{needle}")),
        "{text}"
    );
    let help_start = diagnostic.find("\"help\":").expect("a help field");
    let message_start = diagnostic.find("\"message\":").expect("a message field");
    assert!(
        diagnostic[help_start..message_start].contains(help),
        "{text}"
    );
}

const INIT: &str = "    def __init__(self) -> None:\n        self.x = 1\n";
const EQ: &str = "    def __eq__(self, other: int) -> bool:\n        return True\n";

/// `class R:` with `INIT` and then `body`, followed by `tail`.
fn class_r(body: &str, tail: &str) -> String {
    format!("class R:\n{INIT}\n{body}\n\n{tail}")
}

#[test]
fn instance_hash_programs_match_cpython() {
    assert_native_matches_cpython("e2e_1335_success", SUCCESS, SUCCESS_STDOUT);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn instance_hash_module_bodies_match_cpython_as_extensions() {
    assert_ext_matches_cpython(
        "e2e_1335_ext",
        "pycc_hash_instance_mod",
        SUCCESS,
        SUCCESS_STDOUT,
    );
}

#[test]
fn a_class_with_eq_but_no_hash_is_t0021_unhashable() {
    let help = "`A` defines `__eq__` without `__hash__`";
    let direct = format!("class A:\n{INIT}\n{EQ}\n\nprint(hash(A()))\n");
    assert_one_error(
        "e2e_1335_eq",
        &direct,
        "T0021",
        "unhashable type: `A`",
        help,
    );
    let inherited =
        format!("class A:\n{INIT}\n{EQ}\n\nclass B(A):\n    pass\n\n\nprint(hash(B()))\n");
    assert_one_error(
        "e2e_1335_eq_inherited",
        &inherited,
        "T0021",
        "unhashable type: `B`",
        help,
    );
    // The check phase delivers the verdict for an unannotated private
    // helper too, which the constraint path types first.
    let helper = format!(
        "class A:\n{INIT}\n{EQ}\n\ndef _h():\n    a = A()\n    return hash(a)\n\n\nprint(_h())\n"
    );
    assert_one_error(
        "e2e_1335_eq_helper",
        &helper,
        "T0021",
        "unhashable type: `A`",
        help,
    );
    // A subclass that rebinds only `__eq__` is unhashable too, so it agrees
    // with its base rather than refusing it as a differing subclass.
    let eq_twice = format!("class A:\n{INIT}\n{EQ}\n\nclass B(A):\n{EQ}\n\nprint(hash(A()))\n");
    assert_one_error(
        "e2e_1335_eq_twice",
        &eq_twice,
        "T0021",
        "unhashable type: `A`",
        help,
    );
}

#[test]
fn a_hash_method_returning_a_non_integer_is_t0021() {
    assert_one_error(
        "e2e_1335_ret_str",
        &class_r(
            "    def __hash__(self) -> str:\n        return \"s\"\n",
            "print(hash(R()))\n",
        ),
        "T0021",
        "`__hash__` method should return an integer",
        "`R.__hash__` returns `str`",
    );
}

#[test]
fn an_unannotated_hash_method_uses_its_inferred_return_type() {
    // `__hash__` is private-named, so HIR accepts it without `->`; the
    // solver's inferred return type drives the verdict, never a placeholder.
    assert_native_matches_cpython(
        "e2e_1335_unannotated_int",
        &class_r(
            "    def __hash__(self):\n        return 3\n",
            "print(hash(R()))\n",
        ),
        "3\n",
    );
    assert_one_error(
        "e2e_1335_unannotated_str",
        &class_r(
            "    def __hash__(self):\n        return \"s\"\n",
            "print(hash(R()))\n",
        ),
        "T0021",
        "`__hash__` method should return an integer",
        "`R.__hash__` returns `str`",
    );
}

#[test]
fn a_hash_method_returning_an_optional_integer_is_c0001() {
    // CPython raises only when such a method returns `None` at run time,
    // so the declared return type alone is not a static `TypeError`.
    for (tag, ty) in [
        ("e2e_1335_optional_int", "int"),
        ("e2e_1335_optional_bool", "bool"),
    ] {
        assert_one_error(
            tag,
            &class_r(
                &format!(
                    "    def __hash__(self) -> {ty} | None:\n        v: {ty} | None = None\n        return v\n"
                ),
                "print(hash(R()))\n",
            ),
            "C0001",
            "`hash()` of `R` is valid Python but not implemented yet",
            &format!("`R.__hash__` returns `{ty} | None`"),
        );
    }
}

#[test]
fn an_uncompiled_hash_binding_is_c0001() {
    let needle = "`hash()` of `R` is valid Python but not implemented yet";
    for (tag, body, help) in [
        (
            "e2e_1335_extra_param",
            "    def __hash__(self, y: int) -> int:\n        return y\n",
            "takes parameters besides `self`",
        ),
        (
            "e2e_1335_property",
            "    @property\n    def __hash__(self) -> int:\n        return 1\n",
            "plain `def __hash__(self)`",
        ),
        (
            "e2e_1335_static",
            "    @staticmethod\n    def __hash__() -> int:\n        return 1\n",
            "plain `def __hash__(self)`",
        ),
        (
            "e2e_1335_attr",
            "    __hash__ = 1\n",
            "plain `def __hash__(self)`",
        ),
    ] {
        assert_one_error(
            tag,
            &class_r(body, "print(hash(R()))\n"),
            "C0001",
            needle,
            help,
        );
    }
}

#[test]
fn a_dataclass_without_its_own_hash_is_c0001() {
    for (tag, import, decorator) in [
        (
            "e2e_1335_dataclass",
            "from dataclasses import dataclass",
            "@dataclass",
        ),
        (
            "e2e_1335_dataclass_transform",
            "from typing import dataclass_transform",
            "@dataclass_transform()",
        ),
    ] {
        assert_one_error(
            tag,
            &format!("{import}\n\n\n{decorator}\nclass D:\n    x: int\n\n\nprint(hash(D(1)))\n"),
            "C0001",
            "`hash()` of `D` is valid Python but not implemented yet",
            "define `__hash__` explicitly",
        );
    }
}

#[test]
fn an_enum_exception_protocol_or_generic_value_is_c0001() {
    let cases = [
        (
            "e2e_1335_enum",
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nprint(hash(Color.RED))\n",
            "Color",
            "enum member",
        ),
        (
            "e2e_1335_builtin_exception",
            "try:\n    raise ValueError(\"x\")\nexcept ValueError as e:\n    print(hash(e))\n",
            "ValueError",
            "exception instance",
        ),
        (
            "e2e_1335_user_exception",
            "class E(ValueError):\n    def __init__(self, m: str) -> None:\n        self.m = m\n\n\n\
             e = E(\"x\")\nprint(hash(e))\n",
            "E",
            "exception instance",
        ),
        (
            "e2e_1335_generic",
            "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\n\
             print(hash(Box[int](3)))\n",
            "Box",
            "generic class",
        ),
        // A protocol-typed parameter is not an instance type: it keeps the
        // generic refusal every other unimplemented argument type gets.
        (
            "e2e_1335_protocol",
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def m(self) -> int: ...\n\n\n\
             def f(p: P) -> int:\n    return hash(p)\n",
            "P",
            "or a class instance",
        ),
    ];
    for (tag, source, class, help) in cases {
        assert_one_error(
            tag,
            source,
            "C0001",
            &format!("`hash()` of `{class}` is valid Python but not implemented yet"),
            help,
        );
    }
}

#[test]
fn a_base_whose_subclass_hashes_differently_is_c0001() {
    let source = class_r(
        "    def key(self) -> int:\n        return hash(self)\n",
        "class B(R):\n    def __hash__(self) -> int:\n        return 2\n\n\nprint(R().key())\n",
    );
    assert_one_error(
        "e2e_1335_subclass_differs",
        &source,
        "C0001",
        "`hash()` of `R` is valid Python but not implemented yet",
        "#1337",
    );
}
