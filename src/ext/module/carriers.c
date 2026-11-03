/* Carrier types (v0.9): the shared deallocator, the template root and the
 * compiled-class bookkeeping. Pickling a compiled instance follows in
 * carriers_pickle.c, `copy.copy` in instance_copy.c, the making of a
 * published class in class_maker.c, the instance boundary built on them in
 * instances.c. */

/*
 * The shared `Py_tp_dealloc` for every constructible class -- one function
 * rather than one per class, because none of it is class-dependent.
 *
 * The documented heap-type pattern: a `PyType_FromSpec` type is a heap type,
 * every instance of it holds a reference to it, and `tp_dealloc` is what
 * discharges that reference. Omitting the `Py_DECREF(tp)` leaks the type
 * object invisibly -- nothing observable fails, the module just never
 * releases its own types. `Py_tp_free` is fetched through `PyType_GetSlot`
 * because the limited API exposes no other way to reach the type's
 * deallocator, and it is called *before* the type reference is released so
 * `tp` is still live while the object is freed.
 *
 * The carrier's reference to its `PyInstanceObj` is released last, which
 * frees the instance when nothing compiled still holds it.
 */
/* CPython's `PyObject_CallFinalizerFromDealloc`, found at run time: in the
 * limited API only from 3.15, but exported by every supported interpreter
 * (see the dict-watcher lookup in global_sync.c for the two platforms). */
static void pycc_ext_instance_dealloc(PyObject *self);
#ifndef _WIN32
extern int PyObject_CallFinalizerFromDealloc(PyObject *self) __attribute__((weak));
#endif
static int (*pycc_ext_finalize_from_dealloc(void))(PyObject *)
{
    static int resolved;
    static int (*call)(PyObject *);

    if (!resolved) {
        resolved = 1;
#ifdef _WIN32
        {
            PyObject *handle = PySys_GetObject("dllhandle");
            void *dll = handle == NULL ? NULL : PyLong_AsVoidPtr(handle);

            PyErr_Clear();
            if (dll != NULL) {
                call = (int (*)(PyObject *))GetProcAddress(dll, "PyObject_CallFinalizerFromDealloc");
            }
        }
#else
        call = PyObject_CallFinalizerFromDealloc;
#endif
    }
    return call;
}

/* v0.12: a published class made once (class_once.c) has this function as
 * its own `tp_dealloc`, where the class `type()` made from its template
 * had `subtype_dealloc`, which runs the instance's `tp_finalize` first --
 * the slot `C.__del__ = f` installs (`update_slot`), as an instance of a
 * class of this module never gets one otherwise. So, when the object's
 * own type deallocates through here (not a Python subclass, whose
 * `subtype_dealloc` has already run it) and has a finalizer, it runs as
 * `subtype_dealloc` runs it: before the object is untracked (a compiled
 * class's own object already is, `pycc_ext_carrier_bind`, and stays so),
 * once per object (PEP 442), and an object the finalizer resurrected is
 * left alive.
 * Returns 1 when the object was resurrected. */
static int pycc_ext_instance_finalize(PyObject *self, PyTypeObject *tp)
{
    destructor finalize = (destructor)PyType_GetSlot(tp, Py_tp_finalize);
    int (*call)(PyObject *);

    if (finalize == NULL
        || PyType_GetSlot(tp, Py_tp_dealloc) != (void *)pycc_ext_instance_dealloc) {
        return 0;
    }
    call = pycc_ext_finalize_from_dealloc();
    if (call != NULL) {
        return call(self) < 0;
    }
    /* No such export: the same steps but for PEP 442's "finalized" mark,
     * which only that function can set. */
    if (PyType_IS_GC(tp) && PyObject_GC_IsFinalized(self)) {
        return 0;
    }
    Py_SET_REFCNT(self, 1);
    finalize(self);
    if (Py_REFCNT(self) == 1) {
        Py_SET_REFCNT(self, 0);
        return 0;
    }
    Py_SET_REFCNT(self, Py_REFCNT(self) - 1);
    return 1;
}

