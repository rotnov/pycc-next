/*
 * Making a published class from its template (v0.9), without `compile()`
 * at import (v0.11).
 *
 * When a published class's statement runs, the class is made from its
 * template with the statement's arguments, as `__build_class__` makes a
 * class: `types.resolve_bases` and `types.prepare_class`'s rules, then the
 * metaclass called with the namespace (`pycc_ext_make_class`). Until v0.11
 * that was a Python function compiled from source once per process, and
 * the arguments were evaluated by compiling `(lambda *a, **k: (a, k))` and
 * their source text: `compile()` was most of what importing a package of
 * public classes added to CPython's own work. Both are C now; arguments
 * the build could not reduce to names (`Generic[T]`, a call) are still
 * evaluated from their source.
 *
 * Every failure on the way -- a name the module does not bind, bases
 * CPython cannot combine with the template -- publishes the class as its
 * template with no bases of its own, as before v0.9, so which exception a
 * step raises is never seen.
 */

/* `Py_SETREF`, which the limited API does not have. */
#define PYCC_SETREF(dst, src) do { PyObject *pycc_old_ = (PyObject *)(dst); (dst) = (src); Py_XDECREF(pycc_old_); } while (0)

/*
 * `_SlotDefault`: a class-body default `None` that names a compiled slot
 * (`template = None`), as the published class's attribute. Read on the
 * class it is the default; on an instance it is the slot's value, or the
 * default while the slot is unset (CPython's class attribute shows through
 * an unset instance attribute); assigning and deleting reach the slot.
 */
typedef struct {
    PyObject_HEAD
    PyObject *slot;
    PyObject *dflt;
} PyccSlotDefault;

static PyObject *pycc_ext_slot_default_type = NULL;

static int pycc_slot_default_init(PyObject *self, PyObject *args, PyObject *kwargs)
{
    static char *names[] = {"slot", "default", NULL};
    PyccSlotDefault *d = (PyccSlotDefault *)self;
    PyObject *slot, *dflt, *old_slot, *old_dflt;

    if (!PyArg_ParseTupleAndKeywords(args, kwargs, "OO:__init__", names, &slot, &dflt)) {
        return -1;
    }
    old_slot = d->slot;
    old_dflt = d->dflt;
    d->slot = Py_NewRef(slot);
    d->dflt = Py_NewRef(dflt);
    Py_XDECREF(old_slot);
    Py_XDECREF(old_dflt);
    return 0;
}

/* `self.default`, or NULL with `AttributeError` as an unset slot raises. */
static PyObject *pycc_slot_default_value(PyccSlotDefault *d)
{
    if (d->dflt == NULL) {
        PyErr_SetString(PyExc_AttributeError, "'_SlotDefault' object has no attribute 'default'");
        return NULL;
    }
    return Py_NewRef(d->dflt);
}

static PyObject *pycc_slot_default_get(PyObject *self, PyObject *obj, PyObject *owner)
{
    PyccSlotDefault *d = (PyccSlotDefault *)self;
    PyObject *value;

    if (obj == NULL || obj == Py_None) {
        return pycc_slot_default_value(d);
    }
    if (d->slot == NULL) {
        return pycc_slot_default_value(d);
    }
    value = PyObject_CallMethodObjArgs(d->slot, pycc_ext_attr_name("__get__"), obj,
                                       owner != NULL ? owner : Py_None, NULL);
    if (value == NULL
        && (PyErr_ExceptionMatches(PyExc_AttributeError)
            || PyErr_ExceptionMatches(PyExc_TypeError))) {
        PyErr_Clear();
        return pycc_slot_default_value(d);
    }
    return value;
}

static int pycc_slot_default_set(PyObject *self, PyObject *obj, PyObject *value)
{
    PyccSlotDefault *d = (PyccSlotDefault *)self;
    PyObject *done;

    if (d->slot == NULL) {
        PyErr_SetString(PyExc_AttributeError, "'_SlotDefault' object has no attribute 'slot'");
        return -1;
    }
    done = value == NULL
        ? PyObject_CallMethodObjArgs(d->slot, pycc_ext_attr_name("__delete__"), obj, NULL)
        : PyObject_CallMethodObjArgs(d->slot, pycc_ext_attr_name("__set__"), obj, value, NULL);
    if (done == NULL) {
        return -1;
    }
    Py_DECREF(done);
    return 0;
}

