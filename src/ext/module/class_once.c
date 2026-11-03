/* v0.12: a published class made once, at its statement, from its compiled
 * spec (`src/ext_build/deferred_classes.rs`), when its bases allow it;
 * class_maker.c makes the others from a template. */

/*
 * v0.12: whether `base`, a class statement's argument, lets the class be
 * made in one step (`pycc_ext_make_class_once`): a compiled class of this
 * extension whose metaclass is `type` and whose MRO holds only compiled
 * classes and `object`. Then no `__init_subclass__`, `__set_name__`,
 * `__mro_entries__` or metaclass of Python code can run while the class is
 * made: every hook `type.__new__` would call is `object`'s, which does
 * nothing. 1 or 0, or -1 with an exception set.
 */
static int pycc_ext_once_base(PyObject *base)
{
    PyObject *mro;
    Py_ssize_t i;
    int ok = 1;

    if (!PyType_Check(base) || Py_TYPE(base) != &PyType_Type || !pycc_ext_is_compiled_class(base)) {
        return 0;
    }
    mro = PyObject_GetAttr(base, pycc_ext_attr_name("__mro__"));
    if (mro == NULL) {
        return -1;
    }
    for (i = 0; ok && i < PyTuple_Size(mro); i++) {
        PyObject *cls = PyTuple_GetItem(mro, i);

        ok = cls == (PyObject *)&PyBaseObject_Type || pycc_ext_is_compiled_class(cls);
    }
    Py_DECREF(mro);
    return ok;
}

/*
 * v0.12: the published class `name` of module `modname`, made in one step
 * from its compiled layout `spec` (the template's spec), for the common
 * statement: positional arguments only, each a compiled class
 * `pycc_ext_once_base` accepts, or none but `object`. What
 * `pycc_ext_make_class` gives the class made from the template, this gives
 * the class itself, without the template between it and its bases: the
 * name, `__module__`, `__qualname__`, `__slots__ = ()`, the class-body
 * defaults `literals` (a slot's as a `_SlotDefault`), the class constants
 * (`constants`), the template's methods, slots and field descriptors, and no
 * `__init_subclass__` of its own. `type.__new__` would call no hook but
 * `object`'s (`pycc_ext_once_base`), and nothing in the namespace has a
 * `__set_name__`, so none is called. A new reference; NULL with an
 * exception set on a failure, or with none and `*declined` set when the
 * statement is not the common one, which the caller then makes from the
 * template.
 */
