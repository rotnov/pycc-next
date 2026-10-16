/*
 * Part 1 of #1255: the empty result of a list or set comprehension over a
 * CPython object (`EXT_OBJ_NEW_COLLECTION_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). `kind` is `ObjCollectionKind`'s code
 * there: `0` builds a `list`, `1` a `set`; any other code is a code
 * generator defect and raises `SystemError` rather than guessing.
 *
 * The result is a *new* reference -- the comprehension's value -- released
 * by its consumer when unbound and on the comprehension's failure edges
 * (Part 3 of #1092), owned by a slot that binds it, or `NULL` with the
 * exception set.
 */
PyObject *pycc_ext_obj_new_collection(long long kind)
{
    if (kind == 0) {
        return PyList_New(0);
    }
    if (kind == 1) {
        return PySet_New(NULL);
    }
    PyErr_SetString(PyExc_SystemError,
                    "pycc_ext_obj_new_collection called with an unknown kind");
    return NULL;
}

/*
 * Part 1 of #1255: one element step of a list or set comprehension over a
 * CPython object (`EXT_OBJ_COLLECT_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). `collection` is borrowed and was built
 * by `pycc_ext_obj_new_collection` with the same `kind`.
 *
 * `item` is a *packed* element, a new reference a `pycc_ext_obj_pack_*`
 * helper produced, and it is consumed on every path, exactly as
 * `pycc_ext_obj_call` consumes its arguments. A `NULL` item is a packer that
 * already set the exception, so it is reported as a failure without a new
 * one. `PyList_Append` and `PySet_Add` take their own reference, so the
 * packed one is released after either; `PySet_Add` raises `TypeError` for an
 * unhashable item, which is exactly what CPython's own set comprehension
 * raises.
 *
 * Returns `0`, or `-1` with the exception set. A `NULL` collection or an
 * unknown kind is a code generator defect and raises `SystemError`, as the
 * other NULL guards in this file do.
 */
int pycc_ext_obj_collect(PyObject *collection, long long kind, PyObject *item)
{
    int status;

    if (item == NULL) {
        return -1;
    }
    if (collection == NULL || (kind != 0 && kind != 1)) {
        Py_DECREF(item);
        PyErr_SetString(PyExc_SystemError,
                        "pycc_ext_obj_collect called with an invalid collection");
        return -1;
    }
    status = kind == 0 ? PyList_Append(collection, item) : PySet_Add(collection, item);
    Py_DECREF(item);
    return status < 0 ? -1 : 0;
}

