/*
 * Renders the `, element N` half of an argument-position clause into `buf`,
 * or the empty string when `element` is negative -- the "this value is the
 * argument itself, not one of its tuple elements" sentinel every scalar
 * entry point below passes.
 *
 * Returned rather than written in place so an error arm can splice it into
 * `PyErr_Format` with a single `%s` and pay for it only when it raises. The
 * function name is *not* folded in here: it is an arbitrary-length Python
 * identifier and a fixed buffer would truncate it, so it stays its own `%s`.
 *
 * Both indices are rendered one-based, matching the argument index the
 * scalar messages already use (CPython's own convention, `f() argument 1`).
 * `element 2` is therefore `t[1]`; the alternative -- a one-based argument
 * beside a zero-based element in the same sentence -- reads as a typo.
 */
static const char *pycc_ext_element_clause(char *buf, size_t cap, Py_ssize_t element)
{
    if (element < 0) {
        return "";
    }
    PyOS_snprintf(buf, cap, ", element %zd", element + 1);
    return buf;
}

/*
 * Checks that one argument is a `tuple` of exactly `arity` elements.
 * Returns 0, or -1 with a CPython exception set. The elements themselves
 * are unpacked by the `_at` entry points below, one per declared element
 * type.
 *
 * `PyTuple_Check` and not `PyTuple_CheckExact`: a `tuple` subclass *is* a
 * tuple, and D-244 rule 7's boundary is closed against duck typing, not
 * against subtyping -- the same reading that lets `pycc_ext_unpack_str`
 * accept a `str` subclass. The elements are copied out by value, so the
 * subclass identity does not survive the crossing; `docs/RUNTIME.md` records
 * that narrowing alongside `str`'s.
 *
 * The arity check is what makes `PyTuple_GetItem` infallible at every call
 * site the generated wrapper emits afterwards, so it is never skipped: D-116
 * fixes a tuple type's arity, and a shorter tuple would otherwise reach a
 * `GetItem` that returns NULL with an `IndexError` the caller does not test
 * for.
 */
