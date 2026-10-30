/* Carrier types (v0.9): the shared deallocator, the template root and the
 * compiled-class bookkeeping. The making of a published class from its
 * template follows in class_maker.c, the instance boundary built on them in
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
static void pycc_ext_instance_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst = ((PyccExtInstance *)self)->inst;
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
 * v0.10: pickling a compiled instance, as CPython's `object.__reduce_ex__`
 * pickles an object (`reduce_newobj`). `__reduce__` answers
 * `(copyreg.__newobj__, (type(self), *newargs), self.__getstate__())`, the
 * new arguments from `__getnewargs_ex__` / `__getnewargs__` when the type
 * defines them (`copyreg.__newobj_ex__` for keyword arguments). Unpickling
 * calls `cls.__new__(cls, *newargs)`: a compiled class's `tp_new` makes the
 * object with an instance whose slots are all unassigned, whatever its
 * arguments (a constructor call runs `tp_init` after it, which replaces
 * that instance), so a user-defined `__new__` sees what it sees in CPython.
 * Protocols 0 and 1 reduce as `copyreg._reduce_ex` does, through
 * `pycc_ext_reconstruct` in place of `copyreg._reconstructor`. The state is
 * `__getstate__`'s: the shared default below is `object.__getstate__`'s (a
 * Python subclass's `__dict__` and its own `__slots__`) with the compiled
 * attributes added to the slot state, which pickle sets through their
 * setters; a subclass that defines `__getstate__` / `__setstate__` has them
 * used as in CPython.
 */

static PyObject *pycc_ext_newobj = NULL;
static PyObject *pycc_ext_newobj_ex = NULL;
static PyObject *pycc_ext_object_getstate = NULL;

/* `TypeError: cannot pickle '<qualified name>' object<detail>`, with the
 * attribute `name` Python code cannot see when it is not NULL. */
static void pycc_ext_refuse_pickle(PyObject *self, const char *detail, PyObject *name)
{
    PyObject *type_name = PyType_GetFullyQualifiedName(Py_TYPE(self));
    if (type_name == NULL) {
        return;
    }
    if (name != NULL) {
        PyErr_Format(PyExc_TypeError,
                     "cannot pickle '%U' object: its attribute '%U' is not visible to Python",
                     type_name, name);
    } else {
        PyErr_Format(PyExc_TypeError, "cannot pickle '%U' object%s", type_name, detail);
    }
    Py_DECREF(type_name);
}

/* The data descriptor the compiled classes of `self`'s MRO define for
 * compiled attribute `key` (a new reference), found in their own
 * namespaces: not through `self`'s `__getattribute__` or `__setattr__`,
 * which a Python subclass may override, nor through a subclass's class
 * attribute of the same name. Where the source class keeps the attribute in
 * the instance's `__dict__`, which `object.__getstate__` and pickle's
 * default restore read and write directly, this is the storage. NULL with
 * no exception set: no such descriptor (Python code cannot see it). */
static PyObject *pycc_ext_compiled_descriptor(PyObject *self, PyObject *key)
{
    PyObject *mro = PyObject_GetAttrString((PyObject *)Py_TYPE(self), "__mro__");
    Py_ssize_t n;
    Py_ssize_t i;
    if (mro == NULL) {
        return NULL;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        PyObject *dict;
        PyObject *descr;
        if (!pycc_ext_is_compiled_type(c)) {
            continue;
        }
        dict = PyObject_GetAttrString(c, "__dict__");
        if (dict == NULL) {
            Py_DECREF(mro);
            return NULL;
        }
        descr = PyObject_GetItem(dict, key);
        Py_DECREF(dict);
        if (descr == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_KeyError)) {
                Py_DECREF(mro);
                return NULL;
            }
            PyErr_Clear();
            continue;
        }
        if (PyType_GetSlot(Py_TYPE(descr), Py_tp_descr_get) == NULL
            || PyType_GetSlot(Py_TYPE(descr), Py_tp_descr_set) == NULL) {
            /* A class-level default: the descriptor sits further up. */
            Py_DECREF(descr);
            continue;
        }
        Py_DECREF(mro);
        return descr;
    }
    Py_DECREF(mro);
    return NULL;
}

/* The stored value of compiled attribute `key` (NULL with `AttributeError`
 * when it is unassigned; NULL with no exception: no descriptor). */
