//! memoryview/buffer views over `float` storage: element access, length,
//! allocation and free.

use super::*;

/// The two words a `pycc build --ext` wrapper is allowed to carry across the
/// boundary for a `memoryview` parameter (Part 2 of #1027).
///
/// This is the Rust view of `PyccExtBufferView` in `src/ext/pycc_ext_module.c`
/// -- `{ void *ptr; long long len; }` -- and the layout must agree field for
/// field, which `ext_buffer_view_layout_matches_the_c_struct` below pins.
/// `len` is a *copy* of the exporter's `shape[0]`, taken while the buffer is
/// held; the owning `Py_buffer` and `PyObject *` stay wrapper-side, so nothing
/// here can outlive or reach them.
///
/// Compiled code never constructs one: the generated wrapper fills a stack
/// local and passes its address as the `Ty::MemoryView` argument.
#[repr(C)]
pub struct PyccExtBufferView {
    /// The exporter's data pointer, a C-contiguous one-dimensional `double`
    /// array of `len` elements (the `"d"` format the unpack shim requires).
    pub ptr: *mut core::ffi::c_void,
    /// A copy of the exporter's `shape[0]`, in elements, never in bytes.
    pub len: i64,
}

/// Private half of `pycc_rt_buffer_f64_get` below, split for the same reason
/// `list_get` is: so this file's unit tests can drive the out-of-range
/// raise and inspect the pending state directly.
///
/// # Safety
/// `view.ptr` must point at `view.len` contiguous `f64` values.
pub(crate) unsafe fn buffer_f64_get(view: &PyccExtBufferView, index: i64) -> f64 {
    if index < 0 || index >= view.len {
        // D-173: set the pending exception flag instead of panicking. The
        // message is CPython's own sentence for this failure, so a subject
        // that goes out of bounds reports what `memoryview.__getitem__`
        // would have reported.
        //
        // D-108: a negative index raises here rather than wrapping. The
        // deviation is exactly the range `[-len, -1]`, which CPython would
        // have resolved from the end.
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "index out of bounds on dimension 1",
        );
        return 0.0;
    }
    // The `--ext` wrapper admits an exporter on four properties --
    // writable-or-not, one-dimensional, C-contiguous, format `"d"` -- and
    // none of them implies that the exporter's storage is 8-byte aligned.
    // `memoryview(bytearray(17))[1:].cast("d")` satisfies every one of them
    // on CPython and reports a data address that is `1 mod 8`, so a plain
    // `*const f64` dereference here would be undefined behavior on a buffer
    // the boundary is required to accept. `read_unaligned` costs nothing on
    // an aligned address and keeps the unaligned exporter working rather
    // than turning it into a refusal.
    unsafe { core::ptr::read_unaligned((view.ptr as *const f64).add(index as usize)) }
}

/// Reads the element at `index` of a `pycc build --ext` export's `memoryview`
/// parameter (Python's `b[i]`, Part 2 of #1027), bounds-checked against the
/// `len` the wrapper copied out of the exporter's `shape[0]`.
///
/// Sets the pending `IndexError` flag (D-173) and returns a `0.0` sentinel on
/// an out-of-range index -- a type-valid `f64`, never a trap representation,
/// because the caller's generated code checks the flag after this call rather
/// than inspecting the value. The `--ext` wrapper's D-173 bridge releases the
/// buffer and re-raises it as a CPython `IndexError`.
///
/// # Element representation
/// `index` is a raw element offset; generated code obtains it by decoding an
/// int-compatible expression with `pycc_rt_int_untag_checked`. The return
/// value is an ordinary unencoded `Ty::Float`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView` whose `ptr` addresses `len`
/// contiguous `f64` values -- which is what the generated wrapper guarantees
/// for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_get(view: *const PyccExtBufferView, index: i64) -> f64 {
    unsafe { buffer_f64_get(&*view, index) }
}

