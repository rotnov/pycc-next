//! Generator objects.
//!
//! A compiled generator function is an LLVM coroutine (switch-resumed ABI,
//! `pycc_codegen::generator`). Calling it runs the ramp: it allocates the
//! coroutine frame, binds the arguments, creates a [`PyGenObj`] with
//! [`pycc_rt_gen_new`] and suspends before the first statement of the body,
//! returning the object -- so nothing of the body runs until the first
//! `next()`, as in CPython.
//!
//! The object owns the frame. [`pycc_rt_gen_step`] runs the body to its
//! next `yield` (which stores the value at [`VALUE_OFFSET`] and suspends) or
//! to its end (which marks the object done at its final suspend). The
//! consumer takes ownership of each yielded value. The same area carries a
//! value sent in (`send(v)`, read by the `yield` expression the body resumes
//! at) and the value a `return v` leaves for `StopIteration.value`.
//!
//! The body checks the object's mode each time it resumes: a generator being
//! closed raises `GeneratorExit` at the suspended `yield`, one a value is
//! thrown into (`throw(e)`) raises that exception there.
//!
//! Dropping the last reference to an unfinished generator *closes* it, as
//! CPython's `generator.close()` does on deallocation: the body is resumed
//! closing, so its `finally` clauses and its owned-slot epilogue run and
//! release every reference its frame holds. A `GeneratorExit` that leaves
//! the body is swallowed; any other exception, or the `RuntimeError` of a
//! generator that yields while closing, is reported on stderr and
//! swallowed, as CPython's unraisable hook does. The pending exception of
//! the code that dropped the generator (a generator is often released while
//! an exception unwinds) is saved and restored around the close.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::exception::{
    EXCEPTION_TYPE_GENERATOR_EXIT, EXCEPTION_TYPE_RUNTIME_ERROR, EXCEPTION_TYPE_STOP_ITERATION,
    EXCEPTION_TYPE_TYPE_ERROR, EXCEPTION_TYPE_VALUE_ERROR, PyExceptionObj, raise_builtin,
};

/// Byte offset of the state byte; `pycc_codegen::generator` mirrors it.
pub const STATE_OFFSET: usize = 16;
/// Byte offset of the mode byte (closing / throwing); mirrored by codegen.
pub const MODE_OFFSET: usize = 17;
/// Byte offset of the "a value was sent" flag; mirrored by codegen.
pub const SENT_OFFSET: usize = 19;
/// Byte offset of the "the area holds a return value" flag; mirrored by
/// codegen.
pub const HAS_RET_OFFSET: usize = 20;
/// Byte offset of the value area; mirrored by codegen.
pub const VALUE_OFFSET: usize = 64;

/// Suspended at a `yield` (or before the body's first statement).
const SUSPENDED: u8 = 0;
/// Its body is running.
const RUNNING: u8 = 1;
/// Its body has finished; the frame is freed.
const DONE: u8 = 2;

/// Resumed normally.
const MODE_NONE: u8 = 0;
/// Resumed to close: the `yield` raises `GeneratorExit`.
pub(crate) const MODE_CLOSING: u8 = 1;
/// Resumed by `throw()`: the `yield` raises [`PyGenObj::thrown`].
const MODE_THROWING: u8 = 2;

/// [`pycc_rt_gen_step`]'s answers.
const STEP_FINISHED: i8 = 0;
const STEP_YIELDED: i8 = 1;
const STEP_RETURNED: i8 = 2;

/// Releases a value of the generator's yield (or return) type stored at the
/// pointer; emitted by codegen per generator function, null when the type
/// holds no reference.
type DropFn = Option<extern "C" fn(*mut u8)>;

