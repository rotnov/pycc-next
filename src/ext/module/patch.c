/*
 * The hooks a module's body calls (`forward.c`'s machinery, continued):
 * `__pycc_forward__`, which patches the forwarders of each statement CPython
 * runs, and `__pycc_patch__`, which patches a module's selected functions
 * (docs/design/compile-little.md, section 2.4) once its code has run -- with
 * the owner paths and private names resolved as CPython's class statements
 * store them, and every selected function it cannot patch counted, never
 * dropped silently -- and their install and removal around the body.
 */

/* `name` as the class statement of class `cls` stores it (a new reference):
 * a private `__name` (not a dunder, no dot) becomes `_cls__name`, with the
 * class name's leading underscores dropped, as CPython's compiler mangles
 * it; anything else is `name` itself. NULL with an exception set on failure. */
static PyObject *pycc_fwd_mangle(PyObject *cls, PyObject *name)
{
    Py_ssize_t len = PyUnicode_GetLength(name), start = 0, n;

    if (len < 0 || cls == NULL) {
        return len < 0 ? NULL : Py_NewRef(name);
    }
    if (len < 3 || PyUnicode_ReadChar(name, 0) != '_' || PyUnicode_ReadChar(name, 1) != '_'
        || (PyUnicode_ReadChar(name, len - 1) == '_' && PyUnicode_ReadChar(name, len - 2) == '_')
        || PyUnicode_FindChar(name, '.', 0, len, 1) >= 0) {
        return Py_NewRef(name);
    }
    n = PyUnicode_GetLength(cls);
    while (start < n && PyUnicode_ReadChar(cls, start) == '_') {
        start++;
    }
    if (start == n) {
        return Py_NewRef(name);
    }
    {
        PyObject *stripped = PyUnicode_Substring(cls, start, n);
        PyObject *mangled = stripped == NULL ? NULL : PyUnicode_FromFormat("_%U%U", stripped, name);

        Py_XDECREF(stripped);
        return mangled;
    }
}

/* The dict of the class the owner path `path` (`("C",)`, `("Outer",
 * "Inner")`) names in `ns`, each class along it read from its owner's dict
 * (so no descriptor runs): a new reference, or NULL when some part is
 * missing or not a class. */
static PyObject *pycc_fwd_owner(PyObject *ns, PyObject *path)
{
    PyObject *dict = NULL;
    Py_ssize_t i, n = PyTuple_Size(path);

    for (i = 0; i < n; i++) {
        /* A private nested class is stored mangled by its owner. */
        PyObject *key = i == 0 ? Py_NewRef(PyTuple_GetItem(path, 0))
                               : pycc_fwd_mangle(PyTuple_GetItem(path, i - 1), PyTuple_GetItem(path, i));
        PyObject *obj = key == NULL ? NULL : pycc_fwd_lookup(i == 0 ? ns : dict, key);

        Py_XDECREF(key);
        PyErr_Clear();
        Py_XDECREF(dict);
        dict = obj != NULL && PyType_Check(obj) ? pycc_fwd_class_dict(obj) : NULL;
        Py_XDECREF(obj);
        if (dict == NULL) {
            break;
        }
    }
    return dict;
}

/* The capsule name of a module's record, the `self` of its
 * `__pycc_forward__`. */
#define PYCC_FWD_CAPSULE "pycc.module"

/* `__pycc_forward__(name, ..., (owner, ...), name, ...)`: patches the
 * forwarder each name names in the running statement's namespace, into the
 * slots of the module whose record `self` holds. The build splits each
 * lift's qualified name (`src/fallback/lift.rs`): a name is a member of
 * the class the last owner path before it names (`("C",)` for `C.m`,
 * `("Outer", "Inner")` for `Outer.Inner.m`), or of the namespace itself
 * before any path or after `()`. */
