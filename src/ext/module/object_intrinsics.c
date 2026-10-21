/*
 * v0.9: standard-library functions computed natively at the call sites
 * `pycc_ast::desugar::intrinsics` marks: `html.escape(s[, quote])` and
 * `re.escape(s)` called through a name an import of them binds. A call site
 * hands its callee over; only when that is CPython's own function (by
 * identity) and the argument a `str` is the result computed here, with
 * CPython's results; anything else is the call CPython would make. Part of
 * the one shim translation unit (`src/ext_build.rs`'s `SHIM_C`), after
 * `object_stdlib.c`.
 */

/*
 * `module.function`, read once its module is imported (`sys.modules`, no
 * import of its own), kept in `*slot`. NULL while the module is not
 * imported: the callee cannot be that function then.
 */
static PyObject *pycc_ext_intrinsic_fn(const char *module_name, const char *function,
                                       PyObject **slot)
{
    if (*slot == NULL) {
        PyObject *name = PyUnicode_FromString(module_name);
        PyObject *module = name == NULL ? NULL : PyImport_GetModule(name);

        Py_XDECREF(name);
        if (module != NULL) {
            *slot = PyObject_GetAttrString(module, function);
            Py_DECREF(module);
        }
        PyErr_Clear();
    }
    return *slot;
}

/*
 * `s` with each byte `escape_of` maps to a replacement replaced (every
 * replaced byte is ASCII, so the UTF-8 form is scanned): a new `str`, or `s`
 * itself when `same_if_unchanged` and nothing is replaced. NULL with no
 * exception for a `str` UTF-8 cannot hold (a lone surrogate: the caller
 * hands it to CPython's own implementation), NULL with an exception set for
 * any other failure.
 */
static PyObject *pycc_ext_replace_ascii(PyObject *s, const char *(*escape_of)(char, int),
                                        int flag, int same_if_unchanged)
{
    Py_ssize_t n;
    Py_ssize_t i;
    Py_ssize_t extra = 0;
    const char *in = PyUnicode_AsUTF8AndSize(s, &n);
    char *out;
    char *p;
    PyObject *result;

    if (in == NULL) {
        PyErr_Clear();
        return NULL;
    }
    for (i = 0; i < n; i++) {
        const char *rep = escape_of(in[i], flag);
        if (rep != NULL) {
            extra += (Py_ssize_t)strlen(rep) - 1;
        }
    }
    if (extra == 0 && same_if_unchanged) {
        return Py_NewRef(s);
    }
    out = (char *)PyMem_Malloc((size_t)(n + extra + 1));
    if (out == NULL) {
        return PyErr_NoMemory();
    }
    p = out;
    for (i = 0; i < n; i++) {
        const char *rep = escape_of(in[i], flag);
        if (rep == NULL) {
            *p++ = in[i];
        } else {
            size_t len = strlen(rep);
            memcpy(p, rep, len);
            p += len;
        }
    }
    result = PyUnicode_DecodeUTF8(out, p - out, NULL);
    PyMem_Free(out);
    return result;
}

/* `html.escape`'s replacements, in one pass (CPython replaces `&` first so
 * its own replacements are not escaped again); quotes only when `quote`. */
static const char *pycc_ext_html_escape_of(char c, int quote)
{
    switch (c) {
    case '&':
        return "&amp;";
    case '<':
        return "&lt;";
    case '>':
        return "&gt;";
    case '"':
        return quote ? "&quot;" : NULL;
    case '\'':
        return quote ? "&#x27;" : NULL;
    default:
        return NULL;
    }
}

/* `re.escape`'s `_special_chars_map`: a backslash before each of
 * `()[]{}?*+-|^$\.&~# \t\n\r\v\f`. */
static const char pycc_ext_re_specials[] = "()[]{}?*+-|^$\\.&~# \t\n\r\v\f";
static const char pycc_ext_re_escaped[] =
    "\\(\0\\)\0\\[\0\\]\0\\{\0\\}\0\\?\0\\*\0\\+\0\\-\0\\|\0\\^\0\\$\0\\\\\0\\.\0\\&\0\\~\0"
    "\\#\0\\ \0\\\t\0\\\n\0\\\r\0\\\v\0\\\f";

static const char *pycc_ext_re_escape_of(char c, int unused)
{
    const char *at;

    (void)unused;
    if (c == '\0' || (at = strchr(pycc_ext_re_specials, c)) == NULL) {
        return NULL;
    }
    return pycc_ext_re_escaped + 3 * (at - pycc_ext_re_specials);
}

/*
 * `escape(s)`, `escape(s, q)` or `escape(s, quote=q)` (`mode` says how
 * `quote` was passed: 0 not at all, 1 positionally, 2 by keyword). For
 * CPython's own `html.escape` and a `str`: CPython's five `replace` calls in
 * one pass, `s` itself when nothing needs escaping, as `str.replace`
 * returns it. Every argument borrowed; a new reference, or `None` after
 * bridging a CPython exception.
 */
static PyObject *pycc_ext_html_escape_fn = NULL;

PyObject *pycc_ext_obj_html_escape(PyObject *callee, PyObject *s, PyObject *quote, long long mode)
{
    PyObject *args[2];
    PyObject *result;
    PyObject *fn = pycc_ext_intrinsic_fn("html", "escape", &pycc_ext_html_escape_fn);

    if (callee == fn && fn != NULL && PyUnicode_CheckExact(s)
        && (mode == 0 || quote == Py_True || quote == Py_False)) {
        result = pycc_ext_replace_ascii(s, pycc_ext_html_escape_of,
                                        mode == 0 || quote == Py_True, 1);
        if (result != NULL) {
            return result;
        }
        if (PyErr_Occurred()) {
            return pycc_ext_stdlib_failed();
        }
    }
    args[0] = s;
    args[1] = quote;
    if (mode == 0) {
        result = PyObject_Vectorcall(callee, args, 1, NULL);
    } else if (mode == 1) {
        result = PyObject_Vectorcall(callee, args, 2, NULL);
    } else {
        const char *names[1] = {"quote"};
        PyObject *kwnames = pycc_ext_kwnames(names, 1);

        result = kwnames == NULL ? NULL : PyObject_Vectorcall(callee, args, 1, kwnames);
        Py_XDECREF(kwnames);
    }
    return result == NULL ? pycc_ext_stdlib_failed() : result;
}

/*
 * `re.escape(s)`. For CPython's own and a `str`: `s.translate(
 * _special_chars_map)` computed here -- always a new string, as `translate`
 * makes one (CPython's one-character and empty strings are its shared ones
 * either way).
 */
static PyObject *pycc_ext_re_escape_fn = NULL;

PyObject *pycc_ext_obj_re_escape(PyObject *callee, PyObject *s)
{
    PyObject *result;
    PyObject *fn = pycc_ext_intrinsic_fn("re", "escape", &pycc_ext_re_escape_fn);

    if (callee == fn && fn != NULL && PyUnicode_CheckExact(s)) {
        result = pycc_ext_replace_ascii(s, pycc_ext_re_escape_of, 0, 0);
        if (result != NULL) {
            return result;
        }
        if (PyErr_Occurred()) {
            return pycc_ext_stdlib_failed();
        }
    }
    result = PyObject_Vectorcall(callee, &s, 1, NULL);
    return result == NULL ? pycc_ext_stdlib_failed() : result;
}
