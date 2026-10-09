//! End-to-end proof for comprehensions that produce or iterate a
//! `set[C]`/`frozenset[C]` of a hashable user class (#1344, Part 2 of
//! #1336).
//!
//! `docs/TYPE_SYSTEM.md`'s "Sets of user-class instances" section and
//! D-255's #1344 amendment are the contract. Every program also runs under
//! whatever `PYCC_PYTHON`/`python3` is available, and its stdout must equal
//! both CPython's and the pinned text; the harness is
//! `tests/issue_1336_set_of_instances.rs`'s. Set iteration order is not
//! reproduced (D-123), so every observable is order-insensitive: a length,
//! a sum, or a call log whose order CPython and pycc share (D-255 records
//! the `__eq__` call-count deviation). The `--ext` differential is
//! `#[ignore]`d like its siblings and runs under `--include-ignored` in CI.

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

fn last_stderr_line(output: &Output) -> Option<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .last()
        .map(|line| line.trim_end().to_owned())
}

/// Prefixes `source` with `from __future__ import annotations` (PEP 563,
/// D-229), so an older `python3` oracle does not evaluate
/// `__eq__(self, other: R)` at class creation.
fn with_postponed_annotations(source: &str) -> String {
    format!("from __future__ import annotations\n{source}")
}

/// Whether the oracle is at least CPython `major.minor`.
fn oracle_at_least(major: u32, minor: u32) -> bool {
    let probe = python()
        .args([
            "-c",
            &format!("import sys; print(sys.version_info >= ({major}, {minor}))"),
        ])
        .output()
        .expect("python3 should spawn");
    stdout(&probe).trim() == "True"
}

/// Writes `source` to a scratch directory and builds it natively as `app`.
fn build_native(tag: &str, source: &str) -> ScratchDir {
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
    dir
}

/// Runs the built `app` and the oracle on the same `a.py`.
fn run_both(dir: &ScratchDir) -> (Output, Output) {
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    let oracle = python()
        .arg("a.py")
        .current_dir(&**dir)
        .output()
        .expect("python3 should spawn");
    (run, oracle)
}

/// Builds `source` natively, runs it, and asserts its stdout equals the
/// pinned `expected` and, when the oracle is at least CPython `oracle_min`,
/// CPython's run of the same file.
fn assert_native_matches_cpython_from(
    tag: &str,
    source: &str,
    expected: &str,
    oracle_min: (u32, u32),
) {
    let dir = build_native(tag, source);
    let (run, oracle) = run_both(&dir);
    assert!(run.status.success(), "{}", rendered(&run));
    if oracle_at_least(oracle_min.0, oracle_min.1) {
        assert!(oracle.status.success(), "{}", rendered(&oracle));
        assert_eq!(stdout(&run), stdout(&oracle));
    }
    assert_eq!(stdout(&run), expected);
}

fn assert_native_matches_cpython(tag: &str, source: &str, expected: &str) {
    assert_native_matches_cpython_from(tag, source, expected, (3, 0));
}