/// A generator object: the header below, then the value area (as large as
/// the generator's yield, send and return types need).
#[repr(C)]
pub struct PyGenObj {
    rc: i64,
    /// The coroutine frame (handle); null once freed. Its first word is the
    /// resume function, its second the destroy function (LLVM's
    /// switch-resumed ABI).
    frame: *mut u8,
    state: u8,
    mode: u8,
    /// The body has been resumed at least once (it is past its start).
    started: u8,
    /// The current resume carries a value sent in.
    sent: u8,
    /// The value area holds the value of the body's `return v`.
    has_ret: u8,
    /// A native iterator (`crate::itertools`): its `repr` is
    /// `<NAME object at 0x...>`, as CPython's `itertools` objects print.
    native: u8,
    _pad: [u8; 2],
    size: u32,
    _pad2: u32,
    /// The exception `throw()` raises at the suspended `yield`.
    thrown: *mut PyExceptionObj,
    drop_yield: DropFn,
    drop_ret: DropFn,
    /// The generator function's `__qualname__`, NUL-terminated (null when
    /// unknown), which its `repr` names.
    name: *const u8,
}

const _: () = assert!(std::mem::size_of::<PyGenObj>() == VALUE_OFFSET);

static GEN_LIVE: AtomicI64 = AtomicI64::new(0);

/// The number of generator objects allocated and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_live_objects() -> i64 {
    GEN_LIVE.load(Ordering::Relaxed)
}

fn layout(value_size: usize) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(VALUE_OFFSET + value_size.max(8), 8)
        .expect("a generator's value area fits a layout")
}

/// A new generator of the function named `name` (a NUL-terminated
/// `__qualname__`, or null) owning coroutine frame `frame`, with a value
/// area of `value_size` bytes. Called by the ramp, once, before its first
/// suspend.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_new(
    frame: *mut u8,
    value_size: i64,
    drop_yield: DropFn,
    drop_ret: DropFn,
    name: *const u8,
) -> *mut PyGenObj {
    let size = usize::try_from(value_size).unwrap_or(0);
    // SAFETY: a non-zero-size layout.
    let g = unsafe { std::alloc::alloc_zeroed(layout(size)) }.cast::<PyGenObj>();
    if g.is_null() {
        std::alloc::handle_alloc_error(layout(size));
    }
    // SAFETY: freshly allocated (and zeroed) and large enough for the
    // header.
    unsafe {
        (*g).rc = 1;
        (*g).frame = frame;
        (*g).state = SUSPENDED;
        (*g).size = size as u32;
        (*g).drop_yield = drop_yield;
        (*g).drop_ret = drop_ret;
        (*g).name = name;
    }
    GEN_LIVE.fetch_add(1, Ordering::Relaxed);
    g
}

/// Python `repr(g)`: `<generator object NAME at 0x...>`.
pub(crate) fn gen_repr(g: *mut PyGenObj) -> String {
    // SAFETY (caller): `g` is a live generator.
    let name = crate::closure::c_name(unsafe { (*g).name });
    if unsafe { (*g).native } != 0 {
        return format!("<{name} object at {:#x}>", g as usize);
    }
    format!("<generator object {name} at {:#x}>", g as usize)
}

/// Marks `g` a native iterator (see [`PyGenObj::native`]).
pub(crate) fn mark_native(g: *mut PyGenObj) {
    // SAFETY (caller): `g` is live.
    unsafe { (*g).native = 1 };
}

/// What a native iterator's frame is resumed for: `0` for the next value,
/// [`MODE_CLOSING`] to close, [`MODE_THROWING`] to raise a thrown exception
/// (which this call raises, clearing the mode).
pub(crate) fn native_resume_mode(g: *mut PyGenObj) -> u8 {
    // SAFETY (caller): `g` is live and running.
    let mode = unsafe { (*g).mode };
    if mode == MODE_THROWING {
        // SAFETY: as above.
        unsafe { pycc_rt_gen_raise_mode(g) };
    }
    mode
}

/// Ends a native iterator from its own resume: the object is finished and
/// [`run`] frees its frame.
pub(crate) fn native_finish(g: *mut PyGenObj) {
    // SAFETY (caller): `g` is live and running.
    unsafe { (*g).state = DONE };
}

