/*
 * What the generated code's inline fast paths read
 * (`crates/pycc_codegen/src/fast_ops.rs`): an operation on CPython objects
 * in a generic body -- `a + b`, `a < b`, `s[i]`, `s == "x"` -- first checks
 * for an exact `int` or `str` it can answer in place, as CPython's
 * specializing interpreter does (`BINARY_OP_ADD_INT`, `COMPARE_OP_INT`,
 * `BINARY_SUBSCR_STR_INT`, `COMPARE_OP_STR`), and calls the shim helper for
 * anything else.
 *
 * The code reads the layouts of CPython 3.13 to 3.15 for a 64-bit GIL
 * build, which the limited API does not promise:
 *
 * - every object: `ob_type` at offset 8;
 * - `int` (`PyLongObject`): `lv_tag` at 16, the first 30-bit digit (a
 *   `uint32_t`) at 24; a value is compact (one digit or none) when
 *   `lv_tag < 16`, and its value is `(1 - (lv_tag & 3)) * digit`;
 * - `str` (`PyASCIIObject`): the length at 16, the hash at 24 (-1 before
 *   it is computed), the state at 32 (`0x60` set: compact and ASCII), the
 *   ASCII text at 40;
 * - `bytes` (`PyBytesObject`): the length at 16, the bytes at 32;
 * - `bytearray` (`PyByteArrayObject`): the length at 16, a pointer to the
 *   first byte (`ob_start`) at 40.
 *
 * `pycc_ext_fast_init` checks them against objects made at module exec and
 * fills `pycc_ext_fast` only when every check passes: the type pointers
 * stay NULL otherwise, so the generated code's exact-type checks never
 * pass and every operation takes its helper. `PYCC_NO_FAST_OPS=1` in the
 * environment leaves them NULL too (the tests' way to run the helpers).
 *
 * The generated code reads the struct by byte offset; the layout is fixed
 * here and in `fast_ops.rs` (`STR_TYPE`, `INT_TYPE`, `CHARS`, `CLASSES`). The
 * one-character strings are CPython's own cached latin-1 characters, each
 * held by a reference the struct keeps.
 */

typedef struct {
    PyTypeObject *str_type;
    PyTypeObject *int_type;
    PyObject *chars[128];
    /* `pycc_ext_obj_isinstance`'s built-in classes, by its selector, then
     * `set`, `frozenset`, `bytes`, `bytearray` and `NoneType`. */
    PyTypeObject *classes[12];
} PyccExtFast;

static PyccExtFast pycc_ext_fast;

/* `ob_type`, read where the generated code reads it. */
#define PYCC_FAST_TYPE(o) (*(PyTypeObject **)((char *)(o) + 8))
#define PYCC_FAST_INT_TAG(o) (*(uintptr_t *)((char *)(o) + 16))
#define PYCC_FAST_INT_DIGIT(o) (*(uint32_t *)((char *)(o) + 24))
#define PYCC_FAST_STR_LENGTH(o) (*(Py_ssize_t *)((char *)(o) + 16))
#define PYCC_FAST_STR_HASH(o) (*(Py_hash_t *)((char *)(o) + 24))
#define PYCC_FAST_STR_STATE(o) (*(uint32_t *)((char *)(o) + 32))
#define PYCC_FAST_STR_DATA(o) ((unsigned char *)(o) + 40)

/* Whether `o` reads as the compact `int` `value` (or, for `compact == 0`,
 * as a non-compact one) through the layout above. */
static int pycc_ext_fast_int_reads(PyObject *o, long long value, int compact)
{
    uintptr_t tag;

    if (o == NULL || PYCC_FAST_TYPE(o) != &PyLong_Type || Py_TYPE(o) != &PyLong_Type) {
        return 0;
    }
    tag = PYCC_FAST_INT_TAG(o);
    if (!compact) {
        return tag >= 16;
    }
    return tag < 16 && (long long)(1 - (long long)(tag & 3)) * (long long)PYCC_FAST_INT_DIGIT(o) == value;
}

/* Whether `o` reads as the compact ASCII `str` of `len` bytes `text`. */
static int pycc_ext_fast_str_reads(PyObject *o, const char *text, Py_ssize_t len)
{
    Py_hash_t hash;

    if (o == NULL || PYCC_FAST_TYPE(o) != &PyUnicode_Type || Py_TYPE(o) != &PyUnicode_Type) {
        return 0;
    }
    hash = PyObject_Hash(o);
    return hash != -1 && PYCC_FAST_STR_LENGTH(o) == len && PYCC_FAST_STR_HASH(o) == hash
           && (PYCC_FAST_STR_STATE(o) & 0x60u) == 0x60u
           && memcmp(PYCC_FAST_STR_DATA(o), text, (size_t)len) == 0;
}

/* Fills `pycc_ext_fast` once per artifact (the file comment). A failure
 * leaves the type pointers NULL and no exception set. */
