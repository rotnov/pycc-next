/*
 * #1199: binds one top-level definition on the executing module, called by
 * LLVM-generated code right after the definition's `fnptr_` slot store (and
 * after the last slot of a class statement), so the name becomes a module
 * attribute exactly when CPython would bind it. Before #1199 every export
 * was published before the body ran, and a foreign module that imported
 * this one back mid-body could call a wrapper whose slot was still null.
 *
 * The name is resolved first, against `pycc_ext_methods[]` (module-level
 * functions; never the PEP 562 hook table, which the exec slot adds after
 * the body) and then the generated published-class lookup. A name in
 * neither -- a function the driver did not export, or anything at all in
 * the embedded launcher, whose tables are empty -- is not an error: codegen
 * publishes from the MIR alone and may name a superset of the exports, so
 * the answer is 0 without touching any state. Only a resolved name reads
 * the exec-target key, and a NULL target there (no `Py_mod_exec` running
 * on this thread) is a `SystemError`.
 *
 * A function becomes a `builtin_function_or_method` bound to the module,
 * exactly what `PyModule_AddFunctions` would have made of the same row.
 * Codegen publishes a redefined name only at its last definition, since
 * every definition shares that one's wrapper
 * (`crates/pycc_codegen/src/ext_publish.rs`); a second publish of one name
 * would still just replace the binding.
 * A class is bound from this exec's snapshot (`pycc_ext_exec_classes_key`)
 * of the `pycc_ext_type_object_<Class>` static that
 * `pycc_ext_register_method_types` filled before the body, not from the
 * static itself: the static is one per process, and a nested `Py_mod_exec`
 * of this artifact mid-body replaces it, so reading it here would bind the
 * inner exec's class on the outer module.
 *
 * Returns 0, or -1 with an exception set. Not `static`: LLVM-generated code
 * declares and calls it by this name (`EXT_PUBLISH_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_publish(const char *name)
{
    PyMethodDef *row = NULL;
    PyMethodDef *def;
    PyObject **class_slot = NULL;
    PyObject *module;
    PyObject *classes;
    PyObject *type;
    PyObject *modname;
    PyObject *func;
    int status;

    for (def = pycc_ext_methods; def->ml_name != NULL; def++) {
        if (strcmp(def->ml_name, name) == 0) {
            row = def;
            break;
        }
    }
    if (row == NULL) {
        class_slot = pycc_ext_publish_class_slot(name);
        if (class_slot == NULL) {
            return 0;
        }
    }
    module = pycc_ext_exec_target_key == NULL
        ? NULL
        : (PyObject *)PyThread_tss_get(pycc_ext_exec_target_key);
    if (module == NULL) {
        PyErr_Format(PyExc_SystemError, "pycc: cannot publish '%s' outside its module body", name);
        return -1;
    }
    if (row == NULL) {
        classes = (PyObject *)PyThread_tss_get(pycc_ext_exec_classes_key);
        type = classes == NULL ? NULL : PyDict_GetItemString(classes, name);
        if (type == NULL) {
            PyErr_Format(PyExc_SystemError, "pycc: class '%s' has no type object", name);
            return -1;
        }
        return PyModule_AddObjectRef(module, name, type);
    }
    modname = PyModule_GetNameObject(module);
    if (modname == NULL) {
        return -1;
    }
    func = PyCFunction_NewEx(row, module, modname);
    Py_DECREF(modname);
    if (func == NULL) {
        return -1;
    }
#ifdef PYCC_EXT_HAS_SIGNATURES
    /* #1194: a signature entry installs its Python wrapper instead of
     * binding itself (`src/ext_build/signatures.rs`). */
    status = pycc_ext_signature_publish(module, row->ml_name, func);
    if (status != 0) {
        Py_DECREF(func);
        return status < 0 ? -1 : 0;
    }
#endif
    status = PyModule_AddObjectRef(module, row->ml_name, func);
    Py_DECREF(func);
    return status;
}

