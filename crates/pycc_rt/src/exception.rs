//! Runtime exception objects and per-thread propagation state (D-173).
//!
//! Generated code uses explicit state checks rather than platform unwinding.
//! The state is thread-local so independent Rust test threads cannot race and
//! a future generated thread cannot overwrite another thread's pending
//! exception. Current generated programs remain single-threaded.

use super::{PyStrObj, PyStrPayload};
use std::cell::Cell;
use std::collections::HashSet;

pub const EXCEPTION_TYPE_EXCEPTION: u8 = 0;
pub const EXCEPTION_TYPE_VALUE_ERROR: u8 = 1;
pub const EXCEPTION_TYPE_TYPE_ERROR: u8 = 2;
pub const EXCEPTION_TYPE_KEY_ERROR: u8 = 3;
pub const EXCEPTION_TYPE_INDEX_ERROR: u8 = 4;
pub const EXCEPTION_TYPE_ZERO_DIV_ERROR: u8 = 5;
pub const EXCEPTION_TYPE_RUNTIME_ERROR: u8 = 6;
/// Part A of #1038 (#1063): `OverflowError`'s tag. Every other builtin tag
/// past the flat seven (7..=24, 26..=27 for
/// `ImportError`/`ModuleNotFoundError`, #1292, and 28 for `AssertionError`,
/// #1369) belongs to a class this
/// crate never raises by name, so it declares no constants for them;
/// `AttributeError` (29, #1388) is raised by name and has its own constant
/// below. This
/// crate has no `[dependencies]` and cannot see
/// `pycc_hir::BUILTIN_EXCEPTION_CLASSES`, so the literal is hand-copied and
/// pinned against the array by `ext_bridge`'s
/// `exception_type_tags_match_the_c_shims_hardcoded_switch` and by
/// `src/ext_build_tests/toolchain.rs`.
pub const EXCEPTION_TYPE_OVERFLOW_ERROR: u8 = 25;
/// #1388: `AttributeError`'s tag, raised by
/// `instance::pycc_rt_instance_get_slot_checked` when a slot is read before
/// its first assignment. Hand-copied and pinned exactly like
/// [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_ATTRIBUTE_ERROR: u8 = 29;
/// `EOFError`'s tag, raised by `input()` at end of input. Hand-copied and
/// pinned exactly like [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_EOF_ERROR: u8 = 30;
/// `NameError`'s tag, raised by [`pycc_rt_raise_name_error`]. Hand-copied and
/// pinned exactly like [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_NAME_ERROR: u8 = 31;
/// `MemoryError`'s tag (an allocation CPython refuses), pinned like
/// [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_MEMORY_ERROR: u8 = 32;
/// `StopIteration`'s tag: `next()` of an exhausted generator. Pinned like
/// [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_STOP_ITERATION: u8 = 33;
/// `GeneratorExit`'s tag: raised at the `yield` of a generator being
/// closed. Like [`EXCEPTION_TYPE_FOREIGN_BASE`] it is not an `Exception`, so
/// `except Exception:` does not catch it. Pinned like
/// [`EXCEPTION_TYPE_OVERFLOW_ERROR`] above.
pub const EXCEPTION_TYPE_GENERATOR_EXIT: u8 = 34;
/// #1316: the reserved tag the `ext` C shim's foreign-operation bridge gives
/// a CPython exception that is not an `Exception` at all -- `SystemExit`,
/// `KeyboardInterrupt`, `GeneratorExit`, a non-`Exception`
/// `BaseExceptionGroup`. It is the one tag above every user class's
/// (`pycc_hir::FOREIGN_BASE_EXCEPTION_TYPE_TAG`), and it is the only tag
/// `Exception`'s catch-all does not match: [`pycc_rt_exception_type_matches`]
/// and [`pycc_rt_exception_group_partition`] exclude it, so `except
/// Exception:` lets it through exactly as CPython does, while a bare
/// `except:` (which never calls the matcher) and `finally` still run.
pub const EXCEPTION_TYPE_FOREIGN_BASE: u8 = u8::MAX;

/// Whether a handler for `handler_tag` catches an exception whose own tag is
/// `obj_tag`: an exact match, or `Exception`'s catch-all for anything but a
/// bridged non-`Exception` [`EXCEPTION_TYPE_FOREIGN_BASE`].
fn tag_matches(obj_tag: u8, handler_tag: u8) -> bool {
    obj_tag == handler_tag
        || (handler_tag == EXCEPTION_TYPE_EXCEPTION
            && obj_tag != EXCEPTION_TYPE_FOREIGN_BASE
            && obj_tag != EXCEPTION_TYPE_GENERATOR_EXIT)
}

