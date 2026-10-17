/*
 * Part 2 of #1026: the attribute-load helper compiled code calls for
 * `numpy.pi` on a value whose static type is the opaque `object`.
 *
 * As thin as its neighbour above, and for the same reason: every decision
 * about the lookup -- the descriptor protocol, `__getattr__`, the
 * `AttributeError` a missing name raises -- stays with CPython. It returns a
 * new reference to the attribute's value, or NULL with the CPython
 * exception already set.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_GETATTR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `obj` is borrowed -- the caller holds the reference for at least the
 * duration of this call: a module global or slot, or a produced temporary
 * the caller releases only after this call (Part 1 of #1092). The returned
 * reference is released by compiled code through `pycc_ext_obj_release`
 * when it is consumed unbound, and is owned by any slot that binds it
 * (`crates/pycc_codegen/src/heap_rc.rs`).
 *
 * Deliberately *not* translated into `pycc_rt`'s pending-exception state
 * here. The two failure protocols are kept apart; the caller's own contract
 * (`crates/pycc_codegen/src/foreign_attr.rs`) states which side owns the
 * transition, and why Part 2a does not need it: it tests this function's
 * result for NULL and returns `-1` from the `Py_mod_exec` slot with
 * CPython's exception left exactly as set here.
 *
 * A NULL `obj` is still decided here. `a.b.c` lowers to nested
 * `ObjAttrGet` nodes, and although the caller's own NULL check now stops an
 * inner failure before the outer call is reached, this guard is what makes
 * that a defence in depth rather than the only line: `PyObject_GetAttrString`
 * dereferences `Py_TYPE(obj)` with no guard of its own, so any caller that
 * reaches here with NULL would crash the hosting interpreter instead of
 * raising. Returning NULL unchanged is also the *correct* CPython state --
 * the inner lookup already set its `AttributeError`, so propagating NULL
 * leaves exactly one exception set. Overwriting it with a second, synthetic
 * error would be worse.
 */
/*
 * The interned `str` of an attribute name the compiled code spells, by the
 * spelling's address, so a lookup in a loop neither decodes nor hashes the
 * name again. Every spelling is a constant of the generated code (callers
 * are `pycc_ext_obj_getattr` and `pycc_ext_obj_call_method`, whose name
 * arguments codegen emits as private string constants), which lives as long
 * as the module and is never reused for another text: one address names one
 * text, so a hit is decided by the address alone. A slot holds one name; a
 * colliding name replaces it. Borrowed; NULL with an exception set when the
 * name cannot be created.
 */
#define PYCC_EXT_NAME_SLOTS 256
static struct {
    const char *spelling;
    PyObject *name;
} pycc_ext_names[PYCC_EXT_NAME_SLOTS];

static PyObject *pycc_ext_attr_name(const char *spelling)
{
    /* Constants are byte-aligned and often adjacent: mix every address bit
     * into the slot index (Fibonacci hashing). */
    size_t slot = (size_t)(((uint64_t)(uintptr_t)spelling * 0x9E3779B97F4A7C15ull) >> 56)
                  & (PYCC_EXT_NAME_SLOTS - 1);
    PyObject *name;

    if (pycc_ext_names[slot].spelling == spelling) {
        return pycc_ext_names[slot].name;
    }
    name = PyUnicode_InternFromString(spelling);
    if (name == NULL) {
        return NULL;
    }
    Py_XDECREF(pycc_ext_names[slot].name);
    pycc_ext_names[slot].spelling = spelling;
    pycc_ext_names[slot].name = name;
    return name;
}

PyObject *pycc_ext_obj_getattr(PyObject *obj, const char *name)
{
    PyObject *attr_name;

    if (obj == NULL) {
        return NULL;
    }
    attr_name = pycc_ext_attr_name(name);
    if (attr_name == NULL) {
        return NULL;
    }
    return PyObject_GetAttr(obj, attr_name);
}

/*
 * `self.name(args)` as one call (`EXT_OBJ_CALL_METHOD_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`): `PyObject_VectorcallMethod`, which
 * finds a method on the type and calls it with `self` without making a
 * bound method -- what CPython's own `LOAD_ATTR`/`CALL` pair does -- and
 * otherwise calls whatever attribute the lookup finds. The lookup runs
 * after the arguments were evaluated, so compiled code uses this only when
 * evaluating them can neither raise nor run code (literals and parameter
 * reads), where the two orders cannot be told apart.
 *
 * `self` is borrowed; every `args[i]` is consumed on every path, as for
 * `pycc_ext_obj_call`. Returns a new reference or `NULL` with the exception
 * set.
 */
#define PYCC_EXT_METHOD_STACK 8
PyObject *pycc_ext_obj_call_method(PyObject *self, const char *name, PyObject **args,
                                   long long nargs)
{
    PyObject *stack[PYCC_EXT_METHOD_STACK + 1];
    PyObject *attr_name;
    PyObject *bound;
    PyObject *result = NULL;
    long long i;
    int packed = 1;

    for (i = 0; i < nargs; i++) {
        if (args[i] == NULL) {
            packed = 0;
        }
    }
    if (self != NULL && packed) {
        attr_name = pycc_ext_attr_name(name);
        if (attr_name != NULL && nargs <= PYCC_EXT_METHOD_STACK) {
            stack[0] = self;
            for (i = 0; i < nargs; i++) {
                stack[i + 1] = args[i];
            }
            result = PyObject_VectorcallMethod(attr_name, stack, (size_t)(nargs + 1), NULL);
        } else if (attr_name != NULL) {
            bound = PyObject_GetAttr(self, attr_name);
            if (bound != NULL) {
                result = PyObject_Vectorcall(bound, args, (size_t)nargs, NULL);
                Py_DECREF(bound);
            }
        }
    }
    for (i = 0; i < nargs; i++) {
        Py_XDECREF(args[i]);
    }
    return result;
}

