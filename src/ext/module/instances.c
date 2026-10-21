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
static int pycc_ext_carrier_register_in(PyccExtModule *m, const char *class_name, PyObject *type)
{
    if (pycc_ext_carrier_types_ready(m) < 0) {
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
static void pycc_ext_carrier_bind(PyObject *self, void *inst)
{
    void *previous = ((PyccExtInstance *)self)->inst;
    if (previous != NULL && pycc_rt_ext_instance_carrier(previous) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(previous, NULL);
    }
    ((PyccExtInstance *)self)->inst = inst;
    pycc_rt_ext_instance_set_carrier(inst, self);
    /* The carrier adopted the reference `tp_init` built `inst` with, and
     * drops the one it held to the instance it replaces. */
    if (previous != NULL) {
        pycc_rt_instance_decref(previous);
    }
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
    {0, NULL},
};

static PyType_Spec pycc_ext_template_root_spec = {
    "pycc.compiled",
    0,
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_BASETYPE | Py_TPFLAGS_IMMUTABLETYPE
        | Py_TPFLAGS_DISALLOW_INSTANTIATION,
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

/* Whether a published class is being made from its template: its
 * `__init_subclass__` lets exactly that subclass through. */
static int pycc_ext_making_public_class = 0;

/*
 * A template's `__init_subclass__`: the published class made from it calls
 * the next one along its MRO (`Generic`'s, which records the type
 * parameters); any other subclass -- Python code subclassing a compiled
 * class -- is refused as a type without `Py_TPFLAGS_BASETYPE` is, because
 * compiled methods bind their calls to the class's own methods statically.
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
    if (!pycc_ext_making_public_class) {
        PyObject *base = PyTuple_GetItem(mro, 1);
        PyObject *module = PyObject_GetAttrString(base, "__module__");
        PyObject *qualname = PyObject_GetAttrString(base, "__qualname__");

        if (module != NULL && qualname != NULL) {
            PyErr_Format(PyExc_TypeError, "type '%U.%U' is not an acceptable base type",
                         module, qualname);
        }
        Py_XDECREF(module);
        Py_XDECREF(qualname);
        Py_DECREF(mro);
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
    "def _pycc_make_class(template, name, module, args, kwargs, _types=__import__('types')):\n",
    "    bases = tuple(args)\n",
    "    resolved = tuple(_types.resolve_bases(bases))\n",
    "    full = (template,) + resolved\n",
    "    meta, ns, kwds = _types.prepare_class(name, full, dict(kwargs))\n",
    "    ns['__module__'] = module\n",
    "    ns['__qualname__'] = name\n",
    "    ns['__slots__'] = ()\n",
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
    PyObject *pair;
    PyObject *stack[5];
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
    pair = code == NULL ? NULL : PyEval_EvalCode(code, dict, dict);
    Py_XDECREF(code);
    if (pair == NULL) {
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
        Py_DECREF(pair);
        return NULL;
    }
    stack[0] = template;
    stack[1] = call;
    stack[2] = modname;
    stack[3] = PyTuple_GetItem(pair, 0);
    stack[4] = PyTuple_GetItem(pair, 1);
    pycc_ext_making_public_class++;
    cls = PyObject_Vectorcall(maker, stack, 5, NULL);
    pycc_ext_making_public_class--;
    if (cls == NULL) {
        /* Bases CPython cannot combine with the template (a layout or
         * metaclass conflict): published as the template, as above. */
        PyErr_Clear();
        cls = Py_NewRef(template);
    }
    Py_DECREF(call);
    Py_DECREF(modname);
    Py_DECREF(pair);
    if (cls != NULL && pycc_ext_carrier_register_in(m, name, cls) < 0) {
        Py_CLEAR(cls);
    }
    return cls;
}

/* Generated companion: module name macros, per-export wrappers, method table. */
/*
 * Part 2b of #1142 (#1164): the buffer egress, defined below but called from
 * the generated wrappers in the companion `#include`d next -- the same
 * forward-declaration shape `pycc_ext_module_exec` already uses in the other
 * direction. The definition cannot move above the include: the exporter
 * type's `PyType_Spec` names `PYCC_EXT_MODULE_NAME_STR`, which the companion
 * defines.
 */
static PyObject *pycc_ext_pack_memoryview(PyccExtBufferView *view);
/*
 * Part 1 of #1175: the caller-owned half of the buffer egress, declared here
 * for the same reason its sibling above is -- the generated wrappers in the
 * companion `#include`d next are its only callers. Unlike its sibling it has
 * no dependency on the exporter type, so only the symmetry keeps the two
 * definitions together.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed(PyObject *owner, const Py_buffer *held,
                                                   const char *fn_name, Py_ssize_t index,
                                                   int writable);
/*
 * Part 2 of #1175 (#1179): the sub-range form of the same egress, declared
 * here for the same reason. A separate entry point rather than three more
 * parameters on the whole-view one, so that every wrapper Part 1 generates
 * keeps generating byte-for-byte what it generated before -- only an export
 * that really can return `b[start:stop]` calls this.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed_slice(PyObject *owner, const Py_buffer *held,
                                                         const char *fn_name, Py_ssize_t index,
                                                         int writable, long long start,
                                                         long long stop);

/*
 * #1461: keyword arguments at the host boundary, bound by name as CPython
 * binds a Python function's positional-or-keyword parameters
 * (`initialize_locals`): the positional arguments fill the leading slots,
 * then each keyword, in call order, fills the slot it names -- an unknown
 * name and an already filled slot are the `TypeError`s CPython words the
 * same way -- and finally every required slot must be filled. There is no
 * "too many positional arguments" step: with more positional arguments than
 * slots every slot is already filled, so the first keyword raises "multiple
 * values" or "unexpected", which is also what CPython reports.
 *
 * Only a generated wrapper or `Py_tp_init` whose export the driver made
 * keyword-enabled calls these, and only when the host passed at least one
 * keyword; a keyword-free call never enters them. `slots` receives borrowed
 * references only -- the caller's arguments and keyword values -- and a slot
 * left NULL is a defaulted parameter the generated caller fills from its
 * own default object. `names` holds the `arity` parameter names as UTF-8.
 */
static int pycc_ext_kw_place(const char *fn_name, const char *const *names, Py_ssize_t arity,
                             PyObject **slots, PyObject *key, PyObject *value)
{
    Py_ssize_t i;
    if (!PyUnicode_Check(key)) {
        PyErr_Format(PyExc_TypeError, "%s() keywords must be strings", fn_name);
        return -1;
    }
    for (i = 0; i < arity; i++) {
        if (PyUnicode_EqualToUTF8(key, names[i])) {
            if (slots[i] != NULL) {
                PyErr_Format(PyExc_TypeError, "%s() got multiple values for argument '%s'",
                             fn_name, names[i]);
                return -1;
            }
            slots[i] = value;
            return 0;
        }
    }
    PyErr_Format(PyExc_TypeError, "%s() got an unexpected keyword argument '%U'", fn_name, key);
    return -1;
}

/*
 * The required-argument check, worded as CPython's `format_missing`:
 * `'a'`, `'a' and 'b'`, `'a', 'b', and 'c'`.
 */
static int pycc_ext_kw_finish(const char *fn_name, const char *const *names,
                              Py_ssize_t required, PyObject **slots)
{
    Py_ssize_t i;
    Py_ssize_t missing = 0;
    Py_ssize_t listed = 0;
    PyObject *list;
    for (i = 0; i < required; i++) {
        if (slots[i] == NULL) {
            missing++;
        }
    }
    if (missing == 0) {
        return 0;
    }
    list = PyUnicode_FromString("");
    for (i = 0; i < required && list != NULL; i++) {
        PyObject *longer;
        const char *separator;
        if (slots[i] != NULL) {
            continue;
        }
        if (listed == 0) {
            separator = "";
        } else if (missing == 2) {
            separator = " and ";
        } else if (listed == missing - 1) {
            separator = ", and ";
        } else {
            separator = ", ";
        }
        longer = PyUnicode_FromFormat("%U%s'%s'", list, separator, names[i]);
        Py_DECREF(list);
        list = longer;
        listed++;
    }
    if (list == NULL) {
        return -1;
    }
    PyErr_Format(PyExc_TypeError, "%s() missing %zd required positional argument%s: %U",
                 fn_name, missing, missing == 1 ? "" : "s", list);
    Py_DECREF(list);
    return -1;
}

/* The `METH_FASTCALL | METH_KEYWORDS` form: keyword values follow the
 * `nargs` positional ones in `args`, named by the `kwnames` tuple. */
static int pycc_ext_kw_bind_fastcall(const char *fn_name, const char *const *names,
                                     Py_ssize_t arity, Py_ssize_t required, PyObject **slots,
                                     PyObject *const *args, Py_ssize_t nargs, PyObject *kwnames)
{
    Py_ssize_t i;
    Py_ssize_t count = PyTuple_Size(kwnames);
    for (i = 0; i < arity; i++) {
        slots[i] = i < nargs ? args[i] : NULL;
    }
    for (i = 0; i < count; i++) {
        if (pycc_ext_kw_place(fn_name, names, arity, slots, PyTuple_GetItem(kwnames, i),
                              args[nargs + i]) < 0) {
            return -1;
        }
    }
    return pycc_ext_kw_finish(fn_name, names, required, slots);
}

/* The `Py_tp_init` form: a positional tuple and a keyword dict, whose
 * iteration order is the call's keyword order. */
static int pycc_ext_kw_bind_dict(const char *fn_name, const char *const *names, Py_ssize_t arity,
                                 Py_ssize_t required, PyObject **slots, PyObject *args,
                                 PyObject *kwds)
{
    Py_ssize_t i;
    Py_ssize_t nargs = PyTuple_Size(args);
    Py_ssize_t position = 0;
    PyObject *key;
    PyObject *value;
    for (i = 0; i < arity; i++) {
        slots[i] = i < nargs ? PyTuple_GetItem(args, i) : NULL;
    }
    while (PyDict_Next(kwds, &position, &key, &value)) {
        if (pycc_ext_kw_place(fn_name, names, arity, slots, key, value) < 0) {
            return -1;
        }
    }
    return pycc_ext_kw_finish(fn_name, names, required, slots);
}

/*
 * #1427: the `Py_tp_hash` of a carrier type whose class compiles
 * `__hash__`, finishing what the generated `pycc_ext_hash_<Class>` started:
 * `res` is the compiled method's packed result (a new reference) or NULL
 * with the exception it raised. The conversion is CPython's own
 * `slot_tp_hash`: a non-`int` result is a `TypeError`, an `int` too large
 * for `Py_hash_t` hashes as that `int` does, and `-1` -- the error
 * sentinel -- becomes `-2`.
 */
static Py_hash_t pycc_ext_finish_hash(PyObject *res)
{
    Py_hash_t h;

    if (res == NULL) {
        return -1;
    }
    if (!PyLong_Check(res)) {
        Py_DECREF(res);
        PyErr_SetString(PyExc_TypeError, "__hash__ method should return an integer");
        return -1;
    }
    h = PyLong_AsSsize_t(res);
    if (h == -1 && PyErr_Occurred()) {
        PyErr_Clear();
        h = PyObject_Hash(res);
    } else if (h == -1) {
        h = -2;
    }
    Py_DECREF(res);
    return h;
}

/*
 * #1427: `object`'s identity hash, for a carrier type that installs a
 * `Py_tp_richcompare` while its class binds neither `__eq__` nor
 * `__hash__` (an `__lt__`-only class). `inherit_slots` never inherits
 * `tp_hash` next to an overridden `tp_richcompare`, so without this the
 * type would turn unhashable where CPython's class stays hashable.
 */
static Py_hash_t pycc_ext_identity_hash(PyObject *self)
{
    hashfunc base = (hashfunc)PyType_GetSlot(&PyBaseObject_Type, Py_tp_hash);
    return base(self);
}

/*
 * #1427: the `Py_tp_hash` row of a carrier type whose class binds `__eq__`
 * without `__hash__`. It is a marker: `pycc_ext_unhashable_slots` swaps it
 * for `PyObject_HashNotImplemented` before `PyType_FromSpec` reads the
 * array, because `PyType_Ready` publishes `__hash__ = None` only for that
 * exact address, and on Windows a static initializer naming a function
 * imported from `python3.dll` holds the import thunk's address instead.
 * Should it ever be called, it answers exactly as the real slot does.
 */
static Py_hash_t pycc_ext_unhashable(PyObject *self)
{
    return PyObject_HashNotImplemented(self);
}

static void pycc_ext_unhashable_slots(PyType_Slot *slots)
{
    for (; slots->slot != 0; ++slots) {
        if (slots->slot == Py_tp_hash && slots->pfunc == (void *)pycc_ext_unhashable) {
            slots->pfunc = (void *)PyObject_HashNotImplemented;
        }
    }
}

#include "pycc_ext_exports.inc"

/*
 * #1455: `copy.copy(x)` of a carrier. CPython's `copy.copy` calls a type's
 * `__copy__` before any reduce protocol, so this is the whole copy: a new
 * carrier of `type(x)` wrapping a clone of `x`'s compiled instance. The
 * generated table, keyed by the instance's run-time class, decides whether
 * the class copies at all -- a class whose namespace takes part in the copy
 * protocol, or a PEP 695 template whose specializations share one layout
 * name, is refused with a `TypeError` naming why -- and hands the clone one
 * kind byte per slot.
 *
 * A never-initialized carrier (`mod.C.__new__(mod.C)`, `inst` NULL) has no
 * instance to read a class from; only a published type can produce one, and
 * its name is its class name, so the table is consulted by that name and an
 * accepted copy is another never-initialized carrier.
 *
 * The original is not touched: its reference count is unchanged, as in
 * CPython. The clone takes its own reference to every slot value, by the
 * slot's kind (`pycc_rt_ext_instance_copy`), `o` slots included.
 */
static PyObject *pycc_ext_instance_copy_in(PyccExtModule *m, PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst = ((PyccExtInstance *)self)->inst;
    const unsigned char *cls;
    size_t len = 0;
    const char *kinds = NULL;
    size_t nkinds = 0;
    const char *refused = NULL;
    PyObject *name = NULL;
    void *clone = NULL;
    allocfunc tp_alloc;
    PyObject *carrier;
    int found;

    if (inst == NULL) {
        Py_ssize_t name_len = 0;
        name = PyType_GetName(tp);
        if (name == NULL) {
            return NULL;
        }
        cls = (const unsigned char *)PyUnicode_AsUTF8AndSize(name, &name_len);
        if (cls == NULL) {
            Py_DECREF(name);
            return NULL;
        }
        len = (size_t)name_len;
    }
    else {
        cls = pycc_rt_ext_instance_class(inst, &len);
    }
    found = m->carrier_class_copy_kinds(cls, len, &kinds, &nkinds, &refused);
    Py_XDECREF(name);
    if (found < 0) {
        PyErr_SetString(PyExc_SystemError, "pycc: no copy table entry for a carrier's class");
        return NULL;
    }
    if (found == 0) {
        PyObject *qualified = PyType_GetFullyQualifiedName(tp);
        if (qualified != NULL) {
            PyErr_Format(PyExc_TypeError, "cannot copy '%U' object: %s", qualified, refused);
            Py_DECREF(qualified);
        }
        return NULL;
    }
    if (inst != NULL) {
        clone = pycc_rt_ext_instance_copy(inst, kinds, nkinds);
        if (clone == NULL) {
            PyErr_SetString(PyExc_SystemError,
                            "pycc: copy kind table does not match the instance layout");
            return NULL;
        }
    }
    tp_alloc = (allocfunc)PyType_GetSlot(tp, Py_tp_alloc);
    carrier = tp_alloc(tp, 0);
    if (carrier == NULL) {
        return NULL;
    }
    ((PyccExtInstance *)carrier)->inst = clone;
    if (clone != NULL) {
        pycc_rt_ext_instance_set_carrier(clone, carrier);
    }
    return carrier;
}


/*
 * The carrier type for the class named `name`/`len` (a borrowed reference
 * the cache owns), creating `<module>.<name>` on the first crossing of a
 * class no published type stands for. Returns NULL with a CPython
 * exception set on failure.
 *
 * An empty name is an instance without a layout descriptor. Only an enum
 * member is allocated that way, and `pycc_types` refuses one as a call
 * argument, so this is a `SystemError` rather than a carrier of no class.
 *
 * The type's full name is copied into storage that is never released:
 * `tp_name` may point at the spec's name, and the type itself is never
 * released either.
 */
static PyObject *pycc_ext_carrier_type(PyccExtModule *m, const unsigned char *name, size_t len)
{
    PyObject *key;
    PyObject *type;
    size_t prefix = strlen(m->name);
    char *full;
    PyType_Spec spec;
    int found;

    if (len == 0) {
        PyErr_SetString(PyExc_SystemError,
                        "an instance without a class descriptor cannot cross into CPython");
        return NULL;
    }
    if (pycc_ext_carrier_types_ready(m) < 0) {
        return NULL;
    }
    key = PyUnicode_FromStringAndSize((const char *)name, (Py_ssize_t)len);
    if (key == NULL) {
        return NULL;
    }
    found = PyDict_GetItemRef(m->carrier_types, key, &type);
    if (found != 0) {
        Py_DECREF(key);
        if (found < 0) {
            return NULL;
        }
        Py_DECREF(type);
        return type;
    }
    full = (char *)PyMem_Malloc(prefix + 1 + len + 1);
    if (full == NULL) {
        Py_DECREF(key);
        return PyErr_NoMemory();
    }
    memcpy(full, m->name, prefix);
    full[prefix] = '.';
    memcpy(full + prefix + 1, name, len);
    full[prefix + 1 + len] = '\0';
    spec.name = full;
    spec.basicsize = (int)sizeof(PyccExtInstance);
    spec.itemsize = 0;
    spec.flags = Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE;
    spec.slots = m->carrier_slots;
    type = PyType_FromSpec(&spec);
    if (type == NULL || PyDict_SetItem(m->carrier_types, key, type) < 0) {
        Py_XDECREF(type);
        Py_DECREF(key);
        return NULL;
    }
    Py_DECREF(key);
    Py_DECREF(type);
    return type;
}

/*
 * v0.8 (the whole-package build): the record of every module of this
 * extension whose body has started, so an instance of a class one module
 * compiled that another module's code hands CPython (it reads the class's
 * instances natively, `pycc_codegen`'s `sibling_class`) is carried by the
 * type of the module owning the class. Grows only; one copy per extension,
 * as this unit is.
 */
static PyccExtModule **pycc_ext_modules = NULL;
static size_t pycc_ext_module_count = 0;
static size_t pycc_ext_module_capacity = 0;

/* Enters `m` once. Returns 0, or -1 with a `MemoryError` set. */
static int pycc_ext_register_module_in(PyccExtModule *m)
{
    PyccExtModule **grown;
    size_t i;

    for (i = 0; i < pycc_ext_module_count; i++) {
        if (pycc_ext_modules[i] == m) {
            return 0;
        }
    }
    if (pycc_ext_module_count == pycc_ext_module_capacity) {
        pycc_ext_module_capacity = pycc_ext_module_capacity == 0 ? 8 : 2 * pycc_ext_module_capacity;
        grown = (PyccExtModule **)PyMem_Realloc(
            pycc_ext_modules, pycc_ext_module_capacity * sizeof(PyccExtModule *));
        if (grown == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        pycc_ext_modules = grown;
    }
    pycc_ext_modules[pycc_ext_module_count++] = m;
    return 0;
}

/* Whether `m` compiles the class `name`/`len` (a layout descriptor's, not
 * NUL-terminated): its generated carrier table holds it. */
static int pycc_ext_owns_class(PyccExtModule *m, const unsigned char *name, size_t len)
{
    char small[128];
    char *text = len < sizeof(small) ? small : (char *)PyMem_Malloc(len + 1);
    int owns;

    if (text == NULL) {
        return 0;
    }
    memcpy(text, name, len);
    text[len] = '\0';
    owns = m->carrier_class_isinstance(name, len, text) == 1;
    if (text != small) {
        PyMem_Free(text);
    }
    return owns;
}

/* The module whose carrier type carries an instance of the class
 * `name`/`len` that code of `m` hands CPython: `m` when it compiles the
 * class (or no module of the extension does), else the one module of the
 * extension that does; NULL with a `SystemError` when several do. */
static PyccExtModule *pycc_ext_class_owner(PyccExtModule *m, const unsigned char *name,
                                           size_t len)
{
    PyccExtModule *owner = NULL;
    size_t i;

    if (pycc_ext_owns_class(m, name, len)) {
        return m;
    }
    for (i = 0; i < pycc_ext_module_count; i++) {
        if (pycc_ext_modules[i] != m && pycc_ext_owns_class(pycc_ext_modules[i], name, len)) {
            if (owner != NULL) {
                PyErr_Format(PyExc_SystemError,
                             "modules %s and %s both compile a class %.*s; an instance of it "
                             "cannot cross into CPython from %s",
                             owner->name, pycc_ext_modules[i]->name, (int)len, name, m->name);
                return NULL;
            }
            owner = pycc_ext_modules[i];
        }
    }
    return owner != NULL ? owner : m;
}

/*
 * #1435: the packer for a pycc class instance passed as an argument to a
 * call on a CPython object (`EXT_OBJ_PACK_INSTANCE_SYMBOL`). The packer
 * contract of its scalar neighbours above: it borrows the instance and
 * returns a new reference, which the consuming call helper releases.
 *
 * Identity follows CPython. The instance's live carrier, if it has one, is
 * returned again -- the object the host constructed, or the one an earlier
 * crossing made and the host still holds -- so two crossings of one
 * instance are `is`-equal. Otherwise a fresh carrier of the instance's
 * *run-time* class (its layout descriptor, not the argument's static type)
 * is allocated and linked; its `tp_dealloc` unlinks it. The instance is
 * never freed (D-107, D-154), so the carrier cannot outlive it.
 *
 * A NULL operand is a `@classmethod`'s `cls`, which `MirExpr::NullInstance`
 * lowers to a null pointer and `pycc_types` refuses as an argument; it is
 * answered with a `SystemError`, which the consuming helpers tolerate.
 */
static PyObject *pycc_ext_obj_pack_instance_in(PyccExtModule *m, void *inst)
{
    PyObject *carrier;
    PyObject *type;
    const unsigned char *name;
    size_t len = 0;
    allocfunc tp_alloc;

    if (inst == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "an instance argument to a CPython object's call was NULL");
        return NULL;
    }
    carrier = (PyObject *)pycc_rt_ext_instance_carrier(inst);
    if (carrier != NULL) {
        Py_INCREF(carrier);
        return carrier;
    }
    name = pycc_rt_ext_instance_class(inst, &len);
    m = pycc_ext_class_owner(m, name, len);
    if (m == NULL) {
        return NULL;
    }
    type = pycc_ext_carrier_type(m, name, len);
    if (type == NULL) {
        return NULL;
    }
    tp_alloc = (allocfunc)PyType_GetSlot((PyTypeObject *)type, Py_tp_alloc);
    carrier = tp_alloc((PyTypeObject *)type, 0);
    if (carrier == NULL) {
        return NULL;
    }
    ((PyccExtInstance *)carrier)->inst = inst;
    pycc_rt_ext_instance_set_carrier(inst, carrier);
    /* `inst` is borrowed; the new carrier owns a reference of its own. */
    pycc_rt_instance_incref(inst);
    return carrier;
}

/*
 * Part 7 of #1371: `isinstance(o, C)` with an object `o` and a class `C`
 * compiled in this module (`EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL`). `o` is
 * borrowed; `name` is the class's NUL-terminated name, a constant string
 * compiled code owns. Same 1/0/-1 contract as `pycc_ext_obj_isinstance`.
 *
 * #1435: a carrier of a compiled instance -- an object whose type
 * deallocates through `pycc_ext_instance_dealloc`, which only the carrier
 * types use and which nothing can subclass -- is answered from the
 * instance's run-time class by the generated
 * `pycc_ext_carrier_class_isinstance`, so a carrier of a class that
 * publishes no method, or of an unpublished subclass of a published one,
 * answers as CPython does for the same source. Every other object,
 * including a published type's object no `tp_init` filled, goes to the
 * generated `pycc_ext_compiled_class_isinstance` declared above, which
 * carries the published-family rule. Defined after the companion include,
 * which defines the generated function it calls first.
 */
static int pycc_ext_obj_isinstance_compiled_in(PyccExtModule *m, PyObject *o, const char *name)
{
    void *inst;
    const unsigned char *cls;
    size_t len = 0;

    if (o == NULL || name == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_isinstance_compiled: NULL operand");
        return -1;
    }
    /* An exception class compiled code names is tested as the CPython class
     * that stands for it: a caught exception is a CPython object. */
    {
        PyObject *exception_class = m->exception_class_named(name);
        if (exception_class != NULL) {
            return PyObject_IsInstance(o, exception_class);
        }
    }
    if (pycc_ext_is_carrier_type(Py_TYPE(o))) {
        inst = ((PyccExtInstance *)o)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            return m->carrier_class_isinstance(cls, len, name);
        }
    }
    return m->compiled_class_isinstance(o, name);
}

