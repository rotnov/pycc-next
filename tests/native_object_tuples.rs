//! Native containers of objects (ROADMAP v0.5, accept item 1): variadic
//! tuples `tuple[T, ...]` held natively, and native tuples whose elements may
//! be CPython objects (`tuple[int, Any]`).
//!
//! Every program is compared with CPython on the same source: natively in
//! strict mode, natively with `--non-strict`, and as an `--ext` module
//! imported by a driver that runs the same calls against the source. The
//! `--ext` arms need a CPython with development headers and are `#[ignore]`d
//! like their siblings (`tests/issue_1050_ext_tuple.rs`); CI runs them with
//! `--include-ignored`. The lifetime fixtures also run under valgrind's
//! memcheck with full leak checking when valgrind is installed, and a peak
//! resident set probe proves the tuple operations do not leak per iteration.

#[cfg(unix)]
#[path = "memory_support/mod.rs"]
mod memory_support;

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn succeeded(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout: {}\nstderr: {}",
        text(&pycc_scratch::lf(&output.stdout)),
        text(&pycc_scratch::lf(&output.stderr))
    );
}

/// Builds `src` into `dir/prog` with `flags`.
fn build(dir: &Path, src: &Path, flags: &[&str]) -> PathBuf {
    let bin = dir.join("prog");
    let output = pycc()
        .arg("build")
        .arg(src)
        .arg("-o")
        .arg(&bin)
        .args(flags)
        .output()
        .expect("pycc should spawn");
    succeeded(&output, "pycc build");
    bin
}

/// Runs `bin` under memcheck with full leak checking: no invalid access and
/// no definitely lost block. Skipped (with a note) without valgrind.
fn assert_memcheck_clean(bin: &Path) {
    let Ok(output) = Command::new("valgrind")
        .args([
            "-q",
            "--error-exitcode=99",
            "--leak-check=full",
            "--show-leak-kinds=definite",
            "--errors-for-leak-kinds=definite",
        ])
        .arg(bin)
        .output()
    else {
        eprintln!("valgrind not installed: memcheck skipped");
        return;
    };
    succeeded(&output, "memcheck");
}

/// Builds `name` with `flags`, runs it, and compares its stdout with
/// CPython's run of the same file; then memchecks the program.
fn assert_native_matches_cpython(name: &str, flags: &[&str]) {
    let theirs = python().arg(fixture(name)).output().expect("python3 runs");
    succeeded(&theirs, "python3");
    let dir = ScratchDir::new("native_object_tuples").expect("scratch");
    let bin = build(&dir, &fixture(name), flags);
    let ours = Command::new(&bin).output().expect("the program runs");
    succeeded(&ours, "the compiled program");
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&theirs.stdout)),
        "{name} {flags:?}: stdout differs from CPython"
    );
    assert_memcheck_clean(&bin);
}

#[test]
fn variadic_tuples_match_cpython_in_strict_mode() {
    assert_native_matches_cpython("var_tuple.py", &[]);
}

#[test]
fn variadic_tuples_match_cpython_in_non_strict_mode() {
    assert_native_matches_cpython("var_tuple.py", &["--non-strict"]);
}

#[test]
fn tuples_holding_objects_match_cpython_in_non_strict_mode() {
    assert_native_matches_cpython("object_tuples.py", &["--non-strict"]);
}

/// Tuples of every pooled length and longer, more of them (and of strings)
/// alive at once than a free list holds, freed and rebuilt in place: the
/// recycled blocks (`crates/pycc_rt/src/pool.rs`) never disturb a live value,
/// and memcheck sees no invalid access and no lost block.
#[test]
fn recycled_tuple_and_str_blocks_match_cpython() {
    assert_native_matches_cpython("tuple_pools.py", &[]);
    assert_native_matches_cpython("tuple_pools.py", &["--non-strict"]);
}

