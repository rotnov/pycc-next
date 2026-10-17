/*
 * Non-strict mode: a compiled function value crossing into CPython
 * (`fn = (None, undo_filter_sub, ...)[k]`, `apply(callback)` on an object).
 * Only a function value whose parameters and result are all CPython objects
 * crosses (`pycc_types`' `is_boxable_callable`): its call function
 * (`pycc_rt::closure`) then has one signature per argument count,
 * `PyObject *call(closure, PyObject *...)`, each parameter owning its
 * argument and the result a new reference, so one CPython type calls any of
 * them. The wrapper holds a reference to the function value. Part of the one
 * shim translation unit (`src/ext_build.rs`'s `SHIM_C`), after `bridge.c`.
 */

#include <stddef.h>

extern void pycc_rt_closure_incref(void *obj);
extern void pycc_rt_closure_decref(void *obj);

/* The most parameters a crossing function value may have. */
#define PYCC_EXT_CLOSURE_MAX_ARITY 8

typedef struct {
    PyObject_HEAD
    void *closure;
    long long arity;
    /* CPython's vectorcall entry (`__vectorcalloffset__`): a call passes
     * its arguments as an array, with no tuple built for them. */
    vectorcallfunc vectorcall;
} PyccExtClosure;

static void pycc_ext_closure_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *closure = ((PyccExtClosure *)self)->closure;
    freefunc tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);

    tp_free(self);
    Py_DECREF(tp);
    pycc_rt_closure_decref(closure);
}

/* The call function of the function value `closure`
 * (`pycc_rt::closure::CALL_OFFSET`). */
static void *pycc_ext_closure_call_fn(void *closure)
{
    return *(void **)((char *)closure + 8);
}

/* Calls the function value of `wrapper` with the `n` arguments `items`
 * (borrowed). */
static PyObject *pycc_ext_closure_invoke(PyccExtClosure *wrapper, PyObject *const *items,
                                         Py_ssize_t n)
{
    PyObject *a[PYCC_EXT_CLOSURE_MAX_ARITY];
    Py_ssize_t i;
    void *fn = pycc_ext_closure_call_fn(wrapper->closure);
    void *c = wrapper->closure;
    PyObject *result = NULL;

    if (n != (Py_ssize_t)wrapper->arity) {
        PyErr_Format(PyExc_TypeError, "function takes %lld positional arguments but %zd were given",
                     wrapper->arity, n);
        return NULL;
    }
    /* Each parameter owns its argument. */
    for (i = 0; i < n; i++) {
        a[i] = items[i];
        Py_INCREF(a[i]);
    }
    /* The function value outlives its own call. */
    pycc_rt_closure_incref(c);
    switch (n) {
    case 0:
        result = ((PyObject * (*)(void *))fn)(c);
        break;
    case 1:
        result = ((PyObject * (*)(void *, PyObject *))fn)(c, a[0]);
        break;
    case 2:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *))fn)(c, a[0], a[1]);
        break;
    case 3:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *))fn)(c, a[0], a[1],
                                                                                  a[2]);
        break;
    case 4:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *, PyObject *))fn)(
            c, a[0], a[1], a[2], a[3]);
        break;
    case 5:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *, PyObject *,
                                  PyObject *))fn)(c, a[0], a[1], a[2], a[3], a[4]);
        break;
    case 6:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *, PyObject *,
                                  PyObject *, PyObject *))fn)(c, a[0], a[1], a[2], a[3], a[4],
                                                             a[5]);
        break;
    case 7:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *, PyObject *,
                                  PyObject *, PyObject *, PyObject *))fn)(
            c, a[0], a[1], a[2], a[3], a[4], a[5], a[6]);
        break;
    default:
        result = ((PyObject * (*)(void *, PyObject *, PyObject *, PyObject *, PyObject *,
                                  PyObject *, PyObject *, PyObject *, PyObject *))fn)(
            c, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
        break;
    }
    pycc_rt_closure_decref(c);
    if (pycc_ext_raise_pending()) {
        /* With an exception pending the result is a placeholder, not a reference. */
        return NULL;
    }
    return result;
}