/// An uncaught exception stops the built program after the same stdout,
/// with the same exit status (1) and final exception line, as CPython. Only
/// `pycc run`'s wrapper exits 101; the built binary exits 1.
fn assert_uncaught_matches_cpython(tag: &str, source: &str, expected: &str, last: &str) {
    let dir = build_native(tag, source);
    let (run, oracle) = run_both(&dir);
    assert_eq!(oracle.status.code(), Some(1), "{}", rendered(&oracle));
    assert_eq!(run.status.code(), Some(1), "{}", rendered(&run));
    assert_eq!(stdout(&run), stdout(&oracle));
    assert_eq!(stdout(&run), expected);
    assert_eq!(last_stderr_line(&run), last_stderr_line(&oracle));
    assert_eq!(last_stderr_line(&run).as_deref(), Some(last));
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
/// whose help contains `help`.
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

/// An unannotated private helper, which puts the whole module on the
/// constraint-solver path.
const SOLVER_HELPER: &str = "def _g(x):\n    return x + 1\n\n\n";

/// The lark `RulePtr` shape: `{rp.advance(sym) for rp in rps}` over an
/// annotated `set[RulePtr]`, with an `if` filter, over a
/// `frozenset[RulePtr]`, `{p for p in s}`, list and dict comprehensions
/// over the result, and a production from `range` deduplicated through the
/// user `__eq__`/`__hash__`.
const LARK: &str = r#"log: list[int] = [0]


class RulePtr:
    def __init__(self, rule: int, index: int) -> None:
        self.rule = rule
        self.index = index

    def advance(self, sym: int) -> RulePtr:
        return RulePtr(self.rule, self.index + sym)

    def __eq__(self, other: RulePtr) -> bool:
        log.append(1)
        return self.rule == other.rule and self.index == other.index

    def __hash__(self) -> int:
        return self.rule * 1000 + self.index


def step(rps: set[RulePtr], sym: int) -> set[RulePtr]:
    return {rp.advance(sym) for rp in rps}


def step_if(rps: frozenset[RulePtr], sym: int) -> set[RulePtr]:
    t = {rp.advance(sym) for rp in rps if rp.index > 0}
    return t


def total(s: set[RulePtr]) -> int:
    n = 0
    for p in s:
        n += p.rule * 10 + p.index
    return n


def main() -> None:
    s: set[RulePtr] = {RulePtr(1, 0), RulePtr(2, 1), RulePtr(1, 0), RulePtr(3, 2)}
    t = step(s, 1)
    print(len(t), total(t))
    u = step_if(frozenset(t), 2)
    print(len(u), total(u))
    idx = [p.index for p in u]
    print(len(idx))
    d = {"k": p.index for p in u}
    print(d["k"] >= 0)
    col = {RulePtr(i % 2, 0) for i in range(5)}
    print(len(col))
    same = {p for p in s}
    print(len(same), total(same))
    print(len(log) > 0)


main()
"#;

const LARK_STDOUT: &str = "3 66\n3 72\n3\nTrue\n2\n3 63\nTrue\n";

#[test]
fn the_lark_rule_pointer_shape_matches_cpython_on_both_paths() {
    assert_native_matches_cpython("e2e_1344_lark_check", LARK, LARK_STDOUT);
    assert_native_matches_cpython(
        "e2e_1344_lark_solver",
        &format!("{SOLVER_HELPER}{LARK}print(_g(1))\n"),
        &format!("{LARK_STDOUT}2\n"),
    );
}

/// The six iterable kinds the solver treats differently, times list, set,
/// filtered-set and dict comprehensions, in both the expression and the
/// statement form, plus a set of instances produced from `range` directly
/// and through an annotated factory. `R` hashes by identity. Returns the
/// program and its expected stdout.
fn source_matrix(solver: bool) -> (String, String) {
    let sources = [
        ("param", "s: set[R]", "    return {comp}\n", "{R(1), R(2)}"),
        (
            "param_stmt",
            "s: set[R]",
            "    t = {comp}\n    return t\n",
            "{R(1), R(2)}",
        ),
        (
            "literal",
            "",
            "    s = {R(1), R(2)}\n    return {comp}\n",
            "",
        ),
        (
            "annotated",
            "",
            "    s: set[R] = {R(1), R(2)}\n    return {comp}\n",
            "",
        ),
        ("global", "", "    s = S\n    return {comp}\n", ""),
        (
            "frozen",
            "s: frozenset[R]",
            "    return {comp}\n",
            "frozenset({R(1), R(2)})",
        ),
    ];
    let comps = [
        ("list", "list[int]", "[r.v for r in s]", "lsum", "3"),
        ("set", "set[R]", "{r for r in s}", "tot", "3"),
        ("set_if", "set[R]", "{r for r in s if r.v > 1}", "tot", "2"),
        (
            "dict",
            "dict[str, int]",
            "{\"k\": r.v for r in s}",
            "len",
            "1",
        ),
    ];
    let mut program = String::from(concat!(
        "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n\n",
        "def lsum(xs: list[int]) -> int:\n    n = 0\n    for x in xs:\n        n += x\n    return n\n\n\n",
        "def tot(s: set[R]) -> int:\n    n = 0\n    for r in s:\n        n += r.v\n    return n\n\n\n",
        "def mk(i: int) -> R:\n    return R(i)\n\n\n",
        "def produce() -> set[R]:\n    return {R(i) for i in range(3)}\n\n\n",
        "def produce_mk() -> set[R]:\n    return {mk(i) for i in range(3)}\n\n\n",
        "S: set[R] = {R(1), R(2)}\n\n\n",
    ));
    let mut calls = String::new();
    let mut expected = String::new();
    for (kind, param, body, arg) in sources {
        for (comp_kind, ret, comp, observe, value) in comps {
            let name = format!("{kind}_{comp_kind}");
            program.push_str(&format!(
                "def {name}({param}) -> {ret}:\n{}\n\n",
                body.replace("{comp}", comp)
            ));
            calls.push_str(&format!("print(\"{name}\", {observe}({name}({arg})))\n"));
            expected.push_str(&format!("{name} {value}\n"));
        }
    }
    calls.push_str("print(\"produce\", tot(produce()))\n");
    calls.push_str("print(\"produce_mk\", tot(produce_mk()))\n");
    expected.push_str("produce 3\nproduce_mk 3\n");
    if solver {
        program = format!("{SOLVER_HELPER}{program}");
        calls.push_str("print(_g(1))\n");
        expected.push_str("2\n");
    }
    (format!("{program}{calls}"), expected)
}

#[test]
fn every_comprehension_source_over_a_set_of_instances_matches_cpython() {
    for (path, solver) in [("check", false), ("solver", true)] {
        let (source, expected) = source_matrix(solver);
        assert_native_matches_cpython(&format!("e2e_1344_matrix_{path}"), &source, &expected);
    }
}

/// List and dict comprehensions over a `set[C]` follow their own element
/// rules: `list[int]` and `dict[str, int]` built from instance attributes.
const LIST_AND_DICT: &str = r#"class R:
    def __init__(self, v: int, name: str) -> None:
        self.v = v
        self.name = name

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: R) -> bool:
        return self.v == other.v


