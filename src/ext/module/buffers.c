/*
 * `Py_bf_getbuffer`, filled field by field.
 *
 * Every field is written on every call, including the ones a given `flags`
 * makes NULL: `PyObject_GetBuffer` does not zero the caller's `Py_buffer`, so
 * a field left alone is indeterminate storage the consumer would read.
 *
 * `readonly` is always 0. The storage is the artifact's own heap block, so
 * there is no host-imposed read-only view of it to respect, and a `memoryview`
 * the host cannot write would be a silently narrower object than the one
 * `ndarray(n)` describes.
 *
 * `view->obj` takes a reference to the exporter, which is what keeps the
 * storage alive for exactly as long as the consumer holds the buffer:
 * `PyBuffer_Release` drops it, and the last drop runs `tp_dealloc`.
 */
static int pycc_ext_buffer_getbuffer(PyObject *self, Py_buffer *view, int flags)
{
    PyccExtBufferObj *obj = (PyccExtBufferObj *)self;
    if (obj->view == NULL) {
        /*
         * `view->obj` is NULLed before the failure return because that is
         * what the buffer protocol requires of an exporter that refuses: a
         * consumer may still call `PyBuffer_Release` on the structure it
         * passed in, and a stale non-NULL `obj` would be decrefed.
         */
        view->obj = NULL;
        PyErr_SetString(PyExc_BufferError, "this buffer's storage has been released");
        return -1;
    }
    obj->shape[0] = (Py_ssize_t)obj->view->len;
    obj->strides[0] = (Py_ssize_t)sizeof(double);
    view->buf = obj->view->ptr;
    view->obj = Py_NewRef(self);
    view->len = (Py_ssize_t)obj->view->len * (Py_ssize_t)sizeof(double);
    view->itemsize = (Py_ssize_t)sizeof(double);
    view->readonly = 0;
    view->ndim = 1;
    /*
     * The three shape-describing fields are conditional on what the consumer
     * asked for, exactly as the buffer protocol requires: a `PyBUF_SIMPLE`
     * consumer treats a non-NULL `format` or `shape` as a contract violation.
     * The storage is C-contiguous and one-dimensional, so no request this
     * boundary can receive is refusable on its shape.
     */
    view->format = ((flags & PyBUF_FORMAT) == PyBUF_FORMAT) ? (char *)"d" : NULL;
    view->shape = ((flags & PyBUF_ND) == PyBUF_ND) ? obj->shape : NULL;
    view->strides = ((flags & PyBUF_STRIDES) == PyBUF_STRIDES) ? obj->strides : NULL;
    view->suboffsets = NULL;
    view->internal = NULL;
    return 0;
}

/*
 * `Py_bf_releasebuffer`, deliberately empty.
 *
 * `shape` and `strides` belong to the exporter, not to the view, so there is
 * nothing per-view to free; and the reference `getbuffer` put in `view->obj`
 * is released by `PyBuffer_Release` itself, *after* this slot runs. Dropping
 * it here as well would be a double decref.
 */
static void pycc_ext_buffer_releasebuffer(PyObject *self, Py_buffer *view)
{
    (void)self;
    (void)view;
}

/*
 * The storage's one release point. The heap-type pattern is
 * `pycc_ext_instance_dealloc`'s, and its comment carries the reasoning for
 * the `Py_tp_free`-then-`Py_DECREF(tp)` order; the difference here is the
 * `pycc_rt_buffer_f64_free` in front of it, which is what actually returns
 * the artifact-owned block.
 *
 * `view` is cleared before the object is freed so that the field is never
 * read again from a partially destroyed object, and so `pycc_rt_buffer_f64_free`
 * -- a documented no-op on NULL -- cannot be handed the same pointer twice.
 */
static void pycc_ext_buffer_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    PyccExtBufferObj *obj = (PyccExtBufferObj *)self;
    freefunc tp_free;
    pycc_rt_buffer_f64_free(obj->view);
    obj->view = NULL;
    tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    tp_free(self);
    Py_DECREF(tp);
}

static PyType_Slot pycc_ext_buffer_type_slots[] = {
    {Py_tp_dealloc, pycc_ext_buffer_dealloc},
    {Py_bf_getbuffer, pycc_ext_buffer_getbuffer},
    {Py_bf_releasebuffer, pycc_ext_buffer_releasebuffer},
    {0, NULL},
};

/*
 * `Py_TPFLAGS_DISALLOW_INSTANTIATION` for the same reason a non-constructible
 * class carries it: nothing in Python may produce one of these, because an
 * exporter with a NULL `view` describes no storage. The only producer is
 * `pycc_ext_pack_memoryview`. The leading underscore keeps the name out of the
 * space a user class could collide with, and it is never published as a
 * module attribute in any case.
 */
static PyType_Spec pycc_ext_buffer_type_spec = {
    /* `PYCC_EXT_MODULE_NAME_STR "._buffer"`, set by `pycc_ext_exec_module`
     * before the type is made: the runtime unit of a split shim
     * (`src/ext_build/shim_split.rs`) has no module-name literal. */
    NULL,
    sizeof(PyccExtBufferObj),
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE,
    pycc_ext_buffer_type_slots,
};

