//! Traceback records of compiled frames (v0.8 accept 3).
//!
//! An exception collects one [`TbRecord`] for each compiled frame it is
//! raised in or propagates into (`pycc_codegen::frames` emits the
//! [`pycc_rt_tb_here`] calls on the exception edges), innermost first. A
//! native program prints them as CPython prints a traceback
//! ([`render_traceback`]); an `--ext` module's shim turns them into
//! CPython traceback entries whenever the exception becomes a CPython
//! object (`frames.c`).

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_void};

/// One compiled function's frame description, emitted by codegen
/// (`pycc_codegen::frames::info_type`). The first five fields belong to an
/// `--ext` module's shim, which fills them at import.
#[repr(C)]
pub struct FnInfo {
    pub code: *mut c_void,
    pub func: *mut c_void,
    pub globals: *mut c_void,
    pub builtins: *mut c_void,
    pub ip0: *mut c_void,
    pub name: *const c_char,
    pub qualname: *const c_char,
    pub filename: *const c_char,
    /// `npos` entries of `[line, end_line, col, end_col]`.
    pub positions: *const [u32; 4],
    pub npos: u32,
    pub firstlineno: u32,
    pub flags: u32,
    pub file: u32,
    /// A native program's source block per position (null, or `npos`
    /// pointers, each null or NUL-terminated).
    pub texts: *const *const c_char,
}

impl FnInfo {
    fn text(ptr: *const c_char) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: codegen emits NUL-terminated constants.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn text_at(&self, pos: u32) -> Option<String> {
        if self.texts.is_null() || pos >= self.npos {
            return None;
        }
        // SAFETY: `texts` holds `npos` entries.
        let text = unsafe { *self.texts.add(pos as usize) };
        (!text.is_null()).then(|| Self::text(text))
    }

    fn position(&self, pos: u32) -> Option<[u32; 4]> {
        // SAFETY: `positions` holds `npos` entries.
        (pos < self.npos).then(|| unsafe { *self.positions.add(pos as usize) })
    }
}

/// A traceback entry of a compiled frame: the function, the position it
/// was at, and (in an `--ext` module, for a frame that had a CPython frame
/// object) that object, owned.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TbRecord {
    pub info: *const FnInfo,
    pub pos: u32,
    pub frame: *mut c_void,
    /// Which call of the function (an address on its stack): an exception
    /// observed twice at one position of one call -- a `raise` in a
    /// handler, then the handler's own exit -- is one entry.
    pub activation: *const c_void,
}

/// An exception's records, innermost first, and how many of them the
/// `--ext` shim has already added to its CPython object's traceback.
#[derive(Default)]
pub(crate) struct TbRecords {
    records: RefCell<Vec<TbRecord>>,
    synced: Cell<usize>,
}

impl TbRecords {
    /// Whether the innermost record is `info` at `pos` in `activation`.
    pub(crate) fn is_last(&self, info: *const FnInfo, pos: u32, activation: *const c_void) -> bool {
        self.records.borrow().last().is_some_and(|last| {
            last.info == info && last.pos == pos && last.activation == activation
        })
    }

    pub(crate) fn push(&self, record: TbRecord) {
        self.records.borrow_mut().push(record);
    }

    pub(crate) fn snapshot(&self) -> Vec<TbRecord> {
        self.records.borrow().clone()
    }

    /// Releases the frame objects the records own.
    pub(crate) fn release(&self) {
        self.synced.set(0);
        for record in self.records.borrow_mut().drain(..) {
            if !record.frame.is_null() {
                crate::rc::release_word(b'o', record.frame as i64);
            }
        }
    }
}

/// Records that the pending exception reached position `pos` of the
/// compiled function `info`: a raise there, or an exception propagating
/// out of a call there. A no-op with nothing pending.
///
/// # Safety
///
/// `info` must point to a codegen-emitted [`FnInfo`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tb_here(info: *const FnInfo, pos: u32, activation: *const c_void) {
    unsafe { pycc_rt_tb_here_frame(info, pos, std::ptr::null_mut(), activation) }
}

/// Whether the pending exception's innermost record already is `info` at
/// `pos` in `activation` (the shim asks before making a frame object).
///
/// # Safety
///
/// `info` must point to a codegen-emitted [`FnInfo`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tb_seen(
    info: *const FnInfo,
    pos: u32,
    activation: *const c_void,
) -> i32 {
    let exc = crate::exception::pycc_rt_exception_value();
    // SAFETY: a pending exception is live.
    i32::from(!exc.is_null() && unsafe { &*exc }.tb.is_last(info, pos, activation))
}

