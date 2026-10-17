/*
 * A call on a CPython object that unpacks an argument -- `f(*xs)`,
 * `o.m(a, *xs, b)`, `f(**kw)`, `f(k=v, **kw)` (`pycc_hir`'s `STAR_ARGS`
 * markers). `pycc_ext_obj_call_kw` hands such a call here: the arguments
 * are bound at run time into a tuple and a dict, in source order and with
 * CPython's own checks and messages (`ceval.c`'s `CALL_FUNCTION_EX`,
 * `LIST_EXTEND` and `DICT_MERGE`), then `PyObject_Call` makes the call.
 * Part of the one shim translation unit (`src/ext_build.rs`'s `SHIM_C`),
 * before `object_ops.c`.
 */

/* Whether keyword name `name` is an unpacking marker: `*` (extend the
 * positional arguments with an iterable), `+` (one positional argument
 * after an unpacking) or `**` (merge a mapping into the keywords). */
static int pycc_ext_is_unpacking(const char *name)
{
    return strcmp(name, "*") == 0 || strcmp(name, "+") == 0 || strcmp(name, "**") == 0;
}

/* How CPython names a callable in a call's `TypeError`
 * (`_PyObject_FunctionStr`): `module.qualname()`, or `qualname()` for a
 * builtin, or `str(callable)` without a qualified name. A new reference, or
 * NULL with an exception set. */
static PyObject *pycc_ext_function_str(PyObject *callable)
{
    PyObject *qualname = PyObject_GetAttrString(callable, "__qualname__");
    PyObject *module;
    PyObject *result;

    if (qualname == NULL) {
        PyErr_Clear();
        return PyObject_Str(callable);
    }
    module = PyObject_GetAttrString(callable, "__module__");
    if (module == NULL) {
        PyErr_Clear();
    }
    if (module != NULL && module != Py_None && PyUnicode_Check(module)
        && PyUnicode_CompareWithASCIIString(module, "builtins") != 0) {
        result = PyUnicode_FromFormat("%U.%U()", module, qualname);
    } else {
        result = PyUnicode_FromFormat("%U()", qualname);
    }
    Py_XDECREF(module);
    Py_DECREF(qualname);
    return result;
}

/* Raises `TypeError(f"{callable} {format % type(o).__name__}")`. */
static void pycc_ext_star_error(PyObject *callable, const char *what, PyObject *o)
{
    PyObject *name = pycc_ext_function_str(callable);
    PyObject *type_name = PyType_GetName(Py_TYPE(o));

    if (name != NULL && type_name != NULL) {
        PyErr_Format(PyExc_TypeError, "%U argument after %s, not %U", name, what, type_name);
    }
    Py_XDECREF(name);
    Py_XDECREF(type_name);
}

/* Appends every item of `iterable` to `list` (`*iterable`). 0, or -1 with an
 * exception set. `alone` when the unpacking is the call's only positional
 * argument: CPython then names the callable in its `TypeError`
 * (`CALL_FUNCTION_EX`), and otherwise does not (`LIST_EXTEND`). */
static int pycc_ext_star_extend(PyObject *list, PyObject *callable, PyObject *iterable,
                                  int alone)
{
    PyObject *iterator = PyObject_GetIter(iterable);
    PyObject *item;
    PyObject *type_name;

    if (iterator == NULL) {
        if (PyErr_ExceptionMatches(PyExc_TypeError)
            && PyType_GetSlot(Py_TYPE(iterable), Py_tp_iter) == NULL
            && !PySequence_Check(iterable)) {
            PyErr_Clear();
            if (alone) {
                pycc_ext_star_error(callable, "* must be an iterable", iterable);
            } else {
                type_name = PyType_GetName(Py_TYPE(iterable));
                if (type_name != NULL) {
                    PyErr_Format(PyExc_TypeError, "Value after * must be an iterable, not %U",
                                 type_name);
                    Py_DECREF(type_name);
                }
            }
        }
        return -1;
    }
    while ((item = PyIter_Next(iterator)) != NULL) {
        int failed = PyList_Append(list, item);
        Py_DECREF(item);
        if (failed < 0) {
            Py_DECREF(iterator);
            return -1;
        }
    }
    Py_DECREF(iterator);
    return PyErr_Occurred() ? -1 : 0;
}

