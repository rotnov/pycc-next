/*
 * v0.10 accept 5: the templates of exception classes with a native layout
 * (`src/fallback/exc_layout.rs`).
 *
 * CPython makes such a class from its statement, whose bases the rewrite
 * passes through `__pycc_exc_bases__("__pycc_excl_C__", *bases)`: the hook
 * answers `(template, *bases)`, the template a PEP 697 type over the first
 * exception base (a negative `basicsize`: its objects carry one more word,
 * the native instance pointer, wherever CPython puts it) -- or, over a base
 * whose objects already carry one, a type adding nothing to the layout. Its
 * slots: the layout's field descriptors; a `__reduce__` adding the native
 * attributes to what the bases' answer; and the deallocator, traverse and
 * clear that release the instance and chain to the exception base's.
 *
 * It has no `tp_new` of its own and no `__new__` in its dictionary: it
 * inherits the exception base's, so CPython looks `__new__` up along the
 * class's MRO past it, as for the source class (a Python base's own
 * `__new__` runs), and `BaseException.__new__`'s safety check finds the
 * class's `tp_new` to be the static base's. An object gets its instance of
 * the class's layout, every attribute unassigned, when native code first
 * asks for it (`pycc_ext_exc_bind`, from `pycc_ext_inst_of`).
 *
 * The object and its instance hold each other, as a Python subclass's
 * object of a compiled class and its instance do (`pycc_ext_carrier_bind`):
 * the instance's link is strong, and the cycle is the collector's to break
 * (`pycc_ext_exc_traverse`, `pycc_ext_exc_clear`).
 */

/* Each template -> its layout (a capsule of the companion's entry). */
static PyObject *pycc_ext_exc_layout_of = NULL;
/* (layout entry, exception base) -> its template. */
static PyObject *pycc_ext_exc_templates = NULL;

#define PYCC_EXC_LAYOUT_CAPSULE "pycc.exc_layout"

/* The first static type of `tp`'s MRO: the exception base whose `tp_new`,
 * deallocator, traverse and clear the template's chain to (borrowed). */
static PyTypeObject *pycc_ext_exc_static_base(PyTypeObject *tp)
{
    PyObject *mro = PyObject_GetAttrString((PyObject *)tp, "__mro__");
    PyTypeObject *found = (PyTypeObject *)PyExc_BaseException;
    Py_ssize_t n;
    Py_ssize_t i;
    if (mro == NULL) {
        PyErr_Clear();
        return found;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        if ((PyType_GetFlags((PyTypeObject *)c) & Py_TPFLAGS_HEAPTYPE) == 0) {
            found = (PyTypeObject *)c;
            break;
        }
    }
    Py_DECREF(mro);
    return found;
}

/* The layout of the most derived template of `tp`, or NULL. */
static const PyccExtExcLayout *pycc_ext_exc_layout_for(PyTypeObject *tp)
{
    PyObject *tmpl = pycc_ext_exc_template_for(tp);
    PyObject *capsule;
    if (tmpl == NULL || pycc_ext_exc_layout_of == NULL) {
        return NULL;
    }
    capsule = PyDict_GetItemWithError(pycc_ext_exc_layout_of, tmpl);
    if (capsule == NULL) {
        PyErr_Clear();
        return NULL;
    }
    return (const PyccExtExcLayout *)PyCapsule_GetPointer(capsule, PYCC_EXC_LAYOUT_CAPSULE);
}

/* The instance of the layout of `self`'s class, every attribute unassigned,
 * stored in `self`'s payload `slot` (still empty) and holding `self`
 * strongly (borrowed; NULL when the class has no layout). */
static void *pycc_ext_exc_bind(PyObject *self, void **slot)
{
    const PyccExtExcLayout *layout = pycc_ext_exc_layout_for(Py_TYPE(self));
    void *inst;
    if (layout == NULL) {
        return NULL;
    }
    inst = pycc_ext_instance_of_record(layout->slot_count, layout->record);
    *slot = inst;
    Py_INCREF(self);
    pycc_rt_ext_instance_set_carrier_strong(inst, self);
    return inst;
}

