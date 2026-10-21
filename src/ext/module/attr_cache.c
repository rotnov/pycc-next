/*
 * Attribute caches for `obj.name` and `obj.name(args)` on CPython objects
 * (v0.8 accept 1): the guards CPython 3.13's own specialized `LOAD_ATTR`
 * forms check (`_INSTANCE_VALUE`, `_SLOT`, `_PROPERTY`, `_METHOD_*`), so a
 * hit skips `PyObject_GetAttr`'s type lookup and descriptor checks, and a
 * method call `PyObject_VectorcallMethod`'s. An entry is kept per attribute
 * name and class (the name by its spelling's address, as
 * `pycc_ext_attr_name` keeps it; the class by its version tag, which
 * CPython clears whenever the class or any base changes and never reuses
 * for another class state), so a call site seeing several classes keeps
 * one entry for each.
 *
 * A class is cacheable when its attribute lookup is `object`'s -- its
 * `tp_getattro` is `PyObject_GenericGetAttr`, or the slot CPython installs
 * for a class defining `__getattr__` but not `__getattribute__`, which a
 * found attribute never reaches -- and its instances either keep their
 * attributes inline (`Py_TPFLAGS_INLINE_VALUES`: a plain Python class) or
 * have no instance dict at all (a class with `__slots__`, a builtin such as
 * `dict`). What the MRO holds for the name decides the entry:
 *
 *  - nothing, on an inline-values class: an instance value, at the index the
 *    shared keys of the class's instances give the name. A hit needs valid
 *    inline values and a non-NULL `values[index]`.
 *  - a slot (`member_descriptor` of an object member): the object at its
 *    offset; a hit needs it non-NULL.
 *  - a `property`: its getter, read from the property at each hit (a
 *    property can be re-initialized in place), called with the object.
 *  - a method descriptor (a function, `Py_TPFLAGS_METHOD_DESCRIPTOR`): the
 *    function, called with the object first, or bound through its
 *    `tp_descr_get` for a plain load. The slot and property are data
 *    descriptors, which an instance dict cannot shadow; a method can be
 *    shadowed, so on an inline-values class the shared keys must not hold
 *    the name, and a hit needs valid inline values and the keys' entry count
 *    unchanged (keys only grow, so the name is still absent).
 *
 * A module attribute (`module.name` on an exact `types.ModuleType` whose
 * class has no attribute of that name) is cached by its index among the
 * entries of the module dict's keys, holding the name: a hit needs the
 * dict's keys to be a plain `str`-keyed table with the entry at that index
 * still holding this name and a value. A deleted entry loses its key and a
 * resize makes new keys, so the entry at an index holds the name only while
 * the name is bound there. `__getattr__` of a module is reached only on a
 * miss, as by CPython.
 *
 * A borrowed function or property stays alive while the class's
 * dict is unchanged, which the tag guarantees; a call holds its own
 * reference. Anything else, or a miss, takes the generic path and refills
 * the entry from the class. The layouts read here -- `tp_version_tag`,
 * `tp_flags`, `tp_basicsize`, `tp_getattro`, `tp_dict`, `tp_dictoffset`,
 * `tp_mro`, `ht_cached_keys`, the keys object, the inline values and a
 * member descriptor's `PyMemberDef` -- are CPython 3.13's for a 64-bit GIL
 * build; `pycc_ext_ic_probe` checks them against classes made at first use
 * and leaves every cache off on any other version or any mismatch.
 * `PYCC_NO_ATTR_CACHE=1` in the environment turns them off too.
 */

#define PYCC_IC_TP_BASICSIZE 32
#define PYCC_IC_TP_GETATTRO 144
#define PYCC_IC_TP_FLAGS 168
#define PYCC_IC_TP_DICT 264
#define PYCC_IC_TP_DICTOFFSET 288
#define PYCC_IC_TP_MRO 344
#define PYCC_IC_TP_VERSION_TAG 384
#define PYCC_IC_HT_CACHED_KEYS 880
#define PYCC_IC_DK_LOG2_INDEX_BYTES 9
#define PYCC_IC_DK_KIND 10
#define PYCC_IC_DK_NENTRIES 24
#define PYCC_IC_DK_INDICES 32
#define PYCC_IC_DICT_KEYS_SPLIT 2
#define PYCC_IC_VALUES_VALID 3
#define PYCC_IC_VALUES 8
#define PYCC_IC_MEMBER_DESCR_MEMBER 40
#define PYCC_IC_MEMBER_TYPE 8
#define PYCC_IC_PROPERTY_GET 16
#define PYCC_IC_MODULE_DICT 16
#define PYCC_IC_STR_LENGTH 16
#define PYCC_IC_STR_HASH 24
#define PYCC_IC_STR_STATE 32
#define PYCC_IC_STR_ASCII_DATA 40
#define PYCC_IC_STR_COMPACT_ASCII 0x60u
#define PYCC_IC_DICT_KEYS 32
#define PYCC_IC_DICT_KEYS_UNICODE 1
#define PYCC_IC_MEMBER_OFFSET 16
#define PYCC_IC_T_OBJECT_EX 16
#define PYCC_IC_INLINE_VALUES (1UL << 2)
#define PYCC_IC_MANAGED_DICT (1UL << 4)
#define PYCC_IC_METHOD_DESCRIPTOR (1UL << 17)