/*
 * Part 2 of #1026, PR 2b of #1081: argument marshalling for
 * `pycc_ext_obj_call` below.
 *
 * Four packers, one per scalar `pycc_types` admits as an argument to a
 * foreign method call. Each takes a pycc-side value and returns a *new*
 * reference to its CPython equivalent, or NULL with a CPython exception
 * already set. They are non-`static` for the same reason their `obj_`
 * neighbours are: LLVM-generated code declares and calls them by these
 * exact names (`EXT_OBJ_PACK_*_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * **Every packer borrows its argument.** This is the one place where they
 * deliberately differ from the result packers above (`pycc_ext_pack_int`'s
 * bigint arm releases, `pycc_ext_pack_str` releases unconditionally), which
 * receive an already-transferred return value. An argument is an ordinary
 * module-body temporary that pycc's own machinery still owns, so consuming
 * it here would mean the boundary and `pycc_rt` both believe they hold the
 * last reference. Borrowing keeps ownership entirely on the pycc side, and
 * the price -- a bigint word that reaches the overflow arm is not released
 * -- is the same leak-only rule `docs/RUNTIME.md` already records for this
 * boundary, on a path that aborts the module load anyway.
 */
PyObject *pycc_ext_obj_pack_int(long long encoded)
{
    switch (pycc_rt_ext_int_classify(encoded)) {
    case PYCC_EXT_INT_SMALLINT:
        return PyLong_FromLongLong(pycc_rt_ext_int_decode(encoded));
    case PYCC_EXT_INT_FALSE:
        Py_RETURN_FALSE;
    case PYCC_EXT_INT_TRUE:
        Py_RETURN_TRUE;
    case PYCC_EXT_INT_BIGINT:
        return pycc_ext_bigint_pack(encoded);
    default:
        PyErr_SetString(PyExc_SystemError,
                        "an int argument to a CPython object's method was an "
                        "unrecognized int word");
        return NULL;
    }
}

PyObject *pycc_ext_obj_pack_float(double value)
{
    return PyFloat_FromDouble(value);
}

/*
 * `PyBool_FromLong` hands back one of the two interned singletons, so
 * `x is True` holds on the host side exactly as `pycc_ext_pack_bool`
 * already guarantees for a returned `bool`.
 */
PyObject *pycc_ext_obj_pack_bool(char value)
{
    return PyBool_FromLong(value != 0);
}

/*
 * Borrowing, unlike `pycc_ext_pack_str` above: see the block comment on
 * `pycc_ext_obj_pack_int`. The `PyStrObj` stays owned by the compiled
 * module body, and `PyUnicode_FromStringAndSize` copies the bytes, so the
 * CPython string is an independent object with no shared identity (#1043).
 */
PyObject *pycc_ext_obj_pack_str(void *value)
{
    const unsigned char *bytes;
    size_t len = 0;
    PyObject *shared;
    PyObject *packed;

    if (value == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "a str argument to a CPython object's method was NULL");
        return NULL;
    }
    /* The CPython `str` this one already shares, if any: the text is not
     * copied again (`PyStrObj::foreign`). */
    shared = (PyObject *)pycc_rt_str_foreign(value);
    if (shared != NULL) {
        Py_INCREF(shared);
        return shared;
    }
    bytes = pycc_rt_ext_str_bytes(value, &len);
    packed = PyUnicode_FromStringAndSize((const char *)bytes, (Py_ssize_t)len);
    if (packed != NULL) {
        Py_INCREF(packed);
        pycc_rt_str_set_foreign(value, packed);
    }
    return packed;
}

/*
 * Part 2a of #1371: the packer for an argument (or subscript key) whose
 * static type is itself the opaque `object`, as in `copy.deepcopy(o)` or
 * `table[k]` with an object `k`. Borrowing like its scalar neighbours: the
 * operand stays owned by whatever produced it (a pycc local, or a produced
 * temporary its consumer releases after the call), and the consuming helper that
 * receives the packed value -- `pycc_ext_obj_call` or `pycc_ext_obj_getitem`
 * -- releases exactly the one new reference taken here, so the operand's
 * net refcount is unchanged. `Py_INCREF` rather than `Py_NewRef` keeps the
 * shape of the rest of this file. A NULL operand cannot be produced by
 * checked code (every object producer routes its own NULL to a failure
 * edge first); it is answered with a `SystemError`, which the consuming
 * helpers already tolerate, rather than a crash.
 */
PyObject *pycc_ext_obj_pack_object(PyObject *value)
{
    if (value == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "an object argument to a CPython object's call was NULL");
        return NULL;
    }
    Py_INCREF(value);
    return value;
}

/*
 * Packs a native tuple of `int`/`float`/`bool` (`pycc_types`'
 * `is_packable_tuple`): `items` holds the `n` elements already packed by the
 * scalar packers, new references that are consumed on every path. A `NULL`
 * item is a packer that already set the exception. Returns a new `tuple`,
 * or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_pack_tuple(long long n, PyObject **items)
{
    PyObject *tuple = NULL;
    long long i;

    for (i = 0; i < n; i++) {
        if (items[i] == NULL) {
            goto fail;
        }
    }
    tuple = PyTuple_New((Py_ssize_t)n);
    if (tuple == NULL) {
        goto fail;
    }
    for (i = 0; i < n; i++) {
        /* `PyTuple_SetItem` steals the reference, even on failure. */
        PyObject *item = items[i];
        items[i] = NULL;
        if (PyTuple_SetItem(tuple, (Py_ssize_t)i, item) < 0) {
            Py_DECREF(tuple);
            tuple = NULL;
            goto fail;
        }
    }
    return tuple;