static int pycc_ext_unpack_tuple(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                 Py_ssize_t arity)
{
    PyObject *type_name;
    Py_ssize_t size;

    if (!PyTuple_Check(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError, "object cannot be interpreted as a tuple");
        } else {
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd: '%U' object cannot be interpreted as a tuple",
                         fn_name, index + 1, type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    size = PyTuple_Size(obj);
    if (size != arity) {
        PyErr_Format(PyExc_TypeError,
                     "%s() argument %zd: expected a tuple of length %zd, got %zd",
                     fn_name, index + 1, arity, size);
        return -1;
    }
    return 0;
}

/*
 * Encodes a CPython `int` (never a `bool`, which callers test first) as a
 * D-141 word of any size: the inline smallint when it fits, otherwise a
 * fresh heap bigint carrying one reference (#1040). The bigint travels as
 * CPython's own hex text, so neither side needs the other's digit layout.
 * Returns 0, or -1 with a CPython exception set.
 */
static int pycc_ext_long_encode(PyObject *obj, long long *out)
{
    int overflow = 0;
    long long raw = PyLong_AsLongLongAndOverflow(obj, &overflow);
    PyObject *hex;
    const char *text;
    Py_ssize_t len;

    if (raw == -1 && PyErr_Occurred()) {
        return -1;
    }
    if (overflow == 0) {
        *out = pycc_rt_int_from_i64(raw);
        return 0;
    }
    hex = PyNumber_ToBase(obj, 16);
    if (hex == NULL) {
        return -1;
    }
    text = PyUnicode_AsUTF8AndSize(hex, &len);
    if (text == NULL) {
        Py_DECREF(hex);
        return -1;
    }
    *out = pycc_rt_ext_int_from_hex(text, (size_t)len);
    Py_DECREF(hex);
    return 0;
}

/*
 * Packs a heap-bigint word as a new CPython `int`. Borrows the word: the
 * caller decides whether its reference dies here.
 */
static PyObject *pycc_ext_bigint_pack(long long encoded)
{
    char *hex = pycc_rt_ext_bigint_hex(encoded);
    PyObject *value = PyLong_FromString(hex, NULL, 16);
    pycc_rt_ext_cstr_free(hex);
    return value;
}

/*
 * Unpacks one argument at an `int` parameter into a D-141 encoded word.
 * Returns 0, or -1 with a CPython exception set.
 *
 * The order is `PyBool_Check` -> `PyLong_Check` -> TypeError, and only then
 * the conversion, and every step of it is load-bearing:
 *
 *  - `bool` first, because D-244 rule 7 admits it and D-141 encodes
 *    `False`/`True` as dedicated marker words specifically so identity
 *    survives an int-compatible slot. `PyLong_AsLongLongAndOverflow` would
 *    flatten `True` to 1 and destroy that silently.
 *  - `PyLong_Check` before converting, because
 *    `PyLong_AsLongLongAndOverflow` accepts anything implementing
 *    `__index__`. A duck type is not an `int`, and rule 7's type boundary is
 *    closed.
 *  - Any magnitude is accepted: a value outside the inline-integer range
 *    [-2^62, 2^62-1] becomes a heap bigint (`pycc_ext_long_encode`, #1040).
 */
static int pycc_ext_unpack_int_at(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                  Py_ssize_t element, long long *out)
{
    PyObject *type_name;
    char where[PYCC_EXT_ELEMENT_CLAUSE_MAX];

    if (PyBool_Check(obj)) {
        *out = pycc_rt_ext_bool_encode(obj == Py_True);
        return 0;
    }
    if (!PyLong_Check(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError,
                            "object cannot be interpreted as an integer");
        } else {
            /* CPython's own wording for a non-int at an int converter. */
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd%s: '%U' object cannot be interpreted as an integer",
                         fn_name, index + 1,
                         pycc_ext_element_clause(where, sizeof where, element), type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    return pycc_ext_long_encode(obj, out);
}

/*
 * The plain-argument entry point for a `int` parameter: the same check,
 * with no tuple-element clause in its message. Kept as its own symbol rather
 * than folded into the caller because a scalar parameter is by far the
 * common case and its generated call site should say what it means -- and
 * because the `.inc` fixtures that pin the scalar boundary predate #1050 and
 * stay byte-for-byte unchanged by it.
 */
static int pycc_ext_unpack_int(PyObject *obj, const char *fn_name, Py_ssize_t index,
                               long long *out)
{
    return pycc_ext_unpack_int_at(obj, fn_name, index, -1, out);
}

/*
 * Packs an `int`-typed result. Mirrors `classify_encoded_int`'s four-way
 * split rather than inventing a two-way one: the bool markers are both even
 * and both carry low tag 0b10, so an "odd means smallint, otherwise raise"
 * egress would answer OverflowError for `f(True)`.
 */
static PyObject *pycc_ext_pack_int(const char *fn_name, long long encoded)
{
    switch (pycc_rt_ext_int_classify(encoded)) {
    case PYCC_EXT_INT_SMALLINT:
        return PyLong_FromLongLong(pycc_rt_ext_int_decode(encoded));
    case PYCC_EXT_INT_FALSE:
        Py_RETURN_FALSE;
    case PYCC_EXT_INT_TRUE:
        Py_RETURN_TRUE;
    case PYCC_EXT_INT_BIGINT: {
        /* The caller owns this reference (D-180 rule 6) and it dies here,
         * once CPython holds the value. */
        PyObject *value = pycc_ext_bigint_pack(encoded);
        pycc_rt_bigint_release(encoded);
        return value;
    }
    default:
        /* Deliberately no release: an unclassifiable word is not a pointer
         * this code knows to be live, so freeing through it is worse than
         * leaking it. */
        PyErr_Format(PyExc_SystemError, "%s() returned an unrecognized int word", fn_name);
        return NULL;
    }
}

/*
 * Unpacks one argument at a `float` parameter. Returns 0, or -1 with a
 * CPython exception set.
 *
 * `PyFloat_Check` before `PyFloat_AsDouble`, and no fallback: the converter
 * alone accepts an `int`, a `bool` and every `__float__` duck type, and
 * `docs/TYPE_SYSTEM.md` rule 4 (D-086) forbids implicit numeric widening as
 * well as narrowing at an annotated boundary -- which D-244 rule 7 defers to
 * for conformance. So `f(1)` at a `float` parameter is a `TypeError` here,
 * deliberately unlike every C-API converter's habit.
 */
static int pycc_ext_unpack_float_at(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                    Py_ssize_t element, double *out)
{
    PyObject *type_name;
    char where[PYCC_EXT_ELEMENT_CLAUSE_MAX];

    if (!PyFloat_Check(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError, "object cannot be interpreted as a float");
        } else {
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd%s: '%U' object cannot be interpreted as a float",
                         fn_name, index + 1,
                         pycc_ext_element_clause(where, sizeof where, element), type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    *out = PyFloat_AsDouble(obj);
    return 0;
}

/*
 * Non-strict mode (#3): a Python store into an inferred `float` attribute,
 * which takes a `float` or an `int` (a `bool` included), converted as
 * `float(x)` converts it (`OverflowError` past the `double` range). Any
 * other type is the strict helper's `TypeError`.
 */
static int pycc_ext_unpack_float_or_int(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                        double *out)
{
    double value;

    if (!PyLong_Check(obj)) {
        return pycc_ext_unpack_float_at(obj, fn_name, index, -1, out);
    }
    value = PyLong_AsDouble(obj);
    if (value == -1.0 && PyErr_Occurred()) {
        return -1;
    }
    *out = value;
    return 0;
}

/*
 * The plain-argument entry point for a `float` parameter: the same check,
 * with no tuple-element clause in its message. Kept as its own symbol rather
 * than folded into the caller because a scalar parameter is by far the
 * common case and its generated call site should say what it means -- and
 * because the `.inc` fixtures that pin the scalar boundary predate #1050 and
 * stay byte-for-byte unchanged by it.
 */
static int pycc_ext_unpack_float(PyObject *obj, const char *fn_name, Py_ssize_t index,
                               double *out)
{
    return pycc_ext_unpack_float_at(obj, fn_name, index, -1, out);
}

/*
 * Unpacks one argument at a `bool` parameter. Returns 0, or -1 with a
 * CPython exception set.
 *
 * `PyBool_Check`, never `PyObject_IsTrue`: rule 7's boundary is closed, so
 * a truthy object is not a `bool`. An `int` is refused for the same reason
 * `float` refuses one -- rule 4 / D-086 -- and the `int` parameter's own
 * acceptance of `bool` does not run backwards, subtyping being
 * one-directional.
 *
 * The out parameter is a one-byte `char` because the compiled function's
 * own ABI slot is an `i8` holding 0/1 (`ty_to_basic_type`), and the
 * generated wrapper reaches it through an unchecked `void *` cast.
 */
static int pycc_ext_unpack_bool_at(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                   Py_ssize_t element, char *out)
{
    PyObject *type_name;
    char where[PYCC_EXT_ELEMENT_CLAUSE_MAX];

    if (!PyBool_Check(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError, "object cannot be interpreted as a bool");
        } else {
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd%s: '%U' object cannot be interpreted as a bool",
                         fn_name, index + 1,
                         pycc_ext_element_clause(where, sizeof where, element), type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    *out = (char)(obj == Py_True);
    return 0;
}

/*
 * The plain-argument entry point for a `bool` parameter: the same check,
 * with no tuple-element clause in its message. Kept as its own symbol rather
 * than folded into the caller because a scalar parameter is by far the
 * common case and its generated call site should say what it means -- and
 * because the `.inc` fixtures that pin the scalar boundary predate #1050 and
 * stay byte-for-byte unchanged by it.
 */
static int pycc_ext_unpack_bool(PyObject *obj, const char *fn_name, Py_ssize_t index,
                               char *out)
{
    return pycc_ext_unpack_bool_at(obj, fn_name, index, -1, out);
}

/*
 * Packs a `float`-typed result. No function name and no failure path:
 * `PyFloat_FromDouble` carries every `double`, including the infinities and
 * NaN, so unlike `pycc_ext_pack_int` there is no value it has to refuse.
 */
static PyObject *pycc_ext_pack_float(double value)
{
    return PyFloat_FromDouble(value);
}

/*
 * Packs a `bool`-typed result. `PyBool_FromLong` returns the interned
 * singleton, so `m.f() is True` holds at the boundary rather than merely
 * comparing equal.
 */
/*
 * v0.6: an `int | float` parameter. A `float` crosses as the runtime's
 * tagged float word (inline, or a boxed float past the inline range); an
 * `int` or a `bool` crosses exactly as at an `int` parameter, since the
 * int word is a subset of the `int | float` word. Anything else is a
 * `TypeError` naming both accepted types.
 */
static int pycc_ext_unpack_num(PyObject *obj, const char *fn_name, Py_ssize_t index,
                               long long *out)
{
    PyObject *type_name;

    if (PyFloat_Check(obj)) {
        *out = pycc_rt_num_from_float(PyFloat_AsDouble(obj));
        return 0;
    }
    if (PyLong_Check(obj)) {
        return pycc_ext_unpack_int(obj, fn_name, index, out);
    }
    type_name = PyType_GetName(Py_TYPE(obj));
    if (type_name == NULL) {
        PyErr_SetString(PyExc_TypeError, "object is not an int or a float");
    } else {
        PyErr_Format(PyExc_TypeError, "%s() argument %zd: expected int or float, got '%U'",
                     fn_name, index + 1, type_name);
        Py_DECREF(type_name);
    }
    return -1;
}

/*
 * Packs an `int | float` result, which arrives retained (D-180 rule 6) and
 * dies here: a float word becomes a CPython `float`, anything else is an
 * int word and takes `pycc_ext_pack_int`'s path.
 */
static PyObject *pycc_ext_pack_num(const char *fn_name, long long encoded)
{
    PyObject *value;

    if (!pycc_rt_num_is_float(encoded)) {
        return pycc_ext_pack_int(fn_name, encoded);
    }
    value = PyFloat_FromDouble(pycc_rt_num_to_float(encoded));
    pycc_rt_num_release(encoded);
    return value;
}

static PyObject *pycc_ext_pack_bool(char value)
{
    return PyBool_FromLong(value != 0);
}

/*
 * Unpacks one argument at a `str` parameter. Returns 0, or -1 with a
 * CPython exception set.
 *
 * `PyUnicode_Check` before the converter, and no fallback, for the same
 * reason `pycc_ext_unpack_float` refuses an `int`: D-244 rule 7 defers to
 * `docs/TYPE_SYSTEM.md` rule 4 (D-086), so nothing that merely knows how to
 * become a `str` -- `__str__`, `os.PathLike`, a buffer -- is admitted here.
 * `PyUnicode_Check` does accept a `str` subclass, which is flattened to a
 * plain pycc `str` by the copy below; `docs/RUNTIME.md` records that
 * narrowing.
 *
 * `PyUnicode_AsUTF8AndSize` is in the limited API from `Py_LIMITED_API`
 * 0x030D0000, this shim's floor. It fails on a string holding a lone
 * surrogate, which has no UTF-8 encoding; the resulting `UnicodeEncodeError`
 * is propagated verbatim rather than translated, per the D-244 amendment.
 * The length is carried explicitly, never re-derived with `strlen`, because
 * a Python `str` may contain embedded NUL bytes.
 *
 * An exact `str` is not copied: the native `str` borrows its UTF-8 buffer
 * and holds a reference to it (`pycc_rt_str_adopt_foreign`), which is also
 * the object it shares when it crosses back (`pycc_ext_pack_str`). A
 * subclass instance's bytes are copied by `pycc_rt_str_from_literal`, so
 * the caller need not keep them alive. Either call hands back a fresh
 * reference with refcount 1, which becomes the compiled function's own
 * parameter slot.
 */
static int pycc_ext_unpack_str(PyObject *obj, const char *fn_name, Py_ssize_t index,
                               void **out)
{
    PyObject *type_name;
    const char *utf8;
    Py_ssize_t size;

    if (!PyUnicode_Check(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError, "object cannot be interpreted as a str");
        } else {
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd: '%U' object cannot be interpreted as a str",
                         fn_name, index + 1, type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    utf8 = PyUnicode_AsUTF8AndSize(obj, &size);
    if (utf8 == NULL) {
        return -1;
    }
    if (PyUnicode_CheckExact(obj)) {
        Py_INCREF(obj);
        *out = pycc_rt_str_adopt_foreign((const unsigned char *)utf8, (long long)size, obj);
    } else {
        *out = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);
    }
    return 0;
}

/*
 * Packs a `str`-typed result. No function name, like `pycc_ext_pack_float`:
 * there is no value this has to refuse, and a `PyStrObj` provably cannot
 * hold invalid UTF-8 (every constructor takes already-valid Rust input).
 *
 * The reference is released unconditionally before returning, including on
 * the `PyUnicode_FromStringAndSize` failure path: the compiled function's
 * return hands this boundary a retained `str` (D-180 rule 6, the same
 * ownership `pycc_ext_pack_int`'s bigint arm releases), and the CPython
 * object is an independent copy, so identity does not survive the crossing
 * (#1043).
 *
 * `result` is never NULL on this path -- the generated wrapper checks for a
 * pending pycc exception first, and that branch returns the `NULL` carrier
 * without reaching here -- but the guard costs nothing in C and keeps the
 * accessor's own no-null-arm contract honest.
 */
static PyObject *pycc_ext_pack_str(void *result)
{
    const unsigned char *bytes;
    size_t len = 0;
    PyObject *shared;
    PyObject *packed;

    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "str result was NULL");
        return NULL;
    }
    /* The CPython `str` the result already shares, if any (a parameter
     * returned unchanged, a `str` that crossed before): not copied again. */
    shared = (PyObject *)pycc_rt_str_foreign(result);
    if (shared != NULL) {
        Py_INCREF(shared);
        pycc_rt_str_decref(result);
        return shared;
    }
    bytes = pycc_rt_ext_str_bytes(result, &len);
    packed = PyUnicode_FromStringAndSize((const char *)bytes, (Py_ssize_t)len);
    pycc_rt_str_decref(result);
    return packed;
}

/*
 * Unpacks one argument at a parameter whose type is the opaque CPython
 * object (D-258 rule 5, #1397): an `Any` or `object` annotation, a container
 * of objects, or a foreign-imported class or type variable, which is not
 * checked against the class (D-244's #1386 amendment). Returns
 * 0; there is no type to refuse, because the declared type admits every
 * object -- which is also why `fn_name` and `index` are unused, kept only so
 * every scalar unpack helper shares one call shape.
 *
 * The compiled body receives a *strong* reference: its parameter slot owns
 * it and the body releases it when the frame exits (`heap_rc` in
 * `crates/pycc_codegen`), exactly as a `str` parameter owns the string
 * `pycc_ext_unpack_str` builds. Nothing is owed on a later argument's bail
 * path: the generated wrapper releases the unpacked arguments itself.
 */
static int pycc_ext_unpack_object(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                  void **out)
{
    (void)fn_name;
    (void)index;
    *out = Py_NewRef(obj);
    return 0;
}

/*
 * Packs an object-typed result (D-258 rule 5, #1397): the very `PyObject *`
 * the compiled body returned, so identity survives the round trip
 * (`f(x) is x`). A compiled return hands its caller an owned reference
 * (`heap_rc`), which becomes the reference the CPython calling convention
 * requires of a return value -- the packer consumes it, as
 * `pycc_ext_pack_str` consumes its string. A caller holding a borrowed word
 * (an attribute getter) takes a reference first.
 *
 * `result` is never NULL on this path -- the generated wrapper checks for a
 * pending pycc exception first -- but the guard, as in `pycc_ext_pack_str`,
 * turns a would-be crash into a `SystemError`.
 */
static PyObject *pycc_ext_pack_object(void *result)
{
    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "object result was NULL");
        return NULL;
    }
    return (PyObject *)result;
}