#define PYCC_IC_FIELD(base, offset, type) (*(type *)((char *)(base) + (offset)))
#define PYCC_IC_TAG(type) PYCC_IC_FIELD(type, PYCC_IC_TP_VERSION_TAG, unsigned int)

/* 0 before the probe, 1 when the caches are on, -1 when they are off. */
static int pycc_ext_ic_state = 0;
/* The `tp_getattro` CPython installs for a class defining `__getattr__`. */
static getattrofunc pycc_ext_ic_getattr_hook = NULL;
/* Whether `str`'s 3.13 layout (`PyASCIIObject`) was confirmed, for
 * `pycc_ext_obj_eq_text` (`object_text.c`). */
static int pycc_ext_ic_str_layout = 0;
/* Cache hits (`pycc_ext_attr_cache_hits`). */
static long long pycc_ext_ic_hits = 0;
/* `"__getattribute__"`, interned. */
static PyObject *pycc_ext_ic_getattribute = NULL;
/* `member_descriptor`, the type of a `__slots__` entry. */
static PyTypeObject *pycc_ext_ic_member_type = NULL;

/* `pycc_ext_ic_cacheable`'s flag for a class defining `__getattr__`. */
#define PYCC_IC_HOOKED 4

/* What an entry caches (module comment); `PYCC_IC_NONE`, that the class
 * leaves nothing to cache for the name. */
#define PYCC_IC_NONE 5
#define PYCC_IC_MODULE 6
#define PYCC_IC_VALUE 1
#define PYCC_IC_SLOT 2
#define PYCC_IC_PROPERTY 3
#define PYCC_IC_METHOD 4

#define PYCC_IC_SLOTS 4096
#define PYCC_IC_SLOT_BITS 12
static struct {
    const char *spelling;
    unsigned int tag;
    int kind;
    int inline_values;   /* the class keeps instance values inline */
    Py_ssize_t index;    /* a value's index, a slot's offset, a method's keys count */
    PyObject *object;    /* the property or the method's function (borrowed), or
                          * a module attribute's name (owned) */
    descrgetfunc bind;   /* a method's `tp_descr_get` */
} pycc_ext_ics[PYCC_IC_SLOTS];

static size_t pycc_ext_ic_slot(const char *spelling, unsigned int tag)
{
    uint64_t key = (uint64_t)(uintptr_t)spelling ^ ((uint64_t)tag * 0xD6E8FEB86659FD93ull);

    return (size_t)((key * 0x9E3779B97F4A7C15ull) >> (64 - PYCC_IC_SLOT_BITS)) & (PYCC_IC_SLOTS - 1);
}

/* The inline values of `obj`, whose class keeps them (3.13: right after the
 * object header). */
static char *pycc_ext_ic_values(PyObject *obj)
{
    return (char *)obj + sizeof(PyObject);
}

static int pycc_ext_ic_valid(PyObject *obj)
{
    return PYCC_IC_FIELD(pycc_ext_ic_values(obj), PYCC_IC_VALUES_VALID, uint8_t) != 0;
}

static PyObject *pycc_ext_ic_value_at(PyObject *obj, Py_ssize_t index)
{
    return ((PyObject **)(pycc_ext_ic_values(obj) + PYCC_IC_VALUES))[index];
}

/* The shared keys of `type`'s instances (an inline-values class). */
static char *pycc_ext_ic_keys(PyTypeObject *type)
{
    return PYCC_IC_FIELD(type, PYCC_IC_HT_CACHED_KEYS, char *);
}

static Py_ssize_t pycc_ext_ic_nentries(char *keys)
{
    return PYCC_IC_FIELD(keys, PYCC_IC_DK_NENTRIES, Py_ssize_t);
}

/* The entries of a `str`-keyed keys object (combined or split): key, value pairs. */
static PyObject **pycc_ext_ic_unicode_entries(char *keys)
{
    return (PyObject **)(keys + PYCC_IC_DK_INDICES
                         + ((size_t)1 << PYCC_IC_FIELD(keys, PYCC_IC_DK_LOG2_INDEX_BYTES, uint8_t)));
}

/* The index of `name` among `keys`' entries, -1 when absent, -2 when the
 * keys are not a split table or cannot be read. */