static void pycc_ext_fast_init(void)
{
    static int done = 0;
    static const long long compact[] = {0, 1, -1, 7, -300, 123456, (1LL << 30) - 1, -((1LL << 30) - 1)};
    PyObject *chars[128] = {NULL};
    const char *off = getenv("PYCC_NO_FAST_OPS");
    PyObject *o;
    size_t i;
    int ok = 1;

    if (done) {
        return;
    }
    done = 1;
    if (sizeof(void *) != 8 || (off != NULL && off[0] == '1') || (Py_Version >> 16) < 0x030D
        || (Py_Version >> 16) > 0x030F) {
        return;
    }
    for (i = 0; ok && i < sizeof compact / sizeof compact[0]; i++) {
        o = PyLong_FromLongLong(compact[i]);
        ok = pycc_ext_fast_int_reads(o, compact[i], 1);
        Py_XDECREF(o);
    }
    if (ok) {
        o = PyLong_FromLongLong(1LL << 30);
        ok = pycc_ext_fast_int_reads(o, 0, 0);
        Py_XDECREF(o);
    }
    if (ok) {
        o = PyUnicode_FromString("pycc-fast-text");
        ok = pycc_ext_fast_str_reads(o, "pycc-fast-text", 14);
        Py_XDECREF(o);
    }
    if (ok) {
        /* A non-ASCII `str` must not read as compact ASCII. */
        o = PyUnicode_FromString("pycc-\xc3\xa9");
        ok = o != NULL && PYCC_FAST_STR_LENGTH(o) == 6 && (PYCC_FAST_STR_STATE(o) & 0x60u) != 0x60u;
        Py_XDECREF(o);
    }
    if (ok) {
        PyObject *b = PyBytes_FromStringAndSize("pycc\x00\xff", 6);
        PyObject *ba = PyByteArray_FromStringAndSize("pycc\x00\xff", 6);

        ok = b != NULL && ba != NULL && PYCC_FAST_TYPE(b) == &PyBytes_Type
             && PYCC_FAST_TYPE(ba) == &PyByteArray_Type && *(Py_ssize_t *)((char *)b + 16) == 6
             && memcmp((char *)b + 32, "pycc\x00\xff", 6) == 0 && *(Py_ssize_t *)((char *)ba + 16) == 6
             && *(char **)((char *)ba + 40) == PyByteArray_AsString(ba)
             && memcmp(*(char **)((char *)ba + 40), "pycc\x00\xff", 6) == 0;
        if (ok && PyByteArray_Resize(ba, 2000) == 0) {
            ok = *(Py_ssize_t *)((char *)ba + 16) == 2000 && *(char **)((char *)ba + 40) == PyByteArray_AsString(ba);
        }
        Py_XDECREF(b);
        Py_XDECREF(ba);
    }
    for (i = 0; ok && i < 128; i++) {
        char c = (char)i;

        chars[i] = PyUnicode_FromOrdinal((int)i);
        ok = pycc_ext_fast_str_reads(chars[i], &c, 1);
    }
    if (!ok) {
        PyErr_Clear();
        for (i = 0; i < 128; i++) {
            Py_XDECREF(chars[i]);
        }
        return;
    }
    memcpy(pycc_ext_fast.chars, chars, sizeof chars);
    pycc_ext_fast.classes[0] = &PyLong_Type;
    pycc_ext_fast.classes[1] = &PyUnicode_Type;
    pycc_ext_fast.classes[2] = &PyFloat_Type;
    pycc_ext_fast.classes[3] = &PyBool_Type;
    pycc_ext_fast.classes[4] = &PyList_Type;
    pycc_ext_fast.classes[5] = &PyDict_Type;
    pycc_ext_fast.classes[6] = &PyTuple_Type;
    pycc_ext_fast.classes[7] = &PySet_Type;
    pycc_ext_fast.classes[8] = &PyFrozenSet_Type;
    pycc_ext_fast.classes[9] = &PyBytes_Type;
    pycc_ext_fast.classes[10] = &PyByteArray_Type;
    pycc_ext_fast.classes[11] = Py_TYPE(Py_None);
    pycc_ext_fast.str_type = &PyUnicode_Type;
    pycc_ext_fast.int_type = &PyLong_Type;
}

/* `c in s` for an exact `frozenset` `s` and a one-character ASCII `str` `c`
 * (`pycc_codegen::fast_ops::Fast::ascii_member`): a bit test against the
 * set's ASCII characters, which a `frozenset`'s immutability lets the
 * generated code compute once per set instead of hashing and probing per
 * test (`CONTAINS_OP_SET`'s lookup). Parsers test one character at a time
 * against such constants (`char in WHITESPACE`).
 *
 * The masks live in a two-way cache indexed by the set's address
 * (`PYCC_ASCII_MASK_INDEX`), most recently filled way first; the generated
 * code reads both ways inline and calls `pycc_ext_ascii_mask_fill` on a
 * miss. An entry whose `usable` is 1 holds a reference to its set, so the
 * address cannot be reused while it is cached; its every member is an exact
 * `str` (`bits` bit `b` set when `chr(b)` is one), so the answer is the
 * set's own: an exact `str` equals only an equal exact `str`, whatever the
 * hash. A set with a member of another type (whose `__eq__` CPython's
 * lookup may call) or with more than `PYCC_ASCII_MASK_LIMIT` members is
 * recorded as not `usable`, without a reference: should its address be
 * reused, the new set merely takes the generic lookup until the entry is
 * evicted. Releasing an evicted set can run Python code (a weakref
 * callback on the set), which may refill the same bucket, so a miss returns
 * its answer from the entry it computed, not from the cache. The generated
 * code reads the layout by byte offset (`fast_ops.rs`: `MASK_*`). */
