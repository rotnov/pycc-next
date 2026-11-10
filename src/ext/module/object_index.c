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
/*
 * `KeyError(k)` as an exact dict's `__getitem__` raises it: the key is the
 * exception's one argument even when it is a tuple.
 */
static void pycc_ext_dict_key_error(PyObject *k)
{
    PyObject *args = PyTuple_Pack(1, k);

    if (args != NULL) {
        PyErr_SetObject(PyExc_KeyError, args);
        Py_DECREF(args);
    }
}

/*
 * An exact `dict` (built by CPython or by compiled code, or received) is read
 * through the dict API rather than the object protocol (v0.10 accept 2): the
 * result and the `KeyError` are what `dict.__getitem__` gives, and a
 * subclass (`__missing__`, `OrderedDict`) still takes `PyObject_GetItem`.
 */
PyObject *pycc_ext_obj_getitem(PyObject *o, PyObject *k)
{
    PyObject *result;

    if (o != NULL && k != NULL && PyDict_CheckExact(o)) {
        result = PyDict_GetItemWithError(o, k);
        if (result != NULL) {
            Py_INCREF(result);
        } else if (!PyErr_Occurred()) {
            pycc_ext_dict_key_error(k);
        }
    } else {
        result = (o == NULL || k == NULL) ? NULL : PyObject_GetItem(o, k);
    }
    Py_XDECREF(k);
    return result;
}

/*
 * `o[k]` directly inside a `try` whose first handler catches a plain
 * `KeyError` and cannot see the exception (docs/design/compile-little.md, section 2.7;
 * `crates/pycc_codegen/src/exception/lazy_miss.rs`): as
 * `pycc_ext_obj_getitem`, but a key missing from an exact `dict` returns
 * `PYCC_EXT_LAZY_MISS` with no exception made, and the compiled code goes
 * straight to that handler. An exact dict raises nothing else on a miss
 * (no `__missing__`), so the `KeyError` it would have raised is the only
 * thing skipped. Any other mapping, and any failure of the lookup itself
 * (an unhashable key), raises as `pycc_ext_obj_getitem` does. Consumes `k`.
 */
#define PYCC_EXT_LAZY_MISS ((PyObject *)1)

int pycc_ext_fast_ops_enabled(void);

/* How many lookups answered `PYCC_EXT_LAZY_MISS`, which the tests read with
 * `ctypes` (`RUNTIME_COUNTERS` in `src/ext_build/link.rs`). */
static long long pycc_ext_lazy_miss_count = 0;

long long pycc_ext_lazy_misses(void)
{
    return pycc_ext_lazy_miss_count;
}

PyObject *pycc_ext_obj_getitem_lazy(PyObject *o, PyObject *k)
{
    PyObject *result;

    /* With the fast paths off (`PYCC_NO_FAST_OPS=1`, `fast_ops.c`) every
     * miss raises, which is how the tests compare the two paths. */
    if (o == NULL || k == NULL || !PyDict_CheckExact(o) || !pycc_ext_fast_ops_enabled()) {
        return pycc_ext_obj_getitem(o, k);
    }
    switch (PyDict_GetItemRef(o, k, &result)) {
    case 1:
        break;
    case 0:
        pycc_ext_lazy_miss_count++;
        result = PYCC_EXT_LAZY_MISS;
        break;
    default:
        result = NULL;
        break;
    }
    Py_DECREF(k);
    return result;
}

/*
 * `__pycc_fallback_ns["name"]`: a module global that non-strict fallback
 * code binds, read by compiled code from the module's namespace dict. As
 * `pycc_ext_obj_getitem` (consuming the packed key), but a name the module
 * does not bind is read from the builtins, a lazy import (3.15) is resolved
 * (`pycc_ext_ns_resolve`), and one bound nowhere is
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
                result = pycc_ext_ns_resolve(ns, k, result);
            } else if (!PyErr_Occurred()) {
                /* Not bound in the module: a builtin, as `LOAD_GLOBAL`
                 * reads it (`frozendict` on 3.15 where a module binds the
                 * name only on older versions). */
                PyObject *builtins = PyEval_GetBuiltins(); /* borrowed */

                result = builtins == NULL ? NULL : PyDict_GetItemWithError(builtins, k);
                if (result != NULL) {
                    Py_INCREF(result);
                } else if (!PyErr_Occurred()) {
                    PyErr_Format(PyExc_NameError, "name '%U' is not defined", k);
                }
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
/* 1 once a `tuple`'s and a `list`'s size and items were found where
 * CPython 3.13 and 3.15 keep them (`ob_size` after the header; a tuple's
 * items after it, on 3.14 and later after the tuple's cached hash too; a
 * list's item array after `ob_size`), -1 once they were not (the accessor
 * functions are called then), 0 before the first look. */
