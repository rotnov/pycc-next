/*
 * How the shim (and the package unit, `src/ext/package/finder.c`) calls
 * the stable ABI: the first text of both units, ahead of bridge.c and of
 * finder.c, so every use below them is these forms.
 */
#define Py_LIMITED_API 0x030D0000
#include <Python.h>
#include <string.h>

/*
 * Inline reference counting (v0.8). Under `Py_LIMITED_API >= 3.12` the
 * headers make every `Py_INCREF`/`Py_DECREF` a call into libpython
 * (`_Py_IncRef`/`_Py_DecRef`), so the immortal-object check can change
 * without rebuilding extensions. The count's place is not what may change:
 * `ob_refcnt` at offset 0 of every object is part of the stable ABI, which
 * still runs abi3 extensions built against the 3.2-3.11 limited API, whose
 * macros update it inline and call `_Py_Dealloc` (an `abi_only` stable-ABI
 * function, exported by `python3.dll` too) at zero. These are the
 * non-limited inline forms CPython 3.13 to 3.15 share for a 64-bit GIL
 * build: an object whose low 32 bits are negative is immortal (3.13 keeps
 * them all ones, 3.14 and 3.15 at `3 << 30`), and neither an increment nor
 * a decrement touches it; any other increment adds one to the low 32 bits. A free-threaded build loads no abi3 extension; a debug build
 * (`Py_REF_DEBUG`, whose total-count bookkeeping these would skip) and a
 * 32-bit or big-endian one keep the calls. The generated code inlines the
 * same two forms (`pycc_codegen::object_rc`). A closing brace here carries a
 * comment: `shim_split` reads a bare column-0 `}` as a shim function's end.
 */
#if !defined(Py_GIL_DISABLED) && !defined(Py_REF_DEBUG) && SIZEOF_VOID_P == 8 && !PY_BIG_ENDIAN
#define PYCC_INLINE_RC 1
Py_LOCAL_INLINE(void) pycc_incref(PyObject *op) {
    uint32_t *low = (uint32_t *)op;
    if ((int32_t)*low >= 0) {
        *low = *low + 1;
    }
} /* pycc_incref */
Py_LOCAL_INLINE(void) pycc_decref(PyObject *op) {
    Py_ssize_t *count = (Py_ssize_t *)op;
    if ((int32_t)*count < 0) {
        return;
    }
    if (--*count == 0) {
        _Py_Dealloc(op);
    }
} /* pycc_decref */
Py_LOCAL_INLINE(void) pycc_xincref(PyObject *op) {
    if (op != NULL) {
        pycc_incref(op);
    }
} /* pycc_xincref */
Py_LOCAL_INLINE(void) pycc_xdecref(PyObject *op) {
    if (op != NULL) {
        pycc_decref(op);
    }
} /* pycc_xdecref */
Py_LOCAL_INLINE(PyObject *) pycc_newref(PyObject *op) {
    pycc_incref(op);
    return op;
} /* pycc_newref */
#undef Py_INCREF
#undef Py_DECREF
#undef Py_XINCREF
#undef Py_XDECREF
#undef Py_NewRef
#define Py_INCREF(op) pycc_incref(_PyObject_CAST(op))
#define Py_DECREF(op) pycc_decref(_PyObject_CAST(op))
#define Py_XINCREF(op) pycc_xincref(_PyObject_CAST(op))
#define Py_XDECREF(op) pycc_xdecref(_PyObject_CAST(op))
#define Py_NewRef(op) pycc_newref(_PyObject_CAST(op))
#undef Py_XNewRef
#define Py_XNewRef(op) pycc_xnewref(_PyObject_CAST(op))
Py_LOCAL_INLINE(PyObject *) pycc_xnewref(PyObject *op) {
    pycc_xincref(op);
    return op;
} /* pycc_xnewref */
#endif