static Py_ssize_t pycc_ext_ic_key_index(char *keys, PyObject *name)
{
    Py_ssize_t n, i;
    PyObject **entries;

    if (keys == NULL || PYCC_IC_FIELD(keys, PYCC_IC_DK_KIND, uint8_t) != PYCC_IC_DICT_KEYS_SPLIT) {
        return -2;
    }
    n = pycc_ext_ic_nentries(keys);
    entries = pycc_ext_ic_unicode_entries(keys);
    for (i = 0; i < n; i++) {
        if (entries[2 * i] == name) {
            return i;
        }
    }
    for (i = 0; i < n; i++) {
        PyObject *key = entries[2 * i];
        int equal;

        if (key == NULL || !PyUnicode_CheckExact(key)) {
            continue;
        }
        equal = PyUnicode_Compare(key, name) == 0;
        if (PyErr_Occurred()) {
            PyErr_Clear();
            return -2;
        }
        if (equal) {
            return i;
        }
    }
    return -1;
}

/* The keys of module `obj`'s dict when they are a `str`-keyed combined
 * table, else NULL. */
static char *pycc_ext_ic_module_keys(PyObject *obj)
{
    PyObject *dict = PYCC_IC_FIELD(obj, PYCC_IC_MODULE_DICT, PyObject *);
    char *keys;

    if (dict == NULL) {
        return NULL;
    }
    keys = PYCC_IC_FIELD(dict, PYCC_IC_DICT_KEYS, char *);
    return PYCC_IC_FIELD(keys, PYCC_IC_DK_KIND, uint8_t) == PYCC_IC_DICT_KEYS_UNICODE ? keys : NULL;
}

/* The value module `obj`'s dict binds to `key` at entry `index`, borrowed,
 * or NULL when that entry does not hold `key` now. */
static PyObject *pycc_ext_ic_module_value(PyObject *obj, Py_ssize_t index, PyObject *key)
{
    char *keys = pycc_ext_ic_module_keys(obj);
    PyObject **entries;

    if (keys == NULL || index >= pycc_ext_ic_nentries(keys)) {
        return NULL;
    }
    entries = pycc_ext_ic_unicode_entries(keys);
    return entries[2 * index] == key ? entries[2 * index + 1] : NULL;
}

/* The index of `name` among module `obj`'s dict entries, or -1. */
static Py_ssize_t pycc_ext_ic_module_index(PyObject *obj, PyObject *name)
{
    char *keys = pycc_ext_ic_module_keys(obj);
    PyObject **entries;
    Py_ssize_t i, n;

    if (keys == NULL) {
        return -1;
    }
    entries = pycc_ext_ic_unicode_entries(keys);
    n = pycc_ext_ic_nentries(keys);
    for (i = 0; i < n; i++) {
        if (entries[2 * i] == name && entries[2 * i + 1] != NULL) {
            return i;
        }
    }
    return -1;
}

/* The offset of an object member `descr` (a `member_descriptor`), or -1. */
static Py_ssize_t pycc_ext_ic_member_offset(PyObject *descr)
{
    char *member;

    if (Py_TYPE(descr) != pycc_ext_ic_member_type) {
        return -1;
    }
    member = PYCC_IC_FIELD(descr, PYCC_IC_MEMBER_DESCR_MEMBER, char *);
    if (member == NULL || PYCC_IC_FIELD(member, PYCC_IC_MEMBER_TYPE, int) != PYCC_IC_T_OBJECT_EX) {
        return -1;
    }
    return PYCC_IC_FIELD(member, PYCC_IC_MEMBER_OFFSET, Py_ssize_t);
}

/* A new class `name(object)` with namespace `ns`, or NULL. */
static PyObject *pycc_ext_ic_class(const char *name, PyObject *ns)
{
    PyObject *bases = PyTuple_New(0);
    PyObject *cls = bases == NULL ? NULL : PyObject_CallFunction((PyObject *)&PyType_Type, "sOO", name, bases, ns);

    Py_XDECREF(bases);
    return cls;
}

/*
 * Checks the mirrored layouts against an inline-values class with
 * instances holding `a` and `b`, a `__slots__` class and a class defining
 * `__getattr__` (whose slot it keeps). Run once, at the first fill.
 */
