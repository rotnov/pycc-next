//! End-to-end proof for `set[C]`/`frozenset[C]` of a hashable user class
//! (#1343, Part 1 of #1336).
//!
//! `docs/TYPE_SYSTEM.md`'s set section and D-255 are the contract. Every
//! program also runs under whatever `PYCC_PYTHON`/`python3` is available,
//! and its stdout must equal both CPython's and the pinned text; the
//! harness is `tests/issue_1335_hash_instance.rs`'s. Set iteration order is
//! not reproduced (D-123), so every observable is order-insensitive: a
//! length, a sum, a sum of distinct powers of two, or a call log whose
//! order CPython and pycc share. Call logs are only printed where the
//! oracle's probe sequence visits candidates in insertion order (D-255
//! records the `__eq__` call-count deviation). The `--ext` differentials
//! are `#[ignore]`d like their siblings and run under `--include-ignored`
//! in CI.

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

/// Builds `source` natively, runs it, and asserts its stdout equals
/// CPython's run of the same file and `expected`.
/// Identity-hashed elements; a user `__hash__`/`__eq__` with its call log
/// (one `__hash__` per insertion, identity before `__eq__`,
/// `stored.__eq__(new)`, equal values with different hashes both kept);
/// CPython's `BUILD_SET` split at 30 elements; a `bool` hash; `-1`
/// reduced to `-2`; a bigint hash; iteration, truthiness, helpers and a
/// module global; `frozenset` copies with no calls; inherited methods, a
/// dataclass with its own `__hash__`, and a hashed subclass of a dataclass
/// comparing through the synthesized `__eq__`; a raising `__hash__` and `__eq__`
/// caught at a literal and at `.add`.
const SUCCESS: &str = r#"from dataclasses import dataclass

log: list[int] = [0]


class Id:
    def __init__(self, v: int) -> None:
        self.v = v


class R:
    def __init__(self, v: int, h: int) -> None:
        log.append(100 + v)
        self.v = v
        self.h = h

    def __hash__(self) -> int:
        log.append(200 + self.v)
        return self.h

    def __eq__(self, other: R) -> bool:
        log.append(300 + self.v * 10 + other.v)
        return self.v == other.v


class B:
    def __init__(self, t: bool) -> None:
        self.t = t

    def __hash__(self) -> bool:
        return self.t

    def __eq__(self, other: B) -> bool:
        return self.t == other.t


class Big:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v << 70

    def __eq__(self, other: Big) -> bool:
        return self.v == other.v


class Base:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v % 3

    def __eq__(self, other: Base) -> bool:
        return self.v == other.v


class Leaf(Base):
    pass


@dataclass
class P:
    x: int

    def __hash__(self) -> int:
        return self.x


@dataclass
class DB:
    x: int


class DK(DB):
    def __hash__(self) -> int:
        return self.x


class Boom:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        if self.v < 0:
            raise ValueError("bad hash")
        return self.v

    def __eq__(self, other: Boom) -> bool:
        if self.v == 7:
            raise ValueError("bad eq")
        return self.v == other.v


def _fingerprint(s: set[R]) -> int:
    total = 0
    for r in s:
        total += 1 << r.v
    return total


def _grow(s: set[R], v: int) -> set[R]:
    s.add(R(v, 5))
    return s


G: set[Id] = {Id(1), Id(2)}


def _count_global() -> int:
    n = 0
    for x in G:
        n += x.v
    return n


def _shown() -> str:
    out = ""
    i = 1
    while i < len(log):
        out = f"{out} {log[i]}"
        i += 1
    return out


def _reset() -> None:
    while len(log) > 1:
        log.pop()


# 1. identity hash and eq
a = Id(1)
b = Id(2)
ids = {a, b, a}
print(len(ids))
ids.add(a)
print(len(ids))
ids.add(Id(1))
print(len(ids))

# 2. user __hash__ and __eq__; stored.__eq__(new) order
_reset()
s = {R(1, 5), R(2, 5), R(1, 5)}
print(len(s), _shown())
_reset()
d = {R(1, 5), R(1, 6)}
print(len(d), _shown())
_reset()
s.add(R(2, 5))
print(len(s), _shown())
print(_fingerprint(s))

