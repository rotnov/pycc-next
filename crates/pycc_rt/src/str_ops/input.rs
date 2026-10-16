//! `input()` and the `sys.stdin`/`sys.stdout` calls compiled code makes.
//!
//! Every read and write goes through `crate::io`, which shares CPython's
//! `sys.stdin`/`sys.stdout` when an interpreter is present.

use super::{bytes_of, char_len, new_str, str_list_of, text_of};
use crate::exception::{EXCEPTION_TYPE_EOF_ERROR, EXCEPTION_TYPE_STOP_ITERATION};
use crate::list::PyListObj;
use crate::{PyStrObj, pycc_rt_str_decref, raise_builtin, tag_smallint};

fn input_line() -> *mut PyStrObj {
    // CPython's `input()` flushes stdout before it reads.
    crate::io::flush();
    let Some(line) = crate::io::read_line() else {
        return new_str("");
    };
    let text = text_of(line);
    if text.is_empty() {
        unsafe { pycc_rt_str_decref(line) };
        raise_builtin(EXCEPTION_TYPE_EOF_ERROR, "EOFError", "EOF when reading a line");
        return crate::str_index::empty_str();
    }
    let Some(stripped) = text.strip_suffix('\n') else {
        return line;
    };
    let result = new_str(stripped);
    unsafe { pycc_rt_str_decref(line) };
    result
}

/// `input()`: one line of stdin without its trailing newline.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_input() -> *mut PyStrObj {
    input_line()
}

/// `input(prompt)`: writes `prompt` (no newline) first.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_input_prompt(prompt: *mut PyStrObj) -> *mut PyStrObj {
    crate::io::out(bytes_of(prompt));
    input_line()
}

/// `sys.stdin.readline()`: one line including its newline, `''` at EOF.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_readline() -> *mut PyStrObj {
    crate::io::read_line().unwrap_or_else(|| new_str(""))
}

/// `next(sys.stdin)`: one line including its newline; `StopIteration` at
/// EOF, as the exhausted file iterator raises.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_next() -> *mut PyStrObj {
    let line = pycc_rt_stdin_readline();
    if text_of(line).is_empty() {
        unsafe { pycc_rt_str_decref(line) };
        raise_builtin(EXCEPTION_TYPE_STOP_ITERATION, "StopIteration", "");
        return crate::str_index::empty_str();
    }
    line
}

/// `sys.stdin.read()`: the rest of stdin.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_read() -> *mut PyStrObj {
    crate::io::read_all().unwrap_or_else(|| new_str(""))
}

/// `sys.stdin.readlines()`: the remaining lines, each with its newline.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdin_readlines() -> *mut PyListObj {
    let rest = pycc_rt_stdin_read();
    let text = text_of(rest);
    let list = str_list_of(text.split_inclusive('\n'));
    unsafe { pycc_rt_str_decref(rest) };
    list
}

/// `sys.stdout.write(s)`: writes `s`, returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_stdout_write(s: *mut PyStrObj) -> i64 {
    let text = text_of(s);
    crate::io::out_str(text);
    tag_smallint(char_len(text) as i64)
}

/// `sys.setrecursionlimit(n)`: compiled code recurses on the native stack,
/// so there is no limit to set.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_setrecursionlimit(_limit: i64) {}