static void pycc_ext_ic_probe(void)
{
    PyObject *ns = NULL, *slots_ns = NULL, *cls = NULL, *slotted = NULL, *hooked = NULL;
    PyObject *inst = NULL, *sinst = NULL, *prop = NULL, *a = NULL, *b = NULL;
    PyObject *name_a = NULL, *name_b = NULL, *got = NULL, *module = NULL;
    PyObject *descr, *mro;
    PyTypeObject *type;
    char *keys, *values;
    unsigned int tag;
    const char *off = getenv("PYCC_NO_ATTR_CACHE");
    int ok = 0;

    pycc_ext_ic_state = -1;
    if ((Py_Version >> 16) != 0x030D || sizeof(void *) != 8 || (off != NULL && off[0] == '1')) {
        return;
    }
    ns = PyDict_New();
    slots_ns = Py_BuildValue("{s:(ss)}", "__slots__", "a", "b");
    a = PyLong_FromLong(12345678);
    b = PyUnicode_FromString("pycc-probe");
    name_a = PyUnicode_InternFromString("a");
    name_b = PyUnicode_InternFromString("b");
    pycc_ext_ic_getattribute = PyUnicode_InternFromString("__getattribute__");
    if (ns == NULL || slots_ns == NULL || a == NULL || b == NULL || name_a == NULL || name_b == NULL
        || pycc_ext_ic_getattribute == NULL) {
        goto done;
    }
    /* An inline-values class. */
    cls = pycc_ext_ic_class("PyccIcProbe", ns);
    if (cls == NULL) {
        goto done;
    }
    type = (PyTypeObject *)cls;
    inst = PyObject_CallNoArgs(cls);
    if (inst == NULL || PyObject_SetAttr(inst, name_a, a) != 0 || PyObject_SetAttr(inst, name_b, b) != 0) {
        goto done;
    }
    got = PyObject_GetAttr(inst, name_b);
    mro = PYCC_IC_FIELD(type, PYCC_IC_TP_MRO, PyObject *);
    ok = got == b;
    ok &= PYCC_IC_FIELD(type, PYCC_IC_TP_FLAGS, unsigned long) == PyType_GetFlags(type);
    ok &= (PyType_GetFlags(type) & (PYCC_IC_INLINE_VALUES | PYCC_IC_MANAGED_DICT))
          == (PYCC_IC_INLINE_VALUES | PYCC_IC_MANAGED_DICT);
    ok &= PYCC_IC_FIELD(type, PYCC_IC_TP_BASICSIZE, Py_ssize_t) == (Py_ssize_t)sizeof(PyObject);
    ok &= PYCC_IC_FIELD(type, PYCC_IC_TP_GETATTRO, getattrofunc) == PyObject_GenericGetAttr;
    ok &= PYCC_IC_FIELD(type, PYCC_IC_TP_DICT, PyObject *) != NULL
          && PyDict_CheckExact(PYCC_IC_FIELD(type, PYCC_IC_TP_DICT, PyObject *));
    ok &= mro != NULL && PyTuple_CheckExact(mro) && PyTuple_Size(mro) == 2
          && PyTuple_GetItem(mro, 0) == cls && PyTuple_GetItem(mro, 1) == (PyObject *)&PyBaseObject_Type;
    if (!ok) {
        goto done;
    }
    keys = pycc_ext_ic_keys(type);
    values = pycc_ext_ic_values(inst);
    ok &= keys != NULL && pycc_ext_ic_nentries(keys) == 2;
    ok &= ok && pycc_ext_ic_key_index(keys, name_a) == 0 && pycc_ext_ic_key_index(keys, name_b) == 1;
    ok &= PYCC_IC_FIELD(values, PYCC_IC_VALUES_VALID, uint8_t) == 1;
    ok &= ok && pycc_ext_ic_value_at(inst, 0) == a && pycc_ext_ic_value_at(inst, 1) == b;
    /* A lookup tagged the class; changing it clears the tag. */
    tag = PYCC_IC_TAG(type);
    ok &= tag != 0;
    ok &= PyObject_SetAttrString(cls, "c", Py_None) == 0;
    ok &= PYCC_IC_TAG(type) != tag;
    if (!ok) {
        goto done;
    }
    /* A `__slots__` class: no instance dict, members at their offsets. */
    slotted = pycc_ext_ic_class("PyccIcSlots", slots_ns);
    sinst = slotted == NULL ? NULL : PyObject_CallNoArgs(slotted);
    if (sinst == NULL || PyObject_SetAttr(sinst, name_b, b) != 0) {
        goto done;
    }
    descr = PyDict_GetItemWithError(PYCC_IC_FIELD(slotted, PYCC_IC_TP_DICT, PyObject *), name_b);
    ok &= descr != NULL;
    if (ok) {
        Py_ssize_t offset;

        pycc_ext_ic_member_type = Py_TYPE(descr);
        offset = pycc_ext_ic_member_offset(descr);
        ok &= offset >= (Py_ssize_t)sizeof(PyObject) && PYCC_IC_FIELD(sinst, offset, PyObject *) == b;
        ok &= PYCC_IC_FIELD(slotted, PYCC_IC_TP_DICTOFFSET, Py_ssize_t) == 0
              && !(PyType_GetFlags((PyTypeObject *)slotted) & PYCC_IC_MANAGED_DICT);
        ok &= PYCC_IC_FIELD(type, PYCC_IC_TP_DICTOFFSET, Py_ssize_t) == -1;
    }
    /* A property's getter. */
    prop = ok ? PyObject_CallFunctionObjArgs((PyObject *)&PyProperty_Type, (PyObject *)&PyType_Type, NULL)
              : NULL;
    ok &= prop != NULL && PYCC_IC_FIELD(prop, PYCC_IC_PROPERTY_GET, PyObject *) == (PyObject *)&PyType_Type;
    if (!ok) {
        goto done;
    }
    /* A module's dict and its entries. */
    module = PyModule_New("pycc_ic_probe");
    ok &= module != NULL && PyObject_SetAttr(module, name_b, b) == 0
          && PYCC_IC_FIELD(module, PYCC_IC_MODULE_DICT, PyObject *) == PyModule_GetDict(module);
    if (ok) {
        Py_ssize_t index = pycc_ext_ic_module_index(module, name_b);

        ok &= index >= 0 && pycc_ext_ic_module_value(module, index, name_b) == b;
    }
    if (!ok) {
        goto done;
    }
    /* A compact ASCII `str`: its length, hash, state bits and text. */
    {
        PyObject *text = PyUnicode_FromString("pycc-probe-text");
        PyObject *wide = PyUnicode_FromString("pycc-\xc3\xa9");
        Py_hash_t hash = text == NULL ? -1 : PyObject_Hash(text);

        ok &= text != NULL && wide != NULL && hash != -1
              && PYCC_IC_FIELD(text, PYCC_IC_STR_LENGTH, Py_ssize_t) == 15
              && PYCC_IC_FIELD(text, PYCC_IC_STR_HASH, Py_hash_t) == hash
              && (PYCC_IC_FIELD(text, PYCC_IC_STR_STATE, uint32_t) & PYCC_IC_STR_COMPACT_ASCII)
                     == PYCC_IC_STR_COMPACT_ASCII
              && memcmp((char *)text + PYCC_IC_STR_ASCII_DATA, "pycc-probe-text", 16) == 0
              && PYCC_IC_FIELD(wide, PYCC_IC_STR_LENGTH, Py_ssize_t) == 6
              && (PYCC_IC_FIELD(wide, PYCC_IC_STR_STATE, uint32_t) & PYCC_IC_STR_COMPACT_ASCII)
                     != PYCC_IC_STR_COMPACT_ASCII;
        Py_XDECREF(text);
        Py_XDECREF(wide);
    }
    if (!ok) {
        goto done;
    }
    /* The slot CPython gives a class defining `__getattr__`. */
    if (PyDict_SetItemString(ns, "__getattr__", (PyObject *)&PyType_Type) != 0) {
        goto done;
    }
    hooked = pycc_ext_ic_class("PyccIcHook", ns);
    if (hooked == NULL) {
        goto done;
    }
    pycc_ext_ic_getattr_hook = PYCC_IC_FIELD(hooked, PYCC_IC_TP_GETATTRO, getattrofunc);
    ok &= pycc_ext_ic_getattr_hook != NULL && pycc_ext_ic_getattr_hook != PyObject_GenericGetAttr;
    if (ok) {
        pycc_ext_ic_state = 1;
        pycc_ext_ic_str_layout = 1;
    }
done:
    PyErr_Clear();
    Py_XDECREF(got);
    Py_XDECREF(inst);
    Py_XDECREF(sinst);
    Py_XDECREF(prop);
    Py_XDECREF(module);
    Py_XDECREF(cls);
    Py_XDECREF(slotted);
    Py_XDECREF(hooked);
    Py_XDECREF(ns);
    Py_XDECREF(slots_ns);
    Py_XDECREF(a);
    Py_XDECREF(b);
    Py_XDECREF(name_a);
    Py_XDECREF(name_b);
}

