/*
 * Module globals rebound from outside (v0.8 accept 3): a non-strict
 * module's compiled function reads a global from its own slot, which the
 * module's `__dict__` mirrors; when other code rebinds the attribute
 * (`mod.VERSION = ...`, `mock.patch("mod.deepcopy", ...)`), the function
 * must see the new value, as a Python function reading the dict would.
 *
 * Each read first compares the dict's version (codegen loads it through
 * the module's `pycc_ext_ns_version`) with the one the read last saw
 * (codegen's per-global cache). Only when they differ does
 * `pycc_ext_global_sync` look the name up: if the dict holds another
 * object than the one last seen, the slot takes it (an object as it is, a
 * `str`, `int`, `float` or `bool` converted to the slot's native type).
 *
 * The version is CPython 3.13's `PyDictObject.ma_version_tag`, which every
 * change to the dict renews. CPython 3.14 dropped the tag, so on 3.15 it
 * is a counter of the module's own, which a dict watcher
 * (`PyDict_AddWatcher`) bumps on every change to the dict. CPython allows
 * eight dict watchers per interpreter, so every pycc module of every
 * artifact shares one: the first registers it, and keeps the table from
 * dict to counter in a capsule in the interpreter's dict, where the others
 * find it. The dict is watched from the end of the module body
 * (`pycc_ext_ns_watch`); until then every compiled read looks the name up.
 * A module's `pycc_ext_ns_version` is NULL -- and every read takes
 * its slot as it is -- on another CPython version, when no watcher can be
 * had, and in a native program.
 */

/* `PyDictObject.ma_version_tag` in CPython 3.13. */
#define PYCC_DICT_VERSION_TAG 24

/* CPython's dict-watcher API (non-limited, 3.12+), found at run time: on
 * Linux and macOS the interpreter's own symbols are the module's global
 * scope, so a weak reference finds them; Windows names `python3XY.dll`,
 * whose handle is `sys.dllhandle`, to `GetProcAddress`. */
typedef int (*pycc_dict_watch_cb_t)(int event, PyObject *dict, PyObject *key, PyObject *value);
#define PYCC_DICT_EVENT_DEALLOCATED 5
#ifdef _WIN32
__declspec(dllimport) void *__stdcall GetProcAddress(void *module, const char *name);
#else
extern int PyDict_AddWatcher(pycc_dict_watch_cb_t callback) __attribute__((weak));
extern int PyDict_Watch(int watcher_id, PyObject *dict) __attribute__((weak));
#endif

/* The watcher every pycc module of the interpreter shares (one layout per
 * capsule name): its id and an open-addressed table from a watched dict to
 * its module's counter. A slot's `dict` is NULL when empty and
 * `PYCC_DICT_WATCH_GONE` once its dict is deallocated. */
typedef struct PyccDictWatchSlot {
    PyObject *dict;
    uint64_t *counter;
} PyccDictWatchSlot;

typedef struct PyccDictWatch {
    int id;
    int (*watch)(int watcher_id, PyObject *dict);
    size_t capacity;
    size_t used;
    PyccDictWatchSlot *slots;
} PyccDictWatch;

#define PYCC_DICT_WATCH_CAPSULE "pycc.dict_watch.1"
#define PYCC_DICT_WATCH_GONE ((PyObject *)(uintptr_t)1)

/* The shared watcher, once this artifact has found or made it. */
static PyccDictWatch *pycc_dict_watch = NULL;

static size_t pycc_dict_watch_home(const PyccDictWatch *w, PyObject *dict)
{
    return (size_t)(((uintptr_t)dict >> 4) * 0x9E3779B97F4A7C15ull) & (w->capacity - 1);
}

/* The watcher's callback (the registering artifact's): bumps the counter of
 * the module whose dict changed; forgets a dict going away. */
