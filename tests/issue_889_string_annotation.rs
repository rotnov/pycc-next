//! Part 1 of #889: a string-literal type annotation resolves exactly like
//! its unquoted spelling (`docs/TYPE_SYSTEM.md`, "Annotation semantics").
//! `pycc_parser::parse_all` unquotes each top-level string annotation, and
//! `annotation_to_ty` parses a string nested inside one (`list["int"]`).
//!
//! The native tests run the compiled program through `pycc run`, or pin
//! `pycc check`'s verdict, comparing the quoted spelling with the unquoted
//! one where both are valid. The hosted test is `#[ignore]`d and
//! contributes no line coverage; the Tier-1 `native-build-test` leg runs
//! it with `cargo test --workspace -- --include-ignored`, comparing the
//! extension artifact against CPython's own run of the same source. Every
//! program that runs (the native run and the hosted fixture) is valid under
//! CPython 3.13 (CI's hosted floor), which evaluates an unquoted annotation
//! eagerly; that is why the self-referential annotations there are quoted.
//! The unquoted twins fed only to `pycc check` need not be.

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

/// Writes `body` to `dir/<file>`, creating its parent directories.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
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

/// `pycc check` on `body` fails with exit `1`; returns the first rendered
/// line, `error[CODE]: message`, which carries no span.
fn first_error(tag: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, "m.py", body))
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    rendered.lines().next().unwrap_or_default().to_string()
}

/// Quoted annotations on parameters, returns, a local, a nested container
/// argument and a subscripted generic of the enclosing class.
#[test]
fn quoted_annotations_compile_and_run() {
    assert_prints(
        "889_run",
        "class Node:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def me(self) -> \"Node\":\n        return self\n\
         \x20   def plus(self, o: 'Node') -> \"Node\":\n        return Node(self.n + o.n)\n\
         class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\
         \x20   def same(self) -> 'Box[T]':\n        return self\n\
         def total(xs: list[\"int\"]) -> \"int\":\n    t: \"int\" = 0\n\
         \x20   for x in xs:\n        t += x\n    return t\n\
         def main() -> None:\n    a = Node(2)\n    b: \"Node\" = a.plus(Node(3)).me()\n\
         \x20   print(b.n)\n    print(total([1, 2, 3]))\n    print(Box[int](7).same().v)\n\
         main()\n",
        "5\n6\n7\n",
    );
}

/// A quoted `Final` keeps its `Final`: reassigning it is refused exactly as
/// the unquoted spelling is.
#[test]
fn reassigning_a_quoted_final_is_refused_like_the_unquoted_one() {
    let quoted = first_error("889_final_quoted", "x: \"Final[int]\" = 1\nx = 2\n");
    let unquoted = first_error("889_final_unquoted", "x: Final[int] = 1\nx = 2\n");
    assert!(quoted.starts_with("error[T0045]"), "{quoted}");
    assert_eq!(quoted, unquoted);
}

/// A string that is not a Python expression is `C0001`, naming its text.
#[test]
fn an_unparsable_string_annotation_is_c0001() {
    let line = first_error(
        "889_unparsable",
        "def f(x: \"not a type\") -> int:\n    return 1\n",
    );
    assert!(
        line.starts_with(
            "error[C0001]: string type annotation `not a type` is not a valid Python expression: "
        ),
        "{line}"
    );
}

/// A quoted name gains nothing over the unquoted one: a `TYPE_CHECKING`-only
/// import and a class defined later in the module are refused with the
/// unquoted spelling's own message. Resolving a later class is deferred on
/// #889.
#[test]
fn a_quoted_name_is_refused_exactly_where_the_unquoted_one_is() {
    for (tag, quoted, unquoted) in [
        (
            "889_type_checking",
            "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from fractions import Fraction\n\
             def f(a: \"Fraction\") -> int:\n    return 1\n",
            "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from fractions import Fraction\n\
             def f(a: Fraction) -> int:\n    return 1\n",
        ),
        (
            "889_later_class",
            "def f(a: \"Later\") -> int:\n    return 1\n\
             class Later:\n    def __init__(self) -> None:\n        self.n = 1\n",
            "def f(a: Later) -> int:\n    return 1\n\
             class Later:\n    def __init__(self) -> None:\n        self.n = 1\n",
        ),
    ] {
        let quoted_line = first_error(&format!("{tag}_q"), quoted);
        assert!(
            quoted_line.starts_with("error[C0001]"),
            "{tag}: {quoted_line}"
        );
        assert_eq!(
            quoted_line,
            first_error(&format!("{tag}_u"), unquoted),
            "{tag}"
        );
    }
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
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

/// The lark shape (`lark/parsers/lalr_parser_state.py`, MIT): a generic
/// `ParserState` whose `copy` is annotated `-> 'ParserState[StateT]'`,
/// with quoted slot and parameter annotations, plus a quoted
/// foreign-imported class, which resolves to `object` exactly as its
/// unquoted spelling does (#1367).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_lark_copy_shape_matches_cpython_in_an_extension() {
    let body = "from fractions import Fraction\nfrom typing import Generic, TypeVar\n\
                StateT = TypeVar('StateT')\n\
                class ParseConf(Generic[StateT]):\n    start: str\n    start_state: int\n\
                \x20   def __init__(self, start: str, start_state: int):\n\
                \x20       self.start = start\n        self.start_state = start_state\n\
                class ParserState(Generic[StateT]):\n    __slots__ = 'parse_conf', 'depth'\n\
                \x20   parse_conf: 'ParseConf[StateT]'\n    depth: \"int\"\n\
                \x20   def __init__(self, parse_conf: 'ParseConf[StateT]', depth: int):\n\
                \x20       self.parse_conf = parse_conf\n        self.depth = depth\n\
                \x20   def copy(self) -> 'ParserState[StateT]':\n\
                \x20       return ParserState(self.parse_conf, self.depth + 1)\n\
                \x20   def position(self) -> \"int\":\n\
                \x20       return self.parse_conf.start_state + self.depth\n\
                def half(f: \"Fraction\") -> \"Fraction\":\n\
                \x20   g: \"Fraction\" = f.limit_denominator(10)\n    return g\n\
                def build() -> None:\n    s = ParserState(ParseConf('start', 4), 2)\n\
                \x20   c = s.copy()\n    print(c.parse_conf.start, c.position(), s.position())\n\
                \x20   print(half(Fraction(3, 4)))\n";
    let compiled_dir = ScratchDir::new("889_hosted_lark").expect("scratch");
    let source_dir = ScratchDir::new("889_hosted_lark_src").expect("scratch");
    let source = write(&source_dir, "m.py", body);
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
    let script = "import m\nm.build()\n";
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
    assert_eq!(stdout_of(&compiled), "start 7 6\n3/4\n");
}
