//! Non-strict `--ext`: the attribute caches of `src/ext/module/attr_cache.c`
//! (instance values, slots, properties, methods, module attributes) and the
//! once-built set display a membership test reads give CPython's results
//! through every way a cached entry can go stale -- an attribute deleted or
//! shadowed, a class changed, a property re-initialized, a `__getattr__`
//! added, a module rebinding a name -- and keep every reference balanced.
//! Each fixture runs with the caches on (and must hit them: the probe passes
//! on every CPython 3.13 the tests run on) and with `PYCC_NO_ATTR_CACHE=1`.
//! The oracle is CPython running the fixture.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

/// Builds `tests/fixtures/<fixture>.py` as an extension module and compares
/// importing it, with the caches on and off, against CPython running it;
/// `hits` when the fixture reads attributes the caches must answer (with
/// the caches off, none is answered either way).
fn matches_cpython(fixture: &str, hits: bool) {
    let dir = ScratchDir::new(fixture).expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{fixture}.py"));
    let module = format!("{fixture}_ext");
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict"])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(&module))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    let probe = format!(
        "import ctypes, sys\nimport {module}\n\
         hits = ctypes.CDLL({module}.__file__).pycc_ext_attr_cache_hits()\n\
         print('HITS', hits > 0, file=sys.stderr)\n"
    );
    for off in [false, true] {
        let mut run = python();
        run.args(["-c", &probe]).current_dir(&*dir);
        if off {
            run.env("PYCC_NO_ATTR_CACHE", "1");
        } else {
            run.env_remove("PYCC_NO_ATTR_CACHE");
        }
        let ours = run.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&ours.stderr));
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&oracle.stdout)),
            "caches off: {off}\n{stderr}"
        );
        if off || hits {
            let expected = if off { "HITS False" } else { "HITS True" };
            assert!(stderr.contains(expected), "caches off: {off}\n{stderr}");
        }
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn instance_values_methods_and_their_invalidation() {
    matches_cpython("attr_cache_values", true);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn slots_properties_builtin_methods_and_several_classes_at_one_site() {
    matches_cpython("attr_cache_kinds", true);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn module_attributes_rebound_deleted_and_called() {
    matches_cpython("attr_cache_modules", true);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_method_is_loaded_before_its_arguments_run() {
    matches_cpython("attr_cache_calls", true);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn cached_reads_keep_references_balanced() {
    matches_cpython("attr_cache_leaks", true);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn membership_in_a_set_display_of_literals() {
    matches_cpython("set_display_membership", false);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_empty_literal_and_its_neighbour_compare_apart() {
    matches_cpython("text_literal_compare", false);
}
