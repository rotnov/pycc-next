//! Issue #638 (D-208): public-CLI coverage for releasing a heap-bigint's
//! birth reference on the D-173 exception-unwinding edge.
//!
//! D-181 (#625) closed the normal-path leak of a nested `int` arithmetic
//! temporary but left two residual leak flavors, both closed by this issue.
//! [#834](https://github.com/rotnov/pycc/issues/834) (D-212) later extends
//! this same file with a third, distinct flavor: not an owning temporary at
//! all, but `retain_if_int_duplicate`'s own extra retain of a
//! *duplicate/borrowed* `int` at the `TupleLiteral`-element and `Call`
//! -argument sites below, abandoned the same way when a later sibling's
//! evaluation raises before that retained reference transfers. See
//! `a_tuple_literal_borrowed_element_survives_a_later_siblings_raise` and
//! `a_call_argument_borrowed_value_survives_the_calls_own_raising_argument`
//! below for that flavor's correctness tests, and
//! `tuple_literal_borrowed_element_leak_repro`/`call_argument_borrowed_value_leak_repro`
//! (in `mod peak_rss`) for its peak-RSS proofs.
//!
//! 1. A multi-child MIR node (`BinOp`, `Compare`, a `range()` preheader)
//!    whose earlier child already produced an owned `int` word, and whose
//!    later child's evaluation raises before the parent's own release call
//!    is textually reached. `guard_statement_effects` branches away to the
//!    installed exception target before that release call runs, orphaning
//!    the earlier operand's reference -- one leaked `BigIntObj` per raise.
//! 2. A multi-argument `Call`/`Instantiate` site, where a fresh argument's
//!    ownership is meant to *transfer* to the callee rather than being
//!    released at all -- if a later argument's evaluation raises before the
//!    call is reached, the transfer never happens and the earlier argument's
//!    reference is orphaned the same way. `MirExpr::TupleLiteral`'s own
//!    element-evaluation loop is this same "ownership transfer" flavor: a
//!    fresh element's word is meant to transfer into the aggregate's own
//!    field via `build_insert_value` rather than being released, so a later
//!    sibling element's raising evaluation orphans an earlier element's
//!    reference the identical way a later call argument's raise orphans an
//!    earlier one.
//!
//! Follows `tests/issue_146_bigint_release.rs`'s own `peak_rss` module
//! convention (duplicated helpers, per that file's own stated rationale)
//! rather than sharing a helper crate.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

/// `2^62`, the smallest magnitude that does not round-trip through D-061's
/// tagged 63-bit encoding and therefore always allocates a `BigIntObj`.
const PROMOTED: &str = "4611686018427387904";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_case(case: &str, source: &str) -> (ScratchDir, std::path::PathBuf) {
    let dir = ScratchDir::new(&format!("issue638_{case}")).expect("failed to create scratch dir");
    let src = dir.join("case.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    (dir, src)
}

/// Runs `source` through `pycc run` and asserts it exits `0` with exactly
/// `expected` on stdout -- the no-double-free / value-correctness half of
/// this file's completion criteria (a double release would abort or corrupt
/// the printed value, not merely leak).
fn assert_runs_and_prints(case: &str, source: &str, expected: &str) {
    let (_dir, src) = write_case(case, source);
    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{case} should run to completion, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected,
        "{case} printed the wrong output"
    );
}

// ---------------------------------------------------------------------------
// Value correctness: an exception thrown mid-evaluation must not corrupt the
// live operand it was evaluated alongside, on both the exception path and a
// subsequent normal-path evaluation of the same shape (the no-double-free
// property: at most one of {exception-edge release, fallthrough release}
// ever executes for a given word).
// ---------------------------------------------------------------------------