/// The value area of a native iterator.
pub(crate) fn native_area(g: *mut PyGenObj) -> *mut u8 {
    area(g)
}

/// `str(g)` / `repr(g)` of a generator, as a new `str`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_repr(g: *mut PyGenObj) -> *mut crate::PyStrObj {
    crate::str_core::new_pystr(gen_repr(g).as_bytes())
}

/// The value area of `g`.
fn area(g: *mut PyGenObj) -> *mut u8 {
    // SAFETY (caller): `g` is a live generator.
    unsafe { g.cast::<u8>().add(VALUE_OFFSET) }
}

/// Allocates a coroutine frame of `size` bytes (the generator function's
/// ramp calls it with `llvm.coro.size`). The size is kept in a header word
/// in front of the frame, for [`pycc_rt_gen_frame_free`].
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_frame_alloc(size: i64) -> *mut u8 {
    let size = usize::try_from(size).unwrap_or(0) + 16;
    let layout = std::alloc::Layout::from_size_align(size, 16).expect("a frame fits a layout");
    // SAFETY: a non-zero-size layout.
    let base = unsafe { std::alloc::alloc(layout) };
    if base.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    // SAFETY: the allocation holds the header word and the frame.
    unsafe {
        base.cast::<usize>().write(size);
        base.add(16)
    }
}

/// Frees a frame [`pycc_rt_gen_frame_alloc`] allocated (null is a no-op).
///
/// # Safety
/// `frame` must be null or a frame from [`pycc_rt_gen_frame_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_frame_free(frame: *mut u8) {
    if frame.is_null() {
        return;
    }
    // SAFETY (caller): the header word sits 16 bytes before the frame.
    unsafe {
        let base = frame.sub(16);
        let size = base.cast::<usize>().read();
        std::alloc::dealloc(
            base,
            std::alloc::Layout::from_size_align_unchecked(size, 16),
        );
    }
}

/// Calls the frame's resume (`which == 0`) or destroy (`which == 1`)
/// function.
///
/// # Safety
/// `frame` must be a live coroutine frame of the switch-resumed ABI.
unsafe fn frame_call(frame: *mut u8, which: usize) {
    // SAFETY (caller): the first two words of the frame are its resume and
    // destroy functions.
    let f: extern "C" fn(*mut u8) = unsafe { *(frame.cast::<extern "C" fn(*mut u8)>().add(which)) };
    f(frame);
}

/// Frees the frame of a generator whose body has reached its final
/// suspend.
fn destroy_frame(g: *mut PyGenObj) {
    // SAFETY: `g` is live; a non-null frame is suspended at its final
    // suspend point, where destroying it only frees it.
    unsafe {
        let frame = (*g).frame;
        if !frame.is_null() {
            (*g).frame = std::ptr::null_mut();
            frame_call(frame, 1);
        }
    }
}

/// Releases the value the body's `return v` left in the area, if any.
fn drop_return_value(g: *mut PyGenObj) {
    // SAFETY: `g` is live.
    unsafe {
        if (*g).has_ret != 0 {
            (*g).has_ret = 0;
            if let Some(drop) = (*g).drop_ret {
                drop(area(g));
            }
        }
    }
}

/// Releases a value the body yielded that nobody takes (a generator that
/// yields while it is being closed).
fn drop_yielded_value(g: *mut PyGenObj) {
    // SAFETY: `g` is live.
    unsafe {
        if let Some(drop) = (*g).drop_yield {
            drop(area(g));
        }
    }
}

