/*
 * Module globals rebound from outside (v0.8 accept 3): a non-strict
 * module's compiled function reads a global from its own slot, which the
 * module's `__dict__` mirrors; when other code rebinds the attribute
 * (`mod.VERSION = ...`, `mock.patch("mod.deepcopy", ...)`), the function
 * must see the new value, as a Python function reading the dict would.
 *
 * Each read first compares the dict's version tag (CPython 3.13's
 * `PyDictObject.ma_version_tag`, which every change to the dict renews)
 * with the one the read last saw (codegen's per-global cache). Only when
 * they differ does `pycc_ext_global_sync` look the name up: if the dict
 * holds another object than the one last seen, the slot takes it (an
 * object as it is, a `str`, `int`, `float` or `bool` converted to the
 * slot's native type). A module's `pycc_ext_ns_dict` is NULL -- and every read takes
 * its slot as it is -- on another CPython version, whose dict layout the
 * shim does not know, and in a native program.
 */

/* Each module's `pycc_ext_ns_dict` -- its `__dict__` (borrowed: the module
 * outlives every call of its functions), set before the body runs on CPython
 * 3.13 -- is defined by the module's own unit (`module_init.c`), whose
 * `pycc_ext_global_sync` passes it to `pycc_ext_global_sync_in`. */

/* `PyDictObject.ma_version_tag` in CPython 3.13. */
#define PYCC_DICT_VERSION_TAG 24

/* A global's cache (`pycc_codegen`'s `pycc_gsync` globals): the dict
 * version the read last saw, and -- for a slot of a native type -- the
 * object it last converted (owned). An object slot is compared with the
 * object it holds instead, so the cache keeps no object alive that the
 * module no longer binds. */
typedef struct PyccGlobalCache {
    uint64_t version;
    PyObject *seen;
} PyccGlobalCache;

/* The slot kinds codegen passes. */
#define PYCC_GLOBAL_OBJECT 0
#define PYCC_GLOBAL_STR 1
#define PYCC_GLOBAL_INT 2
#define PYCC_GLOBAL_FLOAT 3
#define PYCC_GLOBAL_BOOL 4

static void pycc_ext_global_sync_in(PyObject *dict, const char *name, void *slot, int kind,
                                    PyccGlobalCache *cache, char *initialized)
{
    PyObject *saved;
    PyObject *value;
    int stored = 0;

    if (dict == NULL) {
        return;
    }
    cache->version = *(uint64_t *)((char *)dict + PYCC_DICT_VERSION_TAG);
    saved = PyErr_GetRaisedException();
    value = PyDict_GetItemString(dict, name);
    if (value == NULL
        || (kind == PYCC_GLOBAL_OBJECT ? initialized != NULL && *initialized
                                             && value == *(PyObject **)slot
                                       : value == cache->seen)) {
        PyErr_Clear();
        PyErr_SetRaisedException(saved);
        return;
    }
    switch (kind) {
    case PYCC_GLOBAL_OBJECT: {
        PyObject *old = *(PyObject **)slot;

        Py_INCREF(value);
        *(PyObject **)slot = value;
        if (initialized != NULL && *initialized) {
            Py_XDECREF(old);
        }
        stored = 1;
        break;
    }
    case PYCC_GLOBAL_STR: {
        void *text = NULL;

        if (pycc_ext_unpack_str(value, "global", 0, &text) == 0) {
            if (initialized != NULL && *initialized) {
                pycc_rt_str_decref(*(void **)slot);
            }
            *(void **)slot = text;
            stored = 1;
        }
        break;
    }
    case PYCC_GLOBAL_INT: {
        long long word = 0;

        if (pycc_ext_unpack_int(value, "global", 0, &word) == 0) {
            if (initialized != NULL && *initialized) {
                pycc_rt_bigint_release(*(long long *)slot);
            }
            *(long long *)slot = word;
            stored = 1;
        }
        break;
    }
    case PYCC_GLOBAL_FLOAT: {
        double number = 0.0;

        if (pycc_ext_unpack_float(value, "global", 0, &number) == 0) {
            *(double *)slot = number;
            stored = 1;
        }
        break;
    }
    case PYCC_GLOBAL_BOOL: {
        char flag = 0;

        if (pycc_ext_unpack_bool(value, "global", 0, &flag) == 0) {
            *(char *)slot = flag;
            stored = 1;
        }
        break;
    }
    default:
        break;
    }
    if (stored && initialized != NULL) {
        *initialized = 1;
    }
    if (kind != PYCC_GLOBAL_OBJECT) {
        PyObject *old_seen = cache->seen;

        Py_INCREF(value);
        cache->seen = value;
        Py_XDECREF(old_seen);
    }
    PyErr_Clear();
    PyErr_SetRaisedException(saved);
}
