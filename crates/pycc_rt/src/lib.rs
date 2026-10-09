//! # Build order: this crate's staticlib is not a normal Cargo dependency
//!
//! This crate's real consumer is not Rust code but pycc-generated object
//! files, which reference `pycc_rt_print_i64` by symbol name and are
//! linked against this crate's `staticlib` output (`libpycc_rt.a` on
//! Unix-like targets, `pycc_rt.lib` on `-msvc` targets -- see D-028) via a
//! linker-driver invocation (`cc`, or on Windows the bundled `clang` --
//! see D-028) -- in `pycc_codegen`'s own tests (`link_object_with_runtime`)
//! and in `pycc`'s real `build`/`run` (`src/main.rs`). Nothing in Cargo's
//! normal dependency graph expresses that relationship: Cargo does not
//! uplift a `staticlib` reached through an ordinary `[dependencies]` edge
//! to a predictable path, and `cargo test` does not build the `staticlib`
//! crate type at all. So neither `cargo build --workspace` nor
//! `cargo test --workspace` can be relied on to leave the archive where
//! anything looks for it.
//!
//! **This is now handled automatically.** `pycc_codegen`'s build script
//! (`crates/pycc_codegen/build.rs`) runs `cargo build --locked -p pycc_rt`
//! for *both* host profiles into a private directory under its own
//! `OUT_DIR`, then installs the resulting archive at `<target-root>/debug/`
//! and `<target-root>/release/` -- the directories
//! `pycc_artifact_layout::find_pycc_rt_lib_dir_in` searches. Because every
//! path into the compiler goes through `pycc_codegen`, a clean checkout
//! builds and tests with no manual step: `cargo build -p pycc_rt` is no
//! longer a prerequisite for `cargo test -p pycc_codegen` or for
//! `cargo run --bin pycc -- build ...`. See
//! `docs/decisions/D-184-build-pycc-rt-from-pycc-codegen-s-build.md`.
//!
//! The deadlock that made an earlier attempt at this unworkable was real
//! and is avoided rather than wished away: a build script that invokes
//! `cargo` at the *same* build directory blocks forever on Cargo's own
//! build-directory lock, which the outer invocation holds for the whole
//! build-script execution. The nested build therefore gets its own
//! `--target-dir` under `OUT_DIR`. This crate declaring no dependencies at
//! all is what keeps that nested build cheap and keeps it from contending
//! for `$CARGO_HOME/.package-cache`; `tests/issue_630_pycc_rt_build_dependency.rs`
//! asserts that property so it cannot be lost silently.
//!
//! Cross-compilation is still explicit. When `pycc build --target <triple>`
//! names a triple this workspace was not itself built for, run
//! `rustup target add <triple>` and
//! `cargo build [--release] --target <triple> -p pycc_rt` first; the build
//! script only produces the host's archives, and the diagnostic says so.

use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

mod exception;
/// D-244 rule 2's `PyObject*` boundary, runtime half (#1025/#1028).
pub mod ext_bridge;
/// `hash()` of `int`, `bool` and int/bool tuples (#1331).
mod hash;
mod instance;
/// `<< >> & | ^` over encoded ints (#1210).
mod int_bitwise;
mod int_encoding;
/// `set[int]`/`frozenset[int]`'s `PyIntSetObj` (D-121, Part 1 of #1319).
mod int_set;
/// `print`'s stdout flush (#1340); the other print primitives stay below.
mod print;

#[cfg(not(test))]
pub use exception::pycc_rt_exception_print_and_exit;
use exception::raise_builtin;
pub use exception::{
    EXCEPTION_TYPE_ATTRIBUTE_ERROR, EXCEPTION_TYPE_EXCEPTION, EXCEPTION_TYPE_FOREIGN_BASE,
    EXCEPTION_TYPE_INDEX_ERROR, EXCEPTION_TYPE_KEY_ERROR, EXCEPTION_TYPE_OVERFLOW_ERROR,
    EXCEPTION_TYPE_RUNTIME_ERROR, EXCEPTION_TYPE_TYPE_ERROR, EXCEPTION_TYPE_VALUE_ERROR,
    EXCEPTION_TYPE_ZERO_DIV_ERROR, PyExceptionObj, pycc_rt_exception_active,
    pycc_rt_exception_alloc, pycc_rt_exception_clear, pycc_rt_exception_group_alloc,
    pycc_rt_exception_group_partition, pycc_rt_exception_message, pycc_rt_exception_raise,
    pycc_rt_exception_raise_with_cause, pycc_rt_exception_type_matches,
    pycc_rt_ext_pending_message, pycc_rt_ext_pending_type,
};
pub use hash::{pycc_rt_hash_int, pycc_rt_hash_pointer, pycc_rt_hash_slot_int, pycc_rt_hash_tuple};
pub use instance::{
    PyInstanceObj, pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_class,
    pycc_rt_ext_instance_copy, pycc_rt_ext_instance_delete_slot, pycc_rt_ext_instance_set_carrier,
    pycc_rt_ext_instance_store_slot, pycc_rt_instance_get_slot, pycc_rt_instance_get_slot_checked,
    pycc_rt_instance_new, pycc_rt_instance_set_slot,
};
pub use int_bitwise::{
    pycc_rt_int_and, pycc_rt_int_lshift, pycc_rt_int_or, pycc_rt_int_rshift, pycc_rt_int_xor,
};
// D-061/D-141's one-word `int` encoding and its heap bigint representation.
// Glob-imported so the operations below -- and their `#[cfg(test)]` tests,
// which reach them through `use super::*` -- keep referring to these names
// unqualified, exactly as when they lived in this file.
use int_encoding::*;
pub use int_set::{
    PyIntSetObj, pycc_rt_int_set_add, pycc_rt_int_set_check_not_resized, pycc_rt_int_set_copy,
    pycc_rt_int_set_decref, pycc_rt_int_set_from_int_list, pycc_rt_int_set_get,
    pycc_rt_int_set_incref, pycc_rt_int_set_len, pycc_rt_int_set_new, pycc_rt_obj_set_add_identity,
    pycc_rt_obj_set_candidate, pycc_rt_obj_set_push,
};
pub use print::pycc_rt_print_flush;

fn format_i64_line(value: i64) -> String {
    format!("{value}\n")
}

fn divmod_small(limbs: &[u32], divisor: u32) -> (Vec<u32>, u32) {
    let mut quotient = vec![0u32; limbs.len()];
    let mut remainder: u64 = 0;
    for i in (0..limbs.len()).rev() {
        let acc = (remainder << 32) | limbs[i] as u64;
        quotient[i] = (acc / divisor as u64) as u32;
        remainder = acc % divisor as u64;
    }
    (quotient, remainder as u32)
}

fn bigint_to_decimal_string(negative: bool, limbs: &[u32]) -> String {
    let mut limbs = limbs.to_vec();
    let mut digits = Vec::new();
    loop {
        let (q, r) = divmod_small(&limbs, 10);
        digits.push(
            std::char::from_digit(r, 10).expect("a remainder of division by 10 is always 0-9"),
        );
        limbs = trim(&q);
        if limbs.len() == 1 && limbs[0] == 0 {
            break;
        }
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

// --- Implementation note / deviation from the task brief -------------
//
// The brief's own doc comment (kept, below, as the historical record of
// the *intended* design) claims that a panic raised by calling one of
// these `pycc_rt_int_*` functions directly from this crate's own Rust
// test code "is an ordinary, same-binary unwind the test harness
// catches -- no FFI boundary is crossed during the test itself." That
// claim does not hold on this toolchain (rustc 1.97.1): whether a panic
// unwinding past a function's own boundary gets caught and turned into
// an abort is a property of *that function's own* declared ABI, decided
// at the point the function itself is compiled -- not of who happens to
// call it. A plain `extern "C" fn` (not `extern "C-unwind"`) always gets
// this abort-on-unwind landing pad around its own body, so calling it
// directly from ordinary same-crate Rust test code panics into a
// `SIGABRT` (confirmed empirically: the `#[should_panic]` tests below
// aborted the whole test binary instead of being caught, before this
// split existed).
//
// The *intent* behind choosing plain `extern "C"` is still correct and
// worth keeping: pycc-generated LLVM IR has no personality routine/unwind
// tables, so a real unwind escaping into it would be genuinely unsafe --
// converting that into a deterministic abort right at this boundary is
// the right call for the actual compiled-program scenario. Only the
// "therefore it's directly unit-testable with `#[should_panic]`" half of
// the claim was wrong.
//
// Fix: split each `pycc_rt_int_*` symbol into a private, ordinary-Rust-ABI
// function holding the real logic (freely panics, unwinds normally, so
// `#[should_panic]` can catch it when called directly) and a thin
// `#[unsafe(no_mangle)] pub extern "C" fn` wrapper of the exact name/
// signature the brief specifies, which every later task's codegen still
// calls unchanged. Tests exercising a panicking path call the private
// function directly; tests only checking a successful return value keep
// calling the public wrapper (also exercising the wrapper's own line, no
// unwind ever crosses its boundary on those paths).
fn int_add(a: i64, b: i64) -> i64 {
    if let (Some(a), Some(b)) = (inline_int_value(a), inline_int_value(b)) {
        if let Some(result) = a.checked_add(b).and_then(fits_smallint) {
            return result;
        }
        // Both operands fit 63 bits, so their true sum always fits i128
        // with room to spare -- exact, no further bigint math needed
        // for this specific promotion step.
        return tag_bigint(bigint_from_i128(a as i128 + b as i128));
    }
    let (a_neg, a_mag) = to_sign_and_magnitude(a);
    let (b_neg, b_mag) = to_sign_and_magnitude(b);
    tag_bigint(bigint_add_signed(a_neg, &a_mag, b_neg, &b_mag))
}

/// # Ownership (no-aliasing invariant, applies to every `pycc_rt_int_*`
/// function below, [D-181])
/// No `int` operation may return an operand's own encoded word. Every
/// bigint result is built through `tag_bigint(..)` on a freshly
/// constructed `BigIntObj`, so a returned heap word is always a *new*
/// object owning exactly one reference and never an alias of `a` or `b`.
///
/// `pycc_codegen` relies on this: it releases both operands' birth
/// references immediately after the call (D-181's `BinOp`/`Compare` release
/// sites). An identity fast path here -- returning `a` unchanged for
/// `b == 0`, say -- would turn each of those releases into a use-after-free
/// on the value just produced. Adding one therefore requires changing the
/// emitter first. `an_int_operation_never_returns_an_operand_s_own_word`
/// below pins this.
///
/// [D-181]: ../../../docs/decisions/D-181-release-a-heap-bigint-s-birth-reference-at-every.md
///
/// # Safety (panic-across-FFI note, applies to every `pycc_rt_int_*`
/// function below)
/// These are plain `extern "C" fn`s, not `extern "C-unwind"`. Since Rust
/// 1.71, a panic that would otherwise unwind past an ordinary
/// `extern "C"` function's boundary is caught at that boundary and turned
/// into a process abort instead of continuing to unwind into a foreign
/// (non-Rust, no unwind tables) caller -- which is exactly what happens
/// here when pycc-generated LLVM code calls one of these and it panics.
/// This is a real, stable Rust guarantee (not assumed UB-avoidance) --
/// see the implementation-note comment above this function for why that
/// same guarantee makes calling *this* wrapper directly unsuitable for
/// `#[should_panic]` testing (the private `int_add` etc. functions above
/// are used for that instead).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_add(a: i64, b: i64) -> i64 {
    int_add(a, b)
}

fn int_sub(a: i64, b: i64) -> i64 {
    if let (Some(a), Some(b)) = (inline_int_value(a), inline_int_value(b)) {
        if let Some(result) = a.checked_sub(b).and_then(fits_smallint) {
            return result;
        }
        return tag_bigint(bigint_from_i128(a as i128 - b as i128));
    }
    let (a_neg, a_mag) = to_sign_and_magnitude(a);
    let (b_neg, b_mag) = to_sign_and_magnitude(b);
    tag_bigint(bigint_add_signed(a_neg, &a_mag, !b_neg, &b_mag))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_sub(a: i64, b: i64) -> i64 {
    int_sub(a, b)
}

/// Part C of #1038 ([#1065](https://github.com/rotnov/pycc/issues/1065)):
/// decodes an encoded `int` word to its inline numeric value, or raises
/// `OverflowError` (D-173) and yields `None` when the word is a heap bigint
/// pointer. It replaces `int_encoding::require_inline_int`, whose `panic!`
/// unwound across the `extern "C"` boundary and became a process abort --
/// fatal to an `ext` module's host interpreter (D-244).
///
/// Every caller must `return` its own type-valid sentinel on `None`
/// *before* reaching any further `raise_builtin` call. `raise_builtin`
/// installs unconditionally and does not check for an already-pending
/// exception, so a later raise would otherwise report over this one; the
/// zero-divisor arms of `int_floordiv`/`int_floormod` and the
/// negative-exponent arm of `int_pow` are exactly that shape, and a bigint
/// operand decoding to the sentinel `0` would reach them. Returning early
/// closes that collision by construction rather than by a
/// `pycc_rt_exception_active()` guard repeated at each raise site.
fn decode_inline_or_raise(encoded: i64, context: &str) -> Option<i64> {
    let value = inline_int_value(encoded);
    if value.is_none() {
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            &format!("{context} a bigint-valued `int` is not supported yet"),
        );
    }
    value
}

fn int_mul(a: i64, b: i64) -> i64 {
    let Some(a) = decode_inline_or_raise(a, "multiplying") else {
        return tag_smallint(0);
    };
    let Some(b) = decode_inline_or_raise(b, "multiplying") else {
        return tag_smallint(0);
    };
    // Two decoded inline operands are each at most 62 magnitude bits, so
    // their exact product always fits in i128. Keep the tagged fast path when
    // possible and promote only the result, matching add/sub without
    // requiring general bigint multiplication yet.
    let product = a as i128 * b as i128;
    i64::try_from(product)
        .ok()
        .and_then(fits_smallint)
        .unwrap_or_else(|| tag_bigint(bigint_from_i128(product)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_mul(a: i64, b: i64) -> i64 {
    int_mul(a, b)
}

fn int_floordiv(a: i64, b: i64) -> i64 {
    let Some(a) = decode_inline_or_raise(a, "dividing") else {
        return tag_smallint(0);
    };
    // Returning here before the `b == 0` arm below is load-bearing, not
    // stylistic: a bigint divisor decodes to no value at all, and letting it
    // fall through as a `0` would report `ZeroDivisionError` over the
    // `OverflowError` just raised. See `decode_inline_or_raise`.
    let Some(b) = decode_inline_or_raise(b, "dividing") else {
        return tag_smallint(0);
    };
    if b == 0 {
        // D-173: set the pending exception flag instead of panicking.
        // Returns a sentinel `0` (tagged smallint 0); the caller's
        // generated code checks `pycc_rt_exception_active()` after this
        // call and branches to the exception handler before using the
        // result.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "integer division by zero",
        );
        return tag_smallint(0);
    }
    // Deviation from the task brief: the brief's own code guarded here
    // against the classic hardware trap on a raw `i64::MIN / -1` (the
    // mathematical quotient `2^63` doesn't fit `i64`, and Rust's checked
    // `/`/`%` themselves panic/trap on that exact pair). That guard is
    // unreachable dead code under D-061/D-141's encoded representation:
    // `a`/`b` here are already decoded from a valid inline int-compatible
    // argument. Smallints decode within D-061's 63-bit range and bool markers
    // decode to `0`/`1`, so neither can equal `i64::MIN`. `cargo llvm-cov`'s
    // region coverage confirmed
    // this empirically: the removed branch's body never executed under
    // any test, including one written specifically to try to hit it.
    // The `fits_smallint` check below still catches the *actual*
    // reachable promotion case: floor-dividing the minimum taggable value
    // by `-1` negates it, producing exactly one more than the maximum
    // taggable magnitude (see
    // `pycc_rt_int_floordiv_promotes_the_negated_minimum_taggable_value`).
    let q = a / b;
    let r = a % b;
    let floored = if r != 0 && (r < 0) != (b < 0) {
        q - 1
    } else {
        q
    };
    fits_smallint(floored).unwrap_or_else(|| tag_bigint(bigint_from_i128(floored as i128)))
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_floordiv(a: i64, b: i64) -> i64 {
    int_floordiv(a, b)
}

fn int_floormod(a: i64, b: i64) -> i64 {
    let Some(a) = decode_inline_or_raise(a, "computing the modulo of") else {
        return tag_smallint(0);
    };
    // Same `b == 0` collision as `int_floordiv`; same reason to return here.
    let Some(b) = decode_inline_or_raise(b, "computing the modulo of") else {
        return tag_smallint(0);
    };
    if b == 0 {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "integer modulo by zero",
        );
        return tag_smallint(0);
    }
    // Deviation from the task brief: the brief's own code special-cased
    // `a == i64::MIN && b == -1` here (mirroring `int_floordiv`'s
    // original guard) to sidestep the same raw `%` hardware trap. Under
    // D-061/D-141 encoded representation this is unreachable for the
    // same reason `int_floordiv`'s removed guard was (see its comment):
    // a decoded inline operand can never equal
    // `i64::MIN`. Floor-mod's *result* can't overflow the taggable range
    // either -- unlike floor-division, which the comment on
    // `int_floordiv` explains can: floor-mod's result always satisfies
    // `|result| < |b|`, and every decoded inline `b` satisfies `|b| <=
    // 2^62` (D-061's 63-bit range), so `floored` always re-fits and the
    // `fits_smallint` round-trip check the brief had here (like
    // `int_floordiv`'s) is provably always-`Some` -- confirmed by
    // `cargo llvm-cov`: its `None` arm never executed under any test.
    // `tag_smallint` alone is therefore correct and simpler.
    let r = a % b;
    let floored = if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    };
    tag_smallint(floored)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_floormod(a: i64, b: i64) -> i64 {
    int_floormod(a, b)
}

fn int_pow(base: i64, exp: i64) -> i64 {
    // The decoded base is discarded -- the loop below squares the *encoded*
    // word through `int_mul` -- but the check is not: without it a bigint
    // base would reach `int_mul` and raise a second, redundant
    // `OverflowError` from inside the loop.
    if decode_inline_or_raise(base, "exponentiating").is_none() {
        return tag_smallint(0);
    }
    // Returning before the `exp < 0` arm below keeps its `RuntimeError` from
    // reporting over this `OverflowError`. See `decode_inline_or_raise`.
    let Some(mut exp) = decode_inline_or_raise(exp, "exponentiating") else {
        return tag_smallint(0);
    };
    if exp < 0 {
        // Part A of #1038 (#1063): a D-173 raise, not an abort. `RuntimeError`
        // rather than a CPython-conformant class because there is no
        // conformant class to name -- CPython computes `2 ** -1` as `0.5` and
        // raises nothing. The deviation is the pre-existing
        // `pycc_types::numeric_result_type` simplification recorded in the
        // message, not a new one; it is only now observable as an exception
        // instead of a process abort. See D-244's 2026-09-13 amendment.
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "negative exponent for `int ** int` is not supported \
             (the real result would need to be `float`, matching CPython's \
             own `int ** int` rule -- a pre-existing pycc_types simplification: \
             pycc_types::numeric_result_type always types `**` as \
             `int`-returning)",
        );
        return tag_smallint(0);
    }
    let mut result = tag_smallint(1);
    // Both variables are released unconditionally below, with no ownership
    // flag: `result` starts as a smallint and `base` was just proved inline by
    // `decode_inline_or_raise`, so a word that classifies as a bigint in either
    // one can only be an object `int_mul` freshly allocated here and this
    // function therefore owns. `bigint_release` is a no-op on every inline
    // kind, so releasing the caller's own untouched word is well defined.
    //
    // Retiring them matters only since Part C of #1038 (#1065) replaced the
    // abort with a raise: before it, the aborting process reclaimed everything.
    // This is not the general temporary-ownership model -- unbound arithmetic
    // temporaries still leak, which stays #146 Part 2 (#625).
    let mut base = base;
    while exp > 0 {
        if exp & 1 == 1 {
            let next = int_mul(result, base);
            // `int_mul` reads its operands without consuming them, so the
            // previous word is still this function's to release.
            bigint_release(result);
            result = next;
        }
        exp >>= 1;
        if exp > 0 {
            let next = int_mul(base, base);
            bigint_release(base);
            base = next;
        }
        // A promoted operand makes the *next* `int_mul` raise. Stop there
        // rather than squaring on and installing the same `OverflowError`
        // again, and retire both temporaries before returning the sentinel.
        if pycc_rt_exception_active() != 0 {
            bigint_release(result);
            bigint_release(base);
            return tag_smallint(0);
        }
    }
    bigint_release(base);
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_pow(base: i64, exp: i64) -> i64 {
    int_pow(base, exp)
}

fn int_cmp(a: i64, b: i64) -> i32 {
    // `0` is this function's sentinel: it is an ordinary value of the `i32`
    // ordering it returns (`Ordering::Equal`), not a D-141 encoded word.
    let Some(a) = decode_inline_or_raise(a, "comparing") else {
        return 0;
    };
    let Some(b) = decode_inline_or_raise(b, "comparing") else {
        return 0;
    };
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_cmp(a: i64, b: i64) -> i32 {
    int_cmp(a, b)
}

fn int_print(tagged: i64) {
    if is_smallint(tagged) {
        pycc_rt_print_i64(untag_smallint(tagged));
        return;
    }
    let s = int_to_str(tagged);
    println!("{}", String::from_utf8_lossy(unsafe { &*s }.bytes()));
    unsafe { pycc_rt_str_decref(s) };
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_print(tagged: i64) {
    int_print(tagged);
}

/// # Safety
/// Called only from pycc-generated code with a plain i64 argument; no
/// pointers involved, so there is nothing for the caller to uphold beyond
/// standard `extern "C"` calling-convention correctness.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_i64(value: i64) {
    print!("{}", format_i64_line(value));
}

/// `int`'s truthiness for `if`/`while` conditions (Task 4, D-141). Valid
/// smallints and bool-identity markers are decoded inline; valid bigint
/// pointers inspect their magnitude. A malformed encoded word is an internal
/// ABI violation and fails closed at this C boundary rather than being
/// dereferenced as a pointer.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_truthy(tagged: i64) -> i8 {
    if let Some(value) = inline_int_value(tagged) {
        return i8::from(value != 0);
    }
    // A bigint can now legitimately be zero (Task 9's `bigint_add_signed`
    // "equal magnitude, opposite sign" case), so the old "any bigint tag
    // is truthy" shortcut (Task 4, before real bigint values existed) is
    // no longer correct -- must inspect the actual magnitude.
    let b = unsafe { bigint_ref(tagged) };
    i8::from(!(b.limbs.len() == 1 && b.limbs[0] == 0))
}

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
fn range_continue(i: i64, stop: i64, step: i64) -> i8 {
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
fn range_normalize_operand(encoded: i64) -> i64 {
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

/// Converts an encoded int-compatible value (D-061/D-141) to `f64` -- the
/// `int` half of Python's `int`/`float` arithmetic promotion (Task 6).
/// Split into a private ordinary-ABI function plus the thin `extern "C"`
/// wrapper below, matching `int_add`/`range_continue`. Part C of #1038
/// ([#1065](https://github.com/rotnov/pycc/issues/1065)) turned the bigint
/// case from a panic into a D-173 raise, so the split no longer guards an
/// unwind-across-`extern "C"` abort; it stays because this crate's own
/// tests call the private function directly, as the converted tests below
/// do.
fn int_to_float(tagged: i64) -> f64 {
    match decode_inline_or_raise(tagged, "converting") {
        Some(value) => value as f64,
        None => 0.0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_to_float(tagged: i64) -> f64 {
    int_to_float(tagged)
}

/// D-141 checked decoder for an int-compatible ABI word. Ordinary smallints
/// decode to their numeric value, while the exact `False`/`True` markers
/// decode to `0`/`1`. Bigints and malformed words fail closed. Generated
/// code uses the numeric result for container indices, slice bounds, and
/// range normalization. Container value ingress calls this function only to
/// validate the word, then stores the original encoded word unchanged so a
/// bool marker keeps its runtime identity.
///
/// --- Implementation note / deviation from the task brief ---------------
///
/// The brief's own Step 4 code makes this a single plain `extern "C" fn`,
/// with a `#[should_panic]` test calling that same public wrapper directly
/// for the bigint case. Per this crate's established convention (see the
/// implementation-note comment above `int_add`, discovered empirically
/// during Task 3, and repeated for `range_continue` and `int_to_float`): a
/// panic that unwinds past a plain `extern "C" fn`'s own boundary is caught
/// right there and turned into a process abort, regardless of who calls it
/// -- including this crate's own same-binary Rust tests, which would
/// `SIGABRT` rather than let `#[should_panic]` catch anything. This
/// function can panic, so it gets the same split every other panicking
/// `pycc_rt_int_*` function already has: a private, ordinary-Rust-ABI
/// function holding the real logic, and a thin `pub extern "C"` wrapper of
/// the exact brief-specified name and signature for Task 11b's generated
/// code to call unchanged.
fn int_untag_checked(tagged: i64) -> i64 {
    match inline_int_value(tagged) {
        Some(value) => value,
        None => {
            panic!("pycc_rt: int boundary does not support bigint-valued values yet")
        }
    }
}

/// # Safety
/// None -- takes no pointer, only an `i64`. (The panic-across-FFI note on
/// `pycc_rt_int_add` applies to this wrapper as well.)
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_untag_checked(tagged: i64) -> i64 {
    int_untag_checked(tagged)
}

/// Materializes a source-level `int` literal (or an `enum` member's
/// discriminant) as an encoded `int` word: the tagged smallint when the
/// value round-trips through D-061's 63-bit tagged encoding, and a heap
/// `BigIntObj` pointer otherwise. `pycc_codegen` folds the tagged case at
/// compile time and only emits a call to this function for the values it
/// cannot fold, but the function itself handles both so the ABI contract
/// is total (D-178).
///
/// `bigint_from_i128` takes an `i128`, which is the only reason for the
/// widening cast -- `i64::MIN` has no magnitude problem here.
///
/// Never panics, so, per this file's established convention (see the
/// implementation note above `pycc_rt_int_add` and the one on
/// `pycc_rt_int_list_new`), it needs no private-logic/public-wrapper
/// split.
///
/// # Safety
/// None -- takes no pointer, only an `i64`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_from_i64(v: i64) -> i64 {
    fits_smallint(v).unwrap_or_else(|| tag_bigint(bigint_from_i128(v as i128)))
}

/// Python true division rejects both positive and negative zero divisors.
/// D-173 (#382): a zero divisor now sets the pending exception state and
/// returns 0.0 instead of panicking. Generated code checks
/// `pycc_rt_exception_active()` after this call.
fn float_div(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float division by zero",
        );
        return 0.0;
    }
    a / b
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_div(a: f64, b: f64) -> f64 {
    float_div(a, b)
}

/// CPython-compatible float floor-division and modulo. Computing both from
/// the same adjusted remainder avoids the off-by-one quotient produced by a
/// naive `(a / b).floor()` for values such as `1.0 // 0.1`.
fn float_divmod(a: f64, b: f64) -> (f64, f64) {
    if b == 0.0 {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(
            EXCEPTION_TYPE_ZERO_DIV_ERROR,
            "ZeroDivisionError",
            "float floor division or modulo by zero",
        );
        return (0.0, 0.0);
    }

    let mut modulo = a % b;
    let mut div = (a - modulo) / b;
    if modulo != 0.0 {
        if (b < 0.0) != (modulo < 0.0) {
            modulo += b;
            div -= 1.0;
        }
    } else {
        modulo = 0.0_f64.copysign(b);
    }

    let floordiv = if div != 0.0 {
        let mut floored = div.floor();
        if div - floored > 0.5 {
            floored += 1.0;
        }
        floored
    } else {
        0.0_f64.copysign(a / b)
    };
    (floordiv, modulo)
}

/// Python's `//` on `float`: floors toward negative infinity and snaps the
/// quotient the same way CPython does after floating-point remainder error.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_floordiv(a: f64, b: f64) -> f64 {
    float_divmod(a, b).0
}

/// Python's `%` on `float`: result takes the divisor's sign, including
/// signed zero.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_floormod(a: f64, b: f64) -> f64 {
    float_divmod(a, b).1
}

/// Python's `**` on `float`: unlike `int_pow`, a negative exponent is
/// perfectly ordinary here (`2.0 ** -1 == 0.5`). But three domains where
/// `f64::powf` silently returns an IEEE-754 special value diverge from
/// Python, which raises or produces a complex result instead (verified against
/// `python3.13`):
/// zero raised to a negative power (`ZeroDivisionError`), a negative base
/// raised to a non-integer power (a complex result -- `pycc` has no
/// complex type), and a finite base/exponent pair whose true result
/// overflows `float` range (`OverflowError`). Part A of #1038 (#1063) turns
/// each into a D-173 raise instead of a panic, matching this crate's
/// division-by-zero convention above; every raising arm returns `0.0` as its
/// sentinel. Generated code *can* observe that sentinel: `Pow` stays in
/// `pycc_codegen::exception::expression_can_set_exception`'s infallible arm,
/// so no check is emitted after the `**` itself and the pending exception is
/// seen only at the next enclosing checkpoint -- a statement or more later, or
/// at program exit. D-244's 2026-09-13 native-mode amendment records that
/// residual and why it is deferred. The sentinel `return` is load-bearing
/// rather than cosmetic: without it the zero-base arm would fall through to
/// `powf`, produce `inf`, and have its `ZeroDivisionError` immediately
/// relabelled `OverflowError` by the third arm in the same call.
fn float_pow(a: f64, b: f64) -> f64 {
    // CPython delegates non-finite exponent/base domains to libm: for
    // example, `(-1.0) ** inf == 1.0`, `0.0 ** -inf == inf`, and
    // `(-inf) ** 0.5 == inf`. The explicit exception/complex guards apply
    // only to finite operands; `fract()` is NaN for an infinite or NaN
    // exponent and would otherwise misclassify those ordinary real results.
    if b.is_finite() {
        if a == 0.0 && b < 0.0 {
            // Conformant: CPython raises `ZeroDivisionError` here, with this
            // exact sentence.
            raise_builtin(
                EXCEPTION_TYPE_ZERO_DIV_ERROR,
                "ZeroDivisionError",
                "0.0 cannot be raised to a negative power",
            );
            return 0.0;
        }
        if a.is_finite() && a < 0.0 && b.fract() != 0.0 {
            // A deliberate deviation: CPython returns a `complex` here, and
            // pycc has no complex type, so there is no conformant class to
            // name. See D-244's 2026-09-13 amendment.
            raise_builtin(
                EXCEPTION_TYPE_RUNTIME_ERROR,
                "RuntimeError",
                "a negative float raised to a non-integer power is not supported yet (would require a complex result)",
            );
            return 0.0;
        }
    }
    let result = a.powf(b);
    if result.is_infinite() && a.is_finite() && b.is_finite() {
        // Conformant: CPython raises `OverflowError` here.
        raise_builtin(
            EXCEPTION_TYPE_OVERFLOW_ERROR,
            "OverflowError",
            "float power overflowed (result too large to represent)",
        );
        return 0.0;
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_pow(a: f64, b: f64) -> f64 {
    float_pow(a, b)
}

/// D-059's `str` representation: up to 22 bytes are stored inline directly
/// in the `PyStrObj` allocation itself (no separate heap allocation for the
/// byte payload); anything longer heap-allocates a second, separate byte
/// buffer. Either way, `PyStrObj` itself (its refcount included) is always
/// exactly one heap allocation -- `pycc_codegen` never sees anything but an
/// opaque pointer to it (the same ABI-avoidance principle D-061 already
/// applies to `int`'s `BigInt`: no struct ever crosses the LLVM/Rust
/// boundary by value).
enum PyStrPayload {
    /// `(bytes, len)` -- only the first `len` bytes of `bytes` are
    /// meaningful; the rest is unused padding.
    Inline([u8; 22], u8),
    Heap(Box<[u8]>),
}

// `pub`, not private: every `pycc_rt_str_*` function below is a public
// (`#[unsafe(no_mangle)] pub extern "C" fn`) FFI entry point taking/
// returning `*mut PyStrObj`, and rustc's `private_interfaces` lint (a hard
// error under this project's `-D warnings` clippy gate) correctly refuses a
// private type in a public signature. `PyStrObj` stays fully opaque to any
// real Rust caller anyway: both fields below stay private, so nothing
// outside this module can construct one or read its contents except
// through these functions -- the "opaque pointer" contract this file's own
// doc comments describe is a privacy-of-fields property, not a
// privacy-of-the-type-name one.
pub struct PyStrObj {
    rc: Cell<u32>,
    payload: PyStrPayload,
}

impl PyStrObj {
    fn bytes(&self) -> &[u8] {
        match &self.payload {
            PyStrPayload::Inline(buf, len) => &buf[..*len as usize],
            PyStrPayload::Heap(b) => b,
        }
    }
}

/// Net count of live `PyStrObj` allocations: incremented by [`new_pystr`],
/// the crate's single construction site, and decremented on the one path in
/// [`pycc_rt_str_decref`] that actually frees an object. A steady-state
/// value of zero across a sequence of calls is therefore exactly the
/// "no `str` was leaked" property issue #1054 is about.
///
/// `Relaxed` is the correct ordering here: the counter orders nothing else,
/// no reader infers the state of any other memory from it, and a probe reads
/// it from the same thread that made the calls it is measuring.
static STR_LIVE: AtomicI64 = AtomicI64::new(0);

/// The current value of the live-`PyStrObj` counter.
///
/// Deliberately **not** `#[cfg(test)]`-gated. Its consumer is a hosted D-244
/// `ext` module, which links the ordinary non-test `libpycc_rt.a`; a
/// test-only symbol would simply not exist there, so the probe in
/// `tests/issue_1054_ext_str_release.rs` could not resolve it. The cost of
/// exporting it unconditionally is one relaxed atomic per string allocation
/// and per string free, which no benchmark in this repository can resolve.
///
/// The value is only meaningful as a *difference* between two reads taken
/// around a known sequence of calls: interned or otherwise long-lived
/// objects legitimately keep it above zero.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_str_live_objects() -> i64 {
    STR_LIVE.load(Ordering::Relaxed)
}

/// Allocates a fresh `PyStrObj` with refcount `1`, choosing the inline or
/// heap `PyStrPayload` per D-059's 22-byte threshold. Shared by every
/// `pycc_rt_str_*` entry point below that constructs a brand-new string (a
/// literal, or a concatenation result) rather than merely operating on
/// already-existing ones.
fn new_pystr(bytes: &[u8]) -> *mut PyStrObj {
    let payload = if bytes.len() <= 22 {
        let mut buf = [0u8; 22];
        buf[..bytes.len()].copy_from_slice(bytes);
        PyStrPayload::Inline(buf, bytes.len() as u8)
    } else {
        PyStrPayload::Heap(bytes.to_vec().into_boxed_slice())
    };
    STR_LIVE.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(Box::new(PyStrObj {
        rc: Cell::new(1),
        payload,
    }))
}

/// Builds a `str` object from UTF-8 bytes the caller already holds.
///
/// Two callers pass through here. `pycc_codegen`'s `MirExpr::StringLiteral`
/// codegen (Task 7) hands it a compile-time literal's own constant global,
/// which is where the name comes from; the D-244 hosted `ext` shim's
/// `pycc_ext_unpack_str` hands it the borrowed UTF-8 buffer
/// `PyUnicode_AsUTF8AndSize` returned for an incoming CPython `str`. The
/// bytes therefore need not come from a compiled literal at all -- the only
/// standing requirement is the safety one below, plus that they be valid
/// UTF-8, which both callers already guarantee.
///
/// Unlike every
/// `pycc_rt_int_*` arithmetic/comparison function, this has no failure mode
/// to guard against (allocation failure aborts via Rust's global allocator
/// rather than unwinding) -- so, per this crate's established convention
/// (see the implementation note above `int_add`, and `pycc_rt_int_truthy`'s
/// own doc comment for the same reasoning), this does not need the
/// private-logic/public-wrapper split: nothing here ever unwinds, so there's
/// no abort-vs-catch distinction for a caller to trip over. The same
/// rationale applies to every other `pycc_rt_str_*` function below.
///
/// # Safety
/// `ptr` must point to at least `len` readable bytes -- true for every
/// `pycc_codegen`-emitted call site, which always passes a compile-time
/// string literal's own constant global and byte length together, and for
/// the `ext` shim, which passes the pointer and length
/// `PyUnicode_AsUTF8AndSize` wrote out together. The bytes are copied here,
/// so neither caller has to keep them alive past the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_from_literal(ptr: *const u8, len: i64) -> *mut PyStrObj {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    new_pystr(bytes)
}

/// Concatenates two `str` objects into a brand-new one (Python's `+` on
/// `str`, Task 7) -- never mutates either operand.
///
/// # Safety
/// `a`/`b` must be live `PyStrObj` pointers -- every `pycc_codegen` call
/// site only ever passes a value it just evaluated from a well-typed
/// `Ty::Str` expression.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_concat(a: *mut PyStrObj, b: *mut PyStrObj) -> *mut PyStrObj {
    let a_bytes = unsafe { &*a }.bytes();
    let b_bytes = unsafe { &*b }.bytes();
    let mut combined = Vec::with_capacity(a_bytes.len() + b_bytes.len());
    combined.extend_from_slice(a_bytes);
    combined.extend_from_slice(b_bytes);
    new_pystr(&combined)
}

/// Repeats a `str` object `count` times into a brand-new one (Python's `*`
/// on `str`, #575 / Part 2 of #123) -- never mutates the operand.
///
/// `count` arrives already decoded to a raw `i64`: `pycc_codegen` untags the
/// D-141-encoded operand with `pycc_rt_int_untag_checked` before the call,
/// exactly as it already does for every other raw runtime counter (list
/// index, slice bound, `range` step). That keeps the bigint rejection in the
/// one place D-141 puts it rather than duplicating the classifier here.
///
/// A non-positive `count` yields the empty string, matching CPython: `"ab" *
/// 0` and `"ab" * -3` are both `""`. Negative counts are reachable from real
/// source both as a folded literal (`-3`, since #602) and as `n = 0 - 3`, so
/// this is a live semantic path, not a defensive one.
///
/// Like every other `pycc_rt_str_*` function (see `pycc_rt_str_from_literal`'s
/// own doc comment), this needs no private-logic/public-wrapper split: it has
/// no failure mode that unwinds. An oversized result aborts inside the global
/// allocator, precisely as `pycc_rt_str_concat`'s own `Vec` growth already
/// does.
///
/// # Safety
/// `s` must be a live `PyStrObj` pointer -- every `pycc_codegen` call site
/// only ever passes a value it just evaluated from a well-typed `Ty::Str`
/// expression.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_repeat(s: *mut PyStrObj, count: i64) -> *mut PyStrObj {
    let bytes = unsafe { &*s }.bytes();
    if count <= 0 {
        return new_pystr(b"");
    }
    new_pystr(&bytes.repeat(count as usize))
}

/// Lexicographic byte-wise ordering (Task 7's `str` comparison codegen) --
/// `-1`/`0`/`1`, matching `pycc_rt_int_cmp`'s own convention.
///
/// # Safety
/// Same as `pycc_rt_str_concat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_cmp(a: *mut PyStrObj, b: *mut PyStrObj) -> i32 {
    match unsafe { &*a }.bytes().cmp(unsafe { &*b }.bytes()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `str`'s truthiness for `if`/`while` conditions (Task 7, mirrors
/// `pycc_rt_int_truthy`'s own doc comment): `False` only for the empty
/// string, matching CPython.
///
/// # Safety
/// `s` must be a live `PyStrObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_truthy(s: *mut PyStrObj) -> i8 {
    i8::from(!unsafe { &*s }.bytes().is_empty())
}

/// #146 Part 1's refcounting for heap bigints: adds one live reference to
/// the D-141 encoded int word `word`.
///
/// A thin `extern "C"` wrapper over `int_encoding::bigint_retain`, split
/// exactly like `pycc_rt_range_continue`/`range_continue`: an `extern "C"`
/// function may not unwind, so the classification panic on an invalid word
/// lives in the private function, which unit tests can call directly.
///
/// Word `0` and every inline kind (smallints, the two bool-identity
/// markers) are no-ops. `pycc_codegen` additionally emits an inline
/// `(word & 3) == 0 && word != 0` test before every call, so an ordinary
/// smallint loop never reaches this symbol at all -- that guard is what
/// keeps D-084/D-140's nbody throughput floor untouched by this change.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bigint_retain(word: i64) {
    bigint_retain(word);
}