/*
 * Part 1 of #1447 (#1449): unpacks one argument at a parameter annotated
 * with a regular class compiled in this module. Returns 0 with `*out` the
 * compiled instance the carrier holds, or -1 with a `TypeError` set.
 *
 * Only a carrier of this module is admitted -- an object whose type
 * deallocates through *this* file's `pycc_ext_instance_dealloc`, which a
 * carrier of another pycc extension does not, since each artifact links
 * its own copy of the static -- and only one whose instance's run-time
 * class has `class_name` on its MRO, answered by the same generated table
 * `isinstance` uses, so a subclass instance is admitted exactly as CPython
 * admits it. A carrier no `tp_init` filled (`mod.C.__new__(mod.C)`) holds
 * no instance and is refused with its own message.
 *
 * `*out` is a new reference to the instance, which the compiled
 * parameter slot receiving it owns, so the carrier dying during the call
 * leaves nothing dangling; the carrier link the instance holds is weak and
 * cleared by the carrier's own `tp_dealloc`.
 */
static int pycc_ext_unpack_instance_in(PyccExtModule *m, PyObject *obj, const char *fn_name,
                                       Py_ssize_t index, const char *class_name, void **out)
{
    PyObject *type_name;
    void *inst = NULL;
    const unsigned char *cls;
    size_t len = 0;
    int carrier = pycc_ext_is_carrier_type(Py_TYPE(obj));

    if (carrier) {
        inst = ((PyccExtInstance *)obj)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            if (m->carrier_class_isinstance(cls, len, class_name)) {
                /* A reference of its own for the receiver: a parameter
                 * slot or the narrowed read owns what it is given. */
                pycc_rt_instance_incref(inst);
                *out = inst;
                return 0;
            }
        }
    }
    /* Fully qualified on both sides, so a carrier of another pycc module
     * that compiled a class of the same name reads `must be m.Conf, not
     * other.Conf` rather than `must be Conf, not Conf`. */
    type_name = PyType_GetFullyQualifiedName(Py_TYPE(obj));
    if (type_name == NULL) {
        return -1;
    }
    if (carrier && inst == NULL) {
        PyErr_Format(PyExc_TypeError,
                     "%s() argument %zd: the %U object is uninitialized (its __init__ never "
                     "ran)",
                     fn_name, index + 1, type_name);
    } else {
        PyErr_Format(PyExc_TypeError, "%s() argument %zd must be %s.%s, not %U", fn_name,
                     index + 1, m->name, class_name, type_name);
    }
    Py_DECREF(type_name);
    return -1;
}

