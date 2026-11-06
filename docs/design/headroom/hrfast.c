/* Hand-written "ideal" C for three hot pydantic functions:
 *   pydantic.json_schema._make_json_hashable
 *   pydantic.json_schema._deduplicate_schemas
 *   pydantic._internal._config.ConfigWrapper.__getattr__
 * Same results and exceptions as the Python source for the inputs that occur. */
#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include <structmember.h>

static PyObject *config_module_dict;   /* _config.__dict__, for the `config_defaults` global */
static PyObject *str_config_defaults;  /* interned "config_defaults" */
static PyObject *str_effective_config; /* interned "effective_config" */
static PyObject *str_items;
static PyTypeObject *config_wrapper_type;
static Py_ssize_t effective_config_offset = -1;

static PyObject *hashable(PyObject *v);

/* tuple(sorted((k, hashable(v)) for k, v in d.items())) */
static PyObject *hashable_dict_exact(PyObject *d)
{
    Py_ssize_t n = PyDict_GET_SIZE(d), pos = 0, i = 0;
    PyObject *k, *v;
    PyObject *list = PyList_New(n);
    if (!list) return NULL;
    while (PyDict_Next(d, &pos, &k, &v)) {
        if (i >= n) { PyErr_SetString(PyExc_RuntimeError, "dictionary changed size during iteration"); goto fail; }
        PyObject *hv = hashable(v);
        if (!hv) goto fail;
        PyObject *pair = PyTuple_New(2);
        if (!pair) { Py_DECREF(hv); goto fail; }
        PyTuple_SET_ITEM(pair, 0, Py_NewRef(k));
        PyTuple_SET_ITEM(pair, 1, hv);
        PyList_SET_ITEM(list, i++, pair);
    }
    if (i != n) { PyErr_SetString(PyExc_RuntimeError, "dictionary changed size during iteration"); goto fail; }
    if (n > 1 && PyList_Sort(list) < 0) goto fail;
    /* steal the sorted items into the result tuple (the list is private) */
    PyObject *t = PyTuple_New(n);
    if (!t) goto fail;
    for (i = 0; i < n; i++) PyTuple_SET_ITEM(t, i, PyList_GET_ITEM(list, i));
    Py_SET_SIZE(list, 0);
    Py_DECREF(list);
    return t;
fail:
    /* unfilled slots are NULL; list_dealloc uses Py_XDECREF */
    Py_DECREF(list);
    return NULL;
}

/* generic path for dict subclasses: honours an overridden items() */
static PyObject *hashable_dict_generic(PyObject *d)
{
    PyObject *items = PyObject_CallMethodNoArgs(d, str_items);
    if (!items) return NULL;
    PyObject *it = PyObject_GetIter(items);
    Py_DECREF(items);
    if (!it) return NULL;
    PyObject *list = PyList_New(0), *item;
    if (!list) { Py_DECREF(it); return NULL; }
    while ((item = PyIter_Next(it))) {
        PyObject *k, *v;
        PyObject *seq = PySequence_Fast(item, "cannot unpack non-iterable object");
        Py_DECREF(item);
        if (!seq) goto fail;
        if (PySequence_Fast_GET_SIZE(seq) != 2) {
            PyErr_Format(PyExc_ValueError, "too many values to unpack (expected 2)");
            Py_DECREF(seq); goto fail;
        }
        k = PySequence_Fast_GET_ITEM(seq, 0);
        v = PySequence_Fast_GET_ITEM(seq, 1);
        PyObject *hv = hashable(v);
        if (!hv) { Py_DECREF(seq); goto fail; }
        PyObject *pair = PyTuple_Pack(2, k, hv);
        Py_DECREF(hv); Py_DECREF(seq);
        if (!pair) goto fail;
        int r = PyList_Append(list, pair);
        Py_DECREF(pair);
        if (r < 0) goto fail;
    }
    if (PyErr_Occurred()) goto fail;
    Py_DECREF(it);
    if (PyList_Sort(list) < 0) { Py_DECREF(list); return NULL; }
    PyObject *t = PyList_AsTuple(list);
    Py_DECREF(list);
    return t;
fail:
    Py_DECREF(it); Py_DECREF(list);
    return NULL;
}

/* tuple(hashable(v) for v in lst) */
static PyObject *hashable_list_exact(PyObject *l)
{
    Py_ssize_t n = PyList_GET_SIZE(l);
    PyObject *t = PyTuple_New(n);
    if (!t) return NULL;
    for (Py_ssize_t i = 0; i < n; i++) {
        if (i >= PyList_GET_SIZE(l)) { /* list shrank: generator semantics stop early */
            if (_PyTuple_Resize(&t, i) < 0) return NULL;
            return t;
        }
        PyObject *hv = hashable(PyList_GET_ITEM(l, i));
        if (!hv) { Py_DECREF(t); return NULL; }
        PyTuple_SET_ITEM(t, i, hv);
    }
    return t;
}