/// #146 Part 1's refcounting for heap bigints: drops one live reference
/// from `word`, freeing the `BigIntObj` when the last one goes away.
/// Same wrapper split, same no-op cases, and the same inline codegen guard
/// as `pycc_rt_bigint_retain` above.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bigint_release(word: i64) {
    bigint_release(word);
}

/// D-060's unconditional refcounting for `str`: increments `s`'s refcount by
/// one. A no-op on a null pointer -- not something Task 7's own codegen
/// ever actually passes, but a documented, tested part of this function's
/// contract nonetheless (mirroring how a null check is cheap insurance
/// against any future caller that binds a `str` local before it's ever
/// assigned).
///
/// # Safety
/// `s` must be either a null pointer or a live `PyStrObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_incref(s: *mut PyStrObj) {
    if s.is_null() {
        return;
    }
    let obj = unsafe { &*s };
    obj.rc.set(obj.rc.get() + 1);
}

/// D-060's unconditional refcounting for `str`: decrements `s`'s refcount by
/// one, freeing the allocation once it reaches zero. A no-op on a null
/// pointer, same rationale as `pycc_rt_str_incref` above.
///
/// # Safety
/// Same as `pycc_rt_str_incref`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_str_decref(s: *mut PyStrObj) {
    if s.is_null() {
        return;
    }
    let new_rc = unsafe { &*s }.rc.get() - 1;
    if new_rc == 0 {
        STR_LIVE.fetch_sub(1, Ordering::Relaxed);
        drop(unsafe { Box::from_raw(s) });
    } else {
        unsafe { &*s }.rc.set(new_rc);
    }
}

/// A `str` object's UTF-8 bytes, writing its length through `len`.
///
/// This is the `str` egress half of the D-244 hosted `ext` boundary: the
/// generated wrapper's `pycc_ext_pack_str` copies these bytes into a CPython
/// `str` with `PyUnicode_FromStringAndSize`, then releases the object. It
/// exists because [`PyStrObj`]'s payload is deliberately opaque -- the inline
/// and heap arms of `PyStrPayload` are not a layout the C shim may read --
/// and it mirrors [`crate::exception::pycc_rt_ext_pending_message`], which
/// hands the shim an exception message the same way.
///
/// The returned pointer borrows `s`'s own storage, so it stays valid exactly
/// as long as `s` does. The length is written out separately rather than
/// implied by a NUL terminator because a pycc `str` may contain embedded NUL
/// bytes, and the empty string is a legitimate value.
///
/// The `ext` shim never calls this with a null `s` (it guards first, where
/// nothing is instrumented), so there is no null arm here.
///
/// # Safety
///
/// `s` must be a live `PyStrObj` pointer, and `len` must be non-null and
/// point to a writable, aligned `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_str_bytes(s: *mut PyStrObj, len: *mut usize) -> *const u8 {
    let bytes = unsafe { &*s }.bytes();
    unsafe { *len = bytes.len() };
    bytes.as_ptr()
}

// --- Implementation note / deviation from the task brief -------------
//
// The brief's own version of `pycc_rt_int_to_str`/`pycc_rt_float_to_str`
// gave each straight to a single `#[unsafe(no_mangle)] pub extern "C" fn`
// (no private-logic/public-wrapper split). Both can panic --
// `int_to_str` via the encoded-int classifier's rejection path (not yet
// exercised by any test in *this* task, since Task 9 is what first makes a
// bigint-valued tagged `int` reachable, but still a real panicking path
// today), `float_to_str` via its own scientific-notation-range rejection
// (directly exercised by this task's own tests) -- and both are plain
// `extern "C" fn`s, not `extern "C-unwind"`. Per this file's own
// established convention (see the implementation note above `int_add`,
// and this crate's project-wide rule that any new `pycc_rt` `extern "C"
// fn` that can panic needs this split), calling either public wrapper
// directly from ordinary same-crate Rust test code would abort the whole
// test binary (`SIGABRT`) instead of letting `#[should_panic]` catch an
// ordinary unwind -- confirmed empirically: `pycc_rt_float_to_str_rejects_
// magnitudes_needing_scientific_notation` aborted the test binary before
// this split existed. Fixed the same way as `int_add`/`pycc_rt_int_add`:
// a private, ordinary-Rust-ABI function holding the real logic (freely
// panics, unwinds normally) and a thin `#[unsafe(no_mangle)] pub extern
// "C" fn` wrapper of the exact name/signature the brief specifies.
// `bool_to_str` has no failure mode (same reasoning as `pycc_rt_str_from_
// literal`'s own doc comment) so it keeps the brief's single-function
// shape unchanged.

/// Formats an encoded int-compatible value the way CPython's own `str(n)`
/// would (Task 8/D-141) --
/// reused unchanged by f-string interpolation and Task 10's `print`. Shares
/// `format_i64_line`'s digit-formatting logic with `pycc_rt_print_i64`
/// rather than duplicating it, trimming the trailing newline that function
/// adds for its own (unrelated) purpose.
fn int_to_str(tagged: i64) -> *mut PyStrObj {
    match classify_encoded_int(tagged) {
        EncodedIntKind::SmallInt => new_pystr(
            format_i64_line(untag_smallint(tagged))
                .trim_end()
                .as_bytes(),
        ),
        EncodedIntKind::BoolFalse => new_pystr(b"False"),
        EncodedIntKind::BoolTrue => new_pystr(b"True"),
        EncodedIntKind::BigInt => {
            let b = unsafe { bigint_ref(tagged) };
            new_pystr(bigint_to_decimal_string(b.negative, &b.limbs).as_bytes())
        }
    }
}

/// See the panic-across-FFI note above `pycc_rt_int_add`: this crosses no
/// FFI boundary on its own successful-return tests, but a real
/// bigint-valued `tagged` (Task 9) would panic here, so this stays a thin
/// wrapper around `int_to_str`'s own freely-panicking logic rather than
/// housing that logic directly.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_to_str(tagged: i64) -> *mut PyStrObj {
    int_to_str(tagged)
}

/// Formats a `bool` the way CPython's own `str(b)` would: capitalized
/// `"True"`/`"False"`, never Rust's own lowercase `Display` spelling (Task
/// 8, reused unchanged by f-string interpolation and Task 10's `print`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bool_to_str(value: i8) -> *mut PyStrObj {
    new_pystr(if value != 0 { b"True" } else { b"False" })
}

/// Formats a `float` the way CPython's own `str(f)` would (Task 8, reused
/// unchanged by f-string interpolation and Task 10's `print`).
///
/// Verified against `python3.13`'s actual `repr(float)`/`str(float)`
/// (identical since Python 3.1): CPython switches to scientific
/// notation once the value's magnitude is `>= 1e16` or (nonzero and)
/// `< 1e-4`; within that range it always shows at least one digit after
/// the decimal point (`3.0`, never bare `3`, unlike Rust's own `{}`
/// `Display` for `f64`), and `inf`/`-inf`/`nan` are lowercase (Rust's
/// own `Display` capitalizes `NaN`). Reproducing CPython's scientific
/// notation formatting exactly is out of scope for this task -- Part B of
/// #1038 (#1064) raises a catchable `RuntimeError` for that narrow range
/// rather than returning a silently wrong digit string (a documented,
/// named gap, same convention as D-026/D-043; the remaining conformance
/// work is tracked as #1071).
fn float_to_str(value: f64) -> *mut PyStrObj {
    if value.is_nan() {
        return new_pystr(b"nan");
    }
    if value.is_infinite() {
        return new_pystr(if value > 0.0 { b"inf" } else { b"-inf" });
    }
    let magnitude = value.abs();
    // clippy's `manual_range_contains`: `!(1e-4..1e16).contains(&magnitude)`
    // is `magnitude < 1e-4 || magnitude >= 1e16` (a `Range` is
    // start-inclusive, end-exclusive) -- the same condition as the task
    // brief's own `magnitude >= 1e16 || magnitude < 1e-4`, just reordered.
    if magnitude != 0.0 && !(1e-4..1e16).contains(&magnitude) {
        // Part B of #1038 (#1064): a D-173 raise, not an abort. The sentinel
        // is an empty `str` -- a *valid* `PyStrObj`, never null, so a caller
        // that touches it before its own pending-exception check is safe.
        // `raise_builtin` copies `msg` immediately, so a borrowed `format!`
        // temporary is fine here.
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            &format!(
                "formatting a float this large or small ({value}) needs \
                 scientific notation, which is not supported yet"
            ),
        );
        return new_pystr(b"");
    }
    let text = format!("{value}");
    let text = if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    };
    new_pystr(text.as_bytes())
}

/// Sets the pending `RuntimeError` exception flag (D-173) and returns an
/// empty `str` when `value` needs scientific notation; the caller's
/// generated code checks the flag after this call.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_float_to_str(value: f64) -> *mut PyStrObj {
    float_to_str(value)
}

/// Writes a `PyStrObj`'s bytes to stdout with no trailing newline (Task 10)
/// -- `print`'s new fully-general dispatch converts every argument to a
/// `str` via `to_str` first (reusing `pycc_rt_int_to_str`/`float_to_str`/
/// `bool_to_str`) and writes each one with this, separated by
/// `pycc_rt_print_space` and finished by `pycc_rt_print_newline`. Distinct
/// from Task 3's `pycc_rt_int_print`, which stays newline-inclusive and
/// int-only and is no longer called by `pycc_codegen`'s print dispatch
/// (still exercised by its own direct unit tests below). Never panics --
/// `String::from_utf8_lossy` cannot fail -- so this needs no
/// private-logic/public-wrapper split (same reasoning as `pycc_rt_str_from_
/// literal`'s own doc comment).
///
/// Deviation from the task brief: the brief's own version of this function
/// signature was a plain (non-`unsafe`) `pub extern "C" fn`, matching its
/// dereference of `s` (`*s`) inside its own internal `unsafe { }` block.
/// That doesn't compile clean under this crate's `-D warnings` clippy gate
/// -- `clippy::not_unsafe_ptr_arg_deref` (`#[deny]`d by default) rejects
/// exactly this shape: a public function taking a raw pointer and
/// dereferencing it without the function itself being `unsafe`. Every other
/// function in this file that dereferences a `*mut PyStrObj`
/// (`pycc_rt_str_from_literal`/`_concat`/`_cmp`/`_truthy`/`_incref`/
/// `_decref`) is already `pub unsafe extern "C" fn` for exactly this
/// reason; fixed the same way here, and documented with its own `# Safety`
/// section per that same established convention.
///
/// # Safety
/// `s` must be a live `*mut PyStrObj` previously returned by one of this
/// crate's own str-producing functions (same contract as
/// `pycc_rt_str_incref`/`pycc_rt_str_decref`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_print_write_str(s: *mut PyStrObj) {
    print!("{}", String::from_utf8_lossy(unsafe { &*s }.bytes()));
}

