//! The two ways the runtime gives up -- a Rust panic and a failed
//! allocation -- reported as one line on stderr before the process aborts,
//! without the standard library's backtrace printer.
//!
//! A runtime panic never unwinds into compiled code or the interpreter: an
//! `extern "C"` entry point aborts when a panic would leave it. The standard
//! library's handlers print the message and then, before aborting, can walk
//! and symbolize the stack (`RUST_BACKTRACE`, and on every allocation
//! failure), and that symbolizer -- `gimli`, `addr2line`, `miniz_oxide`,
//! `rustc_demangle` -- is about 200 KB of an extension module's code, a
//! fifth of pystache's, with a dozen more C library symbols to bind at
//! `dlopen`. The ELF link of an `--ext` module therefore routes the two
//! handlers here (`src/ext_build/link.rs`'s `quiet_abort_args`: `--wrap`
//! each, `--defsym` the wrapper to these functions), and the linker drops
//! the symbolizer. Everything else is unchanged: the message, its
//! location, and the abort.

use std::io::Write;

/// The panic handler of an ELF `--ext` module: `pycc runtime panicked at
/// <file>:<line>:<column>:` and the message, on stderr, then
/// [`std::process::abort`]. Its signature is the standard library's
/// `rust_begin_unwind`, which the link wraps to it.
#[unsafe(no_mangle)]
pub fn pycc_rt_panic_abort(info: &core::panic::PanicInfo<'_>) -> ! {
    let mut err = std::io::stderr();
    let _ = match info.location() {
        Some(at) => writeln!(err, "pycc runtime panicked at {at}:\n{}", info.message()),
        None => writeln!(err, "pycc runtime panicked:\n{}", info.message()),
    };
    std::process::abort()
}

/// The allocation-failure handler of an ELF `--ext` module: the standard
/// library's `memory allocation of N bytes failed` on stderr, then
/// [`std::process::abort`]. Its signature is the standard library's
/// `__rust_alloc_error_handler`, which the link wraps to it.
#[unsafe(no_mangle)]
pub fn pycc_rt_alloc_error_abort(size: usize, _align: usize) -> ! {
    let _ = writeln!(std::io::stderr(), "memory allocation of {size} bytes failed");
    std::process::abort()
}
