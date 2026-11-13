/*
 * Regions of selected functions (docs/design/selected-regions.md, R2): the
 * native copies of a region's members call each other natively, and a
 * member's generic body enters its copy, only while the region is *intact*:
 * every member's module global still holds the function the region was
 * armed with, running the code and defaults it had then, and every module
 * global the copies' proofs rely on (a compiled `re.Pattern`, fact B3) still
 * holds the object it held.
 *
 * Each region has a flag in its module's unit (`pycc_ext_region_flags`,
 * `module_ctx.c`), set once by the module body's trailer
 * (`__pycc_regions__`, `pycc_ext_regions_arm`) and cleared, never to be set
 * again, by two watchers:
 *
 * - a dict watcher (`PyDict_AddWatcher`) on the module's dict clears it when
 *   one of the region's keys is assigned or deleted, or the dict is cleared
 *   or freed;
 * - a function watcher (`PyFunction_AddWatcher`) clears it when a member's
 *   `__code__`, `__defaults__` or `__kwdefaults__` is replaced, or the
 *   function is freed. CPython calls it for every function it creates, so
 *   it returns at once on `PyFunction_EVENT_CREATE`.
 *
 * CPython allows eight dict watchers and eight function watchers per
 * interpreter, so every pycc module of every artifact shares one of each:
 * the first registers them and keeps the table of watched keys in a capsule
 * in the interpreter's dict, where the others find it (as `global_sync.c`
 * shares its dict watcher). When no watcher can be had, no region is armed
 * and every member runs its generic body; the capsule then records the
 * failure, so later artifacts do not register (and leak) watchers again.
 *
 * The callbacks run inside whatever CPython was doing, possibly with an
 * exception set: each saves it first and restores it last, so the lookups
 * that may clear an error of their own leave the caller's alone.
 */

/* CPython's watcher APIs (non-limited, 3.12+), found at run time as in
 * `global_sync.c`. */
typedef int (*pycc_region_dict_cb_t)(int event, PyObject *dict, PyObject *key, PyObject *value);
typedef int (*pycc_region_func_cb_t)(int event, PyObject *func, PyObject *value);
#define PYCC_REGION_DICT_ADDED 0
#define PYCC_REGION_DICT_MODIFIED 1
#define PYCC_REGION_DICT_DELETED 2
#define PYCC_REGION_FUNC_CREATE 0
#define PYCC_REGION_FUNC_DESTROY 1
#define PYCC_REGION_FUNC_MODIFY_CODE 2
#define PYCC_REGION_FUNC_MODIFY_DEFAULTS 3
#define PYCC_REGION_FUNC_MODIFY_KWDEFAULTS 4
#ifndef _WIN32
extern int PyFunction_AddWatcher(pycc_region_func_cb_t callback) __attribute__((weak));
extern int PyDict_ClearWatcher(int watcher_id) __attribute__((weak));
#endif

/* A region's flag: clear (never armed, or no longer intact), intact, or
 * armed but waiting for a pattern global that still holds a PEP 810 lazy
 * import to be resolved. Only `PYCC_REGION_INTACT` lets copies run. */
#define PYCC_REGION_CLEAR 0
#define PYCC_REGION_INTACT 1
#define PYCC_REGION_PENDING 2

/* One watched object: a member function (`key` NULL), or a module dict and
 * one of its keys. `flag` is the region's; `obj` is NULL once the entry no
 * longer watches anything. `pending` marks a pattern global still holding
 * a lazy import: the store that resolves it arms the region, if the
 * resolved object is a pattern. */
typedef struct PyccRegionWatchEntry {
    PyObject *obj;
    PyObject *key;
    unsigned char *flag;
    int pending;
} PyccRegionWatchEntry;

typedef struct PyccRegionWatch {
    /* Set when the watchers could not be registered: no region is armed. */
    int failed;
    int dict_id;
    int (*watch)(int watcher_id, PyObject *dict);
    size_t capacity;
    size_t used;
    PyccRegionWatchEntry *entries;
} PyccRegionWatch;

#define PYCC_REGION_WATCH_CAPSULE "pycc.region_watch.3"

static PyccRegionWatch *pycc_region_watch = NULL;

static int pycc_region_type_is(PyObject *obj, const char *name);

/* Whether `a` and `b`, a watched key and a key a dict event names, are the
 * same string (a module dict's keys need not be the interned ones). */
