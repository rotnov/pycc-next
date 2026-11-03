/* Pickling and `copy.deepcopy` of a compiled instance (v0.10), on the carrier
 * types of carriers.c. */

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