/*
 * Part 4 of #1026 (PR 4a of #1083): `float(o)` on a CPython object value
 * (`EXT_OBJ_TO_FLOAT_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Writes the converted C double through `*out` and answers `0`, or answers
 * `-1` with a CPython exception already set.
 *
 * # Why running CPython's own conversion protocol here is not a D-244
 * # rule-7 violation
 *
 * D-244 rule 7 keeps the type boundary *closed at the thunk export seam*:
 * where a value crosses implicitly, the annotation is the whole contract,
 * so no conversion protocol may run behind the author's back. This helper
 * is on the other side of that distinction. `float(o)` in user source is an
 * *explicit conversion request*: the author wrote the destination type, so
 * running `PyNumber_Float` -- the operand's own `__float__`, `__index__` or
 * string parse -- is exactly what they asked for, not an implicit crossing.
 * The same paragraph is recorded in `docs/TYPE_SYSTEM.md`'s `object` row.
 *
 * # Ownership
 *
 * `PyNumber_Float` hands back a *new* reference, and this helper releases it
 * on every exit, not only the successful one. Part 4's conversions therefore
 * add nothing to the #1092 leak-only set: the converted value is a pycc-side
 * `double` and no CPython reference escapes into compiled code.
 *
 * `PyFloat_AsDouble` on the result of `PyNumber_Float` cannot itself fail --
 * that result is a `float` by construction -- but its `-1.0`-plus-
 * `PyErr_Occurred()` convention is still tested, as defence in depth and so
 * that the ownership rule above holds on a path that is meant to be
 * unreachable.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
int pycc_ext_obj_to_float(PyObject *o, double *out)
{
    PyObject *converted;
    double value;

    if (o == NULL || out == NULL) {
        return -1;
    }
    converted = PyNumber_Float(o);
    if (converted == NULL) {
        return -1;
    }
    value = PyFloat_AsDouble(converted);
    Py_DECREF(converted);
    if (value == -1.0 && PyErr_Occurred()) {
        return -1;
    }
    *out = value;
    return 0;
}

/*
 * Part 4 of #1026 (PR 4b of #1083): `int(o)` on a CPython object value
 * (`EXT_OBJ_TO_INT_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Writes a D-141 *encoded* integer word through `*out` and answers `0`, or
 * answers `-1` with a CPython exception already set.
 *
 * The rule-7 paragraph on `pycc_ext_obj_to_float` above governs this helper
 * too: an explicit `int(o)` in user source names its destination type, so
 * running CPython's own `PyNumber_Long` protocol -- the operand's
 * `__int__`/`__index__` or a string parse -- is what the author asked for,
 * not the implicit thunk-seam crossing D-244 rule 7 closes.
 *
 * # Why this is not `pycc_ext_unpack_int_at`
 *
 * That helper guards `PyBool_Check` and `PyLong_Check` before converting,
 * because rule 7 closes the *thunk* boundary: a `bool` must keep its D-141
 * marker word across an int-shaped parameter slot, and an `__index__` duck
 * type is not an `int`. Neither applies to an explicit conversion, where
 * CPython's own answers -- `int(True)` is `1`, `int(x)` honours
 * `__index__` -- are precisely the contract. Only the encoding tail is
 * shared: `pycc_ext_long_encode` accepts any magnitude (#1040).
 *
 * # Ownership
 *
 * `PyNumber_Long` hands back a *new* reference, released on *every* exit --
 * including the `OverflowError` path, which still holds it at the point it
 * decides to fail. Nothing but an encoded word escapes into compiled code,
 * so this conversion adds nothing to the #1092 leak-only set.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
int pycc_ext_obj_to_int(PyObject *o, long long *out)
{
    PyObject *converted;
    int status;

    if (o == NULL || out == NULL) {
        return -1;
    }
    converted = PyNumber_Long(o);
    if (converted == NULL) {
        return -1;
    }
    status = pycc_ext_long_encode(converted, out);
    Py_DECREF(converted);
    return status;
}

/*
 * Part 4 of #1026 (PR 4b of #1083): `str(o)` on a CPython object value
 * (`EXT_OBJ_TO_STR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Writes a pycc `PyStrObj *` (opaque `void *` on this side, exactly as in
 * `pycc_ext_unpack_str`) through `*out` and answers `0`, or answers `-1`
 * with a CPython exception already set. The handle arrives at refcount 1
 * and becomes the compiled code's own, which is the identical ownership a
 * `str` literal's own `pycc_rt_str_from_literal` produces -- so codegen
 * needs no new rule for it.
 *
 * The rule-7 paragraph on `pycc_ext_obj_to_float` above governs this helper
 * too; `PyObject_Str` *is* `str()`, so there is no other defensible answer.
 * Unlike `pycc_ext_unpack_str` this does not `PyUnicode_Check` its operand:
 * refusing a non-`str` is exactly what an explicit conversion must not do.
 * A `__str__` that raises, and a result holding a lone surrogate (which
 * `PyUnicode_AsUTF8AndSize` refuses with `UnicodeEncodeError`), are both
 * propagated verbatim. The length is carried explicitly, never re-derived
 * with `strlen`, because a Python `str` may contain embedded NUL bytes.
 *
 * # Ownership -- the copy must complete before the release
 *
 * `PyObject_Str` hands back a *new* reference, and `PyUnicode_AsUTF8AndSize`
 * returns a pointer *into that object's own buffer*. So
 * `pycc_rt_str_from_literal`, which copies those bytes, has to run *before*
 * the `Py_DECREF`; releasing first and copying after is a use-after-free.
 * That ordering is load-bearing and is why this helper is not a copy of
 * `pycc_ext_unpack_str`, which merely borrows its caller's argument and may
 * order the two calls freely. The reference is released on every exit,
 * including the `PyUnicode_AsUTF8AndSize` failure path, so nothing here
 * joins the #1092 leak-only set.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
static int pycc_ext_obj_text(PyObject *o, void **out, int repr);

int pycc_ext_obj_to_str(PyObject *o, void **out)
{
    return pycc_ext_obj_text(o, out, 0);
}

/* `repr(o)`: `pycc_ext_obj_to_str`'s contract with `PyObject_Repr`. */
int pycc_ext_obj_repr(PyObject *o, void **out)
{
    return pycc_ext_obj_text(o, out, 1);
}

