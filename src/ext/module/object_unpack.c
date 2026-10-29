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

/* #10: CPython 3.13's argument-binding `TypeError`s for a published
 * compiled function, method or constructor (`ceval.c`'s `missing_arguments`
 * and `too_many_positional`), word for word. `names` holds the `arity`
 * positional parameter names, the receiver excluded; `receiver` is 1 when
 * CPython counts a bound `self` or `cls` in "takes N positional arguments"
 * (an instance method, a classmethod, `__init__`) and 0 otherwise.
 *
 * `pycc_ext_missing_error` reports the parameters among the first
 * `required` that are not filled: a slot of `slots` left NULL, or with
 * `slots` NULL every index from `nargs` on. The list reads `'a'`, `'a' and
 * 'b'`, `'a', 'b', and 'c'`. */
static void pycc_ext_missing_error(const char *fn_name, const char *const *names,
                                   Py_ssize_t required, PyObject *const *slots, Py_ssize_t nargs)
{
    Py_ssize_t i;
    Py_ssize_t missing = 0;
    Py_ssize_t seen = 0;
    PyObject *list;

    for (i = 0; i < required; i++) {
        missing += slots != NULL ? slots[i] == NULL : i >= nargs;
    }
    list = PyUnicode_FromString("");
    for (i = 0; i < required && list != NULL; i++) {
        PyObject *next;
        if (slots != NULL ? slots[i] != NULL : i < nargs) {
            continue;
        }
        seen++;
        if (seen == 1) {
            next = PyUnicode_FromFormat("%U'%s'", list, names[i]);
        } else if (seen < missing) {
            next = PyUnicode_FromFormat("%U, '%s'", list, names[i]);
        } else {
            next = PyUnicode_FromFormat(missing == 2 ? "%U and '%s'" : "%U, and '%s'", list,
                                        names[i]);
        }
        Py_DECREF(list);
        list = next;
    }
    if (list != NULL) {
        PyErr_Format(PyExc_TypeError, "%s() missing %zd required positional argument%s: %U",
                     fn_name, missing, missing == 1 ? "" : "s", list);
        Py_DECREF(list);
    }
}

/* CPython 3.13's "Did you mean" for an unexpected keyword `key`: the
 * closest of the receiver (`receiver`, or NULL) and the `arity` parameter
 * names, by the interpreter's own `_suggestions._generate_suggestions`
 * (what `initialize_locals` calls). A new reference, or NULL -- with no
 * exception set -- when nothing is close enough or the lookup fails. */
static PyObject *pycc_ext_keyword_suggestion(const char *receiver, const char *const *names,
                                             Py_ssize_t arity, PyObject *key)
{
    PyObject *module;
    PyObject *candidates;
    PyObject *found = NULL;
    Py_ssize_t i;

    if (!PyUnicode_Check(key)) {
        return NULL;
    }
    candidates = PyList_New(0);
    if (candidates == NULL) {
        PyErr_Clear();
        return NULL;
    }
    for (i = -1; i < arity; i++) {
        const char *name = i < 0 ? receiver : names[i];
        PyObject *text;
        if (name == NULL) {
            continue;
        }
        text = PyUnicode_FromString(name);
        if (text == NULL || PyList_Append(candidates, text) < 0) {
            Py_XDECREF(text);
            Py_DECREF(candidates);
            PyErr_Clear();
            return NULL;
        }
        Py_DECREF(text);
    }
    module = PyImport_ImportModule("_suggestions");
    if (module != NULL) {
        found = PyObject_CallMethod(module, "_generate_suggestions", "OO", candidates, key);
        Py_DECREF(module);
    }
    Py_DECREF(candidates);
    if (found == NULL) {
        PyErr_Clear();
        return NULL;
    }
    if (found == Py_None) {
        Py_DECREF(found);
        return NULL;
    }
    return found;
}

/* The error of a keyword-free call whose `nargs` positional arguments fall
 * outside `required..=arity`. */
static void pycc_ext_arity_error(const char *fn_name, const char *const *names, Py_ssize_t arity,
                                 Py_ssize_t required, Py_ssize_t receiver, Py_ssize_t nargs)
{
    if (nargs > arity) {
        Py_ssize_t given = nargs + receiver;
        const char *verb = given == 1 ? "was" : "were";
        if (required < arity) {
            PyErr_Format(PyExc_TypeError,
                         "%s() takes from %zd to %zd positional arguments but %zd %s given",
                         fn_name, required + receiver, arity + receiver, given, verb);
        } else {
            Py_ssize_t takes = arity + receiver;
            PyErr_Format(PyExc_TypeError, "%s() takes %zd positional argument%s but %zd %s given",
                         fn_name, takes, takes == 1 ? "" : "s", given, verb);
        }
        return;
    }
    pycc_ext_missing_error(fn_name, names, required, NULL, nargs);
}

/* v0.9: binds a call of a compiled constructor `def __init__(self, a, b,
 * *rest)` (`qualname` names it, `names` its `fixed` parameters before
 * `*rest`) as CPython binds it (`ceval.c`'s `initialize_locals`), with its
 * messages: a keyword names a fixed parameter not already given
 * positionally, and every fixed parameter must be given. `bound` receives
 * `fixed` borrowed references, `*rest` a new reference to the tuple of the
 * positional arguments past them. 0, or -1 with the `TypeError` set. */
static int pycc_ext_bind_star(PyObject *args, PyObject *kwds, const char *qualname,
                              const char *const *names, Py_ssize_t fixed, PyObject **bound,
                              PyObject **rest)
{
    Py_ssize_t given = PyTuple_Size(args);
    Py_ssize_t pos = 0;
    Py_ssize_t i;
    PyObject *key;
    PyObject *value;

    for (i = 0; i < fixed; i++) {
        bound[i] = i < given ? PyTuple_GetItem(args, i) : NULL;
    }
    while (kwds != NULL && PyDict_Next(kwds, &pos, &key, &value)) {
        for (i = 0; i < fixed; i++) {
            if (PyUnicode_Check(key) && PyUnicode_CompareWithASCIIString(key, names[i]) == 0) {
                break;
            }
        }
        if (i == fixed) {
            PyObject *suggestion = pycc_ext_keyword_suggestion("self", names, fixed, key);
            if (suggestion != NULL) {
                PyErr_Format(PyExc_TypeError,
                             "%s() got an unexpected keyword argument '%S'. Did you mean '%S'?",
                             qualname, key, suggestion);
                Py_DECREF(suggestion);
            } else {
                PyErr_Format(PyExc_TypeError, "%s() got an unexpected keyword argument '%S'",
                             qualname, key);
            }
            return -1;
        }
        if (bound[i] != NULL) {
            PyErr_Format(PyExc_TypeError, "%s() got multiple values for argument '%s'", qualname,
                         names[i]);
            return -1;
        }
        bound[i] = value;
    }
    for (i = 0; i < fixed; i++) {
        if (bound[i] == NULL) {
            pycc_ext_missing_error(qualname, names, fixed, bound, 0);
            return -1;
        }
    }
    *rest = PyTuple_GetSlice(args, given < fixed ? given : fixed, given);
    return *rest == NULL ? -1 : 0;
}
