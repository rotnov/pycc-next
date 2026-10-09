//! `print`'s stdout flush (#1340). New code lands here rather than in the
//! oversized `lib.rs` (AGENTS.md's "Keep source files decomposable"; the
//! tracker for `lib.rs` is #550); the older write primitives
//! (`pycc_rt_print_write_str`, `_space`, `_newline`, `_none`) stay in
//! `lib.rs` for now.
//!
//! Every `print` write goes through Rust's own line-buffered stdout, which is
//! separate from CPython's `sys.stdout`. [`pycc_rt_print_flush`] (#1340)
//! empties it before compiled code hands control to CPython in the middle
//! of a `print`, so text already written is not held back behind -- or lost
//! with -- whatever CPython does next.

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
    let _ = std::io::Write::flush(&mut std::io::stdout());
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
