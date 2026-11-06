//! v0.12: a module-level function of a non-strict `--ext` module used as a
//! value that crosses into CPython (`{"k": f}`, `(f, g)`, `return table[k]`)
//! is the function the module binds under its name, as CPython code reads
//! it (`module_ctx.c`'s `pycc_ext_obj_pack_function`): the same object as
//! `m.f`, called with keywords and defaults (`functools.partial(f, **kw)`,
//! pydantic's constraint validators). And a function with a default and a
//! parameter named as the function itself (`def f(x, f=None)`,
//! `def f(*f)`, `def f(**f)`) keeps its own name callable: its compiled
//! entry names that parameter apart (`pycc_ast::ext_signatures`), in
//! strict and non-strict builds.
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

fn run(dir: &Path, driver: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .args(["-B", driver])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// `module`'s `driver` run against its `--ext` build (with `flags`) and
/// against its source, each in a fresh directory.
fn compare(module: &str, driver: &str, flags: &[&str]) {
    let ours = ScratchDir::new(&format!("{module}_ext")).expect("scratch");
    let oracle = ScratchDir::new(&format!("{module}_src")).expect("scratch");
    let file = format!("{module}.py");
    for dir in [&ours, &oracle] {
        std::fs::write(dir.join(driver), source(driver)).expect("write the driver");
        std::fs::write(dir.join(&file), source(&file)).expect("write the module");
    }
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext"])
        .args(flags)
        .arg(ours.join(&file))
        .arg("-o")
        .arg(ours.join(module))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(ours.join(&file)).expect("import the artifact, not the source");
    let (got, want) = (run(&ours, driver), run(&oracle, driver));
    assert_eq!(want.status.code(), Some(0), "{}", text(&want.stderr));
    assert_eq!(got.status.code(), Some(0), "{}", text(&got.stderr));
    assert_eq!(text(&got.stdout), text(&want.stdout));
}

#[test]
fn function_values_crossing_into_cpython_are_the_published_functions() {
    compare("fvals", "driver.py", &["--non-strict", "--select", "module"]);
}

#[test]
fn a_parameter_named_as_its_function_is_apart_from_it_in_a_strict_build() {
    compare("strict", "strict_driver.py", &[]);
}
