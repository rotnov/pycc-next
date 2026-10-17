/*
 * Byte buffers (`pycc_hir::ndarray`'s `NdDtype::Byte`): the run-time check
 * that an object is exactly a `bytearray`, which answers its buffer and
 * size for compiled element access, and the cold paths of that access
 * (each raises `bytearray`'s own exception and answers -1, the shape
 * `foreign_fail.rs` routes).
 *
 * Only limited-API calls read the object, so no `bytearray` layout is
 * assumed. None of these functions runs CPython code on success, and the
 * raising ones never do (`PYTHON_FREE_CALLS`, `crates/pycc_codegen/src/
 * ndarray/fresh.rs`): compiled code keeps its snapshot of the buffer
 * across calls to them.
 */

/*
 * Checks that `o` is exactly a `bytearray`; stores its buffer and size.
 * 0, or -1 with `TypeError` set.
 */
int pycc_ext_ba_check(PyObject *o, char **data, long long *size)
{
    PyObject *type_name;

    if (o != NULL && PyByteArray_CheckExact(o)) {
        *data = PyByteArray_AsString(o);
        *size = (long long)PyByteArray_Size(o);
        return 0;
    }
    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: bad bytearray check");
        return -1;
    }
    type_name = PyType_GetName(Py_TYPE(o));
    if (type_name == NULL) {
        return -1;
    }
    PyErr_Format(PyExc_TypeError, "expected bytearray, got %U", type_name);
    Py_DECREF(type_name);
    return -1;
}

/* `bytearray`'s out-of-range index error. Always -1. */
int pycc_ext_ba_index_error(void)
{
    PyErr_SetString(PyExc_IndexError, "bytearray index out of range");
    return -1;
}

/*
 * The machine index of an `int` word that is not a smallint: a bool marker
 * indexes 0 or 1; a bigint does not fit an index. 0, or -1 with
 * `IndexError` set.
 */
int pycc_ext_ba_index_word(long long encoded, long long *out)
{
    switch (pycc_rt_ext_int_classify(encoded)) {
    case PYCC_EXT_INT_FALSE:
        *out = 0;
        return 0;
    case PYCC_EXT_INT_TRUE:
        *out = 1;
        return 0;
    default:
        PyErr_SetString(PyExc_IndexError, "cannot fit 'int' into an index-sized integer");
        return -1;
    }
}

/* A stored value outside `range(0, 256)`. Always -1. */
int pycc_ext_ba_byte_error(void)
{
    PyErr_SetString(PyExc_ValueError, "byte must be in range(0, 256)");
    return -1;
}

/*
 * The byte an `int` word stores into a `bytearray`, as `bytearray.append`
 * and `bytearray.extend` take it: 0 with `*out` set, or -1 with CPython's
 * `ValueError` set when the value is outside `range(0, 256)` (every
 * big int is).
 */
static int pycc_ext_ba_byte_of_word(long long encoded, char *out)
{
    int value = pycc_rt_ext_byte_of_word(encoded);

    if (value < 0) {
        return pycc_ext_ba_byte_error();
    }
    *out = (char)value;
    return 0;
}

/*
 * `b.append(x)` on a byte buffer (`pycc_hir::builtins::BYTEARRAY_APPEND`):
 * the `int` word `encoded` stored at the end of the `bytearray` `ba`
 * (borrowed). `None`, after bridging the exception on a failure.
 */
PyObject *pycc_ext_obj_ba_append(PyObject *ba, long long encoded)
{
    char byte;
    Py_ssize_t size;

    if (pycc_ext_ba_byte_of_word(encoded, &byte) != 0) {
        return pycc_ext_stdlib_failed();
    }
    size = PyByteArray_Size(ba);
    if (size < 0 || PyByteArray_Resize(ba, size + 1) != 0) {
        return pycc_ext_stdlib_failed();
    }
    PyByteArray_AsString(ba)[size] = byte;
    Py_INCREF(Py_None);
    return Py_None;
}

/*
 * `b.extend(xs)` on a byte buffer (`pycc_hir::builtins::BYTEARRAY_EXTEND`)
 * with a native list of `int`s (borrowed): the `bytearray` grows, takes the
 * items' bytes, and shrinks back when an item is outside `range(0, 256)`,
 * so a failing item leaves it unchanged, as CPython's own `extend` does.
 */
PyObject *pycc_ext_obj_ba_extend(PyObject *ba, void *list)
{
    long long len = pycc_rt_list_len(list);
    Py_ssize_t size = PyByteArray_Size(ba);

    if (size < 0 || PyByteArray_Resize(ba, size + (Py_ssize_t)len) != 0) {
        return pycc_ext_stdlib_failed();
    }
    if (pycc_rt_ext_list_bytes(list, (unsigned char *)PyByteArray_AsString(ba) + size) != 0) {
        /* Back to the old size: the bytes before `size` were not touched. */
        (void)PyByteArray_Resize(ba, size);
        (void)pycc_ext_ba_byte_error();
        return pycc_ext_stdlib_failed();
    }
    Py_INCREF(Py_None);
    return Py_None;
}