/// The private half of [`pycc_rt_buffer_f64_set`], sharing
/// [`buffer_f64_get`]'s bounds rule verbatim so the load and the store can
/// never disagree about which indices are in range.
///
/// # Safety
/// `view.ptr` must address `view.len` contiguous, **writable** `f64` values
/// whenever `index` is in range. The generated `--ext` wrapper guarantees
/// that by acquiring the buffer with `PyBUF_WRITABLE` for exactly the
/// parameters the compiled body stores into (Part 1 of #1142).
pub(crate) unsafe fn buffer_f64_set(view: &PyccExtBufferView, index: i64, value: f64) {
    if index < 0 || index >= view.len {
        // The same two deviations the load carries, stated once per
        // operation rather than shared through a helper so neither can be
        // silently relaxed on one side: D-173's pending-exception flag in
        // place of a panic across the FFI boundary, and D-108's refusal of
        // a negative index instead of CPython's resolve-from-the-end.
        raise_builtin(
            EXCEPTION_TYPE_INDEX_ERROR,
            "IndexError",
            "index out of bounds on dimension 1",
        );
        return;
    }
    // Unaligned for the same reason the load is, and stated again rather
    // than shared so neither side can be relaxed alone: the four properties
    // the wrapper checks do not imply 8-byte alignment, and
    // `memoryview(bytearray(17))[1:].cast("d")` is a CPython buffer that
    // passes all four at a data address `1 mod 8`.
    unsafe { core::ptr::write_unaligned((view.ptr as *mut f64).add(index as usize), value) };
}

/// Writes `value` to the element at `index` of a `pycc build --ext`
/// export's `memoryview` parameter (Python's `b[i] = v`, Part 1 of #1142),
/// bounds-checked against the `len` the wrapper copied out of the
/// exporter's `shape[0]`.
///
/// The sibling of [`pycc_rt_buffer_f64_get`], and `*const` for the same
/// reason that one is: `PyccExtBufferView::ptr` is already `*mut c_void`,
/// so the view itself is only ever read and the mutability lives entirely
/// in the storage it points at.
///
/// Sets the pending `IndexError` flag (D-173) and returns **without
/// dereferencing `ptr`** on an out-of-range index. Unlike the load there is
/// no sentinel to return, so this is a `void` helper: the generated code's
/// statement-level guard is the only thing that ever observes the raise,
/// which is why `pycc_codegen`'s `MirStmt::BufferSet` arm calls
/// `guard_statement_effects` immediately after this call.
///
/// # Element representation
/// `index` is a raw element offset, obtained by decoding an int-compatible
/// expression with `pycc_rt_int_untag_checked`. `value` is an ordinary
/// unencoded `Ty::Float`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView` whose `ptr` addresses `len`
/// contiguous **writable** `f64` values -- which is what the generated
/// wrapper guarantees for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_set(
    view: *const PyccExtBufferView,
    index: i64,
    value: f64,
) {
    unsafe { buffer_f64_set(&*view, index, value) }
}

/// Returns the element count of a `pycc build --ext` export's `memoryview`
/// parameter (Python's `len(b)`, Part 4 of #1027).
///
/// This is the `len` word of the `PyccExtBufferView` the wrapper filled in --
/// a copy of the exporter's `shape[0]`, in elements and never in bytes -- so
/// it agrees with CPython's own `len(view)` for the one-dimensional `"d"`
/// buffers this artifact mode admits.
///
/// Unlike `pycc_rt_buffer_f64_get`, this cannot fail: there is no index to
/// range-check and no pending exception to set (D-173), which is why
/// `pycc_codegen`'s `expression_can_set_exception` answers `false` for the
/// node that calls it.
///
/// # Element representation
/// The returned count is a **raw, untagged** `i64`, matching
/// `pycc_rt_list_len`. Generated code re-tags it with D-141's
/// `raw_i64_to_tagged_int` before the value becomes a user-visible `Ty::Int`.
///
/// # Safety
/// `view` must be a live `PyccExtBufferView`, which is what the generated
/// wrapper guarantees for the whole duration of the compiled call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_len(view: *const PyccExtBufferView) -> i64 {
    unsafe { (*view).len }
}

/// Net count of live artifact-owned [`PyccExtBufferView`] allocations:
/// incremented by [`pycc_rt_buffer_f64_alloc`] and decremented by the one
/// path in [`pycc_rt_buffer_f64_free`] that actually releases storage. A
/// steady-state value of zero across a sequence of calls is exactly the
/// "the compiled function freed every buffer it allocated" property Part 2a
/// of #1142 (#1165) is about.
///
/// `Relaxed` is the correct ordering for the same reason [`STR_LIVE`]'s is:
/// the counter orders nothing else, no reader infers the state of any other
/// memory from it, and a probe reads it from the same thread that made the
/// calls it is measuring.
///
/// Only *artifact-owned* storage moves this counter. A `memoryview`
/// **parameter**'s view is a wrapper-side stack local the artifact never
/// allocated and never frees, so it is invisible here -- which is what makes
/// a difference between two reads a statement about the producer alone.
pub(crate) static BUFFER_LIVE: AtomicI64 = AtomicI64::new(0);

