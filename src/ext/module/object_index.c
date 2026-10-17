/*
 * Subscripts, membership and slices of a CPython object (non-strict mode,
 * #3): `o[k]`, `o[i]` with a native `int` index, `item in o`, and
 * `o[a:b:c]` as a load, a deletion and a store. Part of the one shim
 * translation unit (`src/ext_build.rs`'s `SHIM_C`), after `object_ops.c`,
 * whose packers these use.
 */

/*
 * Part 3 of #1026 (PR 3b of #1082): `o[k]` on a CPython object value
 * (`EXT_OBJ_GETITEM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed, exactly as `pycc_ext_obj_getattr` borrows its own base.
 * `k` is an *owned* reference produced by one of the `pycc_ext_obj_pack_*`
 * helpers above, and this function releases it on every path -- the same
 * arg-slot contract `pycc_ext_obj_call` imposes on the values those helpers
 * produce. Keeping one rule for every packed value is the point: generated
 * code creates a packed reference and hands it to a shim helper, and the
 * shim helper is what releases it, so codegen never has to.
 *
 * The result is a *new* reference, owned as `pycc_ext_obj_call`'s result
 * is (released when an unbound temporary, Part 1 of #1092) --
 * `PyObject_GetItem` hands back a new reference exactly as
 * `PyObject_GetAttrString` does, and the result is the only thing that
 * escapes into compiled code as an `object` value.
 *
 * A NULL `k` means the packer already failed with a CPython exception set.
 * Testing for that here rather than in LLVM
 * keeps the emitted code one straight line for the key and leaves the whole
 * operation with one failure edge instead of two, which is the identical
 * argument `pycc_ext_obj_call`'s own NULL scan records. A NULL `o` is
 * defence in depth: `PyObject_GetItem` dereferences `Py_TYPE(o)` with no
 * guard of its own, and the caller's own NULL check already routed a failed
 * producer to the foreign failure edge with its exception set, so
 * returning NULL without setting a second one leaves exactly one exception
 * pending.
 */
PyObject *pycc_ext_obj_getitem(PyObject *o, PyObject *k)
{
    PyObject *result;

    result = (o == NULL || k == NULL) ? NULL : PyObject_GetItem(o, k);
    Py_XDECREF(k);
    return result;
}

/*
 * `__pycc_fallback_ns["name"]`: a module global that non-strict fallback
 * code binds, read by compiled code from the module's namespace dict. As
 * `pycc_ext_obj_getitem` (consuming the packed key), but a missing name is
 * CPython's `NameError: name 'x' is not defined`, what reading the unbound
 * global raises, not the dict's `KeyError`.
 */
PyObject *pycc_ext_ns_getitem(PyObject *ns, PyObject *k)
{
    PyObject *result = NULL;

    if (ns != NULL && k != NULL) {
        result = PyDict_CheckExact(ns) ? PyDict_GetItemWithError(ns, k) : PyObject_GetItem(ns, k);
        if (PyDict_CheckExact(ns)) {
            if (result != NULL) {
                Py_INCREF(result);
            } else if (!PyErr_Occurred()) {
                PyErr_Format(PyExc_NameError, "name '%U' is not defined", k);
            }
        }
    }
    Py_XDECREF(k);
    return result;
}

/*
 * The smallint an encoded `int` word holds, through `*out`: 1 for a
 * smallint, 0 for any other word (a bool marker or a bigint), which the
 * callers below hand to the generic path instead.
 */
static int pycc_ext_small_index(long long encoded, Py_ssize_t *out)
{
    if (pycc_rt_ext_int_classify(encoded) != PYCC_EXT_INT_SMALLINT) {
        return 0;
    }
    *out = (Py_ssize_t)pycc_rt_ext_int_decode(encoded);
    return 1;
}

