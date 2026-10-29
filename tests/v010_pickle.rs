//! Pickling compiled instances (v0.10): a published compiled class's
//! instances pickle and unpickle as CPython's do (every protocol,
//! `copy.deepcopy`, shared references, nested instances, unassigned
//! attributes, Python subclasses with a `__dict__`), through the shim's
//! `__reduce__` (`pycc_ext_instance_reduce`). `tests/fixtures/
//! v010_pickle_host.py` runs against the module built with `--ext
//! --non-strict` and against its source.

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
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn build(dir: &Path, flags: &[&str]) {
    let source = fixture("v010_pickle", dir, "m.py");
    let build = pycc()
        .args(["build", "--ext"])
        .args(flags)
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
}

fn run(dir: &Path, script: &str) -> Output {
    python()
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

fn pickling_matches_cpython(flags: &[&str]) {
    let compiled = ScratchDir::new("v010_pickle").expect("scratch");
    build(&compiled, flags);
    fixture("v010_pickle_host", &compiled, "host.py");
    let plain = ScratchDir::new("v010_pickle_src").expect("scratch");
    fixture("v010_pickle", &plain, "m.py");
    fixture("v010_pickle_host", &plain, "host.py");
    let script = "exec(open('host.py').read())";
    let ours = run(&compiled, script);
    let theirs = run(&plain, script);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}

#[test]
fn compiled_instances_pickle_ext() {
    pickling_matches_cpython(&["--non-strict"]);
}

#[test]
fn compiled_instances_pickle_ext_release() {
    pickling_matches_cpython(&["--non-strict", "--release"]);
}

/// An instance with an attribute Python code cannot see (a list of native
/// values) refuses to pickle rather than losing the attribute
/// (`docs/DEVIATIONS.md`).
#[test]
fn an_attribute_python_cannot_see_refuses_to_pickle() {
    let compiled = ScratchDir::new("v010_pickle_hidden").expect("scratch");
    build(&compiled, &["--non-strict"]);
    let out = run(
        &compiled,
        "import pickle, m\ntry:\n    pickle.dumps(m.make(4))\nexcept TypeError as e:\n    \
         print(e)\n",
    );
    assert_eq!(
        text(&out.stdout),
        "cannot pickle 'm.Path' object: its attribute 'steps' is not visible to Python\n",
        "{}",
        text(&out.stderr)
    );
}

/// Pickling and unpickling frees what it allocates.
#[test]
fn pickling_does_not_leak() {
    let compiled = ScratchDir::new("v010_pickle_leaks").expect("scratch");
    build(&compiled, &["--non-strict"]);
    let out = run(
        &compiled,
        "import ctypes, pickle, m\nlib = ctypes.CDLL(m.__file__)\n\
         live = lambda: (lib.pycc_rt_str_live_objects(), lib.pycc_rt_instance_live_objects())\n\
         def work():\n    r = m.Ring(m.Point(1, 2.0, 'r', False), 7)\n    \
         pickle.loads(pickle.dumps([r, r, m.Lazy(0), m.Lazy(3)])); m.total_ring(r)\n\
         for _ in range(100):\n    work()\nbefore = live()\nfor _ in range(1000):\n    work()\n\
         print('leaks', 'none' if live() == before else (before, live()))\n",
    );
    assert_eq!(text(&out.stdout), "leaks none\n", "{}", text(&out.stderr));
}