static PyObject *pycc_ext_compiled_field(PyObject *self, PyObject *key)
{
    PyObject *descr = pycc_ext_compiled_descriptor(self, key);
    PyObject *value;
    if (descr == NULL) {
        return NULL;
    }
    value = ((descrgetfunc)PyType_GetSlot(Py_TYPE(descr), Py_tp_descr_get))(
        descr, self, (PyObject *)Py_TYPE(self));
    Py_DECREF(descr);
    return value;
}

/*
 * The shared default `__getstate__`: what `object.__getstate__` answers for
 * the source class, whose compiled attributes live in the instance's
 * `__dict__` -- that dictionary (each assigned compiled attribute as stored,
 * then a Python subclass's own `__dict__` entries), or None when it is
 * empty, paired with a Python subclass's own `__slots__` values when it has
 * any. `pycc_ext_instance_setstate` is its mirror.
 */
static PyObject *pycc_ext_instance_getstate_default(PyccExtModule *m, PyObject *self)
{
    void *inst = ((PyccExtInstance *)self)->inst;
    const unsigned char *layout;
    const unsigned char *end;
    const unsigned char *field;
    size_t len = 0;
    long long count;
    long long i;
    PyObject *base = NULL;
    PyObject *extra = NULL;
    PyObject *slots = NULL;
    PyObject *fields = NULL;
    PyObject *dict_state;
    PyObject *result = NULL;
    const char *kinds = NULL;
    size_t nkinds = 0;
    const char *refused = NULL;

    if (inst == NULL) {
        PyObject *type_name = PyType_GetFullyQualifiedName(Py_TYPE(self));
        if (type_name != NULL) {
            PyErr_Format(PyExc_TypeError, "cannot pickle an uninitialized '%U' object",
                         type_name);
            Py_DECREF(type_name);
        }
        return NULL;
    }
    /* A class `copy.copy` refuses (a generic class's specializations share
     * one layout) is refused here too, for the same reason. */
    {
        const unsigned char *cls = pycc_rt_ext_instance_class(inst, &len);
        if (m->carrier_class_copy_kinds(cls, len, &kinds, &nkinds, &refused) == 0) {
            PyObject *type_name = PyType_GetFullyQualifiedName(Py_TYPE(self));
            if (type_name != NULL) {
                PyErr_Format(PyExc_TypeError, "cannot pickle '%U' object: %s", type_name,
                             refused);
                Py_DECREF(type_name);
            }
            return NULL;
        }
    }
    if (pycc_ext_object_getstate == NULL) {
        pycc_ext_object_getstate =
            PyObject_GetAttrString((PyObject *)&PyBaseObject_Type, "__getstate__");
        if (pycc_ext_object_getstate == NULL) {
            return NULL;
        }
    }
    fields = PyDict_New();
    if (fields == NULL) {
        return NULL;
    }
    /* The slot names follow the class name in the descriptor, NUL-
     * separated; each assigned one's value is read as stored. */
    layout = pycc_rt_ext_instance_layout(inst, &len);
    count = pycc_rt_ext_instance_slot_count(inst);
    end = layout + len;
    field = (const unsigned char *)memchr(layout, 0, len);
    for (i = 0; i < count && field != NULL && field < end; i++) {
        PyObject *key;
        PyObject *value;
        const unsigned char *next;
        field += 1;
        next = (const unsigned char *)memchr(field, 0, (size_t)(end - field));
        key = PyUnicode_FromStringAndSize((const char *)field,
                                          (Py_ssize_t)((next != NULL ? next : end) - field));
        if (key == NULL) {
            goto done;
        }
        /* A slot Python code cannot see (no descriptor on the class: a
         * container of native values, say) cannot be pickled. */
        value = pycc_ext_compiled_field(self, key);
        if (value == NULL && !PyErr_Occurred()) {
            pycc_ext_refuse_pickle(self, "", key);
            Py_DECREF(key);
            goto done;
        }
        if (value == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_AttributeError)) {
                Py_DECREF(key);
                goto done;
            }
            PyErr_Clear();
        } else {
            int stored = PyDict_SetItem(fields, key, value);
            Py_DECREF(value);
            if (stored < 0) {
                Py_DECREF(key);
                goto done;
            }
        }
        Py_DECREF(key);
        field = next;
    }
    /* `object.__getstate__`: None, a Python subclass's `__dict__`, or
     * `(dict, slots)` for its own `__slots__` (the compiled classes declare
     * `__slots__ = ()`). */
    base = PyObject_CallFunctionObjArgs(pycc_ext_object_getstate, self, NULL);
    if (base == NULL) {
        goto done;
    }
    if (PyTuple_Check(base) && PyTuple_Size(base) == 2) {
        extra = PyTuple_GetItem(base, 0);
        slots = PyTuple_GetItem(base, 1);
    } else {
        extra = base;
    }
    if (extra != Py_None && PyDict_Update(fields, extra) < 0) {
        goto done;
    }
    dict_state = PyDict_Size(fields) > 0 ? fields : Py_None;
    if (slots != NULL && PyDict_Check(slots) && PyDict_Size(slots) > 0) {
        result = PyTuple_Pack(2, dict_state, slots);
    } else {
        result = Py_NewRef(dict_state);
    }
