/*
 * A CPython object against native text (non-strict mode): `o == "lit"` and
 * `o != "lit"` with a string literal operand, and `o in fs` for a native
 * frozenset of `str`. An exact `str` operand is answered from its UTF-8
 * bytes, so no CPython `str` is made for the native side and nothing is
 * released; any other operand takes CPython's own operation against the
 * native side converted as an `ObjectBox` converts it. Part of the one shim
 * translation unit (`src/ext_build.rs`'s `SHIM_C`), after
 * `object_convert.c`, whose frozenset packer this uses.
 */

extern signed char pycc_rt_set_contains_utf8(void *set, const char *bytes, size_t len);

/*
 * The interned CPython `str` of a string literal the compiled code compares
 * against, by the address and length of its bytes: every such literal is a
 * private constant of the generated code (`foreign_compare.rs`), alive as
 * long as the module and never reused for another text. The length is part
 * of the key because the bytes are not NUL-terminated, so the empty
 * literal's zero-size constant may share its address with the next one
 * (`""` and `"0"` did). A slot holds one literal; a colliding one replaces
 * it. Borrowed; NULL with an exception set when the `str`
 * cannot be made.
 */
#define PYCC_EXT_TEXT_SLOTS 1024
#define PYCC_EXT_TEXT_SLOT_BITS 10
static struct {
    const char *text;
    Py_ssize_t len;
    PyObject *literal;
    Py_hash_t hash;
    int ascii; /* every byte is ASCII, so `len` is also its length */
} pycc_ext_texts[PYCC_EXT_TEXT_SLOTS];

static int pycc_ext_text_slot(const char *text, Py_ssize_t len)
{
    uint64_t key = (uint64_t)(uintptr_t)text + (uint64_t)len;
    size_t slot = (size_t)((key * 0x9E3779B97F4A7C15ull) >> (64 - PYCC_EXT_TEXT_SLOT_BITS)) & (PYCC_EXT_TEXT_SLOTS - 1);
    PyObject *literal;
    Py_hash_t hash;
    Py_ssize_t i;
    int ascii = 1;

    if (pycc_ext_texts[slot].text == text && pycc_ext_texts[slot].len == len) {
        return (int)slot;
    }
    for (i = 0; i < len; i++) {
        if ((unsigned char)text[i] >= 0x80) {
            ascii = 0;
        }
    }
    literal = PyUnicode_FromStringAndSize(text, len);
    if (literal == NULL) {
        return -1;
    }
    PyUnicode_InternInPlace(&literal);
    hash = PyObject_Hash(literal);
    if (hash == -1) {
        Py_DECREF(literal);
        return -1;
    }
    Py_XDECREF(pycc_ext_texts[slot].literal);
    pycc_ext_texts[slot].text = text;
    pycc_ext_texts[slot].len = len;
    pycc_ext_texts[slot].literal = literal;
    pycc_ext_texts[slot].hash = hash;
    pycc_ext_texts[slot].ascii = ascii;
    return (int)slot;
}

/*
 * `o == text` (`op` is `Py_EQ`) or `o != text` (`Py_NE`), `text` a string
 * literal's `len` UTF-8 bytes; `text_first` when the literal is the left
 * operand. `o` is borrowed. Returns 1 or 0, or -1 with the exception set.
 *
 * The literal is the interned `str` `pycc_ext_text_slot` keeps, as CPython's
 * own literal is an interned constant. An exact `str` is the literal itself
 * (equal); against an ASCII literal, when `str`'s 3.13 layout was confirmed
 * (`pycc_ext_ic_str_layout`), a compact ASCII `str` is decided by its
 * length, computed hash and text, read in place; otherwise it has another
 * hash (unequal), or is decided by its UTF-8 bytes:
 * two `str`s are equal exactly when their encodings are (one with a lone
 * surrogate has none and is compared by CPython). `None` equals no `str`
 * under either operand order: both `__eq__`s answer `NotImplemented` and
 * the identity fallback is false. Anything else -- a `str` subclass, an
 * object with its own `__eq__` -- is compared by CPython, in the source's
 * operand order, against the literal.
 */