typedef struct {
    PyObject *set;
    uint64_t bits[2];
    uint64_t usable;
} PyccAsciiMask;

#define PYCC_ASCII_MASK_WAYS 64
#define PYCC_ASCII_MASK_INDEX(s) ((((uintptr_t)(s)) >> 4) & (PYCC_ASCII_MASK_WAYS - 1))
#define PYCC_ASCII_MASK_LIMIT 256

static PyccAsciiMask pycc_ext_ascii_masks[PYCC_ASCII_MASK_WAYS * 2];

/* How many masks `pycc_ext_ascii_mask_fill` computed (the tests read it
 * with `ctypes`). */
static long long pycc_ext_ascii_mask_fills = 0;

/* After a miss in both ways for the exact `frozenset` `s`: computes its
 * entry into the first way (the first moving to the second, the second's
 * set released) and returns whether the ASCII character `c` is a member, 1
 * or 0, or -1 when the set is not usable. The answer comes from the entry
 * computed here, before the evicted set is released: that release may run
 * Python code (a weakref callback, a finalizer) which refills this bucket,
 * so nothing is read from the cache after it. Never fails: an error while
 * reading the set (none is expected of a `frozenset`) makes it not usable. */
static long long pycc_ext_ascii_mask_fill(PyObject *s, long long c)
{
    PyccAsciiMask *way = &pycc_ext_ascii_masks[PYCC_ASCII_MASK_INDEX(s) * 2];
    PyccAsciiMask fresh = {s, {0, 0}, 0};
    PyccAsciiMask evicted = way[1];
    Py_ssize_t size = PySet_Size(s);
    PyObject *it;
    PyObject *member;
    long long answer;
    int usable = size >= 0 && size <= PYCC_ASCII_MASK_LIMIT;

    it = usable ? PyObject_GetIter(s) : NULL;
    usable = it != NULL;
    while (usable && (member = PyIter_Next(it)) != NULL) {
        if (Py_TYPE(member) != &PyUnicode_Type) {
            usable = 0;
        } else if (PyUnicode_GetLength(member) == 1) {
            Py_UCS4 m = PyUnicode_ReadChar(member, 0);

            if (m < 128) {
                fresh.bits[m >> 6] |= (uint64_t)1 << (m & 63);
            }
        }
        Py_DECREF(member);
    }
    Py_XDECREF(it);
    if (PyErr_Occurred()) {
        PyErr_Clear();
        usable = 0;
    }
    if (usable) {
        fresh.usable = 1;
        Py_INCREF(s);
    }
    pycc_ext_ascii_mask_fills++;
    answer = usable ? (long long)((fresh.bits[(c >> 6) & 1] >> (c & 63)) & 1) : -1;
    way[1] = way[0];
    way[0] = fresh;
    if (evicted.usable) {
        Py_DECREF(evicted.set);
    }
    return answer;
}

long long pycc_ext_ascii_mask_fill_count(void)
{
    return pycc_ext_ascii_mask_fills;
}

/* A compact `int` result of an inline fast path, a new reference or NULL
 * with the exception set (named as a packer: it runs no Python code). */
PyObject *pycc_ext_obj_pack_i64(long long value)
{
    return PyLong_FromLongLong(value);
}

/* 1 when the fast paths are on (`pycc_ext_fast_init` confirmed the layouts),
 * else 0; read by the tests through `ctypes`. */
int pycc_ext_fast_ops_enabled(void)
{
    return pycc_ext_fast.str_type != NULL;
}

/* Before a compiled call site calls a lifted body directly
 * (`pycc_codegen::direct_call`): what the call through the body's
 * forwarder checks (`pycc_fwd_target`). 1 when the call may go ahead
 * (`Py_LeaveRecursiveCall` follows it); 0 at the recursion limit, with no
 * exception set: the site then calls through the forwarder, which raises
 * CPython's `RecursionError` itself. */
static long long pycc_ext_direct_count = 0;

int pycc_ext_direct_enter(void)
{
    if (Py_EnterRecursiveCall(" while calling a Python object")) {
        PyErr_Clear();
        return 0;
    }
    pycc_ext_direct_count++;
    return 1;
}

/* How many direct calls `pycc_ext_direct_enter` let go ahead, which the
 * tests read with `ctypes` (`RUNTIME_COUNTERS`). */
long long pycc_ext_direct_calls(void)
{
    return pycc_ext_direct_count;
}

/* After a direct call `pycc_ext_direct_enter` let go ahead. */
void pycc_ext_direct_leave(void)
{
    Py_LeaveRecursiveCall();
}
