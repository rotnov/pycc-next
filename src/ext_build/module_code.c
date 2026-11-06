/*
 * 1 when the embedded code objects are this interpreter's
 * (`pycc_ext_marshal_loads`). Decided once per process; `marshal.loads`
 * (borrowed from the runtime unit, which keeps it) is kept with it.
 */
static int pycc_ext_chunk_usable = -1;
static PyObject *pycc_ext_chunk_loads = NULL;
/* The tuple of code objects unmarshalled from `pycc_ext_chunk_table_data`
 * (with its `frozenset` constants rebuilt), from the hook's first call
 * until the body has run. */
static PyObject *pycc_ext_chunk_table = NULL;

static int pycc_ext_chunks_usable(void)
{
    if (pycc_ext_chunk_usable >= 0) {
        return pycc_ext_chunk_usable;
    }
    pycc_ext_chunk_loads = pycc_ext_chunk_magic == 0 ? NULL : pycc_ext_marshal_loads(pycc_ext_chunk_magic);
    pycc_ext_chunk_usable = pycc_ext_chunk_loads != NULL;
    return pycc_ext_chunk_usable;
}

/* `compile()` of chunk `chunk`'s embedded source, as the build compiled it. */
static PyObject *pycc_ext_chunk_compile(const struct pycc_ext_chunk *chunk)
{
    PyObject *compile;
    PyObject *source;
    PyObject *result;

    compile = PyDict_GetItemString(PyEval_GetBuiltins(), "compile");
    if (compile == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: builtins.compile is missing");
        return NULL;
    }
    source = PyUnicode_DecodeUTF8(pycc_ext_chunk_text + chunk->source, chunk->source_len, NULL);
    if (source == NULL) {
        return NULL;
    }
    result = PyObject_CallFunction(compile, "Ossii", source, pycc_ext_chunk_text + chunk->filename,
                                   "exec", 0, 1);
    Py_DECREF(source);
    return result;
}

/* `code` with `co_consts[path[at]]` (recursively, then `co_consts[index]`
 * at the end of `path`) replaced by `frozenset(elements)`: a new
 * reference, or NULL with an exception set. */
static PyObject *pycc_ext_chunk_refold(PyObject *code, PyObject *path, Py_ssize_t at,
                                       Py_ssize_t index, PyObject *elements)
{
    PyObject *consts = PyObject_GetAttrString(code, "co_consts");
    PyObject *list = consts == NULL || !PyTuple_Check(consts) ? NULL : PySequence_List(consts);
    PyObject *slot = NULL;
    PyObject *replaced = NULL;
    PyObject *result = NULL;
    Py_ssize_t target = index;

    if (list == NULL) {
        goto done;
    }
    if (at < PyTuple_Size(path)) {
        target = PyLong_AsSsize_t(PyTuple_GetItem(path, at));
    }
    if (target < 0 || target >= PyList_Size(list)) {
        if (!PyErr_Occurred()) {
            PyErr_SetString(PyExc_SystemError, "pycc: a frozenset fix-up names no constant");
        }
        goto done;
    }
    slot = at < PyTuple_Size(path)
               ? pycc_ext_chunk_refold(PyList_GetItem(list, target), path, at + 1, index, elements)
               : PyFrozenSet_New(elements);
    if (slot == NULL || PyList_SetItem(list, target, slot) < 0) {
        goto done;
    }
    replaced = PyList_AsTuple(list);
    if (replaced != NULL) {
        PyObject *replace = PyObject_GetAttrString(code, "replace");
        PyObject *args = replace == NULL ? NULL : PyTuple_New(0);
        PyObject *kwargs = args == NULL ? NULL : Py_BuildValue("{sO}", "co_consts", replaced);

        result = kwargs == NULL ? NULL : PyObject_Call(replace, args, kwargs);
        Py_XDECREF(kwargs);
        Py_XDECREF(args);
        Py_XDECREF(replace);
    }
done:
    Py_XDECREF(replaced);
    Py_XDECREF(list);
    Py_XDECREF(consts);
    return result;
}

/* The unmarshalled `(codes, fix-ups)` pair as the tuple of code objects,
 * each `frozenset` constant the build recorded rebuilt from its source
 * display (`module_code.rs`): a new reference, or NULL. */
static PyObject *pycc_ext_chunk_refolded(PyObject *pair)
{
    PyObject *codes;
    PyObject *fixups;
    Py_ssize_t i;

    if (!PyTuple_Check(pair) || PyTuple_Size(pair) != 2) {
        return NULL;
    }
    codes = PyTuple_GetItem(pair, 0);
    fixups = PyTuple_GetItem(pair, 1);
    if (!PyTuple_Check(codes) || PyTuple_Size(codes) != pycc_ext_chunk_count || !PyTuple_Check(fixups)) {
        return NULL;
    }
    if (PyTuple_Size(fixups) == 0) {
        return Py_NewRef(codes);
    }
    codes = PySequence_List(codes);
    for (i = 0; codes != NULL && i < PyTuple_Size(fixups); i++) {
        PyObject *fixup = PyTuple_GetItem(fixups, i);
        Py_ssize_t chunk = -1;
        Py_ssize_t index = -1;
        PyObject *path = NULL;
        PyObject *elements = NULL;
        PyObject *code;

        if (!PyArg_ParseTuple(fixup, "nO!nO!", &chunk, &PyTuple_Type, &path, &index,
                              &PyTuple_Type, &elements)
            || chunk < 0 || chunk >= pycc_ext_chunk_count) {
            Py_CLEAR(codes);
            break;
        }
        code = pycc_ext_chunk_refold(PyList_GetItem(codes, chunk), path, 0, index, elements);
        if (code == NULL || PyList_SetItem(codes, chunk, code) < 0) {
            Py_CLEAR(codes);
        }
    }
    if (codes != NULL) {
        PyObject *tuple = PyList_AsTuple(codes);

        Py_DECREF(codes);
        codes = tuple;
    }
    return codes;
}

