//! v0.7 accept 5 (the real-world corpus) and the runtime defects found on
//! the way. Each fixture is built in strict mode and with `--non-strict`
//! (and as an `--ext` module where it crosses the boundary), and every run
//! is compared with the live CPython oracle (`PYCC_PYTHON`, default
//! `python3`): stdout, the uncaught exception's last line and the exit
//! status.
//!
//! * #3: `list()`, `tuple()` and `sorted()` of a `range` longer than a C
//!   `ssize_t` raise CPython's `OverflowError` instead of iterating forever.
//! * #4: a `str` container's missing key or index raises without leaking
//!   the placeholder `str` returned beside the exception.
//! * #5, #6, #7 (corpus 188): `isinstance` tests a static type decides,
//!   parameter webs and per-call-site copies of nested helpers
//!   (`pycc_types::static_isinstance`, `pycc_ast::desugar::webs`).

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

/// #3: CPython takes a range's length before materializing it; one past a
/// C `ssize_t` raises `OverflowError` (big bounds, big steps, descending
/// ranges); a short range with big bounds, an empty one and a zero step
/// behave as before.
#[test]
fn materializing_an_oversized_range_raises_overflow_error() {
    assert_native_matches("range_len_overflow.py");
    assert_ext_matches(
        "range_len_overflow.py",
        "try:\n    import range_len_overflow\nexcept OverflowError as e:\n    print('OverflowError', e)\n",
    );
}

/// #4: a missing key of a `dict[str, str]`, an out-of-range index of a
/// `list[str]` and `pop()` of an empty one raise as CPython does.
#[test]
fn missing_str_values_raise_as_cpython_does() {
    assert_native_matches("missing_str_values.py");
}

