//! Generator objects.
//!
//! A compiled generator function is an LLVM coroutine (switch-resumed ABI,
//! `pycc_codegen::generator`). Calling it runs the ramp: it allocates the
//! coroutine frame, binds the arguments, creates a [`PyGenObj`] with
//! [`pycc_rt_gen_new`] and suspends before the first statement of the body,
//! returning the object -- so nothing of the body runs until the first
//! `next()`, as in CPython.
//!
//! The object owns the frame. [`pycc_rt_gen_resume`] runs the body to its
//! next `yield` (which stores the value at [`VALUE_OFFSET`] and suspends) or
//! to its end (which marks the object done at its final suspend). The
//! consumer takes ownership of each yielded value.
//!
//! Dropping the last reference to an unfinished generator *closes* it, as
//! CPython's `generator.close()` does on deallocation: the body is resumed
//! with the closing flag set, the suspended `yield` raises `GeneratorExit`,
//! and the body unwinds through its `finally` clauses and its owned-slot
//! epilogue, which releases every reference its frame holds. A
//! `GeneratorExit` that leaves the body is swallowed; any other exception
//! is reported on stderr and swallowed, as CPython's unraisable hook does.
//! The pending exception of the code that dropped the generator (a
//! generator is often released while an exception unwinds) is saved and
//! restored around the close.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::exception::{
    EXCEPTION_TYPE_FOREIGN_BASE, EXCEPTION_TYPE_STOP_ITERATION, EXCEPTION_TYPE_VALUE_ERROR,
    raise_builtin,
};

/// Byte offset of the state byte; `pycc_codegen::generator` mirrors it.
pub const STATE_OFFSET: usize = 16;
/// Byte offset of the closing flag; mirrored by codegen.
pub const CLOSING_OFFSET: usize = 17;
/// Byte offset of the yielded value; mirrored by codegen.
pub const VALUE_OFFSET: usize = 24;

/// Suspended at a `yield` (or before the body's first statement).
const SUSPENDED: u8 = 0;
/// Its body is running.
const RUNNING: u8 = 1;
/// Its body has finished; the frame is freed.
const DONE: u8 = 2;

/// A generator object: the header below, then the value area (as large as
/// the generator's element type needs).
#[repr(C)]
pub struct PyGenObj {
    rc: i64,
    /// The coroutine frame (handle); null once freed. Its first word is the
    /// resume function, its second the destroy function (LLVM's
    /// switch-resumed ABI).
    frame: *mut u8,
    state: u8,
    closing: u8,
    _pad: [u8; 6],
}

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

/// A new generator owning coroutine frame `frame`, with a value area of
/// `value_size` bytes. Called by the ramp, once, before its first suspend.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_new(frame: *mut u8, value_size: i64) -> *mut PyGenObj {
    let size = usize::try_from(value_size).unwrap_or(0);
    // SAFETY: a non-zero-size layout.
    let g = unsafe { std::alloc::alloc_zeroed(layout(size)) }.cast::<PyGenObj>();
    if g.is_null() {
        std::alloc::handle_alloc_error(layout(size));
    }
    // SAFETY: freshly allocated and large enough for the header; the size
    // of the value area is kept right after the header's padding.
    unsafe {
        (*g).rc = 1;
        (*g).frame = frame;
        (*g).state = SUSPENDED;
        size_cell(g).write(size as u32);
    }
    GEN_LIVE.fetch_add(1, Ordering::Relaxed);
    g
}

/// The value-area size, kept in the header's padding (bytes 20..24).
fn size_cell(g: *mut PyGenObj) -> *mut u32 {
    // SAFETY (caller): `g` is a live generator.
    unsafe { g.cast::<u8>().add(20).cast::<u32>() }
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
        std::alloc::dealloc(base, std::alloc::Layout::from_size_align_unchecked(size, 16));
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

/// Runs `g`'s body to its next `yield` (`1`: a value is at
/// [`VALUE_OFFSET`], owned by the caller) or to its end (`0`: exhausted;
/// an exception the body raised is pending). A generator resumed from its
/// own body raises `ValueError: generator already executing`.
///
/// # Safety
/// `g` must be a live generator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_gen_resume(g: *mut PyGenObj) -> i8 {
    // SAFETY (caller): `g` is live.
    unsafe {
        match (*g).state {
            DONE => return 0,
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
        (*g).state = RUNNING;
        frame_call((*g).frame, 0);
        if (*g).state == DONE {
            destroy_frame(g);
            0
        } else {
            (*g).state = SUSPENDED;
            1
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

/// Raises `GeneratorExit` at the `yield` a closing generator resumes at.
/// It is not an `Exception`: `except Exception:` lets it through, a bare
/// `except:` and `finally` see it, as in CPython.
///
/// Exception objects are not reference counted (`docs/DEVIATIONS.md`), so
/// one `GeneratorExit` per thread is raised by every close rather than a
/// new one each time: closing generators in a loop runs in constant memory.
/// Nothing a program can observe tells the two apart -- it carries no
/// message, and the close swallows it.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_raise_exit() {
    thread_local! {
        static GENERATOR_EXIT: std::cell::Cell<*mut crate::exception::PyExceptionObj> =
            const { std::cell::Cell::new(std::ptr::null_mut()) };
    }
    let exit = GENERATOR_EXIT.with(|cell| {
        if cell.get().is_null() {
            raise_builtin(EXCEPTION_TYPE_FOREIGN_BASE, "GeneratorExit", "");
            cell.set(crate::exception::pycc_rt_exception_value());
            crate::exception::pycc_rt_exception_clear();
        }
        cell.get()
    });
    crate::exception::pycc_rt_exception_raise(exit);
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_incref(g: *mut PyGenObj) {
    if !g.is_null() {
        // SAFETY: a non-null `g` is live.
        unsafe { (*g).rc += 1 };
    }
}

/// Releases one reference; the last one closes an unfinished generator,
/// frees its frame and the object.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_gen_decref(g: *mut PyGenObj) {
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
            close(g);
        }
        destroy_frame(g);
        let size = *size_cell(g) as usize;
        std::alloc::dealloc(g.cast::<u8>(), layout(size));
    }
    GEN_LIVE.fetch_sub(1, Ordering::Relaxed);
}

/// Resumes a suspended generator with its closing flag set, so its body
/// unwinds from the suspended `yield` (see the module documentation).
fn close(g: *mut PyGenObj) {
    let saved = crate::exception::save_pending();
    // SAFETY: `g` is live and suspended, its frame live.
    unsafe {
        (*g).closing = 1;
        // Each `yield` reached while closing raises `GeneratorExit` again,
        // so the body always reaches its final suspend.
        while (*g).state != DONE && !(*g).frame.is_null() {
            (*g).state = RUNNING;
            frame_call((*g).frame, 0);
            if (*g).state != DONE {
                (*g).state = SUSPENDED;
            }
        }
    }
    if crate::exception::pycc_rt_exception_active() != 0 {
        let value = crate::exception::pycc_rt_exception_value();
        if !crate::exception::is_named(value, "GeneratorExit") {
            crate::exception::report_unraisable(value, "generator");
        }
        crate::exception::pycc_rt_exception_clear();
    }
    crate::exception::restore_pending(saved);
}
