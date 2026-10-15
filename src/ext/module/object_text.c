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
 * `o == text` (`op` is `Py_EQ`) or `o != text` (`Py_NE`), `text` a string
 * literal's `len` UTF-8 bytes; `text_first` when the literal is the left
 * operand. `o` is borrowed. Returns 1 or 0, or -1 with the exception set.
 *
 * Two `str`s are equal exactly when their UTF-8 encodings are, so an exact
 * `str` is decided by its bytes (one with a lone surrogate has none and
 * takes the general path). `None` equals no `str` under either operand
 * order: both `__eq__`s answer `NotImplemented` and the identity fallback
 * is false. Anything else -- a `str` subclass, an object with its own
 * `__eq__` -- is compared by CPython, in the source's operand order,
 * against the literal as `pycc_ext_obj_pack_str` would have made it.
 */
int pycc_ext_obj_eq_text(PyObject *o, const char *text, Py_ssize_t len, int op, int text_first)
{
    const char *bytes;
    Py_ssize_t size;
    PyObject *literal;
    PyObject *result;
    int equal;

    if (o == NULL) {
        return -1;
    }
    if (PyUnicode_CheckExact(o)) {
        bytes = PyUnicode_AsUTF8AndSize(o, &size);
        if (bytes != NULL) {
            equal = size == len && memcmp(bytes, text, (size_t)len) == 0;
            return op == Py_EQ ? equal : !equal;
        }
        PyErr_Clear();
    } else if (o == Py_None) {
        return op == Py_NE;
    }
    literal = PyUnicode_FromStringAndSize(text, len);
    if (literal == NULL) {
        return -1;
    }
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
