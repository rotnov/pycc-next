/* Carrier types (v0.9): the shared deallocator, the template root, the
 * compiled-class bookkeeping and the making of a published class from its
 * template. The instance boundary built on them follows in instances.c. */

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

static PyType_Slot pycc_ext_template_root_slots[] = {
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

/* The function that makes a published class from its template (Python:
 * `types.resolve_bases` and `types.prepare_class` are `__build_class__`'s
 * own rules), compiled once. */
static PyObject *pycc_ext_class_maker = NULL;

static const char *const pycc_ext_class_maker_lines[] = {
    "_skip = frozenset(('__module__', '__qualname__', '__doc__', '__dict__', '__weakref__',\n",
    "                    '__init_subclass__', '__new__', '__slots__'))\n",
    "class _SlotDefault:\n",
    "    __slots__ = ('slot', 'default')\n",
    "    def __init__(self, slot, default):\n",
    "        self.slot = slot\n",
    "        self.default = default\n",
    "    def __get__(self, obj, owner=None):\n",
    "        if obj is None:\n",
    "            return self.default\n",
    "        try:\n",
    "            return self.slot.__get__(obj, owner)\n",
    "        except (AttributeError, TypeError):\n",
    "            return self.default\n",
    "    def __set__(self, obj, value):\n",
    "        self.slot.__set__(obj, value)\n",
    "    def __delete__(self, obj):\n",
    "        self.slot.__delete__(obj)\n",
    "def _pycc_make_class(template, name, module, args, kwargs, literals,\n",
    "                     _types=__import__('types')):\n",
    "    bases = tuple(args)\n",
    "    resolved = tuple(_types.resolve_bases(bases))\n",
    "    full = (template,) + resolved\n",
    "    meta, ns, kwds = _types.prepare_class(name, full, dict(kwargs))\n",
    "    ns['__module__'] = module\n",
    "    ns['__qualname__'] = name\n",
    "    ns['__slots__'] = ()\n",
    "    for key, value in literals.items():\n",
    "        slot = template.__dict__.get(key)\n",
    "        if type(slot).__name__ == 'getset_descriptor':\n",
    "            value = _SlotDefault(slot, value)\n",
    "        ns.setdefault(key, value)\n",
    "    for key, value in template.__dict__.items():\n",
    "        if key not in _skip:\n",
    "            ns.setdefault(key, value)\n",
    "    if resolved != bases:\n",
    "        ns['__orig_bases__'] = bases\n",
    "    return meta(name, full, ns, **kwds)\n",
    NULL,
};

static PyObject *pycc_ext_get_class_maker(void)
{
    PyObject *globals;
    PyObject *code;
    PyObject *result;

    if (pycc_ext_class_maker != NULL) {
        return pycc_ext_class_maker;
    }
    globals = PyDict_New();
    if (globals == NULL) {
        return NULL;
    }
    if (PyDict_SetItemString(globals, "__builtins__", PyEval_GetBuiltins()) < 0) {
        Py_DECREF(globals);
        return NULL;
    }
    {
        char source[4096];
        size_t used = 0;
        const char *const *line;

        source[0] = '\0';
        for (line = pycc_ext_class_maker_lines; *line != NULL; line++) {
            size_t len = strlen(*line);
            if (used + len + 1 > sizeof source) {
                break;
            }
            memcpy(source + used, *line, len + 1);
            used += len;
        }
        code = Py_CompileString(source, "<pycc class>", Py_file_input);
    }
    result = code == NULL ? NULL : PyEval_EvalCode(code, globals, globals);
    Py_XDECREF(code);
    if (result == NULL) {
        Py_DECREF(globals);
        return NULL;
    }
    Py_DECREF(result);
    pycc_ext_class_maker = PyDict_GetItemString(globals, "_pycc_make_class");
    Py_XINCREF(pycc_ext_class_maker);
    Py_DECREF(globals);
    if (pycc_ext_class_maker == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: the class maker defined nothing");
    }
    return pycc_ext_class_maker;
}

/*
 * The published class `name` of `module`, made from `template` with the
 * class statement's arguments `arguments` (their source text, parentheses
 * included), evaluated in the module's namespace; entered as the class's
 * carrier type. A new reference, or NULL with an exception set.
 */
static PyObject *pycc_ext_make_public_class_in(PyccExtModule *m, PyObject *module,
                                               const char *name, PyObject *template,
                                               const char *arguments)
{
    PyObject *maker = pycc_ext_get_class_maker();
    PyObject *dict = PyModule_GetDict(module);
    PyObject *modname;
    PyObject *call;
    PyObject *code;
    PyObject *evaluated;
    PyObject *pair;
    PyObject *stack[6];
    PyObject *cls;
    size_t len;
    char *source;
    static const char prefix[] = "(lambda *a, **k: (a, k))";

    if (maker == NULL || dict == NULL) {
        return NULL;
    }
    len = strlen(arguments);
    source = (char *)PyMem_Malloc(sizeof prefix + len);
    if (source == NULL) {
        return PyErr_NoMemory();
    }
    memcpy(source, prefix, sizeof prefix - 1);
    memcpy(source + sizeof prefix - 1, arguments, len + 1);
    code = Py_CompileString(source, "<pycc class bases>", Py_eval_input);
    PyMem_Free(source);
    evaluated = code == NULL ? NULL : PyEval_EvalCode(code, dict, dict);
    Py_XDECREF(code);
    /* `((args, kwargs), literals)`: the statement's arguments and its
     * class-body literals (`attach_class_bases`). */
    pair = evaluated != NULL && PyTuple_Check(evaluated) && PyTuple_Size(evaluated) == 2
        ? PyTuple_GetItem(evaluated, 0)
        : NULL;
    if (pair == NULL) {
        Py_XDECREF(evaluated);
        /* A base only the compiler resolves (a strict module's `Generic[T]`,
         * whose `typing` import binds nothing at run time, or a base
         * published as its template, which `Box[int]` cannot subscript):
         * the class is published as its template, with no bases of its
         * own, as before v0.9. */
        PyErr_Clear();
        return Py_NewRef(template);
    }
    modname = PyModule_GetNameObject(module);
    call = modname == NULL ? NULL : PyUnicode_FromString(name);
    if (call == NULL) {
        Py_XDECREF(modname);
        Py_DECREF(evaluated);
        return NULL;
    }
    stack[0] = template;
    stack[1] = call;
    stack[2] = modname;
    stack[3] = PyTuple_GetItem(pair, 0);
    stack[4] = PyTuple_GetItem(pair, 1);
    stack[5] = PyTuple_GetItem(evaluated, 1);
    pycc_ext_making_public_class++;
    cls = PyObject_Vectorcall(maker, stack, 6, NULL);
    pycc_ext_making_public_class--;
    if (cls == NULL) {
        /* Bases CPython cannot combine with the template (a layout or
         * metaclass conflict): published as the template, as above. */
        PyErr_Clear();
        cls = Py_NewRef(template);
    }
    Py_DECREF(call);
    Py_DECREF(modname);
    Py_DECREF(evaluated);
    if (cls != NULL
        && (pycc_ext_carrier_register_in(m, name, cls) < 0
            || pycc_ext_note_compiled_class(cls) < 0)) {
        Py_CLEAR(cls);
    }
    return cls;
}