/// Heap-allocated builtin exception object. Exception lifetime management is
/// intentionally leak-only in this first implementation: clearing a pending
/// exception drops the runtime reference but does not free the object.
pub struct PyExceptionObj {
    pub(crate) type_tag: u8,
    /// The exception class's source name, as a pointer to UTF-8 bytes with no
    /// terminator, plus `name_len`. Part 2 of #541 (D-189): the name used to
    /// be derived from `type_tag` by a `match` in this module, which could
    /// only ever name the seven builtin classes. User-defined exception
    /// classes carry module-assigned tags this runtime knows nothing about, so
    /// the name now travels with the object. Null means "unknown", which
    /// prints as `Exception`.
    pub(crate) name: *const u8,
    pub(crate) name_len: usize,
    pub(crate) message: *mut PyStrObj,
    /// The enclosing function's source name (`"<module>"` at top level)
    /// where this exception was raised (#707), as a pointer to UTF-8 bytes
    /// with no terminator, plus `frame_function_len`. Null means "not
    /// recorded" -- codegen always calls [`pycc_rt_exception_set_frame`]
    /// immediately after allocating the exception it is about to raise, but
    /// a handler-bound exception that a program merely inspects (e.g. `str
    /// (e)`) without reraising never observably depends on this field being
    /// set, and every unit test in this module that hand-builds a
    /// `PyExceptionObj` bypasses codegen entirely. `exception_print_and_exit`
    /// renders no `File "..."` line at all when this is null, rather than a
    /// blank or placeholder function name.
    pub(crate) frame_function: *const u8,
    pub(crate) frame_function_len: usize,
    /// Explicit `raise ... from cause` chain.
    pub(crate) cause: *mut PyExceptionObj,
    /// Implicit handler context. Reserved but not wired in this part.
    pub(crate) _context: *mut PyExceptionObj,
    /// Part 3 of #382 (#542, PEP 654): an `ExceptionGroup`/`BaseExceptionGroup`
    /// instance's member exceptions, as an owned, heap-allocated array of
    /// `exceptions_len` pointers. Null (with `exceptions_len == 0`) for every
    /// ordinary, non-group exception -- which is every `PyExceptionObj` this
    /// runtime constructs outside [`pycc_rt_exception_group_alloc`] and
    /// [`pycc_rt_exception_group_partition`]. `exceptions_len == 0` doubles
    /// as "not a group" throughout this module: [`pycc_rt_exception_group_partition`]
    /// reads it to decide whether to treat `group` itself as an implicit
    /// single member.
    pub(crate) exceptions: *mut *mut PyExceptionObj,
    pub(crate) exceptions_len: usize,
    /// `str(exc)` when it differs from `message` (a `KeyError`), built on
    /// first use by [`str_form::exception_str`]; null until then.
    pub(crate) str_form: Cell<*mut PyStrObj>,
    /// A `StopIteration`'s `value` (a generator's return value), as the
    /// word codegen stores for the generator's return type, paired with the
    /// tag of that type (`pycc_codegen`'s `stop_value_tag`); `None` for
    /// every other exception and a value-less `StopIteration`. The value is
    /// owned by the exception, which is never freed (`docs/DEVIATIONS.md`).
    /// The tag lets [`pycc_rt_exception_payload`] refuse a word of another
    /// type: a handler's static type for `e.value` comes from the generators
    /// its `try` body resumes, but the `StopIteration` may come from
    /// anywhere a call in that body reaches.
    pub(crate) payload: Cell<Option<(u64, u64)>>,
}

#[derive(Clone, Copy)]
struct ExceptionState {
    active: i8,
    value: *mut PyExceptionObj,
}

impl ExceptionState {
    const CLEAR: Self = Self {
        active: 0,
        value: std::ptr::null_mut(),
    };
}

std::thread_local! {
    static EXCEPTION_STATE: Cell<ExceptionState> = const { Cell::new(ExceptionState::CLEAR) };
}

/// A process-wide mirror of the pending state's `active` byte, read by
/// generated code as one inline `load i8` after every call that may raise,
/// instead of a call to [`pycc_rt_exception_active`].
///
/// The thread-local [`EXCEPTION_STATE`] stays authoritative. Only a native
/// program reads the mirror, and it is single-threaded, so the mirror is
/// exact there. An `ext` module may be entered from several CPython threads,
/// so its generated code calls [`pycc_rt_exception_active`] instead and never
/// reads this byte. Every write to the state goes through [`set_state`],
/// which updates both.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static pycc_rt_exc_pending: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(0);