/*
 * #1199's post-body safety net: binds every published class the body's
 * `pycc_ext_publish` calls did not, so a completed import still exposes
 * every type object it did before #1199. Codegen publishes a class once the
 * last slot any of its type object's methods calls is stored, and every
 * published class has at least one such slot (a method it declares, the
 * `__init__` D-225 synthesizes for a base-less class that declares none,
 * or an inherited method a subclass's type object calls), so today this
 * binds nothing. It exists so that a publication codegen ever misses
 * becomes late visibility rather than a class the host can never reach.
 * A name already in the module dict is left alone: either its publication
 * ran, or the body rebound the name itself. Runs before the PEP 562 hooks
 * are added, so the dict lookup sees the module's own bindings only. Binds
 * from `classes`, this exec's snapshot, for the reason `pycc_ext_publish`
 * does.
 */
static int pycc_ext_publish_unbound_classes(PyObject *module, PyObject *classes)
{
    PyObject *dict = PyModule_GetDict(module);
    const char *const *name;

    for (name = pycc_ext_publish_class_names; *name != NULL; name++) {
        PyObject *type = PyDict_GetItemString(classes, *name);
        if (type == NULL || PyDict_GetItemString(dict, *name) != NULL) {
            continue;
        }
        if (PyModule_AddObjectRef(module, *name, type) != 0) {
            return -1;
        }
    }
    return 0;
}

/*
 * #1199: the snapshot `pycc_ext_exec_classes_key` holds for one exec -- a
 * new dict holding a reference to every published class's type object as
 * `pycc_ext_register_method_types` just made it. NULL with an exception
 * set on failure.
 */
static PyObject *pycc_ext_snapshot_classes(void)
{
    PyObject *classes = PyDict_New();
    const char *const *name;

    if (classes == NULL) {
        return NULL;
    }
    for (name = pycc_ext_publish_class_names; *name != NULL; name++) {
        PyObject *type = *pycc_ext_publish_class_slot(*name);
        if (type != NULL && PyDict_SetItemString(classes, *name, type) != 0) {
            Py_DECREF(classes);
            return NULL;
        }
    }
    return classes;
}

/*
 * PEP 489 multi-phase initialization. The module body cannot run in
 * `PyInit_`: at that point no module object exists, so there is nothing to
 * execute against and no way to report a failure. It runs here, in the
 * `Py_mod_exec` slot CPython calls after creating the module object, whose
 * failure convention is -1 with the exception set.
 *
 * Running it at all is not optional. `pycc` binds each `def` by *storing*
 * its address into a module-level function-pointer slot at the def's own
 * source position, and (#1199) publishes the export only right after that
 * store, through `pycc_ext_publish` below: before the body has reached a
 * definition, its name is simply not an attribute of the module.
 */