/* Defined later in the translation unit (`object_convert.c`,
 * `pycc_ext_object_ops.c`). */
PyObject *pycc_ext_obj_pack_list(void *list);
int pycc_ext_coerce_str_tuple(PyObject *o, const char *what, void **out);
extern void pycc_rt_list_decref(void *list);

/*
 * A `tuple[str, ...]` parameter (v0.5): a CPython `tuple` of `str`s copied
 * into a native tuple object sharing each string and the tuple itself
 * (`pycc_ext_coerce_str_tuple`), so it crosses back as the same object. The
 * fresh reference becomes the compiled function's parameter slot. Anything
 * else is `TypeError`, naming the argument.
 */
static int pycc_ext_unpack_str_tuple(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                     void **out)
{
    char what[256];

    PyOS_snprintf(what, sizeof what, "%s() argument %zd", fn_name, index + 1);
    return pycc_ext_coerce_str_tuple(obj, what, out);
}

/*
 * A native tuple object result -- a `tuple[str, ...]`, or a fixed-arity or
 * variadic tuple of `str`s, scalars, CPython objects and such tuples (v0.5)
 * -- as the CPython `tuple` it crosses as (`pycc_ext_obj_pack_list`: the
 * same object every time it crosses). Consumes the compiled body's owned
 * reference, as `pycc_ext_pack_str` consumes its string.
 */
