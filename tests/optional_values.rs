//! `T | None` values (v0.6 accept 5): pointer-represented optionals (`str`,
//! containers) as locals, returns and parameters; lists and dicts holding
//! `int | None` or `str | None` elements (`[None] * n` then int stores,
//! `["a", None]`); `==`/`!=` against a value; and `while x is not None:` /
//! truthiness narrowing. Each fixture is built in strict mode, with
//! `--non-strict` and as an `--ext` module, and every run is compared with
//! the live CPython oracle (`PYCC_PYTHON`, default `python3`).

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

/// Every shape the corpus programs use, with CPython's output and a
/// `KeyError` from a dict of optional values.
#[test]
fn optional_values_match_cpython() {
    assert_native_matches("optional_values.py");
    assert_ext_matches("optional_values.py", "import optional_values");
}

/// An optional element stored, overwritten, read, compared and printed in
/// a loop releases what it held: the runtime's live list, `str` and dict
/// counts are flat across the hosted loop.
#[test]
fn optional_elements_do_not_leak() {
    let source = support::fixture("optional_leaks.py");
    let dir = pycc_scratch::ScratchDir::new("optional_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("optional_leaks"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    // Every line carries its own indentation inside the literal: Rust's
    // `\`-continuation strips the next source line's leading whitespace.
    let driver = "import ctypes\n\
                  import optional_leaks as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  def counter(name):\n    \
                  f = getattr(lib, name)\n    \
                  f.restype = ctypes.c_int64\n    \
                  f.argtypes = []\n    \
                  return f\n\
                  counters = [counter('pycc_rt_%s_live_objects' % k) for k in ('list', 'str', 'dict')]\n\
                  def live():\n    \
                  return [c() for c in counters]\n\
                  m.churn(6)\n\
                  before = live()\n\
                  for _ in range(200):\n    \
                  m.churn(6)\n\
                  after = live()\n\
                  assert after == before, 'leaked: %r -> %r' % (before, after)\n\
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

/// Strict mode keeps an unnarrowed `int | None` where an `int` is required
/// a compile error.
#[test]
fn an_unnarrowed_optional_element_is_still_refused() {
    let rendered = support::check_fails(
        "def f(n: int) -> int:\n    xs = [None] * n\n    xs[0] = 1\n    return xs[0] + 1\n\nprint(f(2))\n",
    );
    assert!(rendered.contains("T0021"), "{rendered}");
}
