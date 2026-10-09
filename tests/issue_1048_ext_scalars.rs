//! The widened `ext` scalar boundary against a real CPython (#1048, Part 1
//! of #1037).
//!
//! Every test here is `#[ignore]`d: each builds an artifact and asks an
//! installed CPython 3.13+ to import it, which is a property of the machine
//! rather than of the change under test. CI runs them on every Tier-1
//! `native-build-test` leg through that job's `cargo test --workspace --
//! --include-ignored`.
//!
//! They are deliberately *not* where the generator's line coverage comes
//! from -- the coverage job runs `llvm-cov` without `--include-ignored`, so
//! an ignored test earns none. `src/ext_build_tests/generated_c.rs` covers
//! the emitted text; this file covers what only a loaded artifact can show:
//! that the generated C compiles, that each C type agrees with the compiled
//! function's own ABI slot across the unchecked `void *fnptr_` cast, and
//! that D-244 rule 7's admissibility matrix holds at the wrapper.

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

/// Every conforming call in D-244 rule 7's widened matrix, in one build so
/// they cost one compile between them: a `float` round-trip that would come
/// back truncated if any one of the wrapper's slots were still `long long`,
/// a `bool` round-trip that must return the interned singleton, a `-> None`
/// export, and the mixed signature where each slot has a different width.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_round_trips_float_bool_and_none_through_the_boundary() {
    let dir = ScratchDir::new("ext_scalars_ok").expect("scratch");
    build_ext(
        &dir,
        "pycc_scalars_mod",
        "def echo_float(x: float) -> float:\n    return x\n\n\
         def negate(b: bool) -> bool:\n    if b:\n        return False\n    return True\n\n\
         def sink(x: int) -> None:\n    return None\n\n\
         def twice(x: int) -> int:\n    return x * 2\n\n\
         def blend(n: int, f: float, b: bool) -> float:\n    if b:\n        return f + 0.5\n    return f\n",
    );
    run_python(
        &dir,
        "import pycc_scalars_mod as m\n\
         assert m.echo_float(2.5) == 2.5, m.echo_float(2.5)\n\
         assert m.echo_float(-0.75) == -0.75, m.echo_float(-0.75)\n\
         assert m.negate(True) is False, m.negate(True)\n\
         assert m.negate(False) is True, m.negate(False)\n\
         assert m.sink(3) is None, m.sink(3)\n\
         assert m.twice(True) == 2, m.twice(True)\n\
         assert m.blend(1, 2.5, True) == 3.0, m.blend(1, 2.5, True)\n\
         assert m.blend(1, 2.5, False) == 2.5, m.blend(1, 2.5, False)\n\
         print('ok')\n",
    );
}

/// The non-conforming half of the same matrix. `float` refuses an `int` and
/// a `bool`, and `bool` refuses an `int` and a `float`: `docs/TYPE_SYSTEM.md`
/// rule 4 (D-086) forbids implicit numeric widening as well as narrowing at
/// an annotated boundary, which is a deliberate divergence from
/// `PyFloat_AsDouble` and from every C-API converter's habit. The `int`
/// parameter's own acceptance of `bool` is unchanged and asserted alongside,
/// because subtyping is one-directional and the pair is what makes that
/// visible.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_widened_boundary_refuses_every_implicit_numeric_conversion() {
    let dir = ScratchDir::new("ext_scalars_typeerror").expect("scratch");
    build_ext(
        &dir,
        "pycc_scalars_refuse",
        "def echo_float(x: float) -> float:\n    return x\n\n\
         def echo_bool(b: bool) -> bool:\n    return b\n\n\
         def echo_int(x: int) -> int:\n    return x\n",
    );
    run_python(
        &dir,
        // Every line of the script carries its own indentation inside the
        // literal: Rust's `\`-continuation strips the leading whitespace of
        // the next source line, so borrowed indentation would not survive.
        "import pycc_scalars_refuse as m\n\
         cases = [\n\
         (lambda: m.echo_float(1), 'int at a float parameter'),\n\
         (lambda: m.echo_float(True), 'bool at a float parameter'),\n\
         (lambda: m.echo_float('2.5'), 'str at a float parameter'),\n\
         (lambda: m.echo_bool(1), 'int at a bool parameter'),\n\
         (lambda: m.echo_bool(1.0), 'float at a bool parameter'),\n\
         ]\n\
         for call, what in cases:\n    try:\n        call()\n    \
         except TypeError:\n        pass\n    else:\n        \
         raise AssertionError(what + ' must raise TypeError')\n\
         assert m.echo_int(True) == 1, m.echo_int(True)\n\
         assert m.echo_bool(True) is True\n\
         print('ok')\n",
    );
}

/// A raised exception must reach Python from a `-> None` export too. That
/// arm has no result to inspect, so the wrapper's pending-flag check is the
/// only thing standing between the exception and a fabricated `None`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_none_returning_export_raises_rather_than_returning_a_fabricated_none() {
    let dir = ScratchDir::new("ext_scalars_raise").expect("scratch");
    build_ext(
        &dir,
        "pycc_scalars_raise",
        "def check(n: int) -> None:\n    if n == 0:\n        raise ValueError(\"zero\")\n",
    );
    run_python(
        &dir,
        "import pycc_scalars_raise as m\n\
         assert m.check(1) is None\n\
         try:\n    m.check(0)\n\
         except ValueError as e:\n    assert str(e) == 'zero', str(e)\n\
         else:\n    raise AssertionError('the exception did not cross the boundary')\n\
         print('ok')\n",
    );
}