/*
 * The buffer egress: takes ownership of `view` unconditionally and returns a
 * new `memoryview` over it, or NULL with an exception set.
 *
 * **The ownership boundary is the successful `tp_alloc`.** Before it, nothing
 * else can reach the storage, so a failure frees it directly. After it, the
 * exporter owns it and `tp_dealloc` is the only thing that may free it -- so
 * every later failure path releases the *exporter* and frees nothing itself.
 * Freeing directly after the store would be a double free, and returning
 * without freeing before it would be a leak.
 *
 * Both calls that can fail are checked: `tp_alloc` and
 * `PyMemoryView_FromObject`. No Rust allocation runs on this path at all, so
 * nothing here can abort the host interpreter the way an infallible `Box::new`
 * across the `extern "C"` boundary would.
 */
static PyObject *pycc_ext_pack_memoryview(PyccExtBufferView *view)
{
    PyObject *exporter;
    PyObject *result;
    allocfunc tp_alloc;
    /*
     * Neither arm is reachable from a well-formed artifact: the wrapper
     * checks `pycc_rt`'s pending flag before calling this, so a failed
     * allocation has already returned, and the exec slot fails the import
     * outright when the type cannot be created. Both are still handled,
     * because the alternative to a `SystemError` here is a NULL dereference
     * inside the host.
     */
    if (view == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc: internal error: a buffer return produced no storage");
        return NULL;
    }
    if (pycc_ext_buffer_type == NULL) {
        pycc_rt_buffer_f64_free(view);
        PyErr_SetString(PyExc_SystemError,
                        "pycc: internal error: the buffer exporter type is not registered");
        return NULL;
    }
    /*
     * `PyType_GetSlot(tp, Py_tp_alloc)` because the limited API exposes no
     * other way to reach a heap type's allocator, exactly as
     * `pycc_ext_instance_dealloc` reaches `Py_tp_free`. It zeroes the object,
     * so `view` starts NULL and the `shape`/`strides` arrays start defined
     * even if `getbuffer` never runs; and, this being a heap type, it takes
     * the type reference that `pycc_ext_buffer_dealloc` discharges.
     */
    tp_alloc = (allocfunc)PyType_GetSlot((PyTypeObject *)pycc_ext_buffer_type, Py_tp_alloc);
    exporter = tp_alloc((PyTypeObject *)pycc_ext_buffer_type, 0);
    if (exporter == NULL) {
        pycc_rt_buffer_f64_free(view);
        return NULL;
    }
    ((PyccExtBufferObj *)exporter)->view = view;
    /*
     * `PyMemoryView_FromObject` takes its own reference to the exporter
     * through the buffer it acquires, so the wrapper's own reference is
     * released on both arms -- the failing one included, which is what makes
     * a failure here free the storage exactly once, through `tp_dealloc`.
     */
    result = PyMemoryView_FromObject(exporter);
    Py_DECREF(exporter);
    return result;
}