static void pycc_ext_exc_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    destructor base_dealloc =
        (destructor)PyType_GetSlot(pycc_ext_exc_static_base(tp), Py_tp_dealloc);
    void **slot = pycc_ext_inst_slot(self);
    void *inst = slot == NULL ? NULL : *slot;
    if (inst != NULL) {
        if (pycc_rt_ext_instance_carrier(inst) == (void *)self) {
            pycc_rt_ext_instance_set_carrier(inst, NULL);
        }
        *slot = NULL;
    }
    /* The base's deallocator untracks the object, `OSError`'s without a
     * check (`_PyObject_GC_UNTRACK`): it must be tracked, as
     * `subtype_dealloc` leaves it before calling a collected base's. */
    if (!PyObject_GC_IsTracked(self)) {
        PyObject_GC_Track(self);
    }
    base_dealloc(self);
    Py_DECREF(tp);
    if (inst != NULL) {
        pycc_rt_instance_decref(inst);
    }
}

static int pycc_ext_exc_traverse(PyObject *self, visitproc visit, void *arg)
{
    traverseproc base_traverse =
        (traverseproc)PyType_GetSlot(pycc_ext_exc_static_base(Py_TYPE(self)), Py_tp_traverse);
    void **slot = pycc_ext_inst_slot(self);
    void *inst = slot == NULL ? NULL : *slot;
    if (inst != NULL && pycc_rt_ext_instance_carrier_is_strong(inst)
        && pycc_rt_ext_instance_carrier(inst) == (void *)self
        && pycc_rt_ext_instance_refcount(inst) == 1) {
        Py_VISIT(self);
    }
    return base_traverse == NULL ? 0 : base_traverse(self, visit, arg);
}

static int pycc_ext_exc_clear(PyObject *self)
{
    inquiry base_clear =
        (inquiry)PyType_GetSlot(pycc_ext_exc_static_base(Py_TYPE(self)), Py_tp_clear);
    void **slot = pycc_ext_inst_slot(self);
    void *inst = slot == NULL ? NULL : *slot;
    if (inst != NULL && pycc_rt_ext_instance_carrier_is_strong(inst)
        && pycc_rt_ext_instance_carrier(inst) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(inst, self);
        Py_DECREF(self);
    }
    return base_clear == NULL ? 0 : base_clear(self);
}

/*
 * `__reduce__`: what the bases after the template answer
 * (`BaseException.__reduce__`: `(type, args)`, or `(type, args, __dict__)`),
 * with each assigned native attribute added to the state, where the source
 * class keeps it in `__dict__`; `BaseException.__setstate__` sets it back
 * through its descriptor. Another answer (a mixin's own) is kept as it is.
 */
static PyObject *pycc_ext_exc_reduce(PyObject *self, PyTypeObject *defining, PyObject *const *args,
                                     size_t nargsf, PyObject *kwnames)
{
    Py_ssize_t nargs = PyVectorcall_NARGS(nargsf);
    PyObject *sup;
    PyObject *reduced;
    PyObject *state;
    PyObject *merged;
    PyObject *result;
    PyObject *capsule;
    const PyccExtExcLayout *layout;
    PyGetSetDef *def;
    Py_ssize_t n;
    Py_ssize_t i;
    (void)args;
    if (nargs != 0 || (kwnames != NULL && PyTuple_Size(kwnames) != 0)) {
        PyErr_SetString(PyExc_TypeError, "__reduce__() takes no arguments");
        return NULL;
    }
    sup = PyObject_CallFunctionObjArgs((PyObject *)&PySuper_Type, (PyObject *)defining, self, NULL);
    if (sup == NULL) {
        return NULL;
    }
    reduced = PyObject_CallMethod(sup, "__reduce__", NULL);
    Py_DECREF(sup);
    if (reduced == NULL || !PyTuple_Check(reduced) || PyTuple_Size(reduced) < 2) {
        return reduced;
    }
    n = PyTuple_Size(reduced);
    state = n > 2 ? PyTuple_GetItem(reduced, 2) : Py_None;
    if (state != Py_None && !PyDict_Check(state)) {
        return reduced;
    }
    capsule = pycc_ext_exc_layout_of == NULL
                  ? NULL
                  : PyDict_GetItemWithError(pycc_ext_exc_layout_of, (PyObject *)defining);
    layout = capsule == NULL ? NULL
                             : (const PyccExtExcLayout *)PyCapsule_GetPointer(
                                   capsule, PYCC_EXC_LAYOUT_CAPSULE);
    if (layout == NULL) {
        PyErr_Clear();
        return reduced;
    }
    merged = state == Py_None ? PyDict_New() : PyDict_Copy(state);
    if (merged == NULL) {
        Py_DECREF(reduced);
        return NULL;
    }
    for (def = layout->getsets; def != NULL && def->name != NULL; def++) {
        PyObject *value;
        if (def->get == NULL || strcmp(def->name, "__dict__") == 0) {
            continue;
        }
        value = def->get(self, def->closure);
        if (value == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_AttributeError)) {
                Py_DECREF(merged);
                Py_DECREF(reduced);
                return NULL;
            }
            PyErr_Clear();
            continue;
        }
        if (PyDict_SetItemString(merged, def->name, value) < 0) {
            Py_DECREF(value);
            Py_DECREF(merged);
            Py_DECREF(reduced);
            return NULL;
        }
        Py_DECREF(value);
    }
    if (PyDict_Size(merged) == 0) {
        Py_DECREF(merged);
        return reduced;
    }
    result = PyTuple_New(n > 3 ? n : 3);
    if (result == NULL) {
        Py_DECREF(merged);
        Py_DECREF(reduced);
        return NULL;
    }
    for (i = 0; i < (n > 3 ? n : 3); i++) {
        PyObject *item = i == 2 ? merged : Py_NewRef(PyTuple_GetItem(reduced, i));
        PyTuple_SetItem(result, i, item);
    }
    Py_DECREF(reduced);
    return result;
}