static void pycc_ext_instance_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst;
    if (pycc_ext_instance_finalize(self, tp)) {
        return;
    }
    inst = ((PyccExtInstance *)self)->inst;
    if (PyType_IS_GC(tp)) {
        PyObject_GC_UnTrack(self);
    }
    /* #1435: an instance that outlives its carrier must not keep naming it.
     * Cleared only when the back-pointer is this carrier: a carrier that a
     * later `tp_init` superseded (see `pycc_ext_carrier_bind`) no longer
     * owns the link. */
    if (inst != NULL && pycc_rt_ext_instance_carrier(inst) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(inst, NULL);
    }
    freefunc tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    tp_free(self);
    Py_DECREF(tp);
    if (inst != NULL) {
        pycc_rt_instance_decref(inst);
    }
}

/*
 * #1435: the carrier type of each class of module `m`, keyed by the class
 * name its instances' layout descriptors carry (`m->carrier_types`: class
 * names are unique within a module, not within a package's extension). A
 * published class's own type object, and the hidden carrier type of an
 * unpublished class with comparison slots (#1427) or field descriptors
 * (#1448), is entered by the generated `pycc_ext_register_method_types`
 * (`pycc_ext_carrier_register`); any other class -- one with neither --
 * gets a carrier type whose only method is the module's `__copy__` (#1455)
 * the first time one of its instances crosses (`pycc_ext_carrier_type`).
 * The dict holds a strong reference to every type for the rest of the
 * process, which the shim's refusal of subinterpreters licenses exactly as
 * it licenses every other file-scope static here.
 */
static int pycc_ext_carrier_types_ready(PyccExtModule *m)
{
    if (m->carrier_types == NULL) {
        m->carrier_types = PyDict_New();
    }
    return m->carrier_types == NULL ? -1 : 0;
}

/* Enters a published class's type object as its instances' carrier type.
 * Called from the generated registration only, before any compiled code
 * runs, so a later crossing finds the type that carries the class's
 * methods rather than creating one with only `__copy__`. Returns 0, or -1
 * with a CPython exception set. */
static int pycc_ext_note_compiled_class(PyObject *cls);

static int pycc_ext_carrier_register_in(PyccExtModule *m, const char *class_name, PyObject *type)
{
    if (pycc_ext_carrier_types_ready(m) < 0 || pycc_ext_note_compiled_class(type) < 0) {
        return -1;
    }
    return PyDict_SetItemString(m->carrier_types, class_name, type);
}

/* v0.12: drops the carrier type of a published class made at its
 * statement, as each exec starts (`pycc_ext_register_method_types`), so an
 * instance crossing before the statement runs again is carried by the
 * class's template rather than by the class an earlier exec made. Returns
 * 0, or -1 with an exception set. */
static int pycc_ext_carrier_forget_in(PyccExtModule *m, const char *class_name)
{
    if (m->carrier_types == NULL || PyDict_GetItemString(m->carrier_types, class_name) == NULL) {
        return 0;
    }
    return PyDict_DelItemString(m->carrier_types, class_name);
}

/* Stores the instance a successful `tp_init` built and records `self` as
 * its carrier (#1435), so `self` handed out by a method of a
 * host-constructed object is that very object. A re-run `__init__`
 * (`obj.__init__(...)`) replaces the instance; the one it replaces stops
 * naming `self`, because nothing would clear that link once `self` dies and
 * the old instance may still be reachable from compiled code. */
static int pycc_ext_is_compiled_class(PyObject *cls);

static void pycc_ext_carrier_bind(PyObject *self, void *inst)
{
    void *previous = ((PyccExtInstance *)self)->inst;
    int previous_strong = 0;
    if (previous != NULL && pycc_rt_ext_instance_carrier(previous) == (void *)self) {
        previous_strong = pycc_rt_ext_instance_carrier_is_strong(previous);
        pycc_rt_ext_instance_set_carrier(previous, NULL);
    }
    ((PyccExtInstance *)self)->inst = inst;
    /* v0.9: the object of a Python subclass of a compiled class carries
     * state of its own -- its type, its `__dict__` -- so the instance keeps
     * it alive (a strong link) for as long as compiled code holds the
     * instance; the cycle is the template's to collect
     * (`pycc_ext_carrier_traverse`). */
    if (pycc_ext_is_compiled_class((PyObject *)Py_TYPE(self))) {
        pycc_rt_ext_instance_set_carrier(inst, self);
        PyObject_GC_UnTrack(self);
    } else {
        Py_INCREF(self);
        pycc_rt_ext_instance_set_carrier_strong(inst, self);
    }
    /* The carrier adopted the reference `tp_init` built `inst` with, and
     * drops the one it held to the instance it replaces. */
    if (previous != NULL) {
        pycc_rt_instance_decref(previous);
    }
    if (previous_strong) {
        Py_DECREF(self);
    }
}