#[test]
fn a_binop_operand_survives_a_sibling_that_raises_and_is_caught() {
    // `x + x` promotes to a fresh heap bigint (`l`). `1 // z` raises
    // `ZeroDivisionError` while evaluating the outer `+`'s right operand,
    // which must not orphan `l`'s reference -- and the loop's *next* trip
    // exercises the normal (non-raising) path for the very same shape,
    // proving the fix does not turn into a double-release once `z` is
    // nonzero.
    let source = format!(
        "x: int = {PROMOTED}\nz: int = 0\ntotal: int = 0\nfor i in range(0, 4):\n    \
         try:\n        total = total + ((x + x) + (1 // z))\n    except ZeroDivisionError:\n        \
         z = 1\n        total = total + 1\nprint(total)\nprint(x)\n"
    );
    // Trip 0: raises, `z` becomes 1, `total += 1` -> total=1.
    // Trips 1-3: `1 // 1 == 1`, `total += (x+x)+1` each time.
    // `x + x == 2*PROMOTED`; three such additions plus the first `+1`.
    let per_trip: u128 = 2 * 4_611_686_018_427_387_904u128 + 1;
    let expected_total = 1 + per_trip * 3;
    assert_runs_and_prints(
        "binop_exception_edge",
        &source,
        &format!("{expected_total}\n{PROMOTED}\n"),
    );
}

#[test]
fn a_compare_operand_survives_a_sibling_that_raises_and_is_caught() {
    // `(x + x)` is a fresh heap bigint compared against `(1 // z)`, which
    // raises on every trip (`z` is never set away from `0` in the handler).
    // `pycc_rt_int_cmp` hard-aborts (`int_encoding.rs:82`) on any live
    // bigint operand, so unlike the `BinOp`/`Call`-argument siblings above,
    // this shape has no reachable non-exception trip to also exercise the
    // no-double-free property against -- `z` must stay `0` on every trip so
    // `(x + x) < (1 // z)` never actually completes the comparison. `x`
    // reading back intact after three raising trips is this test's
    // no-double-free evidence instead.
    let source = format!(
        "x: int = {PROMOTED}\nz: int = 0\nhits: int = 0\nfor i in range(0, 3):\n    \
         try:\n        if (x + x) < (1 // z):\n            hits = hits + 100\n        else:\n            \
         hits = hits + 1\n    except ZeroDivisionError:\n        hits = hits + 10\nprint(hits)\nprint(x)\n"
    );
    // Every trip raises evaluating `1 // z` before `int_cmp` is ever called
    // -> `hits += 10` three times.
    assert_runs_and_prints(
        "compare_exception_edge",
        &source,
        &format!("30\n{PROMOTED}\n"),
    );
}

#[test]
fn a_for_range_start_bound_survives_a_stop_bound_that_raises() {
    // `range(x, 1 // z)` -- `start`'s freshly evaluated bigint bound must
    // survive `stop`'s raising evaluation. The loop body never runs on the
    // raising trip (the exception fires while building the range itself).
    let source = format!(
        "x: int = {PROMOTED}\nz: int = 0\nhits: int = 0\nfor i in range(0, 3):\n    \
         try:\n        for j in range(x, 1 // z):\n            hits = hits + 1000\n        hits = hits + 1\n    except ZeroDivisionError:\n        \
         z = 1\n        hits = hits + 10\nprint(hits)\nprint(x)\n"
    );
    // Trip 0: `range(x, 1 // z)` raises building `stop` -> +10.
    // Trips 1-2: `1 // 1 == 1`; `range(x, 1)` is empty since x is huge -> +1
    // each, loop body never runs.
    assert_runs_and_prints(
        "for_range_exception_edge",
        &source,
        &format!("12\n{PROMOTED}\n"),
    );
}

#[test]
fn a_call_argument_survives_a_later_sibling_argument_that_raises() {
    // `f(x + x, 1 // z)`: the first argument's fresh bigint word is meant to
    // *transfer* into `f`'s parameter on the normal path. If evaluating the
    // second argument raises before the call is ever reached, that transfer
    // must not have already orphaned the first argument's reference.
    let source = format!(
        "def f(a: int, b: int) -> int:\n    return a + b\n\nx: int = {PROMOTED}\nz: int = 0\ntotal: int = 0\nfor i in range(0, 3):\n    \
         try:\n        total = total + f(x + x, 1 // z)\n    except ZeroDivisionError:\n        \
         z = 1\n        total = total + 1\nprint(total)\nprint(x)\n"
    );
    // Trip 0 raises evaluating `1 // z` -> total += 1.
    // Trips 1-2: `f(x+x, 1) == 2*PROMOTED + 1`, added twice.
    let per_trip: u128 = 2 * 4_611_686_018_427_387_904u128 + 1;
    let expected_total = 1 + per_trip * 2;
    assert_runs_and_prints(
        "call_argument_exception_edge",
        &source,
        &format!("{expected_total}\n{PROMOTED}\n"),
    );
}