static PyObject *pycc_ext_pack_tuple_object(void *result)
{
    PyObject *packed;

    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "tuple result was NULL");
        return NULL;
    }
    packed = pycc_ext_obj_pack_list(result);
    pycc_rt_list_decref(result);
    return packed;
}

/* The `tuple[str, ...]` carrier's egress (`pycc_ext_unpack_str_tuple`). */
static PyObject *pycc_ext_pack_str_tuple(void *result)
{
    return pycc_ext_pack_tuple_object(result);
}

/*
 * The `{ ptr, len }` pair a `memoryview` parameter crosses into compiled
 * code as (Part 1 of #1027, D-244). Declared here rather than derived from
 * `Py_buffer` on purpose, and it is the *whole* of what the compiled body
 * ever sees of an acquired buffer.
 *
 * `len` is a copy of `buf.shape[0]`, the element count -- not `buf.len`,
 * which is a byte count (a 27-element `float64` view reports `len == 216`
 * and `itemsize == 8`). `shape[0]` is what CPython's own `len()` and
 * `IndexError` bound use, so it is the authoritative bound for the indexing
 * Part 2 adds.
 *
 * It is a copy because `Py_buffer.shape` is a `Py_ssize_t *` owned by the
 * *exporter* and valid only until `PyBuffer_Release`. Carrying the pointer
 * would make every generated wrapper's own release a use-after-free for
 * anything that read through it afterwards. `pycc_ext_obj_*`-style opacity
 * applies here too: nothing on the compiled side may reach the `Py_buffer`
 * or the owning `PyObject *` at all, which is what keeps the buffer's
 * lifetime wholly wrapper-owned.
 *
 * `long long` and not `Py_ssize_t` so the field matches the `i64` that
 * `pycc_codegen`'s `ty_to_basic_type` gives every pycc `int`, exactly as
 * `boundary_carrier`'s `Ty::Int` slot does.
 */