/*
 * #1476 (Part 3 of #1387): the read of an `object` name an
 * `isinstance(o, C)` guard narrowed to a regular class `C` compiled in this
 * module (`EXT_OBJ_UNBOX_INSTANCE_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). Writes the compiled instance the
 * carrier holds through `*out` and answers `0`, or answers `-1` with a
 * `TypeError` set.
 *
 * `pycc_ext_unpack_instance`'s exact admission -- a carrier of *this*
 * module whose instance's run-time class has `class_name` on its MRO -- with
 * messages that name no argument position. The guard (`PyObject_IsInstance`
 * against the published carrier types) also holds for a carrier no
 * `tp_init` filled (`C.__new__(C)`), refused with a `TypeError` where
 * CPython would raise `AttributeError` at the first field read; and for a
 * non-carrier whose `__class__` property answers a published type (a mock,
 * a proxy), refused with a `TypeError` where CPython would run the guarded
 * body on it -- the same deliberate deviation as the scalar helpers'
 * (#1476). A host-side
 * Python subclass cannot reach here: a carrier type is not an acceptable
 * base type, so the dealloc-slot test only rejects it defensively.
 *
 * `*out` is a new reference, as at the argument ingress: compiled code
 * owns the narrowed read (`heap_rc::is_borrowed_read` classifies it fresh).
 */