static int pycc_slot_default_traverse(PyObject *self, visitproc visit, void *arg)
{
    PyccSlotDefault *d = (PyccSlotDefault *)self;

    Py_VISIT(d->slot);
    Py_VISIT(d->dflt);
    Py_VISIT((PyObject *)Py_TYPE(self));
    return 0;
}

static int pycc_slot_default_clear(PyObject *self)
{
    PyccSlotDefault *d = (PyccSlotDefault *)self;

    Py_CLEAR(d->slot);
    Py_CLEAR(d->dflt);
    return 0;
}

static void pycc_slot_default_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    freefunc tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);

    PyObject_GC_UnTrack(self);
    (void)pycc_slot_default_clear(self);
    tp_free(self);
    Py_DECREF(tp);
}

static PyMemberDef pycc_slot_default_members[] = {
    {"slot", Py_T_OBJECT_EX, offsetof(PyccSlotDefault, slot), 0, NULL},
    {"default", Py_T_OBJECT_EX, offsetof(PyccSlotDefault, dflt), 0, NULL},
    {NULL, 0, 0, 0, NULL},
};

static PyType_Slot pycc_slot_default_slots[] = {
    {Py_tp_init, (void *)pycc_slot_default_init},
    {Py_tp_new, (void *)PyType_GenericNew},
    {Py_tp_descr_get, (void *)pycc_slot_default_get},
    {Py_tp_descr_set, (void *)pycc_slot_default_set},
    {Py_tp_traverse, (void *)pycc_slot_default_traverse},
    {Py_tp_clear, (void *)pycc_slot_default_clear},
    {Py_tp_dealloc, (void *)pycc_slot_default_dealloc},
    {Py_tp_members, (void *)pycc_slot_default_members},
    {0, NULL},
};

static PyType_Spec pycc_slot_default_spec = {
    /* In `pycc`, as `pycc.compiled`: a builtin type with no module warns
     * (DeprecationWarning) when made. */
    "pycc._SlotDefault",
    sizeof(PyccSlotDefault),
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_BASETYPE | Py_TPFLAGS_HAVE_GC,
    pycc_slot_default_slots,
};

/* A new `_SlotDefault(slot, dflt)`, or NULL with an exception set. */
static PyObject *pycc_ext_slot_default_new(PyObject *slot, PyObject *dflt)
{
    PyObject *args[2];

    if (pycc_ext_slot_default_type == NULL) {
        pycc_ext_slot_default_type = PyType_FromSpec(&pycc_slot_default_spec);
        if (pycc_ext_slot_default_type == NULL) {
            return NULL;
        }
    }
    args[0] = slot;
    args[1] = dflt;
    return PyObject_Vectorcall(pycc_ext_slot_default_type, args, 2, NULL);
}

/*
 * `types.resolve_bases(bases)`: each base that is not a type but has
 * `__mro_entries__` replaced by the tuple that returns. A new reference to
 * `bases` itself when none was replaced, as `resolve_bases` returns it.
 */