static PyObject *pycc_ext_make_class_once(PyType_Spec *spec, int (*constants)(PyObject *),
                                          PyObject *name, PyObject *modname, PyObject *args,
                                          PyObject *literals, int *declined)
{
    PyObject *root = pycc_ext_template_base();
    PyObject *bases = NULL, *cls = NULL, *empty = NULL;
    PyObject *key, *value;
    PyType_Spec once;
    Py_ssize_t i, n, pos;
    int compiled = 0;

    *declined = 0;
    if (root == NULL) {
        return NULL;
    }
    n = PyTuple_Size(args);
    for (i = 0; i < n; i++) {
        PyObject *base = PyTuple_GetItem(args, i);
        int ok;

        if (base == (PyObject *)&PyBaseObject_Type && n == 1) {
            continue;
        }
        ok = pycc_ext_once_base(base);
        if (ok <= 0) {
            *declined = ok == 0;
            return NULL;
        }
        compiled = 1;
    }
    /* `class C:` and `class C(object):` derive from the root every template
     * derives from, as the class made from the template did through it. */
    bases = compiled ? Py_NewRef(args)
                     : n == 0 ? PyTuple_Pack(1, root)
                              : PyTuple_Pack(2, root, (PyObject *)&PyBaseObject_Type);
    if (bases == NULL) {
        return NULL;
    }
    once = *spec;
    once.flags &= ~(unsigned int)Py_TPFLAGS_IMMUTABLETYPE;
    cls = PyType_FromMetaclass(NULL, NULL, &once, bases);
    Py_DECREF(bases);
    if (cls == NULL) {
        return NULL;
    }
    /* The spec is named `<module>.<Class>`; `type.__new__` names the class
     * `<Class>` (`tp_name`, which error messages show) in the module its
     * statement ran in. The template's `__init_subclass__`, which only lets
     * the class made from it through, is not the class's own. */
    empty = PyTuple_New(0);
    if (empty == NULL
        || PyObject_SetAttr(cls, pycc_ext_attr_name("__name__"), name) < 0
        || PyObject_SetAttr(cls, pycc_ext_attr_name("__module__"), modname) < 0
        || PyObject_DelAttr(cls, pycc_ext_attr_name("__init_subclass__")) < 0
        || PyObject_SetAttr(cls, pycc_ext_attr_name("__slots__"), empty) < 0
        || constants(cls) < 0) {
        goto fail;
    }
    Py_CLEAR(empty);
    pos = 0;
    while (PyDict_Next(literals, &pos, &key, &value)) {
        PyObject *slot = NULL;
        PyObject *entry = Py_NewRef(value);
        int status;

        /* A field descriptor is the class's own (its spec's), and reads
         * as itself on the class. */
        if (PyObject_GetOptionalAttr(cls, key, &slot) < 0) {
            Py_DECREF(entry);
            goto fail;
        }
        if (slot != NULL) {
            if (pycc_ext_is_getset_type(Py_TYPE(slot))) {
                PYCC_SETREF(entry, pycc_ext_slot_default_new(slot, value));
            }
            Py_DECREF(slot);
            if (entry == NULL) {
                goto fail;
            }
        }
        status = PyObject_SetAttr(cls, key, entry);
        Py_DECREF(entry);
        if (status < 0) {
            goto fail;
        }
    }
    /* Its storage stays reachable for pickling and copying once Python code
     * replaces a field's entry (`carriers_pickle.c`). */
    if (pycc_ext_note_field_descriptors(cls) < 0) {
        goto fail;
    }
    return cls;
fail:
    Py_XDECREF(empty);
    Py_DECREF(cls);
    return NULL;
}

/*
 * v0.12: the published class `name` of `module` made in one step at its
 * statement (`pycc_ext_make_class_once`), from its compiled layout `spec`
 * and class constants `constants`, its arguments evaluated from their plan
 * `plan` in the module's namespace; entered as the class's carrier type. A
 * new reference; NULL with `*declined` set and no exception when the
 * statement is not the common one (no plan, an argument the namespace does
 * not bind, a base that is not a compiled class) or making it failed: the
 * caller then makes the class from its template
 * (`pycc_ext_make_public_class_in`), as before v0.12, which meets the
 * failure again and handles it as it always has. NULL with an exception set
 * only when the class was made but could not be entered as the carrier.
 */
static PyObject *pycc_ext_make_public_class_once_in(PyccExtModule *m, PyObject *module,
                                                    const char *name, PyType_Spec *spec,
                                                    int (*constants)(PyObject *),
                                                    const char *plan, int *declined)
{
    PyObject *dict = PyModule_GetDict(module);
    PyObject *args = NULL, *kwargs = NULL, *literals = NULL;
    PyObject *modname = NULL, *call = NULL, *cls = NULL;

    *declined = 1;
    if (plan == NULL || dict == NULL) {
        PyErr_Clear();
        return NULL;
    }
    if (pycc_ext_class_args(dict, plan, &args, &kwargs, &literals) < 0) {
        /* Evaluated again, and the failure handled, by the template's path. */
        PyErr_Clear();
        return NULL;
    }
    modname = PyModule_GetNameObject(module);
    call = modname == NULL ? NULL : PyUnicode_FromString(name);
    if (call != NULL) {
        cls = pycc_ext_make_class_once(spec, constants, call, modname, args, literals, declined);
    }
    if (cls == NULL) {
        /* Any failure is the template's path's to meet again, which handles
         * it as before v0.12 (it publishes the template). */
        PyErr_Clear();
        *declined = 1;
    }
    Py_XDECREF(call);
    Py_XDECREF(modname);
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