/// Runs the body once, from where it is suspended, with `sent` telling the
/// `yield` it resumes at whether a value was sent in.
///
/// # Safety
/// `g` must be a live, suspended generator.
unsafe fn run(g: *mut PyGenObj, sent: u8) -> i8 {
    // SAFETY (caller): `g` is live and suspended.
    unsafe {
        (*g).state = RUNNING;
        (*g).sent = sent;
        (*g).started = 1;
        frame_call((*g).frame, 0);
        (*g).sent = 0;
        if (*g).state != DONE {
            (*g).state = SUSPENDED;
            return STEP_YIELDED;
        }
        destroy_frame(g);
        if (*g).has_ret != 0 {
            STEP_RETURNED
        } else {
            STEP_FINISHED
        }
    }
}

/// PEP 479: a `StopIteration` leaving a generator's body becomes
/// `RuntimeError: generator raised StopIteration`.
fn stop_iteration_to_runtime_error() {
    if crate::exception::pycc_rt_exception_active() == 0 {
        return;
    }
    let value = crate::exception::pycc_rt_exception_value();
    // SAFETY: a pending exception value is a live exception object.
    if !value.is_null() && unsafe { (*value).type_tag } == EXCEPTION_TYPE_STOP_ITERATION {
        crate::exception::pycc_rt_exception_clear();
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "generator raised StopIteration",
        );
    }
}

/// Resumes `g`: `1` when it yielded (the value is in the area, owned by the
/// caller), `2` when its body returned a value (in the area, owned by the
/// caller, to become `StopIteration.value`), `0` when it finished without
/// one, was already finished, or raised (the exception is pending). `sent`
/// is `1` when the caller stored a value to send in the area.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_step(g: *mut PyGenObj, sent: i8) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => return STEP_FINISHED,
            RUNNING => {
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                return STEP_FINISHED;
            }
            _ => {}
        }
        let answer = run(g, u8::from(sent != 0));
        if answer == STEP_FINISHED {
            stop_iteration_to_runtime_error();
        }
        answer
    }
}

/// Runs `g`'s body to its next `yield` (`1`: a value is at
/// [`VALUE_OFFSET`], owned by the caller) or to its end (`0`: exhausted;
/// an exception the body raised is pending; a returned value is released).
/// A generator resumed from its own body raises `ValueError: generator
/// already executing`.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_resume(g: *mut PyGenObj) -> i8 {
    // SAFETY (caller): `g` is live.
    match unsafe { pycc_rt_gen_step(g, 0) } {
        STEP_RETURNED => {
            drop_return_value(g);
            STEP_FINISHED
        }
        answer => answer,
    }
}

/// [`pycc_rt_gen_resume`] without PEP 479's conversion: a `StopIteration`
/// the body raised stays pending as it is. For a generator pycc makes of a
/// call's arguments (a keyed `min`/`max`'s pairs, `pycc_ast::desugar::keys`),
/// whose body is the call's own key calls: CPython runs them in no generator.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_resume_raw(g: *mut PyGenObj) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => return STEP_FINISHED,
            RUNNING => {
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                return STEP_FINISHED;
            }
            _ => {}
        }
        match run(g, 0) {
            STEP_RETURNED => {
                drop_return_value(g);
                STEP_FINISHED
            }
            answer => answer,
        }
    }
}

/// `next(g)`: [`pycc_rt_gen_resume`], raising `StopIteration` when the
/// generator is exhausted without an exception of its own.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_next(g: *mut PyGenObj) -> i8 {
    // SAFETY (caller): `g` is live.
    let got = unsafe { pycc_rt_gen_resume(g) };
    if got == 0 && crate::exception::pycc_rt_exception_active() == 0 {
        raise_builtin(EXCEPTION_TYPE_STOP_ITERATION, "StopIteration", "");
    }
    got
}