/// Prints a single space with no newline (Task 10) -- `print`'s separator
/// between arguments, matching CPython's default `sep=" "`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_space() {
    print!(" ");
}

/// Prints `print`'s single trailing newline (Task 10), matching CPython's
/// default `end="\n"`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_newline() {
    println!();
}

/// Prints the literal `None`, capitalized, with no trailing newline (Task 10)
/// -- CPython's `str(None)` -- for any supported materializable non-`print()`
/// `Ty::None` expression. This includes direct user-function, `ListAppend`,
/// and `SetAdd` results, D-075 parameter values, and values loaded from
/// ordinary assignment storage. `None` is an unboxed canonical unit carrier
/// rather than a `PyStrObj`, so there is no `none_to_str` allocation to route
/// through `pycc_rt_print_write_str` instead.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_none() {
    print!("None");
}

/// `list[int]`'s runtime object (D-105: v0.2 codegen only supports
/// `list[int]`; any other element type stays a `pycc_types` diagnostic,
/// T0034, never reaching codegen). Header shape follows `PyStrObj`'s real
/// precedent (`rc: Cell<u32>` plus payload) rather than `docs/RUNTIME.md`'s
/// former stale 16-byte generic-header spec (corrected by this plan's
/// Task 12) -- `PyStrObj` is this runtime's only other *refcounted* heap
/// object, and it never had a `type_id`/`flags` field either. (`BigIntObj`
/// in `int_encoding` is a third heap-allocated type -- `Box::into_raw`'d by
/// `tag_bigint` -- and since #146 Part 1 it carries the same `rc: Cell<u32>`
/// header shape, released at the named-storage and loop-induction sites
/// that change enumerates.)
///
/// No `#[repr(C)]`: exactly like `PyStrObj` (see that struct's own doc
/// comment), `pycc_codegen` never sees anything but an opaque pointer to
/// this type and only ever calls the `pycc_rt_int_list_*` functions below
/// on it -- no struct ever crosses the LLVM/Rust boundary by value, so
/// there is no ABI layout to pin down.
///
/// The growable payload is a safe `Cell<Vec<i64>>`, not a raw pointer with
/// manual `std::alloc`/`realloc`/`dealloc` calls: `Vec<i64>` already gets
/// growth, bounds-safe indexing, and (critically) its own `Drop` glue for
/// free, the same way `PyStrObj`'s `Box<[u8]>` heap payload does. A
/// `RefCell<Vec<i64>>` was considered and rejected -- its runtime
/// borrow-checking panics on any overlapping borrow, a new panic mode this
/// object's callers (future LLVM-generated code with call patterns not
/// fully under this file's control) could trip with no relation to Python
/// semantics. `Cell::take`/`Cell::set` avoid that: `take` moves the `Vec`
/// out (leaving an empty one behind) without holding a borrow, the caller
/// mutates the owned value, then `set` moves it back in -- no borrow is
/// ever held across the mutation, so there is nothing for a runtime check
/// to reject.
///
/// `pub`, not private, for the same reason `PyStrObj` is `pub`: every
/// `pycc_rt_int_list_*` function below is a public FFI entry point
/// returning or taking `*mut PyIntListObj`, and rustc's
/// `private_interfaces` lint refuses a private type in a public
/// signature. Both fields stay private, so the "opaque pointer" contract
/// still holds for any real Rust caller.
///
/// **Element representation (D-141).** Each slot stores the int-compatible
/// encoded word unchanged: odd ordinary smallint, exact `2`/`6` bool marker,
/// or (once container bigints are supported) an aligned bigint pointer.
/// Current generated ingress validates every word with
/// `pycc_rt_int_untag_checked`, so bigint elements remain an explicit scope
/// cut, but it stores the original encoded word rather than the decoded
/// number. Reads, iteration, slicing, and pop therefore preserve `False`/
/// `True` identity. Lengths and indices remain raw implementation counters.
pub struct PyIntListObj {
    rc: Cell<u32>,
    items: Cell<Vec<i64>>,
}

/// Allocates a fresh, empty `PyIntListObj` with refcount `1`. Never
/// panics -- `Vec::new()` performs no allocation -- so, per this file's
/// established convention (see the implementation note above
/// `pycc_rt_int_add`), this needs no private-logic/public-wrapper split:
/// nothing here ever unwinds, so there is no abort-vs-catch distinction
/// for a caller to trip over.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_int_list_new() -> *mut PyIntListObj {
    Box::into_raw(Box::new(PyIntListObj {
        rc: Cell::new(1),
        items: Cell::new(Vec::new()),
    }))
}

/// Appends `value` to the end of `list` (Python's `list.append`, D-105's
/// v0.2 `list[int]` slice). Grows `list`'s backing `Vec` via its own
/// amortized-doubling `push`, so this never needs to reimplement
/// capacity-doubling by hand.
///
/// # Element representation
/// `value` is an encoded D-141 word and is stored exactly as given. The FFI
/// caller must first validate it with `pycc_rt_int_untag_checked`; generated
/// code does so and deliberately ignores the decoded result. This preserves
/// a bool marker while retaining the current explicit rejection of bigint
/// container elements.
///
/// # Safety
/// `list` must be a live `PyIntListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_append(list: *mut PyIntListObj, value: i64) {
    let list = unsafe { &*list };
    let mut items = list.items.take();
    items.push(value);
    list.items.set(items);
}

/// Private half of `pycc_rt_int_list_get` below. Its out-of-range path is
/// a D-173 pending-exception raise rather than an abort, so the split is
/// no longer about panic-across-FFI: it exists so this file's unit tests
/// can drive the raise and inspect the pending state without going through
/// the public `extern "C"` wrapper's raw pointer.
fn int_list_get(list: &PyIntListObj, index: i64) -> i64 {
    let items = list.items.take();
    let len = items.len();
    if index < 0 || index as usize >= len {
        // D-173: set the pending exception flag instead of panicking.
        // Restore the payload before returning (not required for
        // correctness in single-shot FFI usage, but cheap insurance).
        list.items.set(items);
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "list index out of range",
        );
        return tag_smallint(0);
    }
    let value = items[index as usize];
    list.items.set(items);
    value
}

/// Reads the element at `index` (Python's `list[index]`, D-105's v0.2
/// `list[int]` slice). Sets the pending `IndexError` exception flag on an
/// out-of-range index (D-173) and returns a sentinel `tag_smallint(0)` --
/// a *valid* D-141 encoded word, never a raw `0`, which
/// `classify_encoded_int` would reject; the caller's generated code checks
/// the flag after this call.
///
/// Known v0.2 scope cut: negative indices are not supported. Real Python
/// treats `lst[-1]` as the last element, but `pycc_types` has no way to
/// reject a negative index at compile time (the index value is only known
/// at runtime) -- so this raises `IndexError` on *any* negative index, the
/// same "index out of range" raise as a too-large positive one. This is a
/// deliberate, documented v0.2 gap (D-108), not a bug: it means a
/// conformance fixture exercising this function must not use negative
/// indexing, since that would raise here rather than matching CPython's
/// last-element behavior.
///
/// # Element representation
/// `index` is a raw container offset; generated code obtains it by decoding
/// an int-compatible expression with `pycc_rt_int_untag_checked`. The return
/// value is the stored D-141 encoded word unchanged and needs no conversion
/// before being used as an ordinary `Ty::Int` expression.
///
/// # Safety
/// `list` must be a live `PyIntListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_get(list: *mut PyIntListObj, index: i64) -> i64 {
    int_list_get(unsafe { &*list }, index)
}

/// The two words a `pycc build --ext` wrapper is allowed to carry across the
/// boundary for a `memoryview` parameter (Part 2 of #1027).
///
/// This is the Rust view of `PyccExtBufferView` in `src/ext/pycc_ext_module.c`
/// -- `{ void *ptr; long long len; }` -- and the layout must agree field for
/// field, which `ext_buffer_view_layout_matches_the_c_struct` below pins.
/// `len` is a *copy* of the exporter's `shape[0]`, taken while the buffer is
/// held; the owning `Py_buffer` and `PyObject *` stay wrapper-side, so nothing
/// here can outlive or reach them.
///
/// Compiled code never constructs one: the generated wrapper fills a stack
/// local and passes its address as the `Ty::MemoryView` argument.
#[repr(C)]
pub struct PyccExtBufferView {
    /// The exporter's data pointer, a C-contiguous one-dimensional `double`
    /// array of `len` elements (the `"d"` format the unpack shim requires).
    pub ptr: *mut core::ffi::c_void,
    /// A copy of the exporter's `shape[0]`, in elements, never in bytes.
    pub len: i64,
}

/// Private half of `pycc_rt_buffer_f64_get` below, split for the same reason
/// `int_list_get` is: so this file's unit tests can drive the out-of-range
/// raise and inspect the pending state directly.
///
/// # Safety
/// `view.ptr` must point at `view.len` contiguous `f64` values.
unsafe fn buffer_f64_get(view: &PyccExtBufferView, index: i64) -> f64 {
    if index < 0 || index >= view.len {
        // D-173: set the pending exception flag instead of panicking. The
        // message is CPython's own sentence for this failure, so a subject
        // that goes out of bounds reports what `memoryview.__getitem__`
        // would have reported.
        //
        // D-108: a negative index raises here rather than wrapping. The
        // deviation is exactly the range `[-len, -1]`, which CPython would
        // have resolved from the end.
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "index out of bounds on dimension 1",
        );
        return 0.0;
    }
    // The `--ext` wrapper admits an exporter on four properties --
    // writable-or-not, one-dimensional, C-contiguous, format `"d"` -- and
    // none of them implies that the exporter's storage is 8-byte aligned.
    // `memoryview(bytearray(17))[1:].cast("d")` satisfies every one of them
    // on CPython and reports a data address that is `1 mod 8`, so a plain
    // `*const f64` dereference here would be undefined behavior on a buffer
    // the boundary is required to accept. `read_unaligned` costs nothing on
    // an aligned address and keeps the unaligned exporter working rather
    // than turning it into a refusal.
    unsafe { core::ptr::read_unaligned((view.ptr as *const f64).add(index as usize)) }
}

/// Reads the element at `index` of a `pycc build --ext` export's `memoryview`
/// parameter (Python's `b[i]`, Part 2 of #1027), bounds-checked against the
/// `len` the wrapper copied out of the exporter's `shape[0]`.
///
/// Sets the pending `IndexError` flag (D-173) and returns a `0.0` sentinel on
/// an out-of-range index -- a type-valid `f64`, never a trap representation,
/// because the caller's generated code checks the flag after this call rather
/// than inspecting the value. The `--ext` wrapper's D-173 bridge releases the
/// buffer and re-raises it as a CPython `IndexError`.
///
/// # Element representation
/// `index` is a raw element offset; generated code obtains it by decoding an
/// int-compatible expression with `pycc_rt_int_untag_checked`. The return
/// value is an ordinary unencoded `Ty::Float`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView` whose `ptr` addresses `len`
/// contiguous `f64` values -- which is what the generated wrapper guarantees
/// for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_get(view: *const PyccExtBufferView, index: i64) -> f64 {
    unsafe { buffer_f64_get(&*view, index) }
}

/// The private half of [`pycc_rt_buffer_f64_set`], sharing
/// [`buffer_f64_get`]'s bounds rule verbatim so the load and the store can
/// never disagree about which indices are in range.
///
/// # Safety
/// `view.ptr` must address `view.len` contiguous, **writable** `f64` values
/// whenever `index` is in range. The generated `--ext` wrapper guarantees
/// that by acquiring the buffer with `PyBUF_WRITABLE` for exactly the
/// parameters the compiled body stores into (Part 1 of #1142).
unsafe fn buffer_f64_set(view: &PyccExtBufferView, index: i64, value: f64) {
    if index < 0 || index >= view.len {
        // The same two deviations the load carries, stated once per
        // operation rather than shared through a helper so neither can be
        // silently relaxed on one side: D-173's pending-exception flag in
        // place of a panic across the FFI boundary, and D-108's refusal of
        // a negative index instead of CPython's resolve-from-the-end.
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "index out of bounds on dimension 1",
        );
        return;
    }
    // Unaligned for the same reason the load is, and stated again rather
    // than shared so neither side can be relaxed alone: the four properties
    // the wrapper checks do not imply 8-byte alignment, and
    // `memoryview(bytearray(17))[1:].cast("d")` is a CPython buffer that
    // passes all four at a data address `1 mod 8`.
    unsafe { core::ptr::write_unaligned((view.ptr as *mut f64).add(index as usize), value) };
}

/// Writes `value` to the element at `index` of a `pycc build --ext`
/// export's `memoryview` parameter (Python's `b[i] = v`, Part 1 of #1142),
/// bounds-checked against the `len` the wrapper copied out of the
/// exporter's `shape[0]`.
///
/// The sibling of [`pycc_rt_buffer_f64_get`], and `*const` for the same
/// reason that one is: `PyccExtBufferView::ptr` is already `*mut c_void`,
/// so the view itself is only ever read and the mutability lives entirely
/// in the storage it points at.
///
/// Sets the pending `IndexError` flag (D-173) and returns **without
/// dereferencing `ptr`** on an out-of-range index. Unlike the load there is
/// no sentinel to return, so this is a `void` helper: the generated code's
/// statement-level guard is the only thing that ever observes the raise,
/// which is why `pycc_codegen`'s `MirStmt::BufferSet` arm calls
/// `guard_statement_effects` immediately after this call.
///
/// # Element representation
/// `index` is a raw element offset, obtained by decoding an int-compatible
/// expression with `pycc_rt_int_untag_checked`. `value` is an ordinary
/// unencoded `Ty::Float`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView` whose `ptr` addresses `len`
/// contiguous **writable** `f64` values -- which is what the generated
/// wrapper guarantees for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_set(
    view: *const PyccExtBufferView,
    index: i64,
    value: f64,
) {
    unsafe { buffer_f64_set(&*view, index, value) }
}

/// Returns the element count of a `pycc build --ext` export's `memoryview`
/// parameter (Python's `len(b)`, Part 4 of #1027).
///
/// This is the `len` word of the `PyccExtBufferView` the wrapper filled in --
/// a copy of the exporter's `shape[0]`, in elements and never in bytes -- so
/// it agrees with CPython's own `len(view)` for the one-dimensional `"d"`
/// buffers this artifact mode admits.
///
/// Unlike `pycc_rt_buffer_f64_get`, this cannot fail: there is no index to
/// range-check and no pending exception to set (D-173), which is why
/// `pycc_codegen`'s `expression_can_set_exception` answers `false` for the
/// node that calls it.
///
/// # Element representation
/// The returned count is a **raw, untagged** `i64`, matching
/// `pycc_rt_int_list_len`. Generated code re-tags it with D-141's
/// `raw_i64_to_tagged_int` before the value becomes a user-visible `Ty::Int`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView`, which is what the generated
/// wrapper guarantees for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_len(view: *const PyccExtBufferView) -> i64 {
    unsafe { (*view).len }
}

/// Net count of live artifact-owned [`PyccExtBufferView`] allocations:
/// incremented by [`pycc_rt_buffer_f64_alloc`] and decremented by the one
/// path in [`pycc_rt_buffer_f64_free`] that actually releases storage. A
/// steady-state value of zero across a sequence of calls is exactly the
/// "the compiled function freed every buffer it allocated" property Part 2a
/// of #1142 (#1165) is about.
///
/// `Relaxed` is the correct ordering for the same reason [`STR_LIVE`]'s is:
/// the counter orders nothing else, no reader infers the state of any other
/// memory from it, and a probe reads it from the same thread that made the
/// calls it is measuring.
///
/// Only *artifact-owned* storage moves this counter. A `memoryview`
/// **parameter**'s view is a wrapper-side stack local the artifact never
/// allocated and never frees, so it is invisible here -- which is what makes
/// a difference between two reads a statement about the producer alone.
static BUFFER_LIVE: AtomicI64 = AtomicI64::new(0);

/// The current value of the live artifact-owned-buffer counter.
///
/// Deliberately **not** `#[cfg(test)]`-gated, for the reason
/// [`pycc_rt_str_live_objects`] states in full: its consumer is a hosted
/// D-244 `ext` module, which links the ordinary non-test `libpycc_rt.a`, so
/// a test-only symbol would simply not exist there. The cost of exporting it
/// unconditionally is one relaxed atomic per buffer allocation and per
/// buffer free.
///
/// The value is only meaningful as a *difference* between two reads taken
/// around a known sequence of calls.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_live_views() -> i64 {
    BUFFER_LIVE.load(Ordering::Relaxed)
}

/// Decodes the user-supplied `ndarray(n)` length word for #1165's producer.
///
/// This is the one D-141 `int` ingress position that must **not** go
/// through `int_untag_checked`. That decoder `panic!`s on a bigint or
/// malformed word, and a panic unwinding past a plain `extern "C" fn`
/// boundary is caught there and turned into a process abort -- which, for a
/// D-244 `ext` artifact, kills the host CPython interpreter rather than
/// raising anything the host could catch. `make(2 ** 62 - 1)` on a built
/// `--ext` module exited 134 before this function existed.
///
/// So the length is decoded through `decode_inline_or_raise` instead: a
/// bigint leaves a pending `OverflowError` (D-173) and yields the
/// type-valid sentinel `0`. `int_untag_checked` itself is deliberately left
/// alone. Its other call sites -- a list index, a slice bound, a `str`
/// repeat count, container and comprehension element validation -- consume
/// the decoded word with no guard behind them, so returning `0` there would
/// turn a loud abort into a silent wrong answer; converting them needs each
/// site's own guard and is tracked separately (see #1168 and its follow-up).
/// This site is safe because `pycc_codegen` emits
/// `guard_statement_effects` between this call and
/// [`pycc_rt_buffer_f64_alloc`], so the sentinel never reaches the
/// allocator: the generated code branches to the installed exception target
/// first, which is also what keeps this raise from being reported over by
/// the allocator's own negative-length `ValueError`.
///
/// Bool markers decode to `0`/`1` exactly as `int_untag_checked` decodes
/// them, so `ndarray(True)` keeps its D-086 meaning.
///
/// Split into a private ordinary-ABI function plus the thin `extern "C"`
/// wrapper below, following this crate's convention (see the implementation
/// notes above `int_add`, `range_continue` and `int_to_float`). Here the
/// split is for this crate's own tests rather than to guard an unwind: this
/// function never panics, which is the entire point of it.
fn buffer_alloc_untag_len(tagged: i64) -> i64 {
    // `unwrap_or(0)` rather than `unwrap_or_default()`: the `0` is the
    // documented *type-valid sentinel* `decode_inline_or_raise`'s contract
    // requires, not an incidental default.
    decode_inline_or_raise(tagged, "sizing a buffer with").unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_alloc_untag_len(tagged: i64) -> i64 {
    buffer_alloc_untag_len(tagged)
}

/// Allocates artifact-owned storage for `len` `f64` elements and the
/// [`PyccExtBufferView`] that addresses it (Python's `ndarray(n)` /
/// `NDArray(n)`, Part 2a of #1142 -- issue #1165).
///
/// The returned pointer is the *view*, already pointing at the storage, so
/// compiled code keeps handling exactly the `*const PyccExtBufferView` that
/// [`pycc_rt_buffer_f64_get`], [`pycc_rt_buffer_f64_set`] and
/// [`pycc_rt_buffer_len`] already take: a produced buffer and a parameter
/// buffer are the same shape to codegen, and nothing new reaches its
/// calling convention.
///
/// # Zero-fill is a deliberate deviation from `numpy.ndarray(n)`
/// CPython's `numpy.ndarray(5)` returns *uninitialized* storage. This
/// zero-fills, on two grounds recorded in D-244's #1165 amendment: Part 2b
/// hands this storage to a host process, and uninitialized artifact heap
/// reaching one is an information-disclosure surface; and a deterministic
/// producer is what lets a conformance test assert anything at all about
/// the value before a store.
///
/// # Length
/// `len < 0` sets a pending `ValueError` (D-173) and returns null, matching
/// CPython's own `ValueError: negative dimensions are not allowed`. The
/// caller's generated code checks the pending flag after this call, and the
/// null is safe for the epilogue to hand straight to
/// [`pycc_rt_buffer_f64_free`]. `len == 0` is admitted and yields a
/// zero-length view, which every existing helper already handles.
///
/// Both the reservation and the `resize` narrow `len` to `usize`, while the
/// recorded `PyccExtBufferView.len` keeps the original `i64`. That is sound
/// only where `usize` is at least 64 bits wide: on a narrower target a large
/// `len` would truncate for the storage while the view still advertised the
/// full length, and the `i64`-against-`i64` bounds check in
/// [`pycc_rt_buffer_f64_get`] and [`pycc_rt_buffer_f64_set`]
/// (`index < 0 || index >= view.len`) would admit an index past the end of
/// that storage. Every Tier-1 target is 64-bit -- `docs/ROADMAP.md`'s
/// platform table lists Linux x64/arm64, macOS x64/arm64 and Windows x64 --
/// so the case is unreachable as built; a 32-bit target would have to refuse
/// a `len` past `usize::MAX` here rather than truncate it.
///
/// A length whose storage cannot be reserved -- `len * 8` past `isize::MAX`,
/// or a genuine allocator failure -- sets a pending `RuntimeError` and
/// returns null the same way. *Both* allocations this function performs are
/// fallible (`Vec::try_reserve_exact`), the elements and the two-word view
/// alike, precisely so that case cannot `panic!` or `handle_alloc_error`
/// across this `extern "C"` boundary and abort the host interpreter; see
/// the comment at the check for why the class is `RuntimeError` rather than
/// CPython's `MemoryError`. Under a *genuine* out-of-memory condition the
/// pending `RuntimeError` is itself built by `raise_builtin`, whose message
/// and exception objects are still ordinary infallible allocations shared
/// with every other raise site in this runtime; closing that is a
/// runtime-wide change tracked separately, so the guarantee this function
/// makes on its own is exact for the capacity-overflow case and
/// best-effort for a true allocator failure.
///
/// # Allocator pairing
/// The storage is a `Box<[f64]>` and the view is a one-element
/// `Box<[PyccExtBufferView]>`, and [`pycc_rt_buffer_f64_free`] reconstructs
/// *those same two boxes*. The pairing is stated as a `Box` round trip
/// rather than as a hand-built `core::alloc::Layout` precisely so the size
/// and alignment cannot drift between the two halves: a mismatched-layout
/// deallocation is undefined behavior that no test notices by accident.
/// That is also why the view is a slice box rather than a
/// `Box<PyccExtBufferView>` rebuilt from a `Vec`'s pointer: the two layouts
/// do agree today, but only the round trip keeps them agreeing by
/// construction.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_f64_alloc(len: i64) -> *mut PyccExtBufferView {
    if len < 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "negative dimensions are not allowed",
        );
        return core::ptr::null_mut();
    }
    // #1166 round 8. `vec![0.0f64; len]` `panic!`s when `len * 8` exceeds
    // `isize::MAX` (and aborts outright on a genuine allocator failure), and
    // a panic unwinding past this `extern "C"` boundary is caught here and
    // turned into a process abort -- the host CPython interpreter's, for a
    // D-244 `ext` artifact. `ndarray(2 ** 62 - 1)` exited 134 that way, with
    // no bigint anywhere in it: the length is an ordinary inline smallint,
    // so the length *decoder* cannot be what catches this.
    //
    // `try_reserve_exact` is what makes the reservation fallible instead:
    // it reports both the capacity overflow and a real allocator failure as
    // an `Err` rather than unwinding.
    //
    // The two steps after it are non-aborting under a condition worth
    // naming rather than as an unconditional property of the API, since
    // `try_reserve_exact` itself guarantees capacity only *at least* the
    // request. The argument -- `resize` to exactly the reserved capacity
    // cannot reallocate, and `into_boxed_slice` on a vector whose length
    // equals its capacity skips `shrink_to_fit` -- holds only while the
    // capacity comes back exactly `len`. Where a vector's length is below
    // its capacity, `into_boxed_slice` does call `shrink_to_fit`, whose
    // failure path is the infallible `handle_alloc_error`: a process abort,
    // the exact class this function exists to close.
    //
    // What makes it exact is `RawVec`'s own bookkeeping rather than this
    // crate's choice of allocator. `RawVec` records the capacity it
    // *requested*, not the length of the block the allocator handed back --
    // "allocators currently return a `NonNull<[u8]>` whose length matches
    // the size requested. If that ever changes, the capacity here should
    // change to `ptr.len() / size_of::<T>()`" (`alloc::raw_vec`). So a
    // `#[global_allocator]` that over-allocated could not reopen this path
    // (and none is declared in this workspace in any case), while a future
    // std that recorded the returned block length could. The condition to
    // re-check is `len == capacity` at the `into_boxed_slice` below, not
    // the allocator in use.
    //
    // `RuntimeError` is a deliberate deviation, following `int_pow`'s
    // negative-exponent arm: CPython raises `MemoryError` here
    // (`bytearray(2 ** 62)` and `[0.0] * (2 ** 62)` both do), but this
    // runtime carries no `MemoryError` tag -- `pycc_hir`'s
    // `BUILTIN_EXCEPTION_CLASSES` does not list the class -- so there is no
    // conformant class it can name yet. Adding one is a cross-cutting
    // change to the class table, the `ext` bridge's tag switch and their
    // pinned counts, tracked separately rather than widened into this fix.
    //
    // #1166 round 11. The element storage was not the only allocation on
    // this path: the two-word `PyccExtBufferView` itself was a `Box::new`,
    // which is *infallible* -- on a genuine allocator failure it calls
    // `handle_alloc_error`, which aborts. Reserving the elements fallibly
    // and then aborting on the sixteen bytes that describe them closes
    // nothing, so the view is reserved through the same fallible path, and
    // both reservations report through the one failure arm below. The
    // `||` short-circuits, so a refused *length* commits no allocation at
    // all; in the other direction -- elements reserved, view refused --
    // the element reservation has been committed, and `storage`'s own drop
    // at the early return releases it.
    let mut storage: Vec<f64> = Vec::new();
    let mut view: Vec<PyccExtBufferView> = Vec::new();
    if storage.try_reserve_exact(len as usize).is_err() || view.try_reserve_exact(1).is_err() {
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "a buffer of this length cannot be allocated (CPython raises MemoryError; \
             this version has no MemoryError class to name)",
        );
        return core::ptr::null_mut();
    }
    storage.resize(len as usize, 0.0f64);
    let storage: Box<[f64]> = storage.into_boxed_slice();
    let ptr = Box::into_raw(storage) as *mut f64;
    // `push` cannot reallocate into a capacity already reserved for one
    // element, and `into_boxed_slice` on a length-one vector of capacity
    // one skips `shrink_to_fit` -- the same two conditions, and the same
    // `RawVec` exactness argument, that make the element storage above
    // non-aborting.
    view.push(PyccExtBufferView {
        ptr: ptr as *mut core::ffi::c_void,
        len,
    });
    let view: Box<[PyccExtBufferView]> = view.into_boxed_slice();
    BUFFER_LIVE.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(view) as *mut PyccExtBufferView
}

