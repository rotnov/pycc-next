/*
 * Non-strict mode (#3): `sorted`/`list.sort`/`min`/`max` with `key=` or
 * `reverse=`, and the `heapq` and `bisect` functions, over a CPython object
 * (`pycc_hir::builtins`' object arms, `pycc_ast::desugar::keys` and
 * `desugar::stdlib`).
 *
 * `desugar::keys` has already built the `(key, element)` pairs, calling the
 * key function once per element in order, as CPython does. The keyed
 * helpers below order the pairs by their keys alone -- CPython's own stable
 * `list.sort` with `key=operator.itemgetter(0)`, which keeps equal keys in
 * their original order for `reverse=True` too -- or pick the first least
 * (greatest) key with CPython's own `<` (`>`) comparison, and keep the
 * elements. The `heapq`/`bisect` helpers call the real module functions.
 *
 * Every failure is bridged into pycc's pending exception
 * (`pycc_ext_obj_error_bridge`), which the generated code checks right after
 * the call; a helper with an object result then returns a new reference to
 * `None` that the program never sees. Arguments are borrowed.
 */

/* `None`, after bridging the pending CPython exception. */
static PyObject *pycc_ext_stdlib_failed(void)
{
    pycc_ext_obj_error_bridge();
    Py_INCREF(Py_None);
    return Py_None;
}

/* `pairs` (a fresh CPython list of `(key, element)` tuples) sorted in place
 * by key; 0, or -1 with a CPython exception set. */
static int pycc_ext_sort_pairs(PyObject *pairs, char reverse)
{
    PyObject *operator_module;
    PyObject *getter;
    PyObject *sort;
    PyObject *args;
    PyObject *kwargs;
    PyObject *result;

    operator_module = PyImport_ImportModule("operator");
    if (operator_module == NULL) {
        return -1;
    }
    getter = PyObject_CallMethod(operator_module, "itemgetter", "i", 0);
    Py_DECREF(operator_module);
    if (getter == NULL) {
        return -1;
    }
    kwargs = Py_BuildValue("{s:O,s:O}", "key", getter, "reverse", reverse ? Py_True : Py_False);
    Py_DECREF(getter);
    if (kwargs == NULL) {
        return -1;
    }
    sort = PyObject_GetAttrString(pairs, "sort");
    args = sort == NULL ? NULL : PyTuple_New(0);
    result = args == NULL ? NULL : PyObject_Call(sort, args, kwargs);
    Py_XDECREF(args);
    Py_XDECREF(sort);
    Py_DECREF(kwargs);
    if (result == NULL) {
        return -1;
    }
    Py_DECREF(result);
    return 0;
}

/* A new list of the second items of `pairs`, in order. */
static PyObject *pycc_ext_pair_elements(PyObject *pairs)
{
    Py_ssize_t n = PyList_Size(pairs);
    PyObject *out;
    Py_ssize_t i;

    if (n < 0) {
        return NULL;
    }
    out = PyList_New(n);
    for (i = 0; out != NULL && i < n; i++) {
        PyObject *element = PySequence_GetItem(PyList_GetItem(pairs, i), 1);
        if (element == NULL) {
            Py_CLEAR(out);
            break;
        }
        PyList_SetItem(out, i, element);
    }
    return out;
}

/* `sorted(xs, key=f, reverse=r)`: the elements of `pairs` ordered by key. */
PyObject *pycc_ext_obj_sorted_keyed(PyObject *pairs, char reverse)
{
    PyObject *out;

    if (pycc_ext_sort_pairs(pairs, reverse) < 0) {
        return pycc_ext_stdlib_failed();
    }
    out = pycc_ext_pair_elements(pairs);
    return out == NULL ? pycc_ext_stdlib_failed() : out;
}

/* `xs.sort(key=f, reverse=r)`: `xs`'s items replaced by the elements of
 * `pairs` ordered by key. */
void pycc_ext_obj_sort_keyed(PyObject *xs, PyObject *pairs, char reverse)
{
    PyObject *sorted;
    int status;

    if (!PyList_Check(xs)) {
        /* CPython's `AttributeError` for a receiver with no `sort`; any
         * other receiver's own `sort` cannot take the pairs. */
        PyObject *method = PyObject_GetAttrString(xs, "sort");
        if (method != NULL) {
            Py_DECREF(method);
            PyErr_SetString(PyExc_TypeError,
                            "pycc: sort(key=...) of a CPython object that is not a list");
        }
        pycc_ext_obj_error_bridge();
        return;
    }
    if (pycc_ext_sort_pairs(pairs, reverse) < 0) {
        pycc_ext_obj_error_bridge();
        return;
    }
    sorted = pycc_ext_pair_elements(pairs);
    if (sorted == NULL) {
        pycc_ext_obj_error_bridge();
        return;
    }
    status = PySequence_SetSlice(xs, 0, PY_SSIZE_T_MAX, sorted);
    Py_DECREF(sorted);
    if (status < 0) {
        pycc_ext_obj_error_bridge();
    }
}