def main() -> None:
    rps: set[R] = {R(1, "a"), R(2, "b"), R(1, "c")}
    xs: list[int] = [rp.v for rp in rps]
    print(len(xs))
    d: dict[str, int] = {rp.name: rp.v for rp in rps}
    print(len(d), d["a"])


main()
"#;

#[test]
fn list_and_dict_comprehensions_over_a_set_of_instances_match_cpython() {
    assert_native_matches_cpython("e2e_1344_list_dict", LIST_AND_DICT, "2\n2 1\n");
    // An annotated return that disagrees with the comprehension keeps its
    // honest `T0022`; only an inferred return is relabelled `C0001`. Since
    // #1342 the solver types the constructor element too, so its own
    // declared-return wording is the one reported, exactly as for any other
    // annotated return it can type (`def f() -> int: return "x"`).
    assert_one_error(
        "e2e_1344_annotated_mismatch",
        "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n\ndef bad() -> set[int]:\n    return {R(i) for i in range(3)}\n",
        "T0022",
        "return type mismatch: expected `set[int]`, found `set[R]`",
        "return a `set[int]` value",
    );
}

/// Each insertion hashes once and compares identity before
/// `stored.__eq__(new)`, in the order CPython runs them.
const CALL_LOG: &str = r#"class K:
    def __init__(self, v: int) -> None:
        print("init", v)
        self.v = v

    def __hash__(self) -> int:
        print("hash", self.v)
        return self.v % 2

    def __eq__(self, other: K) -> bool:
        print("eq", self.v, other.v)
        return self.v == other.v


def main() -> None:
    s: set[K] = {K(i) for i in range(4)}
    print(len(s))
    t: set[K] = {k for k in s if k.v > 0}
    print(len(t))


main()
"#;

#[test]
fn a_set_comprehension_calls_init_hash_and_eq_in_cpython_order() {
    assert_native_matches_cpython(
        "e2e_1344_call_log",
        CALL_LOG,
        "init 0\nhash 0\ninit 1\nhash 1\ninit 2\nhash 2\neq 0 2\ninit 3\nhash 3\neq 1 3\n4\nhash 1\nhash 2\nhash 3\neq 1 3\n3\n",
    );
}