/// Builds `module` as an `--ext` extension module into one directory and
/// copies its source into another, then runs `driver` against each: the
/// two outputs must be the same.
fn assert_ext_matches_source(module: &str, driver: &str, flags: &[&str]) {
    let dir = ScratchDir::new("native_object_tuples_ext").expect("scratch");
    let built = dir.join("built");
    let source = dir.join("source");
    std::fs::create_dir_all(&built).expect("mkdir");
    std::fs::create_dir_all(&source).expect("mkdir");
    let src = fixture(&format!("{module}.py"));
    std::fs::copy(&src, source.join(format!("{module}.py"))).expect("copy the source");
    let output = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(built.join(module))
        .arg("--ext")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    succeeded(&output, "pycc build --ext");
    // `-I`: the driver's own directory (`tests/fixtures`, which holds the
    // module's source too) must not be on `sys.path`.
    let run = |at: &Path| {
        let output = python()
            .arg("-I")
            .arg(fixture(driver))
            .arg(at)
            .output()
            .expect("python3 runs");
        succeeded(&output, driver);
        text(&pycc_scratch::lf(&output.stdout))
    };
    assert_eq!(run(&built), run(&source), "{module}: --ext differs from CPython");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn variadic_str_tuples_cross_the_ext_boundary_like_cpython_tuples() {
    assert_ext_matches_source("var_tuple_ext", "var_tuple_ext_driver.py", &[]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn tuples_holding_objects_cross_the_ext_boundary_with_their_aliases() {
    assert_ext_matches_source(
        "object_tuples",
        "object_tuples_ext_driver.py",
        &["--non-strict"],
    );
}

/// A strict `tuple[str, ...]` parameter accepts only a `tuple` of `str`,
/// as a strict `str` parameter accepts only a `str` (docs/DEVIATIONS.md).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_strict_variadic_tuple_parameter_refuses_other_values_with_type_error() {
    let dir = ScratchDir::new("native_object_tuples_refuse").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(fixture("var_tuple_ext.py"))
        .arg("-o")
        .arg(dir.join("var_tuple_ext"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    succeeded(&output, "pycc build --ext");
    let run = python()
        .arg("-I")
        .arg("-c")
        .arg(
            "import sys\nsys.path.insert(0, sys.argv[1])\nimport var_tuple_ext as m\n\
             for bad in (['a'], ('a', 1), 'ab'):\n\
             \x20   try:\n\
             \x20       m.stem(bad)\n\
             \x20   except TypeError as e:\n\
             \x20       print(e)\n\
             try:\n\
             \x20   m.Path([1])\n\
             except TypeError as e:\n\
             \x20   print(e)\n",
        )
        .arg(&*dir)
        .output()
        .expect("python3 runs");
    succeeded(&run, "the refusal driver");
    assert_eq!(
        text(&pycc_scratch::lf(&run.stdout)),
        "stem() argument 1 must be tuple, not list\n\
         stem() argument 1 must be a tuple of str, not one holding int\n\
         stem() argument 1 must be tuple, not str\n\
         Path.__init__() argument 1 must be tuple, not list\n"
    );
}

/// `pycc check`'s rendered output for `source`, which must fail.
fn check_fails(source: &str) -> String {
    let dir = ScratchDir::new("native_object_tuples_check").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the subject");
    let output = pycc().arg("check").arg(&src).output().expect("pycc should spawn");
    assert!(!output.status.success(), "`pycc check` accepted:\n{source}");
    format!("{}{}", text(&pycc_scratch::lf(&output.stdout)), text(&pycc_scratch::lf(&output.stderr)))
}

/// A variadic tuple of `int`/`float`/`bool` is not native yet (those
/// tuples are held by value, and a variadic one would need converting at
/// every assignment): strict mode refuses it with a spanned `T0053`.
#[test]
fn a_variadic_tuple_of_scalars_is_refused_in_strict_mode() {
    let rendered = check_fails("def f(t: tuple[int, ...]) -> int:\n    return len(t)\n");
    assert!(rendered.contains("T0053"), "{rendered}");
    assert!(rendered.contains("m.py:1"), "{rendered}");
}

/// `tuple[int, Any]` is an interop type: strict code cannot spell `Any`.
#[test]
fn a_tuple_holding_any_is_refused_in_strict_mode() {
    let rendered = check_fails(
        "from typing import Any\n\n\ndef f(x: int) -> tuple[int, Any]:\n    return x, None\n",
    );
    assert!(rendered.contains("`Any` is not permitted"), "{rendered}");
}

#[cfg(unix)]
mod peak_rss {
    use super::ScratchDir;
    use crate::memory_support::{GROWTH_LIMIT_KB, build_with, run_measured};

    /// Slices, concatenation, repetition, hashing, membership, dict keys
    /// and a tuple returned through an object slot, every iteration.
    fn probe(iterations: u32, flags: &[&str]) -> i64 {
        let source = format!(
            "from typing import Any\n\n\n\
             def pair(i: int) -> tuple[int, Any]:\n\
             \x20   key: tuple[str, ...] = (str(i), 'x')\n\
             \x20   return i, key\n\n\n\
             def main() -> None:\n\
             \x20   seen: dict[tuple[str, ...], int] = {{}}\n\
             \x20   total = 0\n\
             \x20   for i in range({iterations}):\n\
             \x20       key: tuple[str, ...] = ('a', str(i % 7))\n\
             \x20       key += (str(i % 3),)\n\
             \x20       parent = key[:-1] * 2\n\
             \x20       if 'a' in parent:\n\
             \x20           total += len(parent)\n\
             \x20       seen[key] = hash(key) % 5\n\
             \x20       n, value = pair(i)\n\
             \x20       total += n - i\n\
             \x20   print(total, len(seen))\n\n\n\
             main()\n"
        );
        let dir = ScratchDir::new("native_object_tuples_rss").expect("scratch");
        let bin = build_with(&dir, "probe", &source, flags);
        run_measured(&bin).1
    }

    #[test]
    fn tuple_operations_do_not_grow_with_the_trip_count() {
        let small = probe(20_000, &["--non-strict"]);
        let large = probe(220_000, &["--non-strict"]);
        assert!(
            large - small < GROWTH_LIMIT_KB,
            "peak grew {} kB from 20 000 to 220 000 iterations ({small} -> {large})",
            large - small
        );
    }
}
