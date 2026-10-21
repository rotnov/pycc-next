//! v0.8: an `--ext` module's CPython source (the class statements CPython
//! runs, the methods that fall back, the imports it mirrors) ships
//! pre-compiled (`src/ext_build/precompiled.rs`): importing the module
//! unmarshals each code object, as CPython loads a `.pyc`, instead of
//! calling `compile()`. The behaviour is the uncompiled module's: the same
//! results, tracebacks naming the source file and line, and under `-O` the
//! embedded source is compiled at import so its `assert`s are stripped.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
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

const MODULE: &str = r#"from __future__ import annotations

import collections


class Counted(collections.OrderedDict):
    """A class CPython creates (its base is a CPython class)."""

    def add(self, key: str, n: int = 1) -> int:
        self[key] = self.get(key, 0) + n
        return self[key]

    def checked(self, key: str) -> int:
        assert (lambda: key in self)(), "missing " + key
        return self[key]

    def fail(self) -> None:
        raise KeyError((lambda: "boom")())


def doubled(xs: list[int]) -> list[int]:
    return list(map(lambda v: v * 2, xs))
"#;

/// Counts the `compile()` calls for the module's own file while it is
/// imported, then exercises it; the count goes to stderr's first line.
const DRIVER: &str = r#"
import builtins, sys, traceback
calls = []
real = builtins.compile
def counting(source, filename, *args, **kwargs):
    if str(filename).endswith("m.py"):
        calls.append(filename)
    return real(source, filename, *args, **kwargs)
builtins.compile = counting
import m
builtins.compile = real
print("compiled at import:", len(calls), file=sys.stderr)
c = m.Counted()
print(c.add("a"), c.add("a", 5), c.checked("a"), m.doubled([1, 2]))
print("__pycc_code__" in vars(m), m.Counted.__module__, m.Counted.add.__qualname__)
try:
    c.checked("zz")
except (AssertionError, KeyError) as e:
    print(type(e).__name__, e)
try:
    c.fail()
except KeyError:
    frame = traceback.extract_tb(sys.exc_info()[2])[-1]
    print(frame.name, frame.lineno, frame.filename.endswith("m.py"))
"#;

fn run(dir: &Path, optimize: bool) -> Output {
    let mut command = python();
    if optimize {
        command.arg("-O");
    }
    command
        .args(["-c", DRIVER])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

fn compiled_at_import(output: &Output) -> String {
    text(&pycc_scratch::lf(&output.stderr))
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn an_ext_module_imports_its_fallback_code_without_compiling_it() {
    let compiled = ScratchDir::new("precompiled_ext").expect("scratch");
    let source = compiled.join("m.py");
    std::fs::write(&source, MODULE).expect("write the module");
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(
        text(&pycc_scratch::lf(&build.stderr)).contains("class `Counted`"),
        "the class runs in CPython: {}",
        text(&pycc_scratch::lf(&build.stderr))
    );
    let plain = ScratchDir::new("precompiled_ext_src").expect("scratch");
    std::fs::write(plain.join("m.py"), MODULE).expect("write the module");
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    for optimize in [false, true] {
        let ours = run(&compiled, optimize);
        let oracle = run(&plain, optimize);
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&oracle.stdout)),
            "{}",
            text(&pycc_scratch::lf(&ours.stderr))
        );
        assert_eq!(ours.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&ours.stderr)));
        // Unoptimized, every chunk is unmarshalled; under `-O` (whose
        // `assert`s the embedded code still holds) each is compiled from
        // the embedded source, as before.
        let count = compiled_at_import(&ours);
        if optimize {
            assert_ne!(count, "compiled at import: 0", "{}", text(&pycc_scratch::lf(&ours.stderr)));
        } else {
            assert_eq!(count, "compiled at import: 0", "{}", text(&pycc_scratch::lf(&ours.stderr)));
        }
    }
}

/// A constant set display (`{1, 8, 0}`) in fallback code iterates in the
/// source's order: `marshal` rebuilds a frozenset constant in its own
/// iteration order, so the build records each constant's elements in
/// source order and the import rebuilds it as the compiler does (the
/// differential fuzzer's finding), with no `compile()` at import. Checked
/// under several hash seeds, `str` and `int` elements, a nested function's
/// constant and a membership test.
#[test]
fn a_constant_set_display_in_fallback_code_keeps_the_sources_order() {
    let module = "def f(p0: tuple, p1: tuple) -> dict:\n    s: str = {1, 8, 0}\n    \
                  print(s, {3, 70, 1, 900}, frozenset({5, 37, 2}))\n    \
                  print({'d', 'a', 'c', 'b', 'e'}, {1, 9, 17, 25, 33, 2})\n    \
                  def g(k):\n        return [x for x in {'zz', 'k', 'q', 'aa'}], k in {9, 33, 1}\n    \
                  print(g(9))\n\n\nf(6, 5)\n";
    let compiled = ScratchDir::new("precompiled_sets").expect("scratch");
    let source = compiled.join("m.py");
    std::fs::write(&source, module).expect("write the module");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("precompiled_sets_src").expect("scratch");
    std::fs::write(plain.join("m.py"), module).expect("write the module");
    let driver = "import builtins, sys\ncalls = []\nreal = builtins.compile\n\
                  def counting(source, filename, *args, **kwargs):\n    \
                  calls.append(filename)\n    return real(source, filename, *args, **kwargs)\n\
                  builtins.compile = counting\nimport m\nbuiltins.compile = real\n\
                  print('compiled at import:', len(calls), file=sys.stderr)\n";
    for seed in ["0", "1", "7", "12345"] {
        let import = |dir: &Path| {
            python()
                .args(["-B", "-c", driver])
                .env("PYTHONHASHSEED", seed)
                .current_dir(dir)
                .output()
                .expect("python should spawn")
        };
        let (ours, oracle) = (import(&compiled), import(&plain));
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&oracle.stdout)),
            "seed {seed}: {}",
            text(&pycc_scratch::lf(&ours.stderr))
        );
        assert_eq!(ours.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&ours.stderr)));
        assert_eq!(
            compiled_at_import(&ours),
            "compiled at import: 0",
            "seed {seed}: {}",
            text(&pycc_scratch::lf(&ours.stderr))
        );
    }
}