fn set_state(state: ExceptionState) {
    EXCEPTION_STATE.with(|cell| cell.set(state));
    pycc_rt_exc_pending.store(state.active, std::sync::atomic::Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_active() -> i8 {
    EXCEPTION_STATE.with(|state| state.get().active)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_value() -> *mut PyExceptionObj {
    EXCEPTION_STATE.with(|state| state.get().value)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_clear() {
    set_state(ExceptionState::CLEAR);
}

/// D-244's `ext` boundary, exception half: the pending exception's type tag,
/// or `-1` when nothing is pending.
///
/// The `ext` artifact must **return** a failure to CPython, never terminate
/// the process. `pycc_rt_exception_print_and_exit` -- `native` mode's handler,
/// unchanged -- calls `std::process::exit(1)`, which inside an imported
/// extension module kills the interpreter rather than failing the import. So
/// the `ext` epilogue reads the pending state through this accessor trio,
/// raises the matching CPython exception from the C shim, and returns: `NULL`
/// from a per-export wrapper, `-1` from the `Py_mod_exec` slot.
///
/// Split three ways rather than returning a struct because the caller is C:
/// the tag selects a `PyExc_*` object (see this module's tag constants, whose
/// literal values `pycc_rt::ext_bridge`'s drift-guard test pins), and the
/// message travels as pointer + length because a `PyStrObj` is not
/// NUL-terminated.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_pending_type() -> i32 {
    EXCEPTION_STATE.with(|state| {
        let pending = state.get();
        if pending.active == 0 || pending.value.is_null() {
            return -1;
        }
        i32::from(unsafe { (*pending.value).type_tag })
    })
}

/// 1 when the pending exception is an `UnboundLocalError`
/// ([`pycc_rt_raise_unbound_local`]), which carries `NameError`'s tag; 0
/// otherwise. The shim raises CPython's own subclass for it.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_ext_pending_is_unbound_local() -> i32 {
    EXCEPTION_STATE.with(|state| {
        let pending = state.get();
        if pending.active == 0 || pending.value.is_null() {
            return 0;
        }
        let value = unsafe { &*pending.value };
        if value.name.is_null() {
            return 0;
        }
        let name = unsafe { std::slice::from_raw_parts(value.name, value.name_len) };
        i32::from(name == b"UnboundLocalError")
    })
}

/// The pending exception's message as UTF-8 bytes, writing its length through
/// `len`. Null (with `*len == 0`) when nothing is pending.
///
/// The bytes belong to the pending exception object, which this runtime never
/// frees (see [`PyExceptionObj`]'s leak-only note), so they stay readable
/// until the caller has copied them into a CPython exception -- which the
/// shim does immediately, before [`pycc_rt_exception_clear`].
///
/// # Safety
///
/// `len` must be non-null and point to a writable, aligned `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_pending_message(len: *mut usize) -> *const u8 {
    EXCEPTION_STATE.with(|state| {
        let pending = state.get();
        if pending.active == 0 || pending.value.is_null() {
            unsafe { *len = 0 };
            return std::ptr::null();
        }
        let message = unsafe { (*pending.value).message };
        if message.is_null() {
            unsafe { *len = 0 };
            return std::ptr::null();
        }
        let bytes = unsafe { (*message).bytes() };
        unsafe { *len = bytes.len() };
        bytes.as_ptr()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_alloc(
    type_tag: u8,
    name: *const u8,
    name_len: usize,
    message: *mut PyStrObj,
) -> *mut PyExceptionObj {
    Box::into_raw(Box::new(PyExceptionObj {
        type_tag,
        name,
        name_len,
        message,
        frame_function: std::ptr::null(),
        frame_function_len: 0,
        cause: std::ptr::null_mut(),
        _context: std::ptr::null_mut(),
        exceptions: std::ptr::null_mut(),
        exceptions_len: 0,
        str_form: Cell::new(std::ptr::null_mut()),
        payload: Cell::new(None),
    }))
}

/// Records `frame_function`/`frame_function_len` on `obj` (#707) --
/// codegen's `pycc_rt_exception_set_frame` counterpart, called once per
/// `raise`/`raise ... from ...` statement on the exception being raised,
/// immediately after allocating it and before the pending-exception state is
/// set. A no-op on a null `obj`, matching this module's other exception
/// operations' tolerance for a null primary object (e.g.
/// `pycc_rt_exception_raise_with_cause`).
///
/// # Safety
///
/// A non-null `obj` must point to a live `PyExceptionObj`. A non-null
/// `frame_function` must point to `frame_function_len` readable UTF-8 bytes
/// that outlive the object -- codegen only ever supplies a static string
/// constant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_set_frame(
    obj: *mut PyExceptionObj,
    frame_function: *const u8,
    frame_function_len: usize,
) {
    if obj.is_null() {
        return;
    }
    unsafe {
        (*obj).frame_function = frame_function;
        (*obj).frame_function_len = frame_function_len;
    }
}

/// Copies `exceptions_len` member pointers out of the caller-supplied
/// `exceptions` array into a freshly heap-allocated, owned slice, returning
/// its thin pointer and length (`(null, 0)` for an empty array). Shared by
/// [`pycc_rt_exception_group_alloc`] (a source-level `ExceptionGroup(...)`
/// construction) and [`pycc_rt_exception_group_partition`] (each subgroup it
/// builds), so both leave the exact same "owned, independently freed later
/// under this runtime's leak-only model" shape on `PyExceptionObj::exceptions`.
///
/// # Safety
/// `exceptions` must be null (iff `exceptions_len == 0`) or point to
/// `exceptions_len` valid `*mut PyExceptionObj` values, readable for the
/// duration of this call.
unsafe fn copy_member_array(
    exceptions: *const *mut PyExceptionObj,
    exceptions_len: usize,
) -> (*mut *mut PyExceptionObj, usize) {
    if exceptions_len == 0 {
        return (std::ptr::null_mut(), 0);
    }
    let slice = unsafe { std::slice::from_raw_parts(exceptions, exceptions_len) };
    let boxed: Box<[*mut PyExceptionObj]> = slice.to_vec().into_boxed_slice();
    (Box::leak(boxed).as_mut_ptr(), exceptions_len)
}

/// Builds a fresh `ExceptionGroup`-shaped [`PyExceptionObj`] wrapping
/// `members`, or returns null when `members` is empty. Used by
/// [`pycc_rt_exception_group_partition`] for both the matched and remaining
/// halves of a partition -- an empty half means "this handler caught
/// nothing" / "nothing is left to reraise", which codegen tests via a null
/// pointer check exactly like every other D-173 exception-object slot.
fn build_group_or_null(
    members: Vec<*mut PyExceptionObj>,
    group_type_tag: u8,
    group_name: *const u8,
    group_name_len: usize,
    message: *mut PyStrObj,
) -> *mut PyExceptionObj {
    if members.is_empty() {
        return std::ptr::null_mut();
    }
    // Safety: `members` was just built from live `*mut PyExceptionObj`
    // values (either `pycc_rt_exception_group_partition`'s own `group`, its
    // existing member array, or both), so the pointer and length passed to
    // `copy_member_array` are valid for the duration of this call.
    let (exceptions_ptr, exceptions_len) =
        unsafe { copy_member_array(members.as_ptr(), members.len()) };
    Box::into_raw(Box::new(PyExceptionObj {
        type_tag: group_type_tag,
        name: group_name,
        name_len: group_name_len,
        message,
        frame_function: std::ptr::null(),
        frame_function_len: 0,
        cause: std::ptr::null_mut(),
        _context: std::ptr::null_mut(),
        exceptions: exceptions_ptr,
        exceptions_len,
        str_form: Cell::new(std::ptr::null_mut()),
        payload: Cell::new(None),
    }))
}

/// Constructs a heap-allocated `ExceptionGroup`/`BaseExceptionGroup`
/// instance carrying `exceptions_len` member exceptions, copied out of
/// `exceptions` (Part 3 of #382, #542, PEP 654). The one production caller
/// is codegen's lowering of a literal-list `ExceptionGroup(msg, [e1, e2])`
/// construction (`pycc_types`/`pycc_mir` restrict the second argument to a
/// literal list -- see D-202 -- so codegen always has a fixed-size array of
/// already-evaluated member pointers to hand this in).
///
/// # Safety
/// `exceptions` must be null (iff `exceptions_len == 0`) or point to
/// `exceptions_len` valid `*mut PyExceptionObj` values, readable for the
/// duration of this call; the array is copied, not retained by pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_group_alloc(
    type_tag: u8,
    name: *const u8,
    name_len: usize,
    message: *mut PyStrObj,
    exceptions: *const *mut PyExceptionObj,
    exceptions_len: usize,
) -> *mut PyExceptionObj {
    // Safety: forwarded from this function's own safety contract.
    let (exceptions_ptr, exceptions_len) = unsafe { copy_member_array(exceptions, exceptions_len) };
    Box::into_raw(Box::new(PyExceptionObj {
        type_tag,
        name,
        name_len,
        message,
        frame_function: std::ptr::null(),
        frame_function_len: 0,
        cause: std::ptr::null_mut(),
        _context: std::ptr::null_mut(),
        exceptions: exceptions_ptr,
        exceptions_len,
        str_form: Cell::new(std::ptr::null_mut()),
        payload: Cell::new(None),
    }))
}

/// Partitions `group`'s member exceptions by whether they match any tag in
/// `tags` (Part 3 of #382, #542, PEP 654 `except*` dispatch): an empty
/// `tags` (`tags_len == 0`) matches every member, mirroring a bare
/// `except*:`; a nonempty `tags` matches a member whose own `type_tag`
/// equals [`EXCEPTION_TYPE_EXCEPTION`] (the catch-all, consistent with
/// [`pycc_rt_exception_type_matches`], which excludes only
/// [`EXCEPTION_TYPE_FOREIGN_BASE`]) or any entry in `tags`
/// (Part 2 of #541's multi-tag subclass matching, carried over unchanged).
///
/// `matched_out` receives a fresh `ExceptionGroup`-shaped group wrapping the
/// matched members, or null when none matched; `rest_out` receives the same
/// for the unmatched members, or null when none remain. Both constructed
/// groups are tagged `group_type_tag`/`group_name`/`group_name_len` --
/// supplied by codegen (which resolves `ExceptionGroup`'s fixed builtin tag
/// and name at MIR-lowering time) rather than hardcoded here, so this
/// runtime module carries no compile-time knowledge of that tag's numeric
/// value.
///
/// A `group` whose own `exceptions_len` is 0 is treated as an implicit
/// single-member group containing `group` itself: an ordinary exception
/// raised inside a `try*` body that is not already an `ExceptionGroup`,
/// which a matching `except*` clause still catches (and still wraps in a
/// fresh one-member group on the way out, matching CPython's own PEP 654
/// semantics).
///
/// # Safety
/// `group` must be non-null and point to a live `PyExceptionObj`. `tags`
/// must be null (iff `tags_len == 0`) or point to `tags_len` readable
/// bytes. `matched_out`/`rest_out` must be valid, non-null, writable
/// out-pointers.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_group_partition(
    group: *mut PyExceptionObj,
    tags: *const u8,
    tags_len: usize,
    group_type_tag: u8,
    group_name: *const u8,
    group_name_len: usize,
    matched_out: *mut *mut PyExceptionObj,
    rest_out: *mut *mut PyExceptionObj,
) {
    let obj = unsafe { &*group };
    let members: Vec<*mut PyExceptionObj> = if obj.exceptions_len == 0 {
        vec![group]
    } else {
        unsafe { std::slice::from_raw_parts(obj.exceptions, obj.exceptions_len) }.to_vec()
    };
    let tag_slice: &[u8] = if tags_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(tags, tags_len) }
    };
    let mut matched = Vec::new();
    let mut rest = Vec::new();
    for member in members {
        let member_tag = unsafe { (*member).type_tag };
        let is_match =
            tag_slice.is_empty() || tag_slice.iter().any(|tag| tag_matches(member_tag, *tag));
        if is_match {
            matched.push(member);
        } else {
            rest.push(member);
        }
    }
    let message = obj.message;
    unsafe {
        *matched_out =
            build_group_or_null(matched, group_type_tag, group_name, group_name_len, message);
        *rest_out = build_group_or_null(rest, group_type_tag, group_name, group_name_len, message);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_exception_raise(obj: *mut PyExceptionObj) {
    set_state(ExceptionState {
        active: 1,
        value: obj,
    });
}

/// Raises `obj` and records its explicit cause.
///
/// # Safety
///
/// A non-null `obj` and `cause` must point to live `PyExceptionObj`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_raise_with_cause(
    obj: *mut PyExceptionObj,
    cause: *mut PyExceptionObj,
) {
    if !obj.is_null() {
        unsafe { (*obj).cause = cause };
    }
    pycc_rt_exception_raise(obj);
}

