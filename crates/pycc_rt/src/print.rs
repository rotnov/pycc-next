//! `print`'s write primitives (`pycc_rt_print_write_str`, `_space`,
//! `_newline`, `_none`) and its stdout flush (#1340).
//!
//! Every `print` write goes through Rust's own line-buffered stdout, which is
//! separate from CPython's `sys.stdout`. [`pycc_rt_print_flush`] (#1340)
//! empties it before compiled code hands control to CPython in the middle
//! of a `print`, so text already written is not held back behind -- or lost
//! with -- whatever CPython does next.

use crate::{PyStrObj, io};

/// Flushes compiled code's stdout (#1340). `print` calls it before it asks
/// CPython for a `str()` of an object argument: the text written so far on
/// that line (earlier arguments and the separator) sits in Rust's line
/// buffer, and CPython's `__str__` may raise -- ending the `print` without
/// its newline, so nothing would flush that partial line before the host
/// exits -- or write to CPython's own stdout. Flushing first keeps both
/// cases in CPython's order. A flush of an empty buffer makes no write. A
/// flush error is ignored, exactly like `print!`'s own writes, which have
/// nowhere to report one either.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_flush() {
    crate::io::flush();
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
    io::out(unsafe { &*s }.bytes());
}

/// Prints a single space with no newline (Task 10) -- `print`'s separator
/// between arguments, matching CPython's default `sep=" "`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_space() {
    io::out(b" ");
}

/// Prints `print`'s single trailing newline (Task 10), matching CPython's
/// default `end="\n"`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_print_newline() {
    io::out(b"\n");
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
    io::out(b"None");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pycc_rt_print_newline, pycc_rt_print_space};

    #[test]
    fn print_flush_does_not_panic_on_an_empty_or_partial_line() {
        pycc_rt_print_flush();
        pycc_rt_print_space();
        pycc_rt_print_flush();
        pycc_rt_print_newline();
    }
}