/// Releases storage produced by [`pycc_rt_buffer_f64_alloc`].
///
/// A documented **no-op on null**, exactly like [`pycc_rt_str_decref`]'s
/// null arm and for the same reason: the generated epilogue runs over every
/// buffer slot the function declared, including one a path never assigned,
/// whose slot `storage_slot_at_entry` null-initialized.
///
/// `view[0].len` is read *before* the storage is released and the view box
/// is released *last*, because the storage box's length is what makes its
/// deallocation layout the same one [`pycc_rt_buffer_f64_alloc`] used.
///
/// # Safety
/// `view` must be null or a pointer returned by
/// [`pycc_rt_buffer_f64_alloc`] that has not already been freed. It must
/// never be a `memoryview` **parameter**'s view: that storage belongs to the
/// host's exporter, and the checker refuses every shape that could route one
/// here (Part 2a of #1142 refuses assignment to a buffer parameter for
/// exactly this reason).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_free(view: *mut PyccExtBufferView) {
    if view.is_null() {
        return;
    }
    // Rebuilt as the *one-element slice box* the allocation produced, not as
    // a `Box<PyccExtBufferView>`: since #1166's round-11 review the view is
    // reserved fallibly through a one-element `Vec`, and reconstructing the
    // same box is what keeps the deallocation layout identical to the one
    // the allocation used without a hand-written layout argument.
    let view = unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(view, 1)) };
    // Read `len` off the view before the storage box is rebuilt: the slice
    // length is half of the `Box<[f64]>` layout the allocation used.
    let storage =
        core::ptr::slice_from_raw_parts_mut(view[0].ptr as *mut f64, view[0].len as usize);
    drop(unsafe { Box::from_raw(storage) });
    BUFFER_LIVE.fetch_sub(1, Ordering::Relaxed);
    drop(view);
}

/// Returns `list`'s current element count (Python's `len(list)`, D-105's
/// v0.2 `list[int]` slice).
///
/// # Element representation
/// The returned count is a **raw, untagged** `i64`, not a D-061-tagged
/// one (see `PyIntListObj`'s own doc comment) -- it is a plain `usize`
/// element count, unrelated to any stored element's own representation.
/// `len(x)` is itself a `Ty::Int`-typed expression result in ordinary
/// Python semantics, so a caller that uses this return value as a
/// `Ty::Int` anywhere else in generated code (e.g. passing it to
/// `pycc_rt_int_to_str` for `print(len(x))`) must `tag_smallint` it
/// first. Stored-element reads, unlike this raw count, already return an
/// encoded word and need no conversion.
///
/// # Safety
/// `list` must be a live `PyIntListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_len(list: *mut PyIntListObj) -> i64 {
    let list = unsafe { &*list };
    let items = list.items.take();
    let len = items.len() as i64;
    list.items.set(items);
    len
}

/// D-060-style unconditional refcounting for `list[int]`, matching
/// `pycc_rt_str_incref`'s own convention exactly: increments `list`'s
/// refcount by one, a no-op on a null pointer.
///
/// # Safety
/// `list` must be either a null pointer or a live `PyIntListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_incref(list: *mut PyIntListObj) {
    if list.is_null() {
        return;
    }
    let obj = unsafe { &*list };
    obj.rc.set(obj.rc.get() + 1);
}

/// D-060-style unconditional refcounting for `list[int]`, matching
/// `pycc_rt_str_decref`'s own convention exactly: decrements `list`'s
/// refcount by one, freeing the allocation once it reaches zero (which,
/// via `Box::from_raw`'s own drop glue, also frees the `Cell<Vec<i64>>`
/// payload's backing buffer -- no separate manual deallocation call
/// needed). A no-op on a null pointer, same rationale as
/// `pycc_rt_int_list_incref` above.
///
/// # Safety
/// Same as `pycc_rt_int_list_incref`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_decref(list: *mut PyIntListObj) {
    if list.is_null() {
        return;
    }
    let new_rc = unsafe { &*list }.rc.get() - 1;
    if new_rc == 0 {
        drop(unsafe { Box::from_raw(list) });
    } else {
        unsafe { &*list }.rc.set(new_rc);
    }
}

/// Part B of #1038 (#1064) converted this function's three rejected-bound
/// aborts into D-173 pending-exception raises, so it no longer panics.
/// `pycc_rt_int_list_slice` below remains a plain `extern "C" fn`, not
/// `extern "C-unwind"`; this private function holds the real logic and
/// unit tests call it directly, exactly like `int_list_get`'s own split.
fn int_list_slice(list: &PyIntListObj, start: i64, stop: i64, step: i64) -> *mut PyIntListObj {
    // The three bound checks below run before `list.items.take()`, so unlike
    // `int_list_get` there is no payload to restore before returning.
    // The sentinel is a fresh empty list -- a valid `PyIntListObj`, never
    // null, so a caller may safely touch it before its own flag check.
    if start < 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "slice start must be non-negative",
        );
        return pycc_rt_int_list_new();
    }
    if stop < 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "slice stop must be non-negative",
        );
        return pycc_rt_int_list_new();
    }
    if step <= 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "slice step must be positive",
        );
        return pycc_rt_int_list_new();
    }
    let items = list.items.take();
    let len = items.len() as i64;
    let clamped_start = start.min(len);
    let clamped_stop = stop.min(len);
    let result = pycc_rt_int_list_new();
    // Unlike `int_list_get` (which restores `list`'s payload before
    // raising on an out-of-range index), this loop's
    // take-window contains no fallible operation, so there is nothing to
    // restore: `items[i as usize]` cannot panic, since `i` starts at
    // `clamped_start` and the loop guard keeps it below `clamped_stop`,
    // and both clamped bounds are `.min(len)`-derived, so
    // `clamped_start <= i < clamped_stop <= len == items.len()` holds on
    // every iteration. `i += step` cannot overflow `i64` either: `i` never
    // exceeds `len`, and `step` itself already passed
    // `pycc_rt_int_untag_checked`'s 63-bit smallint range check at every
    // real `pycc_codegen` call site, so `i + step` stays far below
    // `i64::MAX`.
    let mut i = clamped_start;
    while i < clamped_stop {
        unsafe { pycc_rt_int_list_append(result, items[i as usize]) };
        i += step;
    }
    list.items.set(items);
    result
}

/// Returns a **new** list containing the clamped, strided sub-range
/// `[start, stop)` of `list`'s elements, stepping by `step` (Python's
/// `list[start:stop:step]`, D-118's v0.2 `list[int]` slice). Sets the
/// pending `ValueError` exception flag (D-173, Part B of #1038, #1064) and
/// returns a new empty list as the sentinel on a negative `start`/`stop` or
/// a non-positive `step`; the caller's generated code checks the flag after
/// this call. v0.2 ships no CPython-style negative-index/negative-step
/// semantics, extending D-108's own uniform "no negative addressing" scope
/// cut (`pycc_rt_int_list_get`) to slicing; the conformance gap is tracked
/// as #1070. `start`/`stop` are clamped into `[0, len]` after the sign
/// check, matching CPython's own out-of-range-slice-bound clamping --
/// required for the accepted subset (omitted/over-long bounds) to match
/// CPython byte-for-byte, not merely a nicety. The three sign/positivity
/// checks run before `list.items.take()`, mirroring `int_list_get`'s own
/// "leave `list` intact" care -- a rejected call here never touches
/// `list`'s payload at all, so there is nothing to restore.
///
/// # Element representation
/// `start`/`stop`/`step` are raw, untagged `i64` offsets/strides, not
/// D-061-tagged `Ty::Int` values -- a caller with a tagged operand must
/// `pycc_rt_int_untag_checked` each one first, exactly like
/// `pycc_rt_int_list_get`'s own `index` parameter. The returned list's own
/// elements are copied through unchanged (already D-141 encoded words per
/// `PyIntListObj`'s representation) -- no per-element
/// conversion happens here, exactly like a single-element read.
///
/// # Safety
/// `list` must be a live `PyIntListObj` pointer. Takes `*mut` rather than
/// the task plan's own sketched `*const`, matching every other
/// `pycc_rt_int_list_*` function in this file (`_get`/`_len` both take
/// `*mut PyIntListObj` even though they too only read `list`'s payload) --
/// a deliberate consistency choice over the plan's literal text, not a
/// functional difference (LLVM's opaque pointer type distinguishes neither
/// side at the `pycc_codegen` call site either way).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_slice(
    list: *mut PyIntListObj,
    start: i64,
    stop: i64,
    step: i64,
) -> *mut PyIntListObj {
    int_list_slice(unsafe { &*list }, start, stop, step)
}

/// Part B of #1038 (#1064) converted this function's empty-list abort into
/// a D-173 pending-exception raise, so it no longer panics.
/// `pycc_rt_int_list_pop` below remains a plain `extern "C" fn`, not
/// `extern "C-unwind"`; this private function holds the real logic and
/// unit tests call it directly, exactly like `int_list_get`'s own split.
fn int_list_pop(list: &PyIntListObj) -> i64 {
    let mut items = list.items.take();
    let Some(value) = items.pop() else {
        // Restore the (empty) payload before raising, same rationale as
        // `int_list_get`'s own "restore before returning" comment.
        list.items.set(items);
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "pop from empty list",
        );
        return tag_smallint(0);
    };
    list.items.set(items);
    value
}

/// Removes and returns the list's **last** element (Python's `list.pop()`,
/// PR-12, D-119). Sets the pending `IndexError` exception flag (D-173) and
/// returns a sentinel `tag_smallint(0)` if `list` is empty, matching
/// CPython's own catchable `IndexError`; the caller's generated code checks
/// the flag after this call. The message is `"pop from empty list"`.
///
/// # Element representation
/// The returned value is the stored D-141 encoded word unchanged, so a bool
/// marker keeps its runtime identity and no output-side conversion is needed.
///
/// # Safety
/// `list` must be a live `PyIntListObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_int_list_pop(list: *mut PyIntListObj) -> i64 {
    int_list_pop(unsafe { &*list })
}

/// `dict[str, int]`'s runtime representation (D-121): a dense,
/// insertion-ordered array of `(key, value)` pairs. `Cell<Vec<...>>`
/// mirrors `PyIntListObj`'s own choice over `RefCell` -- `Cell::take`/
/// `_::set` never holds a borrow across a mutation, so there is no new
/// runtime-panic mode from overlapping borrows. Not `#[repr(C)]` -- never
/// crosses the LLVM/Rust boundary by value, only as an opaque pointer
/// (mirrors `PyStrObj`/`PyIntListObj`). Lookup is linear-scan comparison
/// via `pycc_rt_str_cmp` (D-121), not a hash table. Values are D-141 encoded
/// words preserved unchanged across set/get/get-or-default; generated ingress
/// validates them with `pycc_rt_int_untag_checked` before storage.
pub struct PyDictObj {
    rc: Cell<u32>,
    entries: Cell<Vec<(*mut PyStrObj, i64)>>,
}

/// Allocates a fresh, empty `PyDictObj` with refcount `1`. Never panics.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_dict_new() -> *mut PyDictObj {
    Box::into_raw(Box::new(PyDictObj {
        rc: Cell::new(1),
        entries: Cell::new(Vec::new()),
    }))
}

/// Insert-or-update (D-123): if `key` compares equal (`pycc_rt_str_cmp`)
/// to an already-stored key, that entry's value is overwritten in place,
/// preserving insertion order; otherwise `(key, value)` is appended.
/// Leak-only (D-124): the stored key pointer is neither increfed on
/// insert nor decrefed on update-in-place or ever.
///
/// # Safety
/// `dict` and `key` must be live pointers from `pycc_rt_dict_new`/
/// `pycc_rt_str_new`-family functions respectively.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_set(dict: *mut PyDictObj, key: *mut PyStrObj, value: i64) {
    let mut entries = unsafe { &*dict }.entries.take();
    match entries
        .iter()
        .position(|(k, _)| unsafe { pycc_rt_str_cmp(*k, key) } == 0)
    {
        Some(i) => entries[i].1 = value,
        None => entries.push((key, value)),
    }
    unsafe { &*dict }.entries.set(entries);
}

fn dict_get(dict: &PyDictObj, key: *mut PyStrObj) -> i64 {
    let entries = dict.entries.take();
    let found = entries
        .iter()
        .find(|(k, _)| unsafe { pycc_rt_str_cmp(*k, key) } == 0)
        .map(|(_, v)| *v);
    dict.entries.set(entries);
    found.unwrap_or_else(|| {
        // D-173: set the pending exception flag instead of panicking.
        raise_builtin(EXCEPTION_TYPE_KEY_ERROR, "KeyError", "dict key not found");
        tag_smallint(0)
    })
}

/// Linear-scan lookup (D-121). Sets the pending `KeyError` exception flag
/// if no stored key compares equal to `key` (D-173) and returns a sentinel
/// `0`; the caller's generated code checks the flag after this call. The
/// exception message is `"dict key not found"`.
///
/// # Safety
/// Same as `pycc_rt_dict_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get(dict: *mut PyDictObj, key: *mut PyStrObj) -> i64 {
    dict_get(unsafe { &*dict }, key)
}

fn dict_get_or_default(dict: &PyDictObj, key: *mut PyStrObj, default: i64) -> i64 {
    let entries = dict.entries.take();
    let found = entries
        .iter()
        .find(|(k, _)| unsafe { pycc_rt_str_cmp(*k, key) } == 0)
        .map(|(_, v)| *v);
    dict.entries.set(entries);
    found.unwrap_or(default)
}

/// Returns the value stored for `key`, or `default` if `key` is absent
/// (Python's `dict.get(key, default)`, PR-12, D-119) -- unlike
/// `pycc_rt_dict_get`, this **never panics** on a missing key; that is the
/// entire point of the two-argument form. `default` is passed through
/// unchanged, so this function itself never panics either -- it is a total
/// function given the type gate (`pycc_types`' T0021/T0033) already
/// enforces `key`/`default`'s types.
///
/// # Safety
/// Same as `pycc_rt_dict_get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_get_or_default(
    dict: *mut PyDictObj,
    key: *mut PyStrObj,
    default: i64,
) -> i64 {
    dict_get_or_default(unsafe { &*dict }, key, default)
}

/// Number of entries. `Cell::take` followed by re-`set`ting it, mirroring
/// `pycc_rt_int_list_len`'s own exact pattern (never leave the `Cell`
/// empty on return).
///
/// # Safety
/// `dict` must be a live `PyDictObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_len(dict: *mut PyDictObj) -> i64 {
    let entries = unsafe { &*dict }.entries.take();
    let len = entries.len() as i64;
    unsafe { &*dict }.entries.set(entries);
    len
}

/// Private half of `pycc_rt_dict_key_at` below, same panic-across-FFI
/// split as `int_list_get`/`dict_get` above: a plain `extern "C" fn`
/// turns an unwinding panic into a process abort, so the freely-panicking
/// logic lives here and tests exercising the panic call this directly.
///
/// Uses `Vec::get` (bounds-checked, never panics on its own) rather than
/// indexing directly, so the payload can be restored into the `Cell`
/// *before* the panic branch -- mirroring `int_list_get`'s own "restore
/// before panicking" comment and `dict_get`'s own `unwrap_or_else` shape,
/// instead of `dict.entries.take()` followed by a direct index that would
/// leave the `Cell` holding an empty `Vec` if it panicked. Not required
/// for this compiler's actual generated code (`ForDict`'s own loop bound
/// is always `pycc_rt_dict_len`, so no call site this project's own
/// codegen emits can go out of range), but cheap insurance against
/// leaving `dict` with a permanently emptied payload for any future
/// caller that does catch this unwind (e.g. this file's own
/// `#[should_panic]` test below).
fn dict_key_at(dict: &PyDictObj, index: i64) -> *mut PyStrObj {
    let entries = dict.entries.take();
    let key = entries.get(index as usize).map(|(k, _)| *k);
    dict.entries.set(entries);
    key.unwrap_or_else(|| panic!("pycc_rt: dict_key_at index out of range"))
}

/// Key at a given insertion-order position, used only by `ForDict`'s own
/// iteration codegen (Task 5) -- never a user-facing indexing operation
/// (dict has no positional index in Python). Panics (message
/// `"pycc_rt: dict_key_at index out of range"`) if `index` is out of
/// range; unreachable in this compiler's own generated code (see `#
/// Safety` below) but not undefined behavior if ever hit.
///
/// # Safety
/// `dict` must be a live `PyDictObj` pointer; `index` must satisfy
/// `0 <= index < pycc_rt_dict_len(dict)` (an internal codegen invariant,
/// not user input -- `ForDict`'s own loop bound is `pycc_rt_dict_len`,
/// so an out-of-range call here would be a codegen bug, not a possible
/// user program).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_key_at(dict: *mut PyDictObj, index: i64) -> *mut PyStrObj {
    dict_key_at(unsafe { &*dict }, index)
}

/// Unconditional refcounting (D-124), mirroring `pycc_rt_int_list_incref`
/// exactly. No-op on null.
///
/// # Safety
/// `dict` must be null or a live `PyDictObj` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_incref(dict: *mut PyDictObj) {
    if dict.is_null() {
        return;
    }
    let obj = unsafe { &*dict };
    obj.rc.set(obj.rc.get() + 1);
}

/// Mirrors `pycc_rt_int_list_decref` exactly: frees via `Box::from_raw`
/// once `rc` hits 0. Not called from any `pycc_codegen` site in this PR
/// (D-124, leak-only).
///
/// # Safety
/// `dict` must be null or a live `PyDictObj` pointer not used again after
/// its refcount reaches 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_dict_decref(dict: *mut PyDictObj) {
    if dict.is_null() {
        return;
    }
    let obj = unsafe { &*dict };
    let rc = obj.rc.get() - 1;
    obj.rc.set(rc);
    if rc == 0 {
        drop(unsafe { Box::from_raw(dict) });
    }
}