static int pycc_dict_watch_event(int event, PyObject *dict, PyObject *key, PyObject *value)
{
    PyccDictWatch *w = pycc_dict_watch;
    size_t i;

    (void)key;
    (void)value;
    if (w == NULL || w->capacity == 0) {
        return 0;
    }
    for (i = pycc_dict_watch_home(w, dict); w->slots[i].dict != NULL; i = (i + 1) & (w->capacity - 1)) {
        if (w->slots[i].dict == dict) {
            if (event == PYCC_DICT_EVENT_DEALLOCATED) {
                w->slots[i].dict = PYCC_DICT_WATCH_GONE;
            } else {
                ++*w->slots[i].counter;
            }
            break;
        }
    }
    return 0;
}

/* Adds `dict` -> `counter` to the table, growing it at half full. 0, or -1
 * (no exception set) when out of memory. */
static int pycc_dict_watch_insert(PyccDictWatch *w, PyObject *dict, uint64_t *counter)
{
    size_t i;

    if ((w->used + 1) * 2 > w->capacity) {
        size_t capacity = w->capacity == 0 ? 64 : w->capacity * 2, k;
        PyccDictWatchSlot *old = w->slots, *slots;
        size_t old_capacity = w->capacity;

        slots = (PyccDictWatchSlot *)PyMem_RawCalloc(capacity, sizeof *slots);
        if (slots == NULL) {
            return -1;
        }
        w->slots = slots;
        w->capacity = capacity;
        w->used = 0;
        for (k = 0; k < old_capacity; k++) {
            if (old[k].dict != NULL && old[k].dict != PYCC_DICT_WATCH_GONE) {
                for (i = pycc_dict_watch_home(w, old[k].dict); slots[i].dict != NULL;
                     i = (i + 1) & (capacity - 1)) {
                }
                slots[i] = old[k];
                w->used++;
            }
        }
        PyMem_RawFree(old);
    }
    for (i = pycc_dict_watch_home(w, dict); w->slots[i].dict != NULL; i = (i + 1) & (w->capacity - 1)) {
        if (w->slots[i].dict == dict) {
            w->slots[i].counter = counter;
            return 0;
        }
    }
    w->slots[i].dict = dict;
    w->slots[i].counter = counter;
    w->used++;
    return 0;
}

/* The interpreter's shared watcher: found in its dict, or registered and
 * stored there. NULL (no exception set) when none can be had. */
static PyccDictWatch *pycc_dict_watch_get(void)
{
    PyObject *idict, *capsule;
    int (*add)(pycc_dict_watch_cb_t) = NULL;
    int (*watch)(int, PyObject *) = NULL;
    PyccDictWatch *w;

    if (pycc_dict_watch != NULL) {
        return pycc_dict_watch;
    }
    idict = PyInterpreterState_GetDict(PyInterpreterState_Get());
    if (idict == NULL) {
        PyErr_Clear();
        return NULL;
    }
    capsule = PyDict_GetItemString(idict, PYCC_DICT_WATCH_CAPSULE);
    if (capsule != NULL) {
        pycc_dict_watch = (PyccDictWatch *)PyCapsule_GetPointer(capsule, PYCC_DICT_WATCH_CAPSULE);
        PyErr_Clear();
        return pycc_dict_watch;
    }
#ifdef _WIN32
    {
        PyObject *handle = PySys_GetObject("dllhandle");
        void *dll = handle == NULL ? NULL : PyLong_AsVoidPtr(handle);

        PyErr_Clear();
        if (dll != NULL) {
            add = (int (*)(pycc_dict_watch_cb_t))GetProcAddress(dll, "PyDict_AddWatcher");
            watch = (int (*)(int, PyObject *))GetProcAddress(dll, "PyDict_Watch");
        }
    }
#else
    add = PyDict_AddWatcher;
    watch = PyDict_Watch;
#endif
    if (add == NULL || watch == NULL) {
        return NULL;
    }
    w = (PyccDictWatch *)PyMem_RawCalloc(1, sizeof *w);
    if (w == NULL) {
        return NULL;
    }
    w->watch = watch;
    w->id = add(pycc_dict_watch_event);
    capsule = w->id < 0 ? NULL : PyCapsule_New(w, PYCC_DICT_WATCH_CAPSULE, NULL);
    if (capsule == NULL || PyDict_SetItemString(idict, PYCC_DICT_WATCH_CAPSULE, capsule) != 0) {
        /* The watcher, if registered, stays: its callback finds no table. */
        Py_XDECREF(capsule);
        PyErr_Clear();
        PyMem_RawFree(w);
        return NULL;
    }
    Py_DECREF(capsule);
    pycc_dict_watch = w;
    return w;
}