/// Whether `g.send(value)` may store `value` in the area and resume
/// ([`pycc_rt_gen_step`] with `sent == 1`): `1` when `g` is suspended,
/// else `0` with CPython's exception raised -- `StopIteration` for a
/// finished generator, `ValueError` for a running one, and `TypeError` for
/// a non-`None` value sent to one that has not started (`is_none == 0`).
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_can_send(g: *mut PyGenObj, is_none: i8) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => {
                raise_builtin(EXCEPTION_TYPE_STOP_ITERATION, "StopIteration", "");
                0
            }
            RUNNING => {
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                0
            }
            _ if (*g).started == 0 && is_none == 0 => {
                raise_builtin(
                    EXCEPTION_TYPE_TYPE_ERROR,
                    "TypeError",
                    "can't send non-None value to a just-started generator",
                );
                0
            }
            _ => 1,
        }
    }
}

/// The first half of `g.throw()` on a generator CPython holds (the
/// iterator `pycc_ext_gen_wrap` boxes it in): takes the pending exception
/// as the one to throw and readies `g` to raise it where it is suspended,
/// `1` when the caller is now to resume it (with the value function that
/// resumes it on every other step), `0` with an exception pending -- the
/// thrown one for a finished generator, `ValueError` for a running one.
///
/// # Safety
/// `g` must be a live generator, and an exception pending.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_throw_pending(g: *mut PyGenObj) -> i8 {
    let exc = crate::exception::pycc_rt_exception_take();
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => {
                crate::exception::pycc_rt_exception_raise(exc);
                0
            }
            RUNNING => {
                crate::exception::pycc_rt_exception_decref(exc);
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                0
            }
            _ => {
                (*g).mode = MODE_THROWING;
                (*g).thrown = exc;
                1
            }
        }
    }
}

/// `g.throw(exc)`: raises `exc` (whose reference passes to the call) at the
/// `yield` where `g` is suspended (at its start when it has not run yet),
/// with [`pycc_rt_gen_step`]'s answers. A finished generator raises `exc`
/// in the caller.
///
/// # Safety
/// `g` must be a live generator, `exc` a live exception object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_throw(g: *mut PyGenObj, exc: *mut PyExceptionObj) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => {
                crate::exception::pycc_rt_exception_raise(exc);
                STEP_FINISHED
            }
            RUNNING => {
                crate::exception::pycc_rt_exception_decref(exc);
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                STEP_FINISHED
            }
            _ => {
                (*g).mode = MODE_THROWING;
                (*g).thrown = exc;
                pycc_rt_gen_step(g, 0)
            }
        }
    }
}

/// Raises, at a `yield` the body just resumed at, what the generator's mode
/// asks for: `GeneratorExit` when it is being closed, the thrown exception
/// for `throw()`. The body calls it only when the mode byte is non-zero.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_raise_mode(g: *mut PyGenObj) {
    // SAFETY (caller): `g` is live.
    unsafe {
        if (*g).mode == MODE_THROWING {
            (*g).mode = MODE_NONE;
            let thrown = std::mem::replace(&mut (*g).thrown, std::ptr::null_mut());
            crate::exception::pycc_rt_exception_raise(thrown);
        } else {
            pycc_rt_gen_raise_exit();
        }
    }
}

/// Raises `TypeError` at a `yield` expression resumed by `next()` (no value
/// sent) whose generator's send type does not admit `None`
/// (`docs/DEVIATIONS.md`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_raise_nothing_sent() {
    raise_builtin(
        EXCEPTION_TYPE_TYPE_ERROR,
        "TypeError",
        "a yield expression whose send type excludes None was resumed by next() (send a value)",
    );
}

/// Raises `TypeError` where a `yield from` whose delegate had already
/// finished (CPython's result `None`) needs a value of a non-`None` return
/// type (`docs/DEVIATIONS.md`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_raise_no_return() {
    raise_builtin(
        EXCEPTION_TYPE_TYPE_ERROR,
        "TypeError",
        "yield from a finished generator gave None for a non-None return type",
    );
}