fail:
    for (i = 0; i < n; i++) {
        Py_XDECREF(items[i]);
        items[i] = NULL;
    }
    return NULL;
}

/*
 * Part 2 of #1026, PR 2b of #1081: the method-call helper compiled code
 * calls for `gc.disable()` on a value whose static type is the opaque
 * `object`.
 *
 * Deliberately *not* fused with the attribute load. An earlier revision
 * took `(obj, method)` and performed the `PyObject_GetAttrString` here, so
 * that the bound method never became a pycc value and the whole operation
 * presented one failure edge instead of two. That ordering is observable
 * and wrong: CPython resolves a call's callable *before* it evaluates the
 * arguments, so `obj.missing(1 // 0)` must raise `AttributeError`, while
 * the fused shim raised `ZeroDivisionError` -- the generated code had
 * already evaluated every argument by the time the lookup ran. Codegen now
 * emits `pycc_ext_obj_getattr` before the argument expressions and passes
 * the resulting callable here, which costs a second NULL test and a second
 * failure edge and buys back CPython's own evaluation order. The bound
 * method still never becomes a pycc value: it lives in an LLVM temporary
 * that dominates this call, and this function releases it.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_CALL_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `bound` is an *owned* reference -- the one `pycc_ext_obj_getattr`
 * returned -- and this function releases it on every path. `args` points at
 * `nargs` slots of owned references produced by the `pycc_ext_obj_pack_*`
 * helpers above; this function consumes every one of them on every path, so
 * the generated code never has to. The returned reference is the only
 * thing that escapes into compiled code as an `object` value; compiled code
 * releases it through `pycc_ext_obj_release` when it is an unbound
 * temporary (Part 1 of #1092), and a slot that binds it owns it.
 *
 * A packer that failed stored NULL in its slot with a CPython exception
 * already set. Scanning for that here rather than testing each packer's
 * result in LLVM keeps the emitted code one straight line per argument, and
 * propagating the *first* exception unchanged is the correct CPython state:
 * a second, synthetic error would overwrite the real one.
 *
 * A NULL `bound` is defence in depth: the caller's own NULL check on the
 * lookup already routed a failed attribute load to the foreign failure
 * edge before this call is reached.
 */
PyObject *pycc_ext_obj_call(PyObject *bound, PyObject **args, long long nargs)
{
    PyObject *result;
    long long i;
    int packed = 1;

    for (i = 0; i < nargs; i++) {
        if (args[i] == NULL) {
            packed = 0;
        }
    }
    result = (bound == NULL || !packed)
                 ? NULL
                 : PyObject_Vectorcall(bound, args, (size_t)nargs, NULL);
    Py_XDECREF(bound);
    for (i = 0; i < nargs; i++) {
        Py_XDECREF(args[i]);
    }
    return result;
}

/*
 * #1313: call `callee` itself -- `product(1, 2)` where `product` is a
 * foreign binding, or a call of a `for` loop target bound to an object --
 * rather than a method looked up on it (`EXT_OBJ_CALL_BORROWED_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 *
 * `callee` is BORROWED: it is a reference the caller keeps (a module global
 * the artifact retains, or a `for` loop target's slot), not a fresh
 * reference like the bound method `pycc_ext_obj_call` consumes, so handing
 * it to that helper directly would drop the caller's reference on the first
 * call. The extra reference
 * taken here is the one `pycc_ext_obj_call` then releases, which keeps a
 * single implementation of the packed-argument scan and the vectorcall.
 *
 * Every `args[i]` is CONSUMED on every path, exactly as for
 * `pycc_ext_obj_call`. Returns a new reference (owned as #1092 records) or
 * NULL with a Python exception set; the caller routes NULL to the
 * foreign failure edge (`crates/pycc_codegen/src/foreign_fail.rs`).
 */
PyObject *pycc_ext_obj_call_borrowed(PyObject *callee, PyObject **args,
                                     long long nargs)
{
    Py_XINCREF(callee);
    return pycc_ext_obj_call(callee, args, nargs);
}

/*
 * Part 8 of #1371: a call on a CPython object that passes keyword
 * arguments -- `o.method(x, key=v)`, `f(a, b=c)`, `Cls(arg, flag=True)`
 * (`EXT_OBJ_CALL_KW_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `args` holds `nargs + nkw` slots: the positional arguments, then the
 * keyword values, each an owned reference from a `pycc_ext_obj_pack_*`
 * helper. `names` holds the `nkw` keyword names as NUL-terminated UTF-8
 * (compile-time constants; the parser already refused a repeated name).
 * The names become the `kwnames` tuple of CPython's own vectorcall
 * keyword protocol, which every callable honours -- a callable without a
 * vectorcall slot gets the arguments re-packed into a `kwargs` dict by
 * CPython itself -- so this is observably `PyObject_Call(callable, args,
 * kwargs)` without building the dict here.
 *
 * Ownership is exactly `pycc_ext_obj_call`'s: `callable` and every
 * `args[i]` are CONSUMED on every path, including a failed packer (a NULL
 * slot, whose exception is propagated unchanged) and a failure to build
 * `kwnames`. Returns a new reference (owned as #1092 records) or NULL with
 * a Python exception set.
 */