/*
 * v0.9: garbage collection of a Python subclass's object and the instance
 * it carries, which hold each other (`pycc_ext_carrier_bind`). The cycle is
 * the object's own reference held by its instance, reported to the
 * collector exactly while nothing else -- no compiled code -- holds the
 * instance (its count is the object's one reference); `tp_clear` breaks
 * it. Installed on the templates without `Py_TPFLAGS_HAVE_GC`, so a
 * compiled class's own objects stay untracked: only a Python subclass,
 * which CPython makes a collected type for its `__dict__`, reaches them,
 * through `subtype_traverse`.
 */
static int pycc_ext_carrier_traverse(PyObject *self, visitproc visit, void *arg)
{
    void *inst = ((PyccExtInstance *)self)->inst;
    if (inst != NULL && pycc_rt_ext_instance_carrier_is_strong(inst)
        && pycc_rt_ext_instance_carrier(inst) == (void *)self
        && pycc_rt_ext_instance_refcount(inst) == 1) {
        Py_VISIT(self);
    }
    return 0;
}

static int pycc_ext_carrier_clear(PyObject *self)
{
    void *inst = ((PyccExtInstance *)self)->inst;
    if (inst != NULL && pycc_rt_ext_instance_carrier_is_strong(inst)
        && pycc_rt_ext_instance_carrier(inst) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(inst, self);
        Py_DECREF(self);
    }
    return 0;
}

/*
 * v0.9: published classes with their real bases. Each published class's
 * `PyType_FromSpec` type is a *template* -- its layout (a carrier), slots,
 * methods and field descriptors -- and the class CPython code sees is made
 * from it when the class statement runs (`pycc_ext_publish`): its
 * metaclass called with `(template, *bases)`, the statement's own bases
 * evaluated in the module's namespace and resolved as `__build_class__`
 * resolves them (`__mro_entries__`, `__orig_bases__`, the metaclass and its
 * `__prepare__`), `__slots__ = ()` so instances keep the template's layout.
 * CPython runs the class machinery -- the MRO, `__init_subclass__`
 * (`Generic`'s), `__set_name__` -- and the class is entered as its
 * instances' carrier type. Every template derives from one root carrier
 * type (or from its compiled base's template), so templates of classes
 * combined by multiple inheritance share their instance layout.
 */

/* The root of every template: the carrier layout, nothing else. */
static PyObject *pycc_ext_template_root = NULL;

/*
 * v0.10: `cls.__pycc_reconstruct__()`, what protocols 0 and 1 unpickle a
 * compiled instance through in place of `copyreg._reconstructor`: the
 * object made by the `tp_new` of the first compiled class in `cls`'s MRO,
 * with an unassigned instance, without calling a Python subclass's
 * `__new__` (as `object.__new__(cls)` does for a source class). A class
 * method of the root, so pickle saves it as `getattr(cls, name)`.
 */
static PyObject *pycc_ext_reconstruct(PyObject *cls, PyObject *unused)
{
    PyObject *mro = PyObject_GetAttrString(cls, "__mro__");
    PyObject *result = NULL;
    Py_ssize_t n;
    Py_ssize_t i;
    (void)unused;
    if (mro == NULL) {
        return NULL;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        newfunc make;
        if (c == pycc_ext_template_root || !pycc_ext_is_compiled_class(c)) {
            continue;
        }
        make = (newfunc)PyType_GetSlot((PyTypeObject *)c, Py_tp_new);
        if (make != NULL) {
            PyObject *none = PyTuple_New(0);
            if (none != NULL) {
                result = make((PyTypeObject *)cls, none, NULL);
                Py_DECREF(none);
            }
            Py_DECREF(mro);
            return result;
        }
    }
    Py_DECREF(mro);
    PyErr_SetString(PyExc_TypeError, "__pycc_reconstruct__ needs a constructible compiled class");
    return NULL;
}