static int pycc_ext_obj_unbox_instance_in(PyccExtModule *m, PyObject *o, const char *class_name,
                                          void **out)
{
    PyObject *type_name;
    void *inst = NULL;
    const unsigned char *cls;
    size_t len = 0;
    int carrier;

    if (o == NULL || class_name == NULL || out == NULL) {
        return -1;
    }
    carrier = pycc_ext_is_carrier_type(Py_TYPE(o));
    if (carrier) {
        inst = ((PyccExtInstance *)o)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            if (m->carrier_class_isinstance(cls, len, class_name)) {
                /* A reference of its own for the receiver: a parameter
                 * slot or the narrowed read owns what it is given. */
                pycc_rt_instance_incref(inst);
                *out = inst;
                return 0;
            }
        }
    }
    type_name = PyType_GetFullyQualifiedName(Py_TYPE(o));
    if (type_name == NULL) {
        return -1;
    }
    if (carrier && inst == NULL) {
        PyErr_Format(PyExc_TypeError,
                     "the narrowed %U object is uninitialized (its __init__ never ran)",
                     type_name);
    } else {
        PyErr_Format(PyExc_TypeError,
                     "narrowed object must be a compiled %s.%s instance, not %U",
                     m->name, class_name, type_name);
    }
    Py_DECREF(type_name);
    return -1;
}

