/* `copy.copy` of a compiled instance (#1455), on the carrier types of
 * carriers.c. */

/*
 * #1455: `copy.copy(x)` of a carrier. CPython's `copy.copy` calls a type's
 * `__copy__` before any reduce protocol, so this is the whole copy: a new
 * carrier of `type(x)` wrapping a clone of `x`'s compiled instance. The
 * generated table, keyed by the instance's run-time class, decides whether
 * the class copies at all -- a class whose namespace takes part in the copy
 * protocol, or a PEP 695 template whose specializations share one layout
 * name, is refused with a `TypeError` naming why -- and hands the clone one
 * kind byte per slot.
 *
 * A never-initialized carrier (`mod.C.__new__(mod.C)`, `inst` NULL) has no
 * instance to read a class from; only a published type can produce one, and
 * its name is its class name, so the table is consulted by that name and an
 * accepted copy is another never-initialized carrier.
 *
 * The original is not touched: its reference count is unchanged, as in
 * CPython. The clone takes its own reference to every slot value, by the
 * slot's kind (`pycc_rt_ext_instance_copy`), `o` slots included.
 */
static PyObject *pycc_ext_copy_dispatch = NULL;
static PyObject *pycc_ext_copy_reconstruct = NULL;

/* v0.10: the names whose definition in a Python subclass changes what
 * `copy.copy` of the source class does (it reduces the object): such a
 * copy goes through the reduce protocol rather than the clone. */
static const char *const pycc_ext_copy_hooks[] = {
    "__reduce__", "__reduce_ex__", "__getstate__", "__setstate__", "__getnewargs__",
    "__getnewargs_ex__", "__new__", "__setattr__", "__getattribute__", NULL,
};

/* Whether a Python class of `type`'s MRO (but `object`, wherever it sits)
 * defines a copy hook or non-empty `__slots__` (-1 on error). */
static int pycc_ext_copy_hooked(PyObject *type)
{
    PyObject *mro = PyObject_GetAttrString(type, "__mro__");
    Py_ssize_t n;
    Py_ssize_t i;
    int hooked = 0;
    if (mro == NULL) {
        return -1;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n && hooked == 0; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        PyObject *dict;
        PyObject *declared;
        const char *const *hook;
        if (pycc_ext_is_compiled_type(c) || c == (PyObject *)&PyBaseObject_Type) {
            continue;
        }
        dict = PyObject_GetAttrString(c, "__dict__");
        if (dict == NULL) {
            hooked = -1;
            break;
        }
        for (hook = pycc_ext_copy_hooks; *hook != NULL && hooked == 0; hook++) {
            hooked = PyMapping_HasKeyStringWithError(dict, *hook);
        }
        if (hooked == 0) {
            declared = PyMapping_GetItemString(dict, "__slots__");
            if (declared == NULL) {
                if (PyErr_ExceptionMatches(PyExc_KeyError)) {
                    PyErr_Clear();
                } else {
                    hooked = -1;
                }
            } else {
                hooked = PyObject_IsTrue(declared);
                Py_DECREF(declared);
            }
        }
        Py_DECREF(dict);
    }
    Py_DECREF(mro);
    return hooked;
}

/* `copy.copy` as the source class gets it without a `__copy__`: the
 * reduction `reductor(x)` (from `copyreg.dispatch_table`) or
 * `x.__reduce_ex__(4)`, rebuilt by `copy._reconstruct`. */
static PyObject *pycc_ext_copy_by_reduce(PyObject *self, PyObject *reductor)
{
    PyObject *rv;
    PyObject *args;
    PyObject *result;
    Py_ssize_t n;
    Py_ssize_t i;
    if (pycc_ext_copy_reconstruct == NULL) {
        PyObject *copy = PyImport_ImportModule("copy");
        if (copy == NULL) {
            return NULL;
        }
        pycc_ext_copy_reconstruct = PyObject_GetAttrString(copy, "_reconstruct");
        Py_DECREF(copy);
        if (pycc_ext_copy_reconstruct == NULL) {
            return NULL;
        }
    }
    rv = reductor != NULL ? PyObject_CallFunctionObjArgs(reductor, self, NULL)
                          : PyObject_CallMethod(self, "__reduce_ex__", "i", 4);
    if (rv == NULL) {
        return NULL;
    }
    if (PyUnicode_Check(rv)) {
        Py_DECREF(rv);
        return Py_NewRef(self);
    }
    if (!PyTuple_Check(rv)) {
        Py_DECREF(rv);
        PyErr_SetString(PyExc_TypeError, "__reduce_ex__ must return a string or tuple");
        return NULL;
    }
    n = PyTuple_Size(rv);
    args = PyTuple_New(n + 2);
    if (args == NULL) {
        Py_DECREF(rv);
        return NULL;
    }
    PyTuple_SetItem(args, 0, Py_NewRef(self));
    PyTuple_SetItem(args, 1, Py_NewRef(Py_None));
    for (i = 0; i < n; i++) {
        PyTuple_SetItem(args, i + 2, Py_NewRef(PyTuple_GetItem(rv, i)));
    }
    Py_DECREF(rv);
    result = PyObject_Call(pycc_ext_copy_reconstruct, args, NULL);
    Py_DECREF(args);
    return result;
}