static int pycc_region_same_key(PyObject *a, PyObject *b)
{
    int same;

    if (a == b) {
        return 1;
    }
    if (!PyUnicode_Check(b) || PyUnicode_GetLength(a) != PyUnicode_GetLength(b)) {
        return 0;
    }
    same = PyUnicode_Compare(a, b) == 0;
    /* Two `str`s compare without failing; the caller saved any pending
     * exception, so this clears nothing of anyone else's. */
    PyErr_Clear();
    return same;
}

/* Whether no entry of the region whose flag is `flag` is still pending. */
static int pycc_region_none_pending(PyccRegionWatch *w, unsigned char *flag)
{
    size_t i;

    for (i = 0; i < w->used; i++) {
        if (w->entries[i].obj != NULL && w->entries[i].flag == flag && w->entries[i].pending) {
            return 0;
        }
    }
    return 1;
}

/* An event on every entry whose object is `obj` and, for a dict entry,
 * whose key equals `key` (any key when `key` is NULL). It clears the
 * entry's region, except a store of a pattern (`value`) into a pending
 * entry of a region still waiting, which resolves the entry and arms the
 * region once nothing else is pending. Forgets the entries when `forget`. */
static void pycc_region_watch_event(PyccRegionWatch *w, PyObject *obj, PyObject *key, PyObject *value,
                                    int dict, int forget)
{
    size_t i;

    for (i = 0; i < w->used; i++) {
        PyccRegionWatchEntry *e = &w->entries[i];

        if (e->obj != obj || (e->key == NULL) == dict) {
            continue;
        }
        if (dict && key != NULL && !pycc_region_same_key(e->key, key)) {
            continue;
        }
        if (e->pending && *e->flag == PYCC_REGION_PENDING && value != NULL &&
            pycc_region_type_is(value, "re.Pattern")) {
            e->pending = 0;
            if (pycc_region_none_pending(w, e->flag)) {
                *e->flag = PYCC_REGION_INTACT;
            }
        } else {
            e->pending = 0;
            *e->flag = PYCC_REGION_CLEAR;
        }
        if (forget) {
            e->obj = NULL;
        }
    }
}

static int pycc_region_dict_event(int event, PyObject *dict, PyObject *key, PyObject *value)
{
    PyccRegionWatch *w = pycc_region_watch;
    PyObject *saved;

    if (w == NULL || w->failed) {
        return 0;
    }
    saved = PyErr_GetRaisedException();
    if (event == PYCC_REGION_DICT_ADDED || event == PYCC_REGION_DICT_MODIFIED) {
        pycc_region_watch_event(w, dict, key, value, 1, 0);
    } else if (event == PYCC_REGION_DICT_DELETED) {
        pycc_region_watch_event(w, dict, key, NULL, 1, 0);
    } else {
        /* Cloned into, cleared or freed: every key of the dict. */
        pycc_region_watch_event(w, dict, NULL, NULL, 1, 1);
    }
    PyErr_SetRaisedException(saved);
    return 0;
}

static int pycc_region_func_event(int event, PyObject *func, PyObject *value)
{
    PyccRegionWatch *w = pycc_region_watch;
    PyObject *saved;

    (void)value;
    if (event == PYCC_REGION_FUNC_CREATE || w == NULL || w->failed) {
        return 0;
    }
    if (event == PYCC_REGION_FUNC_DESTROY || event == PYCC_REGION_FUNC_MODIFY_CODE ||
        event == PYCC_REGION_FUNC_MODIFY_DEFAULTS || event == PYCC_REGION_FUNC_MODIFY_KWDEFAULTS) {
        saved = PyErr_GetRaisedException();
        pycc_region_watch_event(w, func, NULL, NULL, 0, event == PYCC_REGION_FUNC_DESTROY);
        PyErr_SetRaisedException(saved);
    }
    return 0;
}

/* Stores `w` in the interpreter's dict `idict` for the other artifacts. */
static void pycc_region_watch_share(PyObject *idict, PyccRegionWatch *w)
{
    PyObject *capsule = PyCapsule_New(w, PYCC_REGION_WATCH_CAPSULE, NULL);

    if (capsule == NULL || PyDict_SetItemString(idict, PYCC_REGION_WATCH_CAPSULE, capsule) != 0) {
        /* This artifact keeps its table; another registers its own. */
        PyErr_Clear();
    }
    Py_XDECREF(capsule);
}