#[test]
fn a_tuple_literal_elements_survives_a_later_siblings_raise() {
    // `(x + x, 1 // z)`: the tuple's first element is a fresh heap bigint
    // that must survive the second element's raising evaluation -- the
    // sixth site closed by this fix (D-208), `MirExpr::TupleLiteral`'s own
    // element-evaluation loop, mirroring `f(x + x, 1 // z)`'s call-argument
    // shape immediately above but with ownership transferring into the
    // aggregate's own field via `build_insert_value` rather than into a
    // callee's parameter slot.
    let source = format!(
        "x: int = {PROMOTED}\nz: int = 0\ntotal: int = 0\nfor i in range(0, 3):\n    \
         try:\n        pair = (x + x, 1 // z)\n        total = total + pair[0]\n    except ZeroDivisionError:\n        \
         z = 1\n        total = total + 1\nprint(total)\nprint(x)\n"
    );
    // Trip 0 raises evaluating `1 // z` (the tuple's second element) before
    // the tuple is ever fully built -> total += 1, z becomes 1.
    // Trips 1-2: `(x + x, 1)[0] == 2*PROMOTED`, added twice.
    let per_trip: u128 = 2 * 4_611_686_018_427_387_904u128;
    let expected_total = 1 + per_trip * 2;
    assert_runs_and_prints(
        "tuple_literal_exception_edge",
        &source,
        &format!("{expected_total}\n{PROMOTED}\n"),
    );
}

#[test]
fn a_tuple_literal_borrowed_element_survives_a_later_siblings_raise() {
    // Issue #834: `(x, 1 // z)` -- `x` is a pre-existing bigint *name*, a
    // borrowed/duplicate reference (`retain_if_int_duplicate`'s own
    // classification), not the fresh/owning shape
    // `a_tuple_literal_elements_survives_a_later_siblings_raise` above
    // already covers. The tuple's second element raises before the
    // aggregate is ever built, so the extra retain this fix now tracks on
    // the exception edge must be released without touching `x`'s own
    // reference -- proven by `x` (and `x + 1`, a fresh use of it) printing
    // correctly after the catch, on both the exception-edge release and a
    // subsequent normal-path read of the same name.
    let source = format!(
        "x: int = {PROMOTED}\nz: int = 0\ntry:\n    pair = (x, 1 // z)\n    \
         print(pair[0])\nexcept ZeroDivisionError:\n    print(\"caught\")\nprint(x)\nprint(x + 1)\n"
    );
    let x_plus_one: u128 = 4_611_686_018_427_387_904u128 + 1;
    assert_runs_and_prints(
        "tuple_literal_borrowed_element_exception_edge",
        &source,
        &format!("caught\n{PROMOTED}\n{x_plus_one}\n"),
    );
}

#[test]
fn a_call_argument_borrowed_value_survives_the_calls_own_raising_argument() {
    // Issue #834's own named double-release-safety proof (see the issue's
    // published implementation plan, section 5): `f(x, 1 // z)` -- `x` is a
    // borrowed/duplicate `Ty::Int` argument, and the second argument's
    // raising evaluation means `build_call` is never reached, so the
    // callee's own parameter-slot release machinery can never fire for
    // this call. If the exception-edge release this fix adds instead fired
    // on a word the callee-side path also released, `x` would come back
    // corrupted (wrong value, or a debug/ASan abort) after the catch --
    // this test's whole point is proving that does not happen.
    let source = "def f(a: int, b: int) -> int:\n    return a + b\n\nz: int = 0\n\
                  x: int = 9223372036854775807 + 5\ntry:\n    r = f(x, 1 // z)\n\
                  except ZeroDivisionError:\n    print(\"caught\")\nprint(x)\nprint(x + 1)\n";
    assert_runs_and_prints(
        "call_argument_borrowed_value_exception_edge",
        source,
        "caught\n9223372036854775812\n9223372036854775813\n",
    );
}