/*
 * `pycc_rt`'s object repr hook (`pycc_rt_object_repr_install`): `repr(o)`
 * as a new native `str`, or NULL with the CPython error bridged into the
 * pending exception, so the native tuple's rendering stops and the
 * compiled caller takes its exception edge.
 */
extern int pycc_ext_obj_error_bridge(void);
void *pycc_ext_obj_repr_hook(void *o)
{
    void *out = NULL;
    if (pycc_ext_obj_repr((PyObject *)o, &out) != 0) {
        pycc_ext_obj_error_bridge();
        return NULL;
    }
    return out;
}

static int pycc_ext_obj_text(PyObject *o, void **out, int repr)
{
    PyObject *converted;
    const char *utf8;
    Py_ssize_t size;
    void *copied;

    if (o == NULL || out == NULL) {
        return -1;
    }
    converted = repr ? PyObject_Repr(o) : PyObject_Str(o);
    if (converted == NULL) {
        return -1;
    }
    utf8 = PyUnicode_AsUTF8AndSize(converted, &size);
    if (utf8 == NULL) {
        Py_DECREF(converted);
        return -1;
    }
    copied = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);
    Py_DECREF(converted);
    *out = copied;
    return 0;
}

/*
 * #1476 (Part 3 of #1387): the read of an `object` name an
 * `isinstance(o, int|float|bool|str)` guard narrowed back to a native type
 * (`EXT_OBJ_UNBOX_*_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Each helper writes the native value through `*out` and answers `0`, or
 * answers `-1` with a CPython exception set. Unlike the explicit
 * conversions above, the narrowed read is an *implicit* crossing, so each
 * helper keeps the closed D-244 rule-7 check of the matching thunk-seam
 * unpacker (`pycc_ext_unpack_int_at`, `_float_at`, `_bool_at`, `_str`)
 * rather than CPython's conversion protocol, and a `bool` under an `int`
 * guard keeps its D-141 marker word. The guard is CPython's own
 * `PyObject_IsInstance`, which also consults a `__class__` attribute: an
 * object whose `__class__` property answers `int` (a mock, a proxy) passes
 * the guard without being a `PyLong`, has no native representation to
 * hand over, and is refused here with a `TypeError` where CPython would run
 * the guarded body on it -- a deliberate, loud deviation (#1476, pinned by
 * `tests/issue_1476_isinstance_narrowing.rs`). The thunk-seam unpackers
 * themselves stay unchanged, because their messages name an argument
 * position this read has none of.
 *
 * An `int` of any magnitude is accepted (`pycc_ext_long_encode`). A `str` (or `str` subclass) is copied into a fresh pycc
 * `PyStrObj` at refcount 1, the ownership `pycc_ext_obj_to_str` documents;
 * the copy completes while the caller still holds `o`. Nothing here takes a
 * CPython reference, so nothing joins the #1092 leak-only set.
 */
static int pycc_ext_obj_unbox_refuse(PyObject *o, const char *wanted)
{
    PyObject *type_name = PyType_GetName(Py_TYPE(o));

    if (type_name == NULL) {
        return -1;
    }
    PyErr_Format(PyExc_TypeError,
                 "isinstance() held for a '%U' object, but only %s has the native "
                 "representation the narrowed read needs",
                 type_name, wanted);
    Py_DECREF(type_name);
    return -1;
}

int pycc_ext_obj_unbox_int(PyObject *o, long long *out)
{
    if (o == NULL || out == NULL) {
        return -1;
    }
    if (PyBool_Check(o)) {
        *out = pycc_rt_ext_bool_encode(o == Py_True);
        return 0;
    }
    if (!PyLong_Check(o)) {
        return pycc_ext_obj_unbox_refuse(o, "an int");
    }
    return pycc_ext_long_encode(o, out);
}

int pycc_ext_obj_unbox_float(PyObject *o, double *out)
{
    if (o == NULL || out == NULL) {
        return -1;
    }
    if (!PyFloat_Check(o)) {
        return pycc_ext_obj_unbox_refuse(o, "a float");
    }
    *out = PyFloat_AsDouble(o);
    return 0;
}