/*
 * The first class in `type`'s MRO whose dict holds `name`, and the value
 * there (borrowed, through `*found`): 1, 0 when none does, -1 when it
 * cannot tell (no exception left set).
 */
static int pycc_ext_ic_lookup(PyTypeObject *type, PyObject *name, PyObject **holder, PyObject **found)
{
    PyObject *mro = PYCC_IC_FIELD(type, PYCC_IC_TP_MRO, PyObject *);
    Py_ssize_t i, n;

    if (mro == NULL || !PyTuple_CheckExact(mro)) {
        return -1;
    }
    n = PyTuple_Size(mro);
    for (i = 0; i < n; i++) {
        PyObject *cls = PyTuple_GetItem(mro, i);
        PyObject *dict = PYCC_IC_FIELD(cls, PYCC_IC_TP_DICT, PyObject *);
        PyObject *value;

        if (dict != NULL && PyDict_CheckExact(dict)) {
            value = PyDict_GetItemWithError(dict, name);
            if (value == NULL && PyErr_Occurred()) {
                PyErr_Clear();
                return -1;
            }
        } else {
            /* A static builtin class keeps its dict elsewhere (3.12+); its
             * contents live as long as the interpreter. */
            PyObject *proxy = PyObject_GetAttrString(cls, "__dict__");

            if (proxy == NULL) {
                PyErr_Clear();
                return -1;
            }
            value = PyObject_GetItem(proxy, name);
            Py_DECREF(proxy);
            if (value == NULL) {
                if (!PyErr_ExceptionMatches(PyExc_KeyError)) {
                    PyErr_Clear();
                    return -1;
                }
                PyErr_Clear();
            } else {
                Py_DECREF(value);
            }
        }
        if (value != NULL) {
            *holder = cls;
            *found = value;
            return 1;
        }
    }
    return 0;
}

