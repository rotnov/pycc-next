//! `int | float` (v0.6 accept 3): a value that is an `int` at one point and
//! a `float` at another, annotated or inferred. Each fixture is built in
//! strict mode, with `--non-strict` and as an `--ext` module, and every run
//! is compared with the live CPython oracle (`PYCC_PYTHON`, default
//! `python3`).

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

/// Every operation on an `int | float` value, with CPython's semantics for
/// whichever it holds: arithmetic (`//`, `%`, `**` included), mixed and
/// exact big-int comparisons, `abs`, `round`, `int()`, `float()`, text,
/// format specs, hashing and the errors each raises.
#[test]
fn every_operation_matches_cpython() {
    assert_native_matches("num_union_ops.py");
    assert_ext_matches("num_union_ops.py", "import num_union_ops");
}

/// The inferred forms: a `float('inf')` sentinel an `int` replaces, an
/// accumulator that starts as `0` and receives floats, branches binding
/// each, a helper called with both, and lists and dicts holding both.
#[test]
fn inferred_unions_match_cpython() {
    assert_native_matches("num_union_infer.py");
    assert_ext_matches("num_union_infer.py", "import num_union_infer");
}

/// A list whose first binding builds `int`s and that later stores a
/// `float` (or the reverse) holds `int | float` elements; an `int | float`
/// index or `range()` bound raises CPython's `TypeError` for a `float`.
#[test]
fn lists_and_indices_match_cpython() {
    assert_native_matches("num_union_lists.py");
    assert_ext_matches("num_union_lists.py", "import num_union_lists");
}

/// An `int | float` parameter takes an `int`, a `bool`, a big `int` or a
/// `float`, refuses anything else with a `TypeError`, and a result crosses
/// back as an `int` or a `float`.
#[test]
fn the_ext_boundary_carries_both_kinds() {
    assert_ext_matches(
        "num_union_ext.py",
        "from num_union_ext import scale, nearest, describe\n\
         for x in [2, 2.5, True, 10 ** 30, 1e300, -0.0, float('nan')]:\n\
         \x20   r = scale(x, 2)\n\
         \x20   print(repr(r), type(r).__name__, describe(x))\n\
         print(scale(3, 0.5), scale(1e200, 1e200), scale(2 ** 70, 2 ** 70))\n\
         print(nearest(0), nearest(4), type(nearest(1)).__name__)\n\
         try:\n\
         \x20   scale(None, 2)\n\
         except TypeError:\n\
         \x20   print('TypeError')\n",
    );
}

/// `int | float` containers looked up with a `float` -- membership in a
/// list, dict and set, a dict read and `.get()` -- including floats only
/// a heap box holds.
#[test]
fn float_lookups_in_num_containers_match_cpython() {
    assert_native_matches("num_union_lookups.py");
    assert_ext_matches("num_union_lookups.py", "import num_union_lookups");
}

/// Every runtime path of an `int | float` value -- arithmetic, the errors
/// it raises, comparisons, `min`/`max`/`abs`/`round`/`hash`, text and
/// format specs, containers holding boxed floats and big ints, a `float`
/// needle or key borrowed by a container lookup, and the `--ext`
/// boundary in both directions -- releases the boxed floats it makes: the
/// runtime's live count is flat across a hosted loop.
#[test]
fn boxed_floats_do_not_leak() {
    let source = support::fixture("num_union_leaks.py");
    let dir = pycc_scratch::ScratchDir::new("num_union_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("num_union_leaks"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    // Every line carries its own indentation inside the literal: Rust's
    // `\`-continuation strips the next source line's leading whitespace.
    let driver = "import ctypes\n\
                  import num_union_leaks as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  live = lib.pycc_rt_num_box_live_objects\n\
                  live.restype = ctypes.c_int64\n\
                  live.argtypes = []\n\
                  def delta(call):\n    \
                  call()\n    \
                  before = live()\n    \
                  for _ in range(200):\n        call()\n    \
                  return live() - before\n\
                  for name, call in (\n\
                  ('churn', lambda: m.churn(5, 3)),\n\
                  ('churn-float', lambda: m.churn(5, -2.5e10)),\n\
                  ('echo', lambda: m.echo(1e200)),\n\
                  ('echo-int', lambda: m.echo(10 ** 40)),\n\
                  ('lookups', lambda: m.lookups(3, 2.5)),\n\
                  ('lookups-boxed', lambda: m.lookups(2, -1e250)),\n\
                  ('lookups-int', lambda: m.lookups(2, 10 ** 40)),\n\
                  ):\n    \
                  d = delta(call)\n    \
                  assert d == 0, '%s leaked %d boxed floats over 200 calls' % (name, d)\n\
                  assert m.echo(1e200) == 1e200 and m.echo(-1e-300) == -1e-300\n\
                  print('ok')\n";
    let run = std::process::Command::new(support::python())
        .args(["-B", "-c", driver])
        .current_dir(&*dir)
        .output()
        .expect("python3 runs");
    assert_eq!(
        pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)).replace("\r\n", "\n"),
        "ok\n",
        "{}",
        pycc_scratch::lf(&*String::from_utf8_lossy(&run.stderr))
    );
}

/// Big `int`s and boxed floats held in lists, tuples, dicts (keys and
/// values) and sets: stored, read, copied, sorted, compared, hashed and
/// printed as CPython does, in every build mode.
#[test]
fn heap_numbers_live_in_containers() {
    assert_native_matches("heap_numbers_in_containers.py");
    assert_ext_matches("heap_numbers_in_containers.py", "import heap_numbers_in_containers");
}

/// Containers whose numbers differ in kind compare by value (`(1, 2.5) ==
/// (1.0, 2.5)`, tuples of different arity included), and a `float` key
/// looks up an `int`-keyed dict, raising `KeyError` with its own repr.
#[test]
fn numbers_of_different_kinds_compare_and_look_up_by_value() {
    assert_native_matches("num_union_cross_kind.py");
    assert_ext_matches("num_union_cross_kind.py", "import num_union_cross_kind");
}