/// [`pycc_rt_tb_here`] for a frame with a CPython frame object `frame`,
/// whose reference passes to the record (released with the exception).
///
/// # Safety
///
/// As [`pycc_rt_tb_here`]; `frame` is null or an owned `PyObject *`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_tb_here_frame(
    info: *const FnInfo,
    pos: u32,
    frame: *mut c_void,
    activation: *const c_void,
) {
    let exc = crate::exception::pycc_rt_exception_value();
    // SAFETY: a pending exception is live.
    if exc.is_null() || unsafe { &*exc }.tb.is_last(info, pos, activation) {
        if !frame.is_null() {
            crate::rc::release_word(b'o', frame as i64);
        }
        return;
    }
    // SAFETY: the pending exception is live.
    unsafe { &*exc }.tb.push(TbRecord {
        info,
        pos,
        frame,
        activation,
    });
}

/// The records of `exc` (a live exception), innermost first: `*len` of
/// them, the first `*synced` already added to its CPython traceback. Valid
/// until the exception records another or is freed.
///
/// # Safety
///
/// `exc` must be a live exception object; `len` and `synced` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_tb(
    exc: *mut crate::exception::PyExceptionObj,
    len: *mut usize,
    synced: *mut usize,
) -> *const TbRecord {
    // SAFETY: the caller's contract.
    let tb = unsafe { &(*exc).tb };
    let records = tb.records.borrow();
    unsafe {
        *len = records.len();
        *synced = tb.synced.get();
    }
    records.as_ptr()
}

/// Marks the first `n` records of `exc` as added to its CPython traceback.
///
/// # Safety
///
/// `exc` must be a live exception object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_tb_synced(
    exc: *mut crate::exception::PyExceptionObj,
    n: usize,
) {
    unsafe { (*exc).tb.synced.set(n) };
}

/// The traceback lines a native program prints for `records` (innermost
/// first): CPython's `Traceback (most recent call last):` block without its
/// header, one `File`/`line`/`in` line per frame, outermost first, each
/// followed by the source block codegen rendered for that position (the
/// statement's lines and carets) when the file is readable now.
pub(crate) fn render_traceback(records: &[TbRecord]) -> String {
    let mut out = String::new();
    let mut sources: std::collections::HashMap<String, Option<Vec<String>>> = Default::default();
    for record in records.iter().rev() {
        // SAFETY: records point to codegen-emitted infos, which live as
        // long as the program.
        let info = unsafe { &*record.info };
        let filename = FnInfo::text(info.filename);
        let name = FnInfo::text(info.name);
        match info.position(record.pos) {
            Some([line, ..]) => {
                out.push_str(&format!("  File \"{filename}\", line {line}, in {name}\n"));
            }
            None => out.push_str(&format!("  File \"{filename}\", in {name}\n")),
        }
        let lines = sources
            .entry(filename.clone())
            .or_insert_with(|| source_lines(&filename));
        if let Some(lines) = lines
            && let Some(text) = info.text_at(record.pos)
        {
            out.push_str(&expand(&text, lines));
        }
    }
    out
}

/// The marker opening a template line that stands for a line of source
/// (`src/traceback_blocks.rs`' `template`).
const SOURCE_LINE: char = '\u{1}';

/// The lines of `path` as CPython's `linecache` reads them when the
/// traceback prints (universal newlines, undecodable bytes replaced), or
/// `None` when it cannot be read: then no source line prints, as in
/// CPython.
fn source_lines(path: &str) -> Option<Vec<String>> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n");
    Some(text.split('\n').map(str::to_string).collect())
}

/// A position's block template (no source text: each source line is
/// `\u{1}<line>,<column>,<trimmed>`) with its source lines read from
/// `lines`. A line the file no longer has prints nothing.
fn expand(template: &str, lines: &[String]) -> String {
    let mut out = String::new();
    for text in template.split_inclusive('\n') {
        let Some(reference) = text.strip_prefix(SOURCE_LINE) else {
            out.push_str(text);
            continue;
        };
        let mut fields = reference.trim_end().split(',').map(|f| f.parse::<usize>().ok());
        let (Some(Some(number)), Some(Some(column)), Some(Some(trimmed))) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some(line) = number.checked_sub(1).and_then(|at| lines.get(at)) else {
            continue;
        };
        let rest = line.get(column..).unwrap_or("");
        out.push_str("    ");
        out.push_str(if trimmed == 1 { rest.trim_end() } else { rest });
        out.push('\n');
    }
    out
}
