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
 * are `pycc_ext_obj_getattr`, `pycc_ext_obj_call_method`,
 * `pycc_ext_obj_setattr` and `pycc_ext_obj_delattr`, whose name arguments
 * codegen emits as private string constants), which lives as long
 * as the module and is never reused for another text: one address names one
 * text, so a hit is decided by the address alone. A slot holds one name; a
 * colliding name replaces it. Borrowed; NULL with an exception set when the
 * name cannot be created.
 */
#define PYCC_EXT_NAME_SLOTS 4096
#define PYCC_EXT_NAME_SLOT_BITS 12
static struct {
    const char *spelling;
    PyObject *name;
} pycc_ext_names[PYCC_EXT_NAME_SLOTS];

static PyObject *pycc_ext_attr_name(const char *spelling)
{
    /* Constants are byte-aligned and often adjacent: mix every address bit
     * into the slot index (Fibonacci hashing). */
    size_t slot = (size_t)(((uint64_t)(uintptr_t)spelling * 0x9E3779B97F4A7C15ull) >> (64 - PYCC_EXT_NAME_SLOT_BITS))
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
    PyObject *value;

    if (obj == NULL) {
        return NULL;
    }
    value = pycc_ext_ic_try(obj, name);
    if (value != PYCC_IC_MISS) {
        return value;
    }
    attr_name = pycc_ext_attr_name(name);
    if (attr_name == NULL) {
        return NULL;
    }
    return pycc_ext_ic_getattr_slow(obj, name, attr_name);
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
                                   long long nargs, PyccExtCallSite *site)
{
    PyObject *stack[PYCC_EXT_METHOD_STACK + 1];
    PyObject *attr_name;
    PyObject *bound;
    PyObject *method;
    PyObject *result = NULL;
    long long i;
    int packed = 1;
    int with_self = 1;

    for (i = 0; i < nargs; i++) {
        if (args[i] == NULL) {
            packed = 0;
        }
    }
    if (self != NULL && packed && nargs <= PYCC_EXT_METHOD_STACK) {
        /* v0.9: the call site's own cache (`pycc_ext_site_method`). */
        method = pycc_ext_site_method(site, self);
        if (method != NULL) {
            stack[0] = self;
            for (i = 0; i < nargs; i++) {
                stack[i + 1] = args[i];
            }
            Py_INCREF(method);
            result = pycc_ext_site_call(site, method, stack, nargs);
            Py_DECREF(method);
            for (i = 0; i < nargs; i++) {
                Py_DECREF(args[i]);
            }
            return result;
        }
    }
    if (self != NULL && packed) {
        /* A hit needs no interned name (`attr_cache.c`). */
        method = nargs <= PYCC_EXT_METHOD_STACK ? pycc_ext_ic_method(self, name, NULL, &with_self) : NULL;
        attr_name = method != NULL ? NULL : pycc_ext_attr_name(name);
        if ((method != NULL || attr_name != NULL) && nargs <= PYCC_EXT_METHOD_STACK) {
            stack[0] = self;
            for (i = 0; i < nargs; i++) {
                stack[i + 1] = args[i];
            }
            if (method == NULL) {
                method = pycc_ext_ic_method(self, name, attr_name, &with_self);
            }
            if (method != NULL) {
                /* A cached method or module attribute (`attr_cache.c`), held
                 * for the call. */
                Py_INCREF(method);
                result = with_self ? PyObject_Vectorcall(method, stack, (size_t)(nargs + 1), NULL)
                                   : PyObject_Vectorcall(method, stack + 1, (size_t)nargs, NULL);
                Py_DECREF(method);
                if (with_self) {
                    pycc_ext_site_fill(site, self, name);
                }
            } else {
                result = PyObject_VectorcallMethod(attr_name, stack, (size_t)(nargs + 1), NULL);
            }
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
 * `obj.name` loaded to be called, before the arguments are evaluated
 * (`EXT_OBJ_LOAD_METHOD_SYMBOL`): CPython's own `LOAD_ATTR` of a method. A
 * method the attribute caches answer for `obj`'s class (`attr_cache.c`) is
 * returned as the function itself, with `*self_out` a new reference to
 * `obj`, so no bound method is made; anything else is `obj.name` as
 * `pycc_ext_obj_getattr` reads it, `*self_out` NULL. `obj` is borrowed.
 * Returns a new reference, or NULL with the exception set (`*self_out`
 * NULL). `pycc_ext_obj_call_loaded` consumes both.
 */
PyObject *pycc_ext_obj_load_method(PyObject *obj, const char *name, PyObject **self_out,
                                   PyccExtCallSite *site)
{
    PyObject *attr_name;
    PyObject *method;
    int with_self = 0;

    *self_out = NULL;
    if (obj == NULL) {
        return NULL;
    }
    /* v0.9: the call site's own cache (`pycc_ext_site_method`). */
    method = pycc_ext_site_method(site, obj);
    if (method != NULL) {
        *self_out = Py_NewRef(obj);
        return Py_NewRef(method);
    }
    method = pycc_ext_ic_method(obj, name, NULL, &with_self);
    if (method == NULL) {
        attr_name = pycc_ext_attr_name(name);
        if (attr_name == NULL) {
            return NULL;
        }
        method = pycc_ext_ic_method(obj, name, attr_name, &with_self);
    }
    if (method != NULL) {
        if (with_self) {
            *self_out = Py_NewRef(obj);
            pycc_ext_site_fill(site, obj, name);
        }
        return Py_NewRef(method);
    }
    return pycc_ext_obj_getattr(obj, name);
}

/*
 * The call of what `pycc_ext_obj_load_method` loaded
 * (`EXT_OBJ_CALL_LOADED_SYMBOL`): `stack[0]` is the receiver or NULL and
 * `stack[1..nargs]` the packed arguments, so `callable(*stack)` when there
 * is a receiver and `callable(*stack[1:])` otherwise, with no copy (the
 * offset flag lets the callee use `stack[0]` as scratch, as CPython's own
 * calls allow). `callable` and every slot are consumed on every path, as by
 * `pycc_ext_obj_call`; a NULL argument is a failed packer whose exception
 * is already set. Returns a new reference or NULL with the exception set.
 */
PyObject *pycc_ext_obj_call_loaded(PyObject *callable, PyObject **stack, long long nargs,
                                   PyccExtCallSite *site)
{
    PyObject *self = stack[0];
    PyObject *result = NULL;
    long long i;
    int packed = callable != NULL;

    for (i = 1; i <= nargs; i++) {
        if (stack[i] == NULL) {
            packed = 0;
        }
    }
    if (packed && self != NULL && site != NULL && site->fast != NULL
        && pycc_ext_site_method(site, self) == callable) {
        /* The method the site's cache loaded (`pycc_ext_obj_load_method`). */
        result = pycc_ext_site_call(site, callable, stack, nargs);
    } else if (packed && self != NULL) {
        result = PyObject_Vectorcall(callable, stack, (size_t)(nargs + 1), NULL);
    } else if (packed) {
        result = PyObject_Vectorcall(callable, stack + 1, (size_t)nargs | PY_VECTORCALL_ARGUMENTS_OFFSET,
                                     NULL);
    }
    Py_XDECREF(callable);
    Py_XDECREF(self);
    for (i = 1; i <= nargs; i++) {
        Py_XDECREF(stack[i]);
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

/* Whether `bytes` would be interned by CPython as a code constant (`len`
 * of 0 or 1 included, both singletons in CPython): all ASCII letters, digits
 * and `_`. */
static int pycc_ext_name_chars(const unsigned char *bytes, size_t len)
{
    size_t i;

    if (len <= 1) {
        return len == 0 || bytes[0] < 0x80;
    }
    for (i = 0; i < len; i++) {
        unsigned char c = bytes[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_')) {
            return 0;
        }
    }
    return 1;
}

/*
 * Borrowing, unlike `pycc_ext_pack_str` above: see the block comment on
 * `pycc_ext_obj_pack_int`. The `PyStrObj` stays owned by the compiled
 * module body, and `PyUnicode_FromStringAndSize` copies the bytes, so the
 * CPython string is an independent object with no shared identity (#1043),
 * but a literal's (below).
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
        /* A literal's text CPython would have interned as a code constant
         * (its characters all ASCII letters, digits and `_`, as CPython's
         * `all_name_chars`; and a one-character string, a singleton in
         * CPython) crosses as the interned string: `'raw' is 'raw'` holds
         * across the boundary as in CPython, and a dict or attribute lookup
         * keyed by it finds the key by identity, not by comparing text. */
        if (pycc_rt_str_is_immortal(value) && pycc_ext_name_chars(bytes, len)) {
            PyUnicode_InternInPlace(&packed);
        }
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
/*
 * The `kwnames` tuple of a call site's keyword names (a new reference), so
 * a keyword call builds and interns it once rather than on every call.
 * The names are constants of the generated code, one address per text
 * (`pycc_ext_attr_name`), so a site is recognized by its name addresses;
 * a slot holds one site, and a colliding site replaces it. A call with
 * more names than a slot records builds its tuple every time. NULL with
 * an exception set when the tuple cannot be made.
 */
#define PYCC_EXT_KWNAMES_SLOTS 1024
#define PYCC_EXT_KWNAMES_SLOT_BITS 10
#define PYCC_EXT_KWNAMES_MAX 8
static struct {
    const char *names[PYCC_EXT_KWNAMES_MAX];
    long long nkw;
    PyObject *tuple;
} pycc_ext_kwnames_cache[PYCC_EXT_KWNAMES_SLOTS];

static PyObject *pycc_ext_kwnames(const char **names, long long nkw)
{
    size_t slot = nkw <= 0 ? 0 : (size_t)(((uint64_t)(uintptr_t)names[0] * 0x9E3779B97F4A7C15ull)
                           >> (64 - PYCC_EXT_KWNAMES_SLOT_BITS))
                  & (PYCC_EXT_KWNAMES_SLOTS - 1);
    PyObject *tuple;
    long long i;

    if (nkw <= 0) {
        return PyTuple_New(0);
    }
    if (nkw <= PYCC_EXT_KWNAMES_MAX && pycc_ext_kwnames_cache[slot].nkw == nkw) {
        for (i = 0; i < nkw && pycc_ext_kwnames_cache[slot].names[i] == names[i]; i++) {
        }
        if (i == nkw) {
            return Py_NewRef(pycc_ext_kwnames_cache[slot].tuple);
        }
    }
    tuple = PyTuple_New((Py_ssize_t)nkw);
    for (i = 0; tuple != NULL && i < nkw; i++) {
        PyObject *name = pycc_ext_attr_name(names[i]);
        if (name == NULL || PyTuple_SetItem(tuple, (Py_ssize_t)i, Py_NewRef(name)) < 0) {
            Py_CLEAR(tuple);
        }
    }
    if (tuple != NULL && nkw <= PYCC_EXT_KWNAMES_MAX) {
        Py_XDECREF(pycc_ext_kwnames_cache[slot].tuple);
        for (i = 0; i < nkw; i++) {
            pycc_ext_kwnames_cache[slot].names[i] = names[i];
        }
        pycc_ext_kwnames_cache[slot].nkw = nkw;
        pycc_ext_kwnames_cache[slot].tuple = Py_NewRef(tuple);
    }
    return tuple;
}

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
        kwnames = pycc_ext_kwnames(names, nkw);
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
