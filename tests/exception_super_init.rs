//! `super().__init__(...)` in an exception class with its own `__init__`
//! reaches the builtin exception's constructor, which a native instance of
//! such a class cannot run (it carries no exception state). pycc used to
//! compile the call into a call of an unbound synthetic function, so the
//! program failed at run time with a `NameError` CPython never raises; it
//! is a compile-time `C0001` now.

use pycc_scratch::ScratchDir;
use std::process::Command;

const PROGRAM: &str = r#"class Owned(Exception):
    def __init__(self, code: int) -> None:
        super().__init__(f"code {code}")
        self.code = code


def make(n: int) -> int:
    return Owned(n).code


print(make(3))
"#;

#[test]
fn super_init_to_a_builtin_exception_is_refused_at_compile_time() {
    let dir = ScratchDir::new("exception_super_init").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(&source, PROGRAM).expect("write the program");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(!build.status.success(), "{stderr}");
    assert!(
        stderr.contains("error[C0001]: `super().__init__(...)` in exception class `Owned`"),
        "{stderr}"
    );
}
