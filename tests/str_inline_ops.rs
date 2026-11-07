//! Native `str` indexing, comparison, length and counts, whose common cases
//! compiled code answers inline (`pycc_codegen::str_inline`), against
//! CPython 3.15.
//!
//! The native fixture runs in both build profiles; only `--release` inlines. The `--ext` arm passes
//! CPython strings into typed functions: such a `str` shares the CPython
//! object's UTF-8 text and code-point count (`pycc_rt_str_adopt_foreign`),
//! so ASCII text is indexed inline from the first call, without a copy or a
//! count. Every test is `#[ignore]`d: it needs the CPython oracle.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/str_inline_ops.py")
}

fn oracle() -> String {
    let out = python().arg(fixture()).output().expect("python3 should spawn");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout))
}

/// The program's output and its optimized LLVM IR.
fn native(release: bool) -> (String, String) {
    let dir = ScratchDir::new(&format!("str_inline_ops_{release}")).expect("scratch");
    let exe = dir.join("prog");
    let ir = dir.join("prog.ll");
    let mut build = Command::new(env!("CARGO_BIN_EXE_pycc"));
    build.arg("build").arg(fixture()).arg("-o").arg(&exe).env("PYCC_DUMP_LLVM", &ir);
    if release {
        build.arg("--release");
    }
    let built = build.output().expect("pycc should spawn");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let out = Command::new(&exe).output().expect("the program should spawn");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let ir = std::fs::read_to_string(&ir).expect("PYCC_DUMP_LLVM writes the IR");
    (pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout)), ir)
}

/// `--release` takes the inline paths: indexing reads the shared
/// one-character strings, and no `__pycc_inline` copy survives as a call.
#[test]
#[ignore = "requires a CPython 3.15 oracle (PYCC_PYTHON, default python3)"]
fn native_str_operations_match_cpython_in_release() {
    let (out, ir) = native(true);
    assert_eq!(out, oracle());
    assert!(ir.contains("@pycc_rt_ascii_chars"), "{ir}");
    assert!(!ir.contains("call ptr @\"__pycc_inline"), "{ir}");
}

/// A debug build keeps the runtime calls.
#[test]
#[ignore = "requires a CPython 3.15 oracle (PYCC_PYTHON, default python3)"]
fn native_str_operations_match_cpython_in_debug() {
    let (out, ir) = native(false);
    assert_eq!(out, oracle());
    assert!(!ir.contains("pycc_rt_ascii_chars"), "{ir}");
}

const MODULE: &str = "\
def count(s: str, ch: str) -> int:
    n = 0
    for i in range(len(s)):
        if s[i] == ch:
            n += 1
    return n


def last(s: str) -> str:
    return s[-1]


def at(s: str, i: int) -> str:
    return s[i]


def before(a: str, b: str) -> bool:
    return a < b


def size(s: str) -> int:
    return len(s)
";

const DRIVER: &str = "\
import sys
import m
texts = ['', 'o', 'hello, world', 'a text longer than the inline payload: foo',
         'h\\u00e9llo w\\u00f6rld \\u4e2d', 'x' * 23, 'naive ' * 50]
for s in texts:
    print(m.size(s), m.count(s, 'o'), m.count(s, '\\u00f6'))
    if s:
        print(repr(m.last(s)), repr(m.at(s, 0)), repr(m.at(s, -len(s))))
    for i in [len(s), -len(s) - 1, 10 ** 30]:
        try:
            m.at(s, i)
        except IndexError as e:
            print('IndexError', e)
for a, b in [('a', 'b'), ('b', 'a'), ('a', 'a'), ('\\u00e9', 'z'), ('', 'a'), ('ab', 'a')]:
    print(m.before(a, b))
";

/// The module built with `--ext` and the same source imported by CPython
/// print the same lines.
#[test]
#[ignore = "requires a CPython 3.15 with development headers on PATH"]
fn borrowed_cpython_strs_index_and_compare_as_in_cpython() {
    let compiled = ScratchDir::new("str_inline_ops_ext").expect("scratch");
    let source = ScratchDir::new("str_inline_ops_src").expect("scratch");
    std::fs::write(compiled.join("m.py"), MODULE).expect("write the module");
    std::fs::write(source.join("m.py"), MODULE).expect("write the module");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(compiled.join("m.py"))
        .arg("-o")
        .arg(compiled.join("m"))
        .args(["--ext", "--release"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    std::fs::remove_file(compiled.join("m.py")).expect("remove the source beside the module");
    let run = |dir: &Path| {
        let out = python()
            .arg("-c")
            .arg(DRIVER)
            .current_dir(dir)
            .output()
            .expect("python3 should spawn");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout))
    };
    let expected = run(&source);
    assert!(expected.contains("IndexError"), "{expected}");
    assert_eq!(run(&compiled), expected);
}