static PyMethodDef pycc_ext_template_root_methods[] = {
    {"__pycc_reconstruct__", (PyCFunction)(void (*)(void))pycc_ext_reconstruct,
     METH_NOARGS | METH_CLASS, NULL},
    {NULL, NULL, 0, NULL},
};

static PyType_Slot pycc_ext_template_root_slots[] = {
    {Py_tp_methods, (void *)pycc_ext_template_root_methods},
    {Py_tp_dealloc, (void *)pycc_ext_instance_dealloc},
    {Py_tp_traverse, (void *)pycc_ext_carrier_traverse},
    {Py_tp_clear, (void *)pycc_ext_carrier_clear},
    {0, NULL},
};

/* `Py_TPFLAGS_HAVE_GC`, which every template and published class
 * inherits, so a Python subclass's `subtype_traverse` reaches
 * `pycc_ext_carrier_traverse`; a compiled class's own object is untracked
 * as soon as it is made (`tp_init`, a crossing, a copy), so the collector
 * never scans it. Every carrier now carries the GC pre-header (two words)
 * and goes through the GC allocator: an allocation-heavy benchmark that
 * regresses should look here first. */
static PyType_Spec pycc_ext_template_root_spec = {
    "pycc.compiled",
    0,
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_BASETYPE | Py_TPFLAGS_IMMUTABLETYPE
        | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_HAVE_GC,
    pycc_ext_template_root_slots,
};

/* The base a template without a published compiled base derives from (a
 * borrowed reference), or NULL with an exception set. */
static PyObject *pycc_ext_template_base(void)
{
    if (pycc_ext_template_root == NULL) {
        pycc_ext_template_root_spec.basicsize = (int)sizeof(PyccExtInstance);
        pycc_ext_template_root = PyType_FromSpec(&pycc_ext_template_root_spec);
    }
    return pycc_ext_template_root;
}

/* Whether `type` is a carrier type of this artifact: one whose instances
 * deallocate through this file's `pycc_ext_instance_dealloc` (a template,
 * a hidden carrier type), or a published class made from a template, whose
 * own deallocator is CPython's `subtype_dealloc`. */
static int pycc_ext_is_carrier_type(PyTypeObject *type)
{
    return PyType_GetSlot(type, Py_tp_dealloc) == (void *)pycc_ext_instance_dealloc
           || (pycc_ext_template_root != NULL
               && PyType_IsSubtype(type, (PyTypeObject *)pycc_ext_template_root));
}

/* Whether a published class is being made from its template. */
static int pycc_ext_making_public_class = 0;

/* v0.9: every class this artifact compiled -- the root, each template and
 * hidden carrier type (`pycc_ext_carrier_register_in`), each published
 * class made from a template -- so a Python subclass of one is told apart
 * from the classes whose methods are compiled. Held for the rest of the
 * process, as the carrier types are. */
static PyObject *pycc_ext_compiled_classes = NULL;

static int pycc_ext_note_compiled_class(PyObject *cls)
{
    if (pycc_ext_compiled_classes == NULL) {
        pycc_ext_compiled_classes = PySet_New(NULL);
        if (pycc_ext_compiled_classes == NULL) {
            return -1;
        }
    }
    return PySet_Add(pycc_ext_compiled_classes, cls);
}

static int pycc_ext_is_compiled_class(PyObject *cls)
{
    return cls == pycc_ext_template_root
           || (pycc_ext_compiled_classes != NULL
               && PySet_Contains(pycc_ext_compiled_classes, cls) == 1);
}

/* v0.10: whether `cls` is compiled -- a compiled class, or a carrier type
 * whose instances deallocate through this file (a hidden carrier type) --
 * rather than a Python class in a carrier's MRO. */