/// Runtime NameError for call-before-`def` (issue #22). pycc's type checker
/// rejects a call to a not-yet-`def`ined function in top-level code statically,
/// but a function body may call a sibling whose `def` has not executed yet at
/// the time the caller is invoked -- CPython raises `NameError` at that point,
/// and so does this. Until v0.3's exception machinery exists, a runtime panic
/// becomes an explicit process failure at the plain-C ABI boundary, matching
/// every other runtime error in this crate.
///
/// Takes a null-terminated C string (the function name) so the codegen can
/// pass a global string constant directly. The panic-across-FFI note on
/// `pycc_rt_int_add` applies: a panic that would unwind past this `extern "C"`
/// boundary is caught and turned into a process abort by Rust's default
/// panic-over-FFI behavior, which is exactly the desired outcome for a
/// runtime NameError in a compiled program.
///
/// Split into a private freely-panicking function (testable with
/// `#[should_panic]`) and a thin `extern "C"` wrapper, matching the same
/// split every other panicking runtime function in this file uses.
fn name_error(name: *const std::os::raw::c_char) -> ! {
    let name_str = if name.is_null() {
        "<unknown>"
    } else {
        unsafe { std::ffi::CStr::from_ptr(name) }
            .to_str()
            .unwrap_or("<invalid utf-8>")
    };
    panic!("pycc_rt: NameError: name '{name_str}' is not defined");
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_name_error(name: *const std::os::raw::c_char) -> ! {
    name_error(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::{Equal, Greater, Less};

    /// #146 Part 1: a heap bigint round-trips through retain/release and is
    /// freed exactly once, at the release that retires its last reference.
    /// `BIGINT_DROPS` is the only way a test can observe the free at all --
    /// nothing about a freed word is inspectable afterwards.
    #[test]
    fn a_bigint_is_freed_only_when_its_last_reference_goes_away() {
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        let word = tag_bigint(bigint_from_i128(1i128 << 62));
        // Birth reference plus two more.
        bigint_retain(word);
        bigint_retain(word);
        bigint_release(word);
        bigint_release(word);
        assert_eq!(
            int_encoding::BIGINT_DROPS.with(|c| c.get()),
            before,
            "a bigint with a live reference left must not be freed"
        );
        // The value is still intact and readable at this point.
        assert_eq!(to_sign_and_magnitude(word), (false, vec![0, 0x4000_0000]));
        bigint_release(word);
        assert_eq!(
            int_encoding::BIGINT_DROPS.with(|c| c.get()),
            before + 1,
            "the release retiring the last reference must free the object"
        );
    }

    /// #146 Part 2 (D-181): the no-aliasing invariant `pycc_codegen`'s
    /// operand releases depend on. No `int` operation may hand back an
    /// operand's own encoded word -- every bigint result is a freshly
    /// allocated object. If an identity fast path were ever added (say
    /// `a + 0 -> a`), the emitter's release of both operands right after
    /// the call would free the value just returned, and this test is what
    /// stands between that change and a use-after-free in every compiled
    /// program.
    ///
    /// Only `int_add`/`int_sub` are exercised: `int_mul`, `int_floordiv`,
    /// `int_floormod`, `int_pow` and `int_cmp` all route through
    /// `decode_inline_or_raise`, which raises `OverflowError` and returns a
    /// sentinel on a bigint operand (Part C of #1038), so they have no
    /// bigint result to alias in the first place (`docs/ROADMAP.md`'s own
    /// bigint capability gap).
    #[test]
    fn an_int_operation_never_returns_an_operand_s_own_word() {
        let big = tag_bigint(bigint_from_i128(1i128 << 62));
        let zero = int_encoding::tag_smallint(0);
        for (name, result) in [
            ("add-zero-right", pycc_rt_int_add(big, zero)),
            ("add-zero-left", pycc_rt_int_add(zero, big)),
            ("sub-zero-right", pycc_rt_int_sub(big, zero)),
            ("add-self", pycc_rt_int_add(big, big)),
            ("sub-self", pycc_rt_int_sub(big, big)),
        ] {
            assert_ne!(
                result, big,
                "`{name}` returned the operand's own encoded word; \
                 pycc_codegen releases both operands after the call, so an \
                 aliased result is a use-after-free"
            );
            bigint_release(result);
        }
        bigint_release(big);
    }

    /// The word `0` is what an `int` storage slot holds before its first
    /// store (`storage_slot_at_entry`'s zero-initialization). It is *not* a
    /// valid encoded int -- `classify_encoded_int` fails closed on it -- so
    /// both operations must return before classifying rather than panic.
    #[test]
    fn the_empty_slot_word_is_a_no_op_for_both_refcount_operations() {
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        bigint_retain(0);
        bigint_release(0);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
    }

    /// Smallints and the two D-141 bool-identity markers own no allocation,
    /// so both operations are no-ops on them. `pycc_codegen` guards its call
    /// sites inline so these never actually reach the runtime, but the
    /// runtime contract holds independently of that guard.
    #[test]
    fn inline_int_kinds_are_no_ops_for_both_refcount_operations() {
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        for word in [tag_smallint(0), tag_smallint(-7), 0b0010, 0b0110] {
            bigint_retain(word);
            bigint_release(word);
        }
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
    }

    /// The private functions, not the `extern "C"` wrappers: an `extern "C"`
    /// function may not unwind, so the fail-closed panic is only observable
    /// through the private path (the same split as `range_continue` versus
    /// `pycc_rt_range_continue`).
    #[test]
    #[should_panic(expected = "pycc_rt: invalid encoded int word 0x2a")]
    fn retaining_an_invalid_encoded_word_fails_closed() {
        bigint_retain(0x2a);
    }

    #[test]
    #[should_panic(expected = "pycc_rt: invalid encoded int word 0x2a")]
    fn releasing_an_invalid_encoded_word_fails_closed() {
        bigint_release(0x2a);
    }

    /// The `extern "C"` wrappers themselves, on the non-panicking paths that
    /// generated code actually reaches.
    #[test]
    fn the_extern_c_bigint_refcount_wrappers_forward_to_the_private_pair() {
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        let word = tag_bigint(bigint_from_i128(1i128 << 62));
        pycc_rt_bigint_retain(word);
        pycc_rt_bigint_release(word);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
        pycc_rt_bigint_retain(0);
        pycc_rt_bigint_release(0);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
        pycc_rt_bigint_release(word);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before + 1);
    }

    /// Part C of #1038 (#1065): `int_pow` squares the *encoded* word through
    /// `int_mul`, so an inline base can promote to a heap bigint that only
    /// `int_pow` itself holds. The very next `int_mul` then raises
    /// `OverflowError` and returns a sentinel. Before Part C the aborting
    /// process reclaimed that object; now a program can catch the exception in
    /// a loop, so `int_pow` must retire its own temporaries on the raising
    /// exit. `BIGINT_DROPS` is the only way a test can observe the free.
    #[test]
    fn int_pow_frees_its_own_promoted_temporaries_when_the_squaring_overflows() {
        for (name, base, exp, freed) in [
            // Promotes `base` at the last squaring; the final set bit's
            // `int_mul` then sees it and raises, so that one object is the
            // only temporary to retire.
            ("2 ** 100", 2i64, 100i64, 1),
            // Promotes the accumulator *and* the base before raising, so both
            // releases have real work to do. An exact count, not `> before`:
            // a regression that retires one of the two and leaks the other
            // must fail here.
            ("(2 ** 21) ** 7", 1i64 << 21, 7i64, 2),
        ] {
            pycc_rt_exception_clear();
            let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
            assert_eq!(
                int_pow(tag_smallint(base), tag_smallint(exp)),
                tag_smallint(0),
                "`{name}` must return the zero sentinel once it raises"
            );
            assert_eq!(
                pycc_rt_exception_active(),
                1,
                "`{name}` must leave the `OverflowError` pending"
            );
            assert_eq!(
                int_encoding::BIGINT_DROPS.with(|c| c.get()),
                before + freed,
                "`{name}` must free exactly the {freed} bigint(s) it promoted"
            );
            pycc_rt_exception_clear();
        }
    }

    /// The success path: every operand stays inline, so both releases are the
    /// no-ops `bigint_release` documents and the result is exact.
    #[test]
    fn int_pow_returns_an_inline_result_without_freeing_anything() {
        pycc_rt_exception_clear();
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        assert_eq!(int_pow(tag_smallint(3), tag_smallint(4)), tag_smallint(81));
        assert_eq!(int_pow(tag_smallint(-7), tag_smallint(1)), tag_smallint(-7));
        assert_eq!(pycc_rt_exception_active(), 0);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before);
    }

    /// The other non-raising exit: the accumulator promotes mid-loop and the
    /// loop still runs out of exponent bits, so `int_pow` hands the caller a
    /// live heap bigint. Without this case the releases added around that exit
    /// are only ever executed on the raising path, and an over-release of
    /// `result` -- handing back a freed word -- would pass every other test
    /// and the coverage gate alike, since the same lines are already hit.
    #[test]
    fn int_pow_hands_back_an_accumulator_that_promoted_mid_loop() {
        pycc_rt_exception_clear();
        let before = int_encoding::BIGINT_DROPS.with(|c| c.get());
        // `(2 ** 21) ** 3` is `2 ** 63`: one past `i64`, so `int_mul` promotes
        // it, and no operand is ever a bigint, so nothing raises.
        let word = int_pow(tag_smallint(1 << 21), tag_smallint(3));
        assert_eq!(pycc_rt_exception_active(), 0);
        assert_eq!(
            int_encoding::BIGINT_DROPS.with(|c| c.get()),
            before,
            "the returned reference is the caller's; nothing may be freed yet"
        );
        assert_eq!(
            to_sign_and_magnitude(word),
            (false, vec![0, 0x8000_0000]),
            "`(2 ** 21) ** 3` must read back as `2 ** 63`"
        );
        bigint_release(word);
        assert_eq!(int_encoding::BIGINT_DROPS.with(|c| c.get()), before + 1);
    }

    #[test]
    fn print_i64_matches_cpython_format() {
        // Confirmed against `python3.14 -c "print(42)"` / `print(-7)`:
        // exactly the digits, then a single trailing newline, nothing else.
        assert_eq!(format_i64_line(42), "42\n");
        assert_eq!(format_i64_line(-7), "-7\n");
    }

    #[test]
    #[should_panic(expected = "pycc_rt: NameError: name 'foo' is not defined")]
    fn name_error_panics_with_function_name() {
        let name = std::ffi::CString::new("foo").unwrap();
        name_error(name.as_ptr());
    }

    #[test]
    #[should_panic(expected = "pycc_rt: NameError: name '<unknown>' is not defined")]
    fn name_error_on_null_pointer_uses_unknown() {
        name_error(std::ptr::null());
    }

    #[test]
    fn extern_c_entry_point_runs_for_positive_negative_and_zero() {
        // This crate's staticlib output is linked into pycc-compiled
        // binaries by a separate `cc` step (Task 8), which cargo-llvm-cov
        // never instruments -- so this is the only place
        // pycc_rt_print_i64 itself (not just its formatting helper) is
        // exercised for the D-014 gate. stdout is captured by the test
        // harness and only shown on failure.
        pycc_rt_print_i64(42);
        pycc_rt_print_i64(-7);
        pycc_rt_print_i64(0);
    }

    #[test]
    fn tagging_a_small_value_and_untagging_it_round_trips() {
        for n in [0i64, 1, -1, 42, -42, i64::MAX >> 1, -(i64::MAX >> 1)] {
            let tagged = tag_smallint(n);
            assert!(is_smallint(tagged), "expected {n} to be tagged as small");
            assert_eq!(untag_smallint(tagged), n);
        }
    }

    #[test]
    fn a_value_needing_the_full_64_bits_does_not_fit_the_tagged_range() {
        assert_eq!(fits_smallint(i64::MAX), None);
        assert_eq!(fits_smallint(i64::MIN), None);
    }

    #[test]
    fn pycc_rt_int_add_computes_the_correct_tagged_sum() {
        let a = tag_smallint(2);
        let b = tag_smallint(3);
        assert_eq!(untag_smallint(pycc_rt_int_add(a, b)), 5);
    }

    #[test]
    fn pycc_rt_int_sub_computes_the_correct_tagged_difference() {
        let a = tag_smallint(5);
        let b = tag_smallint(3);
        assert_eq!(untag_smallint(pycc_rt_int_sub(a, b)), 2);
    }

    #[test]
    fn pycc_rt_int_mul_computes_the_correct_tagged_product() {
        let a = tag_smallint(6);
        let b = tag_smallint(7);
        assert_eq!(untag_smallint(pycc_rt_int_mul(a, b)), 42);
    }

    #[test]
    fn pycc_rt_int_mul_promotes_a_product_outside_the_smallint_range() {
        let product = pycc_rt_int_mul(tag_smallint(i64::MAX >> 1), tag_smallint(2));
        assert!(!is_smallint(product));
        let text = pycc_rt_int_to_str(product);
        assert_eq!(unsafe { &*text }.bytes(), b"9223372036854775806");
        unsafe { pycc_rt_str_decref(text) };
    }

    #[test]
    fn pycc_rt_int_floordiv_matches_python_floor_semantics() {
        // Python: -7 // 2 == -4 (floors toward negative infinity), not -3
        // (truncation toward zero, which is what a raw LLVM/Rust `/` gives).
        assert_eq!(
            untag_smallint(pycc_rt_int_floordiv(tag_smallint(-7), tag_smallint(2))),
            -4
        );
        assert_eq!(
            untag_smallint(pycc_rt_int_floordiv(tag_smallint(7), tag_smallint(2))),
            3
        );
        assert_eq!(
            untag_smallint(pycc_rt_int_floordiv(tag_smallint(7), tag_smallint(-2))),
            -4
        );
    }

    #[test]
    fn pycc_rt_int_floormod_matches_python_floor_semantics() {
        // Python: -7 % 2 == 1 (result takes the divisor's sign), not -1.
        assert_eq!(
            untag_smallint(pycc_rt_int_floormod(tag_smallint(-7), tag_smallint(2))),
            1
        );
        assert_eq!(
            untag_smallint(pycc_rt_int_floormod(tag_smallint(7), tag_smallint(2))),
            1
        );
        assert_eq!(
            untag_smallint(pycc_rt_int_floormod(tag_smallint(7), tag_smallint(-2))),
            -1
        );
    }

    #[test]
    fn pycc_rt_int_floordiv_by_zero_sets_exception_flag() {
        // D-173: zero division now sets the pending exception flag instead
        // of panicking. Clear the flag first for test isolation.
        pycc_rt_exception_clear();
        let result = int_floordiv(tag_smallint(1), tag_smallint(0));
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(untag_smallint(result), 0); // sentinel value
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_int_floormod_by_zero_sets_exception_flag() {
        pycc_rt_exception_clear();
        let result = int_floormod(tag_smallint(1), tag_smallint(0));
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(untag_smallint(result), 0); // sentinel value
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_int_floordiv_handles_the_minimum_taggable_value_divided_by_one() {
        // The task brief's own version of this test's comment claimed this
        // exercised the classic `i64::MIN / -1` hardware-trap pair -- it
        // does not (it divides by `1`, not `-1`); see `int_floordiv`'s own
        // comment for why that trap is actually unreachable here at all.
        // What this test does prove: the boundary value itself (the most
        // negative value that still tags) round-trips correctly through
        // floor-division by `1` without spuriously reporting overflow.
        let min = i64::MIN >> 1; // most negative value that still tags
        assert_eq!(
            untag_smallint(pycc_rt_int_floordiv(tag_smallint(min), tag_smallint(1))),
            min
        );
    }

    #[test]
    fn pycc_rt_int_floordiv_promotes_the_negated_minimum_taggable_value() {
        // The actual reachable promotion case for `int_floordiv` (see its
        // comment): floor-dividing the minimum taggable value by `-1`
        // negates it, producing exactly one more than the maximum taggable
        // magnitude -- a real overflow of the tagged result, distinct from (and
        // the reason the brief's original i64::MIN/-1 *input* guard turned
        // out to be dead code).
        let min = i64::MIN >> 1;
        let quotient = int_floordiv(tag_smallint(min), tag_smallint(-1));
        assert!(!is_smallint(quotient));
        let text = pycc_rt_int_to_str(quotient);
        assert_eq!(unsafe { &*text }.bytes(), b"4611686018427387904");
        unsafe { pycc_rt_str_decref(text) };
    }

    #[test]
    fn pycc_rt_int_pow_computes_the_correct_tagged_power() {
        assert_eq!(
            untag_smallint(pycc_rt_int_pow(tag_smallint(2), tag_smallint(10))),
            1024
        );
        assert_eq!(
            untag_smallint(pycc_rt_int_pow(tag_smallint(5), tag_smallint(0))),
            1
        );
    }

    /// Part A of #1038 (#1063): the pending exception's tag and message, for
    /// the converted `**` abort paths. The state is thread-local and
    /// `cargo test` runs in parallel, so every caller clears before raising
    /// and after asserting; a message left pending would otherwise be read by
    /// whatever test the harness schedules next on this thread.
    pub(crate) fn pending_tag_and_message() -> (u8, String) {
        assert_eq!(pycc_rt_exception_active(), 1);
        let obj = exception::pycc_rt_exception_value();
        let tag = unsafe { (*obj).type_tag };
        let mut len = 0usize;
        let bytes = unsafe { exception::pycc_rt_ext_pending_message(&mut len) };
        assert!(!bytes.is_null());
        let message = String::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec())
            .expect("the message is UTF-8");
        (tag, message)
    }

    #[test]
    fn pycc_rt_int_pow_with_a_negative_exponent_raises_runtime_error() {
        // Part A of #1038 (#1063): was `#[should_panic]`. `RuntimeError`
        // rather than a conformant class because CPython raises nothing at
        // all here -- it computes `2 ** -1` as `0.5`. Calls the private
        // `int_pow`, not the aborting `extern "C"` wrapper, following this
        // module's established convention.
        pycc_rt_exception_clear();
        let result = int_pow(tag_smallint(2), tag_smallint(-1));
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
        assert!(message.contains("negative exponent"), "{message}");
        assert!(!message.contains("pycc_rt: "), "{message}");
        assert_eq!(result, tag_smallint(0)); // sentinel value
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_int_cmp_reports_less_equal_and_greater() {
        assert_eq!(pycc_rt_int_cmp(tag_smallint(1), tag_smallint(2)), -1);
        assert_eq!(pycc_rt_int_cmp(tag_smallint(2), tag_smallint(2)), 0);
        assert_eq!(pycc_rt_int_cmp(tag_smallint(3), tag_smallint(2)), 1);
    }

    /// Part C of #1038 (#1065): a heap bigint word, the operand every
    /// converted site below rejects.
    pub(crate) fn a_bigint_word() -> i64 {
        tag_bigint(bigint_from_i128(1i128 << 80))
    }

    /// Part C of #1038 (#1065): asserts the pending exception is the
    /// converted `OverflowError`, carries `context`, and does not carry the
    /// retired `pycc_rt: ` panic prefix (the Part B convention).
    pub(crate) fn assert_overflow_raised(context: &str) {
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_OVERFLOW_ERROR, "{message}");
        assert!(
            message == format!("{context} a bigint-valued `int` is not supported yet"),
            "{message}"
        );
        assert!(!message.contains("pycc_rt: "), "{message}");
    }

    #[test]
    fn a_bigint_operand_of_int_mul_raises_overflow_error_from_either_side() {
        pycc_rt_exception_clear();
        assert_eq!(int_mul(a_bigint_word(), tag_smallint(2)), tag_smallint(0));
        assert_overflow_raised("multiplying");
        pycc_rt_exception_clear();
        assert_eq!(int_mul(tag_smallint(2), a_bigint_word()), tag_smallint(0));
        assert_overflow_raised("multiplying");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_bigint_operand_of_int_cmp_raises_overflow_error_from_either_side() {
        // Part C of #1038 (#1065): was `#[should_panic]`. The sentinel is a
        // plain `0` -- `int_cmp` returns an `i32` ordering, not a D-141
        // encoded word.
        pycc_rt_exception_clear();
        assert_eq!(int_cmp(a_bigint_word(), tag_smallint(1)), 0);
        assert_overflow_raised("comparing");
        pycc_rt_exception_clear();
        assert_eq!(int_cmp(tag_smallint(1), a_bigint_word()), 0);
        assert_overflow_raised("comparing");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_bigint_dividend_or_divisor_raises_overflow_error_not_zero_division() {
        // The #1065 hazard, closed: `raise_builtin` installs
        // unconditionally, so had the bigint divisor fallen through to
        // `int_floordiv`'s `b == 0` arm as a decoded `0`, the
        // `ZeroDivisionError` would have reported over this `OverflowError`.
        pycc_rt_exception_clear();
        assert_eq!(
            int_floordiv(a_bigint_word(), tag_smallint(2)),
            tag_smallint(0)
        );
        assert_overflow_raised("dividing");
        pycc_rt_exception_clear();
        assert_eq!(
            int_floordiv(tag_smallint(6), a_bigint_word()),
            tag_smallint(0)
        );
        assert_overflow_raised("dividing");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_bigint_operand_of_int_floormod_raises_overflow_error_not_zero_division() {
        pycc_rt_exception_clear();
        assert_eq!(
            int_floormod(a_bigint_word(), tag_smallint(2)),
            tag_smallint(0)
        );
        assert_overflow_raised("computing the modulo of");
        pycc_rt_exception_clear();
        assert_eq!(
            int_floormod(tag_smallint(6), a_bigint_word()),
            tag_smallint(0)
        );
        assert_overflow_raised("computing the modulo of");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_bigint_base_or_exponent_raises_overflow_error_not_runtime_error() {
        // The exponent arm is the second collision of the same shape: a
        // bigint exponent decoding to `0` would not be `< 0`, so `int_pow`
        // would have silently computed `1` instead of propagating at all.
        pycc_rt_exception_clear();
        assert_eq!(int_pow(a_bigint_word(), tag_smallint(2)), tag_smallint(0));
        assert_overflow_raised("exponentiating");
        pycc_rt_exception_clear();
        assert_eq!(int_pow(tag_smallint(2), a_bigint_word()), tag_smallint(0));
        assert_overflow_raised("exponentiating");
        pycc_rt_exception_clear();
    }

    #[test]
    fn an_inline_pow_that_promotes_mid_loop_reports_the_multiplying_context() {
        // `int_pow` is repeated `int_mul`, and `int_mul` *promotes* an
        // overflowing product to a heap bigint. Both operands of `2 ** 40`
        // are inline, so `int_pow`'s own two checks pass; the squaring step
        // then produces a bigint that the next `int_mul` rejects. The raise
        // therefore reports `multiplying` for an expression whose only
        // operator is `**`. Pinned as measured behavior, not as a contract:
        // it is pre-existing in kind (the retired `panic!` said `multiplying`
        // too) and only became observable when Part C turned the abort into a
        // returning raise. Exactly one exception is installed -- the loop's
        // remaining iteration re-installs an identical one at worst.
        pycc_rt_exception_clear();
        assert_eq!(
            int_pow(tag_smallint(1 << 40), tag_smallint(2)),
            tag_smallint(0)
        );
        assert_overflow_raised("multiplying");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_bigint_operand_of_int_to_float_raises_overflow_error() {
        pycc_rt_exception_clear();
        assert_eq!(int_to_float(a_bigint_word()), 0.0);
        assert_overflow_raised("converting");
        pycc_rt_exception_clear();
    }

    #[test]
    #[should_panic(expected = "invalid encoded int word 0x0")]
    fn encoded_int_classification_rejects_null_before_any_pointer_cast() {
        classify_encoded_int(0);
    }

    #[test]
    #[should_panic(expected = "invalid encoded int word 0xa")]
    fn encoded_int_classification_rejects_unrecognized_low_tag_10_words() {
        classify_encoded_int(0b1010);
    }

    #[test]
    #[should_panic(expected = "attempted bigint dereference of a non-pointer int word")]
    fn bigint_dereference_rechecks_the_encoded_word_before_casting() {
        // Even an unsafe internal caller cannot turn a valid bool marker
        // into a pointer without passing the central classifier again.
        unsafe {
            let _ = bigint_ref(BOOL_TRUE_MARKER);
        }
    }

    #[test]
    fn bool_identity_markers_decode_numerically_but_format_as_bools() {
        assert_eq!(int_untag_checked(BOOL_FALSE_MARKER), 0);
        assert_eq!(int_untag_checked(BOOL_TRUE_MARKER), 1);
        assert_eq!(pycc_rt_int_truthy(BOOL_FALSE_MARKER), 0);
        assert_eq!(pycc_rt_int_truthy(BOOL_TRUE_MARKER), 1);
        assert_eq!(pycc_rt_int_to_float(BOOL_FALSE_MARKER), 0.0);
        assert_eq!(pycc_rt_int_to_float(BOOL_TRUE_MARKER), 1.0);

        unsafe {
            let false_text = int_to_str(BOOL_FALSE_MARKER);
            let true_text = int_to_str(BOOL_TRUE_MARKER);
            assert_eq!((&*false_text).bytes(), b"False");
            assert_eq!((&*true_text).bytes(), b"True");
            pycc_rt_str_decref(false_text);
            pycc_rt_str_decref(true_text);
        }
    }

    #[test]
    fn arithmetic_consumes_bool_markers_and_produces_ordinary_ints() {
        let cases = [
            (int_add(BOOL_TRUE_MARKER, tag_smallint(1)), 2),
            (int_add(tag_smallint(1), BOOL_TRUE_MARKER), 2),
            (int_add(BOOL_FALSE_MARKER, tag_smallint(2)), 2),
            (int_add(tag_smallint(2), BOOL_FALSE_MARKER), 2),
            (int_sub(BOOL_TRUE_MARKER, tag_smallint(1)), 0),
            (int_sub(tag_smallint(1), BOOL_TRUE_MARKER), 0),
            (int_mul(BOOL_TRUE_MARKER, tag_smallint(2)), 2),
            (int_mul(tag_smallint(2), BOOL_TRUE_MARKER), 2),
            (int_mul(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
            (int_mul(tag_smallint(2), BOOL_FALSE_MARKER), 0),
            (int_floordiv(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
            (int_floordiv(tag_smallint(2), BOOL_TRUE_MARKER), 2),
            (int_floormod(BOOL_FALSE_MARKER, tag_smallint(2)), 0),
            (int_floormod(tag_smallint(2), BOOL_TRUE_MARKER), 0),
            (int_pow(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), 0),
            (int_pow(tag_smallint(2), BOOL_TRUE_MARKER), 2),
            (int_pow(BOOL_TRUE_MARKER, tag_smallint(2)), 1),
            (int_pow(tag_smallint(2), BOOL_FALSE_MARKER), 1),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, tag_smallint(expected));
            assert!(is_smallint(actual));
        }

        assert_eq!(int_cmp(BOOL_FALSE_MARKER, tag_smallint(0)), 0);
        assert_eq!(int_cmp(tag_smallint(1), BOOL_TRUE_MARKER), 0);
        assert_eq!(int_cmp(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), -1);
        assert_eq!(
            range_continue(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER, BOOL_TRUE_MARKER),
            1
        );
        assert_eq!(
            range_continue(BOOL_TRUE_MARKER, BOOL_TRUE_MARKER, BOOL_TRUE_MARKER),
            0
        );
    }

    #[test]
    fn false_identity_marker_is_a_zero_range_step() {
        // D-173/#150: `False` decodes to the smallint `0`, so this hits the
        // same zero-step guard as any other literal zero and now sets a
        // pending `ValueError` instead of panicking.
        pycc_rt_exception_clear();
        let result = range_continue(tag_smallint(0), tag_smallint(1), BOOL_FALSE_MARKER);
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(result, 0); // sentinel value
        let obj = exception::pycc_rt_exception_value();
        assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
        pycc_rt_exception_clear();
    }

    #[test]
    fn add_and_sub_accept_bool_markers_next_to_bigints_in_both_positions() {
        fn assert_value(encoded: i64, expected: i128) {
            let expected = bigint_from_i128(expected);
            let (actual_negative, actual_limbs) = to_sign_and_magnitude(encoded);
            assert_eq!(actual_negative, expected.negative);
            assert_eq!(actual_limbs, expected.limbs);
        }

        let magnitude = 1i128 << 80;
        let bigint = tag_bigint(bigint_from_i128(magnitude));
        assert_value(int_add(bigint, BOOL_TRUE_MARKER), magnitude + 1);
        assert_value(int_add(BOOL_TRUE_MARKER, bigint), magnitude + 1);
        assert_value(int_sub(bigint, BOOL_TRUE_MARKER), magnitude - 1);
        assert_value(int_sub(BOOL_TRUE_MARKER, bigint), 1 - magnitude);
    }

    #[test]
    fn pycc_rt_int_print_prints_the_untagged_decimal_value() {
        // stdout is captured by the test harness; this exercises
        // `pycc_rt_int_print` itself (not just `pycc_rt_print_i64`) for the
        // D-014 gate, same rationale as this file's existing
        // `extern_c_entry_point_runs_for_positive_negative_and_zero` test.
        pycc_rt_int_print(tag_smallint(42));
        pycc_rt_int_print(tag_smallint(-7));
    }

    #[test]
    fn pycc_rt_int_truthy_is_false_only_for_zero() {
        assert_eq!(pycc_rt_int_truthy(tag_smallint(0)), 0);
        assert_eq!(pycc_rt_int_truthy(tag_smallint(1)), 1);
        assert_eq!(pycc_rt_int_truthy(tag_smallint(-1)), 1);
    }

    #[test]
    fn pycc_rt_range_continue_handles_positive_step() {
        assert_eq!(
            pycc_rt_range_continue(tag_smallint(0), tag_smallint(3), tag_smallint(1)),
            1
        );
        assert_eq!(
            pycc_rt_range_continue(tag_smallint(3), tag_smallint(3), tag_smallint(1)),
            0
        );
    }

    #[test]
    fn pycc_rt_range_continue_handles_negative_step() {
        assert_eq!(
            pycc_rt_range_continue(tag_smallint(3), tag_smallint(0), tag_smallint(-1)),
            1
        );
        assert_eq!(
            pycc_rt_range_continue(tag_smallint(0), tag_smallint(0), tag_smallint(-1)),
            0
        );
    }

    #[test]
    fn pycc_rt_range_continue_with_a_zero_step_sets_exception_flag() {
        // D-173/#150: a zero step (inline fast path) now sets a pending
        // `ValueError` instead of panicking, and returns the ordinary
        // "stop" sentinel. Clear the flag first for test isolation, mirroring
        // `pycc_rt_int_floordiv_by_zero_sets_exception_flag` above.
        pycc_rt_exception_clear();
        let result = range_continue(tag_smallint(0), tag_smallint(3), tag_smallint(0));
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(result, 0); // sentinel value
        let obj = exception::pycc_rt_exception_value();
        assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
        pycc_rt_exception_clear();
    }

    /// A bigint holding exactly `2^62` -- the smallest magnitude that does
    /// not round-trip through the tagged 63-bit encoding, i.e. the first
    /// value `fits_smallint` rejects.
    fn bigint_two_pow_62() -> i64 {
        tag_bigint(bigint_from_i128(1i128 << 62))
    }

    /// A *bigint-tagged zero*, produced the way production code produces one:
    /// `bigint_add_signed`'s equal-magnitude, opposite-sign case. Its
    /// `negative` flag is `false`, which is exactly why sign must never be
    /// read before magnitude.
    fn bigint_zero() -> i64 {
        let big = bigint_two_pow_62();
        let zero = int_sub(big, big);
        assert_eq!(classify_encoded_int(zero), EncodedIntKind::BigInt);
        zero
    }

    #[test]
    fn magnitude_sign_reads_zero_from_the_magnitude_not_the_negative_flag() {
        assert_eq!(magnitude_sign(false, &[0]), 0);
        // The trap this helper exists for: a normalized bigint zero carries
        // `negative: false`, so "not negative" alone would read as positive.
        assert_eq!(magnitude_sign(true, &[0, 0]), 0);
        assert_eq!(magnitude_sign(false, &[1]), 1);
        assert_eq!(magnitude_sign(true, &[1]), -1);
    }

    #[test]
    fn encoded_int_cmp_orders_inline_operands_without_decoding() {
        assert_eq!(encoded_int_cmp(tag_smallint(1), tag_smallint(2)), Less);
        assert_eq!(encoded_int_cmp(tag_smallint(2), tag_smallint(1)), Greater);
        assert_eq!(encoded_int_cmp(tag_smallint(2), tag_smallint(2)), Equal);
        assert_eq!(encoded_int_cmp(BOOL_FALSE_MARKER, tag_smallint(0)), Equal);
        assert_eq!(encoded_int_cmp(BOOL_TRUE_MARKER, tag_smallint(1)), Equal);
        assert_eq!(encoded_int_cmp(BOOL_FALSE_MARKER, BOOL_TRUE_MARKER), Less);
    }

    #[test]
    fn encoded_int_cmp_orders_bigints_against_inline_operands_in_both_positions() {
        let big = bigint_two_pow_62();
        let negative_big = int_sub(tag_smallint(0), big);
        assert_eq!(encoded_int_cmp(big, tag_smallint(i64::MAX >> 1)), Greater);
        assert_eq!(encoded_int_cmp(tag_smallint(i64::MAX >> 1), big), Less);
        assert_eq!(encoded_int_cmp(negative_big, tag_smallint(0)), Less);
        assert_eq!(encoded_int_cmp(tag_smallint(0), negative_big), Greater);
        // Opposite signs on both sides of the comparison.
        assert_eq!(encoded_int_cmp(big, negative_big), Greater);
        assert_eq!(encoded_int_cmp(negative_big, big), Less);
    }

    #[test]
    fn encoded_int_cmp_orders_two_bigints_of_the_same_sign_by_magnitude() {
        let big = bigint_two_pow_62();
        let bigger = int_add(big, tag_smallint(1));
        assert_eq!(encoded_int_cmp(big, bigger), Less);
        assert_eq!(encoded_int_cmp(bigger, big), Greater);
        assert_eq!(encoded_int_cmp(big, int_add(big, tag_smallint(0))), Equal);

        // Two negatives: the larger magnitude is the *smaller* value.
        let negative = int_sub(tag_smallint(0), big);
        let more_negative = int_sub(tag_smallint(0), bigger);
        assert_eq!(encoded_int_cmp(more_negative, negative), Less);
        assert_eq!(encoded_int_cmp(negative, more_negative), Greater);
    }

    #[test]
    fn encoded_int_cmp_treats_a_bigint_zero_as_zero() {
        let zero = bigint_zero();
        assert_eq!(encoded_int_cmp(zero, tag_smallint(0)), Equal);
        assert_eq!(encoded_int_cmp(zero, bigint_zero()), Equal);
        assert_eq!(encoded_int_cmp(zero, tag_smallint(1)), Less);
        assert_eq!(encoded_int_cmp(tag_smallint(-1), zero), Less);
    }

    #[test]
    fn range_normalize_operand_maps_bool_markers_to_ordinary_smallints() {
        // D-141's contract, restated by D-179: `range` consumes the numeric
        // value and produces ordinary integer objects, so the identity
        // markers must not survive into the induction variable.
        assert_eq!(range_normalize_operand(BOOL_FALSE_MARKER), tag_smallint(0));
        assert_eq!(range_normalize_operand(BOOL_TRUE_MARKER), tag_smallint(1));
    }

    #[test]
    fn range_normalize_operand_passes_smallints_and_bigints_through_unchanged() {
        assert_eq!(range_normalize_operand(tag_smallint(7)), tag_smallint(7));
        assert_eq!(range_normalize_operand(tag_smallint(-7)), tag_smallint(-7));
        let big = bigint_two_pow_62();
        // The #147 defect in one assertion: this used to abort.
        assert_eq!(range_normalize_operand(big), big);
        // The public wrapper's own line, on a non-panicking input.
        assert_eq!(pycc_rt_range_normalize_operand(big), big);
    }

    #[test]
    #[should_panic(expected = "pycc_rt: invalid encoded int word")]
    fn range_normalize_operand_rejects_a_malformed_word() {
        // Calls the private fn for the usual `extern "C"` abort reason.
        range_normalize_operand(0);
    }

    #[test]
    fn range_continue_drives_a_loop_whose_operands_are_bigints() {
        let start = bigint_two_pow_62();
        let stop = int_add(start, tag_smallint(2));
        assert_eq!(range_continue(start, stop, tag_smallint(1)), 1);
        assert_eq!(range_continue(stop, stop, tag_smallint(1)), 0);
        // A bigint step, ascending and descending.
        assert_eq!(range_continue(tag_smallint(0), start, start), 1);
        assert_eq!(
            range_continue(
                tag_smallint(0),
                tag_smallint(-1),
                int_sub(tag_smallint(0), start)
            ),
            1
        );
    }

    #[test]
    fn range_continue_stops_when_the_induction_variable_promotes_mid_loop() {
        // The exact shape `range(4611686018427387900, 4611686018427387903, 2)`
        // reaches: `start`/`stop`/`step` all stay smallints, but the third
        // `int_add` produces 2^62, which `fits_smallint` rejects. The loop
        // guard then sees a *bigint* `i` against a *smallint* `stop`.
        let stop = tag_smallint(4_611_686_018_427_387_903);
        let promoted = int_add(tag_smallint(4_611_686_018_427_387_902), tag_smallint(2));
        assert_eq!(classify_encoded_int(promoted), EncodedIntKind::BigInt);
        assert_eq!(range_continue(promoted, stop, tag_smallint(2)), 0);
    }

    #[test]
    fn range_continue_rejects_a_bigint_zero_step() {
        // A bigint-tagged zero is *numerically* zero even though its word is
        // a non-zero pointer, so the guard must compare values, not words.
        // D-173/#150: the general (`encoded_int_cmp`) path also sets a
        // pending `ValueError` instead of panicking now.
        pycc_rt_exception_clear();
        let result = range_continue(tag_smallint(0), tag_smallint(10), bigint_zero());
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(result, 0); // sentinel value
        let obj = exception::pycc_rt_exception_value();
        assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_VALUE_ERROR);
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_int_to_float_converts_the_untagged_value() {
        assert_eq!(pycc_rt_int_to_float(tag_smallint(5)), 5.0);
        assert_eq!(pycc_rt_int_to_float(tag_smallint(-3)), -3.0);
    }

    #[test]
    fn int_untag_checked_recovers_the_original_value() {
        // Calls the public `extern "C"` wrapper (not the private
        // `int_untag_checked`) on purpose: this is the non-panicking path,
        // so no unwind ever crosses that boundary, and it is what covers
        // the wrapper's own line -- the same split this file's `int_add`
        // implementation note establishes for every panicking
        // `pycc_rt_int_*` symbol.
        for n in [0i64, 1, -1, 42, -42, i64::MAX >> 1, -(i64::MAX >> 1)] {
            assert_eq!(pycc_rt_int_untag_checked(tag_smallint(n)), n);
        }
    }

    #[test]
    #[should_panic(expected = "pycc_rt: int boundary does not support bigint-valued values yet")]
    fn int_untag_checked_rejects_a_bigint_tagged_value() {
        // A bigint-tagged value has its low bit clear (see TAG_BIT/
        // is_smallint). `tag_bigint` itself takes a `BigIntObj`, not a bare
        // i64 -- rather than hand-craft a bigint-tagged bit pattern from
        // scratch, force a real overflow through the existing arithmetic
        // path, exactly like this file's own `repeated_addition_exercises_
        // the_general_bigint_plus_smallint_path` test already does:
        // `i64::MAX >> 1` is exactly the largest value that still
        // round-trips through tagging, so adding it to itself overflows and
        // promotes via `int_add`'s own `tag_bigint` path.
        //
        // The setup call uses the public `pycc_rt_int_add` wrapper (that
        // path does not panic); the assertion below calls the *private*
        // `int_untag_checked`, because a panic unwinding past a plain
        // `extern "C" fn`'s boundary aborts the whole test binary instead
        // of being caught by `#[should_panic]` -- see the implementation
        // note above `int_add`.
        let bigint_tagged =
            pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(i64::MAX >> 1));
        assert!(
            !is_smallint(bigint_tagged),
            "test setup: expected this addition to overflow into a real bigint"
        );
        int_untag_checked(bigint_tagged);
    }

    #[test]
    fn pycc_rt_float_floordiv_matches_python_floor_semantics() {
        assert_eq!(pycc_rt_float_floordiv(7.0, 2.0), 3.0);
        assert_eq!(pycc_rt_float_floordiv(-7.0, 2.0), -4.0);
        assert_eq!(pycc_rt_float_floordiv(1.0, 0.1), 9.0);

        // Exercises CPython's quotient snap-up correction: the intermediate
        // quotient is -5603572390.000001, whose `floor()` alone is one low.
        assert_eq!(
            pycc_rt_float_floordiv(
                f64::from_bits(0x8ec2_3615_82f2_e770),
                f64::from_bits(0x0cbb_eab0_bc9a_0e0c),
            ),
            -5_603_572_390.0
        );

        let negative_zero = pycc_rt_float_floordiv(-0.0, 2.0);
        assert_eq!(negative_zero, 0.0);
        assert!(negative_zero.is_sign_negative());
    }

    #[test]
    fn pycc_rt_float_div_computes_a_nonzero_division() {
        assert_eq!(pycc_rt_float_div(7.0, 2.0), 3.5);
    }

    #[test]
    fn float_div_rejects_a_zero_divisor() {
        // D-173: float division by zero now sets the exception flag
        // instead of panicking.
        pycc_rt_exception_clear();
        let result = float_div(1.0, -0.0);
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(result, 0.0);
        pycc_rt_exception_clear();
    }

    #[test]
    fn float_divmod_rejects_a_zero_divisor() {
        // D-173: float floor division/modulo by zero now sets the
        // exception flag instead of panicking.
        pycc_rt_exception_clear();
        let (q, r) = float_divmod(1.0, 0.0);
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(q, 0.0);
        assert_eq!(r, 0.0);
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_float_floormod_matches_python_floor_semantics() {
        assert_eq!(pycc_rt_float_floormod(-7.0, 2.0), 1.0);
        assert_eq!(pycc_rt_float_floormod(7.0, 2.0), 1.0);
        let negative_zero = pycc_rt_float_floormod(8.0, -2.0);
        assert_eq!(negative_zero, 0.0);
        assert!(negative_zero.is_sign_negative());
    }

    #[test]
    fn pycc_rt_float_pow_computes_the_correct_power() {
        assert_eq!(pycc_rt_float_pow(2.0, 10.0), 1024.0);
        assert_eq!(pycc_rt_float_pow(9.0, 0.5), 3.0);
        assert_eq!(pycc_rt_float_pow(2.0, -1.0), 0.5);
        assert_eq!(pycc_rt_float_pow(-1.0, f64::INFINITY), 1.0);
        assert_eq!(pycc_rt_float_pow(-2.0, f64::NEG_INFINITY), 0.0);
        assert_eq!(pycc_rt_float_pow(0.0, f64::NEG_INFINITY), f64::INFINITY);
        assert_eq!(pycc_rt_float_pow(f64::NEG_INFINITY, 0.5), f64::INFINITY);
        assert!(pycc_rt_float_pow(-1.0, f64::NAN).is_nan());
    }

    #[test]
    fn pycc_rt_float_pow_accepts_a_negative_base_with_an_integer_exponent() {
        // An integer exponent (positive or negative) always yields a real
        // result, matching `python3.13`: `(-2.0) ** 3 == -8.0`,
        // `(-2.0) ** -3 == -0.125`, `(-2.0) ** 2 == 4.0`.
        assert_eq!(pycc_rt_float_pow(-2.0, 3.0), -8.0);
        assert_eq!(pycc_rt_float_pow(-2.0, -3.0), -0.125);
        assert_eq!(pycc_rt_float_pow(-2.0, 2.0), 4.0);
    }

    #[test]
    fn pycc_rt_float_pow_accepts_a_tiny_finite_result() {
        // Verified against `python3.13`: `2.0 ** -1024.0` produces a tiny
        // finite positive value, no exception -- only *overflow* to infinity
        // is a Python `OverflowError`, not a representable subnormal result.
        let result = pycc_rt_float_pow(2.0, -1024.0);
        assert!(result > 0.0 && result < 1e-300);
    }

    #[test]
    fn float_pow_raises_zero_division_error_for_zero_to_a_negative_power() {
        // Verified against `python3.13`: `0.0 ** -1.0` raises
        // `ZeroDivisionError: 0.0 cannot be raised to a negative power`,
        // while `f64::powf` alone silently returns `inf`. Part A of #1038
        // (#1063) turned the former `panic!` into this D-173 raise. Calls the
        // private `float_pow` directly, not the public `extern "C"` wrapper,
        // keeping this module's convention for the passing arms; the raise
        // itself would now cross the wrapper safely.
        pycc_rt_exception_clear();
        let result = float_pow(0.0, -1.0);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_ZERO_DIV_ERROR);
        // CPython's own sentence, verbatim -- the `pycc_rt: ` prefix the
        // panic carried is gone, since a raised message is user-visible.
        assert_eq!(message, "0.0 cannot be raised to a negative power");
        assert_eq!(result, 0.0); // sentinel value
        pycc_rt_exception_clear();
    }

    #[test]
    fn float_pow_raises_zero_division_error_for_negative_zero_to_a_negative_power() {
        // `-0.0 == 0.0` under IEEE-754, and `python3.13` raises the same
        // `ZeroDivisionError` for `(-0.0) ** -1.0` as for `0.0 ** -1.0`.
        pycc_rt_exception_clear();
        let result = float_pow(-0.0, -1.0);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_ZERO_DIV_ERROR);
        assert_eq!(message, "0.0 cannot be raised to a negative power");
        assert_eq!(result, 0.0);
        pycc_rt_exception_clear();
    }

    #[test]
    fn float_pow_raises_runtime_error_for_a_negative_base_and_a_non_integer_power() {
        // Verified against `python3.13`: `(-2.0) ** 3.5` returns a complex
        // number (`pycc` has no complex type), while `f64::powf` alone
        // silently returns `NaN`. `RuntimeError` is the deliberate deviation
        // -- there is no conformant class, because CPython raises nothing.
        pycc_rt_exception_clear();
        let result = float_pow(-2.0, 3.5);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
        assert_eq!(
            message,
            "a negative float raised to a non-integer power is not supported yet (would require a complex result)"
        );
        assert_eq!(result, 0.0);
        pycc_rt_exception_clear();
    }

    #[test]
    fn float_pow_raises_overflow_error_for_a_finite_result_outside_float_range() {
        // Verified against `python3.13`: `2.0 ** 1024.0` raises
        // `OverflowError: (34, 'Result too large')`, while `f64::powf` alone
        // silently returns `inf`. This is the arm the new tag exists for.
        pycc_rt_exception_clear();
        let result = float_pow(2.0, 1024.0);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_OVERFLOW_ERROR);
        assert_eq!(
            message,
            "float power overflowed (result too large to represent)"
        );
        assert_eq!(result, 0.0);
        pycc_rt_exception_clear();
    }

    /// Part A of #1038 (#1063), §4b-bis: `raise_builtin` installs pending
    /// state unconditionally, so a later raise on the same thread relabels an
    /// earlier one rather than being suppressed by it. The zero-base arm's
    /// `return` is what keeps that from happening *inside* one `float_pow`
    /// call -- without it, `0.0 ** -1.0` would fall through to `powf`,
    /// produce `inf`, and have its `ZeroDivisionError` overwritten by the
    /// overflow arm. Across *statements* the relabelling is reachable in
    /// generated code too, precisely because `Pow` emits no checkpoint of its
    /// own: two consecutive `**` assignments with no fallible expression
    /// between them leave only the second raise pending. That is the
    /// native-mode residual D-244's 2026-09-13 amendment defers, not a
    /// property this test claims away; what the `return` guarantees is the
    /// narrower thing pinned here -- that no single `float_pow` call can
    /// relabel its own raise.
    #[test]
    fn a_later_pow_raise_relabels_an_earlier_one_but_never_within_one_call() {
        pycc_rt_exception_clear();
        assert_eq!(float_pow(0.0, -1.0), 0.0);
        assert_eq!(pending_tag_and_message().0, EXCEPTION_TYPE_ZERO_DIV_ERROR);
        assert_eq!(float_pow(2.0, 1024.0), 0.0);
        assert_eq!(pending_tag_and_message().0, EXCEPTION_TYPE_OVERFLOW_ERROR);
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_short_literal_round_trips_through_the_inline_representation() {
        // Every `pycc_rt_str_*` function is `unsafe extern "C"` (see their
        // own doc comments' `# Safety` sections) -- a genuinely public,
        // pointer-taking FFI entry point, unlike every plain-`i64`
        // `pycc_rt_int_*` function above, so calling one directly (even
        // from this crate's own same-binary Rust tests) needs an explicit
        // `unsafe` block, upholding each call's own documented precondition.
        unsafe {
            let bytes = b"hi";
            let s = pycc_rt_str_from_literal(bytes.as_ptr(), bytes.len() as i64);
            assert_eq!((*s).bytes(), b"hi");
            pycc_rt_str_decref(s);
        }
    }

    #[test]
    fn a_long_literal_round_trips_through_the_heap_representation() {
        unsafe {
            let long = "x".repeat(23); // one byte past the 22-byte inline cap (D-059)
            let s = pycc_rt_str_from_literal(long.as_ptr(), long.len() as i64);
            assert_eq!((*s).bytes(), long.as_bytes());
            pycc_rt_str_decref(s);
        }
    }

    #[test]
    fn the_ext_str_accessor_exposes_the_bytes_of_both_payload_representations() {
        // The D-244 hosted `ext` shim reads a returned `str` only through
        // this accessor, so it has to see through both arms of the D-059
        // inline/heap split -- the shim has no way to tell them apart and
        // must not care. An embedded NUL and the empty string are covered
        // here too, because the accessor's length-out contract is what makes
        // both round-trip on the boundary instead of truncating at `strlen`.
        unsafe {
            let mut len = usize::MAX;

            let inline = pycc_rt_str_from_literal(b"hi\0there".as_ptr(), 8);
            let ptr = pycc_rt_ext_str_bytes(inline, &raw mut len);
            assert_eq!(len, 8);
            assert_eq!(std::slice::from_raw_parts(ptr, len), b"hi\0there");
            pycc_rt_str_decref(inline);

            let long = "x".repeat(23); // one byte past the 22-byte inline cap (D-059)
            let heap = pycc_rt_str_from_literal(long.as_ptr(), long.len() as i64);
            let ptr = pycc_rt_ext_str_bytes(heap, &raw mut len);
            assert_eq!(len, 23);
            assert_eq!(std::slice::from_raw_parts(ptr, len), long.as_bytes());
            pycc_rt_str_decref(heap);

            let empty = pycc_rt_str_from_literal(b"".as_ptr(), 0);
            let ptr = pycc_rt_ext_str_bytes(empty, &raw mut len);
            assert_eq!(len, 0);
            assert!(!ptr.is_null());
            pycc_rt_str_decref(empty);
        }
    }

    #[test]
    fn concat_joins_bytes_from_both_operands() {
        unsafe {
            let a = pycc_rt_str_from_literal(b"foo".as_ptr(), 3);
            let b = pycc_rt_str_from_literal(b"bar".as_ptr(), 3);
            let joined = pycc_rt_str_concat(a, b);
            assert_eq!((*joined).bytes(), b"foobar");
            pycc_rt_str_decref(a);
            pycc_rt_str_decref(b);
            pycc_rt_str_decref(joined);
        }
    }

    #[test]
    fn repeat_concatenates_the_operand_count_times() {
        unsafe {
            let s = pycc_rt_str_from_literal(b"ab".as_ptr(), 2);
            let thrice = pycc_rt_str_repeat(s, 3);
            assert_eq!((*thrice).bytes(), b"ababab");
            let once = pycc_rt_str_repeat(s, 1);
            assert_eq!((*once).bytes(), b"ab");
            // Past D-059's 22-byte inline cap, so the heap payload branch of
            // `new_pystr` is exercised through repetition too, not only
            // through a long literal.
            let long = pycc_rt_str_repeat(s, 12);
            assert_eq!((*long).bytes(), "ab".repeat(12).as_bytes());
            pycc_rt_str_decref(s);
            pycc_rt_str_decref(thrice);
            pycc_rt_str_decref(once);
            pycc_rt_str_decref(long);
        }
    }

    #[test]
    fn repeat_yields_the_empty_string_for_a_non_positive_count() {
        // CPython: `"ab" * 0` and `"ab" * -3` are both `""`. The negative
        // case is reachable from real source (`n = 0 - 3`) even though a
        // `-3` literal is not yet lowered (#573).
        unsafe {
            let s = pycc_rt_str_from_literal(b"ab".as_ptr(), 2);
            let zero = pycc_rt_str_repeat(s, 0);
            let negative = pycc_rt_str_repeat(s, -3);
            assert_eq!((*zero).bytes(), b"");
            assert_eq!((*negative).bytes(), b"");
            pycc_rt_str_decref(s);
            pycc_rt_str_decref(zero);
            pycc_rt_str_decref(negative);
        }
    }

    #[test]
    fn cmp_orders_strings_lexicographically() {
        unsafe {
            let a = pycc_rt_str_from_literal(b"apple".as_ptr(), 5);
            let b = pycc_rt_str_from_literal(b"banana".as_ptr(), 6);
            assert_eq!(pycc_rt_str_cmp(a, a), 0);
            assert_eq!(pycc_rt_str_cmp(a, b), -1);
            assert_eq!(pycc_rt_str_cmp(b, a), 1);
            pycc_rt_str_decref(a);
            pycc_rt_str_decref(b);
        }
    }

    #[test]
    fn truthy_is_false_only_for_the_empty_string() {
        unsafe {
            let empty = pycc_rt_str_from_literal(b"".as_ptr(), 0);
            let non_empty = pycc_rt_str_from_literal(b"x".as_ptr(), 1);
            assert_eq!(pycc_rt_str_truthy(empty), 0);
            assert_eq!(pycc_rt_str_truthy(non_empty), 1);
            pycc_rt_str_decref(empty);
            pycc_rt_str_decref(non_empty);
        }
    }

    #[test]
    fn incref_then_decref_survives_until_the_final_decref() {
        unsafe {
            let s = pycc_rt_str_from_literal(b"hi".as_ptr(), 2);
            pycc_rt_str_incref(s); // rc 1 -> 2
            pycc_rt_str_decref(s); // rc 2 -> 1, must NOT free yet
            assert_eq!(pycc_rt_str_cmp(s, s), 0); // still safe to read
            pycc_rt_str_decref(s); // rc 1 -> 0, frees
        }
    }

    #[test]
    fn incref_and_decref_on_a_null_pointer_are_safe_no_ops() {
        unsafe {
            pycc_rt_str_incref(std::ptr::null_mut());
            pycc_rt_str_decref(std::ptr::null_mut());
        }
    }

    #[test]
    fn pycc_rt_int_to_str_formats_the_untagged_decimal_value() {
        // Deviation from the task brief: the brief's own version of this
        // (and every other new test below) called `pycc_rt_str_decref`
        // directly, unwrapped -- but that function is `pub unsafe extern
        // "C" fn` (see its own doc comment/safety section above), so an
        // unwrapped call doesn't compile (`E0133`, "call to unsafe function
        // ... is unsafe and requires unsafe block or unsafe function").
        // Every `pycc_rt_str_decref` call below is wrapped in its own
        // `unsafe { ... }` block, matching every other test in this file
        // that already calls an `unsafe fn` (e.g. `cmp_orders_strings_
        // lexicographically` above).
        let s = pycc_rt_int_to_str(tag_smallint(42));
        assert_eq!(unsafe { &*s }.bytes(), b"42");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_int_to_str(tag_smallint(-7));
        assert_eq!(unsafe { &*s }.bytes(), b"-7");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn pycc_rt_bool_to_str_matches_python_s_capitalized_spelling() {
        let s = pycc_rt_bool_to_str(1);
        assert_eq!(unsafe { &*s }.bytes(), b"True");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_bool_to_str(0);
        assert_eq!(unsafe { &*s }.bytes(), b"False");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn pycc_rt_float_to_str_always_shows_a_decimal_point() {
        // CPython: `str(3.0) == "3.0"`, not `"3"` -- unlike Rust's own `{}`
        // `Display` for `f64`, which omits the fractional part entirely for
        // a whole-number value.
        let s = pycc_rt_float_to_str(3.0);
        assert_eq!(unsafe { &*s }.bytes(), b"3.0");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_float_to_str(2.5);
        assert_eq!(unsafe { &*s }.bytes(), b"2.5");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_float_to_str(-0.5);
        assert_eq!(unsafe { &*s }.bytes(), b"-0.5");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn pycc_rt_float_to_str_handles_infinity_and_nan_like_cpython() {
        // CPython: `str(float('inf')) == "inf"`, `str(float('nan')) == "nan"`
        // -- lowercase, unlike Rust's own `{}` (`"inf"`/`"NaN"`, capitalized
        // for `NaN`).
        let s = pycc_rt_float_to_str(f64::INFINITY);
        assert_eq!(unsafe { &*s }.bytes(), b"inf");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_float_to_str(f64::NEG_INFINITY);
        assert_eq!(unsafe { &*s }.bytes(), b"-inf");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_float_to_str(f64::NAN);
        assert_eq!(unsafe { &*s }.bytes(), b"nan");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn pycc_rt_float_to_str_raises_on_magnitudes_needing_scientific_notation() {
        // Part B of #1038 (#1064): was `#[should_panic]`. CPython's
        // `repr(float)` switches to scientific notation outside a specific
        // decimal-exponent range (verified against `python3.13`:
        // `repr(1e17)` is `'1e+17'`, not the full 18-digit expansion) --
        // reproducing that exact algorithm is still out of scope, but the
        // gap is now a catchable `RuntimeError` instead of a process abort
        // (never a silently wrong digit string). The sentinel is an empty
        // `str`, a valid object the caller may touch before its own check.
        pycc_rt_exception_clear();
        let result = pycc_rt_float_to_str(1e17);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
        assert!(message.contains("not supported yet"), "{message}");
        assert!(!message.contains("pycc_rt: "), "{message}");
        assert!(!result.is_null());
        assert_eq!(unsafe { &*result }.bytes(), b""); // sentinel value
        unsafe { pycc_rt_str_decref(result) };
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_float_to_str_raises_on_small_magnitudes_needing_scientific_notation() {
        // Same rationale as the large-magnitude case above, for the *low*
        // end of the supported range: verified against `python3.13`,
        // `repr(1e-5)` is `'1e-05'` (scientific), unlike `repr(1e-4)` which
        // is `'0.0001'` (still positional). Neither the given tests above
        // nor `pycc_rt_float_to_str_always_shows_a_decimal_point`/
        // `_accepts_the_boundary_just_inside_the_supported_range` ever drive
        // `magnitude < 1e-4` to `true` -- every value they use is either
        // `>= 1e-4` in magnitude or exactly `0.0`, so without this test the
        // small-magnitude half of that `||` would never actually fire.
        pycc_rt_exception_clear();
        let result = pycc_rt_float_to_str(1e-5);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR);
        // The message is the only non-static one of Part B's six: it
        // interpolates the offending value, so pin that too.
        assert!(message.contains("(0.00001)"), "{message}");
        assert!(message.contains("scientific notation"), "{message}");
        assert!(!result.is_null());
        assert_eq!(unsafe { &*result }.bytes(), b""); // sentinel value
        unsafe { pycc_rt_str_decref(result) };
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_float_to_str_accepts_the_boundary_just_inside_the_supported_range() {
        // Deviation from the task brief: the brief's own version of this
        // test called `pycc_rt_float_to_str(1e16)`, expecting
        // `b"10000000000000000.0"` -- but `1e16` is exactly the boundary
        // this function's own `magnitude >= 1e16` check *rejects* (verified
        // against `python3.13`: `repr(1e16)` is itself `'1e+16'`, scientific
        // notation, contradicting the brief test's expectation that it
        // stays positional). That test as written would either panic
        // (contradicting its own non-`#[should_panic]` assertion) or --
        // had the boundary check instead been written as `>` -- silently
        // accept a value CPython itself always renders in scientific
        // notation. Fixed to use the actual double just inside the
        // supported range: `9999999999999998.0`, the IEEE-754 `f64`
        // immediately below `1e16` (`ulp` there is `2.0`), which
        // `python3.13`'s own `repr`/`str` renders positionally as
        // `'9999999999999998.0'`.
        let s = pycc_rt_float_to_str(9999999999999998.0);
        assert_eq!(unsafe { &*s }.bytes(), b"9999999999999998.0");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn pycc_rt_float_to_str_handles_zero_and_negative_zero() {
        // CPython: `str(0.0) == "0.0"`, `str(-0.0) == "-0.0"` (verified
        // against `python3.13`) -- neither the brief's own given tests nor
        // any test above ever passes a zero magnitude, so the `magnitude !=
        // 0.0` short-circuit guard's `false` outcome (skipping the
        // large/small-magnitude check entirely) was otherwise never
        // exercised.
        let s = pycc_rt_float_to_str(0.0);
        assert_eq!(unsafe { &*s }.bytes(), b"0.0");
        unsafe { pycc_rt_str_decref(s) };
        let s = pycc_rt_float_to_str(-0.0);
        assert_eq!(unsafe { &*s }.bytes(), b"-0.0");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn adding_past_i64_range_now_promotes_instead_of_panicking() {
        // This is the exact fixture Task 3's
        // `pycc_rt_int_add_panics_on_overflow_before_bigint_promotion_exists`
        // used to require a panic for -- it must now succeed and print the
        // exact mathematical sum.
        let huge = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
        let s = pycc_rt_int_to_str(huge);
        assert_eq!(unsafe { &*s }.bytes(), b"4611686018427387904");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn int_from_i64_keeps_an_in_range_value_inline() {
        let encoded = pycc_rt_int_from_i64(42);
        assert_eq!(classify_encoded_int(encoded), EncodedIntKind::SmallInt);
        assert_eq!(untag_smallint(encoded), 42);
    }

    #[test]
    fn int_from_i64_promotes_values_outside_the_tagged_range() {
        // The four boundary literals `pycc_codegen` can no longer fold:
        // `2^62`, `i64::MAX`, `-(2^62) - 1`, and `i64::MIN`.
        for (value, expected) in [
            (4_611_686_018_427_387_904_i64, "4611686018427387904"),
            (i64::MAX, "9223372036854775807"),
            (-4_611_686_018_427_387_905_i64, "-4611686018427387905"),
            (i64::MIN, "-9223372036854775808"),
        ] {
            let encoded = pycc_rt_int_from_i64(value);
            assert_eq!(classify_encoded_int(encoded), EncodedIntKind::BigInt);
            let s = pycc_rt_int_to_str(encoded);
            assert_eq!(unsafe { &*s }.bytes(), expected.as_bytes());
            unsafe { pycc_rt_str_decref(s) };
        }
    }

    #[test]
    fn subtracting_past_the_negative_range_promotes_correctly() {
        let huge = pycc_rt_int_sub(tag_smallint(i64::MIN >> 1), tag_smallint(1));
        let s = pycc_rt_int_to_str(huge);
        assert_eq!(unsafe { &*s }.bytes(), b"-4611686018427387905");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn repeated_addition_exercises_the_general_bigint_plus_smallint_path() {
        // Simulates unbounded `fib`-style growth. The first `pycc_rt_int_add`
        // call overflows the tagged range (`i64::MAX >> 1` is exactly the
        // largest value that still round-trips through tagging, so adding
        // it to itself does not fit) and promotes via the one-time
        // `i128`-widening path. Every one of the 20 subsequent additions
        // then has a `BigIntObj` as its left operand and a tagged smallint
        // as its right operand -- `is_smallint(a) && is_smallint(b)` is
        // false for all of them, so they all go through
        // `to_sign_and_magnitude`/`bigint_add_signed`'s general limb
        // arithmetic, not the fast path or the one-time promotion shortcut.
        // (The running total here stays well under 128 bits -- about 67 at
        // the end -- this test is not about exceeding `i128`'s own range,
        // only about exercising the general bigint-arithmetic code path
        // repeatedly rather than just once.)
        let mut acc = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(i64::MAX >> 1));
        for _ in 0..20 {
            acc = pycc_rt_int_add(acc, tag_smallint(i64::MAX >> 1));
        }
        assert!(!is_smallint(acc));
        let s = pycc_rt_int_to_str(acc);
        let text = String::from_utf8(unsafe { &*s }.bytes().to_vec()).unwrap();
        assert_eq!(text, (bigint_reference_sum()).to_string());
        unsafe { pycc_rt_str_decref(s) };
    }

    /// Independent reference computation for the test above (22 additions
    /// of `i64::MAX >> 1`, well within `i128`'s own range) -- this doesn't
    /// exercise `pycc_rt`'s bigint code at all, so it's a trustworthy oracle
    /// for what the *correct* sum is, independent of any bug the code under
    /// test might have.
    fn bigint_reference_sum() -> i128 {
        let step = (i64::MAX >> 1) as i128;
        let mut acc = step + step;
        for _ in 0..20 {
            acc += step;
        }
        acc
    }

    #[test]
    fn a_bigint_that_would_fit_back_in_smallint_range_still_formats_correctly() {
        // Two already-promoted values that sum back to something small
        // (mathematically representable as a smallint) are not required to
        // shrink back down (D-061/this task's own "simplest correct" choice
        // -- once a value touches the bigint path, it stays represented as
        // one) -- but the printed *value* must still be exactly right.
        let a = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1)); // a bigint
        let b = pycc_rt_int_sub(tag_smallint(0), a); // -a, also a bigint (sub promotes too)
        let zero = pycc_rt_int_add(a, b);
        let s = pycc_rt_int_to_str(zero);
        assert_eq!(unsafe { &*s }.bytes(), b"0");
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn a_bigint_zero_is_falsy() {
        let a = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
        let b = pycc_rt_int_sub(tag_smallint(0), a);
        let zero = pycc_rt_int_add(a, b);
        assert_eq!(pycc_rt_int_truthy(zero), 0);
    }

    #[test]
    fn a_nonzero_bigint_is_truthy() {
        // Companion to `a_bigint_zero_is_falsy`: pins the *other* outcome of
        // `pycc_rt_int_truthy`'s bigint branch (a magnitude that is not a
        // single zero limb). `i64::MAX >> 1` is the largest tagged smallint,
        // so `+ 1` overflows the fixnum range and promotes to a real bigint.
        let huge = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
        assert!(
            !is_smallint(huge),
            "value must actually be a bigint, not a tagged smallint"
        );
        assert_eq!(pycc_rt_int_truthy(huge), 1);
    }

    #[test]
    fn a_bigint_print_path_runs_without_panicking() {
        // Exercises `int_print`'s bigint branch (`int_to_str` -> `println!`
        // -> `str_decref`). This in-process test only proves that path runs
        // without panicking; the "prints the decimal digits with a trailing
        // newline" behavior itself is asserted end-to-end by pycc_codegen's
        // `compiles_a_loop_whose_accumulator_overflows_into_a_bigint`, which
        // runs the compiled binary and checks its stdout is the digits plus
        // exactly one `\n` (capturing `println!` output inside a libtest unit
        // body would need stdout-redirect plumbing this crate does not have).
        let huge = pycc_rt_int_add(tag_smallint(i64::MAX >> 1), tag_smallint(1));
        pycc_rt_int_print(huge);
    }

    #[test]
    fn adding_two_opposite_sign_bigints_with_the_same_limb_count_but_different_magnitude() {
        // Every other opposite-sign addition/subtraction test above either
        // has exactly equal magnitudes (immediately `Ordering::Equal`,
        // never reaching `magnitude_cmp`'s per-limb loop body) or magnitudes
        // needing a different number of limbs (resolved by `magnitude_cmp`'s
        // own length check before the loop runs at all) -- so this is the
        // only test exercising `magnitude_cmp`'s `a[i] != b[i]` true branch,
        // `bigint_add_signed`'s `Ordering::Greater` arm, and
        // `magnitude_sub`'s borrow path (`diff < 0`), all at once.
        //
        // `step = i64::MAX >> 1 = 0x3FFF_FFFF_FFFF_FFFF` (low limb
        // `0xFFFF_FFFF`, high limb `0x3FFF_FFFF`). `big = 2 * step`,
        // `bigger = 3 * step` -- both promote to real 2-limb bigints (their
        // magnitude exceeds a single `u32`'s range), and `bigger`'s low limb
        // (`0xFFFF_FFFD`) is smaller than `big`'s low limb (`0xFFFF_FFFE`),
        // so subtracting them genuinely borrows from the high limb.
        // `bigger + (-big)` must equal `step` exactly.
        let step = i64::MAX >> 1;
        let big = pycc_rt_int_add(tag_smallint(step), tag_smallint(step));
        let bigger = pycc_rt_int_add(big, tag_smallint(step));
        let neg_big = pycc_rt_int_sub(tag_smallint(0), big);
        assert!(!is_smallint(big) && !is_smallint(bigger) && !is_smallint(neg_big));
        let result = pycc_rt_int_add(bigger, neg_big);
        let s = pycc_rt_int_to_str(result);
        assert_eq!(unsafe { &*s }.bytes(), step.to_string().as_bytes());
        unsafe { pycc_rt_str_decref(s) };
    }

    #[test]
    fn bigint_from_i128_of_zero_still_has_a_single_zero_limb() {
        // Not reachable through `int_add`/`int_sub`'s own overflow-promotion
        // call sites -- a mathematically-zero sum/difference always fits
        // the tagged smallint range, so `checked_add`/`checked_sub` +
        // `fits_smallint` succeed first and this fallback is never invoked
        // with `0`. Tested directly against the private helper instead,
        // matching this file's own convention of testing a general-purpose
        // private function's own contract rather than only the narrower
        // paths its current callers happen to exercise.
        let b = bigint_from_i128(0);
        assert!(!b.negative);
        assert_eq!(b.limbs, vec![0]);
    }

    #[test]
    fn print_write_str_writes_bytes_with_no_trailing_newline() {
        // stdout is captured by the test harness; this only proves the call
        // itself doesn't panic/crash (same rationale as this file's other
        // direct extern-fn exercises). `pycc_rt_str_from_literal`/`pycc_rt_
        // str_decref`/`pycc_rt_print_write_str` (see that function's own
        // doc comment for why it's `unsafe` too, a fix over the task
        // brief's own version) are all `unsafe extern "C" fn`s, so every
        // call below needs an `unsafe` block -- the task brief's own test
        // listing omitted it entirely, a genuine compile-error bug, fixed
        // here the same way this file's other direct `pycc_rt_str_from_
        // literal` call sites already do it.
        unsafe {
            let s = pycc_rt_str_from_literal(b"hi".as_ptr(), 2);
            pycc_rt_print_write_str(s);
            pycc_rt_str_decref(s);
        }
    }

    #[test]
    fn print_space_and_newline_and_none_do_not_panic() {
        pycc_rt_print_space();
        pycc_rt_print_newline();
        pycc_rt_print_none();
    }

    #[test]
    fn int_list_new_starts_empty() {
        // Every `pycc_rt_int_list_*` function taking/dereferencing a
        // `*mut PyIntListObj` is `unsafe extern "C"` (see their own doc
        // comments' `# Safety` sections), matching `PyStrObj`'s own
        // convention -- so, same as `a_short_literal_round_trips_through_
        // the_inline_representation` above, this wraps the whole body in
        // one `unsafe { ... }` block rather than the task brief's own
        // unwrapped calls (a genuine compile-error bug in the brief, not
        // a change to what the test asserts).
        unsafe {
            let list = pycc_rt_int_list_new();
            assert_eq!(pycc_rt_int_list_len(list), 0);
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn int_list_append_then_get_round_trips() {
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, tag_smallint(10));
            pycc_rt_int_list_append(list, tag_smallint(20));
            pycc_rt_int_list_append(list, tag_smallint(30));
            assert_eq!(pycc_rt_int_list_len(list), 3);
            assert_eq!(pycc_rt_int_list_get(list, 0), tag_smallint(10));
            assert_eq!(pycc_rt_int_list_get(list, 1), tag_smallint(20));
            assert_eq!(pycc_rt_int_list_get(list, 2), tag_smallint(30));
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn int_list_read_slice_and_pop_preserve_bool_identity_markers() {
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, BOOL_FALSE_MARKER);
            pycc_rt_int_list_append(list, BOOL_TRUE_MARKER);
            assert_eq!(pycc_rt_int_list_get(list, 0), BOOL_FALSE_MARKER);
            assert_eq!(pycc_rt_int_list_get(list, 1), BOOL_TRUE_MARKER);

            let sliced = pycc_rt_int_list_slice(list, 0, 2, 1);
            assert_eq!(pycc_rt_int_list_get(sliced, 0), BOOL_FALSE_MARKER);
            assert_eq!(pycc_rt_int_list_get(sliced, 1), BOOL_TRUE_MARKER);
            assert_eq!(pycc_rt_int_list_pop(list), BOOL_TRUE_MARKER);
            pycc_rt_int_list_decref(sliced);
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn int_list_grows_past_its_initial_capacity() {
        unsafe {
            let list = pycc_rt_int_list_new();
            for i in 0..1000 {
                pycc_rt_int_list_append(list, tag_smallint(i));
            }
            assert_eq!(pycc_rt_int_list_len(list), 1000);
            assert_eq!(pycc_rt_int_list_get(list, 999), tag_smallint(999));
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn int_list_get_out_of_range_sets_exception_flag() {
        // D-173: out-of-range index now sets the pending exception flag
        // instead of panicking.
        pycc_rt_exception_clear();
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, tag_smallint(1));
            let result = int_list_get(&*list, 5);
            assert_eq!(pycc_rt_exception_active(), 1);
            assert_eq!(untag_smallint(result), 0); // sentinel value
            pycc_rt_int_list_decref(list);
        }
        pycc_rt_exception_clear();
    }

    #[test]
    fn int_list_get_rejects_negative_indices_with_exception_flag() {
        // D-108's documented v0.2 scope cut: a negative index sets the
        // IndexError flag exactly like an out-of-range positive one.
        pycc_rt_exception_clear();
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, tag_smallint(1));
            let result = int_list_get(&*list, -1);
            assert_eq!(pycc_rt_exception_active(), 1);
            assert_eq!(untag_smallint(result), 0); // sentinel value
            pycc_rt_int_list_decref(list);
        }
        pycc_rt_exception_clear();
    }

    /// Part 2 of #1027: the Rust `PyccExtBufferView` is the same two words,
    /// in the same order and at the same offsets, as the C struct in
    /// `src/ext/pycc_ext_module.c` -- `{ void *ptr; long long len; }`. A
    /// silent disagreement here would misread every element, so it is pinned
    /// rather than assumed.
    #[test]
    fn ext_buffer_view_layout_matches_the_c_struct() {
        assert_eq!(
            core::mem::size_of::<PyccExtBufferView>(),
            core::mem::size_of::<*mut core::ffi::c_void>() + core::mem::size_of::<i64>()
        );
        assert_eq!(core::mem::align_of::<PyccExtBufferView>(), 8);
        let storage = [0.0f64; 1];
        let view = PyccExtBufferView {
            ptr: storage.as_ptr() as *mut core::ffi::c_void,
            len: 1,
        };
        let base = &view as *const PyccExtBufferView as usize;
        assert_eq!(&view.ptr as *const _ as usize - base, 0);
        assert_eq!(
            &view.len as *const _ as usize - base,
            core::mem::size_of::<*mut core::ffi::c_void>()
        );
    }

    /// Builds a view over `storage` for the buffer-read tests below.
    fn buffer_view_over(storage: &[f64]) -> PyccExtBufferView {
        PyccExtBufferView {
            ptr: storage.as_ptr() as *mut core::ffi::c_void,
            len: storage.len() as i64,
        }
    }

    #[test]
    fn buffer_f64_get_reads_every_in_range_element() {
        pycc_rt_exception_clear();
        let storage = [1.5f64, 2.5, 3.5];
        let view = buffer_view_over(&storage);
        unsafe {
            assert_eq!(buffer_f64_get(&view, 0), 1.5);
            assert_eq!(buffer_f64_get(&view, 1), 2.5);
            assert_eq!(buffer_f64_get(&view, 2), 3.5);
        }
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    #[test]
    fn buffer_f64_get_past_the_end_sets_the_index_error_flag() {
        // D-173: the pending-exception flag plus a type-valid sentinel, not
        // a panic across the FFI boundary.
        pycc_rt_exception_clear();
        let storage = [1.5f64, 2.5];
        let view = buffer_view_over(&storage);
        let result = unsafe { buffer_f64_get(&view, 2) };
        assert_eq!(result, 0.0);
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(
            pycc_rt_ext_pending_type(),
            i32::from(EXCEPTION_TYPE_INDEX_ERROR)
        );
        pycc_rt_exception_clear();
    }

    #[test]
    fn buffer_f64_get_rejects_negative_indices_with_exception_flag() {
        // D-108: `b[-1]` is refused rather than resolved from the end.
        pycc_rt_exception_clear();
        let storage = [1.5f64, 2.5];
        let view = buffer_view_over(&storage);
        let result = unsafe { buffer_f64_get(&view, -1) };
        assert_eq!(result, 0.0);
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    #[test]
    fn buffer_f64_get_on_an_empty_view_refuses_index_zero() {
        pycc_rt_exception_clear();
        let view = PyccExtBufferView {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
        let result = unsafe { buffer_f64_get(&view, 0) };
        assert_eq!(result, 0.0);
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    #[test]
    fn buffer_f64_set_writes_every_in_range_element() {
        pycc_rt_exception_clear();
        let mut storage = [0.0f64; 3];
        let view = PyccExtBufferView {
            ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
            len: 3,
        };
        unsafe {
            buffer_f64_set(&view, 0, 1.5);
            buffer_f64_set(&view, 1, 2.5);
            buffer_f64_set(&view, 2, 3.5);
        }
        assert_eq!(storage, [1.5, 2.5, 3.5]);
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    /// The four properties the `--ext` wrapper checks -- writable, one
    /// dimension, C-contiguous, format `"d"` -- say nothing about the
    /// alignment of the exporter's storage.
    /// `memoryview(bytearray(17))[1:].cast("d")` passes all four on CPython
    /// and reports a data address that is `1 mod 8`, so both accessors have
    /// to use the unaligned primitives; a plain `f64` dereference there is
    /// undefined behavior. An unaligned exporter must keep working, so this
    /// asserts a round trip rather than a refusal.
    #[test]
    fn buffer_f64_round_trips_through_a_misaligned_view() {
        pycc_rt_exception_clear();
        #[repr(align(8))]
        struct AlignedBytes([u8; 24]);
        let mut storage = AlignedBytes([0u8; 24]);
        // One byte past an 8-byte-aligned base, so every element address is
        // `1 mod 8` -- the same residue the `bytearray(17)[1:]` witness has.
        let base = unsafe { storage.0.as_mut_ptr().add(1) };
        assert_eq!(base as usize % core::mem::align_of::<f64>(), 1);
        let view = PyccExtBufferView {
            ptr: base as *mut core::ffi::c_void,
            len: 2,
        };
        unsafe {
            buffer_f64_set(&view, 0, 1.5);
            buffer_f64_set(&view, 1, -2.25);
            assert_eq!(buffer_f64_get(&view, 0), 1.5);
            assert_eq!(buffer_f64_get(&view, 1), -2.25);
        }
        // The writes landed at the misaligned offsets themselves, not at a
        // rounded-down aligned address: byte 0 is untouched.
        assert_eq!(storage.0[0], 0);
        assert_eq!(storage.0[1..9], 1.5f64.to_ne_bytes());
        assert_eq!(storage.0[9..17], (-2.25f64).to_ne_bytes());
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    #[test]
    fn buffer_f64_set_past_the_end_sets_the_index_error_flag_and_writes_nothing() {
        // D-173's flag, and the "returns without dereferencing `ptr`" half
        // of the contract: the storage is untouched.
        pycc_rt_exception_clear();
        let mut storage = [7.0f64, 8.0];
        let view = PyccExtBufferView {
            ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
            len: 2,
        };
        unsafe { buffer_f64_set(&view, 2, 99.0) };
        assert_eq!(storage, [7.0, 8.0]);
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(
            pycc_rt_ext_pending_type(),
            i32::from(EXCEPTION_TYPE_INDEX_ERROR)
        );
        pycc_rt_exception_clear();
    }

    #[test]
    fn buffer_f64_set_rejects_negative_indices_with_exception_flag() {
        // D-108: `b[-1] = v` is refused rather than resolved from the end,
        // exactly as the load refuses `b[-1]`.
        pycc_rt_exception_clear();
        let mut storage = [7.0f64, 8.0];
        let view = PyccExtBufferView {
            ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
            len: 2,
        };
        unsafe { buffer_f64_set(&view, -1, 99.0) };
        assert_eq!(storage, [7.0, 8.0]);
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    #[test]
    fn buffer_f64_set_on_an_empty_view_refuses_index_zero() {
        pycc_rt_exception_clear();
        let view = PyccExtBufferView {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
        unsafe { buffer_f64_set(&view, 0, 1.0) };
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    /// The `extern "C"` wrapper generated code actually calls, exercised
    /// directly for the same reason the load's is.
    #[test]
    fn pycc_rt_buffer_f64_set_wrapper_writes_and_raises() {
        pycc_rt_exception_clear();
        let mut storage = [0.0f64, 0.0];
        let view = PyccExtBufferView {
            ptr: storage.as_mut_ptr() as *mut core::ffi::c_void,
            len: 2,
        };
        unsafe {
            pycc_rt_buffer_f64_set(&view, 1, 4.25);
            assert_eq!(pycc_rt_exception_active(), 0);
            pycc_rt_buffer_f64_set(&view, 99, 1.0);
        }
        assert_eq!(storage, [0.0, 4.25]);
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    /// The `extern "C"` wrapper is what generated code actually calls, so it
    /// is exercised directly rather than only through its private half.
    #[test]
    fn pycc_rt_buffer_f64_get_wrapper_reads_and_raises() {
        pycc_rt_exception_clear();
        let storage = [4.25f64, 8.5];
        let view = buffer_view_over(&storage);
        unsafe {
            assert_eq!(pycc_rt_buffer_f64_get(&view, 1), 8.5);
            assert_eq!(pycc_rt_exception_active(), 0);
            assert_eq!(pycc_rt_buffer_f64_get(&view, 99), 0.0);
        }
        assert_eq!(pycc_rt_exception_active(), 1);
        pycc_rt_exception_clear();
    }

    /// `len(b)` is the `len` word verbatim, in elements, and never raises.
    #[test]
    fn pycc_rt_buffer_len_returns_the_element_count_and_never_raises() {
        pycc_rt_exception_clear();
        let storage = [1.0f64, 2.0, 3.0, 4.0];
        let view = buffer_view_over(&storage);
        let empty = PyccExtBufferView {
            ptr: core::ptr::null_mut(),
            len: 0,
        };
        unsafe {
            assert_eq!(pycc_rt_buffer_len(&view), 4);
            assert_eq!(pycc_rt_buffer_len(&empty), 0);
        }
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    /// The allocator pairing of #1165, pinned as a round trip: allocate,
    /// write every element through the same `buffer_f64_set` a compiled body
    /// uses, read them all back, and free. A mismatched deallocation layout
    /// is undefined behavior that no test notices by accident, so this test
    /// exists to give the debug allocator a well-formed pairing to check.
    #[test]
    fn buffer_alloc_untag_len_raises_instead_of_aborting_on_a_bigint_length() {
        // #1166 round 8's P1. `ndarray(2 ** 62 - 1)` on a built `--ext`
        // module used to decode its length through
        // `pycc_rt_int_untag_checked`, whose `panic!` becomes a process
        // abort at its own `extern "C"` boundary: the hosted repro exited
        // 134, killing the host CPython interpreter.
        //
        // Called through the `pub extern "C"` wrapper rather than the
        // private function -- the opposite of this module's convention for
        // `int_pow`/`int_to_float`, and deliberately so. That convention
        // exists because those wrappers *could* abort; this one cannot, and
        // calling it is what proves that at the exact boundary the defect
        // lived on. Both arms are asserted in both directions: the value and
        // the pending state.
        pycc_rt_exception_clear();
        assert_eq!(pycc_rt_buffer_alloc_untag_len(tag_smallint(4)), 4);
        assert_eq!(pycc_rt_exception_active(), 0);

        // D-086: a bool marker is a valid length and still decodes to 0/1.
        assert_eq!(pycc_rt_buffer_alloc_untag_len(tag_smallint(0)), 0);
        assert_eq!(pycc_rt_exception_active(), 0);

        let sentinel = pycc_rt_buffer_alloc_untag_len(a_bigint_word());
        assert_eq!(sentinel, 0, "the refusal returns the type-valid sentinel");
        assert_overflow_raised("sizing a buffer with");
        pycc_rt_exception_clear();
    }

    #[test]
    fn a_buffer_allocation_of_the_refusal_sentinel_would_itself_succeed() {
        // Why the decoder's raise is not self-guarding, and therefore why
        // `pycc_codegen` must branch away between the decode and the
        // allocator: the sentinel `0` is a *valid* length. The allocator
        // accepts it, raises nothing, and hands back a live view -- which on
        // the refusal path no `MirStmt::Assign` would ever store into the
        // frame's owned slot. This is the leak the emitted guard prevents,
        // pinned as a runtime property so the codegen ordering test above
        // has something to be an ordering *of*.
        pycc_rt_exception_clear();
        let view = pycc_rt_buffer_f64_alloc(0);
        assert!(!view.is_null(), "a zero length is admitted, not refused");
        assert_eq!(pycc_rt_exception_active(), 0);
        unsafe {
            assert_eq!(pycc_rt_buffer_len(view), 0);
            pycc_rt_buffer_f64_free(view);
        }
    }

    #[test]
    fn buffer_f64_alloc_raises_instead_of_aborting_on_an_unreservable_length() {
        // #1166 round 8's second abort door, found while writing the hosted
        // arm for the first: `ndarray(2 ** 62 - 1)` needs no bigint at all --
        // the length is an ordinary inline smallint that decodes fine and
        // then overflows `Vec`'s capacity, whose `panic!` this
        // `extern "C" fn` turns into a process abort. Measured at exit 134
        // before the fix.
        //
        // `i64::MAX` is chosen so `try_reserve_exact` reports capacity
        // overflow arithmetically, without attempting a real allocation.
        pycc_rt_exception_clear();
        let view = pycc_rt_buffer_f64_alloc(i64::MAX);
        assert!(view.is_null(), "the refusal must not hand back a view");
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_RUNTIME_ERROR, "{message}");
        assert!(message.contains("cannot be allocated"), "{message}");
        // The deviation is stated in the message rather than in the class,
        // because this runtime has no `MemoryError` tag to raise.
        assert!(message.contains("MemoryError"), "{message}");
        pycc_rt_exception_clear();

        // The other direction: an ordinary length is unaffected by the new
        // fallible reservation, and still zero-fills.
        let view = pycc_rt_buffer_f64_alloc(3);
        assert!(!view.is_null());
        assert_eq!(pycc_rt_exception_active(), 0);
        unsafe {
            assert_eq!(pycc_rt_buffer_len(view), 3);
            assert_eq!(pycc_rt_buffer_f64_get(view, 2), 0.0);
            pycc_rt_buffer_f64_free(view);
        }
    }

    #[test]
    fn buffer_f64_alloc_free_round_trips_through_the_element_helpers() {
        pycc_rt_exception_clear();
        let view = pycc_rt_buffer_f64_alloc(4);
        assert!(!view.is_null());
        assert_eq!(pycc_rt_exception_active(), 0);
        unsafe {
            assert_eq!(pycc_rt_buffer_len(view), 4);
            // The zero-fill deviation from `numpy.ndarray(n)`, asserted
            // before any store.
            for i in 0..4 {
                assert_eq!(pycc_rt_buffer_f64_get(view, i), 0.0);
            }
            for i in 0..4 {
                pycc_rt_buffer_f64_set(view, i, 1.5 + i as f64);
            }
            for i in 0..4 {
                assert_eq!(pycc_rt_buffer_f64_get(view, i), 1.5 + i as f64);
            }
            pycc_rt_buffer_f64_free(view);
        }
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    /// `ndarray(0)` is admitted: a zero-length view whose `len` is `0`, which
    /// every existing helper already handles, and whose free is the same
    /// `Box<[f64]>` round trip.
    #[test]
    fn buffer_f64_alloc_admits_a_zero_length_request() {
        pycc_rt_exception_clear();
        let view = pycc_rt_buffer_f64_alloc(0);
        assert!(!view.is_null());
        unsafe {
            assert_eq!(pycc_rt_buffer_len(view), 0);
            pycc_rt_buffer_f64_free(view);
        }
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    /// `ndarray(-1)` raises `ValueError` through D-173's pending-exception
    /// protocol and returns null, matching CPython's own
    /// `ValueError: negative dimensions are not allowed`.
    #[test]
    fn buffer_f64_alloc_refuses_a_negative_length() {
        pycc_rt_exception_clear();
        let view = pycc_rt_buffer_f64_alloc(-1);
        assert!(view.is_null());
        assert_eq!(pycc_rt_exception_active(), 1);
        assert_eq!(
            pycc_rt_ext_pending_type(),
            i32::from(EXCEPTION_TYPE_VALUE_ERROR)
        );
        pycc_rt_exception_clear();
    }

    /// The null arm of the free is a no-op, which is what makes a generated
    /// epilogue safe over a slot a path never assigned.
    #[test]
    fn buffer_f64_free_is_a_no_op_on_null() {
        pycc_rt_exception_clear();
        unsafe { pycc_rt_buffer_f64_free(core::ptr::null_mut()) };
        assert_eq!(pycc_rt_exception_active(), 0);
    }

    #[test]
    fn int_list_incref_decref_round_trip_does_not_free_early() {
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, tag_smallint(1));
            pycc_rt_int_list_incref(list);
            pycc_rt_int_list_decref(list);
            // still alive after one incref/decref pair -- one decref remains
            assert_eq!(pycc_rt_int_list_get(list, 0), tag_smallint(1));
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn int_list_incref_and_decref_on_a_null_pointer_are_safe_no_ops() {
        // D-014's 100% line/region coverage gate: without this,
        // `pycc_rt_int_list_incref`/`_decref`'s `if list.is_null()` early
        // return is dead code, since none of the tests above ever pass a
        // null pointer. Mirrors `PyStrObj`'s own
        // `incref_and_decref_on_a_null_pointer_are_safe_no_ops` test above.
        unsafe {
            pycc_rt_int_list_incref(std::ptr::null_mut());
            pycc_rt_int_list_decref(std::ptr::null_mut());
        }
    }

    /// Reads every encoded element of `list` (possibly zero, for an empty
    /// slice result) into a `Vec<i64>`, for asserting a whole slice result's
    /// contents in one line rather than one `pycc_rt_int_list_get` call per
    /// expected element.
    unsafe fn collect_list(list: *mut PyIntListObj) -> Vec<i64> {
        unsafe {
            let len = pycc_rt_int_list_len(list);
            (0..len).map(|i| pycc_rt_int_list_get(list, i)).collect()
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_returns_the_in_range_sub_range() {
        // D-118's ordinary path: `[10,20,30,40,50][1:4:1] == [20,30,40]`.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [10, 20, 30, 40, 50] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 1, 4, 1);
            assert_eq!(
                collect_list(sliced),
                vec![tag_smallint(20), tag_smallint(30), tag_smallint(40)]
            );
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_clamps_a_too_high_stop() {
        // D-118's clamp-stop-high branch: `[1,2,3][0:100:1] == [1,2,3]`,
        // matching CPython's own out-of-range-slice-bound clamping.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 0, 100, 1);
            assert_eq!(
                collect_list(sliced),
                vec![tag_smallint(1), tag_smallint(2), tag_smallint(3)]
            );
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_clamps_a_too_high_start() {
        // D-118's clamp-start-high branch: `[1,2,3][100:200:1]` clamps
        // *both* bounds to `len` (3), so `clamped_start == clamped_stop`
        // and the result is empty -- the same outcome as the
        // start-at-or-past-stop test below, but pinning the clamp
        // arithmetic itself (both operands clamp to the same value) rather
        // than an already-in-range `start >= stop` relationship.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 100, 200, 1);
            assert_eq!(collect_list(sliced), Vec::<i64>::new());
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_with_start_at_or_past_stop_is_empty() {
        // D-118's empty-result branch: `[1,2,3][2:1:1] == []` -- `start`
        // and `stop` are both already in range, unlike the clamp-driven
        // empty result above, so the `while i < clamped_stop` loop body
        // never runs for a distinct reason (no clamping involved at all).
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 2, 1, 1);
            assert_eq!(collect_list(sliced), Vec::<i64>::new());
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    /// Part B of #1038 (#1064): drives one rejected `int_list_slice` bound
    /// and returns the pending tag/message, asserting the sentinel is a
    /// valid, empty, non-null list along the way. Shared by the four
    /// rejected-bound tests below, which differ only in their arguments.
    fn slice_rejection(start: i64, stop: i64, step: i64) -> (u8, String) {
        pycc_rt_exception_clear();
        unsafe {
            let list = pycc_rt_int_list_new();
            pycc_rt_int_list_append(list, tag_smallint(1));
            let result = int_list_slice(&*list, start, stop, step);
            let pending = pending_tag_and_message();
            assert!(!result.is_null());
            assert_eq!(collect_list(result), Vec::<i64>::new()); // sentinel
            pycc_rt_int_list_decref(result);
            pycc_rt_int_list_decref(list);
            pycc_rt_exception_clear();
            pending
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_raises_on_a_negative_start() {
        // Part B of #1038 (#1064): was `#[should_panic]`. CPython addresses
        // `lst[-1:]` from the end rather than rejecting it; pycc's v0.2
        // slice still does not, but the gap is now a catchable `ValueError`
        // rather than a process abort. Conformance is tracked as #1070.
        let (tag, message) = slice_rejection(-1, 1, 1);
        assert_eq!(tag, EXCEPTION_TYPE_VALUE_ERROR);
        assert_eq!(message, "slice start must be non-negative");
        assert!(!message.contains("pycc_rt: "), "{message}");
    }

    #[test]
    fn pycc_rt_int_list_slice_raises_on_a_negative_stop() {
        let (tag, message) = slice_rejection(0, -1, 1);
        assert_eq!(tag, EXCEPTION_TYPE_VALUE_ERROR);
        assert_eq!(message, "slice stop must be non-negative");
    }

    #[test]
    fn pycc_rt_int_list_slice_raises_on_a_zero_step() {
        let (tag, message) = slice_rejection(0, 1, 0);
        assert_eq!(tag, EXCEPTION_TYPE_VALUE_ERROR);
        assert_eq!(message, "slice step must be positive");
    }

    #[test]
    fn pycc_rt_int_list_slice_raises_on_a_negative_step() {
        let (tag, message) = slice_rejection(0, 1, -1);
        assert_eq!(tag, EXCEPTION_TYPE_VALUE_ERROR);
        assert_eq!(message, "slice step must be positive");
    }

    #[test]
    fn pycc_rt_int_list_slice_with_a_step_greater_than_one_skips_elements() {
        // `[0,1,2,3,4,5][0:6:2] == [0,2,4]`.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [0, 1, 2, 3, 4, 5] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 0, 6, 2);
            assert_eq!(
                collect_list(sliced),
                vec![tag_smallint(0), tag_smallint(2), tag_smallint(4)]
            );
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    #[test]
    fn pycc_rt_int_list_slice_returns_a_genuinely_independent_list() {
        // D-107's leak-only policy still requires the slice result to be a
        // *new* allocation, not an alias of `list`'s own backing storage --
        // appending to the original after slicing must not retroactively
        // change the already-returned slice, and vice versa.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            let sliced = pycc_rt_int_list_slice(list, 0, 3, 1);
            pycc_rt_int_list_append(list, tag_smallint(99));
            assert_eq!(
                collect_list(list),
                vec![
                    tag_smallint(1),
                    tag_smallint(2),
                    tag_smallint(3),
                    tag_smallint(99)
                ]
            );
            assert_eq!(
                collect_list(sliced),
                vec![tag_smallint(1), tag_smallint(2), tag_smallint(3)]
            );
            pycc_rt_int_list_decref(list);
            pycc_rt_int_list_decref(sliced);
        }
    }

    #[test]
    fn pycc_rt_int_list_pop_removes_and_returns_the_last_element() {
        // Python's `list.pop()`: `[1,2,3].pop() == 3`, and the list shrinks
        // by one, leaving `[1,2]` -- the removed element is always the
        // *last* one, D-119.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            assert_eq!(pycc_rt_int_list_pop(list), tag_smallint(3));
            assert_eq!(pycc_rt_int_list_len(list), 2);
            assert_eq!(collect_list(list), vec![tag_smallint(1), tag_smallint(2)]);
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn pycc_rt_int_list_pop_repeated_calls_keep_removing_the_new_last_element() {
        // Two `.pop()`s in a row must each observe the *previous* pop's
        // effect, not some stale snapshot -- exercises the same repeated-
        // mutation-on-the-same-object concern this task's own brief flags
        // for `xs = [xs.pop(), xs.pop()]`-shaped codegen, at the `pycc_rt`
        // layer directly.
        unsafe {
            let list = pycc_rt_int_list_new();
            for v in [1, 2, 3] {
                pycc_rt_int_list_append(list, tag_smallint(v));
            }
            assert_eq!(pycc_rt_int_list_pop(list), tag_smallint(3));
            assert_eq!(pycc_rt_int_list_pop(list), tag_smallint(2));
            assert_eq!(pycc_rt_int_list_len(list), 1);
            assert_eq!(collect_list(list), vec![tag_smallint(1)]);
            pycc_rt_int_list_decref(list);
        }
    }

    #[test]
    fn pycc_rt_int_list_pop_on_an_empty_list_raises_index_error() {
        // Part B of #1038 (#1064): was `#[should_panic]`. CPython raises a
        // catchable `IndexError` here and now so does pycc; the sentinel is
        // `tag_smallint(0)`, a *valid* D-141 encoded word (raw `0` is not --
        // `classify_encoded_int` rejects it), so a caller may decode it
        // before its own pending-exception check.
        pycc_rt_exception_clear();
        unsafe {
            let list = pycc_rt_int_list_new();
            let result = int_list_pop(&*list);
            let (tag, message) = pending_tag_and_message();
            assert_eq!(tag, EXCEPTION_TYPE_INDEX_ERROR);
            assert_eq!(message, "pop from empty list");
            assert!(!message.contains("pycc_rt: "), "{message}");
            assert_eq!(result, tag_smallint(0)); // sentinel value
            // The (empty) payload is restored before the raise, so the list
            // is still usable afterwards.
            pycc_rt_int_list_append(list, tag_smallint(7));
            assert_eq!(collect_list(list), vec![tag_smallint(7)]);
            pycc_rt_int_list_decref(list);
        }
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_dict_set_then_get_round_trips_the_value() {
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"a");
            pycc_rt_dict_set(dict, key, tag_smallint(42));
            assert_eq!(pycc_rt_dict_get(dict, key), tag_smallint(42));
            assert_eq!(pycc_rt_dict_len(dict), 1);
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn dict_update_get_and_default_preserve_bool_identity_markers() {
        unsafe {
            let dict = pycc_rt_dict_new();
            let present = new_pystr(b"present");
            let missing = new_pystr(b"missing");
            pycc_rt_dict_set(dict, present, BOOL_TRUE_MARKER);
            assert_eq!(pycc_rt_dict_get(dict, present), BOOL_TRUE_MARKER);
            assert_eq!(
                pycc_rt_dict_get_or_default(dict, present, BOOL_FALSE_MARKER),
                BOOL_TRUE_MARKER
            );
            assert_eq!(
                pycc_rt_dict_get_or_default(dict, missing, BOOL_FALSE_MARKER),
                BOOL_FALSE_MARKER
            );
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn pycc_rt_dict_set_on_an_existing_key_updates_in_place_without_growing_len() {
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"a");
            pycc_rt_dict_set(dict, key, tag_smallint(1));
            pycc_rt_dict_set(dict, key, tag_smallint(2));
            assert_eq!(pycc_rt_dict_get(dict, key), tag_smallint(2));
            assert_eq!(pycc_rt_dict_len(dict), 1);
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn pycc_rt_dict_preserves_insertion_order_across_key_at() {
        unsafe {
            let dict = pycc_rt_dict_new();
            let a = new_pystr(b"a");
            let b = new_pystr(b"b");
            pycc_rt_dict_set(dict, b, tag_smallint(2));
            pycc_rt_dict_set(dict, a, tag_smallint(1));
            // "b" was inserted first, so it stays at index 0 even though "a"
            // sorts first lexicographically.
            assert_eq!(pycc_rt_str_cmp(pycc_rt_dict_key_at(dict, 0), b), 0);
            assert_eq!(pycc_rt_str_cmp(pycc_rt_dict_key_at(dict, 1), a), 0);
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    #[should_panic(expected = "pycc_rt: dict_key_at index out of range")]
    fn pycc_rt_dict_key_at_out_of_range_panics_honestly() {
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"a");
            pycc_rt_dict_set(dict, key, tag_smallint(1));
            // Calls the private `dict_key_at`, not the public
            // `pycc_rt_dict_key_at` wrapper -- the wrapper is a plain
            // `extern "C" fn`, so a panic crossing its boundary aborts the
            // whole test binary (`SIGABRT`) instead of unwinding into
            // `#[should_panic]`'s own catch (same convention as
            // `int_list_get_out_of_range_panics_honestly`/
            // `pycc_rt_dict_get_on_a_missing_key_panics` just below).
            dict_key_at(&*dict, 1);
        }
    }

    #[test]
    fn pycc_rt_dict_get_on_a_missing_key_sets_exception_flag() {
        pycc_rt_exception_clear();
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"missing");
            // Calls the private `dict_get` directly, matching the
            // private-logic/public-wrapper convention used throughout
            // this file's test module.
            let result = dict_get(&*dict, key);
            assert_eq!(pycc_rt_exception_active(), 1);
            assert_eq!(untag_smallint(result), 0); // sentinel value
            pycc_rt_dict_decref(dict);
        }
        pycc_rt_exception_clear();
    }

    #[test]
    fn pycc_rt_dict_get_or_default_on_a_present_key_returns_the_stored_value() {
        // Python's `dict.get(key, default)` on a present key: the stored
        // value wins, `default` is ignored -- the two-argument form's
        // "found" path, D-119.
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"a");
            pycc_rt_dict_set(dict, key, tag_smallint(42));
            assert_eq!(
                pycc_rt_dict_get_or_default(dict, key, tag_smallint(-1)),
                tag_smallint(42)
            );
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn pycc_rt_dict_get_or_default_on_a_missing_key_returns_the_default_without_panicking() {
        // Unlike `pycc_rt_dict_get`, a missing key never panics here -- it
        // returns `default` instead, the entire point of the two-argument
        // form, D-119.
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"a");
            pycc_rt_dict_set(dict, key, tag_smallint(42));
            let missing = new_pystr(b"missing");
            assert_eq!(
                pycc_rt_dict_get_or_default(dict, missing, tag_smallint(-1)),
                tag_smallint(-1)
            );
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn pycc_rt_dict_get_or_default_on_an_empty_dict_returns_the_default() {
        // No entries at all -- the degenerate case of the missing-key path
        // above, pinned separately since `dict_get_or_default`'s own linear
        // scan over an empty `Vec` is a distinct code path worth its own
        // executing test.
        unsafe {
            let dict = pycc_rt_dict_new();
            let key = new_pystr(b"z");
            assert_eq!(
                pycc_rt_dict_get_or_default(dict, key, tag_smallint(7)),
                tag_smallint(7)
            );
            pycc_rt_dict_decref(dict);
        }
    }

    #[test]
    fn pycc_rt_dict_incref_then_decref_frees_without_leaking() {
        unsafe {
            let dict = pycc_rt_dict_new();
            pycc_rt_dict_incref(dict);
            pycc_rt_dict_decref(dict);
            pycc_rt_dict_decref(dict); // rc reaches 0, frees
        }
    }

    #[test]
    fn pycc_rt_dict_incref_and_decref_on_a_null_pointer_are_safe_no_ops() {
        // D-014's 100% line/region coverage gate: without this,
        // `pycc_rt_dict_incref`/`_decref`'s `if dict.is_null()` early
        // return is dead code, since none of the tests above ever pass a
        // null pointer. Mirrors `PyIntListObj`'s own
        // `int_list_incref_and_decref_on_a_null_pointer_are_safe_no_ops`
        // test above.
        unsafe {
            pycc_rt_dict_incref(std::ptr::null_mut());
            pycc_rt_dict_decref(std::ptr::null_mut());
        }
    }

    /// #1054, coverage-bearing companion to
    /// `crates/pycc_rt/tests/str_live_objects.rs`. `STR_LIVE` is a single
    /// process-wide atomic and dozens of tests in *this* binary construct
    /// and free `PyStrObj` values concurrently, so no absolute reading of
    /// the counter is assertable here -- the construct/free pairing is
    /// proved in that integration test, which owns its own process. What
    /// is true under concurrency, and is the property asserted here, is
    /// that the net count never goes negative: a decrement only ever
    /// retires an object some increment already counted, so an unbalanced
    /// free anywhere in this binary's `str` tests would drive the reading
    /// below zero.
    #[test]
    fn str_live_objects_never_reads_back_a_negative_count() {
        let live = pycc_rt_str_live_objects();
        assert!(
            live >= 0,
            "the live-str count must never go negative, read {live}"
        );
    }
}