typedef struct {
    void *ptr;
    long long len;
} PyccExtBufferView;

/*
 * Part 2b of #1142 (#1164): the release entry point for *artifact-owned*
 * buffer storage, declared here rather than with `pycc_rt`'s other externs at
 * the top of the file because it is the one whose signature names a type this
 * file defines. Its sole C caller is the exporter's `tp_dealloc` below. It is
 * a documented no-op on NULL, and it must never be handed a `memoryview`
 * *parameter*'s view -- that storage belongs to the host's own exporter.
 */
extern void pycc_rt_buffer_f64_free(PyccExtBufferView *view);

/*
 * Unpacks one argument at a buffer parameter -- spelled `memoryview`,
 * `ndarray` or `NDArray` in the source, one pycc type whichever was
 * written. Returns 0 with `*out`
 * holding an acquired buffer the caller must release, or -1 with a CPython
 * exception set and nothing acquired.
 *
 * Four refusals, in this order, and the order is the contract:
 *
 * 1. The object exports no buffer at all -- a pycc-authored `TypeError`,
 *    nothing acquired. `PyObject_CheckBuffer` and not `PyMemoryView_Check`:
 *    #1129 cashed in the widening D-244's Part-1-of-#1027 amendment
 *    statement (a) pre-authorized, so `bytes`, a `bytearray`, an
 *    `array.array` and a bare NumPy array now all pass *this* arm on the
 *    strength of exporting a buffer, and are answered -- or accepted -- by
 *    arms 2-4 on the properties of the buffer they export. This is what
 *    lets a host hand a compiled export a bare `ndarray` without wrapping
 *    it in `memoryview(...)` first, and it widens the `memoryview`
 *    annotation by exactly the same set: every source spelling of the
 *    carrier -- `memoryview`, `ndarray` (#1129) and `NDArray` (#1134) --
 *    lowers to one pycc type and reaches this helper identically.
 * 2. `PyObject_GetBuffer` fails -- the exporter's own exception is
 *    propagated verbatim, because it says more about the operand than a
 *    translated message could, and nothing is acquired when it fails. The
 *    *type* is the exporter's choice and not fixed: a strided `memoryview`
 *    raises CPython's `BufferError: memoryview: underlying buffer is not
 *    C-contiguous`, while a strided NumPy array raises numpy's own
 *    `ValueError: ndarray is not C-contiguous` for the same condition
 *    (D-244's #1129 amendment statement (g)).
 * 3. `ndim != 1` -- released first, then a pycc-authored `TypeError`.
 *    Part 1 admits one dimension only.
 * 4. The element format is not `"d"` -- released first, then a
 *    pycc-authored `TypeError` naming the format seen and the one required.
 *    This is the arm a buffer of the wrong element type takes, e.g. a
 *    `bytes` or `memoryview(b"abc")`, whose format is `'B'`, or a float32
 *    NumPy array, whose format is `'f'`. `itemsize` is checked
 *    alongside the format string rather than instead of it: the format is
 *    the exporter's own claim, and the size is the arithmetic the compiled
 *    code would do.
 *
 * Arms 3 and 4 say "a buffer", not "a memoryview". Arm 1's widening is
 * what makes that accurate rather than cosmetic: the operand that reaches
 * them is any conforming exporter, and a `bytes` or a bare NumPy array is
 * not a `memoryview` by any reading (D-244's #1129 amendment statement
 * (f)). Arm 2 needs no such wording -- it propagates the exporter's own
 * exception and authors no text at all.
 *
 * `PyBUF_C_CONTIGUOUS | PyBUF_FORMAT` is the request. C-contiguity is what
 * makes a plain pointer walk correct at all, and it implies `PyBUF_STRIDES`
 * and so `PyBUF_ND`, which is what makes `ndim` and `shape` populated for
 * arms 3 and 4 to read. `PyBUF_WRITABLE` is added to the request exactly
 * when `writable` is non-zero, which the generator sets for a parameter
 * whose own body stores into it (Part 1 of #1142). A parameter the body
 * only reads keeps the read-only request, so the read-only views this
 * boundary has always accepted are still accepted; a parameter the body
 * writes must not be backed by read-only storage, and CPython's own
 * `BufferError` for that case propagates verbatim -- this shim authors no
 * text for it, because the exporter's message already names the cause
 * better than a pycc-authored one could.
 *
 * Every symbol used here is in the limited API at this shim's
 * `Py_LIMITED_API 0x030D0000` floor.
 */