static PyMethodDef pycc_ext_exc_methods[] = {
    {"__reduce__", (PyCFunction)(void (*)(void))pycc_ext_exc_reduce,
     METH_METHOD | METH_FASTCALL | METH_KEYWORDS, NULL},
    {NULL, NULL, 0, NULL},
};

/* The template of layout `layout` over exception base `base`, made once. A
 * new reference, or NULL with an exception set. */
static PyObject *pycc_ext_exc_template(const PyccExtExcLayout *layout, PyObject *base)
{
    PyObject *key;
    PyObject *tmpl;
    PyObject *payload;
    PyObject *capsule;
    PyGetSetDef *getsets;
    PyType_Slot *slots;
    PyType_Spec *spec;
    Py_ssize_t count = 0;
    Py_ssize_t kept = 0;
    Py_ssize_t i;
    if (pycc_ext_exc_templates == NULL) {
        pycc_ext_exc_templates = PyDict_New();
        pycc_ext_exc_layout_of = PyDict_New();
        pycc_ext_exc_payload_of = PyDict_New();
        pycc_ext_exc_template_of = PyDict_New();
        if (pycc_ext_exc_templates == NULL || pycc_ext_exc_layout_of == NULL
            || pycc_ext_exc_payload_of == NULL || pycc_ext_exc_template_of == NULL) {
            return NULL;
        }
    }
    /* Keyed by the layout entry itself: the modules of a package built
     * whole share this table, and two of them may name a class alike. */
    key = Py_BuildValue("(NO)", PyLong_FromVoidPtr((void *)layout), base);
    if (key == NULL) {
        return NULL;
    }
    tmpl = PyDict_GetItemWithError(pycc_ext_exc_templates, key);
    if (tmpl != NULL || PyErr_Occurred()) {
        Py_DECREF(key);
        return Py_XNewRef(tmpl);
    }
    /* The descriptors but the layout's `__dict__` snapshot, which would
     * hide the exception's own `__dict__`. Never released: the type
     * points at them for the rest of the process. */
    while (layout->getsets != NULL && layout->getsets[count].name != NULL) {
        count++;
    }
    getsets = (PyGetSetDef *)PyMem_Calloc((size_t)count + 1, sizeof(PyGetSetDef));
    slots = (PyType_Slot *)PyMem_Calloc(7, sizeof(PyType_Slot));
    spec = (PyType_Spec *)PyMem_Calloc(1, sizeof(PyType_Spec));
    if (getsets == NULL || slots == NULL || spec == NULL) {
        Py_DECREF(key);
        PyErr_NoMemory();
        return NULL;
    }
    for (i = 0; i < count; i++) {
        if (strcmp(layout->getsets[i].name, "__dict__") != 0) {
            getsets[kept++] = layout->getsets[i];
        }
    }
    /* No `Py_tp_new`: the base's is inherited (the header comment). */
    slots[0].slot = Py_tp_dealloc;
    slots[0].pfunc = (void *)pycc_ext_exc_dealloc;
    slots[1].slot = Py_tp_traverse;
    slots[1].pfunc = (void *)pycc_ext_exc_traverse;
    slots[2].slot = Py_tp_clear;
    slots[2].pfunc = (void *)pycc_ext_exc_clear;
    slots[3].slot = Py_tp_getset;
    slots[3].pfunc = (void *)getsets;
    slots[4].slot = Py_tp_methods;
    slots[4].pfunc = (void *)pycc_ext_exc_methods;
    /* Over a base whose objects already carry an instance pointer (a
     * layout class's), the template adds nothing: the instance it gives
     * them is of its own, larger layout, in the same word. */
    payload = pycc_ext_exc_template_for((PyTypeObject *)base);
    if (payload != NULL) {
        payload = PyDict_GetItemWithError(pycc_ext_exc_payload_of, payload);
    }
    spec->name = "pycc.exception_layout";
    spec->basicsize = payload != NULL ? 0 : -(int)sizeof(void *);
    spec->itemsize = 0;
    spec->flags = Py_TPFLAGS_DEFAULT | Py_TPFLAGS_BASETYPE | Py_TPFLAGS_HAVE_GC;
    spec->slots = slots;
    tmpl = PyType_FromMetaclass(NULL, NULL, spec, base);
    if (tmpl == NULL) {
        Py_DECREF(key);
        return NULL;
    }
    capsule = PyCapsule_New((void *)layout, PYCC_EXC_LAYOUT_CAPSULE, NULL);
    if (capsule == NULL || PyDict_SetItem(pycc_ext_exc_layout_of, tmpl, capsule) < 0
        || PyDict_SetItem(pycc_ext_exc_payload_of, tmpl, payload != NULL ? payload : tmpl) < 0
        || PyDict_SetItem(pycc_ext_exc_templates, key, tmpl) < 0) {
        Py_XDECREF(capsule);
        Py_DECREF(key);
        Py_DECREF(tmpl);
        return NULL;
    }
    Py_DECREF(capsule);
    Py_DECREF(key);
    return tmpl;
}