/* `min(xs, key=f)` (`op` `Py_LT`) or `max(xs, key=f)` (`Py_GT`): the element
 * of the first pair whose key no later key beats, as CPython's `min_max`. */
static PyObject *pycc_ext_extreme_keyed(PyObject *pairs, int op, const char *name)
{
    Py_ssize_t n = PyList_Size(pairs);
    PyObject *best_key = NULL;
    PyObject *best = NULL;
    Py_ssize_t i;

    if (n < 0) {
        return pycc_ext_stdlib_failed();
    }
    if (n == 0) {
        PyErr_Format(PyExc_ValueError, "%s() iterable argument is empty", name);
        return pycc_ext_stdlib_failed();
    }
    for (i = 0; i < n; i++) {
        PyObject *pair = PyList_GetItem(pairs, i);
        PyObject *key = PySequence_GetItem(pair, 0);
        int better;

        if (key == NULL) {
            goto fail;
        }
        if (best_key == NULL) {
            better = 1;
        } else {
            better = PyObject_RichCompareBool(key, best_key, op);
            if (better < 0) {
                Py_DECREF(key);
                goto fail;
            }
        }
        if (better) {
            PyObject *element = PySequence_GetItem(pair, 1);
            if (element == NULL) {
                Py_DECREF(key);
                goto fail;
            }
            Py_XDECREF(best_key);
            Py_XDECREF(best);
            best_key = key;
            best = element;
        } else {
            Py_DECREF(key);
        }
    }
    Py_DECREF(best_key);
    return best;
fail:
    Py_XDECREF(best_key);
    Py_XDECREF(best);
    return pycc_ext_stdlib_failed();
}

PyObject *pycc_ext_obj_min_keyed(PyObject *pairs)
{
    return pycc_ext_extreme_keyed(pairs, Py_LT, "min");
}

PyObject *pycc_ext_obj_max_keyed(PyObject *pairs)
{
    return pycc_ext_extreme_keyed(pairs, Py_GT, "max");
}

/* `module.function(*args)`, the result a new reference (`None` after a
 * bridged failure). */
static PyObject *pycc_ext_stdlib_call(const char *module_name, const char *function,
                                      PyObject *a, PyObject *b)
{
    PyObject *module = PyImport_ImportModule(module_name);
    PyObject *result;

    if (module == NULL) {
        return pycc_ext_stdlib_failed();
    }
    result = b == NULL ? PyObject_CallMethod(module, function, "O", a)
                       : PyObject_CallMethod(module, function, "OO", a, b);
    Py_DECREF(module);
    return result == NULL ? pycc_ext_stdlib_failed() : result;
}

PyObject *pycc_ext_obj_heapify(PyObject *h)
{
    return pycc_ext_stdlib_call("heapq", "heapify", h, NULL);
}

PyObject *pycc_ext_obj_heappop(PyObject *h)
{
    return pycc_ext_stdlib_call("heapq", "heappop", h, NULL);
}

PyObject *pycc_ext_obj_heappush(PyObject *h, PyObject *item)
{
    return pycc_ext_stdlib_call("heapq", "heappush", h, item);
}

PyObject *pycc_ext_obj_heapreplace(PyObject *h, PyObject *item)
{
    return pycc_ext_stdlib_call("heapq", "heapreplace", h, item);
}

PyObject *pycc_ext_obj_heappushpop(PyObject *h, PyObject *item)
{
    return pycc_ext_stdlib_call("heapq", "heappushpop", h, item);
}

PyObject *pycc_ext_obj_bisect_left(PyObject *a, PyObject *x)
{
    return pycc_ext_stdlib_call("bisect", "bisect_left", a, x);
}

PyObject *pycc_ext_obj_bisect_right(PyObject *a, PyObject *x)
{
    return pycc_ext_stdlib_call("bisect", "bisect_right", a, x);
}

PyObject *pycc_ext_obj_insort_left(PyObject *a, PyObject *x)
{
    return pycc_ext_stdlib_call("bisect", "insort_left", a, x);
}

PyObject *pycc_ext_obj_insort_right(PyObject *a, PyObject *x)
{
    return pycc_ext_stdlib_call("bisect", "insort_right", a, x);
}