/// The current value of the live artifact-owned-buffer counter.
///
/// Deliberately **not** `#[cfg(test)]`-gated, for the reason
/// [`pycc_rt_str_live_objects`] states in full: its consumer is a hosted
/// D-244 `ext` module, which links the ordinary non-test `libpycc_rt.a`, so
/// a test-only symbol would simply not exist there. The cost of exporting it
/// unconditionally is one relaxed atomic per buffer allocation and per
/// buffer free.
///
/// The value is only meaningful as a *difference* between two reads taken
/// around a known sequence of calls.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_live_views() -> i64 {
    BUFFER_LIVE.load(Ordering::Relaxed)
}

/// Decodes the user-supplied `ndarray(n)` length word for #1165's producer.
///
/// This is the one D-141 `int` ingress position that must **not** go
/// through `int_untag_checked`. That decoder `panic!`s on a bigint or
/// malformed word, and a panic unwinding past a plain `extern "C" fn`
/// boundary is caught there and turned into a process abort -- which, for a
/// D-244 `ext` artifact, kills the host CPython interpreter rather than
/// raising anything the host could catch. `make(2 ** 62 - 1)` on a built
/// `--ext` module exited 134 before this function existed.
///
/// So the length is decoded through `decode_inline_or_raise` instead: a
/// bigint leaves a pending `OverflowError` (D-173) and yields the
/// type-valid sentinel `0`. `int_untag_checked` itself is deliberately left
/// alone. Its other call sites -- a list index, a slice bound, a `str`
/// repeat count, container and comprehension element validation -- consume
/// the decoded word with no guard behind them, so returning `0` there would
/// turn a loud abort into a silent wrong answer; converting them needs each
/// site's own guard and is tracked separately (see #1168 and its follow-up).
/// This site is safe because `pycc_codegen` emits
/// `guard_statement_effects` between this call and
/// [`pycc_rt_buffer_f64_alloc`], so the sentinel never reaches the
/// allocator: the generated code branches to the installed exception target
/// first, which is also what keeps this raise from being reported over by
/// the allocator's own negative-length `ValueError`.
///
/// Bool markers decode to `0`/`1` exactly as `int_untag_checked` decodes
/// them, so `ndarray(True)` keeps its D-086 meaning.
///
/// Split into a private ordinary-ABI function plus the thin `extern "C"`
/// wrapper below, following this crate's convention (see the implementation
/// notes above `int_add`, `range_continue` and `int_to_float`). Here the
/// split is for this crate's own tests rather than to guard an unwind: this
/// function never panics, which is the entire point of it.
pub(crate) fn buffer_alloc_untag_len(tagged: i64) -> i64 {
    // `unwrap_or(0)` rather than `unwrap_or_default()`: the `0` is the
    // documented *type-valid sentinel* `decode_inline_or_raise`'s contract
    // requires, not an incidental default.
    decode_inline_or_raise(tagged, "sizing a buffer with").unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_alloc_untag_len(tagged: i64) -> i64 {
    buffer_alloc_untag_len(tagged)
}

/// Allocates artifact-owned storage for `len` `f64` elements and the
/// [`PyccExtBufferView`] that addresses it (Python's `ndarray(n)` /
/// `NDArray(n)`, Part 2a of #1142 -- issue #1165).
///
/// The returned pointer is the *view*, already pointing at the storage, so
/// compiled code keeps handling exactly the `*const PyccExtBufferView` that
/// [`pycc_rt_buffer_f64_get`], [`pycc_rt_buffer_f64_set`] and
/// [`pycc_rt_buffer_len`] already take: a produced buffer and a parameter
/// buffer are the same shape to codegen, and nothing new reaches its
/// calling convention.
///
/// # Zero-fill is a deliberate deviation from `numpy.ndarray(n)`
/// CPython's `numpy.ndarray(5)` returns *uninitialized* storage. This
/// zero-fills, on two grounds recorded in D-244's #1165 amendment: Part 2b
/// hands this storage to a host process, and uninitialized artifact heap
/// reaching one is an information-disclosure surface; and a deterministic
/// producer is what lets a conformance test assert anything at all about
/// the value before a store.
///
/// # Length
/// `len < 0` sets a pending `ValueError` (D-173) and returns null, matching
/// CPython's own `ValueError: negative dimensions are not allowed`. The
/// caller's generated code checks the pending flag after this call, and the
/// null is safe for the epilogue to hand straight to
/// [`pycc_rt_buffer_f64_free`]. `len == 0` is admitted and yields a
/// zero-length view, which every existing helper already handles.
///
/// Both the reservation and the `resize` narrow `len` to `usize`, while the
/// recorded `PyccExtBufferView.len` keeps the original `i64`. That is sound
/// only where `usize` is at least 64 bits wide: on a narrower target a large
/// `len` would truncate for the storage while the view still advertised the
/// full length, and the `i64`-against-`i64` bounds check in
/// [`pycc_rt_buffer_f64_get`] and [`pycc_rt_buffer_f64_set`]
/// (`index < 0 || index >= view.len`) would admit an index past the end of
/// that storage. Every Tier-1 target is 64-bit -- `docs/ROADMAP.md`'s
/// platform table lists Linux x64/arm64, macOS x64/arm64 and Windows x64 --
/// so the case is unreachable as built; a 32-bit target would have to refuse
/// a `len` past `usize::MAX` here rather than truncate it.
///
/// A length whose storage cannot be reserved -- `len * 8` past `isize::MAX`,
/// or a genuine allocator failure -- sets a pending `RuntimeError` and
/// returns null the same way. *Both* allocations this function performs are
/// fallible (`Vec::try_reserve_exact`), the elements and the two-word view
/// alike, precisely so that case cannot `panic!` or `handle_alloc_error`
/// across this `extern "C"` boundary and abort the host interpreter; see
/// the comment at the check for why the class is `RuntimeError` rather than
/// CPython's `MemoryError`. Under a *genuine* out-of-memory condition the
/// pending `RuntimeError` is itself built by `raise_builtin`, whose message
/// and exception objects are still ordinary infallible allocations shared
/// with every other raise site in this runtime; closing that is a
/// runtime-wide change tracked separately, so the guarantee this function
/// makes on its own is exact for the capacity-overflow case and
/// best-effort for a true allocator failure.
///
/// # Allocator pairing
/// The storage is a `Box<[f64]>` and the view is a one-element
/// `Box<[PyccExtBufferView]>`, and [`pycc_rt_buffer_f64_free`] reconstructs
/// *those same two boxes*. The pairing is stated as a `Box` round trip
/// rather than as a hand-built `core::alloc::Layout` precisely so the size
/// and alignment cannot drift between the two halves: a mismatched-layout
/// deallocation is undefined behavior that no test notices by accident.
/// That is also why the view is a slice box rather than a
/// `Box<PyccExtBufferView>` rebuilt from a `Vec`'s pointer: the two layouts
/// do agree today, but only the round trip keeps them agreeing by
/// construction.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_buffer_f64_alloc(len: i64) -> *mut PyccExtBufferView {
    if len < 0 {
        raise_builtin(
            EXCEPTION_TYPE_VALUE_ERROR,
            "ValueError",
            "negative dimensions are not allowed",
        );
        return core::ptr::null_mut();
    }
    // #1166 round 8. `vec![0.0f64; len]` `panic!`s when `len * 8` exceeds
    // `isize::MAX` (and aborts outright on a genuine allocator failure), and
    // a panic unwinding past this `extern "C"` boundary is caught here and
    // turned into a process abort -- the host CPython interpreter's, for a
    // D-244 `ext` artifact. `ndarray(2 ** 62 - 1)` exited 134 that way, with
    // no bigint anywhere in it: the length is an ordinary inline smallint,
    // so the length *decoder* cannot be what catches this.
    //
    // `try_reserve_exact` is what makes the reservation fallible instead:
    // it reports both the capacity overflow and a real allocator failure as
    // an `Err` rather than unwinding.
    //
    // The two steps after it are non-aborting under a condition worth
    // naming rather than as an unconditional property of the API, since
    // `try_reserve_exact` itself guarantees capacity only *at least* the
    // request. The argument -- `resize` to exactly the reserved capacity
    // cannot reallocate, and `into_boxed_slice` on a vector whose length
    // equals its capacity skips `shrink_to_fit` -- holds only while the
    // capacity comes back exactly `len`. Where a vector's length is below
    // its capacity, `into_boxed_slice` does call `shrink_to_fit`, whose
    // failure path is the infallible `handle_alloc_error`: a process abort,
    // the exact class this function exists to close.
    //
    // What makes it exact is `RawVec`'s own bookkeeping rather than this
    // crate's choice of allocator. `RawVec` records the capacity it
    // *requested*, not the length of the block the allocator handed back --
    // "allocators currently return a `NonNull<[u8]>` whose length matches
    // the size requested. If that ever changes, the capacity here should
    // change to `ptr.len() / size_of::<T>()`" (`alloc::raw_vec`). So a
    // `#[global_allocator]` that over-allocated could not reopen this path
    // (and none is declared in this workspace in any case), while a future
    // std that recorded the returned block length could. The condition to
    // re-check is `len == capacity` at the `into_boxed_slice` below, not
    // the allocator in use.
    //
    // `RuntimeError` is a deliberate deviation, following `int_pow`'s
    // negative-exponent arm: CPython raises `MemoryError` here
    // (`bytearray(2 ** 62)` and `[0.0] * (2 ** 62)` both do), but this
    // runtime carries no `MemoryError` tag -- `pycc_hir`'s
    // `BUILTIN_EXCEPTION_CLASSES` does not list the class -- so there is no
    // conformant class it can name yet. Adding one is a cross-cutting
    // change to the class table, the `ext` bridge's tag switch and their
    // pinned counts, tracked separately rather than widened into this fix.
    //
    // #1166 round 11. The element storage was not the only allocation on
    // this path: the two-word `PyccExtBufferView` itself was a `Box::new`,
    // which is *infallible* -- on a genuine allocator failure it calls
    // `handle_alloc_error`, which aborts. Reserving the elements fallibly
    // and then aborting on the sixteen bytes that describe them closes
    // nothing, so the view is reserved through the same fallible path, and
    // both reservations report through the one failure arm below. The
    // `||` short-circuits, so a refused *length* commits no allocation at
    // all; in the other direction -- elements reserved, view refused --
    // the element reservation has been committed, and `storage`'s own drop
    // at the early return releases it.
    let mut storage: Vec<f64> = Vec::new();
    let mut view: Vec<PyccExtBufferView> = Vec::new();
    if storage.try_reserve_exact(len as usize).is_err() || view.try_reserve_exact(1).is_err() {
        raise_builtin(
            EXCEPTION_TYPE_RUNTIME_ERROR,
            "RuntimeError",
            "a buffer of this length cannot be allocated (CPython raises MemoryError; \
             this version has no MemoryError class to name)",
        );
        return core::ptr::null_mut();
    }
    storage.resize(len as usize, 0.0f64);
    let storage: Box<[f64]> = storage.into_boxed_slice();
    let ptr = Box::into_raw(storage) as *mut f64;
    // `push` cannot reallocate into a capacity already reserved for one
    // element, and `into_boxed_slice` on a length-one vector of capacity
    // one skips `shrink_to_fit` -- the same two conditions, and the same
    // `RawVec` exactness argument, that make the element storage above
    // non-aborting.
    view.push(PyccExtBufferView {
        ptr: ptr as *mut core::ffi::c_void,
        len,
    });
    let view: Box<[PyccExtBufferView]> = view.into_boxed_slice();
    BUFFER_LIVE.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(view) as *mut PyccExtBufferView
}

