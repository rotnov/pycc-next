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
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "ok\n",
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