/*
 * Whether `obj`'s class is cacheable (module comment): 2 when it keeps
 * instance values inline, 1 when its instances have no dict, 0 otherwise,
 * plus `PYCC_IC_HOOKED` when it defines `__getattr__`; -1 when the class is
 * but `obj`'s inline values are not valid.
 * Probes the layouts on first use.
 */
static int pycc_ext_ic_cacheable(PyObject *obj)
{
    PyTypeObject *type = Py_TYPE(obj);
    unsigned long flags;
    getattrofunc getattro;
    PyObject *holder, *found;
    int shape;

    if (pycc_ext_ic_state == 0) {
        pycc_ext_ic_probe();
    }
    if (pycc_ext_ic_state != 1 || PYCC_IC_TAG(type) == 0) {
        return 0;
    }
    flags = PYCC_IC_FIELD(type, PYCC_IC_TP_FLAGS, unsigned long);
    if ((flags & (PYCC_IC_INLINE_VALUES | PYCC_IC_MANAGED_DICT)) == (PYCC_IC_INLINE_VALUES | PYCC_IC_MANAGED_DICT)
        && PYCC_IC_FIELD(type, PYCC_IC_TP_BASICSIZE, Py_ssize_t) == (Py_ssize_t)sizeof(PyObject)) {
        shape = 2;
    } else if (!(flags & PYCC_IC_MANAGED_DICT) && PYCC_IC_FIELD(type, PYCC_IC_TP_DICTOFFSET, Py_ssize_t) == 0) {
        shape = 1;
    } else {
        return 0;
    }
    getattro = PYCC_IC_FIELD(type, PYCC_IC_TP_GETATTRO, getattrofunc);
    if (getattro == PyObject_GenericGetAttr) {
        return shape == 2 && !pycc_ext_ic_valid(obj) ? -1 : shape;
    }
    /* `__getattr__` alone: the first `__getattribute__` must be `object`'s. */
    if (getattro != pycc_ext_ic_getattr_hook
        || pycc_ext_ic_lookup(type, pycc_ext_ic_getattribute, &holder, &found) != 1
        || holder != (PyObject *)&PyBaseObject_Type) {
        return 0;
    }
    return shape == 2 && !pycc_ext_ic_valid(obj) ? -1 : shape | PYCC_IC_HOOKED;
}

/* Records entry `kind` of `obj.name` for `type`; returns its slot. */
static long pycc_ext_ic_store(PyTypeObject *type, const char *spelling, int kind, int inline_values,
                              Py_ssize_t index, PyObject *object, descrgetfunc bind)
{
    unsigned int tag = PYCC_IC_TAG(type);
    PyObject *released = NULL;
    size_t slot;

    if (tag == 0) {
        return -1;
    }
    slot = pycc_ext_ic_slot(spelling, tag);
    if (pycc_ext_ics[slot].kind == PYCC_IC_MODULE) {
        released = pycc_ext_ics[slot].object;
    }
    if (kind == PYCC_IC_MODULE) {
        Py_INCREF(object);
    }
    pycc_ext_ics[slot].spelling = spelling;
    pycc_ext_ics[slot].tag = tag;
    pycc_ext_ics[slot].kind = kind;
    pycc_ext_ics[slot].inline_values = inline_values;
    pycc_ext_ics[slot].index = index;
    pycc_ext_ics[slot].object = object;
    pycc_ext_ics[slot].bind = bind;
    Py_XDECREF(released);
    return (long)slot;
}

/*
 * Fills the entry of `obj.name` from `obj`'s class. What the class decides
 * is recorded even when nothing can be cached (`PYCC_IC_NONE`), so later
 * reads take the generic path without asking again; what only `obj`
 * decides (invalid inline values, a name no instance has yet) is not.
 * Returns the entry's slot, or -1.
 */