/*
 * Where module dict `dict`'s version is read, for its module's
 * `pycc_ext_ns_version`: 3.13's tag in the dict itself, or on 3.15 the
 * module's `counter` (set to 1, so a read's first look always syncs, as
 * 3.13's tags are never 0). NULL on any other version or when no dict
 * watcher can be had (no exception set).
 *
 * On 3.15 `*deferred` is set: the dict is not watched yet. Every store of
 * the module body would fire the watcher (about 2,700 events in pydantic's
 * import, 0.3M instructions), and compiled code reads few rebindable
 * globals while its own body runs. So until the body has run
 * (`pycc_ext_ns_watch`), each compiled read looks the name up in the dict
 * (`pycc_ext_global_sync` bumps the counter after every read, so no read's
 * cache matches it), as it would if every store had bumped it.
 */
static const uint64_t *pycc_ext_ns_version_of(PyObject *dict, uint64_t *counter, int *deferred)
{
    *deferred = 0;
    if (dict == NULL || sizeof(void *) != 8) {
        return NULL;
    }
    if ((Py_Version >> 16) == 0x030D) {
        return (const uint64_t *)((char *)dict + PYCC_DICT_VERSION_TAG);
    }
    if ((Py_Version >> 16) != 0x030F || pycc_dict_watch_get() == NULL) {
        return NULL;
    }
    *counter = 1;
    *deferred = 1;
    return counter;
}

/*
 * Starts watching module dict `dict` once its body has run
 * (`pycc_ext_ns_version_of`): from now on only a change to the dict bumps
 * `counter`, which is bumped once more here so every read looks once.
 * Where the version is read from now on: `counter`, or NULL (every read
 * takes its slot as it is) when the dict cannot be watched. Keeps any
 * exception set.
 */
static const uint64_t *pycc_ext_ns_watch(PyObject *dict, uint64_t *counter)
{
    PyObject *raised = PyErr_GetRaisedException();
    PyccDictWatch *w = pycc_dict_watch_get();
    const uint64_t *version = NULL;

    if (w != NULL && pycc_dict_watch_insert(w, dict, counter) == 0) {
        if (w->watch(w->id, dict) == 0) {
            ++*counter;
            version = counter;
        }
    }
    PyErr_Clear();
    PyErr_SetRaisedException(raised);
    return version;
}

/*
 * `value`, the module namespace `dict`'s entry for `key`, as a global read
 * sees it: on CPython 3.15 an entry still holding a PEP 810 lazy import
 * (`types.LazyImportType`, bound by an import fallback code runs with the
 * module in `__lazy_modules__`) is resolved and replaced in `dict`, as
 * `LOAD_GLOBAL` does. A new reference, or NULL with an exception set.
 */
static PyObject *pycc_ext_ns_resolve(PyObject *dict, PyObject *key, PyObject *value)
{
    static PyObject *lazy_type = NULL;
    static int looked = 0;
    PyObject *resolved;

    if (!looked && Py_Version >= 0x030F0000) {
        PyObject *types = PyImport_ImportModule("types");

        looked = 1;
        lazy_type = types == NULL ? NULL : PyObject_GetAttrString(types, "LazyImportType");
        Py_XDECREF(types);
        PyErr_Clear();
    }
    if (lazy_type == NULL || (PyObject *)Py_TYPE(value) != lazy_type) {
        return Py_NewRef(value);
    }
    resolved = PyObject_CallMethod(value, "resolve", NULL);
    if (resolved != NULL && PyDict_SetItem(dict, key, resolved) < 0) {
        Py_CLEAR(resolved);
    }
    return resolved;
}

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

static void pycc_ext_global_sync_in(PyObject *dict, const uint64_t *version, const char *name,
                                    void *slot, int kind, PyccGlobalCache *cache, char *initialized)
{
    PyObject *saved;
    PyObject *value;
    int stored = 0;

    if (dict == NULL || version == NULL) {
        return;
    }
    cache->version = *version;
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