/* The interpreter's shared watchers: found in its dict, or registered and
 * stored there. NULL (no exception set) when they cannot be had, or could
 * not be had by an earlier artifact (the shared table is marked failed). */
static PyccRegionWatch *pycc_region_watch_get(void)
{
    PyObject *idict, *capsule;
    int (*add_dict)(pycc_region_dict_cb_t) = NULL;
    int (*watch)(int, PyObject *) = NULL;
    int (*add_func)(pycc_region_func_cb_t) = NULL;
    int (*clear_dict)(int) = NULL;
    PyccRegionWatch *w;

    if (pycc_region_watch != NULL) {
        return pycc_region_watch->failed ? NULL : pycc_region_watch;
    }
    if ((Py_Version >> 16) < 0x030C) {
        return NULL;
    }
    idict = PyInterpreterState_GetDict(PyInterpreterState_Get());
    if (idict == NULL) {
        PyErr_Clear();
        return NULL;
    }
    capsule = PyDict_GetItemString(idict, PYCC_REGION_WATCH_CAPSULE);
    if (capsule != NULL) {
        pycc_region_watch = (PyccRegionWatch *)PyCapsule_GetPointer(capsule, PYCC_REGION_WATCH_CAPSULE);
        PyErr_Clear();
        return pycc_region_watch == NULL || pycc_region_watch->failed ? NULL : pycc_region_watch;
    }
#ifdef _WIN32
    {
        PyObject *handle = PySys_GetObject("dllhandle");
        void *dll = handle == NULL ? NULL : PyLong_AsVoidPtr(handle);

        PyErr_Clear();
        if (dll != NULL) {
            add_dict = (int (*)(pycc_region_dict_cb_t))GetProcAddress(dll, "PyDict_AddWatcher");
            watch = (int (*)(int, PyObject *))GetProcAddress(dll, "PyDict_Watch");
            add_func = (int (*)(pycc_region_func_cb_t))GetProcAddress(dll, "PyFunction_AddWatcher");
            clear_dict = (int (*)(int))GetProcAddress(dll, "PyDict_ClearWatcher");
        }
    }
#else
    add_dict = PyDict_AddWatcher;
    watch = PyDict_Watch;
    add_func = PyFunction_AddWatcher;
    clear_dict = PyDict_ClearWatcher;
#endif
    if (add_dict == NULL || watch == NULL || add_func == NULL || clear_dict == NULL) {
        return NULL;
    }
    w = (PyccRegionWatch *)PyMem_RawCalloc(1, sizeof *w);
    if (w == NULL) {
        return NULL;
    }
    w->watch = watch;
    w->dict_id = add_dict(pycc_region_dict_event);
    if (w->dict_id < 0 || add_func(pycc_region_func_event) < 0) {
        /* Gives the dict watcher back, and marks the table failed so later
         * artifacts neither retry nor exhaust the interpreter's ids. */
        PyErr_Clear();
        if (w->dict_id >= 0 && clear_dict(w->dict_id) != 0) {
            PyErr_Clear();
        }
        w->failed = 1;
    }
    pycc_region_watch = w;
    pycc_region_watch_share(idict, w);
    return w->failed ? NULL : w;
}

/* Adds one entry; 0, or -1 (no exception set) when out of memory. The key,
 * when there is one, is kept alive by the entry. */
static int pycc_region_watch_add(PyccRegionWatch *w, PyObject *obj, PyObject *key, unsigned char *flag,
                                 int pending)
{
    size_t i;

    for (i = 0; i < w->used; i++) {
        if (w->entries[i].obj == NULL) {
            break;
        }
    }
    if (i == w->used) {
        if (w->used == w->capacity) {
            size_t capacity = w->capacity == 0 ? 32 : w->capacity * 2;
            PyccRegionWatchEntry *entries =
                (PyccRegionWatchEntry *)PyMem_RawRealloc(w->entries, capacity * sizeof *entries);

            if (entries == NULL) {
                return -1;
            }
            w->entries = entries;
            w->capacity = capacity;
        }
        w->used++;
    } else {
        Py_XDECREF(w->entries[i].key);
    }
    Py_XINCREF(key);
    w->entries[i].obj = obj;
    w->entries[i].key = key;
    w->entries[i].flag = flag;
    w->entries[i].pending = pending;
    return 0;
}

/* Whether `obj` is a PEP 810 lazy import (`types.LazyImportType`, 3.15)
 * not resolved yet. */