static long pycc_ext_ic_fill(PyObject *obj, const char *spelling, PyObject *name)
{
    PyTypeObject *type = Py_TYPE(obj);
    int cacheable = pycc_ext_ic_cacheable(obj);
    int shape = cacheable & ~PYCC_IC_HOOKED;
    PyObject *holder, *found = NULL;
    Py_ssize_t index;
    int held;

    if (cacheable < 0) {
        return -1;
    }
    if (type == &PyModule_Type && pycc_ext_ic_state == 1) {
        held = pycc_ext_ic_lookup(type, name, &holder, &found);
        if (held != 0) {
            return held < 0 ? -1 : pycc_ext_ic_store(type, spelling, PYCC_IC_NONE, 0, 0, NULL, NULL);
        }
        index = pycc_ext_ic_module_index(obj, name);
        return index < 0 ? -1 : pycc_ext_ic_store(type, spelling, PYCC_IC_MODULE, 0, index, name, NULL);
    }
    if (cacheable == 0) {
        return pycc_ext_ic_store(type, spelling, PYCC_IC_NONE, 0, 0, NULL, NULL);
    }
    held = pycc_ext_ic_lookup(type, name, &holder, &found);
    if (held < 0) {
        return -1;
    }
    if (held == 0) {
        index = shape == 2 ? pycc_ext_ic_key_index(pycc_ext_ic_keys(type), name) : -2;
        if (index == -1) {
            return -1;
        }
        if (index < 0) {
            return pycc_ext_ic_store(type, spelling, PYCC_IC_NONE, 0, 0, NULL, NULL);
        }
        return pycc_ext_ic_store(type, spelling, PYCC_IC_VALUE, 1, index, NULL, NULL);
    }
    index = pycc_ext_ic_member_offset(found);
    if (index >= (Py_ssize_t)sizeof(PyObject)) {
        return pycc_ext_ic_store(type, spelling, PYCC_IC_SLOT, 0, index, NULL, NULL);
    }
    if (Py_TYPE(found) == &PyProperty_Type && !(cacheable & PYCC_IC_HOOKED)) {
        /* Not with `__getattr__`, which an `AttributeError` from the getter
         * reaches: the getter is called once, by the generic path. */
        return pycc_ext_ic_store(type, spelling, PYCC_IC_PROPERTY, 0, 0, found, NULL);
    } else if (PyType_GetFlags(Py_TYPE(found)) & PYCC_IC_METHOD_DESCRIPTOR) {
        descrgetfunc bind = (descrgetfunc)PyType_GetSlot(Py_TYPE(found), Py_tp_descr_get);

        PyErr_Clear();
        if (shape != 2) {
            return pycc_ext_ic_store(type, spelling, PYCC_IC_METHOD, 0, 0, found, bind);
        }
        /* An instance may shadow it once the shared keys hold the name, and
         * keys never drop a name. */
        if (pycc_ext_ic_key_index(pycc_ext_ic_keys(type), name) == -1) {
            return pycc_ext_ic_store(type, spelling, PYCC_IC_METHOD, 1,
                                     pycc_ext_ic_nentries(pycc_ext_ic_keys(type)), found, bind);
        }
    }
    return pycc_ext_ic_store(type, spelling, PYCC_IC_NONE, 0, 0, NULL, NULL);
}

/* The slot of `obj.name`'s entry when it is filled for `obj`'s class, or -1. */
static long pycc_ext_ic_find(PyObject *obj, const char *spelling)
{
    unsigned int tag = PYCC_IC_TAG(Py_TYPE(obj));
    size_t slot = pycc_ext_ic_slot(spelling, tag);

    if (tag != 0 && pycc_ext_ics[slot].spelling == spelling && pycc_ext_ics[slot].tag == tag) {
        return (long)slot;
    }
    return -1;
}

/* Whether a method entry still holds for `obj` (module comment). */
static int pycc_ext_ic_method_holds(PyObject *obj, size_t slot)
{
    return !pycc_ext_ics[slot].inline_values
           || (pycc_ext_ic_valid(obj)
               && pycc_ext_ic_nentries(pycc_ext_ic_keys(Py_TYPE(obj))) == pycc_ext_ics[slot].index);
}

/* What `pycc_ext_ic_try` returns when no entry answers. */
#define PYCC_IC_MISS ((PyObject *)1)

/* `obj.name` through entry `slot`, which is not a plain value or slot. */
static PyObject *pycc_ext_ic_call_read(PyObject *obj, size_t slot)
{
    PyObject *value, *callee;

    if (pycc_ext_ics[slot].kind == PYCC_IC_PROPERTY) {
        callee = PYCC_IC_FIELD(pycc_ext_ics[slot].object, PYCC_IC_PROPERTY_GET, PyObject *);
        if (callee == NULL || callee == Py_None) {
            return PYCC_IC_MISS;
        }
        Py_INCREF(callee);
        value = PyObject_Vectorcall(callee, &obj, 1, NULL);
        Py_DECREF(callee);
        return value;
    }
    if (pycc_ext_ics[slot].kind == PYCC_IC_METHOD && pycc_ext_ics[slot].bind != NULL
        && pycc_ext_ic_method_holds(obj, slot)) {
        callee = Py_NewRef(pycc_ext_ics[slot].object);
        value = pycc_ext_ics[slot].bind(callee, obj, (PyObject *)Py_TYPE(obj));
        Py_DECREF(callee);
        return value;
    }
    return PYCC_IC_MISS;
}

