//! The `str` half of the `ext` boundary against a real CPython (#1049, Part 2
//! of #1037).
//!
//! Every test here is `#[ignore]`d: each builds an artifact and asks an
//! installed CPython 3.13+ to import it, which is a property of the machine
//! rather than of the change under test. CI runs them on every Tier-1
//! `native-build-test` leg through that job's `cargo test --workspace --
//! --include-ignored`.
//!
//! They are deliberately *not* where line coverage comes from -- the coverage
//! job runs `llvm-cov` without `--include-ignored`, so an ignored test earns
//! none, and the two new helpers live in C, which `llvm-cov` does not
//! instrument at all. `src/ext_build_tests/generated_c.rs` covers the emitted
//! text and the shim's own source; this file covers what only a loaded
//! artifact can show: that `PyUnicode_AsUTF8AndSize` and
//! `PyUnicode_FromStringAndSize` really do round-trip a pycc `str` across the
//! unchecked `void *fnptr_` cast, and that D-244 rule 7's admissibility
//! matrix plus the 2026-09-13 amendment hold at the wrapper.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Builds `body` as an extension module named `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

/// Runs `script` with the built module importable from `dir`.
fn run_python(dir: &Path, script: &str) {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}

/// The conforming calls. Both payload representations are exercised on
/// purpose: D-059 splits `PyStrObj` at 22 bytes, and the shim reads through
/// one accessor that has to see past that split. The empty string and an
/// embedded NUL are here because the boundary carries an explicit length in
/// both directions rather than re-deriving one with `strlen` -- a `strlen`
/// anywhere on the path truncates `"a\0b"` to `"a"` in silence.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_round_trips_str_through_both_payload_representations() {
    let dir = ScratchDir::new("ext_str_ok").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_mod",
        "def echo(s: str) -> str:\n    return s\n\n\
         def greet(who: str) -> str:\n    return \"hello, \" + who\n\n\
         def join(a: str, b: str) -> str:\n    return a + b\n\n\
         def sink(s: str) -> int:\n    return 1\n\n\
         def make() -> str:\n    return \"from the module body\"\n",
    );
    run_python(
        &dir,
        "import pycc_str_mod as m\n\
         assert m.echo('') == ''\n\
         assert m.echo('hi') == 'hi'\n\
         assert m.echo('a\\0b') == 'a\\0b', repr(m.echo('a\\0b'))\n\
         long = 'x' * 23\n\
         assert m.echo(long) == long\n\
         assert m.echo('\\u00e9\\u4e2d\\U0001f600') == '\\u00e9\\u4e2d\\U0001f600'\n\
         assert m.greet('world') == 'hello, world'\n\
         assert m.join('ab', 'cd') == 'abcd'\n\
         assert m.join('', '') == ''\n\
         assert m.sink('anything') == 1\n\
         assert m.make() == 'from the module body'\n\
         assert type(m.echo('hi')) is str\n\
         print('ok')\n",
    );
}

/// The boundary carries `str` *values*, not objects, so identity does not
/// survive a crossing even for an exact `str` -- a stronger statement than
/// the `int`/`float` subclass case #1043 was opened for, and one CPython's
/// own `def echo(s): return s` does not share. A `str` subclass is accepted
/// and flattened for the same reason.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_str_boundary_copies_rather_than_preserving_identity() {
    let dir = ScratchDir::new("ext_str_identity").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_identity",
        "def echo(s: str) -> str:\n    return s\n",
    );
    run_python(
        &dir,
        "import pycc_str_identity as m\n\
         s = 'a moderately long string that no interpreter interns'\n\
         assert m.echo(s) == s\n\
         assert m.echo(s) is not s, 'identity is documented not to survive (#1043)'\n\
         class S(str):\n    pass\n\
         sub = S('subclassed')\n\
         out = m.echo(sub)\n\
         assert out == 'subclassed'\n\
         assert type(out) is str, type(out)\n\
         print('ok')\n",
    );
}