/// A raising `__hash__` and `__eq__` inside a comprehension, with a
/// constructor element, inside a loop, and caught by an enclosing `try`.
const RAISE_CONSTRUCTED: &str = r#"mode: list[int] = [0]


def set_mode(m: int) -> None:
    while len(mode) > 1:
        mode.pop()
    for _ in range(m):
        mode.append(0)


def cur() -> int:
    return len(mode) - 1


log: list[int] = [0]


class E(Exception):
    pass


class H:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        log.append(self.v)
        if cur() == 1 and self.v == 2:
            raise E("h")
        return self.v % 2

    def __eq__(self, other: H) -> bool:
        if cur() == 2:
            raise ValueError("eq")
        return self.v == other.v


def build(xs: set[H]) -> int:
    try:
        t = {H(x.v + 1) for x in xs}
        return len(t)
    except E:
        return -1
    except ValueError:
        return -2


def main() -> None:
    s = {H(1), H(3)}
    print(build(s))
    set_mode(1)
    print(build(s))
    set_mode(2)
    print(build(s))
    set_mode(1)
    try:
        u = {H(i) for i in range(4)}
        print(len(u))
    except E:
        print("E")
    set_mode(2)
    n = 0
    for i in range(3):
        try:
            print(len({H(j) for j in range(i + 1)}))
        except ValueError:
            n += 1
    print(n)
    set_mode(0)
    print(len(log))


main()
"#;

/// A bare-name element `{p for p in xs}` whose `__hash__`, then `__eq__`,
/// starts raising after the source set is built.
const RAISE_BARE_NAME: &str = r#"mode: list[int] = [0]


class P:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        if len(mode) == 2 and self.v == 2:
            raise RuntimeError("late hash")
        return self.v % 2

    def __eq__(self, other: P) -> bool:
        if len(mode) == 3:
            raise ValueError("late eq")
        return self.v == other.v


def copy(xs: set[P]) -> int:
    try:
        return len({p for p in xs})
    except RuntimeError as e:
        print("RuntimeError", e)
        return -1
    except ValueError as e:
        print("ValueError", e)
        return -2


def main() -> None:
    xs: set[P] = {P(1), P(2), P(3)}
    print(copy(xs))
    mode.append(1)
    print(copy(xs))
    mode.append(1)
    print(copy(xs))


main()
"#;

#[test]
fn a_raise_inside_a_set_comprehension_is_caught_like_cpython() {
    assert_native_matches_cpython(
        "e2e_1344_raise_constructed",
        RAISE_CONSTRUCTED,
        "2\n-1\n-2\nE\n1\n2\n1\n17\n",
    );
    assert_native_matches_cpython(
        "e2e_1344_raise_bare_name",
        RAISE_BARE_NAME,
        "3\nRuntimeError late hash\n-1\nValueError late eq\n-2\n",
    );
}

#[test]
fn an_uncaught_raise_inside_a_set_comprehension_reports_it_like_cpython() {
    let source = "class H:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def __hash__(self) -> int:\n        return 1\n\n    def __eq__(self, other: H) -> bool:\n        raise ValueError(\"boom\")\n\n\ndef main() -> None:\n    t = {H(i) for i in range(3)}\n    print(len(t))\n\n\nprint(\"start\")\nmain()\n";
    assert_uncaught_matches_cpython("e2e_1344_uncaught", source, "start\n", "ValueError: boom");
}

/// `SRC` grows while a comprehension iterates it: through `M.__hash__` for
/// a set comprehension, and through `f` for the `int` list and dict ones.
const GROWN_PRELUDE: &str = r#"SRC: set[int] = {1, 2, 3}


class M:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        SRC.add(self.v + 100)
        return self.v

    def __eq__(self, other: M) -> bool:
        return self.v == other.v


def f(i: int) -> int:
    SRC.add(i + 1000)
    return i


"#;

