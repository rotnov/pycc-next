//! `tuple(xs)` of a list or a `str` (v0.6 accept 5): a native tuple object
//! (`tuple[T, ...]`) a list can hold, unpacked with CPython's length check,
//! compared, hashed, sorted and printed as a tuple. Built in strict mode,
//! with `--non-strict` and as an `--ext` module, and compared with the live
//! CPython oracle.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

#[test]
fn tuple_of_a_list_matches_cpython() {
    assert_native_matches("tuple_of_list.py");
    assert_ext_matches("tuple_of_list.py", "import tuple_of_list");
}

/// A `tuple[int, ...]` has no fixed-arity form, so a `tuple[int, int]`
/// display does not meet it in a container: a compile error, never a
/// miscompile. The annotation itself stays refused (`T0053`).
#[test]
fn a_fixed_int_tuple_does_not_join_a_variadic_one() {
    let rendered = support::check_fails("s = {tuple([1, 2]), (1, 2)}\nprint(s)\n");
    assert!(rendered.contains("T0037"), "{rendered}");
    let rendered = support::check_fails("def f() -> tuple[int, ...]:\n    return tuple([1])\n");
    assert!(rendered.contains("T0053"), "{rendered}");
}

/// The runtime's live list count is flat across a hosted loop that builds,
/// stores, unpacks and drops `tuple(xs)` values.
#[test]
fn tuples_of_lists_do_not_leak() {
    let source = support::fixture("tuple_of_list_leaks.py");
    let dir = pycc_scratch::ScratchDir::new("tuple_of_list_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("tuple_of_list_leaks"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    let driver = "import ctypes\n\
                  import tuple_of_list_leaks as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  live = lib.pycc_rt_list_live_objects\n\
                  live.restype = ctypes.c_int64\n\
                  live.argtypes = []\n\
                  strs = lib.pycc_rt_str_live_objects\n\
                  strs.restype = ctypes.c_int64\n\
                  strs.argtypes = []\n\
                  m.churn(5)\n\
                  before = (live(), strs())\n\
                  for _ in range(200):\n    \
                  m.churn(5)\n\
                  after = (live(), strs())\n\
                  assert after == before, 'leaked: %r -> %r' % (before, after)\n\
                  print('ok')\n";
    let run = std::process::Command::new(support::python())
        .args(["-B", "-c", driver])
        .current_dir(&*dir)
        .output()
        .expect("python3 runs");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "ok\n",
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
