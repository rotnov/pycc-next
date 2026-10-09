//! #1411: `type(self)(...)` inside an instance method constructs an instance
//! of the receiver's exact class, as CPython does -- including a subclass
//! receiver running an inherited body (`docs/TYPE_SYSTEM.md`, "Classes").
//!
//! pycc dispatches statically (D-006), so the class is the one `self` is
//! typed as in the body being compiled; an inherited body that constructs
//! through its receiver is compiled again for each subclass with `self`
//! retyped (D-254), so each subclass constructs itself. Where such a copy
//! cannot be typed -- the constructed subclass instance escapes into a slot
//! typed as the base, or the subclass's constructor takes other arguments
//! -- the program is refused and the diagnostic names the subclass.
//!
//! The native tests run the compiled program through `pycc run` and pin the
//! stdout CPython prints for the same source. The hosted test is `#[ignore]`d
//! and contributes no line coverage; the Tier-1 `native-build-test` leg runs
//! it with `cargo test --workspace -- --include-ignored`, comparing the
//! extension artifact against CPython's own run of the same source.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `body` to `dir/<file>`.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// `body` compiles, runs to completion and prints exactly `expected`.
fn assert_prints(tag: &str, body: &str, expected: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("run")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{tag}: {}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), expected, "{tag}");
}

/// `pycc check --error-format json` on `body` fails with exit `1`; returns
/// the rendered diagnostics.
fn check_fails(tag: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    rendered
}

const HEAD: &str = "from __future__ import annotations\n\
    class A:\n    def __init__(self, n: int) -> None:\n        self.n = n\n";

/// Each receiver constructs its own class through the one inherited body:
/// the base constructs a base, a subclass constructs the subclass (whose
/// override then runs), and a subclass with its own `__init__` and an extra
/// slot runs that constructor.
#[test]
fn an_inherited_body_constructs_each_receivers_own_class() {
    assert_prints(
        "1411_dispatch",
        "class Base:\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def kind(self) -> str:\n        return \"base\"\n\
         \x20   def bump(self) -> str:\n\
         \x20       c = type(self)(self.n + 1)\n        print(c.n)\n        return c.kind()\n\
         class Sub(Base):\n\
         \x20   def kind(self) -> str:\n        return \"sub\"\n\
         class Sub2(Sub):\n\
         \x20   def __init__(self, n: int) -> None:\n        self.n = n\n        self.tag = \"two\"\n\
         \x20   def kind(self) -> str:\n        return self.tag\n\
         print(Base(1).bump())\nprint(Sub(2).bump())\nprint(Sub2(3).bump())\n",
        "2\nbase\n3\nsub\n4\ntwo\n",
    );
}

/// The construction returns the receiver's class, so a `-> A` method of a
/// class without subclasses returns it; a local spelled like the class does
/// not capture the call; the call may sit in a comprehension, in a
/// conditional, and in its own arguments.
#[test]
fn the_receivers_class_is_constructed_wherever_the_call_sits() {
    assert_prints(
        "1411_positions",
        &format!(
            "{HEAD}\
             \x20   def clone(self) -> A:\n        return type(self)(self.n + 1)\n\
             \x20   def shadow(self) -> int:\n        A = 5\n        return type(self)(A).n + A\n\
             \x20   def many(self) -> list[int]:\n\
             \x20       return [type(self)(i).n for i in range(self.n)]\n\
             \x20   def pick(self, b: bool) -> int:\n\
             \x20       return type(self)(1).n if b else type(self)(type(self)(2).n * 3).n\n\
             a = A(3)\n\
             print(a.clone().n, a.clone().clone().n, a.n)\n\
             print(a.shadow())\nns = a.many()\nprint(len(ns), ns[0], ns[2])\nprint(a.pick(True), a.pick(False))\n"
        ),
        "4 5 3\n10\n3 0 2\n1 6\n",
    );
}

/// The arguments are ordinary expressions: a walrus in an `if` condition, a
/// generic function call, a generic class instantiation and a call through
/// a protocol-typed parameter. A `@staticmethod` whose parameter named
/// `self` is typed as the class constructs that class.
#[test]
fn the_arguments_are_ordinary_expressions() {
    assert_prints(
        "1411_args",
        "from __future__ import annotations\nfrom typing import Protocol\n\
         def ident[T](x: T) -> T:\n    return x\n\
         class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\
         class HasN(Protocol):\n    def get(self) -> int: ...\n\
         class G:\n    def get(self) -> int:\n        return 7\n\
         def use(h: HasN) -> int:\n    return h.get()\n\
         class A:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def walrus(self) -> int:\n\
         \x20       if type(self)((k := self.n + 1)).n > 0:\n            return k\n        return 0\n\
         \x20   def generic(self) -> int:\n\
         \x20       return type(self)(ident(self.n)).n + type(self)(Box[int](self.n + 1).v).n\n\
         \x20   def proto(self) -> int:\n        return type(self)(use(G())).n\n\
         \x20   @staticmethod\n    def make(self: A) -> int:\n        return type(self)(self.n * 2).n\n\
         a = A(4)\nprint(a.walrus(), a.generic(), a.proto(), A.make(a))\n",
        "5 9 7 8\n",
    );
}

