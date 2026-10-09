//! End-to-end coverage for Part B of #1038 (#1064): the converted list, set
//! and float-formatting abort paths.
//!
//! Part A (#1063) converted the four `**` aborts; this file covers the
//! remaining six sites, all of which used to abort the process across a plain
//! `extern "C"` boundary and are now D-173 pending-exception raises:
//! `int_list_slice`'s three rejected bounds (`ValueError`), `int_list_pop` on
//! an empty list (`IndexError`), `float_to_str` outside the
//! `1e-4 <= |x| < 1e16` positional-notation window (`RuntimeError`), and
//! `check_set_len_unchanged` (`RuntimeError`, CPython's own
//! `Set changed size during iteration`).
//!
//! The set site is the one that also needed a codegen change: a D-173 raise
//! returns normally, so `MirStmt::ForSet`'s loop-continue condition gained an
//! `pycc_rt_exception_active() == 0` conjunct. Without it the loop spins
//! forever rather than reporting the error, because the length it compares
//! against is exactly the one the body just grew.
//!
//! Everything here goes through the public `pycc` CLI, mirroring
//! `tests/issue_1063_overflow_error.rs`'s harness. The `ext`-mode test is
//! `#[ignore]`d for the same reason every test in
//! `tests/issue_1050_ext_tuple.rs` is: it asks an installed CPython 3.13+ to
//! import a built artifact, which is a property of the machine. CI runs it on
//! every Tier-1 `native-build-test` leg through that job's
//! `cargo test --workspace -- --include-ignored`. It is deliberately not where
//! line coverage comes from -- the coverage job runs `llvm-cov` without
//! `--include-ignored`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("1064_{tag}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// `(exit_success, stdout, stderr)` of the built program.
fn build_and_run(tag: &str, source: &str) -> (bool, String, String) {
    let dir = scratch(tag);
    let src = write_fixture(&dir, &format!("{tag}.py"), source);
    let out = dir.join(tag);
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

/// All four converted runtime sites are catchable with an ordinary
/// `try`/`except` naming the CPython-conformant class, in one program. Every
/// one of them aborted the process with `SIGABRT` before this change, so the
/// `except` suites here could not have run at all.
///
/// The empty list is produced by `.pop()`-ing a one-element literal rather
/// than written as `xs = []`: an empty list literal's element type cannot be
/// inferred without an annotation (T0021, D-105), the same reason
/// `tests/container_methods1_codegen_depth.rs` gives.
#[test]
fn every_converted_abort_path_is_catchable_by_its_cpython_class() {
    let (ok, stdout, stderr) = build_and_run(
        "catchable",
        "xs = [1]\n\
         xs.pop()\n\
         try:\n\
         \x20   y = xs.pop()\n\
         except IndexError as e:\n\
         \x20   print(e)\n\
         neg = 0 - 1\n\
         try:\n\
         \x20   a = xs[neg:3]\n\
         except ValueError as e:\n\
         \x20   print(e)\n\
         try:\n\
         \x20   b = xs[0:neg]\n\
         except ValueError as e:\n\
         \x20   print(e)\n\
         try:\n\
         \x20   c = xs[0:3:0]\n\
         except ValueError as e:\n\
         \x20   print(e)\n\
         try:\n\
         \x20   d = 1e20 + 0.0\n\
         \x20   t = f\"{d}\"\n\
         except RuntimeError as e:\n\
         \x20   print(e)\n\
         s = {1}\n\
         try:\n\
         \x20   for x in s:\n\
         \x20       s.add(x + 1)\n\
         except RuntimeError as e:\n\
         \x20   print(e)\n\
         print(\"END\")\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "pop from empty list\n\
         slice start must be non-negative\n\
         slice stop must be non-negative\n\
         slice step must be positive\n\
         formatting a float this large or small (100000000000000000000) needs \
         scientific notation, which is not supported yet\n\
         Set changed size during iteration\n\
         END\n",
        "stderr: {stderr}"
    );
}

/// None of the six messages carries Part A's old `pycc_rt: ` panic prefix: a
/// raised exception's message is user-facing Python text, not a runtime
/// diagnostic. `int_list_get`'s already-converted `IndexError` is checked
/// alongside them as the established precedent.
#[test]
fn no_converted_message_carries_the_pycc_rt_panic_prefix() {
    let (ok, stdout, stderr) = build_and_run(
        "no_prefix",
        "xs = [1]\n\
         xs.pop()\n\
         try:\n\
         \x20   y = xs.pop()\n\
         except IndexError as e:\n\
         \x20   print(e)\n\
         try:\n\
         \x20   z = xs[5]\n\
         except IndexError as e:\n\
         \x20   print(e)\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert!(!stdout.contains("pycc_rt: "), "unexpected prefix: {stdout}");
    assert_eq!(stdout, "pop from empty list\nlist index out of range\n");
}

/// An uncaught raise from a converted site reports the CPython class name and
/// exits non-zero, rather than aborting with `SIGABRT` as it used to.
#[test]
fn an_uncaught_converted_raise_names_its_class_and_exits_non_zero() {
    let (ok, _stdout, stderr) = build_and_run(
        "uncaught",
        "xs = [1]\n\
         xs.pop()\n\
         y = xs.pop()\n\
         print(y)\n",
    );
    assert!(!ok, "the program should have exited non-zero");
    assert!(
        stderr.contains("IndexError: pop from empty list"),
        "unexpected stderr: {stderr}"
    );
}

/// Plan section C4, pinned as an *observed* result rather than a caveat: the
/// float sentinel is an empty `str`, and `print` writes it before the
/// exception reaches its checkpoint, so `print(1e20)` emits a bare newline on
/// stdout before the `RuntimeError` surfaces on stderr.
///
/// This is accepted, not unavoidable, and deliberately not guarded: a
/// `print`-side guard would be a half guard, because an f-string reaches
/// `pycc_rt_float_to_str` through the interpolation path instead. It is not a
/// new deviation either -- D-244's 2026-09-13 Part A amendment already accepts
/// exactly this "sentinel observable before the exception is reported" class,
/// and the pre-change behavior (a `SIGABRT` mid-`print`) was strictly worse.
#[test]
fn the_float_sentinel_is_printed_before_the_exception_is_reported() {
    let (ok, stdout, stderr) = build_and_run("sentinel_stdout", "print(1e20)\nprint(\"END\")\n");
    assert!(!ok, "the program should have exited non-zero");
    // The empty-string sentinel, then `print`'s own newline -- and nothing
    // else: `END` is never reached, because the raise propagates out.
    assert_eq!(stdout, "\n", "unexpected stdout: {stdout:?}");
    assert!(
        stderr.contains("RuntimeError: formatting a float this large or small"),
        "unexpected stderr: {stderr}"
    );
}

/// The same class of artefact for `list.pop()`, measured rather than assumed:
/// `.pop()` is not itself a D-173 checkpoint (that classification lives in
/// `expression_can_set_exception` and is explicitly out of scope here), so the
/// `tag_smallint(0)` sentinel is printed by the *next* statement before the
/// enclosing suite's own checkpoint observes the pending `IndexError`.
///
/// The sentinel being a valid encoded word is exactly what makes this safe:
/// raw `0` is not a valid D-141 word and `classify_encoded_int` would panic on
/// it, which is why every converted int-returning site returns
/// `tag_smallint(0)` and never a bare `0`.
#[test]
fn the_pop_sentinel_is_observable_before_its_checkpoint() {
    let (ok, stdout, stderr) = build_and_run(
        "sentinel_pop",
        "xs = [1]\n\
         xs.pop()\n\
         try:\n\
         \x20   y = xs.pop()\n\
         \x20   print(y)\n\
         except IndexError as e:\n\
         \x20   print(e)\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "0\npop from empty list\n");
}

/// Plan section 3(a-bis): the `ForSet` loop-test guard reads the pending flag,
/// not the resize check's own result, so a loop whose *body* raises also exits
/// at the next test rather than running to the collection's end. That is
/// strictly more correct under D-173 and is the reason the guard was chosen
/// over widening `pycc_rt_int_set_check_not_resized` to return a status.
///
/// The set has three elements and the body raises on the first iteration, so
/// an unguarded loop would report `3`.
#[test]
fn a_for_set_loop_whose_body_raises_exits_at_the_next_test() {
    let (ok, stdout, stderr) = build_and_run(
        "early_exit",
        "def early() -> int:\n\
         \x20   s = {1, 2, 3}\n\
         \x20   n = 0\n\
         \x20   try:\n\
         \x20       for x in s:\n\
         \x20           n = n + 1\n\
         \x20           xs = [1]\n\
         \x20           xs.pop()\n\
         \x20           xs.pop()\n\
         \x20   except IndexError:\n\
         \x20       return n\n\
         \x20   return 99\n\n\n\
         print(early())\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "1\n");
}

/// The guard's soundness condition, pinned because it is the shape that would
/// silently break it later: pending exception state is never legitimately live
/// on entry to a loop test, so the guard cannot cut a loop short spuriously.
/// `crates/pycc_codegen/src/exception.rs` clears the pending state before an
/// `except` handler body runs and before a `finally` body runs (restoring it
/// afterwards), so a `for` loop in either position iterates its set in full.
///
/// Both functions would return `0` instead of `3` if that were not so.
#[test]
fn a_for_set_loop_inside_a_handler_or_finally_body_iterates_in_full() {
    let (ok, stdout, stderr) = build_and_run(
        "pending_bodies",
        "def in_handler() -> int:\n\
         \x20   s = {1, 2, 3}\n\
         \x20   n = 0\n\
         \x20   try:\n\
         \x20       raise ValueError(\"boom\")\n\
         \x20   except ValueError:\n\
         \x20       for x in s:\n\
         \x20           n = n + 1\n\
         \x20   return n\n\n\
         def in_finally() -> int:\n\
         \x20   s = {1, 2, 3}\n\
         \x20   n = 0\n\
         \x20   try:\n\
         \x20       n = n + 0\n\
         \x20   finally:\n\
         \x20       for x in s:\n\
         \x20           n = n + 1\n\
         \x20   return n\n\n\n\
         print(in_handler())\n\
         print(in_finally())\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "3\n3\n");
}

/// The set loop terminates and leaves the set in the state the aborted
/// iteration produced, rather than looping forever. This is the test the
/// codegen guard exists for: with the runtime converted and the guard absent,
/// this program hangs instead of failing, which is why plan item C1a requires
/// both to land in one commit.
#[test]
fn growing_a_set_during_its_own_iteration_terminates_with_a_runtime_error() {
    let (ok, stdout, stderr) = build_and_run(
        "set_grow",
        "s = {1}\n\
         try:\n\
         \x20   for x in s:\n\
         \x20       s.add(x + 1)\n\
         except RuntimeError as e:\n\
         \x20   print(e)\n\
         print(len(s))\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "Set changed size during iteration\n2\n");
}

/// Review round 2 of #1064: a body that both *grows* the set and raises. The
/// resize check is emitted before the loop test reads the pending flag, and
/// `pycc_rt_exception_raise` replaces the thread-local pending value
/// unconditionally, so without `check_set_len_unchanged`'s own
/// already-pending early return this program selects the `RuntimeError`
/// handler where CPython selects `IndexError`. Measured against CPython
/// 3.14.x: `IndexError`.
///
/// This is the gap `a_for_set_loop_whose_body_raises_exits_at_the_next_test`
/// leaves open -- its body raises but never grows the set, so the resize check
/// never fires on top of the body's own pending exception.
#[test]
fn a_for_set_body_that_grows_the_set_and_raises_keeps_its_own_exception() {
    let (ok, stdout, stderr) = build_and_run(
        "grow_and_raise",
        "def both() -> str:\n\
         \x20   s = {1}\n\
         \x20   try:\n\
         \x20       for x in s:\n\
         \x20           s.add(x + 1)\n\
         \x20           xs = [1]\n\
         \x20           xs.pop()\n\
         \x20           xs.pop()\n\
         \x20   except IndexError:\n\
         \x20       return \"IndexError\"\n\
         \x20   except RuntimeError:\n\
         \x20       return \"RuntimeError\"\n\
         \x20   return \"none\"\n\n\n\
         print(both())\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "IndexError\n");
}

/// Review round 3 of #1064: the loop's *exceptional* exit edge must not fall
/// through into the statement after the loop. `for_set_after` is now reached
/// by two edges -- ran out of elements, and the pending-exception conjunct --
/// so it ends in `guard_statement_effects` like every other fallible
/// statement. Pre-fix this program printed `BAD` before the `RuntimeError`
/// reached its handler; CPython prints neither.
///
/// Both arms are asserted in one program so the guard cannot be satisfied by
/// suppressing the normal path too: `quiet()`'s loop raises nothing and must
/// still reach its own trailing `print`.
#[test]
fn the_statement_after_a_raising_for_set_loop_does_not_run() {
    let (ok, stdout, stderr) = build_and_run(
        "after_block_guard",
        "def grow() -> None:\n\
         \x20   s = {1, 2, 3}\n\
         \x20   for x in s:\n\
         \x20       s.add(x + 100)\n\
         \x20   print(\"BAD\")\n\n\n\
         def quiet() -> None:\n\
         \x20   s = {1, 2, 3}\n\
         \x20   for x in s:\n\
         \x20       y = x + 1\n\
         \x20   print(\"AFTER\")\n\n\n\
         quiet()\n\
         try:\n\
         \x20   grow()\n\
         except RuntimeError:\n\
         \x20   print(\"caught\")\n\
         print(\"END\")\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "AFTER\ncaught\nEND\n");
}

/// Review round 2 of #1064, thread 2: the accepted D-173 sentinel residual in
/// its *assignment* direction. Neither `MirExpr::Slice` nor `MirExpr::ListPop`
/// is a `pycc_codegen::exception::expression_can_set_exception` checkpoint, so
/// each commits its sentinel to the target name before the enclosing suite's
/// checkpoint observes the pending exception -- a binding CPython would have
/// left at its previous value. Both arms are pinned together so a future
/// half-fix that classifies only one of them fails here: the class belongs to
/// the classifier, not to either expression, and `MirExpr::FString` reaches
/// `float_to_str` with the same shape.
///
/// Widening `expression_can_set_exception` is out of scope for #1064 (plan
/// section 9) -- it is pinned by
/// `exception_guard_classification_keeps_the_arithmetic_fast_path_clean`
/// because `nbody.py` evaluates ten `Pow` nodes per hot-loop iteration under
/// the D-084/D-095/D-140 speedup floors. The residual closes with #1031.
#[test]
fn a_rejected_slice_and_an_empty_pop_both_commit_their_sentinel_to_the_target() {
    let (ok, stdout, stderr) = build_and_run(
        "sentinel_binding",
        "xs = [9, 9, 9]\n\
         result = [9, 9, 9]\n\
         try:\n\
         \x20   result = xs[-1:1]\n\
         except ValueError as e:\n\
         \x20   print(e)\n\
         print(len(result))\n\
         empty = [1]\n\
         empty.pop()\n\
         y = 7\n\
         try:\n\
         \x20   y = empty.pop()\n\
         except IndexError as e:\n\
         \x20   print(e)\n\
         print(y)\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "slice start must be non-negative\n0\npop from empty list\n0\n"
    );
}

/// The same six raises seen from a CPython host through an `ext`-mode artifact
/// (D-244), which is the mode where being catchable actually matters: the host
/// interpreter, not pycc, owns the process, so a `SIGABRT` there took the whole
/// interpreter down.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_converted_raises_reach_a_cpython_host_as_catchable_exceptions() {
    let dir = ScratchDir::new("1064_ext").expect("scratch");
    let src = write_fixture(
        &dir,
        "m.py",
        "def pop_empty() -> int:\n\
         \x20   xs = [1]\n\
         \x20   xs.pop()\n\
         \x20   return xs.pop()\n\n\
         def bad_slice(start: int, stop: int, step: int) -> int:\n\
         \x20   xs = [1, 2, 3]\n\
         \x20   ys = xs[start:stop:step]\n\
         \x20   return len(ys)\n\n\
         def big_float() -> str:\n\
         \x20   d = 1e20 + 0.0\n\
         \x20   return f\"{d}\"\n\n\
         def grow_during_iteration() -> int:\n\
         \x20   s = {1}\n\
         \x20   for x in s:\n\
         \x20       s.add(x + 1)\n\
         \x20   return len(s)\n\n\
         def ok_slice() -> int:\n\
         \x20   xs = [1, 2, 3]\n\
         \x20   return len(xs[0:2])\n",
    );
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_1064_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let script = "import pycc_1064_mod as m\n\
         try:\n\
         \x20   m.pop_empty()\n\
         \x20   raise AssertionError('expected IndexError')\n\
         except IndexError as e:\n\
         \x20   assert str(e) == 'pop from empty list', str(e)\n\
         for args, msg in (\n\
         \x20   ((-1, 3, 1), 'slice start must be non-negative'),\n\
         \x20   ((0, -1, 1), 'slice stop must be non-negative'),\n\
         \x20   ((0, 3, 0), 'slice step must be positive'),\n\
         ):\n\
         \x20   try:\n\
         \x20       m.bad_slice(*args)\n\
         \x20       raise AssertionError('expected ValueError for %r' % (args,))\n\
         \x20   except ValueError as e:\n\
         \x20       assert str(e) == msg, str(e)\n\
         try:\n\
         \x20   m.big_float()\n\
         \x20   raise AssertionError('expected RuntimeError')\n\
         except RuntimeError as e:\n\
         \x20   assert 'scientific notation' in str(e), str(e)\n\
         try:\n\
         \x20   m.grow_during_iteration()\n\
         \x20   raise AssertionError('expected RuntimeError')\n\
         except RuntimeError as e:\n\
         \x20   assert str(e) == 'Set changed size during iteration', str(e)\n\
         # A conforming call still returns normally afterwards, so none of the\n\
         # wrappers left pending state behind.\n\
         assert m.ok_slice() == 2\n";
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