static PyObject *pycc_ext_closure_call(PyObject *self, PyObject *args, PyObject *kwargs)
{
    PyObject *items[PYCC_EXT_CLOSURE_MAX_ARITY];
    Py_ssize_t n = PyTuple_Size(args);
    Py_ssize_t i;

    if (kwargs != NULL && PyDict_Size(kwargs) > 0) {
        PyErr_SetString(PyExc_TypeError, "a compiled function value takes no keyword arguments");
        return NULL;
    }
    for (i = 0; i < n && i < PYCC_EXT_CLOSURE_MAX_ARITY; i++) {
        items[i] = PyTuple_GetItem(args, i);
    }
    return pycc_ext_closure_invoke((PyccExtClosure *)self, items, n);
}

static PyObject *pycc_ext_closure_vectorcall(PyObject *self, PyObject *const *args, size_t nargsf,
                                             PyObject *kwnames)
{
    if (kwnames != NULL && PyTuple_Size(kwnames) > 0) {
        PyErr_SetString(PyExc_TypeError, "a compiled function value takes no keyword arguments");
        return NULL;
    }
    return pycc_ext_closure_invoke((PyccExtClosure *)self, args, PyVectorcall_NARGS(nargsf));
}

static PyMemberDef pycc_ext_closure_members[] = {
    {"__vectorcalloffset__", Py_T_PYSSIZET, offsetof(PyccExtClosure, vectorcall), Py_READONLY, NULL},
    {NULL, 0, 0, 0, NULL},
};

static PyType_Slot pycc_ext_closure_slots[] = {
    {Py_tp_dealloc, (void *)pycc_ext_closure_dealloc},
    {Py_tp_call, (void *)pycc_ext_closure_call},
    {Py_tp_members, (void *)pycc_ext_closure_members},
    {0, NULL},
};

static PyObject *pycc_ext_closure_type = NULL;

/*
 * The packer of a function value of `arity` object parameters
 * (`EXT_OBJ_PACK_CLOSURE_SYMBOL`): borrows `closure`, returns a new
 * reference to a CPython callable calling it, or NULL with an exception set.
 */
PyObject *pycc_ext_obj_pack_closure(void *closure, long long arity)
{
    PyccExtClosure *wrapper;
    PyType_Spec spec;

    if (closure == NULL || arity < 0 || arity > PYCC_EXT_CLOSURE_MAX_ARITY) {
        PyErr_SetString(PyExc_SystemError, "pycc: bad function value crossing");
        return NULL;
    }
    if (pycc_ext_closure_type == NULL) {
        spec.name = "pycc.compiled_function";
        spec.basicsize = (int)sizeof(PyccExtClosure);
        spec.itemsize = 0;
        spec.flags = Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION
                     | Py_TPFLAGS_IMMUTABLETYPE | Py_TPFLAGS_HAVE_VECTORCALL;
        spec.slots = pycc_ext_closure_slots;
        pycc_ext_closure_type = PyType_FromSpec(&spec);
        if (pycc_ext_closure_type == NULL) {
            return NULL;
        }
    }
    wrapper = PyObject_New(PyccExtClosure, (PyTypeObject *)pycc_ext_closure_type);
    if (wrapper == NULL) {
        return NULL;
    }
    pycc_rt_closure_incref(closure);
    wrapper->closure = closure;
    wrapper->arity = arity;
    wrapper->vectorcall = pycc_ext_closure_vectorcall;
    return (PyObject *)wrapper;
}

/*
 * `pycc_ext_obj_pack_closure` for a module-level function used as a value
 * (`EXT_OBJ_PACK_FUNCTION_SYMBOL`): the callable is made once and kept in
 * `*slot` (a per-function global of the artifact, holding its own
 * reference), so every crossing of the function hands CPython the same
 * object, as a Python function is one object. Returns a new reference, or
 * NULL with an exception set.
 */
PyObject *pycc_ext_obj_pack_function(PyObject **slot, void *closure, long long arity)
{
    PyObject *packed;

    if (*slot != NULL) {
        Py_INCREF(*slot);
        return *slot;
    }
    packed = pycc_ext_obj_pack_closure(closure, arity);
    if (packed != NULL) {
        Py_INCREF(packed);
        *slot = packed;
    }
    return packed;
}