/* The reduction `copy.copy` would use instead of the clone: the type's
 * `copyreg.dispatch_table` entry, or (in `*by_reduce`) the object's own
 * reduce protocol when a Python subclass hooks it. */
static int pycc_ext_copy_route(PyObject *self, PyObject **reductor, int *by_reduce)
{
    PyObject *type = (PyObject *)Py_TYPE(self);
    *reductor = NULL;
    *by_reduce = 0;
    if (pycc_ext_copy_dispatch == NULL) {
        PyObject *copyreg = PyImport_ImportModule("copyreg");
        if (copyreg == NULL) {
            return -1;
        }
        pycc_ext_copy_dispatch = PyObject_GetAttrString(copyreg, "dispatch_table");
        Py_DECREF(copyreg);
        if (pycc_ext_copy_dispatch == NULL) {
            return -1;
        }
    }
    *reductor = PyObject_GetItem(pycc_ext_copy_dispatch, type);
    if (*reductor != NULL) {
        *by_reduce = 1;
        return 0;
    }
    if (!PyErr_ExceptionMatches(PyExc_KeyError)) {
        return -1;
    }
    PyErr_Clear();
    if (!pycc_ext_is_compiled_type(type)) {
        int hooked = pycc_ext_copy_hooked(type);
        if (hooked < 0) {
            return -1;
        }
        *by_reduce = hooked;
    }
    return 0;
}

static PyObject *pycc_ext_instance_copy_default(PyccExtModule *m, PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst = ((PyccExtInstance *)self)->inst;
    {
        PyObject *reductor;
        int by_reduce;
        if (pycc_ext_copy_route(self, &reductor, &by_reduce) < 0) {
            return NULL;
        }
        if (by_reduce) {
            PyObject *copied = pycc_ext_copy_by_reduce(self, reductor);
            Py_XDECREF(reductor);
            return copied;
        }
    }
    const unsigned char *cls;
    size_t len = 0;
    const char *kinds = NULL;
    size_t nkinds = 0;
    const char *refused = NULL;
    PyObject *name = NULL;
    void *clone = NULL;
    allocfunc tp_alloc;
    PyObject *carrier;
    int found;

    if (inst == NULL) {
        Py_ssize_t name_len = 0;
        name = PyType_GetName(tp);
        if (name == NULL) {
            return NULL;
        }
        cls = (const unsigned char *)PyUnicode_AsUTF8AndSize(name, &name_len);
        if (cls == NULL) {
            Py_DECREF(name);
            return NULL;
        }
        len = (size_t)name_len;
    }
    else {
        cls = pycc_rt_ext_instance_class(inst, &len);
    }
    found = m->carrier_class_copy_kinds(cls, len, &kinds, &nkinds, &refused);
    Py_XDECREF(name);
    if (found < 0) {
        PyErr_SetString(PyExc_SystemError, "pycc: no copy table entry for a carrier's class");
        return NULL;
    }
    if (found == 0) {
        PyObject *qualified = PyType_GetFullyQualifiedName(tp);
        if (qualified != NULL) {
            PyErr_Format(PyExc_TypeError, "cannot copy '%U' object: %s", qualified, refused);
            Py_DECREF(qualified);
        }
        return NULL;
    }
    if (inst != NULL) {
        clone = pycc_rt_ext_instance_copy(inst, kinds, nkinds);
        if (clone == NULL) {
            PyErr_SetString(PyExc_SystemError,
                            "pycc: copy kind table does not match the instance layout");
            return NULL;
        }
    }
    tp_alloc = (allocfunc)PyType_GetSlot(tp, Py_tp_alloc);
    carrier = tp_alloc(tp, 0);
    if (carrier == NULL) {
        return NULL;
    }
    ((PyccExtInstance *)carrier)->inst = clone;
    if (pycc_ext_is_compiled_class((PyObject *)tp)) {
        if (clone != NULL) {
            pycc_rt_ext_instance_set_carrier(clone, carrier);
        }
        if (PyType_IS_GC(tp)) {
            PyObject_GC_UnTrack(carrier);
        }
        return carrier;
    }
    /* v0.9: a Python subclass's object: the clone is linked strongly, as
     * `pycc_ext_carrier_bind` links one, and gets a shallow copy of the
     * object's `__dict__`, as `copy.copy` of a Python object does. */
    if (clone != NULL) {
        Py_INCREF(carrier);
        pycc_rt_ext_instance_set_carrier_strong(clone, carrier);
    }
    {
        PyObject *dict = PyObject_GetAttrString(self, "__dict__");
        PyObject *copied = dict == NULL ? NULL : PyDict_Copy(dict);
        Py_XDECREF(dict);
        if (copied == NULL) {
            PyErr_Clear();
        }
        else {
            int failed = PyObject_SetAttrString(carrier, "__dict__", copied);
            Py_DECREF(copied);
            if (failed < 0) {
                Py_DECREF(carrier);
                return NULL;
            }
        }
    }
    return carrier;
}

static PyObject *pycc_ext_instance_copy_in(PyccExtModule *m, PyObject *self)
{
    PyObject *hook = pycc_ext_object_position_hook(self, "__copy__");
    if (hook != NULL) {
        PyObject *result = PyObject_CallNoArgs(hook);
        Py_DECREF(hook);
        return result;
    }
    return PyErr_Occurred() ? NULL : pycc_ext_instance_copy_default(m, self);
}