/// The closed half of rule 7. `PyUnicode_Check` and no converter fallback,
/// so nothing that merely knows how to become a `str` is admitted:
/// `docs/TYPE_SYSTEM.md` rule 4 (D-086) forbids implicit conversion at an
/// annotated boundary in either direction. `bytes` is the case a C-API
/// converter habit would most plausibly let through.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_str_parameter_refuses_every_object_that_is_not_a_str() {
    let dir = ScratchDir::new("ext_str_typeerror").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_refuse",
        "def echo(s: str) -> str:\n    return s\n\n\
         def echo_int(x: int) -> int:\n    return x\n",
    );
    run_python(
        &dir,
        // Every line of the script carries its own indentation inside the
        // literal: Rust's `\`-continuation strips the leading whitespace of
        // the next source line, so borrowed indentation would not survive.
        "import pathlib\n\
         import pycc_str_refuse as m\n\
         class Stringy:\n    def __str__(self):\n        return 'nope'\n\
         cases = [\n\
         (lambda: m.echo(b'bytes'), 'bytes at a str parameter'),\n\
         (lambda: m.echo(1), 'int at a str parameter'),\n\
         (lambda: m.echo(None), 'None at a str parameter'),\n\
         (lambda: m.echo(Stringy()), '__str__ duck type at a str parameter'),\n\
         (lambda: m.echo(pathlib.Path('p')), 'PathLike at a str parameter'),\n\
         (lambda: m.echo_int('1'), 'str at an int parameter'),\n\
         ]\n\
         for call, what in cases:\n    try:\n        call()\n    \
         except TypeError as e:\n        assert 'echo' in str(e), str(e)\n    \
         else:\n        raise AssertionError(what + ' must raise TypeError')\n\
         try:\n    m.echo()\n\
         except TypeError:\n    pass\n\
         else:\n    raise AssertionError('arity must be checked')\n\
         print('ok')\n",
    );
}

/// The D-244 amendment of 2026-09-13. A lone surrogate is a legal CPython
/// `str` with no UTF-8 encoding, so `PyUnicode_AsUTF8AndSize` refuses it;
/// the resulting `UnicodeEncodeError` is propagated verbatim rather than
/// translated into rule 7's `TypeError`, because the condition is a property
/// of the value and not of the annotation.
///
/// The second half is the error-path cleanup this change owed: when a later
/// argument is refused, every `str` already unpacked is released before the
/// wrapper bails. The loop cannot observe the refcount from Python, so it
/// stands as a crash/liveness assertion -- a double free or a use-after-free
/// in that branch shows up here rather than in the generated text.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_lone_surrogate_raises_cpythons_own_error_and_leaves_earlier_arguments_released() {
    let dir = ScratchDir::new("ext_str_surrogate").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_surrogate",
        "def join(a: str, b: str) -> str:\n    return a + b\n",
    );
    run_python(
        &dir,
        "import pycc_str_surrogate as m\n\
         try:\n    m.join('ok', '\\ud800')\n\
         except UnicodeEncodeError:\n    pass\n\
         else:\n    raise AssertionError('a lone surrogate must not be encodable')\n\
         for _ in range(2000):\n    \
         try:\n        m.join('first argument', 2)\n    \
         except TypeError:\n        pass\n    \
         else:\n        raise AssertionError('the second argument must be refused')\n\
         assert m.join('still', ' alive') == 'still alive'\n\
         print('ok')\n",
    );
}

/// A raised exception must reach Python from a `-> str` export too: the
/// wrapper's pending-flag check runs before the egress, and the carrier on
/// that branch is a null pointer holding no reference, so nothing is packed
/// and nothing is released.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_str_returning_export_raises_rather_than_packing_the_neutral_carrier() {
    let dir = ScratchDir::new("ext_str_raise").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_raise",
        "def pick(n: int) -> str:\n    if n == 0:\n        raise ValueError(\"zero\")\n    return \"nonzero\"\n",
    );
    run_python(
        &dir,
        "import pycc_str_raise as m\n\
         assert m.pick(1) == 'nonzero'\n\
         try:\n    m.pick(0)\n\
         except ValueError as e:\n    assert str(e) == 'zero', str(e)\n\
         else:\n    raise AssertionError('the exception did not cross the boundary')\n\
         assert m.pick(2) == 'nonzero'\n\
         print('ok')\n",
    );
}
