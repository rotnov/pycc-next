/*
 * Typed numpy arrays (`pycc_hir::ndarray`): the run-time check that an
 * object is a `numpy.ndarray` of the static dtype and dimension count, and
 * the cold paths of compiled element access (each raises numpy's own
 * exception and answers -1, the shape `foreign_fail.rs` routes).
 *
 * Compiled code reads an array's data pointer, dimension count, shape,
 * strides and flags straight out of the array object, at the offsets of
 * numpy's `PyArrayObject_fields` (numpy's own `PyArray_DATA`, `PyArray_DIMS`,
 * ... are the same field reads). The layout is the same in numpy 1.x and
 * 2.x; `pycc_nd_validate` still confirms it against the array's Python-level
 * attributes the first time an array is checked, and refuses to run on any
 * other layout, so an incompatible numpy is a clean `RuntimeError` instead
 * of a wild read. The offsets below must match `crates/pycc_codegen/src/
 * ndarray/mod.rs`'s.
 *
 * `pycc_ext_nd_index_error`, `pycc_ext_nd_index_word`,
 * `pycc_ext_nd_int_word` and `pycc_ext_nd_readonly` must never run CPython
 * code (no attribute access, no `__index__`, no GC-tracked allocation):
 * compiled code keeps its array header snapshots across calls to them
 * (`PYTHON_FREE_CALLS`, `crates/pycc_codegen/src/ndarray/fresh.rs`).
 * Neither may a successful `pycc_ext_nd_check` once `pycc_nd_layout_ok` is
 * set: until then it imports numpy and reads Python attributes, after it a
 * success is `PyObject_TypeCheck`, raw field reads and numpy's C dtype
 * comparison. Its failures may run CPython code (formatting the dtype);
 * compiled code marks every snapshot stale on that path.
 */

#define PYCC_ND_HEAD ((Py_ssize_t)sizeof(PyObject))
#define PYCC_ND_DATA(o) (*(char **)((char *)(o) + PYCC_ND_HEAD))
#define PYCC_ND_NDIM(o) (*(int *)((char *)(o) + PYCC_ND_HEAD + 8))
#define PYCC_ND_DIMS(o) (*(Py_ssize_t **)((char *)(o) + PYCC_ND_HEAD + 16))
#define PYCC_ND_STRIDES(o) (*(Py_ssize_t **)((char *)(o) + PYCC_ND_HEAD + 24))
#define PYCC_ND_DESCR(o) (*(PyObject **)((char *)(o) + PYCC_ND_HEAD + 40))
#define PYCC_ND_FLAGS(o) (*(int *)((char *)(o) + PYCC_ND_HEAD + 48))
#define PYCC_ND_WRITEABLE 0x0400

/* `numpy.ndarray` and the three canonical dtypes, by `NdDtype::code`.
 * Strong references held for the life of the process. */
static PyTypeObject *pycc_nd_type = NULL;
static PyObject *pycc_nd_dtypes[3] = {NULL, NULL, NULL};
static const char *const pycc_nd_dtype_names[3] = {"float64", "int64", "bool"};
static int pycc_nd_layout_ok = 0;

/* `numpy.ndarray` and the float64, int64 and bool dtypes again, for the
 * inline success path of compiled code's check (`crates/pycc_codegen/src/
 * ndarray/mod.rs`, `emit_check`): set only once the layout is validated, so
 * while it is all null every check takes `pycc_ext_nd_check`. */
PyObject *pycc_ext_nd_canon[4] = {NULL, NULL, NULL, NULL};

static int pycc_nd_init(void)
{
    PyObject *numpy;
    PyObject *dtype_fn;
    int i;

    if (pycc_nd_type != NULL) {
        return 0;
    }
    if (PYCC_ND_HEAD != 16) {
        PyErr_SetString(PyExc_RuntimeError,
                        "pycc: typed numpy arrays need a 64-bit CPython object header");
        return -1;
    }
    numpy = PyImport_ImportModule("numpy");
    if (numpy == NULL) {
        return -1;
    }
    dtype_fn = PyObject_GetAttrString(numpy, "dtype");
    if (dtype_fn == NULL) {
        Py_DECREF(numpy);
        return -1;
    }
    for (i = 0; i < 3; i++) {
        pycc_nd_dtypes[i] = PyObject_CallFunction(dtype_fn, "s", pycc_nd_dtype_names[i]);
        if (pycc_nd_dtypes[i] == NULL) {
            Py_DECREF(dtype_fn);
            Py_DECREF(numpy);
            return -1;
        }
    }
    Py_DECREF(dtype_fn);
    pycc_nd_type = (PyTypeObject *)PyObject_GetAttrString(numpy, "ndarray");
    Py_DECREF(numpy);
    if (pycc_nd_type == NULL) {
        return -1;
    }
    if (!PyType_Check((PyObject *)pycc_nd_type)) {
        Py_CLEAR(pycc_nd_type);
        PyErr_SetString(PyExc_RuntimeError, "pycc: numpy.ndarray is not a type");
        return -1;
    }
    return 0;
}