/* Chunk `index`'s code object from the embedded tuple (unmarshalled on the
 * first call), a new reference; NULL with no exception set when it cannot
 * be had that way. */
static PyObject *pycc_ext_chunk_code(Py_ssize_t index)
{
    PyObject *code;

    if (!pycc_ext_chunks[index].precompiled || !pycc_ext_chunks_usable()) {
        return NULL;
    }
    if (pycc_ext_chunk_table == NULL) {
        PyObject *view = PyMemoryView_FromMemory((char *)pycc_ext_chunk_table_data,
                                                 pycc_ext_chunk_table_len, PyBUF_READ);
        PyObject *pair = NULL;

        if (view != NULL) {
            pair = PyObject_CallFunctionObjArgs(pycc_ext_chunk_loads, view, NULL);
            Py_DECREF(view);
        }
        pycc_ext_chunk_table = pair == NULL ? NULL : pycc_ext_chunk_refolded(pair);
        Py_XDECREF(pair);
        if (pycc_ext_chunk_table == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    /* The build's tuple holds a code object, or None for a chunk it could
     * not compile (then compiled here, failing as `compile()` does). */
    code = PyTuple_GetItem(pycc_ext_chunk_table, index);
    if (code == NULL || code == Py_None) {
        PyErr_Clear();
        return NULL;
    }
    return Py_NewRef(code);
}

static PyObject *pycc_ext_chunk_builtins_name = NULL;

/* `exec(code, ns)`: `__builtins__` bound first (as `exec` binds it), then
 * the audit event; 0, or -1 with an exception set. */
static int pycc_ext_chunk_exec(PyObject *code, PyObject *ns)
{
    PyObject *done;
    int has;

    if (pycc_ext_chunk_builtins_name == NULL) {
        pycc_ext_chunk_builtins_name = PyUnicode_InternFromString("__builtins__");
        if (pycc_ext_chunk_builtins_name == NULL) {
            return -1;
        }
    }
    has = PyDict_Contains(ns, pycc_ext_chunk_builtins_name);
    if (has < 0 || (has == 0 && PyDict_SetItem(ns, pycc_ext_chunk_builtins_name, PyEval_GetBuiltins()) != 0)) {
        return -1;
    }
    if (PySys_Audit("exec", "O", code) < 0) {
        return -1;
    }
    done = PyEval_EvalCode(code, ns, ns);
    if (done == NULL) {
        return -1;
    }
    Py_DECREF(done);
    return 0;
}

/* Runs chunk `index` in `ns` (as `exec(<its code>, ns)` when `as_exec`:
 * the audit event and `__builtins__`); 0, or -1 with an exception set. */
static int pycc_ext_chunk_run_in(Py_ssize_t index, PyObject *ns, int as_exec)
{
    PyObject *code;
    int status;

    if (index < 0 || index >= pycc_ext_chunk_count) {
        PyErr_SetString(PyExc_SystemError, "pycc: no such chunk");
        return -1;
    }
    code = pycc_ext_chunk_code(index);
    if (code == NULL) {
        code = pycc_ext_chunk_compile(&pycc_ext_chunks[index]);
        if (code == NULL) {
            return -1;
        }
    }
    if (as_exec) {
        status = pycc_ext_chunk_exec(code, ns);
    } else {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        status = done == NULL ? -1 : 0;
        Py_XDECREF(done);
    }
    Py_DECREF(code);
    return status;
}

/* `__pycc_run__(k)`: runs chunk `k` in the namespace (`self`), as
 * `exec(<its code>, ns)` does. */
static PyObject *pycc_ext_run(PyObject *self, PyObject *arg)
{
    Py_ssize_t index = PyLong_AsSsize_t(arg);

    if (index == -1 && PyErr_Occurred()) {
        return NULL;
    }
    if (pycc_ext_chunk_run_in(index, self, 1) != 0) {
        return NULL;
    }
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_run_def = {"__pycc_run__", pycc_ext_run, METH_O, NULL};

/* Binds the hook in `module`'s namespace for the body; 0, or -1 with an
 * exception set. */
static int pycc_ext_code_install(PyObject *module)
{
    PyObject *dict = PyModule_GetDict(module);
    PyObject *hook = PyCFunction_NewEx(&pycc_ext_run_def, dict, NULL);
    int status;

    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(dict, "__pycc_run__", hook);
    Py_DECREF(hook);
    return status;
}

/* Removes the hook once the body has run, keeping any exception the body
 * raised. */
static void pycc_ext_code_remove(PyObject *module)
{
    PyObject *raised = PyErr_GetRaisedException();
    PyObject *dict = PyModule_GetDict(module);

    if (PyDict_DelItemString(dict, "__pycc_run__") != 0) {
        PyErr_Clear();
    }
    /* No chunk is read after the body: the code objects it made live on in
     * what it defined. */
    Py_CLEAR(pycc_ext_chunk_table);
    PyErr_SetRaisedException(raised);
}