int pycc_ext_obj_unbox_bool(PyObject *o, char *out)
{
    if (o == NULL || out == NULL) {
        return -1;
    }
    if (!PyBool_Check(o)) {
        return pycc_ext_obj_unbox_refuse(o, "a bool");
    }
    *out = (char)(o == Py_True);
    return 0;
}

int pycc_ext_obj_unbox_str(PyObject *o, void **out)
{
    const char *utf8;
    Py_ssize_t size;

    if (o == NULL || out == NULL) {
        return -1;
    }
    if (!PyUnicode_Check(o)) {
        return pycc_ext_obj_unbox_refuse(o, "a str");
    }
    utf8 = PyUnicode_AsUTF8AndSize(o, &size);
    if (utf8 == NULL) {
        return -1;
    }
    *out = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);
    return 0;
}

/*
 * #1340: an f-string interpolation `f"{o}"` of a CPython object
 * (`EXT_OBJ_FORMAT_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `pycc_ext_obj_to_str`'s exact contract and shape, with
 * `PyObject_Format(o, NULL)` in place of `PyObject_Str(o)`. CPython renders
 * an f-string's `{value}` as `format(value, '')`, which calls the value's
 * `__format__` rather than its `__str__`; the two differ whenever a class
 * overrides `__format__`, so an interpolation must not reuse the `str()`
 * helper. A NULL spec is the empty spec, exactly as in CPython's own
 * `FORMAT_VALUE` path.
 *
 * # Ownership -- the copy must complete before the release
 *
 * The paragraph on `pycc_ext_obj_to_str` above, unchanged:
 * `PyObject_Format` hands back a new reference whose buffer
 * `PyUnicode_AsUTF8AndSize` points into, so `pycc_rt_str_from_literal` runs
 * before the `Py_DECREF`, and the reference is released on every exit.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
int pycc_ext_obj_format(PyObject *o, void **out)
{
    PyObject *formatted;
    const char *utf8;
    Py_ssize_t size;
    void *copied;

    if (o == NULL || out == NULL) {
        return -1;
    }
    formatted = PyObject_Format(o, NULL);
    if (formatted == NULL) {
        return -1;
    }
    utf8 = PyUnicode_AsUTF8AndSize(formatted, &size);
    if (utf8 == NULL) {
        Py_DECREF(formatted);
        return -1;
    }
    copied = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);
    Py_DECREF(formatted);
    *out = copied;
    return 0;
}

/*
 * Part 4 of #1026 (PR 4c of #1083): unpacking a CPython object into a
 * fixed-arity all-`float` `tuple` annotation at a module-level annotated
 * assignment (`EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 *
 * Writes `arity` C doubles through `out[0..arity)` and answers `0`, or
 * answers `-1` with a CPython exception already set.
 *
 * `arity` is a parameter rather than a compile-time constant on purpose:
 * the admission rule is "any fixed arity, every element `float`", so
 * nothing on either side of this seam may hard-code the three of
 * `tuple[float, float, float]`.
 *
 * # Strict container, converting elements
 *
 * The *container* is checked and the *elements* are converted, and the two
 * halves answer two different questions.
 *
 * `PyTuple_Check` with an exact-arity test is the same closed-boundary
 * reading `pycc_ext_unpack_tuple` documents: D-116 fixes a tuple type's
 * arity, pycc holds the result as a by-value LLVM struct of that exact
 * width (D-115), and there is no shape to write a shorter or longer
 * sequence into. `PyTuple_Check` and not `PyTuple_CheckExact` for that
 * helper's reason too -- a `tuple` subclass *is* a tuple, and the elements
 * are copied out by value so the subclass identity does not survive the
 * crossing. A `list`, a generator or any other iterable is refused: this is
 * an assignment to a declared `tuple`, not an unpacking protocol.
 *
 * The *elements*, by contrast, run `PyNumber_Float` -- the operand's own
 * `__float__`, `__index__` or string parse -- which is the rule-7 paragraph
 * on `pycc_ext_obj_to_float` above, unchanged: the author wrote `float` in
 * the annotation, so CPython's own conversion to `float` is what they asked
 * for. It is therefore deliberately *not* `pycc_ext_unpack_float_at`, whose
 * `PyFloat_Check` refusal exists because the *thunk export seam* is closed.
 *
 * # Ownership
 *
 * Each `PyNumber_Float` temporary is released inside the same loop
 * iteration that produced it, before the next one is created. So the
 * failing exit holds nothing: an element that fails to convert leaves the
 * previous iterations' temporaries already released and produces no
 * temporary of its own. Nothing but doubles escapes into compiled code, and
 * this helper adds nothing to the #1092 leak-only set -- Part 4's property,
 * unchanged.
 *
 * `PyTuple_GetItem` returns a *borrowed* reference and is infallible here,
 * because the exact-arity check above has already run --
 * `pycc_ext_unpack_tuple` records that the arity gate exists precisely to
 * make the later `GetItem`s infallible, and the same gate serves the same
 * purpose here.
 *
 * `PyFloat_AsDouble` on the result of `PyNumber_Float` cannot itself fail,
 * that result being a `float` by construction; its `-1.0`-plus-
 * `PyErr_Occurred()` convention is still tested for the reason
 * `pycc_ext_obj_to_float` records, as defence in depth and so that the
 * ownership rule above holds on a path that is meant to be unreachable.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 * `arity` is guarded with it: codegen only ever emits a call for an arity
 * of at least one (the type-checker's own admission rule requires a
 * non-empty tuple), so a non-positive arity can only mean a corrupted call.
 */