static int pycc_ext_is_compiled_type(PyObject *cls)
{
    return pycc_ext_is_compiled_class(cls)
           || (PyType_Check(cls)
               && PyType_GetSlot((PyTypeObject *)cls, Py_tp_dealloc)
                      == (void *)pycc_ext_instance_dealloc);
}

/*
 * v0.10: the shared pickling and copying defaults the compiled classes
 * carry (`__reduce_ex__`, `__getstate__`, `__setstate__`, `__copy__`) stand
 * where CPython's sit for the source class: at `object`, the end of the MRO.
 * A Python class after the compiled ones in `type(self).__mro__` (a mixin
 * listed after the compiled base) that defines `name` therefore wins: this
 * answers its `name` bound to `self` (a new reference), or NULL with no
 * exception set when none does. Classes before the first compiled one are
 * not consulted: one defining `name` is found before the default is, and
 * reaches it only through `super()`.
 */
static PyObject *pycc_ext_object_position_hook(PyObject *self, const char *name)
{
    PyObject *mro = PyObject_GetAttrString((PyObject *)Py_TYPE(self), "__mro__");
    Py_ssize_t n;
    Py_ssize_t i;
    int past_compiled = 0;
    if (mro == NULL) {
        return NULL;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        PyObject *dict;
        PyObject *hook;
        descrgetfunc get;
        if (pycc_ext_is_compiled_type(c)) {
            past_compiled = 1;
            continue;
        }
        if (!past_compiled || c == (PyObject *)&PyBaseObject_Type) {
            continue;
        }
        dict = PyObject_GetAttrString(c, "__dict__");
        if (dict == NULL) {
            Py_DECREF(mro);
            return NULL;
        }
        hook = PyMapping_GetItemString(dict, name);
        Py_DECREF(dict);
        if (hook == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_KeyError)) {
                Py_DECREF(mro);
                return NULL;
            }
            PyErr_Clear();
            continue;
        }
        Py_DECREF(mro);
        get = (descrgetfunc)PyType_GetSlot(Py_TYPE(hook), Py_tp_descr_get);
        if (get != NULL) {
            PyObject *bound = get(hook, self, (PyObject *)Py_TYPE(self));
            Py_DECREF(hook);
            return bound;
        }
        return hook;
    }
    Py_DECREF(mro);
    return NULL;
}

/*
 * v0.9: a Python subclass's object that never runs the compiled class's
 * `__init__` starts with the class's instance default `name` (`None`,
 * `pycc_ast`'s instance defaults) in the slot -- or with the value the
 * subclass's own class body rebinds `name` to, which CPython reads before
 * the base's, so compiled code reading the slot reads it too; called again
 * once the compiled `__init__` ran for the object (`rebound_only`), which
 * stored the class's default. Never sets an exception.
 */
PYCC_SHIM void pycc_ext_store_instance_default(PyObject *self, const char *name, int rebound_only)
{
    PyObject *mro = PyObject_GetAttrString((PyObject *)Py_TYPE(self), "__mro__");
    PyObject *value = NULL;
    Py_ssize_t i;

    if (mro == NULL || !PyTuple_Check(mro)) {
        Py_XDECREF(mro);
        PyErr_Clear();
        return;
    }
    for (i = 0; i < PyTuple_Size(mro); i++) {
        PyObject *cls = PyTuple_GetItem(mro, i);
        PyObject *dict = PyObject_GetAttrString(cls, "__dict__");
        PyObject *found = dict == NULL ? NULL : PyMapping_GetItemString(dict, name);
        Py_XDECREF(dict);
        if (found == NULL) {
            PyErr_Clear();
            continue;
        }
        if (!pycc_ext_is_compiled_class(cls)) {
            /* The subclass's value, kept for the compiled slot below. */
            if (value == NULL) {
                value = found;
            }
            else {
                Py_DECREF(found);
            }
            continue;
        }
        /* The compiled class's slot descriptor (`_SlotDefault` or the
         * template's `getset_descriptor`) stores the value. After the
         * compiled `__init__` ran (`rebound_only`), only a subclass's value
         * replaces the default it stored, never a value it stored. */
        if (rebound_only) {
            PyObject *current = value == NULL ? NULL
                : PyObject_CallMethod(found, "__get__", "OO", self, (PyObject *)Py_TYPE(self));
            int keep = current == NULL || current != Py_None;
            Py_XDECREF(current);
            if (keep) {
                Py_DECREF(found);
                break;
            }
        }
        {
            PyObject *stored = PyObject_CallMethod(found, "__set__", "OO", self,
                                                   value != NULL ? value : Py_None);
            Py_XDECREF(stored);
        }
        Py_DECREF(found);
        break;
    }
    Py_XDECREF(value);
    Py_DECREF(mro);
    PyErr_Clear();
}

