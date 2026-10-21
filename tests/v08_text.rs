//! v0.8 accept 4: what a text-processing package (mistune, a Markdown
//! engine) needed from the compiler, each a general Python feature:
//!
//! * `str` crossing into an `--ext` module is not copied (the native string
//!   borrows the CPython `str`'s UTF-8 text) and crossing back shares the
//!   CPython `str` it came from, so a function returning its argument returns
//!   the same object, as CPython does; `replace`/`strip` leaving a string
//!   unchanged return it, and the searches (`find`, `count`, `in`, `split`,
//!   `replace`) run on bytes;
//! * `typing.Optional[X]` and `typing.Union[...]` are the unions `X | None`
//!   and `X | ...`, in strict and non-strict mode, and in non-strict mode a
//!   `typing` name pycc does not model (`Match`, `Pattern`,
//!   `MutableMapping`) is the CPython object it names;
//! * a string annotation naming a `TYPE_CHECKING`-only import, under
//!   `from __future__ import annotations`, is erased whole (quotes included);
//! * `for c in s` reads one character per trip instead of building the
//!   list of every character first;
//! * `typing.cast(T, x)` of a CPython object is that object, whatever `T`
//!   spells (non-strict mode: `cast(str, token["raw"])`,
//!   `cast(Callable[..., str], f)`);
//! * a private module-level function whose parameters are immutable values
//!   (`str`, `int`, ...) is an attribute of its `--ext` module, as a Python
//!   module's is.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) running the source.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&oracle.stderr)));
}

/// Builds `source` with `flags`, asserting it builds and that no function,
/// method or class runs in CPython; returns the report.
fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for unit in ["fallback: function", "fallback: method", "fallback: class"] {
        assert!(!report.contains(unit), "no function should run in CPython:\n{report}");
    }
    report
}

/// The fixture as a native program built with `flags`, against `python3`.
fn native(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    build(&source, &dir.join("app"), flags);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

/// The fixture as an `--ext` module built with `flags` and imported by
/// `python3` running `driver`, against the same driver importing its source.
fn ext(name: &str, flags: &[&str], driver: &str) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", driver])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// The identities CPython gives the `str` methods and a returned argument:
/// the same object, when nothing changed.
const STR_IDENTITY_DRIVER: &str = "import m\n\
for t in m.TEXTS:\n    \
    print(m.ident(t) is t, m.esc(t) is t, m.multi(t, 'q', 'z') is t, m.multi(t, '&', '&') == t)\n\
s = 'x' * 50 + '\\u00e9'\n\
print(m.ident(s) is s, m.cat(s, '') == s, m.split_text(s, 'x')[-3:])\n";

#[test]
fn str_ops_native_strict() {
    native("v08_str_ops", &[]);
}

#[test]
fn str_ops_native_non_strict() {
    native("v08_str_ops", &["--non-strict"]);
}

#[test]
fn str_ops_ext_strict() {
    ext("v08_str_ops", &[], "import m");
    ext("v08_str_ops", &[], STR_IDENTITY_DRIVER);
}

#[test]
fn str_ops_ext_non_strict() {
    ext("v08_str_ops", &["--non-strict"], "import m");
    ext("v08_str_ops", &["--non-strict"], STR_IDENTITY_DRIVER);
}

/// A `str` the host passes in and gets back, unchanged or replaced, many
/// times: the compiled module's live strings return to where they were and
/// the host's string keeps its reference count (the borrowed text holds one
/// reference, released with the native string).
#[test]
fn str_crossings_leak_nothing() {
    let dir = ScratchDir::new("v08_str_leaks").expect("scratch");
    let source = fixture("v08_str_ops", &dir, "m.py");
    // The functions only: the fixture's own run is the other tests'.
    // (A Windows checkout's fixture has `\r\n` line ends.)
    let functions = std::fs::read_to_string(&source)
        .unwrap()
        .replace("\r\n", "\n")
        .replace("\nmain()\n", "\n");
    assert!(!functions.contains("\nmain()"), "the fixture's own run should be removed");
    std::fs::write(&source, functions).unwrap();
    build(&source, &dir.join("m"), &["--ext"]);
    std::fs::remove_file(&source).expect("import the artifact");
    let driver = "import ctypes, sys\n\
import m\n\
lib = ctypes.CDLL(m.__file__)\n\
live = lib.pycc_rt_str_live_objects\n\
live.restype = ctypes.c_int64\n\
s = 'h\\u00e9llo & <w\\u00f6rld> ' * 8\n\
p = 'plain text, longer than the twenty-two inline bytes'\n\
def churn():\n    \
    for _ in range(2000):\n        \
        m.ident(s); m.esc(s); m.esc(p); m.cat(s, p); m.finds(s, '&'); m.strips(p)\n        \
        m.split_text(s, '&'); m.multi(p, 'x', 'y'); m.joined(s, p, ',')\n\
churn()\n\
before = (live(), sys.getrefcount(s), sys.getrefcount(p))\n\
churn()\n\
after = (live(), sys.getrefcount(s), sys.getrefcount(p))\n\
print('ok' if before == after else (before, after))\n";
    let run = python()
        .args(["-c", driver])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), "ok\n", "{}", text(&pycc_scratch::lf(&run.stderr)));
}

#[test]
fn typing_unions_native_strict() {
    native("v08_typing_unions", &[]);
}

#[test]
fn typing_unions_native_non_strict() {
    native("v08_typing_unions", &["--non-strict"]);
}

#[test]
fn typing_unions_ext_non_strict() {
    ext("v08_typing_unions", &["--non-strict"], "import m");
}