/*
 * Non-strict mode: a CPython object passed by compiled code for a parameter
 * annotated with the regular class `class_name` compiled in module `m`
 * (`EXT_OBJ_COERCE_INSTANCE_SYMBOL` in `pycc_codegen`'s `object_coerce`):
 * the argument ingress's admission (`pycc_ext_unpack_instance_in`), its
 * `TypeError` naming the call's argument as `what` (`f() argument 2`).
 * `*out` is a new reference the parameter slot owns.
 */
static int pycc_ext_obj_coerce_instance_in(PyccExtModule *m, PyObject *o, const char *class_name,
                                           const char *what, void **out)
{
    PyObject *type_name;
    void *inst = NULL;
    const unsigned char *cls;
    size_t len = 0;
    int carrier;

    if (o == NULL || class_name == NULL || what == NULL || out == NULL) {
        return -1;
    }
    carrier = pycc_ext_is_carrier_type(Py_TYPE(o));
    if (carrier) {
        inst = ((PyccExtInstance *)o)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            if (m->carrier_class_isinstance(cls, len, class_name)) {
                pycc_rt_instance_incref(inst);
                *out = inst;
                return 0;
            }
        }
    }
    type_name = PyType_GetFullyQualifiedName(Py_TYPE(o));
    if (type_name == NULL) {
        return -1;
    }
    if (carrier && inst == NULL) {
        PyErr_Format(PyExc_TypeError, "%s: the %U object is uninitialized (its __init__ never ran)",
                     what, type_name);
    } else {
        PyErr_Format(PyExc_TypeError, "%s must be %s.%s, not %U", what, m->name, class_name,
                     type_name);
    }
    Py_DECREF(type_name);
    return -1;
}