/// Returns whether `obj` matches the requested builtin exception tag.
/// Matches `Exception`'s catch-all tag (for anything but
/// [`EXCEPTION_TYPE_FOREIGN_BASE`], #1316), or `obj`'s own tag exactly; it walks
/// no hierarchy -- a handler whose class has subclasses matches through its
/// MIR-built tag set, which codegen's dispatch chain
/// (`pycc_codegen::exception`) ORs over one call to this function per tag.
///
/// # Safety
///
/// A non-null `obj` must point to a live `PyExceptionObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_type_matches(
    obj: *mut PyExceptionObj,
    type_tag: u8,
) -> i8 {
    if obj.is_null() {
        return 0;
    }
    let obj_tag = unsafe { (*obj).type_tag };
    i8::from(tag_matches(obj_tag, type_tag))
}

/// Returns the exception's own message string, borrowed and unretained
/// (Part 3A of #541, #736): `print(e)`/f-string interpolation of a caught
/// exception binding must render CPython's `str(e)` semantics -- the message
/// alone, e.g. `boom` -- never `exception_print_and_exit`'s own uncaught-
/// exception `"{type}: {message}"` format, which this function does not
/// touch. No refcount/retain work is needed here: like
/// `pycc_rt_print_write_str`/`pycc_rt_str_concat`, this only borrows an
/// existing `PyStrObj` pointer rather than producing a new owned reference.
///
/// # Safety
///
/// A non-null `obj` must point to a live `PyExceptionObj` whose `message`
/// field is a live `PyStrObj` pointer -- true of every `PyExceptionObj` this
/// compiler's own codegen ever constructs (`pycc_rt_exception_alloc` always
/// receives a message, defaulting to `"unknown"` when the source `raise` has
/// no argument -- see `pycc_mir::lower_exception_value`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_message(obj: *mut PyExceptionObj) -> *mut PyStrObj {
    str_form::exception_str(unsafe { &*obj })
}