/* Compares one `long long`-valued attribute path with the raw field. */
static int pycc_nd_attr_equals(PyObject *o, const char *attr, long long expected)
{
    PyObject *value = PyObject_GetAttrString(o, attr);
    long long actual;

    if (value == NULL) {
        return -1;
    }
    actual = PyLong_AsLongLong(value);
    Py_DECREF(value);
    if (actual == -1 && PyErr_Occurred()) {
        return -1;
    }
    return actual == expected;
}

/* Compares a tuple-valued attribute (`shape`, `strides`) with a raw array. */
static int pycc_nd_tuple_equals(PyObject *o, const char *attr, const Py_ssize_t *raw, int n)
{
    PyObject *value = PyObject_GetAttrString(o, attr);
    int i;
    int equal = 1;

    if (value == NULL) {
        return -1;
    }
    if (!PyTuple_Check(value) || PyTuple_Size(value) != n) {
        Py_DECREF(value);
        return 0;
    }
    for (i = 0; i < n && equal; i++) {
        long long item = PyLong_AsLongLong(PyTuple_GetItem(value, i));
        if (item == -1 && PyErr_Occurred()) {
            Py_DECREF(value);
            return -1;
        }
        equal = item == (long long)raw[i];
    }
    Py_DECREF(value);
    return equal;
}

/* Confirms, once, that `o`'s raw fields agree with its Python attributes. */
static int pycc_nd_validate(PyObject *o)
{
    PyObject *iface;
    PyObject *data;
    PyObject *dtype;
    PyObject *flags;
    PyObject *writeable;
    int ok;
    int nd = PYCC_ND_NDIM(o);

    ok = nd >= 0 && nd <= 64;
    if (ok) {
        ok = pycc_nd_attr_equals(o, "ndim", nd);
    }
    if (ok == 1) {
        ok = pycc_nd_tuple_equals(o, "shape", PYCC_ND_DIMS(o), nd);
    }
    if (ok == 1) {
        ok = pycc_nd_tuple_equals(o, "strides", PYCC_ND_STRIDES(o), nd);
    }
    if (ok == 1) {
        iface = PyObject_GetAttrString(o, "__array_interface__");
        if (iface == NULL) {
            return -1;
        }
        data = PyDict_Check(iface) ? PyDict_GetItemString(iface, "data") : NULL;
        ok = data != NULL && PyTuple_Check(data) && PyTuple_Size(data) >= 1;
        if (ok) {
            ok = PyLong_AsVoidPtr(PyTuple_GetItem(data, 0)) == (void *)PYCC_ND_DATA(o);
        }
        Py_DECREF(iface);
        if (PyErr_Occurred()) {
            return -1;
        }
    }
    if (ok == 1) {
        dtype = PyObject_GetAttrString(o, "dtype");
        if (dtype == NULL) {
            return -1;
        }
        ok = dtype == PYCC_ND_DESCR(o);
        Py_DECREF(dtype);
    }
    if (ok == 1) {
        flags = PyObject_GetAttrString(o, "flags");
        if (flags == NULL) {
            return -1;
        }
        writeable = PyObject_GetAttrString(flags, "writeable");
        Py_DECREF(flags);
        if (writeable == NULL) {
            return -1;
        }
        ok = (writeable == Py_True) == ((PYCC_ND_FLAGS(o) & PYCC_ND_WRITEABLE) != 0);
        Py_DECREF(writeable);
    }
    if (ok < 0) {
        return -1;
    }
    if (!ok) {
        PyErr_SetString(PyExc_RuntimeError,
                        "pycc: this numpy's array layout is not the one compiled code reads");
        return -1;
    }
    pycc_nd_layout_ok = 1;
    pycc_ext_nd_canon[1] = pycc_nd_dtypes[0];
    pycc_ext_nd_canon[2] = pycc_nd_dtypes[1];
    pycc_ext_nd_canon[3] = pycc_nd_dtypes[2];
    pycc_ext_nd_canon[0] = (PyObject *)pycc_nd_type;
    return 0;
}

/*
 * Checks that `o` is a `numpy.ndarray` of dtype `code` (`NdDtype::code`:
 * 0 float64, 1 int64, 2 bool) with `ndim` dimensions. 0, or -1 with
 * `TypeError` (or the import's own error) set.
 */