# 3. 30 elements evaluate first, 31 interleave
_reset()
thirty = {R(0, 0), R(1, 1), R(2, 2), R(3, 3), R(4, 4), R(5, 5), R(6, 6), R(7, 7), R(8, 8), R(9, 9), R(10, 10), R(11, 11), R(12, 12), R(13, 13), R(14, 14), R(15, 15), R(16, 16), R(17, 17), R(18, 18), R(19, 19), R(20, 20), R(21, 21), R(22, 22), R(23, 23), R(24, 24), R(25, 25), R(26, 26), R(27, 27), R(28, 28), R(29, 29)}
print(len(thirty), log[1], log[30], log[31])
_reset()
thirty_one = {R(0, 0), R(1, 1), R(2, 2), R(3, 3), R(4, 4), R(5, 5), R(6, 6), R(7, 7), R(8, 8), R(9, 9), R(10, 10), R(11, 11), R(12, 12), R(13, 13), R(14, 14), R(15, 15), R(16, 16), R(17, 17), R(18, 18), R(19, 19), R(20, 20), R(21, 21), R(22, 22), R(23, 23), R(24, 24), R(25, 25), R(26, 26), R(27, 27), R(28, 28), R(29, 29), R(30, 30)}
print(len(thirty_one), log[1], log[2], log[3])

# 4. bool __hash__
bs = {B(True), B(False), B(True)}
print(len(bs))

# 5. -1 reduces to -2
_reset()
neg = {R(1, -1), R(1, -2)}
print(len(neg), _shown())

# 6. a bigint __hash__
bigs = {Big(1), Big(2), Big(1)}
bigs.add(Big(3))
print(len(bigs))

# 7. iteration, truthiness, helpers, a global
total = 0
for r in s:
    total += r.v
print(total)
if s:
    print("truthy")
print(len(_grow(s, 9)), _fingerprint(s))
print(_count_global())

# 8. frozenset copies without calls
_reset()
f = frozenset(s)
ff = frozenset(f)
n = 0
for r in ff:
    n += r.v
print(len(f), len(ff), n, len(log))

# 10. inherited methods; a dataclass with its own __hash__
leaves = {Leaf(1), Leaf(4), Leaf(1)}
print(len(leaves))
ps = {P(1), P(1), P(2)}
print(len(ps))
dks = {DK(1), DK(1), DK(2)}
print(len(dks))

# 11. a raising __hash__, in a literal and in .add
k = {Boom(1)}
try:
    k = {Boom(2), Boom(-1)}
except ValueError as e:
    print("literal", e, len(k))
try:
    k.add(Boom(-3))
except ValueError as e:
    print("add", e, len(k))

# 12. a raising __eq__
e7 = {Boom(7)}
try:
    e7.add(Boom(7 + 0 * len(e7)))
    print("identity or equal")
except ValueError as e:
    print("eq", e, len(e7))
"#;

const SUCCESS_STDOUT: &str = r#"2
2
3
2  101 102 101 201 202 312 201 311
2  101 101 201 201
2  102 202 312 322
6
30 100 129 200
31 100 200 101
2
1  101 101 201 201 311
3
3
truthy
3 518
3
3 3 12 1
2
2
2
literal bad hash 1
add bad hash 1
eq bad eq 1
"#;

/// A module with unannotated helpers, so every function takes the
/// constraint-solver path: a literal and `.add` of `set[C]`, `frozenset`
/// of a `set[C]` parameter, literal local, annotated local and module
/// global, an unannotated `frozenset(s)` helper resolved from its caller's
/// `set[R]`, and `set[int]` comprehension helpers still inferred as
/// `set[int]`.
const MIXED: &str = r#"class R:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: R) -> bool:
        return self.v == other.v


class Q:
    def __init__(self, v: int) -> None:
        self.v = v


def _g(x):
    return x + 1


def _f(s):
    return frozenset(s)


def _w(n):
    return {i * 2 for i in range(n)}


def _v(xs: list[int]):
    t = {x for x in xs}
    return t


def _v2(xs: list[int]):
    return {x for x in xs}


def _fz(xs: list[int]):
    return frozenset({x for x in xs})


S: set[Q] = {Q(1), Q(2)}


def _from_param(u: set[Q]) -> frozenset[Q]:
    return frozenset(u)


def _from_literal() -> frozenset[Q]:
    u = {Q(3), Q(4)}
    return frozenset(u)


def _from_annotated() -> frozenset[Q]:
    u: set[Q] = {Q(5)}
    return frozenset(u)


def _from_global() -> frozenset[Q]:
    return frozenset(S)


def _via_helper(x: set[R]) -> int:
    n = 0
    f = _f(x)
    for r in f:
        n += r.v
    return n


def _sum(f: frozenset[Q]) -> int:
    n = 0
    for q in f:
        n += q.v
    return n