const GROWN_COMPS: [(&str, &str); 3] = [
    ("list", "[f(i) for i in SRC]"),
    ("set", "{M(i) for i in SRC}"),
    ("dict", "{\"k\": f(i) for i in SRC}"),
];

/// `(form, body)`: the comprehension `{c}` as an expression and as a
/// statement, each followed by a `BAD` sentinel the raise must skip.
const GROWN_FORMS: [(&str, &str); 2] = [
    ("expr", "    print(len({c}))\n    print(\"BAD\")\n"),
    ("stmt", "    t = {c}\n    print(\"BAD\", len(t))\n"),
];

/// A source set grown during a comprehension raises CPython's
/// `RuntimeError` instead of looping forever, for `set[int]` and `set[C]`
/// alike, caught inside a `try` and uncaught with a sentinel after it.
#[test]
fn a_source_set_grown_during_a_comprehension_raises_like_cpython() {
    let mut caught = String::from(GROWN_PRELUDE);
    let mut calls = String::new();
    let mut expected = String::new();
    for (kind, comp) in GROWN_COMPS {
        for (form, body) in GROWN_FORMS {
            let indented = body.replace("\n    ", "\n        ");
            caught.push_str(&format!(
                "def {kind}_{form}() -> None:\n    try:\n    {}    except RuntimeError as e:\n        print(\"RuntimeError\", e)\n\n\n",
                indented.replace("{c}", comp)
            ));
            calls.push_str(&format!("{kind}_{form}()\n"));
            expected.push_str("RuntimeError Set changed size during iteration\n");
            let uncaught = format!(
                "{GROWN_PRELUDE}def g() -> None:\n{}\n\nprint(\"start\")\ng()\nprint(\"BAD\")\n",
                body.replace("{c}", comp)
            );
            assert_uncaught_matches_cpython(
                &format!("e2e_1344_grown_uncaught_{kind}_{form}"),
                &uncaught,
                "start\n",
                "RuntimeError: Set changed size during iteration",
            );
        }
    }
    // Each comprehension adds exactly one element before it notices.
    expected.push_str("9\n");
    assert_native_matches_cpython(
        "e2e_1344_grown_caught",
        &format!("{caught}{calls}print(len(SRC))\n"),
        &expected,
    );
}

/// Comprehensions of instances inside a PEP 695 generic function, which
/// monomorphization rewrites per instantiation. A pre-3.12 oracle cannot
/// parse the syntax, so only the pinned text is compared there.
const PEP_695: &str = r#"class R:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: R) -> bool:
        return self.v == other.v


def count[T](x: T, n: int) -> int:
    s: set[R] = {R(i % 3) for i in range(n)}
    t: set[R] = {r for r in s if r.v > 0}
    total = 0
    for r in t:
        total += r.v
    return len(s) * 100 + total


print(count(1, 6))
print(count("a", 2))
"#;

#[test]
fn a_set_comprehension_in_a_generic_function_matches_cpython() {
    assert_native_matches_cpython_from("e2e_1344_pep695", PEP_695, "303\n201\n", (3, 12));
}

/// A non-generic function's set comprehensions in a module that also holds
/// an unrelated PEP 695 generic, so `monomorphize` rewrites the function's
/// body too: elements reading a module global, a module-level instance, an
/// attribute and a method call, in the statement and expression forms.
const GENERIC_NEIGHBOUR: &str = r#"class R:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v

    def __eq__(self, other: R) -> bool:
        return self.v == other.v

    def clone(self) -> R:
        return R(self.v)


G: R = R(7)
N: int = 3


def ident[T](x: T) -> T:
    return x


def vsum(s: set[R]) -> int:
    t = 0
    for r in s:
        t += r.v
    return t


def run() -> None:
    a = {G for i in range(3)}
    print(len(a), vsum(a))
    b: set[R] = {R(i + N) for i in range(3)}
    print(len(b), vsum(b))
    rs: set[R] = {R(1), R(2)}
    d = {r.clone() for r in rs}
    print(len(d), vsum(d))
    e = {R(r.v * 10) for r in rs}
    print(len(e), vsum(e))
    print(vsum({R(r.v) for r in rs}), ident(5))