/// #4: the placeholder `str` a raising `str` lookup returns is the shared
/// immortal empty `str`, never a fresh one the exception path drops: the
/// runtime's live `str` count is flat across hosted calls that catch the
/// error, exactly as for the same lookups on `int` values.
#[cfg(not(target_os = "windows"))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn raising_str_lookups_do_not_leak_their_placeholder() {
    let source = support::fixture("missing_str_values.py");
    let dir = pycc_scratch::ScratchDir::new("missing_str_values").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("missing_str_values"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    // Every line carries its own indentation inside the literal: Rust's
    // `\`-continuation strips the next source line's leading whitespace.
    let driver = "import ctypes, io, contextlib\n\
                  with contextlib.redirect_stdout(io.StringIO()):\n    \
                  import missing_str_values as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  live = lib.pycc_rt_str_live_objects\n\
                  live.restype = ctypes.c_int64\n\
                  live.argtypes = []\n\
                  def delta(call):\n    \
                  call()\n    \
                  before = live()\n    \
                  for _ in range(200):\n        call()\n    \
                  return live() - before\n\
                  key = delta(lambda: m.missing_str('zz'))\n\
                  key_int = delta(lambda: m.missing_int('zz'))\n\
                  assert key == key_int, 'dict[str, str] KeyError leaked %d strs (int values: %d)' % (key, key_int)\n\
                  index = delta(lambda: m.out_of_range(7))\n\
                  index_int = delta(lambda: m.out_of_range_int(7))\n\
                  assert index == index_int, 'list[str] IndexError leaked %d strs (int values: %d)' % (index, index_int)\n\
                  pop = delta(lambda: m.pop_empty(0))\n\
                  assert pop == index_int, 'list[str].pop() leaked %d strs (int index: %d)' % (pop, index_int)\n\
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

/// #5, #6 and #7 (corpus 188): `isinstance` tests a subject's static type
/// decides fold to the branch that runs (their subjects still evaluated,
/// `IndexError` included); a parameter such a guard normalizes, or any
/// unconditionally rebound parameter, gets a variable per value; a nested
/// helper whose calls give it different argument types gets a copy per
/// call site.
#[test]
fn static_isinstance_guards_match_cpython() {
    assert_native_matches("static_isinstance.py");
}

/// The same through `--ext` entry points.
#[test]
fn static_isinstance_guards_match_cpython_through_ext() {
    assert_native_matches("static_isinstance_ext.py");
    assert_ext_matches(
        "static_isinstance_ext.py",
        "import static_isinstance_ext as m\n\
         print(m.fib_mod(7, 100), m.shapes(5), m.head(2))\n",
    );
}

/// A test the subject's type does not decide is not folded: in a module
/// that builds a `deque` (typed `list[T]`), `isinstance(x, list)` of a
/// `list[T]` may be either, and strict mode still refuses it.
#[test]
fn an_undecided_isinstance_test_is_still_refused() {
    let rendered = support::check_fails(
        "from collections import deque\n\n\n\
         def _f(xs: list[list[int]]) -> int:\n    if isinstance(xs[0], list):\n        return 1\n    return 0\n\n\n\
         print(_f([[1]]), deque([1]))\n",
    );
    assert!(rendered.contains("`list` is not a known class"), "{rendered}");
}

/// The folded guards, their evaluated subjects and the per-call-site
/// copies release what they build: the runtime's live list and `str`
/// counts are flat across hosted calls.
#[cfg(not(target_os = "windows"))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn static_isinstance_guards_do_not_leak() {
    let source = support::fixture("static_isinstance_ext.py");
    let dir = pycc_scratch::ScratchDir::new("static_isinstance_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("static_isinstance_ext"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    let driver = "import ctypes, io, contextlib\n\
                  with contextlib.redirect_stdout(io.StringIO()):\n    \
                  import static_isinstance_ext as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  def counter(name):\n    \
                  f = getattr(lib, name)\n    \
                  f.restype = ctypes.c_int64\n    \
                  f.argtypes = []\n    \
                  return f\n\
                  lists = counter('pycc_rt_list_live_objects')\n\
                  strs = counter('pycc_rt_str_live_objects')\n\
                  def delta(call):\n    \
                  call()\n    \
                  before = (lists(), strs())\n    \
                  for _ in range(200):\n        call()\n    \
                  return (lists() - before[0], strs() - before[1])\n\
                  for name, call in (\n\
                  ('fib_mod', lambda: m.fib_mod(50, 1000)),\n\
                  ('shapes', lambda: m.shapes(4)),\n\
                  ('head', lambda: m.head(3)),\n\
                  ):\n    \
                  d = delta(call)\n    \
                  assert d == (0, 0), '%s leaked %r lists/strs over 200 calls' % (name, d)\n\
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

/// Corpus 168: `range` objects kept in a variable, or in a list, and only
/// iterated compile as their `(start, stop, step)` triples
/// (`pycc_ast::desugar::range_values`); an `x is not None` test of a
/// parameter defaulting to `None` that every call passes folds
/// (`pycc_types::static_isinstance`).
#[test]
fn iterated_range_values_match_cpython() {
    assert_native_matches("range_values.py");
}

/// A range used any other way is still refused in strict mode: printed,
/// or made with a step that is not a nonzero literal (CPython raises
/// `ValueError` for a zero step when the range is made).
#[test]
fn other_range_values_are_still_refused() {
    for source in [
        "def _f(n: int) -> None:\n    d = range(n)\n    print(d)\n\n\n_f(3)\n",
        "def _f(n: int) -> int:\n    d = range(0, n, 0)\n    t = 0\n    for v in d:\n        t += v\n    return t\n\n\nprint(_f(3))\n",
    ] {
        let rendered = support::check_fails(source);
        assert!(rendered.contains("`range` is valid Python but not implemented yet"), "{rendered}");
    }
}

/// Replacing an element of a list of `int` tuples (`rows[0] = (a, b, c)`,
/// how a range list stores a range) releases the boxed tuple it overwrites:
/// it leaked one per store before. Checked against CPython too.
#[test]
fn replaced_tuple_elements_match_cpython() {
    assert_native_matches("tuple_rows.py");
}

#[cfg(not(target_os = "windows"))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn replaced_tuple_elements_do_not_leak() {
    let source = support::fixture("tuple_rows.py");
    let dir = pycc_scratch::ScratchDir::new("tuple_rows_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("tuple_rows"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    let driver = "import ctypes, io, contextlib\n\
                  with contextlib.redirect_stdout(io.StringIO()):\n    \
                  import tuple_rows as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  live = lib.pycc_rt_list_live_objects\n\
                  live.restype = ctypes.c_int64\n\
                  live.argtypes = []\n\
                  for name, call in (('replace_rows', lambda: m.replace_rows(7)), ('ranges', lambda: m.ranges(5))):\n    \
                  call()\n    \
                  before = live()\n    \
                  for _ in range(200):\n        call()\n    \
                  d = live() - before\n    \
                  assert d == 0, '%s leaked %d lists over 200 calls' % (name, d)\n\
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