done:
    Py_XDECREF(base);
    Py_DECREF(fields);
    return result;
}

/*
 * The shared default `__setstate__`, the mirror of
 * `pycc_ext_instance_getstate_in` and of pickle's default restore (its
 * `BUILD`, `copy._reconstruct`): `state` is a dictionary or None, or a pair
 * of it and a slot dictionary. A dictionary entry naming a compiled
 * attribute is stored through the compiled descriptor (where the source
 * class's `__dict__` holds it), any other one into the instance's
 * `__dict__` directly; a slot entry is set with `setattr`, as pickle sets
 * one.
 */
static PyObject *pycc_ext_instance_setstate_default(PyObject *self, PyObject *state)
{
    PyObject *dict_state = state;
    PyObject *slot_state = NULL;
    PyObject *key;
    PyObject *value;
    Py_ssize_t pos = 0;

    if (PyTuple_Check(state) && PyTuple_Size(state) == 2) {
        dict_state = PyTuple_GetItem(state, 0);
        slot_state = PyTuple_GetItem(state, 1);
    }
    if (dict_state != Py_None) {
        PyObject *items;
        PyObject *dict = NULL;
        Py_ssize_t n;
        Py_ssize_t i;
        if (!PyDict_Check(dict_state)) {
            PyErr_SetString(PyExc_TypeError, "state is not a dictionary");
            return NULL;
        }
        items = PyDict_Items(dict_state);
        if (items == NULL) {
            return NULL;
        }
        n = PyList_Size(items);
        for (i = 0; i < n; i++) {
            PyObject *item = PyList_GetItem(items, i);
            PyObject *descr = NULL;
            int failed;
            key = PyTuple_GetItem(item, 0);
            value = PyTuple_GetItem(item, 1);
            if (PyUnicode_Check(key)) {
                descr = pycc_ext_compiled_descriptor(self, key);
                if (descr == NULL && PyErr_Occurred()) {
                    failed = -1;
                    goto item_done;
                }
            }
            if (descr != NULL) {
                failed = ((descrsetfunc)PyType_GetSlot(Py_TYPE(descr), Py_tp_descr_set))(
                    descr, self, value);
                Py_DECREF(descr);
            } else {
                if (dict == NULL) {
                    dict = PyObject_GetAttrString(self, "__dict__");
                    if (dict == NULL) {
                        failed = -1;
                        goto item_done;
                    }
                }
                if (PyUnicode_Check(key)) {
                    Py_INCREF(key);
                    PyUnicode_InternInPlace(&key);
                    failed = PyObject_SetItem(dict, key, value);
                    Py_DECREF(key);
                } else {
                    failed = PyObject_SetItem(dict, key, value);
                }
            }
        item_done:
            if (failed < 0) {
                Py_XDECREF(dict);
                Py_DECREF(items);
                return NULL;
            }
        }
        Py_XDECREF(dict);
        Py_DECREF(items);
    }
    if (slot_state != NULL && slot_state != Py_None) {
        if (!PyDict_Check(slot_state)) {
            PyErr_SetString(PyExc_TypeError, "slot state is not a dictionary");
            return NULL;
        }
        while (PyDict_Next(slot_state, &pos, &key, &value)) {
            if (PyObject_SetAttr(self, key, value) < 0) {
                return NULL;
            }
        }
    }
    Py_RETURN_NONE;
}

/* `copyreg.<name>`, cached in `*slot`. */
static PyObject *pycc_ext_copyreg(PyObject **slot, const char *name)
{
    if (*slot == NULL) {
        PyObject *copyreg = PyImport_ImportModule("copyreg");
        if (copyreg == NULL) {
            return NULL;
        }
        *slot = PyObject_GetAttrString(copyreg, name);
        Py_DECREF(copyreg);
    }
    return *slot;
}