/// Non-strict mode: `typing` names pycc does not model are CPython objects,
/// and annotations naming them compile.
#[test]
fn unmodeled_typing_names_native_non_strict() {
    native("v08_typing_objects", &["--non-strict"]);
}

#[test]
fn unmodeled_typing_names_ext_non_strict() {
    ext("v08_typing_objects", &["--non-strict"], "import m");
}

/// Strict mode keeps refusing a `typing` name pycc does not model.
#[test]
fn unmodeled_typing_names_stay_refused_in_strict_mode() {
    let dir = ScratchDir::new("v08_typing_strict").expect("scratch");
    let source = dir.join("s.py");
    std::fs::write(&source, "from typing import Match\n\n\ndef f(x: int) -> int:\n    return x\n")
        .unwrap();
    let check = pycc().arg("check").arg(&source).output().expect("pycc should spawn");
    let stderr = text(&pycc_scratch::lf(&check.stderr)) + &text(&pycc_scratch::lf(&check.stdout));
    assert!(!check.status.success(), "{stderr}");
    assert!(
        stderr.contains("module `typing` has no importable symbol named `Match`"),
        "{stderr}"
    );
    let source = dir.join("o.py");
    std::fs::write(
        &source,
        "from typing import Optional\n\n\ndef f(x: Optional[int, str]) -> int:\n    return 1\n",
    )
    .unwrap();
    let check = pycc().arg("check").arg(&source).output().expect("pycc should spawn");
    let stderr = text(&pycc_scratch::lf(&check.stderr)) + &text(&pycc_scratch::lf(&check.stdout));
    assert!(!check.status.success(), "{stderr}");
    assert!(stderr.contains("`Optional[...]` takes exactly one type argument"), "{stderr}");
    let source = dir.join("u.py");
    std::fs::write(&source, "from typing import Union\n\n\ndef f(x: Union[int, str]) -> int:\n    return 1\n")
        .unwrap();
    let check = pycc().arg("check").arg(&source).output().expect("pycc should spawn");
    let stderr = text(&pycc_scratch::lf(&check.stderr)) + &text(&pycc_scratch::lf(&check.stdout));
    assert!(!check.status.success(), "{stderr}");
    assert!(stderr.contains("T0048"), "{stderr}");
}

/// A package module whose annotations name `TYPE_CHECKING`-only imports, as
/// strings, under `from __future__ import annotations`: the annotations are
/// erased and every function compiles; the package imports and runs.
#[test]
fn type_checking_string_annotations_ext_non_strict() {
    let compiled = ScratchDir::new("v08_tc_annotations").expect("scratch");
    let pkg = compiled.join("pkg");
    std::fs::create_dir(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(
        pkg.join("state.py"),
        "class State:\n    def __init__(self, src: str) -> None:\n        self.src = src\n        \
         self.pos = 0\n",
    )
    .unwrap();
    let source = fixture("v08_type_checking_annotations", &pkg, "use.py");
    build(
        &source,
        &pkg.join("use"),
        &["--ext", "--non-strict", "--foreign-relative-imports"],
    );
    let driver = "from pkg.use import advance, describe\nfrom pkg.state import State\n\
s = State('hello world')\nprint(advance(s, 3), advance(s, 4), describe(s, 'x'), s.pos)\n\
try:\n    advance(s, -100)\nexcept ValueError as e:\n    print('ValueError', e)\n";
    let run = |dir: &Path| {
        python()
            .args(["-c", driver])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    std::fs::remove_file(&source).expect("the artifact, not the source");
    let ours = run(&compiled);
    let plain = ScratchDir::new("v08_tc_annotations_src").expect("scratch");
    let plain_pkg = plain.join("pkg");
    std::fs::create_dir(&plain_pkg).unwrap();
    for file in ["__init__.py", "state.py"] {
        std::fs::copy(pkg.join(file), plain_pkg.join(file)).unwrap();
    }
    fixture("v08_type_checking_annotations", &plain_pkg, "use.py");
    let oracle = run(&plain);
    assert_matches(&ours, &oracle);
}

/// A private module-level function taking `str` and `int` is an attribute of
/// the `--ext --non-strict` module, so `from m import _helper` works as for
/// the source module.
#[test]
fn private_value_functions_are_module_attributes() {
    ext(
        "v08_private_functions",
        &["--non-strict"],
        "from m import _wrap, _repeat, public\nprint(_wrap('x', 2), _repeat('ab', 3), public('q'))\n\
import m\nprint(sorted(n for n in dir(m) if n.startswith('_') and not n.startswith('__')))\n",
    );
}

#[test]
fn str_loop_native_strict() {
    native("v08_str_loop", &[]);
}

#[test]
fn str_loop_native_non_strict() {
    native("v08_str_loop", &["--non-strict"]);
}

#[test]
fn str_loop_ext_strict() {
    ext("v08_str_loop", &[], "import m");
}

#[test]
fn str_loop_ext_non_strict() {
    ext("v08_str_loop", &["--non-strict"], "import m");
}

#[test]
fn cast_of_objects_native_non_strict() {
    native("v08_cast_objects", &["--non-strict"]);
}

#[test]
fn cast_of_objects_ext_non_strict() {
    ext("v08_cast_objects", &["--non-strict"], "import m");
    ext("v08_cast_method", &["--non-strict"], "import m");
}

/// Non-strict mode: `typing` forms a module also uses as values are their
/// CPython objects.
#[test]
fn typing_forms_used_as_values_native_non_strict() {
    native("v08_typing_values", &["--non-strict"]);
}

#[test]
fn typing_forms_used_as_values_ext_non_strict() {
    ext("v08_typing_values", &["--non-strict"], "import m");
}
