//! A module global that only fallback code may bind, read by compiled code
//! of a non-strict `--ext` module, is read as CPython's `LOAD_GLOBAL` reads
//! it (both found by tomli 2.5.0 on CPython 3.15): a name the module never
//! binds is the builtin's -- tomli imports `MappingProxyType as frozendict`
//! only before 3.15, where `frozendict` is a builtin -- and a PEP 810 lazy
//! import (`__lazy_modules__`) is resolved. A name bound nowhere is still
//! `NameError`.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// `abs` is bound by the import only when the condition holds (never), so
/// every read is the builtin's; `missing` is bound nowhere.
const MODULE: &str = r#"import sys

if sys.version_info < (3, 0):
    from os.path import join as abs

X = abs(-3)


def twice() -> int:
    return abs(-2) * 2


def broken() -> int:
    return missing() + 1
"#;

const DRIVER: &str = "import m\nprint(m.X, m.twice())\ntry:\n    m.broken()\n\
                      except NameError as e:\n    print('NameError', e)\n";

fn run(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    let out = python()
        .args(["-B", "-c", driver])
        .current_dir(dir)
        .output()
        .expect("python should spawn");
    (text(&out.stdout), text(&out.stderr), out.status.code())
}

#[test]
fn a_global_the_module_never_binds_is_read_from_the_builtins() {
    let compiled = ScratchDir::new("fallback_builtin_read").expect("scratch");
    std::fs::write(compiled.join("m.py"), MODULE).expect("write the module");
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--select", "module"])
        .arg(compiled.join("m.py"))
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(compiled.join("m.py")).expect("import the artifact, not the source");
    let plain = ScratchDir::new("fallback_builtin_read_src").expect("scratch");
    std::fs::write(plain.join("m.py"), MODULE).expect("write the module");
    let (ours, oracle) = (run(&compiled, DRIVER), run(&plain, DRIVER));
    assert_eq!(
        oracle.0, "3 4\nNameError name 'missing' is not defined\n",
        "{oracle:?}"
    );
    assert_eq!(ours, oracle);
}

/// A package whose `__init__` lists its `_re` submodule in
/// `__lazy_modules__`: on 3.15 the `from ._re import ...` that fallback code
/// runs binds lazy imports, which compiled `first` reads; elsewhere the
/// import is eager.
const LAZY_INIT: &str = r#"__lazy_modules__ = ["pkg._re"]

from ._re import PATTERN, Any

KIND: Any = 1


def first(text: str) -> str:
    m = PATTERN.match(text)
    return m.group(0) if m else ""
"#;

const LAZY_RE: &str = "import re\nfrom typing import Any\n\nPATTERN = re.compile(\"[a-z]+\")\n";

#[test]
fn a_lazy_import_fallback_code_binds_is_resolved_when_compiled_code_reads_it() {
    let write = |root: &Path| {
        std::fs::create_dir_all(root.join("pkg")).expect("mkdir");
        std::fs::write(root.join("pkg/__init__.py"), LAZY_INIT).expect("write");
        std::fs::write(root.join("pkg/_re.py"), LAZY_RE).expect("write");
    };
    let compiled = ScratchDir::new("fallback_lazy_import").expect("scratch");
    write(&compiled);
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--select", "module"])
        .arg(compiled.join("pkg"))
        .arg("-o")
        .arg(compiled.join("pkg/__init__"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    for source in ["pkg/__init__.py", "pkg/_re.py"] {
        std::fs::remove_file(compiled.join(source)).expect("import the artifact, not the source");
    }
    let plain = ScratchDir::new("fallback_lazy_import_src").expect("scratch");
    write(&plain);
    let driver = "import pkg\nprint(pkg.first('abc1'), pkg.first('1'), pkg.KIND)\n";
    let (ours, oracle) = (run(&compiled, driver), run(&plain, driver));
    assert_eq!(oracle.0, "abc  1\n", "{oracle:?}");
    assert_eq!(ours, oracle);
}