/// Records `text` as `obj`'s `str()` form. The CPython error bridge passes
/// the bridged exception's own `str(exc)`, which is also its message, so a
/// bridged `KeyError` shows CPython's text rather than its `repr` again.
///
/// # Safety
///
/// `obj` must point to a live `PyExceptionObj` and `text` to a live
/// `PyStrObj` that outlives it (the leak-only message the bridge allocated).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_set_str_form(
    obj: *mut PyExceptionObj,
    text: *mut PyStrObj,
) {
    unsafe { (*obj).str_form.set(text) };
}

/// The exception class's name, or `Exception` when the object carries none.
///
/// # Safety
///
/// A non-null `exc.name` must point to `exc.name_len` readable UTF-8 bytes
/// that outlive this call. Codegen only ever supplies a static string
/// constant, and `raise_builtin` only ever supplies a `&'static str`.
fn exception_type_name(exc: &PyExceptionObj) -> &str {
    if exc.name.is_null() {
        return "Exception";
    }
    let bytes = unsafe { std::slice::from_raw_parts(exc.name, exc.name_len) };
    std::str::from_utf8(bytes).unwrap_or("Exception")
}

/// The enclosing function's name `exc` was raised from (#707), or `None` when
/// no `raise`/`raise ... from ...` statement ever called
/// [`pycc_rt_exception_set_frame`] on it -- a handler-bound exception a
/// program only inspects (`str(e)`, `e.args`) without reraising, or any
/// `PyExceptionObj` a unit test builds directly without going through
/// codegen's emission path.
///
/// # Safety
///
/// A non-null `exc.frame_function` must point to `exc.frame_function_len`
/// readable UTF-8 bytes that outlive this call -- true of every frame name
/// `pycc_rt_exception_set_frame` ever records, since codegen only ever
/// supplies a static string constant.
fn exception_frame_function(exc: &PyExceptionObj) -> Option<&str> {
    if exc.frame_function.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(exc.frame_function, exc.frame_function_len) };
    Some(std::str::from_utf8(bytes).unwrap_or("<unknown>"))
}