static PyObject *hashable_iter_generic(PyObject *o)
{
    PyObject *it = PyObject_GetIter(o), *item;
    if (!it) return NULL;
    PyObject *list = PyList_New(0);
    if (!list) { Py_DECREF(it); return NULL; }
    while ((item = PyIter_Next(it))) {
        PyObject *hv = hashable(item);
        Py_DECREF(item);
        if (!hv || PyList_Append(list, hv) < 0) { Py_XDECREF(hv); Py_DECREF(it); Py_DECREF(list); return NULL; }
        Py_DECREF(hv);
    }
    Py_DECREF(it);
    if (PyErr_Occurred()) { Py_DECREF(list); return NULL; }
    PyObject *t = PyList_AsTuple(list);
    Py_DECREF(list);
    return t;
}

static PyObject *hashable(PyObject *v)
{
    if (PyDict_CheckExact(v) || PyList_CheckExact(v)) {
        if (Py_EnterRecursiveCall(" in _make_json_hashable")) return NULL;
        PyObject *r = PyDict_CheckExact(v) ? hashable_dict_exact(v) : hashable_list_exact(v);
        Py_LeaveRecursiveCall();
        return r;
    }
    /* scalars that occur (str, int, bool, float, None): exact-type fast exit */
    if (PyUnicode_CheckExact(v) || PyLong_CheckExact(v) || v == Py_None || PyBool_Check(v) || PyFloat_CheckExact(v))
        return Py_NewRef(v);
    int r = PyObject_IsInstance(v, (PyObject *)&PyDict_Type);
    if (r < 0) return NULL;
    if (r) {
        if (Py_EnterRecursiveCall(" in _make_json_hashable")) return NULL;
        PyObject *res = hashable_dict_generic(v);
        Py_LeaveRecursiveCall();
        return res;
    }
    r = PyObject_IsInstance(v, (PyObject *)&PyList_Type);
    if (r < 0) return NULL;
    if (r) {
        if (Py_EnterRecursiveCall(" in _make_json_hashable")) return NULL;
        PyObject *res = hashable_iter_generic(v);
        Py_LeaveRecursiveCall();
        return res;
    }
    return Py_NewRef(v);
}

static PyObject *make_json_hashable(PyObject *m, PyObject *v) { return hashable(v); }

/* list({_make_json_hashable(s): s for s in schemas}.values()) */
static PyObject *deduplicate_schemas(PyObject *m, PyObject *schemas)
{
    PyObject *d = PyDict_New();
    if (!d) return NULL;
    if (PyList_CheckExact(schemas) || PyTuple_CheckExact(schemas)) {
        for (Py_ssize_t i = 0; i < PySequence_Fast_GET_SIZE(schemas); i++) {
            PyObject *s = PySequence_Fast_GET_ITEM(schemas, i);
            Py_INCREF(s);
            PyObject *k = hashable(s);
            if (!k || PyDict_SetItem(d, k, s) < 0) { Py_XDECREF(k); Py_DECREF(s); Py_DECREF(d); return NULL; }
            Py_DECREF(k); Py_DECREF(s);
        }
    } else {
        PyObject *it = PyObject_GetIter(schemas), *s;
        if (!it) { Py_DECREF(d); return NULL; }
        while ((s = PyIter_Next(it))) {
            PyObject *k = hashable(s);
            if (!k || PyDict_SetItem(d, k, s) < 0) { Py_XDECREF(k); Py_DECREF(s); Py_DECREF(it); Py_DECREF(d); return NULL; }
            Py_DECREF(k); Py_DECREF(s);
        }
        Py_DECREF(it);
        if (PyErr_Occurred()) { Py_DECREF(d); return NULL; }
    }
    PyObject *res = PyDict_Values(d);
    Py_DECREF(d);
    return res;
}

/* ConfigWrapper.__getattr__(self, name):
 *   try: return self.effective_config[name]
 *   except KeyError:
 *       try: return config_defaults[name]
 *       except KeyError: raise AttributeError(f'Config has no attribute {name!r}') from None */
static PyObject *lookup_keyerror(PyObject *mapping, PyObject *name, int *missing)
{
    *missing = 0;
    if (PyDict_CheckExact(mapping)) {
        PyObject *r;
        int rc = PyDict_GetItemRef(mapping, name, &r);
        if (rc < 0) return NULL;
        if (rc == 0) { *missing = 1; return NULL; }
        return r;
    }
    PyObject *r = PyObject_GetItem(mapping, name);
    if (!r && PyErr_ExceptionMatches(PyExc_KeyError)) { PyErr_Clear(); *missing = 1; }
    return r;
}

