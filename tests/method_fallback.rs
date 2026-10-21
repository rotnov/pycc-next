//! Method-level fallback (v0.7 accept 3): one construct `--non-strict`
//! cannot compile sends only its own method or function to CPython. The
//! class statement holding it runs in CPython, every other method body
//! compiled (`src/fallback/lift.rs`).
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) running the fixture itself:
//! a native build bundling that interpreter, an `--ext` module imported
//! into it, and Python code subclassing the compiled module's classes.

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
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/method_fallback.py");
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), oracle.status.code());
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&oracle.stderr)));
}

/// What runs in CPython, from `--report-fallbacks`: only the methods using
/// what pycc cannot compile, never a whole class's methods. In an `--ext`
/// module `Bag` stays a compiled class, its failing method attached to it.
fn assert_report(report: &str, ext: bool) {
    for method in ["Bag.ranked", "Rich.ordered"] {
        assert!(
            report.contains(&format!("fallback: method `{method}` at ")),
            "{method} should fall back alone:\n{report}"
        );
    }
    for method in [
        "Bag.total", "Bag.add", "Bag.tagged", "Bag.checked", "Rich.total", "Rich.add",
        "Rich.secret", "Rich.size", "Rich.of", "Rich.describe", "Labeled.total",
    ] {
        assert!(
            !report.contains(&format!("method `{method}`")),
            "{method} should compile:\n{report}"
        );
    }
    if ext {
        assert!(!report.contains("class `Bag`"), "{report}");
        assert!(report.contains("the method attached to it"), "{report}");
        assert!(!report.contains("method `Rich.evens`"), "{report}");
    }
    assert!(report.contains("lifted: function `summed`"), "{report}");
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
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    text(&pycc_scratch::lf(&build.stderr))
}

#[test]
fn only_the_failing_methods_run_in_cpython_in_a_native_build() {
    let dir = ScratchDir::new("method_fallback_native").expect("scratch");
    let source = fixture(&dir, "m.py");
    let report = build(&source, &dir.join("app"), false);
    assert_report(&report.replace(&source.display().to_string(), "m.py"), false);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_eq!(oracle.status.code(), Some(1));
    assert_matches(&ours, &oracle);
}

#[test]
fn only_the_failing_methods_run_in_cpython_in_an_ext_module() {
    let compiled = ScratchDir::new("method_fallback_ext").expect("scratch");
    let source = fixture(&compiled, "m.py");
    let report = build(&source, &compiled.join("m"), true);
    assert_report(&report.replace(&source.display().to_string(), "m.py"), true);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("method_fallback_ext_src").expect("scratch");
    fixture(&plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import m\nm.r.add(1000)"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// Python code subclasses a class whose methods are compiled, overrides
/// one, reaches the compiled one through `super()`, and sees the compiled
/// methods dispatch to its override.
const SUBCLASS: &str = r#"
import m


class Mine(m.Rich):
    def add(self, value):
        super().add(value * 10)

    def total(self):
        return -super().total()


b = Mine(1)
b.add(1)
b.add(2)
print(b.total(), b.items, b.describe("M", z=1), type(b).__mro__[1].__name__, b.secret())
print(m.Rich.total.__name__, m.Rich.add.__qualname__, m.Labeled.__mro__[1] is m.Rich)
print(Mine.of(3).total(), list(Mine.of(2, 4).evens()))
"#;

#[test]
fn python_code_subclasses_a_class_with_compiled_methods() {
    let compiled = ScratchDir::new("method_fallback_subclass").expect("scratch");
    let source = fixture(&compiled, "m.py");
    build(&source, &compiled.join("m"), true);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("method_fallback_subclass_src").expect("scratch");
    fixture(&plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", SUBCLASS])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// A lifted method called in a loop holds no memory per call: the
/// forwarder's arguments, the compiled body's object operations on `self`
/// and its result are all released.
#[cfg(unix)]
#[test]
fn lifted_methods_run_in_constant_memory() {
    let dir = ScratchDir::new("method_fallback_memory").expect("scratch");
    let mut peaks = Vec::new();
    for n in [20_000u64, 220_000] {
        let source = format!(
            "class Acc:\n    def __init__(self) -> None:\n        self.items = []\n        \
             self.total = 0\n\n    def add(self, v: int) -> int:\n        \
             self.items.append(str(v))\n        if len(self.items) > 8:\n            \
             self.items = []\n        self.total += v\n        return self.total\n\n    \
             def keyed(self):\n        key = lambda v: -v\n        return key(1)\n\n\n\
             a = Acc()\nfor i in range({n}):\n    a.add(i)\nprint(a.total, a.keyed())\n"
        );
        let name = format!("acc_{n}");
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