/// Renders one exception's own `Traceback (most recent call last):` block
/// (when a frame was recorded, #707) followed by its `Type: message` line --
/// CPython's format for the single frame this compiler tracks, minus the
/// `File "...", line N,` prefix CPython's own block carries: `pycc_hir`
/// drops every statement's source span before `pycc_mir` lowering, so there
/// is no line number, and pycc has no notion of the original `.py` source
/// path at this point in the pipeline either (see the `not_proven` entries
/// this issue corrected in `tests/fixtures/conformance-breadth-manifest.json`
/// for the full accounting). An exception with no recorded frame renders
/// only its `Type: message` line, matching this runtime's pre-#707 output
/// exactly -- the common case for a `PyExceptionObj` a unit test builds by
/// hand, or a builtin error raised from inside a runtime helper (division by
/// zero, an out-of-range index) rather than from a source-level `raise`.
///
/// Pure and side-effect-free (no `eprintln!`/`process::exit`) so this is
/// exercised directly by unit tests below, unlike
/// [`exception_print_and_exit`] itself, which is `cfg(not(test))` and only
/// reachable from a compiled Python program's own uncaught-exception exit
/// path.
fn render_single_exception(exc: &PyExceptionObj) -> String {
    let mut out = String::new();
    if let Some(frame) = exception_frame_function(exc) {
        out.push_str("Traceback (most recent call last):\n");
        out.push_str(&format!("  File \"<compiled>\", in {frame}\n"));
    }
    let type_name = crate::qualified::exception(exception_type_name(exc));
    let type_name = type_name.as_str();
    // CPython's traceback prints a bare `ValueError` for `raise
    // ValueError("")` and a bare `AssertionError` for a failing `assert`
    // with no message (#1369, which lowers that to `AssertionError("")`):
    // the `: ` separator is omitted whenever `str(exc)` is empty.
    let shown = str_form::exception_str(exc);
    let msg_bytes: &[u8] = if shown.is_null() {
        &[]
    } else {
        unsafe { (*shown).bytes() }
    };
    if msg_bytes.is_empty() {
        out.push_str(type_name);
    } else {
        out.push_str(&format!(
            "{type_name}: {}",
            String::from_utf8_lossy(msg_bytes)
        ));
    }
    out
}

