/*
 * 1 when the embedded code objects are this interpreter's
 * (`pycc_ext_marshal_loads`). Decided once per process; `marshal.loads`
 * (borrowed from the runtime unit, which keeps it) is kept with it.
 */
static int pycc_ext_chunk_usable = -1;
static PyObject *pycc_ext_chunk_loads = NULL;
/* The unmarshalled table (`pycc_ext_chunk_table_data`), from the hook's
 * first call until the body has run. */
static PyObject *pycc_ext_chunk_table = NULL;
/* `fix(code, orders)` (`ORDERS_FIX` in `src/ext_build/precompiled.rs`),
 * made once from the table's last entry; NULL when unavailable. */
static PyObject *pycc_ext_chunk_fix = NULL;

/* `pycc_ext_chunk_fix`, made by running the table's last entry; NULL (no
 * exception left set) when it cannot be. */
static PyObject *pycc_ext_chunk_fixer(void)
{
    PyObject *code, *ns;

    if (pycc_ext_chunk_fix != NULL) {
        return pycc_ext_chunk_fix;
    }
    code = PyTuple_GetItem(pycc_ext_chunk_table, pycc_ext_chunk_count);
    ns = code == NULL ? NULL : PyDict_New();
    if (ns != NULL && PyDict_SetItemString(ns, "__builtins__", PyEval_GetBuiltins()) == 0) {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        Py_XDECREF(done);
        if (done != NULL) {
            pycc_ext_chunk_fix = PyDict_GetItemString(ns, "fix");
            Py_XINCREF(pycc_ext_chunk_fix);
        }
    }
    Py_XDECREF(ns);
    PyErr_Clear();
    return pycc_ext_chunk_fix;
}

static int pycc_ext_chunks_usable(void)
{
    if (pycc_ext_chunk_usable >= 0) {
        return pycc_ext_chunk_usable;
    }
    pycc_ext_chunk_loads = pycc_ext_chunk_magic == 0 ? NULL : pycc_ext_marshal_loads(pycc_ext_chunk_magic);
    pycc_ext_chunk_usable = pycc_ext_chunk_loads != NULL;
    return pycc_ext_chunk_usable;
}

/* `compile()` of chunk `chunk`'s embedded source, as the rewrite did. */
static PyObject *pycc_ext_chunk_compile(const struct pycc_ext_chunk *chunk)
{
    PyObject *compile;
    PyObject *source;
    PyObject *flags;
    PyObject *result;

    compile = PyDict_GetItemString(PyEval_GetBuiltins(), "compile");
    if (compile == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: builtins.compile is missing");
        return NULL;
    }
    if (chunk->future_annotations) {
        PyObject *future = PyImport_ImportModule("__future__");
        PyObject *feature;
        if (future == NULL) {
            return NULL;
        }
        feature = PyObject_GetAttrString(future, "annotations");
        Py_DECREF(future);
        if (feature == NULL) {
            return NULL;
        }
        flags = PyObject_GetAttrString(feature, "compiler_flag");
        Py_DECREF(feature);
    } else {
        flags = PyLong_FromLong(0);
    }
    if (flags == NULL) {
        return NULL;
    }
    source = PyUnicode_DecodeUTF8(chunk->source, chunk->source_len, NULL);
    if (source == NULL) {
        Py_DECREF(flags);
        return NULL;
    }
    result = PyObject_CallFunction(compile, "OssOi", source, chunk->filename, "exec", flags, 1);
    Py_DECREF(source);
    Py_DECREF(flags);
    return result;
}

/* The table's entry for chunk `index` (the table unmarshalled on the first
 * call); NULL with no exception set when it cannot be had that way. */
