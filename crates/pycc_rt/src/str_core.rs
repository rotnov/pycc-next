//! The `str` object (`PyStrObj`) and its core operations: construction, concatenation,
//! repetition, comparison, truthiness and reference counting.

use super::*;

/// D-059's `str` representation: up to 22 bytes are stored inline directly
/// in the `PyStrObj` allocation itself (no separate heap allocation for the
/// byte payload); anything longer heap-allocates a second, separate byte
/// buffer. Either way, `PyStrObj` itself (its refcount included) is always
/// exactly one heap allocation -- `pycc_codegen` never sees anything but an
/// opaque pointer to it (the same ABI-avoidance principle D-061 already
/// applies to `int`'s `BigInt`: no struct ever crosses the LLVM/Rust
/// boundary by value).
pub(crate) enum PyStrPayload {
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
    pub(crate) rc: Cell<u32>,
    pub(crate) payload: PyStrPayload,
    /// `hash(s)` once computed (a `str` is immutable), `-1` before: CPython
    /// caches it the same way, so a dict or set keyed by `str` hashes each
    /// key object once.
    pub(crate) hash: Cell<i64>,
}

impl PyStrObj {
    pub(crate) fn bytes(&self) -> &[u8] {
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
pub(crate) static STR_LIVE: AtomicI64 = AtomicI64::new(0);

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
pub(crate) fn new_pystr(bytes: &[u8]) -> *mut PyStrObj {
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
        hash: Cell::new(-1),
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

/// Formats a `bool` the way CPython's own `str(b)` would: capitalized
/// `"True"`/`"False"`, never Rust's own lowercase `Display` spelling (Task
/// 8, reused unchanged by f-string interpolation and Task 10's `print`).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bool_to_str(value: i8) -> *mut PyStrObj {
    new_pystr(if value != 0 { b"True" } else { b"False" })
}