static PyObject *pycc_ext_resolve_bases(PyObject *bases)
{
    PyObject *out = NULL;
    Py_ssize_t i, n = PyTuple_Size(bases);
    int updated = 0;

    out = PyList_New(0);
    if (out == NULL) {
        return NULL;
    }
    for (i = 0; i < n; i++) {
        PyObject *base = PyTuple_GetItem(bases, i);
        PyObject *entries = NULL;
        PyObject *meth = NULL;
        int found;

        if (PyType_Check(base)) {
            if (PyList_Append(out, base) < 0) {
                goto fail;
            }
            continue;
        }
        found = PyObject_GetOptionalAttr(base, pycc_ext_attr_name("__mro_entries__"), &meth);
        if (found < 0) {
            goto fail;
        }
        if (found == 0) {
            if (PyList_Append(out, base) < 0) {
                goto fail;
            }
            continue;
        }
        entries = PyObject_CallFunctionObjArgs(meth, bases, NULL);
        Py_DECREF(meth);
        if (entries == NULL) {
            goto fail;
        }
        updated = 1;
        if (!PyTuple_Check(entries)) {
            Py_DECREF(entries);
            PyErr_SetString(PyExc_TypeError, "__mro_entries__ must return a tuple");
            goto fail;
        }
        {
            Py_ssize_t j;

            for (j = 0; j < PyTuple_Size(entries); j++) {
                if (PyList_Append(out, PyTuple_GetItem(entries, j)) < 0) {
                    Py_DECREF(entries);
                    goto fail;
                }
            }
        }
        Py_DECREF(entries);
    }
    if (!updated) {
        Py_DECREF(out);
        return Py_NewRef(bases);
    }
    {
        PyObject *tuple = PyList_AsTuple(out);

        Py_DECREF(out);
        return tuple;
    }
fail:
    Py_DECREF(out);
    return NULL;
}

/*
 * `types._calculate_meta(meta, bases)`: the most derived of `meta` and the
 * bases' metaclasses; a new reference, or NULL (a conflict) with an
 * exception set.
 */
static PyObject *pycc_ext_calculate_meta(PyObject *meta, PyObject *bases)
{
    PyObject *winner = meta;
    Py_ssize_t i;

    for (i = 0; i < PyTuple_Size(bases); i++) {
        PyObject *base_meta = (PyObject *)Py_TYPE(PyTuple_GetItem(bases, i));
        int sub = PyObject_IsSubclass(winner, base_meta);

        if (sub < 0) {
            return NULL;
        }
        if (sub) {
            continue;
        }
        sub = PyObject_IsSubclass(base_meta, winner);
        if (sub < 0) {
            return NULL;
        }
        if (sub) {
            winner = base_meta;
            continue;
        }
        PyErr_SetString(PyExc_TypeError, "metaclass conflict");
        return NULL;
    }
    return Py_NewRef(winner);
}

/* `ns.setdefault(key, value)`; 0, or -1 with an exception set. */
static int pycc_ext_ns_setdefault(PyObject *ns, PyObject *key, PyObject *value)
{
    PyObject *done;

    if (PyDict_CheckExact(ns)) {
        int has = PyDict_Contains(ns, key);

        return has < 0 ? -1 : has ? 0 : PyDict_SetItem(ns, key, value);
    }
    done = PyObject_CallMethodObjArgs(ns, pycc_ext_attr_name("setdefault"), key, value, NULL);
    Py_XDECREF(done);
    return done == NULL ? -1 : 0;
}

/* Whether `key` is one of the template's own entries a class never takes
 * over from it. */
static int pycc_ext_class_skipped(PyObject *key)
{
    static const char *const skipped[] = {
        "__module__", "__qualname__", "__doc__", "__dict__", "__weakref__",
        "__init_subclass__", "__new__", "__slots__", NULL,
    };
    const char *const *name;

    if (!PyUnicode_Check(key)) {
        return 0;
    }
    for (name = skipped; *name != NULL; name++) {
        if (PyUnicode_CompareWithASCIIString(key, *name) == 0) {
            return 1;
        }
    }
    return 0;
}

/*
 * The published class `name` of module `module` (its name), made from
 * `template` with the statement's positional arguments `args` (a tuple),
 * keyword arguments `kwargs` (a dict) and class-body defaults `literals`
 * (a dict): as `__build_class__` would make `class name(template, *args,
 * **kwargs)` whose namespace holds `__module__`, `__qualname__`, empty
 * `__slots__`, the defaults (a slot's as a `_SlotDefault`), the template's
 * own entries and, when a base was resolved, `__orig_bases__`. A new
 * reference, or NULL with an exception set.
 */