run()
"#;

#[test]
fn a_set_comprehension_beside_a_generic_function_matches_cpython() {
    assert_native_matches_cpython_from(
        "e2e_1344_generic_neighbour",
        GENERIC_NEIGHBOUR,
        "1 7\n3 12\n2 3\n2 30\n3 5\n",
        (3, 12),
    );
}

/// #1105 (open): `monomorphize` seeds no foreign name, so once a module
/// holds a PEP 695 generic, `pycc build` refuses a foreign read in any
/// function with `T0021` even though `pycc check` accepts it. A set
/// comprehension's element fails exactly like a plain statement does: #1344
/// adds no failure of its own to that pass. This pins today's behaviour
/// and must flip to a CPython match when #1105 lands.
#[test]
fn a_foreign_read_beside_a_generic_function_fails_like_a_plain_statement() {
    let refusal = |tag: &str, body: &str| {
        let source = format!(
            "import fractions\n\n\ndef ident[T](x: T) -> T:\n    return x\n\n\n\
             def run() -> None:\n{body}\n\n\nrun()\n"
        );
        let dir = ScratchDir::new(tag).expect("scratch");
        std::fs::write(dir.join("a.py"), &source).expect("write the subject");
        let check = pycc()
            .arg("check")
            .arg(dir.join("a.py"))
            .output()
            .expect("pycc should spawn");
        assert!(check.status.success(), "{}", rendered(&check));
        let build = pycc()
            .arg("build")
            .arg(dir.join("a.py"))
            .arg("-o")
            .arg(dir.join("a.bin"))
            .output()
            .expect("pycc should spawn");
        assert_eq!(build.status.code(), Some(1), "{}", rendered(&build));
        rendered(&build)
            .lines()
            .next()
            .expect("a diagnostic")
            .to_string()
    };
    let comp = refusal(
        "e2e_1344_1105_comp",
        "    y = {i for i in range(int(fractions.Fraction(3, 1)))}\n    print(len(y))",
    );
    let plain = refusal(
        "e2e_1344_1105_plain",
        "    y = int(fractions.Fraction(3, 1))\n    print(y)",
    );
    assert_eq!(comp, "error[T0021]: name `fractions` is not defined");
    assert_eq!(comp, plain);
}

const HASHED_R: &str = "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def __hash__(self) -> int:\n        return self.v % 2\n\n    def __eq__(self, other: R) -> bool:\n        return self.v == other.v\n\n\n";

/// The solver types an unannotated helper's set comprehension from an
/// annotated factory's return, in the expression and statement forms and
/// flowing into an annotated `set[R]`; an annotated return is the
/// workaround for a constructor element.
#[test]
fn the_solver_derives_a_set_of_instances_from_a_typed_element() {
    let source = format!(
        "{HASHED_R}def mk(i: int) -> R:\n    return R(i)\n\n\n\
         def _h(n):\n    return {{mk(i) for i in range(n)}}\n\n\n\
         def _h2(n):\n    t = {{mk(i % 2) for i in range(n)}}\n    return t\n\n\n\
         def _h3(n) -> set[R]:\n    return {{R(i) for i in range(n)}}\n\n\n\
         print(len(_h(3)))\nprint(len(_h2(3)))\ns: set[R] = _h(4)\nprint(len(s))\nprint(len(_h3(5)))\n"
    );
    assert_native_matches_cpython("e2e_1344_solver_mk", &source, "3\n2\n4\n5\n");
}

/// The shape the solver cannot type yet: a set-typed name's element
/// (#1360). It is one honest `C0001`, never a `T0022` about a `set[int]`
/// nobody wrote. A class-constructor element is typed since #1342; see
/// `an_inferred_set_of_constructed_instances_matches_cpython`.
#[test]
fn an_untypable_inferred_set_return_is_c0001() {
    let r = "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def m(self) -> R:\n        return R(self.v + 1)\n\n\n";
    let global = "G: set[R] = {R(1)}\n\n\n";
    for (tag, body) in [
        ("global", "def _h(n):\n    return {p for p in G}\n"),
        (
            "global_method",
            "def _h(n):\n    return {p.m() for p in G}\n",
        ),
    ] {
        assert_one_error(
            &format!("e2e_1344_inferred_{tag}"),
            &format!("{r}{global}{body}\n\nprint(len(_h(3)))\n"),
            "C0001",
            "cannot infer an unannotated private helper's `set[R]` return yet",
            "#1360",
        );
    }
}

