//! Classes from libraries (v0.7 accept 2) in `--non-strict`: classes
//! inheriting CPython classes (`threading.Thread`, `collections.Counter`,
//! `argparse.Action`, a `namedtuple`, a builtin exception), a metaclass and
//! `__init_subclass__`, a descriptor with `__set_name__`, class attributes
//! of any type, `@property`, `@staticmethod`, `@classmethod`,
//! `functools.cached_property` and `functools.wraps` decorators, library
//! annotations (`re.Match`), unions (`int | float`, `X | None` of
//! objects), the `with` statement and starred expressions. The class
//! statements run in CPython with their method bodies compiled
//! (`src/fallback/lift.rs`); the oracle is CPython running the fixture.

#[cfg(unix)]
#[path = "memory_support/mod.rs"]
mod memory_support;

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

fn fixture(dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library_classes.py");
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), oracle.status.code());
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

fn build(source: &Path, out: &Path, ext: bool) -> String {
    let mut command = pycc();
    command.args(["build", "--non-strict", "--report-fallbacks"]);
    if ext {
        command.arg("--ext");
    }
    let build = command
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    text(&build.stderr)
}

/// Every method and function of the fixture compiles, but for the nested
/// `wrapper` closures the decorators return (whose enclosing `def` runs in
/// CPython).
/// An embedded executable does not publish a generator to CPython, so
/// there the `contextmanager` generator `scoped` runs in CPython too.
fn assert_compiled(report: &str, ext: bool) {
    for line in report.lines() {
        if let Some(rest) = line.strip_prefix("fallback: method `") {
            panic!("a method fell back: {rest}\n{report}");
        }
        if line.starts_with("fallback: function `")
            && !line.starts_with("fallback: function `traced`")
            && (ext || !line.starts_with("fallback: function `scoped`"))
        {
            panic!("a function fell back: {line}\n{report}");
        }
    }
}

#[test]
fn library_classes_match_cpython_in_a_native_build() {
    let dir = ScratchDir::new("library_classes_native").expect("scratch");
    let source = fixture(&dir, "m.py");
    assert_compiled(&build(&source, &dir.join("app"), false), false);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_eq!(oracle.status.code(), Some(1));
    assert_matches(&ours, &oracle);
}

#[test]
fn library_classes_match_cpython_in_an_ext_module() {
    let compiled = ScratchDir::new("library_classes_ext").expect("scratch");
    let source = fixture(&compiled, "lc.py");
    assert_compiled(&build(&source, &compiled.join("lc"), true), true);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("library_classes_ext_src").expect("scratch");
    fixture(&plain, "lc.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import lc\nlc.acct.balance = -5"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// Python code subclasses the module's classes: the metaclass registers the
/// subclass and the compiled `__init_subclass__` tags it, an override is
/// seen by the compiled methods, and `super()` reaches the compiled ones.
const SUBCLASS: &str = r#"
import lc


class Lower(lc.Plugin):
    def words(self, text):
        return [w + "!" for w in super().words(text)]


class Savings(lc.Account):
    rate = 3

    def deposit(self, amount):
        return super().deposit(amount * self.rate)


print(lc.registry[-1], Lower.tag, Lower().words("hi yo"))
s = Savings("sam", 10)
print(s.deposit(5), s.balance, s.label, isinstance(s, lc.Account), lc.Account.opened())
try:
    s.balance = -3
except ValueError as e:
    print("refused", e)
w = lc.Worker(10)
w.run()
print(w.result, lc.Tally("aab").top(1), str(lc.AppError(3, "x")))
"#;

#[test]
fn python_code_subclasses_the_library_classes() {
    let compiled = ScratchDir::new("library_classes_subclass").expect("scratch");
    let source = fixture(&compiled, "lc.py");
    build(&source, &compiled.join("lc"), true);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("library_classes_subclass_src").expect("scratch");
    fixture(&plain, "lc.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", SUBCLASS])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// A program sending values its generator's send type excludes (a fuzzer
/// finding): the generator function runs in CPython, natively and in an
/// `--ext` module CPython imports.
#[test]
fn a_generator_sent_other_values_runs_in_cpython() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sent_generator.py");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    let dir = ScratchDir::new("sent_generator").expect("scratch");
    let report = build(&source, &dir.join("app"), false);
    assert!(report.contains("fallback: function `acc`"), "{report}");
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    assert_matches(&ours, &oracle);
    let ext = dir.join("ext");
    std::fs::create_dir_all(&ext).expect("ext dir");
    build(&source, &ext.join("sg"), true);
    let ours = python()
        .args(["-c", "import sg"])
        .current_dir(&ext)
        .output()
        .expect("python should spawn");
    assert_matches(&ours, &oracle);
}

/// A compiled `with` body and a `Thread` subclass's compiled `run`, in a
/// loop, hold no memory per iteration.
#[cfg(unix)]
#[test]
fn with_statements_and_library_subclasses_run_in_constant_memory() {
    let dir = ScratchDir::new("library_classes_memory").expect("scratch");
    let mut peaks = Vec::new();
    for n in [20_000u64, 220_000] {
        let source = format!(
            "import collections\nimport contextlib\n\n\n\
             @contextlib.contextmanager\ndef scope(log):\n    log.append(1)\n    yield len(log)\n\n\n\
             class Tally(collections.Counter):\n    def bump(self, key, by: int) -> int:\n        \
             self[key] += by\n        return self[key]\n\n\n\
             def step(t, i: int) -> int:\n    log = []\n    with scope(log) as k:\n        \
             t.bump(str(i % 7), k)\n    try:\n        with scope(log):\n            \
             raise KeyError(i)\n    except KeyError:\n        pass\n    return len(log)\n\n\n\
             t = Tally()\ns = 0\nfor i in range({n}):\n    s += step(t, i)\nprint(s, sorted(t.items()))\n"
        );
        let name = format!("lib_{n}");
        let bin = memory_support::build_with(&dir, &name, &source, &["--non-strict"]);
        let (out, peak) = memory_support::run_measured(&bin);
        assert_eq!(
            text(&out),
            text(&memory_support::cpython(&dir.join(format!("{name}.py"))))
        );
        peaks.push(peak);
    }
    assert!(
        peaks[1] - peaks[0] <= memory_support::GROWTH_LIMIT_KB,
        "peak resident set grew from {} kB to {} kB",
        peaks[0],
        peaks[1]
    );
}
