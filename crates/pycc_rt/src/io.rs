//! Standard input and output for compiled code.
//!
//! A pure native program owns the process's stdin/stdout: `print` writes
//! through Rust's line-buffered stdout and `input()` reads Rust's buffered
//! stdin.
//!
//! When the program runs with a CPython interpreter -- an `--ext` module in
//! its host, or a native build that embeds CPython for a foreign import or
//! non-strict fallback code -- CPython's `sys.stdin`/`sys.stdout` hold
//! buffers of their own, and two independent buffers over one file
//! descriptor reorder output and steal each other's input lines. The C
//! shim therefore installs hooks ([`pycc_rt_io_install`]) before any
//! compiled code runs, and every native read and write then goes through
//! `sys.stdin`/`sys.stdout` themselves: one buffer, CPython's, whatever
//! either side does (including a program that rebinds `sys.stdout`).

use super::*;
use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicPtr, Ordering};

/// Writes `len` UTF-8 bytes to `sys.stdout`; nonzero when it raised (the
/// exception is then pending as a pycc exception).
type WriteHook = unsafe extern "C" fn(*const u8, usize) -> i32;
/// `sys.stdin.readline()` (`all == 0`) or `sys.stdin.read()` as a new
/// `str`, null when it raised (pending as a pycc exception).
type ReadHook = unsafe extern "C" fn(i32) -> *mut PyStrObj;
/// `sys.stdout.flush()`; nonzero when it raised.
type FlushHook = unsafe extern "C" fn() -> i32;

static WRITE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static READ: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static FLUSH: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Routes all native standard I/O through CPython's `sys` streams (called
/// by the C shim when its module is executed).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_io_install(write: WriteHook, read: ReadHook, flush: FlushHook) {
    WRITE.store(write as *mut (), Ordering::Relaxed);
    READ.store(read as *mut (), Ordering::Relaxed);
    FLUSH.store(flush as *mut (), Ordering::Relaxed);
}

fn write_hook() -> Option<WriteHook> {
    let p = WRITE.load(Ordering::Relaxed);
    (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut (), WriteHook>(p) })
}

fn read_hook() -> Option<ReadHook> {
    let p = READ.load(Ordering::Relaxed);
    (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut (), ReadHook>(p) })
}

fn flush_hook() -> Option<FlushHook> {
    let p = FLUSH.load(Ordering::Relaxed);
    (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut (), FlushHook>(p) })
}

/// Writes `bytes` to standard output.
pub(crate) fn out(bytes: &[u8]) {
    match write_hook() {
        Some(write) => {
            unsafe { write(bytes.as_ptr(), bytes.len()) };
        }
        None => {
            // On Windows too the bytes are written as they are: a native
            // program's `\n` stays `\n`, where CPython's text-mode stdout
            // writes `\r\n` (docs/DEVIATIONS.md).
            let _ = std::io::stdout().lock().write_all(bytes);
        }
    }
}

/// What CPython's universal-newline stdin reads for the bytes `buf`: on
/// Windows a `\r\n` line ending is read as `\n`, as is a lone `\r`. Pure, so
/// every host tests it; applied only on Windows.
fn from_crlf(buf: Vec<u8>) -> Vec<u8> {
    if !buf.contains(&b'\r') {
        return buf;
    }
    let mut out = Vec::with_capacity(buf.len());
    let mut i = 0;
    while i < buf.len() {
        match buf[i] {
            b'\r' => {
                out.push(b'\n');
                if buf.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    out
}

/// Stdin bytes as text, with Windows line endings translated.
fn native_text(buf: Vec<u8>) -> *mut PyStrObj {
    let buf = if cfg!(windows) { from_crlf(buf) } else { buf };
    new_pystr(String::from_utf8_lossy(&buf).as_bytes())
}


/// Writes `text` to standard output.
pub(crate) fn out_str(text: &str) {
    out(text.as_bytes());
}

/// Flushes standard output.
pub(crate) fn flush() {
    match flush_hook() {
        Some(flush) => {
            unsafe { flush() };
        }
        None => {
            let _ = std::io::stdout().flush();
        }
    }
}

/// One line of stdin including its newline, `Some("")` at end of input,
/// `None` when the read raised (the exception is pending).
pub(crate) fn read_line() -> Option<*mut PyStrObj> {
    match read_hook() {
        Some(read) => {
            let s = unsafe { read(0) };
            (!s.is_null()).then_some(s)
        }
        None => {
            let _ = std::io::stdout().flush();
            let mut buf = Vec::new();
            let _ = std::io::stdin().lock().read_until(b'\n', &mut buf);
            Some(native_text(buf))
        }
    }
}

/// The rest of stdin, `None` when the read raised.
pub(crate) fn read_all() -> Option<*mut PyStrObj> {
    match read_hook() {
        Some(read) => {
            let s = unsafe { read(1) };
            (!s.is_null()).then_some(s)
        }
        None => {
            let _ = std::io::stdout().flush();
            let mut buf = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut buf);
            Some(native_text(buf))
        }
    }
}

#[cfg(test)]
mod newline_tests {
    use super::from_crlf;

    #[test]
    fn reads_translate_crlf_and_lone_cr_to_newline() {
        assert_eq!(from_crlf(b"abc\n".to_vec()), b"abc\n");
        assert_eq!(from_crlf(b"a\r\nb\rc\r".to_vec()), b"a\nb\nc\n");
    }
}