/// `inferred_set_return_limit`'s `frozenset` arm: `frozenset(...)` of an
/// untypable set comprehension is the same honest `C0001`, never a `T0022`.
#[test]
fn an_untypable_inferred_frozenset_return_is_c0001() {
    assert_one_error(
        "e2e_1344_inferred_frozenset_global",
        &format!(
            "{HASHED_R}G: set[R] = {{R(1)}}\n\n\n\
             def _h(n):\n    return frozenset({{p for p in G}})\n\n\nprint(len(_h(3)))\n"
        ),
        "C0001",
        "cannot infer an unannotated private helper's `frozenset[R]` return yet",
        "#1360",
    );
}

/// #1342: an unannotated helper's set comprehension (or `frozenset(...)`
/// of one) whose element is a class-constructor call infers `set[R]` /
/// `frozenset[R]`, in the expression and the statement forms.
#[test]
fn an_inferred_set_of_constructed_instances_matches_cpython() {
    let source = format!(
        "{HASHED_R}def _h(n):\n    return {{R(i) for i in range(n)}}\n\n\n\
         def _h2(n):\n    t = {{R(i) for i in range(n)}}\n    return t\n\n\n\
         def _f(n):\n    return frozenset({{R(i) for i in range(n)}})\n\n\n\
         def _f2(n):\n    t = {{R(i) for i in range(n)}}\n    return frozenset(t)\n\n\n\
         s: set[R] = _h(4)\nprint(len(s), len(_h2(3)), len(_f(5)), len(_f2(2)))\n"
    );
    assert_native_matches_cpython("e2e_1342_inferred_set", &source, "4 3 5 2\n");
}

/// A helper whose untypable `set[R]` local never escapes: it returns an
/// `int`, so no inferred `set[int]` return meets a `set[R]` and nothing is
/// refused. Iterated by a `for` loop and by a list comprehension.
#[test]
fn an_untypable_local_used_only_inside_its_helper_matches_cpython() {
    let source = format!(
        "{HASHED_R}def _h(n):\n    t = {{R(i) for i in range(n)}}\n    s = 0\n    for r in t:\n        s += r.v\n    return s\n\n\n\
         def _h2(n):\n    t = {{R(i) for i in range(n)}}\n    xs = [r.v for r in t]\n    s = 0\n    for x in xs:\n        s += x\n    return s + len(xs)\n\n\n\
         print(_h(4))\nprint(_h2(4))\n"
    );
    assert_native_matches_cpython("e2e_1344_internal_local", &source, "6\n10\n");
}

/// The annotated-return workaround the `C0001` help names: a set-typed
/// name's element (#1360), bare and through a method call. The
/// constructor-element `frozenset[R]` helpers keep their annotations here
/// although #1342 infers them now.
#[test]
fn the_annotated_return_workaround_compiles_every_refused_shape() {
    let source = format!(
        "{HASHED_R}    def m(self) -> R:\n        return R(self.v + 1)\n\n\n\
         G: set[R] = {{R(1), R(2), R(3)}}\n\n\n\
         def _f(n) -> frozenset[R]:\n    return frozenset({{R(i) for i in range(n)}})\n\n\n\
         def _f2(n) -> frozenset[R]:\n    t = {{R(i) for i in range(n)}}\n    return frozenset(t)\n\n\n\
         def _g(n) -> set[R]:\n    return {{p for p in G}}\n\n\n\
         def _g2(n) -> set[R]:\n    return {{p.m() for p in G}}\n\n\n\
         print(len(_f(3)), len(_f2(5)))\nb = _g2(1)\nprint(len(_g(1)), len(b))\n\
         s = 0\nfor p in b:\n    s += p.v\nprint(s)\n"
    );
    assert_native_matches_cpython("e2e_1344_workarounds", &source, "3 5\n3 3\n9\n");
}

