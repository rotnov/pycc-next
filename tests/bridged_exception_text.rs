//! v0.8: a CPython exception that compiled `--ext` code catches carries no
//! text of its own: `str(e)` is CPython's `str()` of the original, rendered
//! when compiled code first reads it (`pycc_ext_exception_str_hook`), so a
//! handled exception whose text is never shown costs no `str()` call, and a
//! `__str__` that reads state shows the state of the moment it is read, as
//! in CPython (the text used to be rendered when the exception was caught).
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

const HELPER: &str = r#"LABEL = ["first"]


class Late(Exception):
    def __str__(self):
        return "late:" + LABEL[0]


def boom():
    raise Late()
"#;

const MODULE: &str = r#"import helper


def late() -> str:
    try:
        helper.boom()
    except Exception as e:
        helper.LABEL[0] = "changed"
        return str(e) + "|" + f"{e}"
    return ""


def missing(d: object, k: object) -> str:
    try:
        return str(d[k])
    except KeyError as e:
        return "KeyError " + str(e)


def absent(o: object) -> str:
    try:
        return str(o.nope)
    except AttributeError as e:
        return "AttributeError " + str(e)


def quiet(d: object) -> int:
    n = 0
    for i in range(1000):
        try:
            n += d[i]
        except KeyError:
            n -= 1
    return n


def escape(d: object) -> object:
    return d["gone"]
"#;

const DRIVER: &str = r#"
import m
print(m.late())
print(m.missing({}, "k"), m.missing({}, 3), m.missing({}, (1, "a")))
print(m.absent(object()))
print(m.quiet({1: 5}))
try:
    m.escape({})
except KeyError as e:
    print(type(e).__name__, e, e.args)
"#;

fn run(dir: &Path) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .args(["-c", DRIVER])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

#[test]
fn a_caught_cpython_exception_shows_its_text_when_it_is_read() {
    let compiled = ScratchDir::new("bridged_text_ext").expect("scratch");
    let source = compiled.join("m.py");
    std::fs::write(&source, MODULE).expect("write the module");
    // Written after the build: `helper` is a CPython module, not a
    // project module compiled into the artifact.
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    std::fs::write(compiled.join("helper.py"), HELPER).expect("write the helper");
    let plain = ScratchDir::new("bridged_text_src").expect("scratch");
    std::fs::write(plain.join("m.py"), MODULE).expect("write the module");
    std::fs::write(plain.join("helper.py"), HELPER).expect("write the helper");
    let ours = run(&compiled);
    let oracle = run(&plain);
    assert_eq!(oracle.status.code(), Some(0), "{}", text(&oracle.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}