/*
 * v0.9: whether the object carrying `inst` -- an instance of a Python
 * subclass of a compiled class -- resolves `name` to an attribute of a
 * class that is not compiled: a Python override, which a compiled call of
 * the method must run instead of the compiled body. The first class along
 * the object's MRO whose `__dict__` holds `name` decides. 0 for an instance
 * no Python subclass carries; never sets an exception.
 */
PYCC_SHIM int pycc_ext_instance_py_override(void *inst, const char *name)
{
    PyObject *carrier;
    PyObject *mro;
    PyObject *key;
    Py_ssize_t i;
    int found = 0;

    if (!pycc_rt_ext_instance_carrier_is_strong(inst)) {
        return 0;
    }
    carrier = (PyObject *)pycc_rt_ext_instance_carrier(inst);
    mro = PyObject_GetAttrString((PyObject *)Py_TYPE(carrier), "__mro__");
    key = PyUnicode_InternFromString(name);
    if (mro == NULL || key == NULL || !PyTuple_Check(mro)) {
        Py_XDECREF(mro);
        Py_XDECREF(key);
        PyErr_Clear();
        return 0;
    }
    for (i = 0; i < PyTuple_Size(mro); i++) {
        PyObject *cls = PyTuple_GetItem(mro, i);
        PyObject *dict = PyObject_GetAttrString(cls, "__dict__");
        int has = dict == NULL ? 0 : PyMapping_HasKey(dict, key);
        Py_XDECREF(dict);
        if (has) {
            found = !pycc_ext_is_compiled_class(cls);
            break;
        }
    }
    Py_DECREF(mro);
    Py_DECREF(key);
    PyErr_Clear();
    return found;
}

/*
 * A template's `__init_subclass__`: calls the next one along the subclass's
 * MRO past the root (`Generic`'s, which records the type parameters), for
 * the published class made from it and, since v0.9, for a Python subclass
 * too, whose overrides compiled calls run (`pycc_ext_instance_py_override`).
 */
static PyObject *pycc_ext_template_init_subclass(PyObject *cls, PyObject *args, PyObject *kwargs)
{
    PyObject *mro;
    PyObject *template;
    PyObject *super;
    PyObject *next;
    PyObject *result;
    PyObject *base_args[2];

    mro = PyObject_GetAttrString(cls, "__mro__");
    if (mro == NULL) {
        return NULL;
    }
    if (!PyTuple_Check(mro) || PyTuple_Size(mro) < 3) {
        Py_DECREF(mro);
        PyErr_SetString(PyExc_SystemError, "pycc: a compiled class's subclass has no MRO");
        return NULL;
    }
    /* `super(root, cls).__init_subclass__(*args, **kwargs)`: every
     * template's hook shares this body, so the call skips past the root
     * every template derives from, to the bases CPython code wrote
     * (`Generic`'s hook). */
    template = pycc_ext_template_root;
    base_args[0] = template;
    base_args[1] = cls;
    super = PyObject_Vectorcall((PyObject *)&PySuper_Type, base_args, 2, NULL);
    Py_DECREF(mro);
    if (super == NULL) {
        return NULL;
    }
    next = PyObject_GetAttrString(super, "__init_subclass__");
    Py_DECREF(super);
    if (next == NULL) {
        return NULL;
    }
    result = PyObject_Call(next, args, kwargs);
    Py_DECREF(next);
    return result;
}