/// Every `type(...)(...)` but `type(self)(...)` in an instance method is
/// refused with a message naming the construct.
#[test]
fn every_other_type_call_shape_is_refused_by_name() {
    for (tag, body, message) in [
        (
            "1411_not_self",
            format!("{HEAD}    def m(self, o: A) -> A:\n        return type(o)(1)\n"),
            "calling `type(...)` is supported only as `type(self)(...)`",
        ),
        (
            "1411_module_level",
            format!("{HEAD}x = type(self)(1)\n"),
            "`type(self)(...)` is supported only inside a method of a class",
        ),
        (
            "1411_classmethod",
            format!(
                "{HEAD}    @classmethod\n    def make(cls, n: int) -> int:\n        \
                 return type(self)(n).n\nprint(A.make(3))\n"
            ),
            "`type(self)(...)` is supported only in an instance method",
        ),
        (
            "1411_staticmethod",
            format!(
                "{HEAD}    @staticmethod\n    def make(self: int) -> int:\n        \
                 return type(self)(self).n\nprint(A.make(3))\n"
            ),
            "`type(self)(...)` is supported only in an instance method",
        ),
        (
            "1411_module_rebind",
            format!(
                "{HEAD}    def m(self) -> A:\n        return type(self)(1)\n\
                 def type(x: int) -> int:\n    return x\n"
            ),
            "this program rebinds the name `type`",
        ),
        (
            "1411_local_rebind",
            format!(
                "{HEAD}    def m(self) -> int:\n        type = 3\n        return type(self)(1).n\n"
            ),
            "this program rebinds the name `type`",
        ),
    ] {
        let rendered = check_fails(tag, &body);
        assert!(rendered.contains("\"C0001\""), "{tag}: {rendered}");
        assert!(rendered.contains(message), "{tag}: {rendered}");
    }
}

/// A wrong argument is the ordinary constructor `T0021`. An omitted
/// defaulted constructor argument is not filled in (`docs/TYPE_SYSTEM.md`),
/// so it is refused as a short call although CPython accepts it.
#[test]
fn a_wrong_constructor_argument_is_refused() {
    let rendered = check_fails(
        "1411_wrong_arg",
        &format!("{HEAD}    def m(self) -> A:\n        return type(self)(\"x\")\n"),
    );
    assert!(rendered.contains("\"T0021\""), "{rendered}");
    assert!(
        rendered.contains("argument 1 of `A` expects `int`, got `str`"),
        "{rendered}"
    );
    let rendered = check_fails(
        "1411_default_arg",
        "class A:\n    def __init__(self, n: int, k: int = 0) -> None:\n        \
         self.n = n\n        self.k = k\n    def m(self) -> int:\n        \
         return type(self)(1).n\nprint(A(1).m())\n",
    );
    assert!(rendered.contains("\"T0021\""), "{rendered}");
    assert!(
        rendered.contains("`A` expects 2 argument(s), got 1"),
        "{rendered}"
    );
}

/// A generic function's body may not call another generic function, and a
/// method of a generic class is one: the call in `type(self)(...)`'s
/// arguments is found and refused like any other.
#[test]
fn a_generic_call_in_a_generic_method_is_refused() {
    let rendered = check_fails(
        "1411_generic_body",
        "def ident[T](x: T) -> T:\n    return x\n\
         class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\
         \x20   def m(self, x: T) -> T:\n        return type(self)(ident(x)).v\n",
    );
    assert!(rendered.contains("\"T0042\""), "{rendered}");
    assert!(
        rendered.contains("generic function `Box.m` calls generic function `ident`"),
        "{rendered}"
    );
}