int pycc_ext_obj_eq_text(PyObject *o, const char *text, Py_ssize_t len, int op, int text_first)
{
    const char *bytes;
    Py_ssize_t size;
    PyObject *literal;
    PyObject *result;
    Py_hash_t hash;
    int equal;
    int slot;

    if (o == NULL) {
        return -1;
    }
    slot = pycc_ext_text_slot(text, len);
    if (slot < 0) {
        return -1;
    }
    literal = pycc_ext_texts[slot].literal;
    if (PyUnicode_CheckExact(o)) {
        if (o == literal) {
            return op == Py_EQ;
        }
        if (pycc_ext_ic_state == 0) {
            pycc_ext_ic_probe();
        }
        if (pycc_ext_ic_str_layout && pycc_ext_texts[slot].ascii
            && (PYCC_IC_FIELD(o, PYCC_IC_STR_STATE, uint32_t) & PYCC_IC_STR_COMPACT_ASCII)
                   == PYCC_IC_STR_COMPACT_ASCII) {
            hash = PYCC_IC_FIELD(o, PYCC_IC_STR_HASH, Py_hash_t);
            equal = PYCC_IC_FIELD(o, PYCC_IC_STR_LENGTH, Py_ssize_t) == len
                    && (hash == -1 || hash == pycc_ext_texts[slot].hash)
                    && memcmp((char *)o + PYCC_IC_STR_ASCII_DATA, text, (size_t)len) == 0;
            return op == Py_EQ ? equal : !equal;
        }
        hash = PyObject_Hash(o);
        if (hash == -1) {
            return -1;
        }
        if (hash != pycc_ext_texts[slot].hash) {
            return op == Py_NE;
        }
        bytes = PyUnicode_AsUTF8AndSize(o, &size);
        if (bytes != NULL) {
            equal = size == len && memcmp(bytes, text, (size_t)len) == 0;
            return op == Py_EQ ? equal : !equal;
        }
        PyErr_Clear();
    } else if (o == Py_None) {
        return op == Py_NE;
    }
    Py_INCREF(literal);
    result = text_first ? PyObject_RichCompare(literal, o, op) : PyObject_RichCompare(o, literal, op);
    Py_DECREF(literal);
    if (result == NULL) {
        return -1;
    }
    equal = PyObject_IsTrue(result);
    Py_DECREF(result);
    return equal;
}

/*
 * `item in set` for a native frozenset of `str` (borrowed, as is `item`).
 * Returns 1 or 0, or -1 with the exception set.
 *
 * An exact `str` is looked up by its UTF-8 bytes in the native table, which
 * hashes and compares `str` keys by those bytes. Any other item, or a set
 * the runtime cannot answer for (`pycc_rt_set_contains_utf8` -1), takes
 * `PySequence_Contains` against the frozenset's CPython copy, the one an
 * `ObjectBox` of it shares.
 */
int pycc_ext_obj_in_text_set(PyObject *item, void *set)
{
    const char *bytes;
    Py_ssize_t size;
    PyObject *packed;
    int found;

    if (item == NULL) {
        return -1;
    }
    if (PyUnicode_CheckExact(item)) {
        bytes = PyUnicode_AsUTF8AndSize(item, &size);
        if (bytes != NULL) {
            found = pycc_rt_set_contains_utf8(set, bytes, (size_t)size);
            if (found >= 0) {
                return found;
            }
        } else {
            PyErr_Clear();
        }
    }
    packed = pycc_ext_obj_pack_frozenset(set);
    if (packed == NULL) {
        return -1;
    }
    found = PySequence_Contains(packed, item);
    Py_DECREF(packed);
    return found;
}