int pycc_ext_nd_check(PyObject *o, long long code, long long ndim)
{
    const char *dtype_name;
    PyObject *descr;
    int equal;

    if (o == NULL || code < 0 || code > 2) {
        PyErr_SetString(PyExc_SystemError, "pycc: bad typed array check");
        return -1;
    }
    if (pycc_nd_init() < 0) {
        return -1;
    }
    dtype_name = pycc_nd_dtype_names[code];
    if (!PyObject_TypeCheck(o, pycc_nd_type)) {
        PyObject *type_name = PyType_GetName(Py_TYPE(o));
        if (type_name == NULL) {
            return -1;
        }
        PyErr_Format(PyExc_TypeError, "expected numpy.ndarray[%s, %lld-D], got %U", dtype_name,
                     ndim, type_name);
        Py_DECREF(type_name);
        return -1;
    }
    if (!pycc_nd_layout_ok && pycc_nd_validate(o) < 0) {
        return -1;
    }
    if (PYCC_ND_NDIM(o) != ndim) {
        PyErr_Format(PyExc_TypeError, "expected numpy.ndarray[%s, %lld-D], got a %d-D array",
                     dtype_name, ndim, PYCC_ND_NDIM(o));
        return -1;
    }
    descr = PYCC_ND_DESCR(o);
    if (descr == pycc_nd_dtypes[code]) {
        return 0;
    }
    equal = PyObject_RichCompareBool(descr, pycc_nd_dtypes[code], Py_EQ);
    if (equal < 0) {
        return -1;
    }
    if (!equal) {
        PyErr_Format(PyExc_TypeError, "expected numpy.ndarray[%s, %lld-D], got an array of dtype %S",
                     dtype_name, ndim, descr);
        return -1;
    }
    return 0;
}

/* numpy's out-of-range index error. Always -1. */
int pycc_ext_nd_index_error(long long index, long long axis, long long size)
{
    PyErr_Format(PyExc_IndexError, "index %lld is out of bounds for axis %lld with size %lld",
                 index, axis, size);
    return -1;
}

/*
 * The machine index of an `int` word that is not a smallint: a bool marker
 * indexes 0 or 1; a bigint is not an index numpy accepts. 0, or -1 with
 * `IndexError` set.
 */
int pycc_ext_nd_index_word(long long encoded, long long *out)
{
    switch (pycc_rt_ext_int_classify(encoded)) {
    case PYCC_EXT_INT_FALSE:
        *out = 0;
        return 0;
    case PYCC_EXT_INT_TRUE:
        *out = 1;
        return 0;
    default:
        PyErr_SetString(PyExc_IndexError,
                        "only integers, slices (`:`), ellipsis (`...`), numpy.newaxis (`None`) and "
                        "integer or boolean arrays are valid indices");
        return -1;
    }
}

/*
 * The `int64` value of an `int` word that is not a smallint, for a store
 * into an `int64` array. 0, or -1 with numpy's `OverflowError` set.
 */
int pycc_ext_nd_int_word(long long encoded, long long *out)
{
    PyObject *value;
    long long v;

    switch (pycc_rt_ext_int_classify(encoded)) {
    case PYCC_EXT_INT_SMALLINT:
        *out = pycc_rt_ext_int_decode(encoded);
        return 0;
    case PYCC_EXT_INT_FALSE:
        *out = 0;
        return 0;
    case PYCC_EXT_INT_TRUE:
        *out = 1;
        return 0;
    default:
        value = pycc_ext_bigint_pack(encoded);
        if (value == NULL) {
            return -1;
        }
        v = PyLong_AsLongLong(value);
        Py_DECREF(value);
        if (v == -1 && PyErr_Occurred()) {
            PyErr_Clear();
            PyErr_SetString(PyExc_OverflowError, "Python int too large to convert to C long");
            return -1;
        }
        *out = v;
        return 0;
    }
}

/* A store into a read-only array. Always -1. */
int pycc_ext_nd_readonly(void)
{
    PyErr_SetString(PyExc_ValueError, "assignment destination is read-only");
    return -1;
}

/*
 * A store into a typed array attribute of an `--ext` class from CPython
 * (`src/ext_build/getset/setter.rs`): the value must pass the check
 * compiled code applies to its own stores. Then it is unpacked as any
 * object is, taking a new reference. 0, or -1 with `TypeError` set.
 */
static int pycc_ext_unpack_ndarray(PyObject *value, const char *fn_name, long long code,
                                   long long ndim, void **out)
{
    (void)fn_name;
    if (pycc_ext_nd_check(value, code, ndim) != 0) {
        return -1;
    }
    *out = Py_NewRef(value);
    return 0;
}