/// Raises `GeneratorExit` at the `yield` a closing generator resumes at.
/// It is not an `Exception`: `except Exception:` lets it through, a bare
/// `except:` and `finally` see it, as in CPython.
///
/// One `GeneratorExit` per thread, kept alive by a reference of its own, is
/// raised by every close rather than a new one each time. Nothing a program
/// can observe tells the two apart -- it carries no message, and the close
/// swallows it.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_raise_exit() {
    thread_local! {
        static GENERATOR_EXIT: std::cell::Cell<*mut crate::exception::PyExceptionObj> =
            const { std::cell::Cell::new(std::ptr::null_mut()) };
    }
    let exit = GENERATOR_EXIT.with(|cell| {
        if cell.get().is_null() {
            raise_builtin(EXCEPTION_TYPE_GENERATOR_EXIT, "GeneratorExit", "");
            cell.set(crate::exception::pycc_rt_exception_take());
        }
        cell.get()
    });
    // SAFETY: the cached `GeneratorExit` is a live exception the cell owns.
    // Each close starts its traceback afresh: the frames the last close
    // passed through are not this one's.
    unsafe { (*exit).tb.release() };
    unsafe { crate::exception::pycc_rt_exception_incref(exit) };
    crate::exception::pycc_rt_exception_raise(exit);
}

/// Whether the pending exception is `GeneratorExit`.
fn generator_exit_pending() -> bool {
    crate::exception::pycc_rt_exception_active() != 0
        && crate::exception::is_named(crate::exception::pycc_rt_exception_value(), "GeneratorExit")
}

/// `g.close()`: `1` when the generator is closed (it was finished, never
/// started, or its body let `GeneratorExit` out or returned), `0` with an
/// exception pending -- the body raised something else, or yielded
/// (`RuntimeError: generator ignored GeneratorExit`).
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_close(g: *mut PyGenObj) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => return 1,
            RUNNING => {
                raise_builtin(
                    EXCEPTION_TYPE_VALUE_ERROR,
                    "ValueError",
                    "generator already executing",
                );
                return 0;
            }
            _ => {}
        }
        (*g).mode = MODE_CLOSING;
        let answer = run(g, 0);
        (*g).mode = MODE_NONE;
        match answer {
            STEP_YIELDED => {
                drop_yielded_value(g);
                raise_builtin(
                    EXCEPTION_TYPE_RUNTIME_ERROR,
                    "RuntimeError",
                    "generator ignored GeneratorExit",
                );
                0
            }
            STEP_RETURNED => {
                drop_return_value(g);
                1
            }
            _ => {
                stop_iteration_to_runtime_error();
                if generator_exit_pending() {
                    crate::exception::pycc_rt_exception_clear();
                }
                i8::from(crate::exception::pycc_rt_exception_active() == 0)
            }
        }
    }
}

/// One step of `yield from sub` in the generator `this`: `first` starts
/// `sub` (`next`); otherwise `this` was just resumed and the step forwards
/// what resumed it -- a sent value (`send`, copied into `sub` when
/// `forward_send`), an exception (`throw`), or a close (`sub` is closed and
/// `GeneratorExit` raised in `this`). Answers like [`pycc_rt_gen_step`]; a
/// value `sub` yields is moved into `this`'s area (`yield_size` bytes), a
/// value it returns stays in `sub`'s.
///
/// # Safety
/// `this` and `sub` must be live generators.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_delegate(
    this: *mut PyGenObj,
    sub: *mut PyGenObj,
    first: i8,
    forward_send: i8,
    yield_size: i64,
    send_size: i64,
) -> i8 {
    // SAFETY (caller): both are live.
    unsafe {
        let answer = if first != 0 {
            pycc_rt_gen_step(sub, 0)
        } else {
            match (*this).mode {
                MODE_CLOSING => {
                    if pycc_rt_gen_close(sub) != 0 {
                        pycc_rt_gen_raise_exit();
                    }
                    return STEP_FINISHED;
                }
                MODE_THROWING => {
                    (*this).mode = MODE_NONE;
                    let thrown = std::mem::replace(&mut (*this).thrown, std::ptr::null_mut());
                    pycc_rt_gen_throw(sub, thrown)
                }
                _ if (*this).sent != 0 => {
                    if forward_send != 0 {
                        if pycc_rt_gen_can_send(sub, 0) == 0 {
                            return STEP_FINISHED;
                        }
                        std::ptr::copy_nonoverlapping(
                            area(this),
                            area(sub),
                            usize::try_from(send_size).unwrap_or(0),
                        );
                    }
                    pycc_rt_gen_step(sub, 1)
                }
                _ => pycc_rt_gen_step(sub, 0),
            }
        };
        if answer == STEP_YIELDED {
            std::ptr::copy_nonoverlapping(
                area(sub),
                area(this),
                usize::try_from(yield_size).unwrap_or(0),
            );
        }
        answer
    }
}