/* The new arguments `reduce_newobj` passes: `__getnewargs_ex__`'s pair,
 * else `__getnewargs__`'s tuple, else none. Stores new references. */
static int pycc_ext_newargs(PyObject *self, PyObject **newargs, PyObject **kwargs)
{
    PyObject *hook = NULL;
    *newargs = NULL;
    *kwargs = NULL;
    if (PyObject_GetOptionalAttrString(self, "__getnewargs_ex__", &hook) < 0) {
        return -1;
    }
    if (hook != NULL) {
        PyObject *pair = PyObject_CallNoArgs(hook);
        Py_DECREF(hook);
        if (pair == NULL) {
            return -1;
        }
        if (!PyTuple_Check(pair) || PyTuple_Size(pair) != 2
            || !PyTuple_Check(PyTuple_GetItem(pair, 0))
            || !PyDict_Check(PyTuple_GetItem(pair, 1))) {
            PyErr_SetString(PyExc_TypeError,
                            "__getnewargs_ex__ should return a tuple of length 2 holding a "
                            "tuple and a dict");
            Py_DECREF(pair);
            return -1;
        }
        *newargs = Py_NewRef(PyTuple_GetItem(pair, 0));
        *kwargs = Py_NewRef(PyTuple_GetItem(pair, 1));
        Py_DECREF(pair);
        return 0;
    }
    if (PyObject_GetOptionalAttrString(self, "__getnewargs__", &hook) < 0) {
        return -1;
    }
    if (hook == NULL) {
        *newargs = PyTuple_New(0);
        return *newargs == NULL ? -1 : 0;
    }
    *newargs = PyObject_CallNoArgs(hook);
    Py_DECREF(hook);
    if (*newargs == NULL) {
        return -1;
    }
    if (!PyTuple_Check(*newargs)) {
        PyErr_SetString(PyExc_TypeError, "__getnewargs__ should return a tuple");
        Py_CLEAR(*newargs);
        return -1;
    }
    return 0;
}

static PyObject *pycc_ext_object_reduce = NULL;

/* Whether a Python class in `cls`'s MRO (but `object`) defines
 * `__getstate__` (otherwise the compiled class's default is the one). */
static int pycc_ext_getstate_overridden(PyObject *cls)
{
    PyObject *mro = PyObject_GetAttrString(cls, "__mro__");
    Py_ssize_t n;
    Py_ssize_t i;
    int found = 0;
    if (mro == NULL) {
        return -1;
    }
    n = PyTuple_Check(mro) ? PyTuple_Size(mro) : 0;
    for (i = 0; i < n && found == 0; i++) {
        PyObject *c = PyTuple_GetItem(mro, i);
        PyObject *dict;
        if (pycc_ext_is_compiled_type(c) || c == (PyObject *)&PyBaseObject_Type) {
            continue;
        }
        dict = PyObject_GetAttrString(c, "__dict__");
        if (dict == NULL) {
            found = -1;
            break;
        }
        found = PyMapping_HasKeyStringWithError(dict, "__getstate__");
        Py_DECREF(dict);
    }
    Py_DECREF(mro);
    return found;
}

/* `__reduce_ex__(proto)`: a subclass's own `__reduce__` wins, as in
 * `object.__reduce_ex__`; protocols 0 and 1 refuse an object whose class
 * declares `__slots__` without a `__getstate__` of its own, as
 * `copyreg._reduce_ex` does. */