PyObject *pycc_ext_obj_call_kw(PyObject *callable, PyObject **args,
                               long long nargs, const char **names,
                               long long nkw)
{
    PyObject *result = NULL;
    PyObject *kwnames = NULL;
    long long total = nargs + nkw;
    long long i;
    int packed = 1;
    int unpacking = 0;

    for (i = 0; i < total; i++) {
        if (args[i] == NULL) {
            packed = 0;
        }
    }
    for (i = 0; i < nkw; i++) {
        unpacking |= pycc_ext_is_unpacking(names[i]);
    }
    /* `*xs` / `**kw` among the arguments (`object_unpack.c`): bound at run
     * time, with CPython's checks. */
    if (callable != NULL && packed && unpacking) {
        result = pycc_ext_call_unpacking(callable, args, nargs, names, nkw);
    } else if (callable != NULL && packed) {
        kwnames = PyTuple_New((Py_ssize_t)nkw);
        for (i = 0; kwnames != NULL && i < nkw; i++) {
            PyObject *name = PyUnicode_InternFromString(names[i]);
            if (name == NULL || PyTuple_SetItem(kwnames, (Py_ssize_t)i, name) < 0) {
                Py_CLEAR(kwnames);
            }
        }
        if (kwnames != NULL) {
            result = PyObject_Vectorcall(callable, args, (size_t)nargs, kwnames);
        }
    }
    Py_XDECREF(kwnames);
    Py_XDECREF(callable);
    for (i = 0; i < total; i++) {
        Py_XDECREF(args[i]);
    }
    return result;
}

/*
 * Part 8 of #1371: `pycc_ext_obj_call_kw` with a BORROWED callable
 * (`EXT_OBJ_CALL_KW_BORROWED_SYMBOL`), the keyword twin of
 * `pycc_ext_obj_call_borrowed`: a module global such as a foreign class or
 * function, or a `for` loop target. The extra reference taken here is the
 * one `pycc_ext_obj_call_kw` releases. Every `args[i]` is consumed.
 */
PyObject *pycc_ext_obj_call_kw_borrowed(PyObject *callable, PyObject **args,
                                        long long nargs, const char **names,
                                        long long nkw)
{
    Py_XINCREF(callable);
    return pycc_ext_obj_call_kw(callable, args, nargs, names, nkw);
}

/*
 * Part 3 of #1026 (PR 3a of #1082): `len(o)` on a CPython object value
 * (`EXT_OBJ_LEN_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed and its refcount is untouched on every path --
 * `PyObject_Size` neither steals nor retains, and the result is a plain
 * `Py_ssize_t`, so no new reference exists to leak under the #1092
 * leak-only regime.
 *
 * The D-141 encode is fused in here rather than emitted by codegen so the
 * whole operation presents *one* failure edge to the caller: `PyObject_Size`
 * raises `TypeError` for an operand with no length, and the encode refuses a
 * value outside the inline range `[-2**62, 2**62-1]`. Two `-1` returns from
 * one symbol let `foreign_len.rs` emit a single branch to the foreign
 * failure edge instead of two. The encode arm is unreachable for a real
 * container -- no object has 2**62 elements -- and exists as defence in
 * depth; `PyErr_NoMemory` is the honest report for a length that large.
 *
 * As with `pycc_ext_obj_getattr`, a NULL `o` is decided here as defence in
 * depth: `PyObject_Size` dereferences `Py_TYPE(o)` with no guard of its own,
 * and the caller's own NULL check already routed a failed producer to the
 * foreign failure edge with its exception set, so returning `-1` without
 * setting a second one leaves exactly one exception pending.
 */
int pycc_ext_obj_len(PyObject *o, long long *out)
{
    Py_ssize_t size;

    if (o == NULL) {
        return -1;
    }
    size = PyObject_Size(o);
    if (size == -1 && PyErr_Occurred()) {
        return -1;
    }
    *out = pycc_rt_int_from_i64((long long)size);
    return 0;
}

/*
 * Part 3 of #1026 (PR 3a of #1082): truth testing a CPython object value
 * (`EXT_OBJ_TRUTHY_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Returns `1` for a truthy operand, `0` for a falsy one, and `-1` with a
 * CPython exception set on failure -- `PyObject_IsTrue` calls the operand's
 * `__bool__` or `__len__`, which is arbitrary Python code and really can
 * raise. `o` is borrowed and its refcount is untouched on every path.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
int pycc_ext_obj_truthy(PyObject *o)
{
    if (o == NULL) {
        return -1;
    }
    return PyObject_IsTrue(o);
}

/*
 * Part 11 of #1371: `type(o)` on a CPython object value
 * (`EXT_OBJ_TYPE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Returns a new reference to `o`'s class (`PyObject_Type`), owned like every
 * other producer's: released when consumed unbound (Part 1 of #1092),
 * owned by a slot that binds it.
 * `o` is borrowed. `PyObject_Type` cannot fail for a live object; a NULL
 * `o` is the defence in depth `pycc_ext_obj_getattr` and `pycc_ext_obj_len`
 * document: a NULL operand comes only from a producer whose own NULL check
 * already routed its exception, so returning NULL without setting a second
 * one leaves exactly one exception pending.
 */
PyObject *pycc_ext_obj_type(PyObject *o)
{
    if (o == NULL) {
        return NULL;
    }
    return PyObject_Type(o);
}