static int pycc_region_is_lazy(PyObject *obj)
{
    static PyObject *lazy_type = NULL;
    static int looked = 0;

    if (!looked && Py_Version >= 0x030F0000) {
        PyObject *types = PyImport_ImportModule("types");

        looked = 1;
        lazy_type = types == NULL ? NULL : PyObject_GetAttrString(types, "LazyImportType");
        Py_XDECREF(types);
        PyErr_Clear();
    }
    return lazy_type != NULL && (PyObject *)Py_TYPE(obj) == lazy_type;
}

/* Whether `obj`'s type is exactly the one CPython names `name`
 * (`re.Pattern`, `function`). */
static int pycc_region_type_is(PyObject *obj, const char *name)
{
    PyObject *full = PyType_GetFullyQualifiedName(Py_TYPE(obj));
    int same;

    if (full == NULL) {
        PyErr_Clear();
        return 0;
    }
    same = PyUnicode_CompareWithASCIIString(full, name) == 0;
    Py_DECREF(full);
    return same;
}

/*
 * `__pycc_regions__(k, members, globals)`, from the module body's trailer
 * once its selected functions are patched: arms region `k` of the module
 * whose flags are `flags` (`nflags` of them) and whose dict is `ns`.
 * `members` names the region's member functions, `globals` the other module
 * globals its copies rely on (each an exact `re.Pattern`). The region is
 * intact when every member is a function, every global a pattern, and the
 * watchers watch them all; otherwise it stays clear and the members run
 * their generic bodies. A global still holding a lazy import (a module in
 * `__lazy_modules__`) leaves the region pending until the global read that
 * resolves it stores a pattern. Returns None; raises nothing.
 */
static PyObject *pycc_ext_regions_arm(unsigned char *flags, Py_ssize_t nflags, PyObject *ns,
                                      PyObject *const *args, Py_ssize_t nargs)
{
    PyccRegionWatch *w;
    Py_ssize_t k, i, n;
    PyObject *members, *globals;
    int ok = 1, pending = 0;

    if (nargs != 3 || ns == NULL || !PyDict_CheckExact(ns) || !PyLong_Check(args[0]) ||
        !PyTuple_Check(args[1]) || !PyTuple_Check(args[2])) {
        Py_RETURN_NONE;
    }
    k = PyLong_AsSsize_t(args[0]);
    if (k < 0 || k >= nflags) {
        PyErr_Clear();
        Py_RETURN_NONE;
    }
    flags[k] = PYCC_REGION_CLEAR;
    members = args[1];
    globals = args[2];
    w = pycc_region_watch_get();
    if (w == NULL || w->watch(w->dict_id, ns) != 0) {
        PyErr_Clear();
        Py_RETURN_NONE;
    }
    n = PyTuple_Size(members);
    for (i = 0; ok && i < n; i++) {
        PyObject *name = PyTuple_GetItem(members, i);
        PyObject *func = PyDict_GetItemWithError(ns, name);

        ok = func != NULL && pycc_region_type_is(func, "function") &&
             pycc_region_watch_add(w, func, NULL, &flags[k], 0) == 0 &&
             pycc_region_watch_add(w, ns, name, &flags[k], 0) == 0;
    }
    n = PyTuple_Size(globals);
    for (i = 0; ok && i < n; i++) {
        PyObject *name = PyTuple_GetItem(globals, i);
        PyObject *value = PyDict_GetItemWithError(ns, name);

        int lazy = value != NULL && pycc_region_is_lazy(value);

        ok = value != NULL && (lazy || pycc_region_type_is(value, "re.Pattern")) &&
             pycc_region_watch_add(w, ns, name, &flags[k], lazy) == 0;
        pending |= lazy;
    }
    PyErr_Clear();
    if (ok) {
        flags[k] = pending ? PYCC_REGION_PENDING : PYCC_REGION_INTACT;
    }
    Py_RETURN_NONE;
}

/* Before a native copy that closes a cycle of region edges runs its body:
 * 1, or 0 at the recursion limit (no exception set: the copy raises
 * `RecursionError` itself). `pycc_ext_region_leave` follows the body when
 * it answered 1. */
int pycc_ext_region_enter(void)
{
    if (Py_EnterRecursiveCall(" while calling a Python object")) {
        PyErr_Clear();
        return 0;
    }
    return 1;
}

void pycc_ext_region_leave(void)
{
    Py_LeaveRecursiveCall();
}