/*
 * v0.12: the stable-ABI calls CPython's own import never makes, routed to
 * ones it does. A release built with BOLT (python.org's and the standalone
 * builds') lays out the functions its profile ran in its hot text and keeps
 * the rest, in their original order, in a cold section; the first call into
 * a cold function faults in the 64 KiB around it. Each form below is what
 * the replaced function does, through functions CPython's import runs:
 *
 * - the singletons are the stable ABI's own data (`_Py_NoneStruct` and the
 *   rest, exported by `python3.dll` too), which the 3.13+ limited API
 *   reaches through `Py_GetConstantBorrowed`;
 * - `PyObject_CallNoArgs(f)` is `PyObject_Vectorcall(f, NULL, 0, NULL)`,
 *   `PySet_New(it)` a vectorcall of `set`, `PyErr_ExceptionMatches(e)`
 *   `PyErr_GivenExceptionMatches(PyErr_Occurred(), e)`;
 * - `PyDict_GetItemString` and `PyDict_DelItemString` make the key and use
 *   the `PyObject *` forms, the first suppressing every error as it does;
 * - `PyMem_Calloc` and `PyMem_RawCalloc` are the `Malloc`s and a `memset`;
 * - `PyTuple_SetItem` on a tuple stores into its item array (after the
 *   3.14+ cached hash) when the interpreter is 3.13 to 3.15 and the inline
 *   reference counts apply (`PYCC_INLINE_RC`), as `PyTuple_SET_ITEM` does,
 *   releasing the item it replaces; any other case makes the call.
 */
#undef Py_None
#undef Py_True
#undef Py_False
#undef Py_NotImplemented
#undef Py_Ellipsis
#define Py_None (&_Py_NoneStruct)
#define Py_True _PyObject_CAST(&_Py_TrueStruct)
#define Py_False _PyObject_CAST(&_Py_FalseStruct)
#define Py_NotImplemented (&_Py_NotImplementedStruct)
#define Py_Ellipsis (&_Py_EllipsisObject)
#define PyObject_CallNoArgs(f) PyObject_Vectorcall((f), NULL, 0, NULL)
#define PyErr_ExceptionMatches(exc) PyErr_GivenExceptionMatches(PyErr_Occurred(), (exc))
#define PySet_New(iterable) pycc_set_new(iterable)
#define PyDict_GetItemString(dict, key) pycc_dict_get_item_string((dict), (key))
#define PyDict_DelItemString(dict, key) pycc_dict_del_item_string((dict), (key))
#define PyMem_Calloc(n, size) pycc_calloc(PyMem_Malloc, (n), (size))
#define PyMem_RawCalloc(n, size) pycc_calloc(PyMem_RawMalloc, (n), (size))
Py_LOCAL_INLINE(PyObject *) pycc_set_new(PyObject *iterable) {
    return PyObject_Vectorcall((PyObject *)&PySet_Type, iterable == NULL ? NULL : &iterable,
                               iterable == NULL ? 0 : 1, NULL);
} /* pycc_set_new */
Py_LOCAL_INLINE(PyObject *) pycc_dict_get_item_string(PyObject *dict, const char *key) {
    PyObject *value = NULL;
    PyObject *exc = PyErr_Occurred() ? PyErr_GetRaisedException() : NULL;

    if (PyDict_GetItemStringRef(dict, key, &value) < 0) {
        PyErr_Clear();
    }
    if (exc != NULL) {
        PyErr_SetRaisedException(exc);
    }
    /* Borrowed, as the dict holds it. */
    Py_XDECREF(value);
    return value;
} /* pycc_dict_get_item_string */
Py_LOCAL_INLINE(int) pycc_dict_del_item_string(PyObject *dict, const char *key) {
    PyObject *name = PyUnicode_FromString(key);
    int status = name == NULL ? -1 : PyDict_DelItem(dict, name);

    Py_XDECREF(name);
    return status;
} /* pycc_dict_del_item_string */
Py_LOCAL_INLINE(void *) pycc_calloc(void *(*alloc)(size_t), size_t n, size_t size) {
    void *block;

    if (size != 0 && n > (size_t)PY_SSIZE_T_MAX / size) {
        return NULL;
    }
    block = alloc(n * size == 0 ? 1 : n * size);
    if (block != NULL) {
        memset(block, 0, n * size);
    }
    return block;
} /* pycc_calloc */
#ifdef PYCC_INLINE_RC
#define PyTuple_SetItem(tuple, index, item) pycc_tuple_setitem((tuple), (index), (item))
Py_LOCAL_INLINE(int) pycc_tuple_setitem(PyObject *tuple, Py_ssize_t index, PyObject *item) {
    unsigned long version = (unsigned long)Py_Version >> 16;

    if (version >= 0x030D && version <= 0x030F && Py_IS_TYPE(tuple, &PyTuple_Type)
        && index >= 0 && index < Py_SIZE(tuple)) {
        PyObject **items = (PyObject **)((char *)tuple + sizeof(PyVarObject)
                                         + (version >= 0x030E ? sizeof(Py_hash_t) : 0));
        PyObject *old = items[index];

        items[index] = item;
        Py_XDECREF(old);
        return 0;
    }
    return (PyTuple_SetItem)(tuple, index, item);
} /* pycc_tuple_setitem */
#endif