/*
 * Drop one reference to a CPython object (`EXT_OBJ_RELEASE_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 *
 * Every `object` slot of compiled code -- a local, a parameter, a module
 * global, an instance attribute -- owns one reference, and every value an
 * expression produces is either a borrowed read of such a slot or a
 * reference of its own (`crates/pycc_codegen/src/heap_rc.rs`). Generated
 * code calls this for a consumed temporary (`object_release.rs`), a slot's
 * replaced value, and a frame's slots on every exit. `Py_XDECREF` rather
 * than `Py_DECREF` because a slot never assigned, or a producer that failed,
 * holds NULL.
 */
void pycc_ext_obj_release(PyObject *o)
{
    Py_XDECREF(o);
}

/*
 * Take a new reference to a borrowed CPython object
 * (`EXT_OBJ_RETAIN_SYMBOL` in `crates/pycc_codegen/src/ext.rs`): before a
 * borrowed value moves into a slot that owns it, is passed to a parameter,
 * is returned, or is pinned across a sibling that may run code.
 * `Py_XINCREF` mirrors `pycc_ext_obj_release`'s NULL tolerance.
 */
void pycc_ext_obj_retain(PyObject *o)
{
    Py_XINCREF(o);
}

/*
 * The same pair as `pycc_rt`'s object hooks (`pycc_rt_object_install`,
 * installed by `pycc_ext_exec_module`), which free a compiled instance's
 * `o` slots and store into them from a host setter.
 */
static void pycc_ext_obj_retain_hook(void *o)
{
    Py_XINCREF((PyObject *)o);
}

static void pycc_ext_obj_release_hook(void *o)
{
    Py_XDECREF((PyObject *)o);
}

/*
 * Part 1 of #1371: `l op r` where at least one operand is a CPython object
 * (`EXT_OBJ_RICHCOMPARE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `op` is CPython's own rich-comparison selector (`Py_LT` .. `Py_GE`), and
 * the call is `PyObject_RichCompare`, never `PyObject_RichCompareBool`: the
 * latter short-circuits on identity, so `n == n` would answer `True` for a
 * NaN `n` where CPython's `==` answers `False`, and it would collapse a
 * non-`bool` result (an array, say) to its truth value.
 *
 * `owned` says which operands were produced by a `pycc_ext_obj_pack_*`
 * helper (bit 0 the left, bit 1 the right): those are consumed on every
 * path, including a `NULL` one, exactly as `pycc_ext_obj_getitem` consumes
 * its packed key, so a failed packer needs no failure edge of its own.
 * Every other operand is borrowed. The result is a *new* reference, owned
 * as `pycc_ext_obj_call`'s result is (released when an unbound temporary,
 * Part 1 of #1092), or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_richcompare(PyObject *l, PyObject *r, int op, int owned)
{
    PyObject *result;

    result = (l == NULL || r == NULL) ? NULL : PyObject_RichCompare(l, r, op);
    if (owned & 1) {
        Py_XDECREF(l);
    }
    if (owned & 2) {
        Py_XDECREF(r);
    }
    return result;
}

/*
 * `bool(l op r)` for a rich comparison in a truth context, `if o == "x":`
 * (`EXT_OBJ_COMPARE_BOOL_SYMBOL` in `crates/pycc_codegen/src/ext.rs`):
 * `pycc_ext_obj_richcompare` followed by `PyObject_IsTrue` of its result,
 * in one call. Two exact `str` operands are compared by
 * `PyUnicode_Compare`, which orders code points as `str`'s own comparison
 * does, so no result object is made. The operands follow
 * `pycc_ext_obj_richcompare`'s `owned` contract. Returns 1 or 0, or -1 with
 * the exception set.
 */
int pycc_ext_obj_compare_bool(PyObject *l, PyObject *r, int op, int owned)
{
    PyObject *result;
    int status = -1;
    int order;

    if (l != NULL && r != NULL) {
        if (PyUnicode_CheckExact(l) && PyUnicode_CheckExact(r)) {
            order = (l == r) ? 0 : PyUnicode_Compare(l, r);
            switch (op) {
            case Py_LT:
                status = order < 0;
                break;
            case Py_LE:
                status = order <= 0;
                break;
            case Py_EQ:
                status = order == 0;
                break;
            case Py_NE:
                status = order != 0;
                break;
            case Py_GT:
                status = order > 0;
                break;
            default:
                status = order >= 0;
                break;
            }
        } else {
            result = PyObject_RichCompare(l, r, op);
            if (result != NULL) {
                status = PyObject_IsTrue(result);
                Py_DECREF(result);
            }
        }
    }
    if (owned & 1) {
        Py_XDECREF(l);
    }
    if (owned & 2) {
        Py_XDECREF(r);
    }
    return status;
}

/*
 * Part 1 of #1371: a *borrowed* pointer to CPython's `None` singleton, the
 * operand compiled code compares an object against for `o is None`
 * (`EXT_OBJ_NONE_SYMBOL`). `None` is immortal, so the borrow never dangles.
 */
PyObject *pycc_ext_obj_none(void)
{
    return Py_None;
}

/*
 * #1418: a *borrowed* pointer to CPython's `NotImplemented` singleton, the
 * value of an admitted `return NotImplemented` in a comparison method
 * (`EXT_OBJ_NOT_IMPLEMENTED_SYMBOL`). It is immortal on CPython 3.13+, so a
 * compiled return may hand the borrow back as is; when the result crosses
 * the export boundary, `pycc_ext_pack_object` takes the strong reference
 * (`Py_NewRef`) the host caller owns.
 */
PyObject *pycc_ext_obj_not_implemented(void)
{
    return Py_NotImplemented;
}

/*
 * The value of a `...` literal (`EXT_OBJ_ELLIPSIS_SYMBOL`): a borrowed
 * pointer to CPython's immortal `Ellipsis`, handed out exactly like
 * `NotImplemented` above.
 */