/// The D-255 residual: an annotated caller of a helper that hits the
/// #1360 `C0001` also reports a `T0025` against the helper's still-`set[int]`
/// inferred signature. Inside a function the `C0001` comes first; a
/// module-scope caller's `T0025` is reported alone.
#[test]
fn an_annotated_caller_of_an_untypable_helper_reports_the_d255_residual() {
    let helper = format!(
        "{HASHED_R}G: set[R] = {{R(1)}}\n\n\ndef _h(n):\n    return {{p for p in G}}\n\n\n"
    );
    let codes = |tag: &str, source: &str| {
        let dir = ScratchDir::new(tag).expect("scratch");
        std::fs::write(dir.join("a.py"), with_postponed_annotations(source))
            .expect("write the subject");
        let output = pycc()
            .arg("check")
            .arg("--error-format")
            .arg("json")
            .arg(dir.join("a.py"))
            .output()
            .expect("pycc should spawn");
        assert_eq!(output.status.code(), Some(1), "{}", rendered(&output));
        rendered(&output)
            .lines()
            .map(|line| {
                let start = line.find("\"code\":\"").expect("a code field") + 8;
                line[start..start + 5].to_string()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        codes(
            "e2e_1344_residual_module",
            &format!("{helper}s: set[R] = _h(4)\nprint(len(s))\n"),
        ),
        ["T0025"]
    );
    assert_eq!(
        codes(
            "e2e_1344_residual_function",
            &format!(
                "{helper}def run() -> None:\n    s: set[R] = _h(4)\n    print(len(s))\n\n\nrun()\n"
            ),
        ),
        ["C0001", "T0025"]
    );
}

/// A comprehension is an insertion site like a literal, so
/// `check_set_element`'s refusals reach it unchanged.
#[test]
fn a_set_comprehension_keeps_the_set_element_refusals() {
    assert_one_error(
        "e2e_1344_t0054",
        "class E:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    def __eq__(self, other: E) -> bool:\n        return True\n\n\ns = {E(i) for i in range(3)}\n",
        "T0054",
        "cannot use 'E' as a set element (unhashable type: 'E')",
        "`E` defines `__eq__` without `__hash__`",
    );
    assert_one_error(
        "e2e_1344_dataclass",
        "from dataclasses import dataclass\n\n\n@dataclass\nclass D:\n    x: int\n\n\ns = {D(i) for i in range(3)}\n",
        "C0001",
        "a set element of class `D` is valid Python but not implemented yet",
        "`D` has no `__hash__` of its own",
    );
    let subclassed = format!("{HASHED_R}class B(R):\n    pass\n\n\n");
    assert_one_error(
        "e2e_1344_subclassed",
        &format!("{subclassed}s = {{R(i) for i in range(3)}}\nprint(len(s))\n"),
        "C0001",
        "a set element of class `R` is valid Python but not implemented yet",
        "subclass `B` derives from the element class",
    );
    // The leaf class itself has no subclass, so a set of it compiles.
    assert_native_matches_cpython(
        "e2e_1344_leaf_element",
        &format!("{subclassed}s = {{B(i % 3) for i in range(5)}}\nprint(len(s))\n"),
        "3\n",
    );
}

const MODULE_BODY: &str = r#"class R:
    def __init__(self, v: int) -> None:
        self.v = v

    def __hash__(self) -> int:
        return self.v % 3

    def __eq__(self, other: R) -> bool:
        return self.v == other.v


S: set[R] = {R(1), R(2), R(4)}
T = {R(p.v % 2) for p in S}
print(len(T))
L = [p.v for p in T]
print(len(L))
"#;

#[test]
fn a_module_scope_set_comprehension_matches_cpython() {
    assert_native_matches_cpython("e2e_1344_module", MODULE_BODY, "2\n2\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_module_scope_set_comprehension_matches_cpython_as_an_extension() {
    assert_ext_matches_cpython("e2e_1344_ext", "m1344", MODULE_BODY, "2\n2\n");
}