static PyObject *pycc_ext_chunk_entry(Py_ssize_t index)
{
    PyObject *entry;

    if (pycc_ext_chunk_table == NULL) {
        PyObject *view = PyMemoryView_FromMemory((char *)pycc_ext_chunk_table_data,
                                                 pycc_ext_chunk_table_len, PyBUF_READ);

        if (view != NULL) {
            pycc_ext_chunk_table = PyObject_CallFunctionObjArgs(pycc_ext_chunk_loads, view, NULL);
            Py_DECREF(view);
        }
        if (pycc_ext_chunk_table != NULL
            && (!PyTuple_Check(pycc_ext_chunk_table)
                || PyTuple_Size(pycc_ext_chunk_table) != pycc_ext_chunk_count + 1)) {
            Py_CLEAR(pycc_ext_chunk_table);
        }
        if (pycc_ext_chunk_table == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    entry = PyTuple_GetItem(pycc_ext_chunk_table, index);
    if (entry == NULL || !PyTuple_Check(entry)
        || (PyTuple_Size(entry) != 2 && PyTuple_Size(entry) != 1)) {
        PyErr_Clear();
        return NULL;
    }
    return entry;
}

/* `code` with its `frozenset` constants rebuilt in source order (`orders`,
 * None when it has none); NULL with no exception set when it cannot be. */
static PyObject *pycc_ext_chunk_reordered(PyObject *code, PyObject *orders)
{
    PyObject *fix;

    if (orders == Py_None) {
        return Py_NewRef(code);
    }
    fix = pycc_ext_chunk_fixer();
    code = fix == NULL ? NULL : PyObject_CallFunctionObjArgs(fix, code, orders, NULL);
    if (code == NULL) {
        PyErr_Clear();
    }
    return code;
}

/* The code object of a `(code, orders)` entry; NULL with no exception set
 * when it cannot be had. */
static PyObject *pycc_ext_chunk_entry_code(PyObject *entry)
{
    return pycc_ext_chunk_reordered(PyTuple_GetItem(entry, 0), PyTuple_GetItem(entry, 1));
}

/* `types.FunctionType`, kept from the first `def` entry run. */
static PyObject *pycc_ext_chunk_function_type = NULL;

/*
 * Runs a `def` entry in `ns` as the chunk's own code would: the chunk is
 * only `def` statements whose defaults, keyword defaults and annotations
 * are constants (no decorator, no closure), each perhaps followed by a
 * `__pycc_forward__(...)` line (`precompile.py`'s `defs`), so it can
 * raise nothing of its own and needs no frame. Each `(code, defaults,
 * kwdefaults, annotations, name, forward, orders)` is a function made as
 * `MAKE_FUNCTION` makes it -- name, qualified name, docstring, `__module__`
 * and builtins from the code (its set constants rebuilt from `orders`) and
 * `ns` -- with the annotations as the dict CPython makes from the same
 * tuple on first read, stored as `name`, and then the forwarders patched.
 * A `(name, code, order, orders)` is a nested statement's scaffold chunk
 * (`src/fallback/nested.rs`), whose `__pycc_fallback_nestspec` call the
 * build made: `name` is bound to its answer, `(code, order)`, and the
 * factory it was asked of is never made. 0; -1 with an exception set; 1 (nothing defined) when a code cannot be
 * had this way, and the chunk is to be compiled.
 */
static int pycc_ext_chunk_define(PyObject *specs, PyObject *ns)
{
    Py_ssize_t i, n = PyTuple_Size(specs);
    PyObject *codes = PyTuple_New(n);
    int status = -1;

    if (codes == NULL) {
        return -1;
    }
    /* Every function's code first, its set constants in source order: when
     * one cannot be had, nothing is defined and the chunk is compiled. */
    for (i = 0; i < n; i++) {
        PyObject *spec = PyTuple_GetItem(specs, i);
        Py_ssize_t size = spec == NULL || !PyTuple_Check(spec) ? 0 : PyTuple_Size(spec);
        PyObject *code = size == 7   ? pycc_ext_chunk_reordered(PyTuple_GetItem(spec, 0), PyTuple_GetItem(spec, 6))
                         : size == 4 ? pycc_ext_chunk_reordered(PyTuple_GetItem(spec, 1), PyTuple_GetItem(spec, 3))
                                     : NULL;

        if (code == NULL || PyTuple_SetItem(codes, i, code) != 0) {
            PyErr_Clear();
            Py_DECREF(codes);
            return 1;
        }
    }
    if (pycc_ext_chunk_function_type == NULL) {
        PyObject *types = PyImport_ImportModule("types");

        if (types == NULL) {
            goto done;
        }
        pycc_ext_chunk_function_type = PyObject_GetAttrString(types, "FunctionType");
        Py_DECREF(types);
        if (pycc_ext_chunk_function_type == NULL) {
            goto done;
        }
    }
    for (i = 0; i < n; i++) {
        PyObject *spec = PyTuple_GetItem(specs, i);
        PyObject *annotations, *forward, *kwdefaults, *func;
        int stored;

        if (PyTuple_Size(spec) == 4) {
            /* A nested statement's scaffold (`src/fallback/nested.rs`):
             * what `__pycc_fallback_nestspec` answers, its code and the
             * order of its cells. */
            PyObject *value = PyTuple_Pack(2, PyTuple_GetItem(codes, i), PyTuple_GetItem(spec, 2));

            stored = value == NULL ? -1 : PyDict_SetItem(ns, PyTuple_GetItem(spec, 0), value);
            Py_XDECREF(value);
            if (stored != 0) {
                goto done;
            }
            continue;
        }
        annotations = PyTuple_GetItem(spec, 3);
        forward = PyTuple_GetItem(spec, 5);
        /* A fresh keyword-defaults dict per function, as `BUILD_CONST_KEY_MAP`
         * makes one each time the statement runs (the defaults tuple is a
         * constant, shared as `LOAD_CONST` shares it). */
        kwdefaults = PyTuple_GetItem(spec, 2);
        kwdefaults = kwdefaults == Py_None ? Py_NewRef(kwdefaults) : PyDict_Copy(kwdefaults);
        if (kwdefaults == NULL) {
            goto done;
        }
        func = PyObject_CallFunctionObjArgs(pycc_ext_chunk_function_type, PyTuple_GetItem(codes, i), ns,
                                            Py_None, PyTuple_GetItem(spec, 1), Py_None, kwdefaults, NULL);
        Py_DECREF(kwdefaults);
        if (func == NULL) {
            goto done;
        }
        if (annotations != Py_None) {
            PyObject *dict = PyDict_New();
            Py_ssize_t at, size = PyTuple_Size(annotations);

            stored = dict == NULL ? -1 : 0;
            for (at = 0; stored == 0 && at + 1 < size; at += 2) {
                stored = PyDict_SetItem(dict, PyTuple_GetItem(annotations, at),
                                        PyTuple_GetItem(annotations, at + 1));
            }
            if (stored == 0) {
                stored = PyObject_SetAttrString(func, "__annotations__", dict);
            }
            Py_XDECREF(dict);
            if (stored != 0) {
                Py_DECREF(func);
                goto done;
            }
        }
        stored = PyDict_SetItem(ns, PyTuple_GetItem(spec, 4), func);
        Py_DECREF(func);
        if (stored != 0) {
            goto done;
        }
        if (forward != Py_None) {
            PyObject *hook = PyDict_GetItemString(ns, "__pycc_forward__");
            PyObject *result;

            if (hook == NULL) {
                PyErr_SetString(PyExc_NameError, "name '__pycc_forward__' is not defined");
                goto done;
            }
            result = PyObject_Call(hook, forward, NULL);
            if (result == NULL) {
                goto done;
            }
            Py_DECREF(result);
        }
    }
    status = 0;
done:
    Py_DECREF(codes);
    return status;
}

/* `"__builtins__"`, interned once. */
static PyObject *pycc_ext_chunk_builtins_name = NULL;

static PyObject *pycc_ext_chunk_builtins_key(void)
{
    if (pycc_ext_chunk_builtins_name == NULL) {
        pycc_ext_chunk_builtins_name = PyUnicode_InternFromString("__builtins__");
    }
    return pycc_ext_chunk_builtins_name;
}

/* Binds `__builtins__` in `ns` when it is not, as `exec` does; 0, or -1
 * with an exception set. */
static int pycc_ext_chunk_bind_builtins(PyObject *ns)
{
    PyObject *key = pycc_ext_chunk_builtins_key();
    int has = key == NULL ? -1 : PyDict_Contains(ns, key);

    if (has == 0) {
        return PyDict_SetItem(ns, key, PyEval_GetBuiltins());
    }
    return has < 0 ? -1 : 0;
}

/* `exec(code, ns)`: `__builtins__` bound first, then the audit event. */
static int pycc_ext_chunk_exec(PyObject *code, PyObject *ns)
{
    PyObject *done;

    if (pycc_ext_chunk_bind_builtins(ns) != 0) {
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
    const struct pycc_ext_chunk *chunk;
    PyObject *code = NULL;
    int status;

    if (index < 0 || index >= pycc_ext_chunk_count) {
        PyErr_SetString(PyExc_SystemError, "pycc: no such pre-compiled chunk");
        return -1;
    }
    chunk = &pycc_ext_chunks[index];
    if (chunk->precompiled && pycc_ext_chunks_usable()) {
        /* Owned while it is used: Python code it runs (an audit hook, a
         * re-import of this module) may finish another body run, which
         * frees the table. */
        PyObject *entry = Py_XNewRef(pycc_ext_chunk_entry(index));

        if (entry != NULL && PyTuple_Size(entry) == 1) {
            status = as_exec && pycc_ext_chunk_bind_builtins(ns) != 0
                         ? -1
                         : pycc_ext_chunk_define(PyTuple_GetItem(entry, 0), ns);
            if (status <= 0) {
                Py_DECREF(entry);
                return status;
            }
            Py_CLEAR(entry);
        }
        code = entry == NULL ? NULL : pycc_ext_chunk_entry_code(entry);
        Py_XDECREF(entry);
        /* Unreadable after all: compile the source instead. */
    }
    if (code == NULL) {
        code = pycc_ext_chunk_compile(chunk);
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