int pycc_ext_obj_unpack_float_tuple(PyObject *o, long long arity, double *out)
{
    PyObject *type_name;
    PyObject *converted;
    Py_ssize_t size;
    Py_ssize_t index;
    double value;

    if (o == NULL || out == NULL || arity < 1) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc_ext_obj_unpack_float_tuple called with an invalid argument");
        return -1;
    }
    if (!PyTuple_Check(o)) {
        type_name = PyType_GetName(Py_TYPE(o));
        if (type_name == NULL) {
            PyErr_Format(PyExc_TypeError,
                         "expected a tuple of %zd floats, got a non-tuple object",
                         (Py_ssize_t)arity);
        } else {
            PyErr_Format(PyExc_TypeError,
                         "expected a tuple of %zd floats, got a '%U' object",
                         (Py_ssize_t)arity, type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    size = PyTuple_Size(o);
    if (size != (Py_ssize_t)arity) {
        PyErr_Format(PyExc_TypeError,
                     "expected a tuple of %zd floats, got a tuple of length %zd",
                     (Py_ssize_t)arity, size);
        return -1;
    }
    for (index = 0; index < size; index++) {
        converted = PyNumber_Float(PyTuple_GetItem(o, index));
        if (converted == NULL) {
            return -1;
        }
        value = PyFloat_AsDouble(converted);
        Py_DECREF(converted);
        if (value == -1.0 && PyErr_Occurred()) {
            return -1;
        }
        out[index] = value;
    }
    return 0;
}

/*
 * The `tp_name` CPython's "cannot unpack non-iterable" message prints, as a
 * new `str` reference, or NULL with an exception set. A static type's
 * `tp_name` is "module.name", from which CPython derives both `__module__`
 * and `__name__`, so it is rebuilt from them, a `builtins` type printing
 * its bare name (`int`, `builtin_function_or_method`); a heap type created
 * by a `class` statement has its `__name__` as `tp_name`. A heap type built
 * from a `PyType_Spec` whose spec name is dotted prints `__name__` here and
 * the dotted name in CPython: the Limited API cannot tell the two heap kinds
 * apart.
 */
static PyObject *pycc_ext_obj_unpack_type_name(PyTypeObject *type)
{
    PyObject *name;
    PyObject *module;
    PyObject *dotted;

    name = PyType_GetName(type);
    if (name == NULL || (PyType_GetFlags(type) & Py_TPFLAGS_HEAPTYPE) != 0) {
        return name;
    }
    module = PyType_GetModuleName(type);
    if (module == NULL) {
        Py_DECREF(name);
        return NULL;
    }
    if (PyUnicode_CompareWithASCIIString(module, "builtins") == 0) {
        Py_DECREF(module);
        return name;
    }
    dotted = PyUnicode_FromFormat("%U.%U", module, name);
    Py_DECREF(module);
    Py_DECREF(name);
    return dotted;
}

/*
 * `t1, ..., tn = o` where `o` is a CPython object (Part 1 of #891,
 * `EXT_OBJ_UNPACK_SYMBOL` in `crates/pycc_codegen/src/ext.rs`): a new
 * reference to a `tuple` of exactly `n` items taken from `o`, or NULL with
 * CPython's own exception set.
 *
 * It mirrors CPython's `unpack_iterable` (Python/ceval.c), which the
 * Limited API does not export:
 *
 * - `iter(o)` failing with `TypeError` for an object that has neither
 *   `__iter__` nor the sequence protocol is replaced by CPython's own
 *   "cannot unpack non-iterable T object"; any other failure of `iter()`
 *   (a raising `__iter__`, say) propagates unchanged. CPython prints the
 *   type's `tp_name`, which the Limited API does not expose;
 *   `pycc_ext_obj_unpack_type_name` rebuilds it (CPython's `%.200s`
 *   truncation of a longer name is not mimicked);
 * - fewer than `n` items is `ValueError` "not enough values to unpack
 *   (expected n, got i)";
 * - an `n + 1`-th item is `ValueError` "too many values to unpack
 *   (expected n)". From CPython 3.14 the interpreter appends ", got K" when
 *   `o` is an exact `list`, `tuple` or `dict`, whose size it can read
 *   without consuming anything; this helper does the same when the running
 *   interpreter is 3.14 or newer (`Py_Version`), so the message matches the
 *   host's own byte for byte;
 * - an exception raised by `__next__` propagates unchanged.
 *
 * # Ownership
 *
 * An exact `tuple` of exactly `n` items -- the common case, and lark's
 * parse-table entries -- is returned itself with one new reference: a
 * tuple is immutable, so the items read back from it are the ones
 * iteration would have produced, and no allocation is made. Every other
 * value is iterated into a fresh tuple. The iterator and every item
 * fetched so far are released on each failing exit; on success the
 * iterator is released and the items are owned by the returned tuple. The
 * extra item fetched to detect "too many" is released at once. The
 * returned reference is bound to the unpacking temporary, a slot that owns
 * it. `PyTuple_New` plus `PyTuple_SetItem` (which steals) build the
 * fresh tuple, both in the Limited API.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents;
 * `n` is guarded with it, codegen only ever emitting a positive arity.
 */
PyObject *pycc_ext_obj_unpack(PyObject *o, long long n)
{
    PyObject *iter;
    PyObject *items;
    PyObject *item;
    PyObject *type_name;
    Py_ssize_t index;
    Py_ssize_t size;

    if (o == NULL || n < 1) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_unpack called with an invalid argument");
        return NULL;
    }
    if (PyTuple_CheckExact(o) && PyTuple_Size(o) == (Py_ssize_t)n) {
        return Py_NewRef(o);
    }
    iter = PyObject_GetIter(o);
    if (iter == NULL) {
        if (PyErr_ExceptionMatches(PyExc_TypeError)
            && PyType_GetSlot(Py_TYPE(o), Py_tp_iter) == NULL && !PySequence_Check(o)) {
            type_name = pycc_ext_obj_unpack_type_name(Py_TYPE(o));
            if (type_name != NULL) {
                PyErr_Format(PyExc_TypeError, "cannot unpack non-iterable %U object", type_name);
                Py_DECREF(type_name);
            }
        }
        return NULL;
    }
    items = PyTuple_New((Py_ssize_t)n);
    if (items == NULL) {
        Py_DECREF(iter);
        return NULL;
    }
    for (index = 0; index < (Py_ssize_t)n; index++) {
        item = PyIter_Next(iter);
        if (item == NULL) {
            if (!PyErr_Occurred()) {
                PyErr_Format(PyExc_ValueError,
                             "not enough values to unpack (expected %zd, got %zd)",
                             (Py_ssize_t)n, index);
            }
            Py_DECREF(items);
            Py_DECREF(iter);
            return NULL;
        }
        PyTuple_SetItem(items, index, item);
    }
    item = PyIter_Next(iter);
    Py_DECREF(iter);
    if (item != NULL) {
        Py_DECREF(item);
        size = -1;
        if (Py_Version >= 0x030E0000) {
            if (PyList_CheckExact(o)) {
                size = PyList_Size(o);
            } else if (PyTuple_CheckExact(o)) {
                size = PyTuple_Size(o);
            } else if (PyDict_CheckExact(o)) {
                size = PyDict_Size(o);
            }
        }
        if (size >= 0) {
            PyErr_Format(PyExc_ValueError,
                         "too many values to unpack (expected %zd, got %zd)", (Py_ssize_t)n,
                         size);
        } else {
            PyErr_Format(PyExc_ValueError, "too many values to unpack (expected %zd)",
                         (Py_ssize_t)n);
        }
        Py_DECREF(items);
        return NULL;
    }
    if (PyErr_Occurred()) {
        Py_DECREF(items);
        return NULL;
    }
    return items;
}

/*
 * The host-side carrier for a compiled instance (#1145): one CPython object
 * per `mod.Class(...)`, holding nothing but the opaque `PyInstanceObj *`
 * that every compiled method of that class takes as its receiver.
 *
 * Every constructible class's generated `PyType_Spec` sets
 * `basicsize = sizeof(PyccExtInstance)` and uses `PyType_GenericNew`, which
 * zeroes the allocation -- so `inst` is NULL until `tp_init` stores one, and
 * `mod.Class.__new__(mod.Class)` (which never runs `tp_init`) yields a
 * carrier whose NULL every generated wrapper checks before dereferencing.
 *
 * `PyObject_HEAD` is available under `Py_LIMITED_API` and is what makes this
 * a well-formed object layout without naming any field of `PyObject`.
 */
typedef struct {
    PyObject_HEAD
    void *inst;
} PyccExtInstance;

/*
 * Non-strict mode: a fresh native container copied into the CPython one
 * (`EXT_OBJ_PACK_LIST_SYMBOL` in `crates/pycc_codegen/src/ext.rs`). The
 * front end admits this only for a value nothing else holds yet -- a
 * `list(...)`/`sorted(...)` result, a `str.split()`, a comprehension or
 * display of such values -- so a copy cannot break aliasing.
 *
 * `list` is a pycc `PyListObj` (a list or a tuple object), borrowed. Each
 * element is packed by its runtime kind (`LIST_KIND_*` in
 * `crates/pycc_rt/src/list.rs`): an `int`, `float` (bit pattern), `bool`,
 * `str`, a compiled instance (its carrier, so its identity survives), or a
 * nested list or tuple, copied the same way. A tuple object becomes a
 * CPython `tuple`, a list a `list`.
 *
 * Returns a new reference, or `NULL` with the exception set.
 */
extern long long pycc_rt_list_len(void *list);
extern long long pycc_rt_list_get(void *list, long long index);
extern long long pycc_rt_list_elem_kind(void *list, long long index);
extern long long pycc_rt_list_is_tuple(void *list);
PyObject *pycc_ext_obj_pack_instance(void *inst);

PyObject *pycc_ext_obj_pack_list(void *list);
extern void *pycc_rt_tuple_foreign(void *tuple);
extern void pycc_rt_tuple_set_foreign(void *tuple, void *object);

/*
 * A native frozenset of `int`, `float`, `bool` or `str` copied into a CPython
 * `frozenset` (`EXT_OBJ_PACK_FROZENSET_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`): its elements, in the set's order,
 * packed as `pycc_ext_obj_pack_list` packs a list's. The front end admits
 * only a frozenset, which no one can mutate, so the copy is
 * indistinguishable from the value.
 *
 * `set` is a pycc `PySetObj`, borrowed. Returns a new reference, or `NULL`
 * with the exception set.
 */
extern void *pycc_rt_set_list(void *set);
extern void pycc_rt_list_decref(void *list);

PyObject *pycc_ext_obj_pack_frozenset(void *set)
{
    void *items;
    PyObject *list;
    PyObject *out;
    PyObject *shared;

    if (set == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_pack_frozenset called with NULL");
        return NULL;
    }
    /* The CPython `frozenset` this one already shares, if any: the
     * elements are not copied again (`PySetObj::foreign`). */
    shared = (PyObject *)pycc_rt_set_foreign(set);
    if (shared != NULL) {
        Py_INCREF(shared);
        return shared;
    }
    items = pycc_rt_set_list(set);
    list = pycc_ext_obj_pack_list(items);
    pycc_rt_list_decref(items);
    if (list == NULL) {
        return NULL;
    }
    out = PyFrozenSet_New(list);
    Py_DECREF(list);
    if (out != NULL) {
        Py_INCREF(out);
        pycc_rt_set_set_foreign(set, out);
    }
    return out;
}

/*
 * One container word of element kind `kind` (`pycc_rt::list::LIST_KIND_*`)
 * packed as a new `PyObject *` reference, or `NULL` with the exception set:
 * a scalar is converted, a `str` shares its CPython `str`, a list or tuple
 * is copied (`pycc_ext_obj_pack_list`), and a CPython object element
 * (kind 11) is the object itself. The word is borrowed.
 */
static PyObject *pycc_ext_pack_word(long long kind, long long word)
{
    PyObject *object;
    double value;

    switch (kind) {
    case 0:
        return pycc_ext_obj_pack_int(word);
    case 1:
        memcpy(&value, &word, sizeof value);
        return pycc_ext_obj_pack_float(value);
    case 2:
        return pycc_ext_obj_pack_bool((char)(word != 0));
    case 3:
        return pycc_ext_obj_pack_str((void *)(intptr_t)word);
    case 4:
        return pycc_ext_obj_pack_instance((void *)(intptr_t)word);
    case 5:
    case 7:
        return pycc_ext_obj_pack_list((void *)(intptr_t)word);
    case 11:
        object = (PyObject *)(intptr_t)word;
        Py_INCREF(object);
        return object;
    default:
        PyErr_SetString(PyExc_SystemError,
                        "pycc_ext_obj_pack_list met an element kind it cannot copy");
        return NULL;
    }
}

PyObject *pycc_ext_obj_pack_list(void *list)
{
    long long len;
    long long i;
    int is_tuple;
    PyObject *out;

    if (list == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_pack_list called with NULL");
        return NULL;
    }
    len = pycc_rt_list_len(list);
    is_tuple = pycc_rt_list_is_tuple(list) != 0;
    /* A tuple is immutable: the CPython `tuple` made for it the first time
     * (or the one it was copied from) is the one it crosses as every time,
     * so `f(t) is g(t)` holds for a native `t` as it does in CPython. */
    if (is_tuple) {
        out = (PyObject *)pycc_rt_tuple_foreign(list);
        if (out != NULL) {
            Py_INCREF(out);
            return out;
        }
    }
    out = is_tuple ? PyTuple_New((Py_ssize_t)len) : PyList_New((Py_ssize_t)len);
    if (out == NULL) {
        return NULL;
    }
    for (i = 0; i < len; i++) {
        long long word = pycc_rt_list_get(list, i);
        PyObject *item;

        item = pycc_ext_pack_word(pycc_rt_list_elem_kind(list, i), word);
        if (item == NULL) {
            Py_DECREF(out);
            return NULL;
        }
        if (is_tuple) {
            PyTuple_SetItem(out, (Py_ssize_t)i, item);
        } else {
            PyList_SetItem(out, (Py_ssize_t)i, item);
        }
    }
    if (is_tuple) {
        Py_INCREF(out);
        pycc_rt_tuple_set_foreign(list, out);
    }
    return out;
}

extern void *pycc_rt_tuple_new(long long len, const unsigned char *kinds);
extern void pycc_rt_tuple_set_word(void *tuple, long long index, long long word);
extern void pycc_rt_word_retain(long long kind, long long word);
extern void pycc_rt_list_decref(void *list);

/*
 * v0.5 (`pycc_codegen`'s `tuple_rebox`): a new tuple object of `n`
 * elements of the kinds `kinds`, from the tuple object `tuple` of the same
 * length. An element whose target kind is the object one (11) and whose
 * own kind is not is packed (`pycc_ext_pack_word`); every other word is
 * shared, with a new reference. `tuple` is borrowed. Returns the new tuple,
 * or `NULL` with the exception set when a packer fails.
 */
void *pycc_ext_obj_tuple_rebox(void *tuple, const unsigned char *kinds, long long n)
{
    void *out;
    long long i;

    if (tuple == NULL || pycc_rt_list_len(tuple) != n) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_tuple_rebox called with a bad tuple");
        return NULL;
    }
    out = pycc_rt_tuple_new(n, kinds);
    for (i = 0; i < n; i++) {
        long long word = pycc_rt_list_get(tuple, i);
        long long kind = pycc_rt_list_elem_kind(tuple, i);

        if (kinds[i] == 11 && kind != 11) {
            PyObject *item = pycc_ext_pack_word(kind, word);
            if (item == NULL) {
                /* Words not yet written are zero, which releasing skips. */
                pycc_rt_list_decref(out);
                return NULL;
            }
            word = (long long)(intptr_t)item;
        } else {
            pycc_rt_word_retain(kind, word);
        }
        pycc_rt_tuple_set_word(out, i, word);
    }
    return out;
}