/// Where the body compiled for a subclass cannot be typed, the program is
/// refused -- never compiled to construct the base -- and the diagnostic
/// names the subclass: the constructed subclass instance escapes into a
/// `-> Base` return, or the subclass's constructor takes other arguments.
/// An abstract base's own construction is refused as for `B(...)`.
#[test]
fn an_untypeable_subclass_construction_is_refused_and_names_the_subclass() {
    for (tag, body, code, note) in [
        (
            "1411_escape",
            format!(
                "{HEAD}    def clone(self) -> A:\n        return type(self)(self.n)\n\
                 class B(A):\n    pass\nprint(B(2).clone().n)\n"
            ),
            "\"T0022\"",
            "while compiling `A.clone` inherited by subclass `B`",
        ),
        (
            "1411_other_ctor",
            format!(
                "{HEAD}    def bump(self) -> int:\n        return type(self)(self.n + 1).n\n\
                 class B(A):\n    def __init__(self, n: int, k: int) -> None:\n        \
                 self.n = n\n        self.k = k\nprint(B(1, 2).bump())\n"
            ),
            "\"T0021\"",
            "while compiling `A.bump` inherited by subclass `B`",
        ),
        // The abstract base's own body is compiled too, and constructing
        // the abstract class there is refused, although CPython only runs
        // it for a concrete subclass (`docs/TYPE_SYSTEM.md`).
        (
            "1411_abstract_base",
            "from abc import ABC, abstractmethod\n\
             class B(ABC):\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
             \x20   @abstractmethod\n    def k(self) -> int:\n        pass\n\
             \x20   def again(self) -> int:\n        return type(self)(self.n + 1).k()\n\
             class C(B):\n    def k(self) -> int:\n        return self.n\n\
             print(C(1).again())\n"
                .to_string(),
            "\"C0001\"",
            "cannot instantiate abstract class `B`",
        ),
    ] {
        let rendered = check_fails(tag, &body);
        assert!(rendered.contains(code), "{tag}: {rendered}");
        assert!(rendered.contains(note), "{tag}: {rendered}");
    }
}

// ---------------------------------------------------------------------
// Hosted: the extension artifact against CPython's own run.
// ---------------------------------------------------------------------

/// The artifact's file name for `-o <dir>/m`: `.pyd` on Windows and
/// `.abi3.so` elsewhere (`docs/CLI_SPEC.md`).
fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

/// Runs `script` with `dir` as the working directory.
fn host_run(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

/// The lark shape (`lark/parsers/lalr_parser_state.py`, MIT, the #1207
/// subject's line 60): `ParserState.copy` constructs `type(self)(...)` from
/// its slots, copying the state stack and deep- or shallow-copying the value
/// stack. The host passes the stacks in and the module reports identity and
/// equality of each copy's slots. `from __future__ import annotations`
/// keeps the `-> ParserState` annotation lazy for CPython 3.13, which
/// evaluates annotations eagerly otherwise.
const LARK: &str = "from __future__ import annotations\n\
    from copy import copy, deepcopy\n\
    from typing import Any, Generic, TypeVar\n\
    StateT = TypeVar('StateT')\n\
    class ParseConf(Generic[StateT]):\n\
    \x20   def __init__(self, start: str) -> None:\n        self.start = start\n\
    class ParserState(Generic[StateT]):\n\
    \x20   __slots__ = 'parse_conf', 'lexer', 'state_stack', 'value_stack'\n\
    \x20   parse_conf: ParseConf[StateT]\n    lexer: Any\n    state_stack: list\n    value_stack: list\n\
    \x20   def __init__(self, parse_conf: ParseConf[StateT], lexer: Any, state_stack: list, \
    value_stack: list) -> None:\n\
    \x20       self.parse_conf = parse_conf\n        self.lexer = lexer\n\
    \x20       self.state_stack = state_stack\n        self.value_stack = value_stack\n\
    \x20   def copy(self, deepcopy_values: bool = True) -> ParserState:\n\
    \x20       return type(self)(\n\
    \x20           self.parse_conf,\n            self.lexer,\n\
    \x20           copy(self.state_stack),\n\
    \x20           deepcopy(self.value_stack) if deepcopy_values else copy(self.value_stack),\n\
    \x20       )\n\
    def _show(c: ParserState, s: ParserState, lexer: Any, ss: list, vs: list) -> None:\n\
    \x20   print(c.parse_conf.start, c.lexer is lexer, c.lexer is s.lexer)\n\
    \x20   print(c.state_stack is ss, c.state_stack == ss)\n\
    \x20   print(c.value_stack is vs, c.value_stack == vs, c.value_stack[0] is vs[0])\n\
    def run(lexer: Any, ss: list, vs: list) -> None:\n\
    \x20   s = ParserState(ParseConf('start'), lexer, ss, vs)\n\
    \x20   deep = s.copy(True)\n    shallow = s.copy(False)\n\
    \x20   _show(deep, s, lexer, ss, vs)\n    _show(shallow, s, lexer, ss, vs)\n\
    \x20   print(deep.value_stack, shallow.value_stack, deep.state_stack)\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_parser_state_copy_matches_cpython_in_an_extension() {
    let compiled_dir = ScratchDir::new("1411_hosted_lark").expect("scratch");
    let source_dir = ScratchDir::new("1411_hosted_lark_src").expect("scratch");
    let source = write(&source_dir, "m.py", LARK);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
    let script = "import m\nm.run(object(), [1, 2], [[3], 4])\n";
    let compiled = host_run(&compiled_dir, script);
    let oracle = host_run(&source_dir, script);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    // `copy(True)` deep-copies the value stack (a fresh inner list) and
    // `copy(False)` shares it; the outer lists are always fresh.
    assert_eq!(
        stdout_of(&compiled),
        "start True True\nFalse True\nFalse True False\n\
         start True True\nFalse True\nFalse True True\n\
         [[3], 4] [[3], 4] [1, 2]\n"
    );
}