/*
 * `obj.name` when an entry for `obj`'s class answers it: a new reference,
 * or NULL with the exception set (a property's getter raised);
 * `PYCC_IC_MISS` otherwise. `obj` is not NULL. Needs no interned name, so
 * a hit neither looks the name up nor hashes it.
 */
static PyObject *pycc_ext_ic_try(PyObject *obj, const char *spelling)
{
    unsigned int tag = PYCC_IC_TAG(Py_TYPE(obj));
    size_t slot = pycc_ext_ic_slot(spelling, tag);
    PyObject *value;

    if (pycc_ext_ics[slot].spelling != spelling || pycc_ext_ics[slot].tag != tag || tag == 0) {
        return PYCC_IC_MISS;
    }
    switch (pycc_ext_ics[slot].kind) {
    case PYCC_IC_VALUE:
        if (!pycc_ext_ic_valid(obj)) {
            return PYCC_IC_MISS;
        }
        value = pycc_ext_ic_value_at(obj, pycc_ext_ics[slot].index);
        break;
    case PYCC_IC_SLOT:
        value = PYCC_IC_FIELD(obj, pycc_ext_ics[slot].index, PyObject *);
        break;
    case PYCC_IC_MODULE:
        value = pycc_ext_ic_module_value(obj, pycc_ext_ics[slot].index, pycc_ext_ics[slot].object);
        break;
    case PYCC_IC_NONE:
        return PYCC_IC_MISS;
    default:
        value = pycc_ext_ic_call_read(obj, slot);
        if (value != PYCC_IC_MISS) {
            pycc_ext_ic_hits++;
        }
        return value;
    }
    if (value == NULL) {
        return PYCC_IC_MISS;
    }
    pycc_ext_ic_hits++;
    return Py_NewRef(value);
}

/*
 * `obj.name` on a miss of `pycc_ext_ic_try`, `name` the interned name of
 * `spelling`: `PyObject_GetAttr`, after which an entry is filled when the
 * class has none yet. A new reference, or NULL with the exception set.
 */
static PyObject *pycc_ext_ic_getattr_slow(PyObject *obj, const char *spelling, PyObject *name)
{
    PyObject *value = PyObject_GetAttr(obj, name);

    long slot;

    if (value == NULL || pycc_ext_ic_state < 0) {
        return value;
    }
    /* An entry for the class already says what it can cache, except for a
     * module, whose entry names one binding of one module. */
    slot = pycc_ext_ic_find(obj, spelling);
    if (slot < 0 || pycc_ext_ics[slot].kind == PYCC_IC_MODULE) {
        (void)pycc_ext_ic_fill(obj, spelling, name);
    }
    return value;
}

/*
 * What `obj.name(...)` calls when an entry answers it (borrowed): a cached
 * method of `obj`'s class, called with `obj` first (`*with_self` set), or a
 * module's attribute, called as it is. NULL when the call must take the
 * generic path, or when `name` is NULL and the entry has to be filled first
 * (the caller then makes the name and asks again). Never sets an exception.
 */
static PyObject *pycc_ext_ic_method(PyObject *obj, const char *spelling, PyObject *name, int *with_self)
{
    long slot = pycc_ext_ic_find(obj, spelling);
    PyObject *value;

    if (slot < 0 || (pycc_ext_ics[slot].kind == PYCC_IC_MODULE && name != NULL)) {
        if (name == NULL || pycc_ext_ic_state < 0) {
            return NULL;
        }
        slot = pycc_ext_ic_fill(obj, spelling, name);
    }
    if (slot < 0) {
        return NULL;
    }
    if (pycc_ext_ics[slot].kind == PYCC_IC_MODULE) {
        value = pycc_ext_ic_module_value(obj, pycc_ext_ics[slot].index, pycc_ext_ics[slot].object);
        if (value != NULL) {
            pycc_ext_ic_hits++;
            *with_self = 0;
        }
        return value;
    }
    if (pycc_ext_ics[slot].kind != PYCC_IC_METHOD || !pycc_ext_ic_method_holds(obj, (size_t)slot)) {
        return NULL;
    }
    pycc_ext_ic_hits++;
    *with_self = 1;
    return pycc_ext_ics[slot].object;
}

/*
 * How many attribute reads and method calls the caches answered, which the
 * tests read with `ctypes` (`RUNTIME_COUNTERS` in `src/ext_build.rs`) to
 * prove the caches are on where the probe should pass.
 */
long long pycc_ext_attr_cache_hits(void)
{
    return pycc_ext_ic_hits;
}