static PyObject *pycc_ext_instance_reduce_default(PyObject *self, PyObject *proto)
{
    PyObject *cls = (PyObject *)Py_TYPE(self);
    PyObject *reduce;
    PyObject *newargs;
    PyObject *kwargs;
    PyObject *callable;
    PyObject *call_args = NULL;
    PyObject *state = NULL;
    PyObject *result = NULL;
    Py_ssize_t n;
    Py_ssize_t i;
    long protocol;

    if (pycc_ext_object_reduce == NULL) {
        pycc_ext_object_reduce =
            PyObject_GetAttrString((PyObject *)&PyBaseObject_Type, "__reduce__");
        if (pycc_ext_object_reduce == NULL) {
            return NULL;
        }
    }
    reduce = PyObject_GetAttrString(cls, "__reduce__");
    if (reduce == NULL) {
        return NULL;
    }
    Py_DECREF(reduce);
    if (reduce != pycc_ext_object_reduce) {
        return PyObject_CallMethod(self, "__reduce__", NULL);
    }
    protocol = PyLong_AsLong(proto);
    if (protocol == -1 && PyErr_Occurred()) {
        return NULL;
    }
    if (protocol < 2) {
        PyObject *declared = NULL;
        int truthy;
        int overridden = pycc_ext_getstate_overridden(cls);
        if (overridden < 0) {
            return NULL;
        }
        if (PyObject_GetOptionalAttrString(self, "__slots__", &declared) < 0) {
            return NULL;
        }
        truthy = declared != NULL ? PyObject_IsTrue(declared) : 0;
        Py_XDECREF(declared);
        if (truthy < 0) {
            return NULL;
        }
        if (!overridden && truthy) {
            PyErr_SetString(PyExc_TypeError, "a class that defines __slots__ without "
                                             "defining __getstate__ cannot be pickled");
            return NULL;
        }
        /* `copyreg._reduce_ex`'s `(_reconstructor, (cls, object, None),
         * state)`: the object is made without the class's `__new__` and
         * without new arguments (`pycc_ext_reconstruct`), the state set
         * only when it is true. */
        callable = PyObject_GetAttrString(cls, "__pycc_reconstruct__");
        if (callable == NULL) {
            return NULL;
        }
        state = PyObject_CallMethod(self, "__getstate__", NULL);
        if (state == NULL) {
            Py_DECREF(callable);
            return NULL;
        }
        truthy = PyObject_IsTrue(state);
        if (truthy > 0) {
            result = Py_BuildValue("(O()O)", callable, state);
        } else if (truthy == 0) {
            result = Py_BuildValue("(O())", callable);
        }
        Py_DECREF(state);
        Py_DECREF(callable);
        return result;
    }
    if (pycc_ext_newargs(self, &newargs, &kwargs) < 0) {
        return NULL;
    }
    if (kwargs != NULL && PyDict_Size(kwargs) > 0) {
        callable = pycc_ext_copyreg(&pycc_ext_newobj_ex, "__newobj_ex__");
        if (callable == NULL) {
            goto done;
        }
        call_args = PyTuple_Pack(3, cls, newargs, kwargs);
    } else {
        callable = pycc_ext_copyreg(&pycc_ext_newobj, "__newobj__");
        if (callable == NULL) {
            goto done;
        }
        n = PyTuple_Size(newargs);
        call_args = PyTuple_New(n + 1);
        if (call_args != NULL) {
            PyTuple_SetItem(call_args, 0, Py_NewRef(cls));
            for (i = 0; i < n; i++) {
                PyTuple_SetItem(call_args, i + 1, Py_NewRef(PyTuple_GetItem(newargs, i)));
            }
        }
    }
    if (call_args == NULL) {
        goto done;
    }
    state = PyObject_CallMethod(self, "__getstate__", NULL);
    if (state == NULL) {
        goto done;
    }
    result = PyTuple_Pack(3, callable, call_args, state);
done:
    Py_XDECREF(state);
    Py_XDECREF(call_args);
    Py_XDECREF(kwargs);
    Py_DECREF(newargs);
    return result;
}

/* The defaults' entry points, at `object`'s position in the MRO
 * (`pycc_ext_object_position_hook`). */
static PyObject *pycc_ext_instance_getstate_in(PyccExtModule *m, PyObject *self)
{
    PyObject *hook = pycc_ext_object_position_hook(self, "__getstate__");
    if (hook != NULL) {
        PyObject *result = PyObject_CallNoArgs(hook);
        Py_DECREF(hook);
        return result;
    }
    return PyErr_Occurred() ? NULL : pycc_ext_instance_getstate_default(m, self);
}

static PyObject *pycc_ext_instance_setstate_in(PyObject *self, PyObject *state)
{
    PyObject *hook = pycc_ext_object_position_hook(self, "__setstate__");
    if (hook != NULL) {
        PyObject *result = PyObject_CallFunctionObjArgs(hook, state, NULL);
        Py_DECREF(hook);
        return result;
    }
    return PyErr_Occurred() ? NULL : pycc_ext_instance_setstate_default(self, state);
}

static PyObject *pycc_ext_instance_reduce_in(PyObject *self, PyObject *proto)
{
    PyObject *hook = pycc_ext_object_position_hook(self, "__reduce_ex__");
    if (hook != NULL) {
        PyObject *result = PyObject_CallFunctionObjArgs(hook, proto, NULL);
        Py_DECREF(hook);
        return result;
    }
    return PyErr_Occurred() ? NULL : pycc_ext_instance_reduce_default(self, proto);
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