static int pycc_ext_seq_layout = 0;
static size_t pycc_ext_tuple_items_at = 0;
#define PYCC_SEQ_SIZE(o) (*(Py_ssize_t *)((char *)(o) + sizeof(PyObject)))
#define PYCC_TUPLE_ITEMS(o) ((PyObject **)((char *)(o) + pycc_ext_tuple_items_at))
#define PYCC_LIST_ITEMS(o) (*(PyObject ***)((char *)(o) + sizeof(PyObject) + sizeof(Py_ssize_t)))

/* How many `int` indexes `pycc_ext_obj_getitem_int` read at the layout,
 * which the tests read with `ctypes` (`RUNTIME_COUNTERS` in
 * `src/ext_build/link.rs`) to prove the probe passes. */
static long long pycc_ext_seq_reads = 0;

long long pycc_ext_seq_layout_reads(void)
{
    return pycc_ext_seq_reads;
}

static int pycc_ext_seq_probe(void)
{
    PyObject *tuple = PyTuple_Pack(2, Py_None, Py_True);
    PyObject *list = tuple == NULL ? NULL : PySequence_List(tuple);

    pycc_ext_seq_layout = -1;
    pycc_ext_tuple_items_at = sizeof(PyObject) + sizeof(Py_ssize_t)
                              + ((Py_Version >> 16) >= 0x030E ? sizeof(Py_hash_t) : 0);
    if (list != NULL && PYCC_SEQ_SIZE(tuple) == 2 && PYCC_TUPLE_ITEMS(tuple)[0] == Py_None
        && PYCC_TUPLE_ITEMS(tuple)[1] == Py_True && PYCC_SEQ_SIZE(list) == 2
        && PYCC_LIST_ITEMS(list) != NULL && PYCC_LIST_ITEMS(list)[0] == Py_None
        && PYCC_LIST_ITEMS(list)[1] == Py_True) {
        pycc_ext_seq_layout = 1;
    }
    PyErr_Clear();
    Py_XDECREF(list);
    Py_XDECREF(tuple);
    return pycc_ext_seq_layout;
}

PyObject *pycc_ext_obj_getitem_int(PyObject *o, long long encoded)
{
    Py_ssize_t index;
    Py_ssize_t size;
    PyObject *item;
    int tuple;

    if (o != NULL && pycc_ext_small_index(encoded, &index)
        && ((tuple = PyTuple_CheckExact(o)) || PyList_CheckExact(o))) {
        if (pycc_ext_seq_layout > 0 || (pycc_ext_seq_layout == 0 && pycc_ext_seq_probe() > 0)) {
            /* What the accessors below read, without calling them. */
            size = PYCC_SEQ_SIZE(o);
            if (index < 0) {
                index += size;
            }
            if (index >= 0 && index < size) {
                item = tuple ? PYCC_TUPLE_ITEMS(o)[index] : PYCC_LIST_ITEMS(o)[index];
                pycc_ext_seq_reads++;
                Py_INCREF(item);
                return item;
            }
        } else {
            size = tuple ? PyTuple_Size(o) : PyList_Size(o);
            if (index < 0) {
                index += size;
            }
            if (index >= 0 && index < size) {
                item = tuple ? PyTuple_GetItem(o, index) : PyList_GetItem(o, index);
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

    if (container == NULL || item == NULL) {
        result = -1;
    } else if (PyDict_CheckExact(container)) {
        /* `k in d` of an exact dict (see `pycc_ext_obj_getitem`). */
        result = PyDict_Contains(container, item);
    } else {
        result = PySequence_Contains(container, item);
    }
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
 * `s[start:stop]` of an exact `str` `s` with `Py_ssize_t`-range bounds and
 * no step (`str_words` in `crates/pycc_codegen`): an absent start is 0 and
 * an absent stop `PY_SSIZE_T_MAX`. The bounds are clamped as
 * `PySlice_AdjustIndices` clamps them for a step of 1, so the result is the
 * object `unicode_subscript` builds: the empty string for an empty slice,
 * `s` itself sliced whole, `PyUnicode_Substring` otherwise (a cached
 * one-character string for one Latin-1 character). No `slice` object or
 * bound `int` is made. Returns a new reference or `NULL` with the
 * exception set (a failed allocation).
 */
PyObject *pycc_ext_str_slice(PyObject *s, long long start, long long stop)
{
    Py_ssize_t size = PyUnicode_GetLength(s);
    Py_ssize_t lo = (Py_ssize_t)start;
    Py_ssize_t hi = (Py_ssize_t)stop;

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
    if (hi <= lo) {
        return PyUnicode_FromStringAndSize("", 0);
    }
    return PyUnicode_Substring(s, lo, hi);
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