static PyObject *pycc_ext_make_class(PyObject *template, PyObject *name, PyObject *module,
                                     PyObject *args, PyObject *kwargs, PyObject *literals)
{
    PyObject *resolved = NULL, *full = NULL, *kwds = NULL, *meta = NULL, *ns = NULL;
    PyObject *tdict = NULL, *items = NULL, *call_args = NULL, *cls = NULL;
    PyObject *key, *value;
    Py_ssize_t i, pos;
    int found, differs;

    resolved = pycc_ext_resolve_bases(args);
    if (resolved == NULL) {
        goto done;
    }
    full = PyTuple_New(PyTuple_Size(resolved) + 1);
    if (full == NULL) {
        goto done;
    }
    PyTuple_SetItem(full, 0, Py_NewRef(template));
    for (i = 0; i < PyTuple_Size(resolved); i++) {
        PyTuple_SetItem(full, i + 1, Py_NewRef(PyTuple_GetItem(resolved, i)));
    }
    /* `types.prepare_class(name, full, dict(kwargs))`. */
    kwds = PyDict_Copy(kwargs);
    if (kwds == NULL) {
        goto done;
    }
    found = PyDict_GetItemRef(kwds, pycc_ext_attr_name("metaclass"), &meta);
    if (found < 0) {
        goto done;
    }
    if (found) {
        if (PyDict_DelItem(kwds, pycc_ext_attr_name("metaclass")) < 0) {
            goto done;
        }
    } else {
        meta = Py_NewRef((PyObject *)Py_TYPE(template));
    }
    found = PyObject_IsInstance(meta, (PyObject *)&PyType_Type);
    if (found < 0) {
        goto done;
    }
    if (found) {
        PyObject *winner = pycc_ext_calculate_meta(meta, full);

        if (winner == NULL) {
            goto done;
        }
        PYCC_SETREF(meta, winner);
    }
    {
        PyObject *prepare = NULL;

        found = PyObject_GetOptionalAttr(meta, pycc_ext_attr_name("__prepare__"), &prepare);
        if (found < 0) {
            goto done;
        }
        if (found) {
            PyObject *pargs = PyTuple_Pack(2, name, full);

            ns = pargs == NULL ? NULL : PyObject_Call(prepare, pargs, kwds);
            Py_XDECREF(pargs);
            Py_DECREF(prepare);
        } else {
            ns = PyDict_New();
        }
        if (ns == NULL) {
            goto done;
        }
    }
    if (PyObject_SetItem(ns, pycc_ext_attr_name("__module__"), module) < 0
        || PyObject_SetItem(ns, pycc_ext_attr_name("__qualname__"), name) < 0) {
        goto done;
    }
    {
        PyObject *empty = PyTuple_New(0);

        if (empty == NULL || PyObject_SetItem(ns, pycc_ext_attr_name("__slots__"), empty) < 0) {
            Py_XDECREF(empty);
            goto done;
        }
        Py_DECREF(empty);
    }
    tdict = PyObject_GetAttr(template, pycc_ext_attr_name("__dict__"));
    if (tdict == NULL) {
        goto done;
    }
    pos = 0;
    while (PyDict_Next(literals, &pos, &key, &value)) {
        PyObject *slot = NULL;
        PyObject *entry = Py_NewRef(value);
        int status;

        if (PyMapping_GetOptionalItem(tdict, key, &slot) < 0) {
            Py_DECREF(entry);
            goto done;
        }
        if (slot != NULL) {
            PyObject *kind = PyType_GetName(Py_TYPE(slot));
            int getset = kind != NULL && PyUnicode_CompareWithASCIIString(kind, "getset_descriptor") == 0;

            Py_XDECREF(kind);
            if (getset) {
                PYCC_SETREF(entry, pycc_ext_slot_default_new(slot, value));
            }
            Py_DECREF(slot);
            if (entry == NULL) {
                goto done;
            }
        }
        status = pycc_ext_ns_setdefault(ns, key, entry);
        Py_DECREF(entry);
        if (status < 0) {
            goto done;
        }
    }
    items = PyMapping_Items(tdict);
    if (items == NULL) {
        goto done;
    }
    for (i = 0; i < PyList_Size(items); i++) {
        PyObject *item = PyList_GetItem(items, i);

        key = PyTuple_GetItem(item, 0);
        if (!pycc_ext_class_skipped(key)
            && pycc_ext_ns_setdefault(ns, key, PyTuple_GetItem(item, 1)) < 0) {
            goto done;
        }
    }
    differs = PyObject_RichCompareBool(resolved, args, Py_NE);
    if (differs < 0
        || (differs && PyObject_SetItem(ns, pycc_ext_attr_name("__orig_bases__"), args) < 0)) {
        goto done;
    }
    call_args = PyTuple_Pack(3, name, full, ns);
    if (call_args != NULL) {
        cls = PyObject_Call(meta, call_args, kwds);
    }
done:
    Py_XDECREF(call_args);
    Py_XDECREF(items);
    Py_XDECREF(tdict);
    Py_XDECREF(ns);
    Py_XDECREF(meta);
    Py_XDECREF(kwds);
    Py_XDECREF(full);
    Py_XDECREF(resolved);
    return cls;
}

