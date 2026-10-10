//! `range()` loop stepping and operand normalization.

use super::*;

// --- Implementation note / deviation from the task brief -------------
//
// The brief's own Step 2 code makes `pycc_rt_range_continue` a single
// plain `extern "C" fn`, with a Step 1 `#[should_panic]` test calling that
// same public wrapper directly for the zero-step case. Per this crate's
// own established convention (see the implementation-note comment above
// `int_add`, discovered empirically during Task 3): a panic that unwinds
// past a plain `extern "C" fn`'s own boundary is caught right there and
// turned into a process abort, regardless of who calls it -- including
// this crate's own same-binary Rust tests. `pycc_rt_range_continue` *can*
// still panic (`classify_encoded_int`'s fail-closed rejection of a
// malformed encoded word), so it keeps the same split every other panicking
// `pycc_rt_int_*` function already gets: a private, ordinary-Rust-ABI
// `range_continue` holding the real logic (freely panics, unwinds
// normally, `#[should_panic]`-testable), and a thin `pub extern "C"`
// wrapper of the exact brief-specified name/signature for pycc-generated
// code to call. Since #150 (D-173's pending-exception mechanism), the
// zero-step case no longer panics -- it raises a `ValueError` and returns
// the ordinary "stop" sentinel instead -- so the tests for that case below
// assert the pending exception state rather than `#[should_panic]`.
//
// Since #147 the operands are ordered through `encoded_int_cmp` rather than
// decoded to an inline value, so a bigint start, stop, step, or
// mid-loop-promoted induction variable drives the loop normally instead of
// aborting at D-141's runtime `int` boundary. Note the zero-step check reads
// the *step's own encoded order against zero*, not the raw word: a bigint
// zero step (reachable via `a - a` on two promoted values) is still rejected,
// and a bigint word is never numerically `0`.
//
// D-173 (#382) established a pending-exception mechanism (`raise_builtin`)
// for exactly this shape of runtime failure boundary: since #150, a zero
// step sets a `ValueError` (CPython's own `range() arg 3 must not be zero`
// message) instead of panicking, and returns `0` -- the same sentinel this
// function already returns for ordinary loop exhaustion. No codegen change
// was needed for that to surface reliably: a `0` return makes the enclosing
// `for`/comprehension loop exit exactly as if it had completed normally, and
// that statement is then subject to the same pre-existing
// `pycc_rt_exception_active` checkpoints every other D-173 exception already
// relies on (the top-level statement loop and the `try`-body per-statement
// check in `pycc_codegen`) -- mirroring `float_div`'s own scope exactly: any
// context other than the top-level statement loop or a `try` body observes
// the pending exception only at the next enclosing checkpoint, not
// immediately, which is an existing, accepted D-173 characteristic rather
// than a new gap this change introduces (see
// `uncaught_zero_step_range_inside_a_function_body_is_observed_at_the_next_checkpoint`
// in `tests/issue_150_zero_step_range.rs`, which pins the exact observed
// shape).
pub(crate) fn range_continue(i: i64, stop: i64, step: i64) -> i8 {
    // Fast path: three inline operands -- the shape of every ordinary
    // smallint loop -- are ordered as plain `i64`s. Sending them through
    // `encoded_int_cmp` instead measured a consistent ~22% slowdown on a
    // 50-million-iteration release loop (0.32s -> 0.39s across five paired
    // rounds; see D-179's Consequences), and this restores the pre-#147 cost
    // exactly. It is not a semantic special case: `inline_int_value` yields
    // `None` for a bigint, so any promoted operand falls through to the
    // general path below, and a malformed word still fails closed inside
    // `classify_encoded_int` on either path.
    if let (Some(i), Some(stop), Some(step)) = (
        inline_int_value(i),
        inline_int_value(stop),
        inline_int_value(step),
    ) {
        return match step.cmp(&0) {
            std::cmp::Ordering::Greater => i8::from(i < stop),
            std::cmp::Ordering::Less => i8::from(i > stop),
            std::cmp::Ordering::Equal => {
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "range() arg 3 must not be zero",
                );
                0
            }
        };
    }
    let zero = tag_smallint(0);
    match encoded_int_cmp(step, zero) {
        std::cmp::Ordering::Greater => {
            i8::from(encoded_int_cmp(i, stop) == std::cmp::Ordering::Less)
        }
        std::cmp::Ordering::Less => {
            i8::from(encoded_int_cmp(i, stop) == std::cmp::Ordering::Greater)
        }
        std::cmp::Ordering::Equal => {
            raise_builtin(
                EXCEPTION_TYPE_VALUE_ERROR,
                "ValueError",
                "range() arg 3 must not be zero",
            );
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_range_continue(i: i64, stop: i64, step: i64) -> i8 {
    range_continue(i, stop, step)
}

/// Normalizes one `range()` operand for D-141's bool-identity contract:
/// `range` consumes the numeric *value* of its arguments and produces
/// ordinary integer objects, so `True`/`False` markers become the ordinary
/// smallints `1`/`0` before entering the induction phi. A smallint and --
/// since #147 -- a heap bigint pass through unchanged; a malformed word
/// fails closed inside `classify_encoded_int` exactly as before.
///
/// This replaces the `range_untag_operand` call to
/// `pycc_rt_int_untag_checked` that codegen used to emit: that decoder
/// rejects every bigint, which is precisely the #147 defect. Normalization
/// stays a runtime call because only `pycc_rt` may interpret an encoded
/// word.
///
/// Split into a private fn plus a thin wrapper for the same reason
/// `range_continue` is (see the note above): it can panic.
pub(crate) fn range_normalize_operand(encoded: i64) -> i64 {
    match classify_encoded_int(encoded) {
        EncodedIntKind::SmallInt | EncodedIntKind::BigInt => encoded,
        EncodedIntKind::BoolFalse => tag_smallint(0),
        EncodedIntKind::BoolTrue => tag_smallint(1),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_range_normalize_operand(encoded: i64) -> i64 {
    range_normalize_operand(encoded)
}