static int pycc_ext_exec_module(PyObject *module)
{
    void *saved_target;
    void *saved_classes;
    PyObject *classes;
    int status;
    long long exec_status;

    /* Native stdin/stdout share CPython's `sys` streams from here on. */
    pycc_rt_io_install(pycc_ext_io_write, pycc_ext_io_read, pycc_ext_io_flush);
    /* An instance's `o` slots are counted with CPython's own counts. */
    pycc_rt_object_install(pycc_ext_obj_retain_hook, pycc_ext_obj_release_hook);
    /* An `o` element of a native tuple renders with CPython's `repr`. */
    pycc_rt_object_repr_install(pycc_ext_obj_repr_hook);

    /*
     * The synthesized user exception classes are created and published as
     * module attributes before the module body runs, so a body that raises
     * one already finds it registered. This is the only place the module
     * object is reachable (`m_size = 0`, so there is no module state), and
     * a failure here fails the import loudly rather than importing a module
     * whose `except m.MyError:` silently never matches.
     */
    /*
     * Part 2b of #1142 (#1164): the buffer exporter type, before anything
     * that could call into compiled code, because a compiled function that
     * returns a buffer needs it the first time it is called. Created at most
     * once: a module reload runs this slot again against the same file
     * static, and creating a second type object would leave `memoryview`s
     * from before the reload exporting through a type nothing else holds.
     *
     * Not published as a module attribute -- it is not part of the module's
     * Python-visible surface, and `Py_TPFLAGS_DISALLOW_INSTANTIATION` means
     * a host that reached it anyway could do nothing with it.
     */
    if (pycc_ext_buffer_type == NULL) {
        pycc_ext_buffer_type = PyType_FromSpec(&pycc_ext_buffer_type_spec);
        if (pycc_ext_buffer_type == NULL) {
            return -1;
        }
    }
    if (pycc_ext_register_exception_classes(module) != 0) {
        return -1;
    }
    /*
     * One type object per class that exports a method or is constructible,
     * created here but not yet added to the module (#1199): it is kept in
     * its `pycc_ext_type_object_<Class>` static, and `pycc_ext_publish`
     * binds it under the bare class name once the body has run the class
     * statement (this exec snapshots it just below, and `pycc_ext_publish`
     * binds from the snapshot). PEP 489 puts the module in `sys.modules` before this slot
     * runs, so a foreign module the body imports can import this one back
     * and read its attributes mid-body; a class bound here would be visible
     * there before CPython would bind it. Created before the body anyway,
     * because a compiled function the body calls may box an instance of the
     * class into its carrier type, which must already exist. A class
     * carrying an exception tag never gets a type object at all: the
     * exception classes above already own that attribute. Same
     * -1-with-exception-set convention as everything else in this slot.
     */
    if (pycc_ext_register_method_types(module) != 0) {
        return -1;
    }
    /*
     * #1366: the exec-target key's thread-specific storage, created once
     * per process. A re-import after `del sys.modules[...]` runs this slot
     * again, and must not replace a key that live wrappers on other threads
     * still read.
     */
    if (pycc_ext_exec_target_key == NULL) {
        Py_tss_t *key = PyThread_tss_alloc();
        if (key == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        if (PyThread_tss_create(key) != 0) {
            PyThread_tss_free(key);
            PyErr_SetString(PyExc_RuntimeError, "pycc: cannot create the exec-target key");
            return -1;
        }
        pycc_ext_exec_target_key = key;
    }
    /* #1199: the exec-classes key, on the same create-once rule. */
    if (pycc_ext_exec_classes_key == NULL) {
        Py_tss_t *key = PyThread_tss_alloc();
        if (key == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        if (PyThread_tss_create(key) != 0) {
            PyThread_tss_free(key);
            PyErr_SetString(PyExc_RuntimeError, "pycc: cannot create the exec-classes key");
            return -1;
        }
        pycc_ext_exec_classes_key = key;
    }
    /*
     * #1199: this exec's classes, taken now, before the body can run a
     * nested exec that replaces the type statics; installed and restored
     * on both exits alongside the #1366 target below, and released only
     * after the safety net has read them.
     */
    classes = pycc_ext_snapshot_classes();
    if (classes == NULL) {
        return -1;
    }
    /*
     * #1366: this module is the one a relative import in its body resolves
     * against, for exactly the duration of the body. The previous value is
     * restored on both exits, so an exec running beneath another one on
     * this thread (see `pycc_ext_exec_target_key`) hands the outer one's
     * target back.
     */
    saved_target = PyThread_tss_get(pycc_ext_exec_target_key);
    saved_classes = PyThread_tss_get(pycc_ext_exec_classes_key);
    if (PyThread_tss_set(pycc_ext_exec_target_key, module) != 0
        || PyThread_tss_set(pycc_ext_exec_classes_key, classes) != 0) {
        (void)PyThread_tss_set(pycc_ext_exec_target_key, saved_target);
        Py_DECREF(classes);
        PyErr_SetString(PyExc_RuntimeError, "pycc: cannot record the executing module");
        return -1;
    }
    exec_status = pycc_ext_module_exec();
    (void)PyThread_tss_set(pycc_ext_exec_target_key, saved_target);
    (void)PyThread_tss_set(pycc_ext_exec_classes_key, saved_classes);
    if (exec_status != 0) {
        Py_DECREF(classes);
        /*
         * The generic `ImportError` is a last resort, not the default. A
         * failing body reports through one of two channels: `pycc_rt`'s
         * thread-local pending state, which `pycc_ext_raise_pending` turns
         * into a live CPython exception, or -- for a body that called into
         * CPython directly -- an exception CPython already set, with no
         * pycc pending state at all. The first channel has a third case
         * since #1293: a pending exception the failed-import bridge made
         * from a CPython one, which `pycc_ext_raise_pending` re-raises as
         * that original object.
         * `pycc_ext_raise_pending` returns 0 for the second channel, so
         * raising unconditionally here would replace
         * the real exception (a `ModuleNotFoundError` from a failed host
         * import, say) with a message that names neither the cause nor the
         * culprit. Check `PyErr_Occurred` before overwriting, and keep
         * `pycc_ext_raise_pending` as the left operand: it is the call that
         * sets the exception, so short-circuiting must not skip it.
         */
        if (!pycc_ext_raise_pending() && !PyErr_Occurred()) {
            PyErr_SetString(PyExc_ImportError, "pycc module body failed");
        }
        return -1;
    }
    status = pycc_ext_publish_unbound_classes(module, classes);
    Py_DECREF(classes);
    if (status != 0) {
        return -1;
    }
    /*
     * #1467: the module's PEP 562 `__getattr__` / `__dir__`, published
     * only now, not at their `def` the way `pycc_ext_publish` binds every
     * other function (#1199). Once in the dict, `__getattr__` answers every
     * missing attribute -- including the ones importlib and a re-entrant
     * host probe while the body is still running -- so binding it at its
     * `def` would let it answer for names the rest of the body has yet to
     * bind. A re-import after `del sys.modules[...]` runs this again and
     * rebinds them.
     */
    if (PyModule_AddFunctions(module, pycc_ext_module_hooks) != 0) {
        return -1;
    }
    return 0;
}

static PyModuleDef_Slot pycc_ext_slots[] = {
    {Py_mod_exec, (void *)pycc_ext_exec_module},
    /*
     * `pycc_rt`'s refcounts are non-atomic `Cell<u32>` and its exception
     * state is thread-local, both of which are safe only under one GIL in
     * one interpreter. Refuse a subinterpreter explicitly rather than
     * corrupting state in one.
     */
    {Py_mod_multiple_interpreters, Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED},
    {0, NULL},
};

/* The entry module's docstring, when it has one (`with_module_doc` in
 * `src/ext_build.rs`), else no `__doc__` text. */
#ifndef PYCC_EXT_MODULE_DOC
#define PYCC_EXT_MODULE_DOC NULL
#endif

static struct PyModuleDef pycc_ext_moduledef = {
    PyModuleDef_HEAD_INIT,
    PYCC_EXT_MODULE_NAME_STR,
    PYCC_EXT_MODULE_DOC,
    0,
    /*
     * #1199: no `m_methods`. CPython would add every row before the body
     * runs; `pycc_ext_publish` adds each one as its `def` executes instead.
     */
    NULL,
    pycc_ext_slots,
    NULL,
    NULL,
    NULL,
};

#define PYCC_EXT_CONCAT_(a, b) a##b
#define PYCC_EXT_CONCAT(a, b) PYCC_EXT_CONCAT_(a, b)

PyMODINIT_FUNC PYCC_EXT_CONCAT(PyInit_, PYCC_EXT_MODULE_NAME)(void)
{
    /*
     * The free-threaded guard, and it has to be here -- before
     * `PyModuleDef_Init` -- because it must refuse the module outright.
     *
     * A free-threaded host's `EXTENSION_SUFFIXES` does include `.abi3.so`,
     * so its finder locates this artifact happily; with no guard the failure
     * is a bare `SystemError: init function of <mod> returned uninitialized
     * object`. `sys._is_gil_enabled()` is not a usable detector here: it
     * returned True on a free-threaded 3.14 at `PyInit_` time. The version
     * string is, verified across 3.13 GIL, 3.14 GIL and 3.14t.
     */
    if (strstr(Py_GetVersion(), "free-threading build") != NULL) {
        PyErr_SetString(PyExc_ImportError,
                        PYCC_EXT_MODULE_NAME_STR " was built for a GIL-enabled CPython "
                        "(stable ABI); this interpreter is a free-threaded build, which is "
                        "not supported");
        return NULL;
    }
    return PyModuleDef_Init(&pycc_ext_moduledef);
}