/* Stores `kwargs[key] = value`, refusing a key already there as CPython
 * does. 0, or -1 with an exception set. */
static int pycc_ext_star_keyword(PyObject *kwargs, PyObject *callable, PyObject *key,
                                   PyObject *value)
{
    int present = PyDict_Contains(kwargs, key);
    PyObject *name;

    if (present < 0) {
        return -1;
    }
    if (present) {
        name = pycc_ext_function_str(callable);
        if (name != NULL) {
            PyErr_Format(PyExc_TypeError, "%U got multiple values for keyword argument '%S'",
                         name, key);
            Py_DECREF(name);
        }
        return -1;
    }
    return PyDict_SetItem(kwargs, key, value);
}

/* Merges `mapping` into `kwargs` (`**mapping`). 0, or -1 with an exception
 * set. */
static int pycc_ext_star_merge(PyObject *kwargs, PyObject *callable, PyObject *mapping)
{
    PyObject *keys;
    PyObject *iterator;
    PyObject *key;

    if (PyDict_Check(mapping)) {
        keys = PyDict_Keys(mapping);
    } else {
        keys = PyObject_CallMethod(mapping, "keys", NULL);
        if (keys == NULL && PyErr_ExceptionMatches(PyExc_AttributeError)) {
            PyErr_Clear();
            pycc_ext_star_error(callable, "** must be a mapping", mapping);
            return -1;
        }
    }
    if (keys == NULL) {
        return -1;
    }
    iterator = PyObject_GetIter(keys);
    Py_DECREF(keys);
    if (iterator == NULL) {
        return -1;
    }
    while ((key = PyIter_Next(iterator)) != NULL) {
        PyObject *value = PyObject_GetItem(mapping, key);
        int failed = value == NULL ? -1 : pycc_ext_star_keyword(kwargs, callable, key, value);
        Py_XDECREF(value);
        Py_DECREF(key);
        if (failed < 0) {
            Py_DECREF(iterator);
            return -1;
        }
    }
    Py_DECREF(iterator);
    return PyErr_Occurred() ? -1 : 0;
}

/* `callable(args[0..nargs], <names[i]: args[nargs + i]>...)` with unpacking
 * markers among the names (file comment). Borrows everything; returns a new
 * reference or NULL with an exception set. */
static PyObject *pycc_ext_call_unpacking(PyObject *callable, PyObject **args, long long nargs,
                                         const char **names, long long nkw)
{
    PyObject *positional = PyList_New(0);
    PyObject *kwargs = PyDict_New();
    PyObject *tuple = NULL;
    PyObject *result = NULL;
    long long i;
    long long positionals = nargs;
    int failed = positional == NULL || kwargs == NULL;

    for (i = 0; i < nkw; i++) {
        if (strcmp(names[i], "*") == 0 || strcmp(names[i], "+") == 0) {
            positionals++;
        }
    }
    for (i = 0; !failed && i < nargs; i++) {
        failed = PyList_Append(positional, args[i]) < 0;
    }
    for (i = 0; !failed && i < nkw; i++) {
        PyObject *value = args[nargs + i];
        if (strcmp(names[i], "*") == 0) {
            failed = pycc_ext_star_extend(positional, callable, value, positionals == 1) < 0;
        } else if (strcmp(names[i], "+") == 0) {
            failed = PyList_Append(positional, value) < 0;
        } else if (strcmp(names[i], "**") == 0) {
            failed = pycc_ext_star_merge(kwargs, callable, value) < 0;
        } else {
            PyObject *key = PyUnicode_InternFromString(names[i]);
            failed = key == NULL || pycc_ext_star_keyword(kwargs, callable, key, value) < 0;
            Py_XDECREF(key);
        }
    }
    if (!failed) {
        tuple = PyList_AsTuple(positional);
    }
    if (tuple != NULL) {
        result = PyObject_Call(callable, tuple, PyDict_Size(kwargs) > 0 ? kwargs : NULL);
    }
    Py_XDECREF(tuple);
    Py_XDECREF(positional);
    Py_XDECREF(kwargs);
    return result;
}