rs = {R(1), R(2), R(1)}
rs.add(R(_g(2)))
print(len(rs), _via_helper(rs))
qs = {Q(1), Q(2)}
qs.add(Q(3))
print(len(qs), _g(1))
print(_sum(_from_param(qs)), _sum(_from_literal()), _sum(_from_annotated()), _sum(_from_global()))
print(len(_w(3)), len(_v([1, 1, 2])), len(_v2([3])), len(_fz([4, 4])))
"#;

const MIXED_STDOUT: &str = r#"3 6
3 2
6 7 5 3
3 2 1 1
"#;

/// An `__eq__` raising at `.add`, uncaught.
const UNCAUGHT: &str = r#"class E:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: E) -> bool:
        raise ValueError("no eq")


s = {E(1)}
print(len(s))
s.add(E(1))
print("unreachable")
"#;

/// Prefixes `source` with `from __future__ import annotations` (PEP 563,
/// D-229). `__eq__(self, other: R)` names its own class, which CPython only
/// evaluates lazily from 3.14 on; an older `python3` oracle (the fallback
/// when `PYCC_PYTHON` is unset, as on the `native-build-test` legs) would
/// otherwise raise `NameError` at class creation.
fn with_postponed_annotations(source: &str) -> String {
    format!("from __future__ import annotations\n{source}")
}

/// Builds `source` natively, runs it, and asserts its stdout equals
/// CPython's run of the same file and `expected`.
fn assert_native_matches_cpython(tag: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("a.py"), with_postponed_annotations(source))
        .expect("write the subject");
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
    if oracle_splits_build_set() {
        assert_eq!(stdout(&run), stdout(&oracle));
    }
    assert_eq!(stdout(&run), expected);
}

/// Whether the oracle evaluates a set literal of more than 30 elements
/// incrementally, as CPython 3.11+ does and pycc reproduces. An older
/// oracle (a runner's system `python3` 3.9) evaluates every element first,
/// so only the pinned 3.11+ text is compared there.
fn oracle_splits_build_set() -> bool {
    let probe = python()
        .args(["-c", "import sys; print(sys.version_info >= (3, 11))"])
        .output()
        .expect("python3 should spawn");
    stdout(&probe).trim() == "True"
}

/// Builds `source` as the extension module `module`, imports it in CPython,
/// and asserts the import's stdout equals `runpy.run_path` of the same
/// source and `expected`.
fn assert_ext_matches_cpython(tag: &str, module: &str, source: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("m.py"), with_postponed_annotations(source))
        .expect("write the subject");
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

/// `class {name}{bases}:` with `INIT` and then `body`.
fn class(name: &str, bases: &str, body: &str) -> String {
    format!("class {name}{bases}:\n{INIT}\n{body}\n\n")
}

const HASH_1: &str = "    def __hash__(self) -> int:\n        return 1\n";

/// A hashed class `A` whose `__eq__` has the signature `eq_sig`.
fn class_a_with_eq(eq_sig: &str, eq_body: &str) -> String {
    class(
        "A",
        "",
        &format!("{HASH_1}\n    def __eq__{eq_sig}:\n        return {eq_body}\n"),
    )
}