PyObject *pycc_ext_obj_ellipsis(void)
{
    return Py_Ellipsis;
}

/*
 * `hash(o)` of a CPython object (`EXT_OBJ_HASH_SYMBOL`): writes
 * `PyObject_Hash(o)` through `*out` and answers 0, or answers -1 with the
 * exception CPython raised (an unhashable object's `TypeError`, a raising
 * `__hash__`) set. `o` is borrowed.
 */
int pycc_ext_obj_hash(PyObject *o, long long *out)
{
    Py_hash_t hash;

    if (o == NULL || out == NULL) {
        return -1;
    }
    hash = PyObject_Hash(o);
    if (hash == -1 && PyErr_Occurred()) {
        return -1;
    }
    *out = (long long)hash;
    return 0;
}

/*
 * Part 1 of #1371: `isinstance(o, cls)` with an object `o`
 * (`EXT_OBJ_ISINSTANCE_SYMBOL`). Both operands are borrowed. A `NULL`
 * `cls` selects a builtin class by `builtin`: 0 `int`, 1 `str`, 2 `float`,
 * 3 `bool`, and since Part 7 of #1371 4 `list`, 5 `dict`, 6 `tuple`
 * (`pycc_mir::ObjBuiltinClass::shim_code`). Returns 1, 0, or -1
 * with the exception set (`PyObject_IsInstance` raises `TypeError` for a
 * `cls` that is not a class, and propagates a raising
 * `__instancecheck__`).
 */
/*
 * Whether `type` is one of the built-in types whose instances' `__class__`
 * is their type and cannot be overridden: an exact `dict`, `list`, `tuple`,
 * `str`, `int`, `float`, `bool`, `set`, `frozenset` or `None`.
 */
static int pycc_ext_plain_builtin(PyTypeObject *type)
{
    return type == &PyDict_Type || type == &PyList_Type || type == &PyTuple_Type
           || type == &PyUnicode_Type || type == &PyLong_Type || type == &PyFloat_Type
           || type == &PyBool_Type || type == &PySet_Type || type == &PyFrozenSet_Type
           || (PyObject *)type == (PyObject *)Py_TYPE(Py_None);
}

/*
 * `isinstance(o, cls)`: `PyObject_IsInstance`, answered directly when `o` is
 * an instance of a plain built-in type and `cls` is a class whose own type
 * is `type` (no `__instancecheck__` of a metaclass), or a tuple of such
 * classes. That is the answer `PyObject_IsInstance` reaches too: the type
 * test, and for a miss a `__class__` lookup that for these objects names
 * the same type again.
 */
static int pycc_ext_isinstance(PyObject *o, PyObject *cls)
{
    Py_ssize_t i;
    Py_ssize_t n;
    PyObject *item;

    if (pycc_ext_plain_builtin(Py_TYPE(o))) {
        if (PyType_CheckExact(cls)) {
            return PyType_IsSubtype(Py_TYPE(o), (PyTypeObject *)cls);
        }
        if (PyTuple_CheckExact(cls)) {
            n = PyTuple_Size(cls);
            for (i = 0; i < n; i++) {
                item = PyTuple_GetItem(cls, i);
                if (item == NULL || !PyType_CheckExact(item)) {
                    break;
                }
                if (PyType_IsSubtype(Py_TYPE(o), (PyTypeObject *)item)) {
                    return 1;
                }
            }
            if (i == n) {
                return 0;
            }
        }
    }
    return PyObject_IsInstance(o, cls);
}

int pycc_ext_obj_isinstance(PyObject *o, PyObject *cls, int builtin)
{
    if (cls == NULL) {
        switch (builtin) {
        case 0:
            cls = (PyObject *)&PyLong_Type;
            break;
        case 1:
            cls = (PyObject *)&PyUnicode_Type;
            break;
        case 2:
            cls = (PyObject *)&PyFloat_Type;
            break;
        case 3:
            cls = (PyObject *)&PyBool_Type;
            break;
        case 4:
            cls = (PyObject *)&PyList_Type;
            break;
        case 5:
            cls = (PyObject *)&PyDict_Type;
            break;
        case 6:
            cls = (PyObject *)&PyTuple_Type;
            break;
        default:
            PyErr_SetString(PyExc_SystemError,
                            "pycc_ext_obj_isinstance: unrecognized builtin class selector");
            return -1;
        }
    }
    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_isinstance: NULL operand");
        return -1;
    }
    return pycc_ext_isinstance(o, cls);
}

/*
 * The answer for a compiled class no published type descends from (a
 * private class, one exporting no method that the host cannot construct
 * either, every class of an embedded build), asked of an object that carries no compiled instance (a carrier,
 * #1435, is answered before this is reached): no such object is an
 * instance of the class, but CPython's own
 * `isinstance` does not answer False before it has looked up the object's
 * `__class__` (`object_isinstance` in `Objects/abstract.c`), so an error
 * raised there propagates as -1 here too. A missing `__class__` is 0, and
 * whatever it names cannot be a subclass of a class with no type object.
 * The generated `pycc_ext_compiled_class_isinstance` calls it by the name
 * `UNPUBLISHED_CLASS_ISINSTANCE` in `src/ext_build/method_types.rs` spells.
 */
static int pycc_ext_unpublished_class_isinstance(PyObject *o)
{
    PyObject *cls = NULL;
    if (PyObject_GetOptionalAttrString(o, "__class__", &cls) < 0) {
        return -1;
    }
    Py_XDECREF(cls);
    return 0;
}