static PyObject *config_getattr(PyObject *m, PyObject *const *args, Py_ssize_t nargs)
{
    if (nargs != 2) { PyErr_SetString(PyExc_TypeError, "__getattr__ expects (self, name)"); return NULL; }
    PyObject *self = args[0], *name = args[1], *ec;
    if (effective_config_offset >= 0 && PyObject_TypeCheck(self, config_wrapper_type)) {
        ec = *(PyObject **)((char *)self + effective_config_offset);
        if (!ec) return PyObject_GetAttr(self, str_effective_config); /* unset slot: original semantics */
        Py_INCREF(ec);
    } else {
        ec = PyObject_GetAttr(self, str_effective_config);
        if (!ec) return NULL;
    }
    int missing;
    PyObject *r = lookup_keyerror(ec, name, &missing);
    Py_DECREF(ec);
    if (r || !missing) return r;
    PyObject *defaults;
    if (PyDict_GetItemRef(config_module_dict, str_config_defaults, &defaults) <= 0) {
        if (!PyErr_Occurred()) {
            /* global missing -> NameError, as the Python would (context: the KeyError) */
            PyErr_SetObject(PyExc_KeyError, name);
            PyObject *ke = PyErr_GetRaisedException();
            PyErr_Format(PyExc_NameError, "name 'config_defaults' is not defined");
            PyObject *ne = PyErr_GetRaisedException();
            PyException_SetContext(ne, ke);
            PyErr_SetRaisedException(ne);
        }
        return NULL;
    }
    r = lookup_keyerror(defaults, name, &missing);
    Py_DECREF(defaults);
    if (r || !missing) return r;
    /* rare path: build the exact exception chain of `raise ... from None` inside `except KeyError` */
    PyErr_SetObject(PyExc_KeyError, name);
    PyObject *inner = PyErr_GetRaisedException();
    PyErr_SetObject(PyExc_KeyError, name);
    PyObject *outer = PyErr_GetRaisedException();
    PyException_SetContext(inner, outer);
    PyObject *msg = PyUnicode_FromFormat("Config has no attribute %R", name);
    if (!msg) { Py_DECREF(inner); return NULL; }
    PyObject *ae = PyObject_CallOneArg(PyExc_AttributeError, msg);
    Py_DECREF(msg);
    if (!ae) { Py_DECREF(inner); return NULL; }
    PyException_SetContext(ae, inner);
    PyException_SetCause(ae, NULL);   /* from None: __cause__ = None, __suppress_context__ = True */
    PyErr_SetRaisedException(ae);
    return NULL;
}

static PyObject *setup(PyObject *m, PyObject *const *args, Py_ssize_t nargs)
{
    if (nargs != 2) { PyErr_SetString(PyExc_TypeError, "setup(config_module, ConfigWrapper)"); return NULL; }
    PyObject *d = PyModule_GetDict(args[0]);
    if (!d) return NULL;
    Py_XSETREF(config_module_dict, Py_NewRef(d));
    if (!PyType_Check(args[1])) { PyErr_SetString(PyExc_TypeError, "ConfigWrapper must be a type"); return NULL; }
    Py_XSETREF(config_wrapper_type, (PyTypeObject *)Py_NewRef(args[1]));
    PyObject *descr = PyObject_GetAttr(args[1], str_effective_config);
    if (!descr) return NULL;
    if (Py_IS_TYPE(descr, &PyMemberDescr_Type)) {
        PyMemberDef *md = ((PyMemberDescrObject *)descr)->d_member;
        if (md->type == Py_T_OBJECT_EX) effective_config_offset = md->offset;
    }
    Py_DECREF(descr);
    return PyLong_FromSsize_t(effective_config_offset);
}

static PyObject *instancemethod(PyObject *m, PyObject *f) { return PyInstanceMethod_New(f); }

static PyMethodDef M[] = {
    {"_make_json_hashable", make_json_hashable, METH_O, NULL},
    {"_deduplicate_schemas", deduplicate_schemas, METH_O, NULL},
    {"config_getattr", (PyCFunction)(void (*)(void))config_getattr, METH_FASTCALL, NULL},
    {"setup", (PyCFunction)(void (*)(void))setup, METH_FASTCALL, NULL},
    {"instancemethod", instancemethod, METH_O, NULL},
    {NULL}};
static struct PyModuleDef D = {PyModuleDef_HEAD_INIT, "hrfast", NULL, -1, M};
PyMODINIT_FUNC PyInit_hrfast(void)
{
    str_config_defaults = PyUnicode_InternFromString("config_defaults");
    str_effective_config = PyUnicode_InternFromString("effective_config");
    str_items = PyUnicode_InternFromString("items");
    if (!str_config_defaults || !str_effective_config || !str_items) return NULL;
    return PyModule_Create(&D);
}