static PyObject *pycc_ext_forward(PyObject *self, PyObject *const *args, Py_ssize_t nargs)
{
    PyObject *ns = PyEval_GetGlobals();
    PyccExtModule *m = (PyccExtModule *)PyCapsule_GetPointer(self, PYCC_FWD_CAPSULE);
    /* The dict the names are read from: `ns` (borrowed) or an owner's. */
    PyObject *owner = NULL, *in;
    Py_ssize_t i;

    if (m == NULL) {
        return NULL;
    }
    if (pycc_fwd_original == NULL || ns == NULL || !PyDict_Check(ns)) {
        Py_RETURN_NONE;
    }
    in = ns;
    for (i = 0; i < nargs; i++) {
        PyObject *obj;

        if (PyTuple_Check(args[i])) {
            Py_CLEAR(owner);
            if (PyTuple_Size(args[i]) == 0) {
                in = ns;
            } else {
                owner = pycc_fwd_owner(ns, args[i]);
                in = owner;
            }
            continue;
        }
        obj = in == NULL ? NULL : pycc_fwd_lookup(in, args[i]);
        if (obj != NULL) {
            pycc_fwd_walk(m, ns, obj, 0, NULL);
            Py_DECREF(obj);
        }
        PyErr_Clear();
    }
    Py_XDECREF(owner);
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_forward_def = {
    "__pycc_forward__", (PyCFunction)(void (*)(void))pycc_ext_forward, METH_FASTCALL, NULL,
};

/* `__pycc_patch__(owner, name, lift, line, ..., (owner, ...), name, ...)`:
 * points each selected function at its compiled body, once the module's
 * code has run (`src/fallback/selected.rs`). Owner paths are
 * `__pycc_forward__`'s; each function is its name, its lift's name and its
 * first line times two, plus one when the lift takes the class. The
 * function bound under the name, or the one a `property`, `classmethod`,
 * `staticmethod` or decorator's `__wrapped__` there holds, is patched when
 * its code's qualified name and first line are the selected function's.
 * Each lift is then unbound from the namespace: it was published only for
 * this. */
/* Whether an environment variable is set to something other than "" or
 * "0". */
static int pycc_fwd_env_on(const char *name)
{
    const char *value = getenv(name);

    return value != NULL && value[0] != '\0' && strcmp(value, "0") != 0;
}

/* With no patching on the running interpreter (the frames probe failed: a
 * CPython other than 3.13 and 3.15, or `PYCC_NO_PATCH` set), the selected
 * functions would run as their bytecode: never silently. Under
 * `PYCC_REQUIRE_PATCH` (`pycc profile`'s pass 3, which must not time
 * bytecode against bytecode) the import fails; otherwise it warns once per
 * module. 0, or -1 with an exception set. */
static int pycc_fwd_patch_unavailable(PyObject *ns)
{
    PyObject *name = PyDict_GetItemString(ns, "__name__");

    if (name == NULL) {
        name = Py_None;
    }
    if (pycc_fwd_env_on("PYCC_REQUIRE_PATCH")) {
        PyErr_Format(PyExc_ImportError,
                     "pycc: %S: the selected functions cannot be patched on this interpreter "
                     "(patching needs CPython 3.13's or 3.15's layouts): they would run as bytecode",
                     name);
        return -1;
    }
    return PyErr_WarnFormat(PyExc_RuntimeWarning, 1,
                            "pycc: %S: the selected functions run as bytecode: patching them needs "
                            "CPython 3.13's or 3.15's layouts",
                            name);
}

static PyObject *pycc_ext_patch(PyObject *self, PyObject *const *args, Py_ssize_t nargs)
{
    PyObject *ns = PyEval_GetGlobals();
    PyccExtModule *m = (PyccExtModule *)PyCapsule_GetPointer(self, PYCC_FWD_CAPSULE);
    PyObject *owner = NULL, *in, *prefix = PyUnicode_FromString("");
    /* The owner path's last class, which mangles a private member's name. */
    PyObject *cls = NULL;
    PyObject *missed = NULL;
    Py_ssize_t i, misses = 0;
    int available;

    if (m == NULL || prefix == NULL) {
        Py_XDECREF(prefix);
        return NULL;
    }
    if (ns == NULL || !PyDict_Check(ns)) {
        Py_DECREF(prefix);
        Py_RETURN_NONE;
    }
    available = pycc_fwd_original != NULL && !pycc_fwd_env_on("PYCC_NO_PATCH");
    if (!available && nargs > 0 && pycc_fwd_patch_unavailable(ns) != 0) {
        Py_DECREF(prefix);
        return NULL;
    }
    in = ns;
    for (i = 0; i < nargs; i++) {
        PycFwdWant want;
        PyObject *obj;
        long flags;

        if (PyTuple_Check(args[i])) {
            Py_CLEAR(owner);
            Py_CLEAR(prefix);
            cls = NULL;
            if (PyTuple_Size(args[i]) == 0) {
                in = ns;
                prefix = PyUnicode_FromString("");
            } else {
                cls = PyTuple_GetItem(args[i], PyTuple_Size(args[i]) - 1);
                PyObject *dot = PyUnicode_FromString(".");

                owner = pycc_fwd_owner(ns, args[i]);
                in = owner;
                prefix = dot == NULL ? NULL : PyUnicode_Join(dot, args[i]);
                Py_XDECREF(dot);
                if (prefix != NULL) {
                    PyObject *joined = prefix;

                    prefix = PyUnicode_FromFormat("%U.", joined);
                    Py_DECREF(joined);
                }
            }
            if (prefix == NULL) {
                Py_XDECREF(owner);
                Py_XDECREF(missed);
                return NULL;
            }
            continue;
        }
        if (i + 2 >= nargs) {
            break;
        }
        flags = PyLong_AsLong(args[i + 2]);
        want.target = PyDict_GetItemWithError(ns, args[i + 1]);
        want.qualname = PyUnicode_FromFormat("%U%U", prefix, args[i]);
        want.firstlineno = (int)(flags >> 1);
        want.uses_class = (int)(flags & 1);
        if (available) {
            Py_ssize_t used = m->fwd_used;
            PyObject *key = pycc_fwd_mangle(cls, args[i]);

            obj = key == NULL || in == NULL ? NULL : pycc_fwd_lookup(in, key);
            Py_XDECREF(key);
            if (want.qualname != NULL && want.target != NULL && obj != NULL) {
                Py_INCREF(want.target);
                pycc_fwd_walk(m, ns, obj, 0, &want);
                Py_DECREF(want.target);
            }
            Py_XDECREF(obj);
            /* A selected function not patched would run as its bytecode:
             * counted, never dropped silently. */
            if (m->fwd_used == used) {
                misses++;
                if (missed == NULL && want.qualname != NULL) {
                    missed = Py_NewRef(want.qualname);
                }
            }
        }
        Py_XDECREF(want.qualname);
        PyErr_Clear();
        if (PyDict_DelItem(ns, args[i + 1]) != 0) {
            PyErr_Clear();
        }
        i += 2;
    }
    Py_XDECREF(owner);
    Py_XDECREF(prefix);
    if (misses > 0) {
        PyObject *name = PyDict_GetItemString(ns, "__name__");
        int status;

        if (name == NULL) {
            name = Py_None;
        }
        if (missed == NULL) {
            missed = PyUnicode_FromString("?");
        }
        if (missed == NULL) {
            return NULL;
        }
        if (pycc_fwd_env_on("PYCC_REQUIRE_PATCH")) {
            PyErr_Format(PyExc_ImportError,
                         "pycc: %S: %zd selected functions could not be patched (%U among them): "
                         "they would run as bytecode",
                         name, misses, missed);
            status = -1;
        } else {
            status = PyErr_WarnFormat(PyExc_RuntimeWarning, 1,
                                      "pycc: %S: %zd selected functions could not be patched (%U "
                                      "among them): they run as bytecode",
                                      name, misses, missed);
        }
        Py_DECREF(missed);
        if (status != 0) {
            return NULL;
        }
    }
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_patch_def = {
    "__pycc_patch__", (PyCFunction)(void (*)(void))pycc_ext_patch, METH_FASTCALL, NULL,
};

/* Binds `__pycc_forward__` in the namespace of `module`, of record `m`, for
 * the body; 0, or -1 with an exception set. */
static int pycc_ext_forward_install(PyccExtModule *m, PyObject *module)
{
    PyObject *hook;
    PyObject *capsule;
    int status;

    if (*m->fwd_slot_count > 0 && m->fwd_slots == NULL) {
        /* Each slot is cleared when a forwarder takes it. */
        m->fwd_slots = (PycFwdSlot *)PyMem_Malloc((size_t)*m->fwd_slot_count * sizeof(PycFwdSlot));
        if (m->fwd_slots == NULL) {
            PyErr_NoMemory();
            return -1;
        }
    }
    pycc_forward_probe();
    capsule = PyCapsule_New(m, PYCC_FWD_CAPSULE, NULL);
    if (capsule == NULL) {
        return -1;
    }
    hook = PyCFunction_NewEx(&pycc_ext_forward_def, capsule, NULL);
    if (hook == NULL) {
        Py_DECREF(capsule);
        return -1;
    }
    status = PyDict_SetItemString(PyModule_GetDict(module), "__pycc_forward__", hook);
    Py_DECREF(hook);
    hook = status != 0 ? NULL : PyCFunction_NewEx(&pycc_ext_patch_def, capsule, NULL);
    Py_DECREF(capsule);
    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(PyModule_GetDict(module), "__pycc_patch__", hook);
    Py_DECREF(hook);
    return status;
}

/* Removes the hook once the body has run, keeping any exception. */
static void pycc_ext_forward_remove(PyObject *module)
{
    PyObject *raised = PyErr_GetRaisedException();

    if (PyDict_DelItemString(PyModule_GetDict(module), "__pycc_forward__") != 0) {
        PyErr_Clear();
    }
    if (PyDict_DelItemString(PyModule_GetDict(module), "__pycc_patch__") != 0) {
        PyErr_Clear();
    }
    PyErr_SetRaisedException(raised);
}