// ---------------------------------------------------------------------------
// Peak-RSS marginal comparison: the leak is invisible to every value
// assertion above -- releasing an object one iteration late (or never) does
// not corrupt any value this program prints, only its memory footprint.
//
// A naive single-vs-double-iteration RSS *ratio* (the convention
// `tests/issue_146_bigint_release.rs`'s own `peak_rss` module uses) is
// blind here: every repro below raises and catches `ZeroDivisionError` on
// *every* trip, and `crates/pycc_rt/src/exception.rs` documents that
// exception-object lifetime management is "intentionally leak-only in this
// first implementation" -- `pycc_rt_exception_clear` never frees the
// `PyExceptionObj` (or its message) that `pycc_rt_exception_alloc` built.
// That pre-existing, already-accepted, out-of-#638-scope leak scales
// linearly with the raise count on *both* a correctly-fixed and a
// still-buggy binary, so a plain ratio against a `< 1.35` threshold would
// fail unconditionally regardless of whether this issue's fix works --
// confirmed empirically: with the D-208 codegen changes applied, the
// straight single-vs-double ratio of the binop repro alone is still
// ~1.947 (250k -> 38,125,568 bytes; 500k -> 74,235,904 bytes), because the
// ratio measures the exception-object leak, not the bigint leak.
//
// Each test below instead compares the *marginal* RSS growth (250k -> 500k
// iterations) of the leak-shape repro against a same-exception-rate control
// repro that never allocates the fresh temporary this issue is about (its
// exception-side operand is a plain `Name` read, D-181's own
// "duplicate reference" case, not a birth reference). Both repros raise
// exactly once per trip, so both marginals include one exception-object
// leak per iteration; the control's marginal is therefore the calibration
// the plain ratio is missing: the leak rate contributed by the
// pre-existing, accepted exception-object model alone. Measured directly:
// with the D-208 fix applied, `leak_marginal / control_marginal` is
// ~0.9995 for all three flavors (the two marginals track each other almost
// exactly); reverting the fix alone (keeping this test's own repro shapes)
// widens it to ~1.333 (one extra `BigIntObj` release's worth of marginal
// growth per iteration). `< 1.15` sits with a comfortable margin on both
// sides of that gap.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod peak_rss {
    use super::{PROMOTED, pycc_bin, write_case};
    use std::process::Command;

    /// Duplicated from `tests/issue_146_bigint_release.rs::peak_rss::peak_rss`
    /// per that file's own stated convention for this measurement.
    #[allow(clippy::zombie_processes)]
    fn peak_rss(mut command: Command) -> libc::c_long {
        let child = command.spawn().expect("command must spawn");
        let pid = child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        let waited = loop {
            // SAFETY: `pid` identifies the live child spawned immediately
            // above; both output pointers are valid for writes for the
            // duration of the call, and a successful `wait4` initializes the
            // complete `rusage` value.
            let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
            if result != -1 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::Interrupted,
                "wait failed for {command:?}: {error}"
            );
        };
        assert_eq!(waited, pid, "wait4 reaped the wrong child for {command:?}");
        // SAFETY: the successful `wait4` above initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "command failed: {command:?}"
        );
        usage.ru_maxrss
    }

    fn built_program_peak_rss(case: &str, source: &str) -> libc::c_long {
        let (dir, src) = write_case(case, source);
        let bin = dir.join(format!("{case}_rss"));
        let build = Command::new(pycc_bin())
            .arg("build")
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .status()
            .expect("pycc build must spawn");
        assert!(build.success(), "pycc build of {case} failed");
        peak_rss(Command::new(&bin))
    }

    /// Computes the marginal peak-RSS growth of a repro generator between
    /// `iterations` and `2 * iterations` trips -- see the module doc comment
    /// for why the marginal, not the raw ratio, is the discriminating
    /// measurement here.
    fn marginal_rss(case_prefix: &str, repro: impl Fn(u32) -> String, iterations: u32) -> i64 {
        let single = built_program_peak_rss(&format!("{case_prefix}_1x"), &repro(iterations));
        let double = built_program_peak_rss(&format!("{case_prefix}_2x"), &repro(iterations * 2));
        double - single
    }

    /// The plan's own repro shape (issue #638, corrected against the
    /// current tree): `(x + x) + (1 // z)` inside a `try`/`except` that
    /// always re-zeroes `z`, so every trip raises and takes the exception
    /// edge. Before this fix, `(x + x)`'s birth reference was orphaned on
    /// every trip.
    fn binop_exception_edge_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        y = (x + x) + (1 // z)\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    /// Same exception rate (one raise per trip) as [`binop_exception_edge_repro`]
    /// but the left `BinOp` operand is a plain `Name` read of `x` -- D-181's
    /// "duplicate reference" case, not a birth reference -- so this repro's
    /// marginal RSS growth reflects only the pre-existing, accepted
    /// exception-object leak-only model, not any `BigIntObj` release.
    fn binop_control_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        y = x + (1 // z)\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    #[test]
    fn a_binop_operand_orphaned_on_the_exception_edge_does_not_grow_with_the_iteration_count() {
        let leak_marginal = marginal_rss("rss_exc_binop", binop_exception_edge_repro, 250_000);
        let control_marginal = marginal_rss("rss_ctl_binop", binop_control_repro, 250_000);
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "leak-shape marginal RSS growth must track the same-exception-rate \
             control's, not scale by an extra `BigIntObj` release per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    /// Same shape via `Compare` instead of `BinOp`: `(x + x) < (1 // z)`.
    fn compare_exception_edge_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\nfor i in range({iterations}):\n    \
             try:\n        if (x + x) < (1 // z):\n            pass\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    /// Control for the `Compare` flavor: `x < (1 // z)` -- `x` read directly,
    /// no fresh `BinOp` temporary on the left-hand side.
    fn compare_control_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\nfor i in range({iterations}):\n    \
             try:\n        if x < (1 // z):\n            pass\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    #[test]
    fn a_compare_operand_orphaned_on_the_exception_edge_does_not_grow_with_the_iteration_count() {
        let leak_marginal = marginal_rss("rss_exc_cmp", compare_exception_edge_repro, 250_000);
        let control_marginal = marginal_rss("rss_ctl_cmp", compare_control_repro, 250_000);
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "leak-shape marginal RSS growth must track the same-exception-rate \
             control's, not scale by an extra `BigIntObj` release per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    /// The `Call`-argument ownership-transfer flavor (#638's own "second
    /// flavor" finding, not present in the original issue text): `f(x + x,
    /// 1 // z)`, where the first argument's fresh word must survive the
    /// second argument's raising evaluation.
    fn call_argument_exception_edge_repro(iterations: u32) -> String {
        format!(
            "def f(a: int, b: int) -> int:\n    return a + b\n\nx: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        y = f(x + x, 1 // z)\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    /// Control for the `Call`-argument flavor: `f(x, 1 // z)` -- the first
    /// argument is a plain `Name` read of `x`, transferring no fresh
    /// temporary.
    fn call_argument_control_repro(iterations: u32) -> String {
        format!(
            "def f(a: int, b: int) -> int:\n    return a + b\n\nx: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        y = f(x, 1 // z)\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    #[test]
    fn a_call_argument_orphaned_on_the_exception_edge_does_not_grow_with_the_iteration_count() {
        let leak_marginal =
            marginal_rss("rss_exc_call", call_argument_exception_edge_repro, 250_000);
        let control_marginal = marginal_rss("rss_ctl_call", call_argument_control_repro, 250_000);
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "leak-shape marginal RSS growth must track the same-exception-rate \
             control's, not scale by an extra `BigIntObj` release per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    /// The `TupleLiteral` element-transfer flavor (this fix's own sixth
    /// site): `(x + x, 1 // z)`, where the first element's fresh word must
    /// survive the second element's raising evaluation, mirroring
    /// [`call_argument_exception_edge_repro`]'s shape but with the
    /// aggregate's own field taking ownership instead of a callee's
    /// parameter slot.
    fn tuple_literal_exception_edge_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        pair = (x + x, 1 // z)\n        y = pair[0]\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    /// Control for the `TupleLiteral` flavor: `(x, 1 // z)` -- the first
    /// element is a plain `Name` read of `x`, transferring no fresh
    /// temporary.
    fn tuple_literal_control_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\nz: int = 0\ny: int = 0\nfor i in range({iterations}):\n    \
             try:\n        pair = (x, 1 // z)\n        y = pair[0]\n    except ZeroDivisionError:\n        z = 0\nprint(x)\n"
        )
    }

    #[test]
    fn a_tuple_literal_element_orphaned_on_the_exception_edge_does_not_grow_with_the_iteration_count()
     {
        let leak_marginal =
            marginal_rss("rss_exc_tuple", tuple_literal_exception_edge_repro, 250_000);
        let control_marginal = marginal_rss("rss_ctl_tuple", tuple_literal_control_repro, 250_000);
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "leak-shape marginal RSS growth must track the same-exception-rate \
             control's, not scale by an extra `BigIntObj` release per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    /// Issue #834's own repro shape: a *borrowed* tuple element (`x`, a
    /// bare `Name` read, `retain_if_int_duplicate`'s duplicate-reference
    /// classification), freshly rebound to a new bigint every trip so each
    /// iteration's extra retain -- when leaked -- contributes its own
    /// `BigIntObj`'s worth of marginal growth. `x`'s own per-iteration
    /// allocation happens identically in the leak and control scripts
    /// below (holding allocation/release cost fixed); only whether `x`
    /// (leak) or a plain literal `1` (control) is placed into the tuple
    /// differs, isolating the retain itself from tuple-construction cost.
    fn tuple_literal_borrowed_element_leak_repro(iterations: u32) -> String {
        format!(
            "z: int = 0\nfor i in range({iterations}):\n    x: int = 9223372036854775807 + i\n    \
             try:\n        pair = (x, 1 // z)\n    except ZeroDivisionError:\n        pass\nprint(1)\n"
        )
    }

    /// Control for [`tuple_literal_borrowed_element_leak_repro`]: the same
    /// per-iteration fresh-bigint allocation into `x`, but the tuple's
    /// first element is a plain literal `1` -- `x` is computed but never
    /// placed anywhere `retain_if_int_duplicate` would see it, so this
    /// repro's marginal growth reflects only `x`'s own allocation/release
    /// cost and the pre-existing exception-object leak-only model.
    fn tuple_literal_borrowed_element_control_repro(iterations: u32) -> String {
        format!(
            "z: int = 0\nfor i in range({iterations}):\n    x: int = 9223372036854775807 + i\n    \
             try:\n        pair = (1, 1 // z)\n    except ZeroDivisionError:\n        pass\nprint(1)\n"
        )
    }

    #[test]
    fn a_tuple_literal_borrowed_element_marginal_rss_is_flat() {
        let leak_marginal = marginal_rss(
            "rss_exc_tuple_borrowed",
            tuple_literal_borrowed_element_leak_repro,
            250_000,
        );
        let control_marginal = marginal_rss(
            "rss_ctl_tuple_borrowed",
            tuple_literal_borrowed_element_control_repro,
            250_000,
        );
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "borrowed-element marginal RSS growth must track the same-allocation-rate \
             control's, not scale by an extra `BigIntObj` retain-and-leak per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }

    /// Issue #834's call-argument counterpart to
    /// [`tuple_literal_borrowed_element_leak_repro`]: the same per-iteration
    /// fresh-bigint `x`, passed as a borrowed call argument instead of a
    /// tuple element.
    fn call_argument_borrowed_value_leak_repro(iterations: u32) -> String {
        format!(
            "def f(a: int, b: int) -> int:\n    return a + b\n\nz: int = 0\nfor i in range({iterations}):\n    \
             x: int = 9223372036854775807 + i\n    try:\n        y = f(x, 1 // z)\n    except ZeroDivisionError:\n        pass\nprint(1)\n"
        )
    }

    /// Control for [`call_argument_borrowed_value_leak_repro`]: same
    /// per-iteration fresh-bigint allocation into `x`, but the call's first
    /// argument is a plain literal `1`.
    fn call_argument_borrowed_value_control_repro(iterations: u32) -> String {
        format!(
            "def f(a: int, b: int) -> int:\n    return a + b\n\nz: int = 0\nfor i in range({iterations}):\n    \
             x: int = 9223372036854775807 + i\n    try:\n        y = f(1, 1 // z)\n    except ZeroDivisionError:\n        pass\nprint(1)\n"
        )
    }

    #[test]
    fn a_call_argument_borrowed_value_marginal_rss_is_flat() {
        let leak_marginal = marginal_rss(
            "rss_exc_call_borrowed",
            call_argument_borrowed_value_leak_repro,
            250_000,
        );
        let control_marginal = marginal_rss(
            "rss_ctl_call_borrowed",
            call_argument_borrowed_value_control_repro,
            250_000,
        );
        assert!(
            (leak_marginal as f64) < (control_marginal as f64) * 1.15,
            "borrowed-argument marginal RSS growth must track the same-allocation-rate \
             control's, not scale by an extra `BigIntObj` retain-and-leak per trip: \
             leak_marginal={leak_marginal} control_marginal={control_marginal}"
        );
    }
}
