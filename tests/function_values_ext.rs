//! v0.12: a module-level function of a non-strict `--ext` module used as a
//! value that crosses into CPython (`{"k": f}`, `(f, g)`, `return table[k]`)
//! is the function the module binds under its name, as CPython code reads
//! it (`module_ctx.c`'s `pycc_ext_obj_pack_function`): the same object as
//! `m.f`, called with keywords and defaults (`functools.partial(f, **kw)`,
//! pydantic's constraint validators). And a function with a default and a
//! parameter named as the function itself (`def f(x, f=None)`) keeps its
//! own name callable: no compiled entry is planned for it
//! (`pycc_ast::ext_signatures`), its `def` runs in CPython.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn source(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/function_values").join(name);
    std::fs::read_to_string(path).expect("read the fixture").replace("\r\n", "\n")
}

fn run(dir: &Path) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .args(["-B", "driver.py"])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

#[test]
fn function_values_crossing_into_cpython_are_the_published_functions() {
    let ours = ScratchDir::new("function_values").expect("scratch");
    let oracle = ScratchDir::new("function_values_src").expect("scratch");
    for dir in [&ours, &oracle] {
        std::fs::write(dir.join("driver.py"), source("driver.py")).expect("write the driver");
        std::fs::write(dir.join("fvals.py"), source("fvals.py")).expect("write the module");
    }
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict"])
        .arg(ours.join("fvals.py"))
        .arg("-o")
        .arg(ours.join("fvals"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(ours.join("fvals.py")).expect("import the artifact, not the source");
    let (got, want) = (run(&ours), run(&oracle));
    assert_eq!(want.status.code(), Some(0), "{}", text(&want.stderr));
    assert_eq!(got.status.code(), Some(0), "{}", text(&got.stderr));
    assert_eq!(text(&got.stdout), text(&want.stdout));
}