/// Renders `exc`'s full uncaught-exception text, walking its explicit
/// `.cause` chain (PEP 409, #707): each cause's own [`render_single_exception`]
/// block is rendered first, oldest cause first, joined to its effect by
/// CPython's exact `The above exception was the direct cause of the
/// following exception:` separator -- matching CPython's own chained
/// rendering order (the earliest exception in the chain prints first).
/// Implicit `__context__` chaining (a bare `raise` inside a handler with no
/// explicit `from`) is #606's separate, still-open scope; `_context` stays
/// unread here exactly as it already was before #707.
///
/// Valid Python source can make `.cause` cyclic -- `except ValueError as e:
/// raise e from e` sets `e.cause = e`, and a longer cycle
/// (`e1.cause = e2; e2.cause = e1`) is reachable the same way through two
/// `raise ... from` statements. This walks the chain iteratively (rather
/// than recursing per cause, which would overflow the stack on a cycle),
/// tracking every exception pointer already visited and stopping the walk
/// the moment a pointer repeats -- CPython applies the same cycle-breaking
/// rule to its own `__cause__`/`__context__` chains rather than looping
/// forever. Every distinct exception reachable before the walk detects a
/// repeat still renders exactly once, oldest first -- including a node
/// whose own `.cause` closes a cycle that started earlier in the chain
/// (e.g. `a.cause = b; b.cause = c; c.cause = b`: `a`, `b`, and `c` all
/// render once each, and the walk stops on re-visiting `b`).
fn render_exception_chain(exc: &PyExceptionObj) -> String {
    let mut chain: Vec<&PyExceptionObj> = Vec::new();
    let mut visited: HashSet<*const PyExceptionObj> = HashSet::new();
    let mut current: *const PyExceptionObj = exc;
    loop {
        if !visited.insert(current) {
            break;
        }
        let current_ref = unsafe { &*current };
        chain.push(current_ref);
        if current_ref.cause.is_null() {
            break;
        }
        current = current_ref.cause;
    }
    let mut out = String::new();
    for (idx, e) in chain.into_iter().rev().enumerate() {
        if idx > 0 {
            out.push_str(
                "\n\nThe above exception was the direct cause of the following exception:\n\n",
            );
        }
        out.push_str(&render_single_exception(e));
    }
    out
}

#[cfg(not(test))]
fn exception_print_and_exit(obj: *mut PyExceptionObj) -> ! {
    // Whatever was printed reaches stdout before the traceback and exit.
    crate::io::flush();
    if obj.is_null() {
        eprintln!("Exception");
        std::process::exit(1);
    }
    let exc = unsafe { &*obj };
    eprintln!("{}", render_exception_chain(exc));
    std::process::exit(1);
}

/// Prints an uncaught exception and exits with status 1.
///
/// # Safety
///
/// A non-null `obj` must point to a live `PyExceptionObj`.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_print_and_exit(obj: *mut PyExceptionObj) -> ! {
    exception_print_and_exit(obj)
}

/// The pending exception state, to be put back with [`restore_pending`]
/// after running code (a generator's close) that must not see or clobber
/// it.
pub(crate) fn save_pending() -> (i8, *mut PyExceptionObj) {
    let state = EXCEPTION_STATE.with(Cell::get);
    set_state(ExceptionState::CLEAR);
    (state.active, state.value)
}

pub(crate) fn restore_pending(saved: (i8, *mut PyExceptionObj)) {
    set_state(ExceptionState {
        active: saved.0,
        value: saved.1,
    });
}

/// Whether `obj` is an exception of the class named `name`.
pub(crate) fn is_named(obj: *mut PyExceptionObj, name: &str) -> bool {
    if obj.is_null() {
        return false;
    }
    // SAFETY: a non-null `obj` is a live exception object.
    let exc = unsafe { &*obj };
    !exc.name.is_null()
        && unsafe { std::slice::from_raw_parts(exc.name, exc.name_len) } == name.as_bytes()
}

/// Reports an exception nothing can catch (raised while a generator is
/// closed on deallocation) the way CPython's unraisable hook does.
pub(crate) fn report_unraisable(obj: *mut PyExceptionObj, what: &str) {
    crate::io::flush();
    eprintln!("Exception ignored in: <{what} object>");
    if obj.is_null() {
        eprintln!("Exception");
    } else {
        // SAFETY: a non-null `obj` is a live exception object.
        eprintln!("{}", render_exception_chain(unsafe { &*obj }));
    }
}

fn alloc_exception_message(msg: &str) -> *mut PyStrObj {
    let bytes = msg.as_bytes();
    let len = bytes.len();
    let payload = if len <= 22 {
        let mut buf = [0u8; 22];
        buf[..len].copy_from_slice(bytes);
        PyStrPayload::Inline(buf, len as u8)
    } else {
        PyStrPayload::Heap(bytes.into())
    };
    Box::into_raw(Box::new(PyStrObj {
        rc: Cell::new(1),
        payload,
        hash: Cell::new(-1),
    }))
}