/*
 * The value name `text` (`len` bytes) has in the module namespace `dict`,
 * as `eval` with `dict` as its globals and locals reads it: from `dict`,
 * else from its builtins (`__builtins__`, a module's dict or a mapping; the
 * interpreter's when `dict` has none). A new reference, or NULL with an
 * exception set.
 */
static PyObject *pycc_ext_class_arg(PyObject *dict, const char *text, Py_ssize_t len)
{
    PyObject *key = PyUnicode_FromStringAndSize(text, len);
    PyObject *value = NULL;

    if (key == NULL) {
        return NULL;
    }
    if (PyDict_GetItemRef(dict, key, &value) == 0) {
        PyObject *builtins = NULL;
        int found = PyDict_GetItemRef(dict, pycc_ext_attr_name("__builtins__"), &builtins);

        if (found == 0) {
            builtins = Py_NewRef(PyEval_GetBuiltins());
        } else if (found > 0 && PyModule_Check(builtins)) {
            PYCC_SETREF(builtins, Py_NewRef(PyModule_GetDict(builtins)));
        }
        if (found >= 0 && PyMapping_GetOptionalItem(builtins, key, &value) == 0) {
            PyErr_Format(PyExc_NameError, "name '%U' is not defined", key);
        }
        Py_XDECREF(builtins);
    }
    Py_DECREF(key);
    return value;
}

/*
 * Evaluates a class statement's arguments from their plan `spec`
 * (`class_spec` in src/ext_build/publication.rs: an `a NAME` line per
 * positional argument, in source order, then an `n NAME` line per `None`
 * default) in the module namespace `dict`, into `*args` (a tuple),
 * `*kwargs` (an empty dict) and `*literals` (a dict), as `(lambda *a, **k:
 * (a, k))(...)` and the defaults' dict display would. 0, or -1 with an
 * exception set and nothing stored.
 */
static int pycc_ext_class_args(PyObject *dict, const char *spec, PyObject **args,
                               PyObject **kwargs, PyObject **literals)
{
    PyObject *positional = PyList_New(0);
    PyObject *keywords = PyDict_New();
    PyObject *defaults = PyDict_New();
    const char *line = spec;

    if (positional == NULL || keywords == NULL || defaults == NULL) {
        goto fail;
    }
    while (*line != '\0') {
        const char *eol = strchr(line, '\n');
        const char *text = line + 2;
        PyObject *value;
        int status;

        if (eol == NULL) {
            eol = line + strlen(line);
        }
        if (line[0] == 'a') {
            value = pycc_ext_class_arg(dict, text, eol - text);
            status = value == NULL ? -1 : PyList_Append(positional, value);
            Py_XDECREF(value);
        } else {
            PyObject *key = PyUnicode_FromStringAndSize(text, eol - text);

            status = key == NULL ? -1 : PyDict_SetItem(defaults, key, Py_None);
            Py_XDECREF(key);
        }
        if (status < 0) {
            goto fail;
        }
        line = *eol == '\0' ? eol : eol + 1;
    }
    *args = PyList_AsTuple(positional);
    Py_DECREF(positional);
    if (*args == NULL) {
        Py_DECREF(keywords);
        Py_DECREF(defaults);
        return -1;
    }
    *kwargs = keywords;
    *literals = defaults;
    return 0;
fail:
    Py_XDECREF(positional);
    Py_XDECREF(keywords);
    Py_XDECREF(defaults);
    return -1;
}