#define PYCC_EXC_CAPSULE "pycc.module"

/* `__pycc_exc_bases__(hidden, *bases)`: `(template, *bases)` for the
 * class whose layout the hidden class `hidden` declares, the template over
 * its first exception base; the bases as they are when there is none. */
static PyObject *pycc_ext_exc_bases(PyObject *self, PyObject *const *args, Py_ssize_t nargs)
{
    PyccExtModule *m = (PyccExtModule *)PyCapsule_GetPointer(self, PYCC_EXC_CAPSULE);
    const PyccExtExcLayout *layout;
    const char *hidden;
    PyObject *base = NULL;
    PyObject *tmpl;
    PyObject *out;
    Py_ssize_t i;
    if (m == NULL) {
        return NULL;
    }
    if (nargs < 1 || !PyUnicode_Check(args[0])) {
        PyErr_SetString(PyExc_TypeError, "__pycc_exc_bases__: a layout name first");
        return NULL;
    }
    hidden = PyUnicode_AsUTF8AndSize(args[0], NULL);
    if (hidden == NULL) {
        return NULL;
    }
    layout = m->exc_layout(hidden);
    for (i = 1; i < nargs && layout != NULL; i++) {
        if (PyType_Check(args[i])
            && PyType_IsSubtype((PyTypeObject *)args[i], (PyTypeObject *)PyExc_BaseException)) {
            base = args[i];
            break;
        }
    }
    if (base == NULL) {
        out = PyTuple_New(nargs - 1);
        for (i = 1; out != NULL && i < nargs; i++) {
            PyTuple_SetItem(out, i - 1, Py_NewRef(args[i]));
        }
        return out;
    }
    tmpl = pycc_ext_exc_template(layout, base);
    if (tmpl == NULL) {
        return NULL;
    }
    out = PyTuple_New(nargs);
    if (out == NULL) {
        Py_DECREF(tmpl);
        return NULL;
    }
    PyTuple_SetItem(out, 0, tmpl);
    for (i = 1; i < nargs; i++) {
        PyTuple_SetItem(out, i, Py_NewRef(args[i]));
    }
    return out;
}

static PyMethodDef pycc_ext_exc_bases_def = {
    "__pycc_exc_bases__", (PyCFunction)(void (*)(void))pycc_ext_exc_bases, METH_FASTCALL, NULL,
};

/* Binds `__pycc_exc_bases__` in the namespace of `module`, of record `m`,
 * for the body (with `__pycc_forward__`); 0, or -1 with an exception set. */
static int pycc_ext_exc_hook_install(PyccExtModule *m, PyObject *module)
{
    PyObject *capsule = PyCapsule_New(m, PYCC_EXC_CAPSULE, NULL);
    PyObject *hook;
    int status;
    if (capsule == NULL) {
        return -1;
    }
    hook = PyCFunction_NewEx(&pycc_ext_exc_bases_def, capsule, NULL);
    Py_DECREF(capsule);
    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(PyModule_GetDict(module), "__pycc_exc_bases__", hook);
    Py_DECREF(hook);
    return status;
}