/// Releases storage produced by [`pycc_rt_buffer_f64_alloc`].
///
/// A documented **no-op on null**, exactly like [`pycc_rt_str_decref`]'s
/// null arm and for the same reason: the generated epilogue runs over every
/// buffer slot the function declared, including one a path never assigned,
/// whose slot `storage_slot_at_entry` null-initialized.
///
/// `view[0].len` is read *before* the storage is released and the view box
/// is released *last*, because the storage box's length is what makes its
/// deallocation layout the same one [`pycc_rt_buffer_f64_alloc`] used.
///
/// # Safety
/// `view` must be null or a pointer returned by
/// [`pycc_rt_buffer_f64_alloc`] that has not already been freed. It must
/// never be a `memoryview` **parameter**'s view: that storage belongs to the
/// host's exporter, and the checker refuses every shape that could route one
/// here (Part 2a of #1142 refuses assignment to a buffer parameter for
/// exactly this reason).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_buffer_f64_free(view: *mut PyccExtBufferView) {
    if view.is_null() {
        return;
    }
    // Rebuilt as the *one-element slice box* the allocation produced, not as
    // a `Box<PyccExtBufferView>`: since #1166's round-11 review the view is
    // reserved fallibly through a one-element `Vec`, and reconstructing the
    // same box is what keeps the deallocation layout identical to the one
    // the allocation used without a hand-written layout argument.
    let view = unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(view, 1)) };
    // Read `len` off the view before the storage box is rebuilt: the slice
    // length is half of the `Box<[f64]>` layout the allocation used.
    let storage =
        core::ptr::slice_from_raw_parts_mut(view[0].ptr as *mut f64, view[0].len as usize);
    drop(unsafe { Box::from_raw(storage) });
    BUFFER_LIVE.fetch_sub(1, Ordering::Relaxed);
    drop(view);
}