static int pycc_ext_unpack_memoryview(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                      int writable, Py_buffer *out)
{
    PyObject *type_name;
    const char *declared;
    int ndim;

    if (!PyObject_CheckBuffer(obj)) {
        type_name = PyType_GetName(Py_TYPE(obj));
        if (type_name == NULL) {
            PyErr_SetString(PyExc_TypeError, "object does not export a buffer");
        } else {
            PyErr_Format(PyExc_TypeError,
                         "%s() argument %zd: '%U' object does not export a buffer",
                         fn_name, index + 1, type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    if (PyObject_GetBuffer(obj, out,
                           PyBUF_C_CONTIGUOUS | PyBUF_FORMAT |
                               (writable ? PyBUF_WRITABLE : 0)) != 0) {
        return -1;
    }
    ndim = out->ndim;
    if (ndim != 1) {
        PyBuffer_Release(out);
        PyErr_Format(PyExc_TypeError,
                     "%s() argument %zd: a buffer with ndim %d is not supported yet -- "
                     "only a one-dimensional buffer is",
                     fn_name, index + 1, ndim);
        return -1;
    }
    /*
     * `out->format` points into storage the exporter owns, and the message
     * below names what this boundary was actually handed (D-244 statement
     * (e)). The exporter chooses that string and a `ctypes.Structure`'s is
     * routinely dozens of characters (`T{<d:x:<d:y:}`), so it is *not*
     * copied into a fixed buffer first -- that only ever named a prefix.
     * The message is built while the buffer is still held instead, and the
     * release moved after it: `PyErr_Format` copies the characters it reads
     * as it formats, and it runs no interpreted code that could invalidate
     * the view in between, so the release below cannot leave the exception
     * naming freed storage. The same reasoning already governs the `%U`
     * of the arm above that #1129 widened to `PyObject_CheckBuffer`.
     */
    declared = (out->format == NULL) ? "" : out->format;
    if (strcmp(declared, "d") != 0 || out->itemsize != (Py_ssize_t)sizeof(double)) {
        PyErr_Format(PyExc_TypeError,
                     "%s() argument %zd: a buffer of format '%s' is not supported -- "
                     "only format 'd' (a contiguous float64 buffer) is",
                     fn_name, index + 1, declared);
        PyBuffer_Release(out);
        return -1;
    }
    return 0;
}