/*
 * `o[i]` with a native `int` key (`EXT_OBJ_GETITEM_INT_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`): `pycc_ext_obj_getitem` without boxing
 * the key when `o` is an exact `tuple` or `list` and the index is in range,
 * which is what CPython's own subscript of those types answers. Any other
 * case -- another type, a bool or bigint word, an index out of range, whose
 * `IndexError` is CPython's to word -- packs the key and takes the generic
 * path. `o` is borrowed and the word is borrowed, as `pycc_ext_obj_pack_int`
 * borrows it. Returns a new reference or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_getitem_int(PyObject *o, long long encoded)
{
    Py_ssize_t index;
    Py_ssize_t size;
    PyObject *item;

    if (o != NULL && pycc_ext_small_index(encoded, &index)) {
        if (PyTuple_CheckExact(o)) {
            size = PyTuple_Size(o);
            if (index < 0) {
                index += size;
            }
            if (index >= 0 && index < size) {
                item = PyTuple_GetItem(o, index);
                Py_XINCREF(item);
                return item;
            }
        } else if (PyList_CheckExact(o)) {
            size = PyList_Size(o);
            if (index < 0) {
                index += size;
            }
            if (index >= 0 && index < size) {
                item = PyList_GetItem(o, index);
                Py_XINCREF(item);
                return item;
            }
        }
    }
    return pycc_ext_obj_getitem(o, pycc_ext_obj_pack_int(encoded));
}

/*
 * Part 2b of #1371: `item in container` with a CPython object container
 * (`EXT_OBJ_CONTAINS_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `PySequence_Contains` is CPython's own `in`: `__contains__`, then
 * iteration, then the old `__getitem__` protocol, raising `TypeError` when
 * the container supports none of them. Returns 1, 0, or -1 with the
 * exception set; compiled code flips the answer for `not in`.
 *
 * The container is borrowed. The item is always a packed value -- an
 * object item too, through `pycc_ext_obj_pack_object`'s new reference -- and
 * is consumed on every path, a `NULL` one included, exactly as
 * `pycc_ext_obj_getitem` consumes its packed key: a `NULL` item is a failed
 * packer whose exception is already set. A `NULL` container is defence in
 * depth for the reason `pycc_ext_obj_getitem` records.
 */
int pycc_ext_obj_contains(PyObject *container, PyObject *item)
{
    int result;

    result = (container == NULL || item == NULL) ? -1 : PySequence_Contains(container, item);
    Py_XDECREF(item);
    return result;
}

/*
 * Parts 2b and 2c of #1371: builds the `slice` object for `o[start:stop:step]`
 * and `del o[start:stop:step]`, the shared half of `pycc_ext_obj_getslice`
 * and `pycc_ext_obj_delslice`.
 *
 * `present` says which bounds the source spelled (bit 0 start, bit 1 stop,
 * bit 2 step). A present bound is a packed value, consumed here on every
 * path -- a `NULL` one included, which is a failed packer whose exception is
 * already set -- and an absent bound's pointer is ignored
 * and handed to `PySlice_New` as `NULL`, which it reads as `None`, the
 * value CPython's own `o[:b]` builds. A `NULL` base is defence in depth for
 * the reason `pycc_ext_obj_getitem` records. Returns a new reference to the
 * slice, or `NULL` with the exception set (or, for a `NULL` base, with
 * whatever exception produced it).
 */
static PyObject *pycc_ext_obj_slice_of(PyObject *o, PyObject *start, PyObject *stop,
                                       PyObject *step, int present)
{
    PyObject *bounds[3];
    PyObject *slice = NULL;
    int failed = (o == NULL);
    int i;

    bounds[0] = (present & 1) ? start : NULL;
    bounds[1] = (present & 2) ? stop : NULL;
    bounds[2] = (present & 4) ? step : NULL;
    for (i = 0; i < 3; i++) {
        if ((present & (1 << i)) && bounds[i] == NULL) {
            failed = 1;
        }
    }
    if (!failed) {
        slice = PySlice_New(bounds[0], bounds[1], bounds[2]);
    }
    for (i = 0; i < 3; i++) {
        Py_XDECREF(bounds[i]);
    }
    return slice;
}