/*
 * The same from the arguments' source text `arguments` (`(...), {...}`,
 * parentheses included), for arguments the build could not plan: compiled
 * and evaluated in `dict` as `(lambda *a, **k: (a, k))` called with them.
 * 0, or -1 with an exception set and nothing stored.
 */
static int pycc_ext_class_args_from_source(PyObject *dict, const char *arguments,
                                           PyObject **args, PyObject **kwargs,
                                           PyObject **literals)
{
    static const char prefix[] = "(lambda *a, **k: (a, k))";
    size_t len = strlen(arguments);
    char *source = (char *)PyMem_Malloc(sizeof prefix + len);
    PyObject *code, *evaluated, *pair;

    if (source == NULL) {
        PyErr_NoMemory();
        return -1;
    }
    memcpy(source, prefix, sizeof prefix - 1);
    memcpy(source + sizeof prefix - 1, arguments, len + 1);
    code = Py_CompileString(source, "<pycc class bases>", Py_eval_input);
    PyMem_Free(source);
    evaluated = code == NULL ? NULL : PyEval_EvalCode(code, dict, dict);
    Py_XDECREF(code);
    /* `((args, kwargs), literals)`. */
    pair = evaluated != NULL && PyTuple_Check(evaluated) && PyTuple_Size(evaluated) == 2
        ? PyTuple_GetItem(evaluated, 0)
        : NULL;
    if (pair == NULL || !PyTuple_Check(pair) || PyTuple_Size(pair) != 2) {
        Py_XDECREF(evaluated);
        if (!PyErr_Occurred()) {
            PyErr_SetString(PyExc_SystemError, "pycc: class arguments evaluated to no pair");
        }
        return -1;
    }
    *args = Py_NewRef(PyTuple_GetItem(pair, 0));
    *kwargs = Py_NewRef(PyTuple_GetItem(pair, 1));
    *literals = Py_NewRef(PyTuple_GetItem(evaluated, 1));
    Py_DECREF(evaluated);
    return 0;
}

/*
 * The published class `name` of `module`, made from `template` with its
 * statement's arguments -- their plan `spec` when the build made one, else
 * their source text `arguments` -- evaluated in the module's namespace;
 * entered as the class's carrier type. A new reference, or NULL with an
 * exception set.
 */
static PyObject *pycc_ext_make_public_class_in(PyccExtModule *m, PyObject *module,
                                               const char *name, PyObject *template,
                                               const char *spec, const char *arguments)
{
    PyObject *dict = PyModule_GetDict(module);
    PyObject *args = NULL, *kwargs = NULL, *literals = NULL;
    PyObject *modname;
    PyObject *call;
    PyObject *cls;
    int status;

    if (dict == NULL) {
        return NULL;
    }
    status = spec != NULL ? pycc_ext_class_args(dict, spec, &args, &kwargs, &literals)
                          : pycc_ext_class_args_from_source(dict, arguments, &args, &kwargs,
                                                            &literals);
    if (status < 0 || !PyTuple_Check(args) || !PyDict_Check(kwargs) || !PyDict_Check(literals)) {
        Py_XDECREF(args);
        Py_XDECREF(kwargs);
        Py_XDECREF(literals);
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
        Py_DECREF(args);
        Py_DECREF(kwargs);
        Py_DECREF(literals);
        return NULL;
    }
    pycc_ext_making_public_class++;
    cls = pycc_ext_make_class(template, call, modname, args, kwargs, literals);
    pycc_ext_making_public_class--;
    if (cls == NULL) {
        /* Bases CPython cannot combine with the template (a layout or
         * metaclass conflict): published as the template, as above. */
        PyErr_Clear();
        cls = Py_NewRef(template);
    }
    Py_DECREF(call);
    Py_DECREF(modname);
    Py_DECREF(args);
    Py_DECREF(kwargs);
    Py_DECREF(literals);
    if (cls != NULL
        && (pycc_ext_carrier_register_in(m, name, cls) < 0
            || pycc_ext_note_compiled_class(cls) < 0)) {
        Py_CLEAR(cls);
    }
    return cls;
}
