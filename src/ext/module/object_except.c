/*
 * Non-strict mode: an `except` clause whose type is a CPython object
 * (`except struct.error:`, `except mod.Error as e:`). `pycc_hir` lowers
 * such a `try` to a clause catching `Exception` whose body tests the caught
 * pycc exception against the object (`pycc_ext_obj_exc_match`) and binds
 * the CPython exception it is (`pycc_ext_obj_exc_object`), re-raising it
 * when no clause matches. Part of the one shim translation unit
 * (`src/ext_build.rs`'s `SHIM_C`), after `bridge.c`, whose
 * `pycc_ext_raise_pending` builds the CPython exception.
 */

extern void pycc_rt_exception_incref(void *obj);

/*
 * The CPython exception the pycc exception `exc` (borrowed) stands for: the
 * original it was bridged from, else the exception the export boundary would
 * raise for it (`pycc_ext_raise_pending`), which becomes its original from
 * then on, so it is built once and an escaping `exc` re-raises that very
 * object. Borrowed from `exc`; NULL with a CPython exception set on failure.
 * Must be called with no pycc exception pending (inside a handler).
 */
static PyObject *pycc_ext_exception_cpython(void *exc)
{
    PyObject *original = pycc_rt_exception_foreign(exc);
    PyObject *saved;
    PyObject *built;

    if (original != NULL) {
        pycc_ext_tb_sync(exc, original);
        return original;
    }
    saved = PyErr_GetRaisedException();
    /* Pending with a reference of its own, which `pycc_ext_raise_pending`
     * releases when it clears the pending state. */
    pycc_rt_exception_incref(exc);
    pycc_rt_exception_raise(exc);
    pycc_ext_raise_pending();
    built = PyErr_GetRaisedException();
    PyErr_SetRaisedException(saved);
    if (built == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: an exception could not be translated");
        return NULL;
    }
    pycc_rt_exception_set_foreign(exc, built);
    return built;
}

/* Whether `cls` may be caught: an exception class, or a tuple of them
 * (CPython's `except` check). 1, or 0 with `TypeError` set. */
static int pycc_ext_catchable(PyObject *cls)
{
    Py_ssize_t i;

    if (PyTuple_Check(cls)) {
        for (i = 0; i < PyTuple_Size(cls); i++) {
            PyObject *item = PyTuple_GetItem(cls, i);
            if (!(PyType_Check(item) && PyType_IsSubtype((PyTypeObject *)item,
                                                         (PyTypeObject *)PyExc_BaseException))) {
                goto refuse;
            }
        }
        return 1;
    }
    if (PyType_Check(cls) && PyType_IsSubtype((PyTypeObject *)cls,
                                              (PyTypeObject *)PyExc_BaseException)) {
        return 1;
    }
refuse:
    PyErr_SetString(PyExc_TypeError,
                    "catching classes that do not inherit from BaseException is not allowed");
    return 0;
}

/* Whether the caught pycc exception `exc` matches the `except` clause type
 * `cls` (both borrowed). A failure is bridged into pycc's pending exception
 * (the `except` clause's own `TypeError` replaces the caught one, as in
 * CPython). */
signed char pycc_ext_obj_exc_match(void *exc, PyObject *cls)
{
    PyObject *e;

    if (!pycc_ext_catchable(cls)) {
        pycc_ext_obj_error_bridge();
        return 0;
    }
    e = pycc_ext_exception_cpython(exc);
    if (e == NULL) {
        pycc_ext_obj_error_bridge();
        return 0;
    }
    return (signed char)PyErr_GivenExceptionMatches(e, cls);
}

/* The caught pycc exception `exc` (borrowed) as the CPython exception it
 * is: a new reference, what an `except T as e:` clause binds. */
PyObject *pycc_ext_obj_exc_object(void *exc)
{
    PyObject *e = pycc_ext_exception_cpython(exc);

    if (e == NULL) {
        pycc_ext_obj_error_bridge();
        Py_RETURN_NONE;
    }
    Py_INCREF(e);
    return e;
}