/*
 * #1427: whether a generated `tp_richcompare` calls its compiled comparison
 * on `other` when that method's parameter is annotated with the compiled
 * class `class_name`. An operand `pycc_ext_unpack_instance` would refuse as
 * the wrong type answers 0, and the slot returns `NotImplemented` -- what a
 * `@dataclass`'s own `__eq__` returns for another class -- instead of
 * raising that ingress `TypeError`, so `p == None` is `False` as in CPython.
 * A carrier of this module that no `tp_init` filled answers 1, so the
 * wrapper refuses it with its own `uninitialized` message.
 */
static int pycc_ext_slot_operand_is_in(PyccExtModule *m, PyObject *other, const char *class_name)
{
    void *inst;
    const unsigned char *cls;
    size_t len = 0;

    if (!pycc_ext_is_carrier_type(Py_TYPE(other))) {
        return 0;
    }
    inst = ((PyccExtInstance *)other)->inst;
    if (inst == NULL) {
        return 1;
    }
    cls = pycc_rt_ext_instance_class(inst, &len);
    return m->carrier_class_isinstance(cls, len, class_name);
}

/*
 * Part 1 of #1447 (#1449): packs a result annotated with a regular class
 * compiled in this module through #1435's carrier egress, so a returned
 * instance that already has a live carrier -- `self`, or an argument the
 * host still holds -- is that very object (`o.m() is o`), and any other
 * instance gets a fresh carrier of its run-time class. Returns a new
 * reference, the CPython return-value convention.
 *
 * `result` is never NULL on this path -- the generated wrapper checks for a
 * pending pycc exception first, and a compiled function annotated with a
 * class cannot return `None` -- but, as in `pycc_ext_pack_object`, the
 * guard turns a would-be crash into a `SystemError` with a message that
 * names the return position rather than #1435's argument position.
 */