/// Raises a value-less `StopIteration` unless an exception is already
/// pending: what a `next()`/`send()`/`throw()` of a generator answering
/// `0` ([`pycc_rt_gen_step`]) raises in its caller.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_stop_unless_raised() {
    if crate::exception::pycc_rt_exception_active() == 0 {
        raise_builtin(EXCEPTION_TYPE_STOP_ITERATION, "StopIteration", "");
    }
}

/// Takes one more reference to `g` (null is a no-op).
///
/// # Safety
/// `g` must be null or a live generator object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_incref(g: *mut PyGenObj) {
    if !g.is_null() {
        // SAFETY: a non-null `g` is live.
        unsafe { (*g).rc += 1 };
    }
}

/// Releases one reference; the last one closes an unfinished generator,
/// frees its frame and the object.
///
/// # Safety
/// `g` must be null or a live generator object whose reference the caller
/// owns and gives up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_decref(g: *mut PyGenObj) {
    if g.is_null() {
        return;
    }
    // SAFETY: a non-null `g` is live, and this is the only place it is
    // freed.
    unsafe {
        (*g).rc -= 1;
        if (*g).rc > 0 {
            return;
        }
        if (*g).state == SUSPENDED && !(*g).frame.is_null() {
            close_on_release(g);
        }
        destroy_frame(g);
        drop_return_value(g);
        crate::exception::pycc_rt_exception_decref((*g).thrown);
        std::alloc::dealloc(g.cast::<u8>(), layout((*g).size as usize));
    }
    GEN_LIVE.fetch_sub(1, Ordering::Relaxed);
}

/// Closes a suspended generator whose last reference is going: its body
/// unwinds from the suspended `yield` (see the module documentation).
fn close_on_release(g: *mut PyGenObj) {
    let saved = crate::exception::save_pending();
    // SAFETY: `g` is live and suspended, its frame live.
    unsafe {
        let started = (*g).started != 0;
        (*g).mode = MODE_CLOSING;
        if run(g, 0) == STEP_YIELDED {
            // The body caught `GeneratorExit` and yielded again: reported
            // as CPython reports it, and the frame is freed where it stands
            // (its locals are not released -- `docs/DEVIATIONS.md`).
            drop_yielded_value(g);
            raise_builtin(
                EXCEPTION_TYPE_RUNTIME_ERROR,
                "RuntimeError",
                "generator ignored GeneratorExit",
            );
            crate::exception::report_unraisable(
                crate::exception::pycc_rt_exception_value(),
                "generator",
            );
            crate::exception::pycc_rt_exception_clear();
            let frame = std::mem::replace(&mut (*g).frame, std::ptr::null_mut());
            (*g).state = DONE;
            frame_call(frame, 1);
        }
        (*g).mode = MODE_NONE;
        if crate::exception::pycc_rt_exception_active() != 0 {
            stop_iteration_to_runtime_error();
            if started && !generator_exit_pending() {
                crate::exception::report_unraisable(
                    crate::exception::pycc_rt_exception_value(),
                    "generator",
                );
            }
            crate::exception::pycc_rt_exception_clear();
        }
    }
    crate::exception::restore_pending(saved);
}
