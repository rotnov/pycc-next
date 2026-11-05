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
    matches_cpython_hitting(fixture, usize::from(hits));
}

/// [`matches_cpython`], the caches answering at least `min_hits` reads
/// (`0`: not checked).
fn matches_cpython_hitting(fixture: &str, min_hits: usize) {
    matches_cpython_counting(fixture, min_hits, 0);
}

/// [`matches_cpython_hitting`], the call sites' caches also making at least
/// `min_forward` calls straight to a compiled body behind its Python
/// forwarder (`pycc_ext_site_forward_calls`; none with the caches off).
fn matches_cpython_counting(fixture: &str, min_hits: usize, min_forward: usize) {
    let hits = min_hits > 0;
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
         print('HITS', hits > 0, file=sys.stderr)\n\
         print('COUNT', hits, file=sys.stderr)\n\
         print('FORWARD', ctypes.CDLL({module}.__file__).pycc_ext_site_forward_calls(), \
         file=sys.stderr)\n"
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
        if !off && hits {
            let count: usize = stderr
                .lines()
                .find_map(|line| line.strip_prefix("COUNT "))
                .and_then(|n| n.trim().parse().ok())
                .expect("the probe prints the hit count");
            assert!(count >= min_hits, "{count} hits, at least {min_hits} expected\n{stderr}");
        }
        if min_forward > 0 {
            let forward: usize = stderr
                .lines()
                .find_map(|line| line.strip_prefix("FORWARD "))
                .and_then(|n| n.trim().parse().ok())
                .expect("the probe prints the forwarded call count");
            if off {
                assert_eq!(forward, 0, "caches off\n{stderr}");
            } else {
                assert!(
                    forward >= min_forward,
                    "{forward} forwarded calls, at least {min_forward} expected\n{stderr}"
                );
            }
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

/// v0.9: a module attribute whose dict key is a `str` equal to the read's
/// name but not the same object -- what a non-strict module's compiled
/// global stores (`ns['LIMIT'] = LIMIT`) insert -- is cached too: every
/// read in the loop is a hit, and a rebinding is still seen.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn module_attributes_stored_by_compiled_code_are_cached() {
    matches_cpython_hitting("attr_cache_module_keys", 400);
}

/// v0.12: a method call site's cache holds a method of a class CPython
/// creates whose instances keep their values inline, as long as no instance
/// attribute can shadow it, and calls a compiled method behind its Python
/// forwarder straight through the forwarder's slot: CPython's results with
/// every count of arguments the cache copies (and one more), keywords,
/// an instance attribute of the method's name, a new attribute in the
/// shared keys, a materialized instance dict and class attributes rebound.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn method_calls_on_a_class_cpython_creates() {
    matches_cpython_counting("forwarder_calls", 1, 300);
}

/// v0.12: an `int` index into an exact `tuple` or `list` read at its
/// layout, `isinstance` of a built-in class answered for a plain built-in
/// type, and standard-library functions found by their call site's names
/// in a hashed table give CPython's results, with the caches on and off.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn builtin_object_operations_answered_by_the_shim() {
    matches_cpython("builtin_object_ops", false);
}