static PyObject *pycc_ext_pack_instance_in(PyccExtModule *m, void *result)
{
    PyObject *packed;
    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "instance result was NULL");
        return NULL;
    }
    /* `result` is a reference the caller owns and hands over: the carrier
     * holds its own, so this one is released once packed. */
    packed = pycc_ext_obj_pack_instance_in(m, result);
    pycc_rt_instance_decref(result);
    return packed;
}

/*
 * Part 2b of #1142 (#1164): the artifact-owned buffer exporter.
 *
 * A compiled function that returns a buffer hands the wrapper the
 * `PyccExtBufferView *` `pycc_rt_buffer_f64_alloc` produced -- artifact-owned
 * storage with no CPython object anywhere in it. The host must receive a real
 * `memoryview`, and a `memoryview` only ever exists over an *exporter*, so
 * this type is that exporter: one refcounted object per returned buffer,
 * holding the view and freeing it in `tp_dealloc`.
 *
 * That is what moves the free off the compiled frame's own epilogue. A
 * returned buffer is the one buffer the frame must *not* free (see
 * `pycc_codegen`'s `MirStmt::Return`), because the host may hold the
 * `memoryview` arbitrarily long after the call returned; the exporter's
 * refcount is the lifetime that replaces the frame's.
 *
 * The type object itself is a file static rather than module state. `m_size`
 * is 0, so there is no module state to put it in, and
 * `Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED` plus the free-threading
 * refusal in `PyInit_` mean exactly one interpreter on one GIL ever reaches
 * it -- the same single-interpreter argument `pycc_rt`'s non-atomic refcounts
 * already rest on.
 */
typedef struct {
    PyObject_HEAD
    PyccExtBufferView *view;
    /*
     * `Py_buffer.shape` and `.strides` must outlive the `getbuffer` call, so
     * they live in the exporter rather than on its stack. Both are constant
     * for the object's whole life -- the storage is never resized -- so two
     * concurrent views sharing them observe the same values.
     */
    Py_ssize_t shape[1];
    Py_ssize_t strides[1];
} PyccExtBufferObj;

/*
 * The exporter type, created once in `pycc_ext_exec_module` and never
 * released: it has to outlive every `memoryview` the host still holds, and a
 * module reload runs the exec slot again against the same file static.
 */
static PyObject *pycc_ext_buffer_type = NULL;

