/*
 * v0.9: the per-call-site cache of `obj.name(args)` (`EXT_OBJ_CALL_METHOD`
 * and the load/call pair): the class the site last saw, by its version
 * tag, and what the class's attribute cache answered for the name -- a
 * method of the class, called with `obj` first. A hit is two compares
 * against the site's own record instead of the shared table's hashing.
 * A class whose instances have no dict (a compiled class's carrier, a
 * class with `__slots__`) caches here, as does one keeping its instances'
 * values inline when the class's entry says no instance can shadow the
 * method yet (`pycc_ext_ic_method_holds`): the site keeps that entry's
 * count of the shared keys, and a hit also needs `obj`'s values inline
 * and the count unchanged. A method descriptor with a `METH_FASTCALL` C
 * function (a compiled class's method wrapper) is called through that
 * function directly, as `method_descriptor`'s vectorcall calls it; a
 * compiled method's Python forwarder (`forward.c`) through its slot, as
 * its trampoline calls it. The record is static storage codegen gives
 * each call site, zero at first (`call_site_record` in
 * `crates/pycc_codegen/src/foreign_call.rs`, six words).
 */
struct PycFwdSlot;
typedef struct {
    PyObject *type;
    unsigned long long version;
    PyObject *method;
    void *fast;
    Py_ssize_t keys;          /* the shared keys' count, or -1: no instance dict */
    struct PycFwdSlot *fwd;   /* the forwarder's slot when `method` is one */
} PyccExtCallSite;

static struct PycFwdSlot *pycc_fwd_slot_of(PyObject *func);
static PyObject *pycc_fwd_direct(struct PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                                 Py_ssize_t n);

/* `method_descriptor`'s layout (`PyMethodDescrObject`): its `PyMethodDef *`
 * after the common descriptor fields, checked on `str.join`. */
#define PYCC_IC_METHOD_DESCR_DEF 40
static PyObject *pycc_ext_site_descr_type = NULL;
static int pycc_ext_site_descr_ok = 0;

static void pycc_ext_site_probe(void)
{
    PyObject *join;

    if (pycc_ext_site_descr_ok != 0) {
        return;
    }
    pycc_ext_site_descr_ok = -1;
    join = PyObject_GetAttrString((PyObject *)&PyUnicode_Type, "join");
    if (join == NULL) {
        PyErr_Clear();
        return;
    }
    {
        PyMethodDef *def = PYCC_IC_FIELD(join, PYCC_IC_METHOD_DESCR_DEF, PyMethodDef *);
        if (def != NULL && def->ml_name != NULL && strcmp(def->ml_name, "join") == 0) {
            pycc_ext_site_descr_type = (PyObject *)Py_TYPE(join);
            pycc_ext_site_descr_ok = 1;
        }
    }
    Py_DECREF(join);
}

/* The method `site` holds for `obj`'s class, or NULL (borrowed). */
static PyObject *pycc_ext_site_method(PyccExtCallSite *site, PyObject *obj)
{
    PyTypeObject *type = Py_TYPE(obj);

    if (site == NULL || site->type != (PyObject *)type || pycc_ext_ic_state <= 0
        || site->version != PYCC_IC_TAG(type)) {
        return NULL;
    }
    if (site->keys >= 0
        && (!pycc_ext_ic_valid(obj) || pycc_ext_ic_nentries(pycc_ext_ic_keys(type)) != site->keys)) {
        return NULL;
    }
    return site->method;
}

/* Fills `site` for `obj.spelling(...)` from the class's attribute cache,
 * once the generic path has filled that. Never sets an exception. */
static void pycc_ext_site_fill(PyccExtCallSite *site, PyObject *obj, const char *spelling)
{
    PyTypeObject *type = Py_TYPE(obj);
    unsigned long flags = PyType_GetFlags(type);
    int has_dict = (flags & (PYCC_IC_INLINE_VALUES | PYCC_IC_MANAGED_DICT)) != 0
                   || PYCC_IC_FIELD(type, PYCC_IC_TP_DICTOFFSET, Py_ssize_t) != 0;
    PyObject *method;
    long slot;

    if (site == NULL || pycc_ext_ic_state <= 0 || PYCC_IC_TAG(type) == 0) {
        return;
    }
    slot = pycc_ext_ic_find(obj, spelling);
    if (slot < 0 || PYCC_IC(slot).kind != PYCC_IC_METHOD
        || !pycc_ext_ic_method_holds(obj, (size_t)slot)
        || (has_dict && !PYCC_IC(slot).inline_values)) {
        return;
    }
    method = PYCC_IC(slot).object;
    pycc_ext_site_probe();
    site->fast = NULL;
    site->fwd = NULL;
    if (pycc_ext_site_descr_ok > 0 && (PyObject *)Py_TYPE(method) == pycc_ext_site_descr_type) {
        PyMethodDef *def = PYCC_IC_FIELD(method, PYCC_IC_METHOD_DESCR_DEF, PyMethodDef *);
        if (def != NULL && (def->ml_flags & ~METH_COEXIST) == METH_FASTCALL) {
            site->fast = (void *)def->ml_meth;
        }
    } else if ((PyObject *)Py_TYPE(method) == pycc_frames_function_type) {
        site->fwd = pycc_fwd_slot_of(method);
    }
    site->keys = PYCC_IC(slot).inline_values ? PYCC_IC(slot).index : -1;
    site->type = (PyObject *)type;
    site->version = PYCC_IC_TAG(type);
    site->method = method;
}

/* Calls what `site` holds for `self` with `stack[1..nargs]` (`stack[0]` is
 * `self`); a new reference, or NULL with the exception set. */
static PyObject *pycc_ext_site_call(PyccExtCallSite *site, PyObject *method, PyObject **stack,
                                    long long nargs)
{
    PyObject *result;

    if (site->fwd != NULL) {
        return pycc_fwd_direct(site->fwd, method, stack, (Py_ssize_t)(nargs + 1));
    }
    if (site->fast == NULL) {
        return PyObject_Vectorcall(method, stack, (size_t)(nargs + 1), NULL);
    }
    if (Py_EnterRecursiveCall(" while calling a Python object")) {
        return NULL;
    }
    result = ((PyObject * (*)(PyObject *, PyObject *const *, Py_ssize_t)) site->fast)(
        stack[0], stack + 1, (Py_ssize_t)nargs);
    Py_LeaveRecursiveCall();
    return result;
}