/*
 * Part 2d of #1371: a list display bound to an object slot, `x: object =
 * [a, b]` (`EXT_OBJ_BUILD_LIST_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `items` holds `n` *packed* elements, in source order, each a new
 * reference a `pycc_ext_obj_pack_*` helper produced. Every one is consumed
 * on every path, exactly as `pycc_ext_obj_call` consumes its arguments: a
 * `NULL` element is a packer that already set the exception, so the list is
 * never built and the rest are released; a failed `PyList_New` releases
 * them all. Otherwise each reference moves into the fresh list through
 * `PyList_SetItem`, which steals it -- the limited API (abi3) has no
 * `PyList_SET_ITEM`. The index is always in range of a list just built
 * with `n` slots, so it cannot fail; the check is defence in depth and
 * releases what has not moved yet. The result is a *new* reference, owned
 * as `pycc_ext_obj_call`'s result is (released when an unbound temporary,
 * Part 1 of #1092), or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_build_list(PyObject **items, long long n)
{
    PyObject *list = NULL;
    int failed = 0;
    long long i;

    for (i = 0; i < n; i++) {
        if (items[i] == NULL) {
            failed = 1;
        }
    }
    if (!failed) {
        list = PyList_New((Py_ssize_t)n);
    }
    if (list == NULL) {
        for (i = 0; i < n; i++) {
            Py_XDECREF(items[i]);
        }
        return NULL;
    }
    for (i = 0; i < n; i++) {
        if (PyList_SetItem(list, (Py_ssize_t)i, items[i]) < 0) {
            for (i = i + 1; i < n; i++) {
                Py_DECREF(items[i]);
            }
            Py_DECREF(list);
            return NULL;
        }
    }
    return list;
}

/*
 * #1457 (Part 2 of #1443): `o.name = value` with a CPython object `o`
 * (`EXT_OBJ_SETATTR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed; `value` is a packer's new reference, consumed on every
 * path. A NULL `value` is a failed packer (an out-of-range bigint's
 * `OverflowError`, say) whose exception is already set, and it must not
 * reach `PyObject_SetAttrString`: a NULL value there *deletes* the
 * attribute. A NULL `o` is guarded as `pycc_ext_obj_getattr` guards it.
 * Returns `0`, or `-1` with the exception set -- whatever the object's own
 * `__setattr__`, a read-only property or a `__slots__` layout raises.
 */
int pycc_ext_obj_setattr(PyObject *o, const char *name, PyObject *value)
{
    int status;

    if (value == NULL) {
        return -1;
    }
    if (o == NULL) {
        Py_DECREF(value);
        return -1;
    }
    status = PyObject_SetAttrString(o, name, value);
    Py_DECREF(value);
    return status;
}

/*
 * #1457: `del o.name` with a CPython object `o`
 * (`EXT_OBJ_DELATTR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`), `o`
 * borrowed. Returns `0`, or `-1` with the exception set: a missing
 * attribute raises `AttributeError`, exactly as CPython's own `del` does.
 */
int pycc_ext_obj_delattr(PyObject *o, const char *name)
{
    if (o == NULL) {
        return -1;
    }
    return PyObject_DelAttrString(o, name);
}

/*
 * Part 9 of #1371: `raise o` with a CPython object `o`
 * (`EXT_OBJ_RAISE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Decides what is raised in the order CPython's own `raise` does:
 *
 *  - an exception class is called with no arguments, and its result must
 *    be an exception instance: a raising constructor's own exception is the
 *    one raised, and any other result raises `TypeError` with CPython's
 *    own message;
 *  - an exception instance is raised as it is;
 *  - anything else raises `TypeError: exceptions must derive from
 *    BaseException`.
 *
 * The instance is set with `PyErr_SetObject`, as CPython's `raise` does,
 * not `PyErr_SetRaisedException`: it keeps the instance's identity and also
 * sets its implicit `__context__` from the exception CPython is handling,
 * such as the host's own `except` block around the call. A pycc `except`
 * handler is not a CPython handler, so an object raised inside one gets no
 * `__context__` from the pycc exception it handles.
 *
 * It then hands the CPython exception to `pycc_ext_obj_error_bridge`, which
 * makes it a pending pycc exception and keeps the original in the bridge
 * table. The caller branches to its innermost exception target, so an
 * enclosing `try` runs in a function body and in the module body alike,
 * and an exception that escapes reaches the host as the original object.
 *
 * `o` is borrowed. Total: on return a pycc exception is always pending and
 * CPython's error indicator is clear. A NULL `o` is the same defence in
 * depth `pycc_ext_obj_get_iter` documents, and bridges a `SystemError`.
 */
void pycc_ext_obj_raise(PyObject *o)
{
    PyObject *instance;

    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: raise of a missing CPython object");
    } else if (PyExceptionClass_Check(o)) {
        instance = PyObject_CallNoArgs(o);
        if (instance == NULL) {
            /* The constructor's own exception is the one raised. */
        } else if (!PyExceptionInstance_Check(instance)) {
            PyErr_Format(PyExc_TypeError,
                         "calling %R should have returned an instance of BaseException, not %R",
                         o, (PyObject *)Py_TYPE(instance));
            Py_DECREF(instance);
        } else {
            PyErr_SetObject((PyObject *)Py_TYPE(instance), instance);
            Py_DECREF(instance);
        }
    } else if (PyExceptionInstance_Check(o)) {
        PyErr_SetObject((PyObject *)Py_TYPE(o), o);
    } else {
        PyErr_SetString(PyExc_TypeError, "exceptions must derive from BaseException");
    }
    (void)pycc_ext_obj_error_bridge();
}