#[test]
fn set_of_instance_programs_match_cpython() {
    assert_native_matches_cpython("e2e_1343_success", SUCCESS, SUCCESS_STDOUT);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn set_of_instance_module_bodies_match_cpython_as_extensions() {
    assert_ext_matches_cpython(
        "e2e_1343_ext",
        "pycc_set_instance_mod",
        SUCCESS,
        SUCCESS_STDOUT,
    );
}

#[test]
fn set_of_instance_constraint_path_programs_match_cpython() {
    assert_native_matches_cpython("e2e_1343_mixed", MIXED, MIXED_STDOUT);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn set_of_instance_constraint_path_module_bodies_match_cpython_as_extensions() {
    assert_ext_matches_cpython(
        "e2e_1343_mixed_ext",
        "pycc_set_instance_mixed_mod",
        MIXED,
        MIXED_STDOUT,
    );
}

/// An inherited `__hash__`/`__eq__` runs the element class's receiver-exact
/// copy (D-254), so a leaf override of the key they call decides membership
/// in a set, a frozenset, and at `.add`.
const LEAF_OVERRIDE: &str = r#"class Base:
    def __init__(self, v: int) -> None:
        self.v = v

    def key(self) -> int:
        return self.v

    def __hash__(self) -> int:
        return self.key()

    def __eq__(self, other: Base) -> bool:
        return self.key() == other.key()


class Leaf(Base):
    def key(self) -> int:
        return self.v % 3


s: set[Leaf] = {Leaf(1), Leaf(4), Leaf(2)}
print(len(s))
s.add(Leaf(7))
s.add(Leaf(5))
print(len(s))
f: frozenset[Leaf] = frozenset(s)
print(len(f))
"#;

#[test]
fn an_inherited_hash_and_eq_honour_a_leaf_override_like_cpython() {
    assert_native_matches_cpython("e2e_1343_leaf", LEAF_OVERRIDE, "2\n2\n2\n");
}

/// An uncaught `__eq__` raise stops the program after the same output, with
/// the same exit status and final exception line, as CPython. A native
/// build prints only the frame that raised, an existing native convention.
#[test]
fn an_uncaught_eq_raise_reports_the_exception_like_cpython() {
    let dir = ScratchDir::new("e2e_1343_uncaught").expect("scratch");
    std::fs::write(dir.join("a.py"), with_postponed_annotations(UNCAUGHT))
        .expect("write the subject");
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
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(oracle.status.code(), Some(1), "{}", rendered(&oracle));
    assert_eq!(run.status.code(), Some(1), "{}", rendered(&run));
    assert_eq!(stdout(&run), stdout(&oracle));
    assert_eq!(stdout(&run), "1\n");
    let last_line = |output: &Output| {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .map(str::to_owned)
    };
    assert_eq!(last_line(&run), last_line(&oracle));
    assert_eq!(last_line(&run).as_deref(), Some("ValueError: no eq"));
}

#[test]
fn a_class_with_eq_but_no_hash_is_t0054_at_every_insertion_site() {
    let eq = "    def __eq__(self, other: E) -> bool:\n        return True\n";
    let e = class("E", "", eq);
    let message = "cannot use 'E' as a set element (unhashable type: 'E')";
    let help = "`E` defines `__eq__` without `__hash__`, so CPython sets `__hash__ = None`";
    assert_one_error(
        "e2e_1343_t0054_literal",
        &format!("{e}s = {{E()}}\n"),
        "T0054",
        message,
        help,
    );
    assert_one_error(
        "e2e_1343_t0054_add",
        &format!("{e}def f(s: set[E]) -> None:\n    s.add(E())\n"),
        "T0054",
        message,
        help,
    );
    assert_one_error(
        "e2e_1343_t0054_helper",
        &format!("{e}def _h():\n    s = {{E()}}\n    return len(s)\n\n\nprint(_h())\n"),
        "T0054",
        message,
        help,
    );
    // Inherited from a base: the help names the class that binds `__eq__`.
    let inherited = format!("{e}class P(E):\n    pass\n\n\ns = {{P()}}\n");
    assert_one_error(
        "e2e_1343_t0054_inherited",
        &inherited,
        "T0054",
        "cannot use 'P' as a set element (unhashable type: 'P')",
        "`E` defines `__eq__` without `__hash__`",
    );
}

#[test]
fn an_uncompiled_element_class_is_c0001() {
    let message = |class: &str| {
        format!("a set element of class `{class}` is valid Python but not implemented yet")
    };
    let dataclass =
        "from dataclasses import dataclass\n\n\n@dataclass\nclass D:\n    x: int\n\n\ns = {D(1)}\n";
    assert_one_error(
        "e2e_1343_dataclass",
        dataclass,
        "C0001",
        &message("D"),
        "`D` has no `__hash__` of its own",
    );
    let enumeration =
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\ns = {Color.RED}\n";
    assert_one_error(
        "e2e_1343_enum",
        enumeration,
        "C0001",
        &message("Color"),
        "enum member",
    );
    let exception = "class Err(Exception):\n    pass\n\n\ndef f(e: Err) -> int:\n    s = {e}\n    return len(s)\n";
    assert_one_error(
        "e2e_1343_exception",
        exception,
        "C0001",
        &message("Err"),
        "exception instance",
    );
    let differs = format!(
        "{}{}s = {{A()}}\n",
        class("A", "", HASH_1),
        class(
            "B",
            "(A)",
            "    def __hash__(self) -> int:\n        return 2\n"
        )
    );
    assert_one_error(
        "e2e_1343_subclass_differs",
        &differs,
        "C0001",
        &message("A"),
        "subclass `B` hashes differently",
    );
    let subclassed = format!(
        "{}class B(A):\n    pass\n\n\ns = {{A()}}\n",
        class_a_with_eq("(self, other: A) -> bool", "True")
    );
    assert_one_error(
        "e2e_1343_subclassed",
        &subclassed,
        "C0001",
        &message("A"),
        "subclass `B` derives from the element class",
    );
    let property = format!(
        "{}s = {{A()}}\n",
        class(
            "A",
            "",
            &format!(
                "{HASH_1}\n    @property\n    def __eq__(self) -> bool:\n        return True\n"
            )
        )
    );
    assert_one_error(
        "e2e_1343_eq_property",
        &property,
        "C0001",
        &message("A"),
        "binds `__eq__` as something other than a plain",
    );
    let signature = "compiles it only as `def __eq__(self, other: K) -> bool`";
    let unrelated = "class U:\n    def __init__(self) -> None:\n        self.y = 1\n\n\n";
    for (tag, prefix, eq_sig, body) in [
        ("extra", "", "(self, other: A, extra: int) -> bool", "True"),
        ("int", "", "(self, other: A) -> int", "1"),
        ("unrelated", unrelated, "(self, other: U) -> bool", "True"),
    ] {
        let source = format!("{prefix}{}s = {{A()}}\n", class_a_with_eq(eq_sig, body));
        assert_one_error(
            &format!("e2e_1343_eq_{tag}"),
            &source,
            "C0001",
            &message("A"),
            signature,
        );
    }
    let hash_param = format!(
        "{}s = {{A()}}\n",
        class(
            "A",
            "",
            "    def __hash__(self, k: int) -> int:\n        return k\n"
        )
    );
    assert_one_error(
        "e2e_1343_hash_param",
        &hash_param,
        "C0001",
        &message("A"),
        "takes parameters besides `self`",
    );
    let hash_optional = format!(
        "{}s = {{A()}}\n",
        class(
            "A",
            "",
            "    def __hash__(self) -> int | None:\n        v: int | None = None\n        return v\n"
        )
    );
    assert_one_error(
        "e2e_1343_hash_optional",
        &hash_optional,
        "C0001",
        &message("A"),
        "returns `int | None`",
    );
}

#[test]
fn a_hash_method_returning_a_non_integer_is_t0021() {
    let source = format!(
        "{}s = {{A()}}\n",
        class(
            "A",
            "",
            "    def __hash__(self) -> str:\n        return 'a'\n"
        )
    );
    assert_one_error(
        "e2e_1343_hash_str",
        &source,
        "T0021",
        "`__hash__` method should return an integer",
        "returns `str`",
    );
}

#[test]
fn an_uncompiled_set_element_type_is_still_refused() {
    let r = class("R", "", "");
    let widened = "only set[int] and a set of a user-class instance are";
    assert_one_error(
        "e2e_1343_t0038_str",
        "def f(s: set[str]) -> None:\n    pass\n",
        "T0038",
        "set[str] is not compiled yet (D-122)",
        "",
    );
    assert_one_error(
        "e2e_1343_t0038_nested",
        &format!("{r}def f(s: set[set[R]]) -> None:\n    pass\n"),
        "T0038",
        &format!("set[set[R]] is not compiled yet (D-122) -- {widened}"),
        "",
    );
    // An optional instance is refused by the `Optional` gate first.
    assert_one_error(
        "e2e_1343_optional_element",
        &format!("{r}def f(s: set[R | None]) -> None:\n    pass\n"),
        "T0049",
        "`Optional[R]` is not supported yet",
        "",
    );
    assert_one_error(
        "e2e_1343_hash_frozenset",
        &format!("{r}def f(s: frozenset[R]) -> int:\n    return hash(s)\n"),
        "C0001",
        "`hash()` of `frozenset[R]` is valid Python but not implemented yet",
        "",
    );
}

const COMP_R: &str = "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n\n";

/// A set comprehension of instances is admitted since #1344
/// (`tests/issue_1344_set_comprehensions.rs` carries its differentials).
/// Since #1342 an unannotated helper returning one from a class-constructor
/// element infers `set[R]` as well, so the program runs like CPython.
#[test]
fn an_unannotated_helper_returning_a_set_comprehension_of_instances_matches_cpython() {
    for (kind, body) in [
        ("expr", "def _h():\n    return {R(i) for i in range(3)}\n"),
        (
            "stmt",
            "def _h():\n    t = {R(i) for i in range(3)}\n    return t\n",
        ),
    ] {
        assert_native_matches_cpython(
            &format!("e2e_1343_comp_unannotated_{kind}"),
            &format!("{COMP_R}{body}\n\nprint(len(_h()))\n"),
            "3\n",
        );
    }
}

#[test]
fn a_set_comprehension_element_error_in_an_unannotated_helper_is_reported() {
    // The solver collects a set comprehension's element before it builds
    // the container term; an element that fails collection must surface
    // its own diagnostic rather than a container one.
    assert_one_error(
        "e2e_1343_set_comp_element_error",
        "import math\ndef _f(n):\n    return {math.sqrt for x in range(n)}\n\nprint(len(_f(3)))\n",
        "T0021",
        "`math.sqrt` is a stdlib function and must be called",
        "call it: `math.sqrt(...)`",
    );
}