/// Raises `NameError: name '<name>' is not defined` for a read of the
/// module global `name` (`len` UTF-8 bytes at `name`) before it was bound.
///
/// # Safety
/// `name` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_raise_name_error(name: *const u8, len: usize) {
    let name = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(name, len) });
    // A global some function rebinds lives in the cell `_pycc_global_<name>`
    // (`pycc_ast::desugar`): the user's name is the variable's.
    let name = name.strip_prefix("_pycc_global_").unwrap_or(&name);
    raise_builtin(
        EXCEPTION_TYPE_NAME_ERROR,
        "NameError",
        &format!("name '{name}' is not defined"),
    );
}

/// Raises CPython's `UnboundLocalError` for a read of the function local
/// `name` before any assignment to it ran. `UnboundLocalError` subclasses
/// `NameError`, whose tag it carries, so `except NameError` catches it.
///
/// # Safety
/// `name` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_raise_unbound_local(name: *const u8, len: usize) {
    let name = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(name, len) });
    raise_builtin(
        EXCEPTION_TYPE_NAME_ERROR,
        "UnboundLocalError",
        &format!("cannot access local variable '{name}' where it is not associated with a value"),
    );
}

/// Raises `StopIteration` carrying `value` (a generator's return value, the
/// word of its type, whose tag is `tag`; owned by the exception) and
/// `message` (`str(value)`, owned; null for a generator that returned no
/// value, whose `str(e)` is empty).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_raise_stop_iteration(value: u64, message: *mut PyStrObj, tag: u64) {
    let name = "StopIteration";
    let message_was_null = message.is_null();
    let message = if message.is_null() {
        alloc_exception_message("")
    } else {
        message
    };
    let obj = pycc_rt_exception_alloc(
        EXCEPTION_TYPE_STOP_ITERATION,
        name.as_ptr(),
        name.len(),
        message,
    );
    if !message_was_null {
        // SAFETY: freshly allocated.
        unsafe { (*obj).payload.set(Some((value, tag))) };
    }
    pycc_rt_exception_raise(obj);
}

/// The `value` word of a `StopIteration` [`pycc_rt_raise_stop_iteration`]
/// raised (borrowed: the exception keeps its reference), read as the type
/// whose tag is `tag` -- `0` reads it as `None`, the return type of a
/// generator that returns nothing. A word of any other type is never
/// reinterpreted: it raises `TypeError` and answers `0`, as does a
/// `StopIteration` without a value (its `value` is `None`) read as a type
/// that excludes `None` (`docs/DEVIATIONS.md`).
///
/// # Safety
/// `obj` must be a live exception object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_payload(obj: *mut PyExceptionObj, tag: u64) -> u64 {
    // SAFETY (caller): live.
    let payload = (!obj.is_null())
        .then(|| unsafe { (*obj).payload.get() })
        .flatten();
    match payload {
        Some((word, found)) if found == tag => word,
        None if tag == 0 => 0,
        None => {
            raise_builtin(
                EXCEPTION_TYPE_TYPE_ERROR,
                "TypeError",
                "StopIteration.value is None here, which its generator's return type excludes",
            );
            0
        }
        Some(_) => {
            raise_builtin(
                EXCEPTION_TYPE_TYPE_ERROR,
                "TypeError",
                "StopIteration.value here comes from a generator whose return type differs \
                 from that of the generators this `try` body resumes",
            );
            0
        }
    }
}

pub(crate) fn raise_builtin(type_tag: u8, name: &'static str, msg: &str) {
    let message = alloc_exception_message(msg);
    pycc_rt_exception_raise(pycc_rt_exception_alloc(
        type_tag,
        name.as_ptr(),
        name.len(),
        message,
    ));
}

/// Raises `KeyError(key)` for a missing container key of `kind`: a `str`
/// key's message is its text (whose `repr` `str(e)` shows); any other
/// key's `str(e)` is its `repr`, preset here.
pub(crate) fn raise_key_error(kind: i64, key: i64) {
    if kind == crate::list::LIST_KIND_STR {
        let text = String::from_utf8_lossy(unsafe { (*(key as *mut PyStrObj)).bytes() });
        raise_builtin(EXCEPTION_TYPE_KEY_ERROR, "KeyError", &text);
        return;
    }
    let mut rendered = String::new();
    crate::value::repr(kind, key, &mut rendered);
    let name = "KeyError";
    let obj = pycc_rt_exception_alloc(
        EXCEPTION_TYPE_KEY_ERROR,
        name.as_ptr(),
        name.len(),
        alloc_exception_message(&rendered),
    );
    unsafe { (*obj).str_form.set(alloc_exception_message(&rendered)) };
    pycc_rt_exception_raise(obj);
}

mod str_form;

#[cfg(test)]
mod foreign_base_tests;

#[cfg(test)]
mod tests;