/*
 * Part 2b of #1371: `o[start:stop:step]` with a CPython object `o`
 * (`EXT_OBJ_GETSLICE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * The bounds follow `pycc_ext_obj_slice_of`'s contract, and the base is
 * borrowed. The `slice` object is released after the load. The result is a
 * *new* reference, owned as `pycc_ext_obj_call`'s result is (released when
 * an unbound temporary, Part 1 of #1092), or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_getslice(PyObject *o, PyObject *start, PyObject *stop, PyObject *step,
                                int present)
{
    PyObject *slice = pycc_ext_obj_slice_of(o, start, stop, step, present);
    PyObject *result;

    if (slice == NULL) {
        return NULL;
    }
    result = PyObject_GetItem(o, slice);
    Py_DECREF(slice);
    return result;
}

/*
 * `o[start:stop]` with native `int` bounds and no step
 * (`EXT_OBJ_GETSLICE_INT_SYMBOL`): `present` says which bounds were written
 * (bit 0 start, bit 1 stop), and an absent bound's word is ignored. An exact
 * `tuple` or `list` with smallint bounds is sliced directly, the bounds
 * clamped as `PySlice_AdjustIndices` clamps them for a step of 1, so the
 * result is the object CPython's own slice builds (an exact `tuple` sliced
 * whole is that tuple itself). Anything else packs the bounds and takes
 * `pycc_ext_obj_getslice`. The base and the words are borrowed. Returns a
 * new reference or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_getslice_int(PyObject *o, long long start, long long stop, int present)
{
    Py_ssize_t lo = 0;
    Py_ssize_t hi = PY_SSIZE_T_MAX;
    Py_ssize_t size;
    int tuple;
    int small = 1;

    if (present & 1) {
        small = small && pycc_ext_small_index(start, &lo);
    }
    if (present & 2) {
        small = small && pycc_ext_small_index(stop, &hi);
    }
    if (o != NULL && small && (PyTuple_CheckExact(o) || PyList_CheckExact(o))) {
        tuple = PyTuple_CheckExact(o);
        size = tuple ? PyTuple_Size(o) : PyList_Size(o);
        if (lo < 0) {
            lo += size;
            if (lo < 0) {
                lo = 0;
            }
        } else if (lo > size) {
            lo = size;
        }
        if (hi < 0) {
            hi += size;
            if (hi < 0) {
                hi = 0;
            }
        } else if (hi > size) {
            hi = size;
        }
        return tuple ? PyTuple_GetSlice(o, lo, hi) : PyList_GetSlice(o, lo, hi);
    }
    return pycc_ext_obj_getslice(o, (present & 1) ? pycc_ext_obj_pack_int(start) : NULL,
                                 (present & 2) ? pycc_ext_obj_pack_int(stop) : NULL, NULL,
                                 present & 3);
}

/*
 * Part 2c of #1371: `del o[start:stop:step]` with a CPython object `o`
 * (`EXT_OBJ_DELSLICE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`), the
 * statement twin of `pycc_ext_obj_getslice`.
 *
 * The bounds follow `pycc_ext_obj_slice_of`'s contract, and the base is
 * borrowed. `PyObject_DelItem` runs the base's own `__delitem__` with the
 * `slice`, which is released afterwards; nothing else outlives the call.
 * Returns `0`, or `-1` with the exception set: a `tuple` raises
 * `TypeError`, and a `dict` raises `KeyError` for the (hashable since
 * 3.12) slice key, exactly as CPython's own `del` does.
 */
int pycc_ext_obj_delslice(PyObject *o, PyObject *start, PyObject *stop, PyObject *step,
                          int present)
{
    PyObject *slice = pycc_ext_obj_slice_of(o, start, stop, step, present);
    int status;

    if (slice == NULL) {
        return -1;
    }
    status = PyObject_DelItem(o, slice);
    Py_DECREF(slice);
    return status;
}

/*
 * Non-strict mode (#3): `o[start:stop:step] = value` with a CPython object
 * `o` (`EXT_OBJ_SETSLICE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`), the
 * store twin of `pycc_ext_obj_delslice`.
 *
 * The bounds follow `pycc_ext_obj_slice_of`'s contract and the base is
 * borrowed. `value` is a packer's new reference, consumed on every path; a
 * NULL `value` is a failed packer whose exception is already set, and must
 * not reach `PyObject_SetItem`. Returns `0`, or `-1` with the exception
 * set -- whatever the base's own `__setitem__` raises for the `slice` key.
 */
int pycc_ext_obj_setslice(PyObject *o, PyObject *start, PyObject *stop, PyObject *step,
                          int present, PyObject *value)
{
    PyObject *slice = pycc_ext_obj_slice_of(o, start, stop, step, present);
    int status;

    if (slice == NULL || value == NULL) {
        Py_XDECREF(slice);
        Py_XDECREF(value);
        return -1;
    }
    status = PyObject_SetItem(o, slice, value);
    Py_DECREF(slice);
    Py_DECREF(value);
    return status;
}