/*
 * Part 1 of #1175: the buffer egress when the returned view is the *host's*
 * own buffer -- the name a `memoryview` parameter binds, returned by name.
 *
 * This is the one packer on the buffer path that allocates nothing and frees
 * nothing. `owner` is the argument object, a borrowed reference the caller
 * holds for the whole call; this function takes no reference of its own and
 * releases none, and there is no artifact-owned block anywhere in the path,
 * so `pycc_rt_buffer_live_views` is untouched by it.
 *
 * What keeps the returned view valid is not a reference to `owner` but a
 * buffer **export** on it: `PyMemoryView_FromObject` acquires its own, held
 * for exactly as long as the `memoryview` lives, and independent of the
 * `Py_buffer` the generated wrapper acquired and releases on its way out. A
 * bare `Py_INCREF(owner)` plus a `{ptr, len}` pair would *not* do -- a strong
 * reference does not pin storage, and a resizable exporter such as
 * `array.array('d')` relocates its block under `extend`. Holding an export is
 * what makes that same `extend` raise `BufferError` instead.
 *
 * The wrapper acquires this view *before* releasing its own buffer, so the
 * host object never reaches zero outstanding exports across the boundary;
 * `src/ext_build/wrappers.rs`'s `caller_owned_buffer_acquire` carries the
 * PEP 688 argument for why that ordering is required rather than tidy.
 *
 * NULL with the exception set on failure, which the generated wrapper returns
 * as-is: it must never fall through to `pycc_ext_pack_memoryview`, whose
 * exporter would free the host's storage in `tp_dealloc`.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed(PyObject *owner, const Py_buffer *held,
                                                   const char *fn_name, Py_ssize_t index,
                                                   int writable)
{
    PyObject *view;
    Py_buffer probe;
    const char *held_format;
    const char *probe_format;
    int differs;

    /*
     * The second export, and the reason it must be checked. PEP 688 lets an
     * exporter answer a later `__buffer__` with a *different* window, and
     * `PyMemoryView_FromObject` performs exactly such a later call. Nothing
     * routes that window through `pycc_ext_unpack_memoryview`, so without the
     * comparison below the host could receive storage the compiled body never
     * saw -- or a shape and format D-244 statement (e) refuses outright.
     *
     * Transferring the *first* export into the returned object instead is not
     * available under `Py_LIMITED_API`: `PyMemoryView_FromBuffer` copies the
     * `Py_buffer` without taking ownership of it (measured: the exporter's
     * `__release_buffer__` never runs), so that route leaks the export
     * permanently rather than moving it.
     *
     * Refusing an exotic-but-legal exporter is this boundary's existing idiom
     * -- `pycc_ext_unpack_memoryview` already refuses a conforming 2-D or
     * non-'d' one. A returned view whose window is not the one the call
     * operated on is refused the same way.
     */
    view = PyMemoryView_FromObject(owner);
    if (view == NULL) {
        return NULL;
    }
    /*
     * Probing the `memoryview` rather than `owner` deliberately: a
     * `memoryview`'s own `bf_getbuffer` re-describes the buffer it already
     * holds, so this reads back export #2 without asking the exporter for a
     * third one.
     */
    if (PyObject_GetBuffer(view, &probe,
                           PyBUF_C_CONTIGUOUS | PyBUF_FORMAT |
                               (writable ? PyBUF_WRITABLE : 0)) != 0) {
        Py_DECREF(view);
        return NULL;
    }
    held_format = (held->format == NULL) ? "" : held->format;
    probe_format = (probe.format == NULL) ? "" : probe.format;
    differs = probe.buf != held->buf || probe.len != held->len ||
              probe.itemsize != held->itemsize || probe.ndim != held->ndim ||
              probe.readonly != held->readonly || strcmp(probe_format, held_format) != 0;
    PyBuffer_Release(&probe);
    if (differs) {
        Py_DECREF(view);
        PyErr_Format(PyExc_BufferError,
                     "%s() argument %zd: the exporter described a different buffer on its "
                     "second export, so the window this call operated on cannot be returned",
                     fn_name, index + 1);
        return NULL;
    }
    return view;
}

/*
 * Part 2 of #1175 (#1179): `return b[start:stop]` over a caller-owned buffer
 * parameter.
 *
 * The bounds arrive out of band, through the three trailing `long long *`
 * out-slots the compiled function writes (`has_slice`, `start`, `stop`) --
 * `src/ext_build/wrappers.rs` carries why they cannot be in band. The
 * compiled body still returns the *whole* view's `PyccExtBufferView *`, so
 * the wrapper's `result == &a{index}` provenance test stays an exact pointer
 * identity and this function sees the same `owner`/`held` pair the
 * whole-view path does.
 *
 * Deriving the sub-range host-side, from a `memoryview` CPython itself
 * sliced, is what keeps the semantics Python's rather than this boundary's:
 * negative bounds, a `stop` past the end, and a crossed pair (`b[3:1]`, an
 * empty view) are all CPython's own, already-tested behaviour. An absent
 * bound is encoded by the compiled body as `0` for `start` and `LLONG_MAX`
 * for `stop`, both of which `slice` clamps exactly as `b[:]` does.
 *
 * The whole-window PEP 688 check runs first, unchanged, via the sibling
 * above: the window whose identity must be proven is the one the compiled
 * call operated on, not the sub-range derived from it afterwards. The
 * resulting `memoryview` holds the second export exactly as the whole-view
 * path's does -- `PyObject_GetItem` on a `memoryview` returns a new
 * `memoryview` that holds its own reference to the sliced one -- so the
 * `pycc_rt_buffer_live_views` accounting and the `array.array` resize
 * `BufferError` behave identically for both.
 *
 * NULL with the exception set on failure, which the generated wrapper
 * returns as-is.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed_slice(PyObject *owner, const Py_buffer *held,
                                                         const char *fn_name, Py_ssize_t index,
                                                         int writable, long long start,
                                                         long long stop)
{
    PyObject *whole;
    PyObject *start_obj;
    PyObject *stop_obj;
    PyObject *slice;
    PyObject *sub;

    whole = pycc_ext_pack_memoryview_borrowed(owner, held, fn_name, index, writable);
    if (whole == NULL) {
        return NULL;
    }
    start_obj = PyLong_FromLongLong(start);
    if (start_obj == NULL) {
        Py_DECREF(whole);
        return NULL;
    }
    stop_obj = PyLong_FromLongLong(stop);
    if (stop_obj == NULL) {
        Py_DECREF(start_obj);
        Py_DECREF(whole);
        return NULL;
    }
    /* `PySlice_New` borrows nothing: it takes its own references. */
    slice = PySlice_New(start_obj, stop_obj, NULL);
    Py_DECREF(start_obj);
    Py_DECREF(stop_obj);
    if (slice == NULL) {
        Py_DECREF(whole);
        return NULL;
    }
    sub = PyObject_GetItem(whole, slice);
    Py_DECREF(slice);
    Py_DECREF(whole);
    return sub;
}