/*
 * `raise o from cause` with a CPython object `o`
 * (`EXT_OBJ_RAISE_FROM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`): what
 * `pycc_ext_obj_raise` raises, with `__cause__` set as CPython's own
 * `raise ... from ...` sets it. The cause is either a CPython object
 * (`object_cause`) or a pycc exception (`native_cause`), which crosses as
 * the CPython exception it was bridged from, or else as the one the bridge
 * builds for it when it escapes. Both are borrowed.
 */
extern void pycc_rt_exception_incref(void *obj);

void pycc_ext_obj_raise_from(PyObject *o, void *native_cause, PyObject *object_cause)
{
    PyObject *cause = NULL;
    PyObject *instance = NULL;
    PyObject *foreign;

    if (object_cause != NULL) {
        cause = Py_NewRef(object_cause);
    } else if (native_cause != NULL) {
        foreign = pycc_rt_exception_foreign(native_cause);
        if (foreign != NULL) {
            cause = Py_NewRef(foreign);
        } else {
            /* The pending state takes its own reference, which raising it
             * through the bridge then releases. */
            pycc_rt_exception_incref(native_cause);
            pycc_rt_exception_raise(native_cause);
            if (pycc_ext_raise_pending()) {
                cause = PyErr_GetRaisedException();
            }
        }
    }
    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: raise of a missing CPython object");
    } else if (PyExceptionClass_Check(o)) {
        instance = PyObject_CallNoArgs(o);
        if (instance != NULL && !PyExceptionInstance_Check(instance)) {
            PyErr_Format(PyExc_TypeError,
                         "calling %R should have returned an instance of BaseException, not %R",
                         o, (PyObject *)Py_TYPE(instance));
            Py_CLEAR(instance);
        }
    } else if (PyExceptionInstance_Check(o)) {
        instance = Py_NewRef(o);
    } else {
        PyErr_SetString(PyExc_TypeError, "exceptions must derive from BaseException");
    }
    if (instance != NULL) {
        if (cause != NULL && cause != Py_None && !PyExceptionInstance_Check(cause)) {
            if (PyExceptionClass_Check(cause)) {
                PyObject *made = PyObject_CallNoArgs(cause);
                Py_DECREF(cause);
                cause = made;
                if (cause == NULL) {
                    Py_CLEAR(instance);
                }
            } else {
                PyErr_SetString(PyExc_TypeError,
                                "exception causes must derive from BaseException");
                Py_CLEAR(cause);
                Py_CLEAR(instance);
            }
        }
    }
    if (instance != NULL) {
        /* Steals `cause`; a NULL cause still suppresses the context. */
        PyException_SetCause(instance, cause);
        cause = NULL;
        PyErr_SetObject((PyObject *)Py_TYPE(instance), instance);
        Py_DECREF(instance);
    }
    Py_XDECREF(cause);
    (void)pycc_ext_obj_error_bridge();
}

/*
 * Part 3 of #1026 (PR 3c of #1082): `iter(o)` for a `for x in <object>:`
 * loop (`EXT_OBJ_GET_ITER_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed. The result is a *new* reference the caller owns: since
 * Part 3 of #1092 the compiled `for` loop or comprehension releases it on
 * its normal exit and on every failure edge that leaves it, as
 * `docs/RUNTIME.md` records.
 *
 * A non-iterable operand makes `PyObject_GetIter` set `TypeError` and
 * return NULL, which the caller routes to the foreign failure edge, so
 * "this object cannot be iterated" needs no compile-time test: pycc knows
 * nothing about the pointee and could not perform one.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
PyObject *pycc_ext_obj_get_iter(PyObject *o)
{
    if (o == NULL) {
        return NULL;
    }
    return PyObject_GetIter(o);
}

/*
 * Part 3 of #1026 (PR 3c of #1082): one step of a `for x in <object>:` loop
 * (`EXT_OBJ_ITER_NEXT_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Three-valued, unlike every other helper here:
 *
 *   1  -- a *new* reference to the next item has been written to `*out`;
 *   0  -- the iterator is cleanly exhausted, `*out` untouched;
 *  -1  -- iteration failed with a CPython exception already set.
 *
 * `PyIter_Next` collapses the last two into NULL and only `PyErr_Occurred()`
 * tells them apart. That discrimination lives here rather than in emitted
 * LLVM IR on purpose: it is a CPython calling convention, and open-coding it
 * in the code generator would put a second, independently maintained copy of
 * that convention in a place where it could silently drift. It is also what
 * keeps *exhaustion off the failure edge* -- a loop that simply ends is not
 * a foreign failure, and fusing the two would have made every `for` loop
 * over a foreign object terminate the module body.
 *
 * Each item written through `*out` is a new reference, which the `for`
 * target or comprehension variable owns and releases on the next trip.
 *
 * `it` and `out` are NULL-guarded as defence in depth, exactly like
 * `pycc_ext_obj_len`'s own operand: returning -1 without setting an
 * exception would be wrong, so this path sets one itself -- unlike the
 * NULL-returning helpers above, whose caller has already seen a real
 * CPython exception from the producer that returned NULL.
 */
long long pycc_ext_obj_iter_next(PyObject *it, PyObject **out)
{
    PyObject *item;

    if (it == NULL || out == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc_ext_obj_iter_next called with a NULL argument");
        return -1;
    }
    item = PyIter_Next(it);
    if (item != NULL) {
        *out = item;
        return 1;
    }
    return PyErr_Occurred() == NULL ? 0 : -1;
}

