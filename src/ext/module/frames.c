/*
 * Compiled frames (v0.8 accept 3): a compiled function's traceback entries
 * as real CPython frames.
 *
 * Codegen emits a `PyccFnInfo` record per compiled function (its name,
 * file, first line and the source position of each statement;
 * `pycc_codegen::frames`), and an exception collects one `PyccTbRecord` per
 * compiled frame it is raised in or propagates into (`pycc_rt::frames`).
 * When that exception becomes a CPython object -- leaving the module
 * (`pycc_ext_raise_pending`) or bound by an `except ... as e`
 * (`pycc_ext_exception_cpython`) -- `pycc_ext_tb_sync` adds the records not
 * yet added to its `__traceback__`, outermost last, so CPython's traceback
 * machinery prints a compiled frame exactly as it prints a Python one.
 *
 * Each record's frame is a real `PyFrameObject` over a code object made for
 * the function on first use (`pycc_fninfo_ready`): the template's
 * fields, with one `NOP` per position after its `RESUME` and a location
 * table placing NOP `k` at position `k`. A frame object for a record is
 * made by linking a CPython 3.13 `_PyInterpreterFrame` for it onto the
 * thread's frame chain, asking `PyEval_GetFrame()` for its object and
 * copying the frame into that object before unlinking it, as CPython's own
 * `take_ownership` does for a returning Python frame.
 *
 * Those two structures, `PyThreadState.current_frame` and `PyCodeObject`'s
 * bytecode offset are not part of the stable ABI. They are mirrored here
 * for CPython 3.13 only, and used only after `pycc_frames_probe` has
 * checked the mirror against the running interpreter; on any other version
 * or a failed probe, compiled frames add no traceback entries, as before
 * (docs/DEVIATIONS.md). Every call below is a stable-ABI function.
 */

typedef struct PyccFnInfo {
    PyObject *code;
    PyObject *func;
    PyObject *globals;
    PyObject *builtins;
    uint16_t *ip0;
    const char *name;
    const char *qualname;
    const char *filename;
    const uint32_t (*positions)[4];
    uint32_t npos;
    uint32_t firstlineno;
    uint32_t flags;
    uint32_t file;
    const char *const *texts;
} PyccFnInfo;

typedef struct PyccTbRecord {
    const PyccFnInfo *info;
    uint32_t pos;
    PyObject *frame;
    const void *activation;
} PyccTbRecord;

/* `PyccFnInfo.flags` (`pycc_codegen::frames`). */
#define PYCC_FN_INFO_MODULE 1u

extern const PyccTbRecord *pycc_rt_exception_tb(void *exc, size_t *len, size_t *synced);
extern void pycc_rt_exception_tb_synced(void *exc, size_t n);

/* CPython 3.13's `_PyInterpreterFrame` (Include/internal/pycore_frame.h). */
typedef struct PyccIFrame {
    PyObject *f_executable;
    struct PyccIFrame *previous;
    PyObject *f_funcobj;
    PyObject *f_globals;
    PyObject *f_builtins;
    PyObject *f_locals;
    PyObject *frame_obj;
    uint16_t *instr_ptr;
    int stacktop;
    uint16_t return_offset;
    char owner;
    PyObject *localsplus[2];
} PyccIFrame;

/* `_frameowner`. */
#define PYCC_FRAME_OWNED_BY_THREAD 0
#define PYCC_FRAME_OWNED_BY_GENERATOR 1
#define PYCC_FRAME_OWNED_BY_FRAME_OBJECT 2
#define PYCC_FRAME_OWNED_BY_CSTACK 3

/* CPython 3.13's `struct _frame` (`PyFrameObject`). */
typedef struct PyccFrameObject {
    PyObject_HEAD
    struct PyccFrameObject *f_back;
    PyccIFrame *f_frame;
    PyObject *f_trace;
    int f_lineno;
    char f_trace_lines;
    char f_trace_opcodes;
    PyObject *f_extra_locals;
    PyObject *f_locals_cache;
    PyObject *_f_frame_data[1];
} PyccFrameObject;

/* Offsets in CPython 3.13's `PyThreadState` and `PyCodeObject`. */
#define PYCC_TSTATE_CURRENT_FRAME 72
#define PYCC_CODE_FIRSTTRACEABLE 184
#define PYCC_CODE_ADAPTIVE 200

/* 3.13 opcodes of the synthetic code objects. */
#define PYCC_OP_NOP 30
#define PYCC_OP_RETURN_CONST 103
#define PYCC_OP_RESUME 149

/* `co_flags` (at this offset) of a signature wrapper's code
 * (`WRAPPER_FLAG` in crates/pycc_ast/src/desugar/ext_signatures.rs). */
#define PYCC_CODE_FLAGS 48
#define PYCC_CO_SIGNATURE 0x40000000

#define PYCC_CO_OPTIMIZED 0x0001
#define PYCC_CO_NEWLOCALS 0x0002

/*
 * A compiled frame's `co_filename`: where the module's source file sits
 * next to the module itself, as CPython names an imported module's frames
 * -- the directory of the module's `__file__` joined with the base name of
 * the source path the module was built from. The build path itself when
 * the module has no `__file__`. A new reference, or NULL.
 *
 * On POSIX, `posixpath.dirname` and `posixpath.join` are done here (one
 * per module, at its import: importing `os.path` and calling into it cost
 * about 15,000 instructions each time); Windows calls `ntpath`, whose
 * drives and separators are not worth repeating.
 */
static PyObject *pycc_frames_filename(PyObject *globals, const char *built)
{
    PyObject *file = PyDict_GetItemString(globals, "__file__");
    const char *base = built;
    const char *p;
#ifdef _WIN32
    PyObject *os_path, *dir, *joined;
#else
    Py_ssize_t len, cut, keep;
    PyObject *head, *joined;
#endif

    for (p = built; *p; p++) {
        if (*p == '/' || *p == '\\') {
            base = p + 1;
        }
    }
    if (file == NULL || !PyUnicode_Check(file)) {
        return PyUnicode_FromString(built);
    }
#ifdef _WIN32
    os_path = PyImport_ImportModule("os.path");
    if (os_path == NULL) {
        return NULL;
    }
    dir = PyObject_CallMethod(os_path, "dirname", "O", file);
    joined = dir == NULL ? NULL : PyObject_CallMethod(os_path, "join", "Os", dir, base);
    Py_XDECREF(dir);
    Py_DECREF(os_path);
    return joined;
#else
    /* `dirname`: everything up to the last `/`, then trailing `/`s off
     * unless the head is all `/`s. */
    len = PyUnicode_GetLength(file);
    if (len < 0) {
        return NULL;
    }
    cut = PyUnicode_FindChar(file, '/', 0, len, -1);
    if (cut == -2) {
        return NULL;
    }
    cut += 1;
    keep = cut;
    while (keep > 0 && PyUnicode_ReadChar(file, keep - 1) == '/') {
        keep--;
    }
    if (keep == 0) {
        keep = cut;
    }
    if (keep == 0) {
        /* `join('', base)` is `base`. */
        return PyUnicode_FromString(base);
    }
    head = PyUnicode_Substring(file, 0, keep);
    if (head == NULL) {
        return NULL;
    }
    /* `join(head, base)`: one `/` between, none added after a `/`. */
    joined = PyUnicode_FromFormat(
        PyUnicode_ReadChar(head, keep - 1) == '/' ? "%U%s" : "%U/%s", head, base);
    Py_DECREF(head);
    return joined;
#endif
}

/* 1 once the mirror is verified, -1 when compiled frames are off, 0 before
 * the first check. */
static int pycc_frames_state = 0;
/* `types.FunctionType`, and the template code object (`def f(): pass`). */
static PyObject *pycc_frames_function_type = NULL;
static PyObject *pycc_frames_template = NULL;
/* The constants, empty tuple and empty bytes every compiled code object
 * shares with the template. */
static PyObject *pycc_frames_consts = NULL;
static PyObject *pycc_frames_empty_tuple = NULL;
static PyObject *pycc_frames_empty_bytes = NULL;

static PyccIFrame **pycc_frames_current(PyThreadState *tstate)
{
    return (PyccIFrame **)((char *)tstate + PYCC_TSTATE_CURRENT_FRAME);
}

static uint16_t *pycc_code_base(PyObject *code)
{
    return (uint16_t *)((char *)code + PYCC_CODE_ADAPTIVE);
}

static int pycc_code_firsttraceable(PyObject *code)
{
    return *(int *)((char *)code + PYCC_CODE_FIRSTTRACEABLE);
}

static int pycc_iframe_incomplete(PyccIFrame *frame)
{
    if (frame->owner == PYCC_FRAME_OWNED_BY_CSTACK) {
        return 1;
    }
    return frame->owner != PYCC_FRAME_OWNED_BY_GENERATOR
           && frame->instr_ptr < pycc_code_base(frame->f_executable)
                                     + pycc_code_firsttraceable(frame->f_executable);
}

/* Appends `value` to `out` as a location-table varint. */
static void pycc_lt_varint(unsigned char **out, uint32_t value)
{
    while (value >= 64) {
        *(*out)++ = (unsigned char)(64 | (value & 63));
        value >>= 6;
    }
    *(*out)++ = (unsigned char)value;
}

static void pycc_lt_svarint(unsigned char **out, int32_t value)
{
    pycc_lt_varint(out, value < 0 ? ((uint32_t)(-value) << 1) | 1 : (uint32_t)value << 1);
}

/*
 * Builds `info`'s code object and function on first use: `RESUME`, one
 * `NOP` per position, `RETURN_CONST 0`, located per Objects/locations.md
 * (a long entry per NOP, a no-column entry where the position has none).
 * 1 when ready; 0 when compiled frames are off or it could not be built
 * (with no CPython exception set).
 */
static int pycc_fninfo_ready(PyccFnInfo *info)
{
    PyObject *bytecode = NULL, *table = NULL, *args = NULL;
    PyObject *code = NULL, *func = NULL, *globals, *filename = NULL;
    unsigned char *buf = NULL, *out;
    Py_ssize_t n = (Py_ssize_t)info->npos + 2, i;
    int32_t line = (int32_t)info->firstlineno;
    PyObject *saved;

    if (info->code != NULL) {
        return 1;
    }
    /* `globals` is the dict of the module the function belongs to, which
     * that module's exec stored (`pycc_ext_fn_info_bind`). */
    globals = info->globals;
    if (pycc_frames_state != 1 || globals == NULL) {
        return 0;
    }
    saved = PyErr_GetRaisedException();
    buf = (unsigned char *)PyMem_Malloc((size_t)n * 2 + 8 + (size_t)n * 24);
    if (buf == NULL || globals == NULL) {
        goto done;
    }
    out = buf;
    *out++ = PYCC_OP_RESUME;
    *out++ = 0;
    for (i = 0; i < (Py_ssize_t)info->npos; i++) {
        *out++ = PYCC_OP_NOP;
        *out++ = 0;
    }
    *out++ = PYCC_OP_RETURN_CONST;
    *out++ = 0;
    bytecode = PyBytes_FromStringAndSize((const char *)buf, out - buf);
    out = buf;
    /* RESUME: the first line, no columns (code 13, length 1). */
    *out++ = 0x80 | (13 << 3);
    pycc_lt_svarint(&out, 0);
    for (i = 0; i < (Py_ssize_t)info->npos; i++) {
        const uint32_t *p = info->positions[i];

        if (p[2] == 0xFFFFFFFFu || p[3] == 0xFFFFFFFFu) {
            *out++ = 0x80 | (13 << 3);
            pycc_lt_svarint(&out, (int32_t)p[0] - line);
        } else {
            *out++ = 0x80 | (14 << 3);
            pycc_lt_svarint(&out, (int32_t)p[0] - line);
            pycc_lt_varint(&out, p[1] - p[0]);
            pycc_lt_varint(&out, p[2] + 1);
            pycc_lt_varint(&out, p[3] + 1);
        }
        line = (int32_t)p[0];
    }
    /* RETURN_CONST: no location (code 15). */
    *out++ = 0x80 | (15 << 3);
    table = PyBytes_FromStringAndSize((const char *)buf, out - buf);
    if (bytecode == NULL || table == NULL) {
        goto done;
    }
    filename = pycc_frames_filename(globals, info->filename);
    if (filename == NULL) {
        goto done;
    }
    /* `types.CodeType(...)` with the template's other fields, positionally:
     * `code.replace(**changes)` built and parsed a dict of keywords. */
    if (pycc_frames_consts == NULL) {
        pycc_frames_consts = PyTuple_Pack(1, Py_None);
        pycc_frames_empty_tuple = PyTuple_New(0);
        pycc_frames_empty_bytes = PyBytes_FromStringAndSize("", 0);
        if (pycc_frames_consts == NULL || pycc_frames_empty_tuple == NULL
            || pycc_frames_empty_bytes == NULL) {
            Py_CLEAR(pycc_frames_consts);
            goto done;
        }
    }
    {
        PyObject *items[18];
        Py_ssize_t k;
        int complete = 1;

        items[0] = PyLong_FromLong(0); /* argcount */
        items[1] = PyLong_FromLong(0); /* posonlyargcount */
        items[2] = PyLong_FromLong(0); /* kwonlyargcount */
        items[3] = PyLong_FromLong(0); /* nlocals */
        items[4] = PyLong_FromLong(1); /* stacksize */
        items[5] = PyLong_FromLong((info->flags & PYCC_FN_INFO_MODULE)
                                       ? 0
                                       : (PYCC_CO_OPTIMIZED | PYCC_CO_NEWLOCALS));
        items[6] = Py_NewRef(bytecode);
        items[7] = Py_NewRef(pycc_frames_consts);
        items[8] = Py_NewRef(pycc_frames_empty_tuple); /* names */
        items[9] = Py_NewRef(pycc_frames_empty_tuple); /* varnames */
        items[10] = Py_NewRef(filename);
        items[11] = PyUnicode_FromString(info->name);
        items[12] = PyUnicode_FromString(info->qualname);
        items[13] = PyLong_FromLong((long)info->firstlineno);
        items[14] = Py_NewRef(table);
        items[15] = Py_NewRef(pycc_frames_empty_bytes); /* exceptiontable */
        items[16] = Py_NewRef(pycc_frames_empty_tuple); /* freevars */
        items[17] = Py_NewRef(pycc_frames_empty_tuple); /* cellvars */
        args = PyTuple_New(18);
        for (k = 0; k < 18; k++) {
            if (items[k] == NULL) {
                complete = 0;
            } else if (args == NULL) {
                Py_DECREF(items[k]);
            } else {
                PyTuple_SetItem(args, k, items[k]);
            }
        }
        if (!complete) {
            Py_CLEAR(args);
        }
    }
    if (args == NULL) {
        goto done;
    }
    code = PyObject_Call((PyObject *)Py_TYPE(pycc_frames_template), args, NULL);
    if (code == NULL) {
        goto done;
    }
    /* A function's globals name its builtins, as a Python module's do: the
     * builtins' dict, which `exec` stores in a module's namespace. */
    if (PyDict_GetItemString(globals, "__builtins__") == NULL
        && PyDict_SetItemString(globals, "__builtins__", PyEval_GetBuiltins()) != 0) {
        goto done;
    }
    func = PyObject_CallFunctionObjArgs(pycc_frames_function_type, code, globals, NULL);
    if (func == NULL) {
        goto done;
    }
    info->builtins = PyEval_GetBuiltins();
    Py_XINCREF(info->builtins);
    info->ip0 = pycc_code_base(code) + 1;
    info->func = func;
    info->code = code;
    func = NULL;
    code = NULL;
done:
    Py_XDECREF(bytecode);
    Py_XDECREF(filename);
    Py_XDECREF(table);
    Py_XDECREF(args);
    Py_XDECREF(code);
    Py_XDECREF(func);
    PyMem_Free(buf);
    PyErr_Clear();
    PyErr_SetRaisedException(saved);
    return info->code != NULL;
}

/*
 * Binds the records of one module's compiled functions (`table`, `count`:
 * its `pycc_fn_info_table`) to `globals`, its dict, at each exec of the
 * module: a frame's `f_globals` and its code's file come from the module
 * the function belongs to, which in a package's extension is not the only
 * module the runtime serves. A record whose code is already made keeps the
 * dict it was made with, as a function object keeps its `__globals__`.
 */
static void pycc_ext_fn_info_bind(PyObject *globals, PyccFnInfo *const *table, long long count)
{
    long long i;

    for (i = 0; i < count; i++) {
        PyccFnInfo *info = table[i];

        if (info->code == NULL && info->globals != globals) {
            Py_XDECREF(info->globals);
            info->globals = Py_NewRef(globals);
        }
    }
}

/* Whether `frame` runs a signature wrapper, which stands for the compiled
 * function it calls: that function's frame links past it. */
static int pycc_iframe_is_wrapper(const PyccIFrame *frame)
{
    return frame->owner == PYCC_FRAME_OWNED_BY_THREAD
           && (*(const int *)((const char *)frame->f_executable + PYCC_CODE_FLAGS)
               & PYCC_CO_SIGNATURE) != 0;
}

/* Fills `frame` as `info`'s frame at position `pos`, linked on top of
 * `tstate`'s chain (past a signature wrapper on top). Returns the top it
 * replaced, which `pycc_iframe_pop` restores. */
static PyccIFrame *pycc_iframe_push(PyThreadState *tstate, PyccIFrame *frame,
                                    const PyccFnInfo *info, uint32_t pos)
{
    PyccIFrame **current = pycc_frames_current(tstate);
    PyccIFrame *top = *current;

    frame->f_executable = info->code;
    frame->previous = top != NULL && pycc_iframe_is_wrapper(top) ? top->previous : top;
    frame->f_funcobj = info->func;
    frame->f_globals = info->globals;
    frame->f_builtins = info->builtins;
    frame->f_locals = (info->flags & PYCC_FN_INFO_MODULE) ? info->globals : NULL;
    frame->frame_obj = NULL;
    frame->instr_ptr = info->ip0 + pos;
    frame->stacktop = 0;
    frame->return_offset = 0;
    frame->owner = PYCC_FRAME_OWNED_BY_THREAD;
    *current = frame;
    return top;
}

/*
 * Unlinks `frame`, the top of `tstate`'s chain. A frame object made for it
 * that outlives it takes the frame over first (CPython's `take_ownership`):
 * the frame is copied into the object, which owns its own references from
 * then on, and its `f_back` is the object of the first complete frame
 * below, made the way `PyEval_GetFrame` makes the top one.
 */
static void pycc_iframe_pop(PyThreadState *tstate, PyccIFrame *frame, PyccIFrame *top)
{
    PyccIFrame **current = pycc_frames_current(tstate);
    PyccFrameObject *f = (PyccFrameObject *)frame->frame_obj;

    if (f != NULL) {
        frame->frame_obj = NULL;
        if (Py_REFCNT((PyObject *)f) > 1) {
            PyccIFrame *copy = (PyccIFrame *)f->_f_frame_data;
            PyccIFrame *prev = frame->previous;
            PyObject *saved = PyErr_GetRaisedException();

            while (prev != NULL && pycc_iframe_incomplete(prev)) {
                prev = prev->previous;
            }
            memcpy(copy, frame, (size_t)((char *)frame->localsplus - (char *)frame));
            Py_INCREF(copy->f_executable);
            Py_INCREF(copy->f_funcobj);
            Py_XINCREF(copy->f_locals);
            copy->previous = NULL;
            copy->owner = PYCC_FRAME_OWNED_BY_FRAME_OBJECT;
            f->f_frame = copy;
            if (prev != NULL && f->f_back == NULL) {
                PyFrameObject *back;

                *current = prev;
                back = PyEval_GetFrame();
                if (back != NULL) {
                    Py_INCREF((PyObject *)back);
                    f->f_back = (PyccFrameObject *)back;
                }
                PyErr_Clear();
            }
            if (!PyObject_GC_IsTracked((PyObject *)f)) {
                PyObject_GC_Track(f);
            }
            PyErr_SetRaisedException(saved);
        }
        Py_DECREF((PyObject *)f);
    }
    *current = top;
}

/* A new frame object for `info` at position `pos`, whose `f_back` is the
 * frame running now; NULL (no exception set) when none can be made. */
static PyObject *pycc_frame_fabricate(PyccFnInfo *info, uint32_t pos)
{
    PyThreadState *tstate;
    PyccIFrame frame;
    PyccIFrame *top;
    PyObject *f;

    if (!pycc_fninfo_ready(info)) {
        return NULL;
    }
    tstate = PyThreadState_Get();
    top = pycc_iframe_push(tstate, &frame, info, pos);
    f = (PyObject *)PyEval_GetFrame();
    Py_XINCREF(f);
    pycc_iframe_pop(tstate, &frame, top);
    if (f == NULL) {
        PyErr_Clear();
    }
    return f;
}

/*
 * Adds the records of the pycc exception `native` that its CPython object
 * `obj` (both borrowed) does not have yet to `obj.__traceback__`, innermost
 * first, so the outermost frame ends up first, as unwinding builds it. Each
 * entry goes through `PyTraceBack_Here`, with `obj` raised and the frame
 * standing at the record's position. CPython's error indicator is left as
 * it was.
 */
static void pycc_ext_tb_sync(void *native, PyObject *obj)
{
    size_t len = 0, synced = 0, i;
    const PyccTbRecord *records;
    PyObject *saved;

    if (pycc_frames_state != 1 || native == NULL || obj == NULL) {
        return;
    }
    records = pycc_rt_exception_tb(native, &len, &synced);
    if (synced >= len) {
        return;
    }
    saved = PyErr_GetRaisedException();
    for (i = synced; i < len; i++) {
        PyccFnInfo *info = (PyccFnInfo *)records[i].info;
        PyObject *f = records[i].frame;
        PyccIFrame *iframe;
        uint16_t *ip;

        if (f != NULL) {
            Py_INCREF(f);
        } else {
            f = pycc_frame_fabricate(info, records[i].pos);
            if (f == NULL) {
                continue;
            }
        }
        iframe = ((PyccFrameObject *)f)->f_frame;
        ip = iframe->instr_ptr;
        iframe->instr_ptr = info->ip0 + records[i].pos;
        Py_INCREF(obj);
        PyErr_SetRaisedException(obj);
        (void)PyTraceBack_Here((PyFrameObject *)f);
        iframe->instr_ptr = ip;
        Py_XDECREF(PyErr_GetRaisedException());
        Py_DECREF(f);
    }
    pycc_rt_exception_tb_synced(native, len);
    PyErr_SetRaisedException(saved);
}

/*
 * Turns compiled frames on when the running interpreter is CPython 3.13
 * and the mirror above matches it: inside the module body's own frame
 * chain, the first complete frame of `current_frame` must be the frame
 * `PyEval_GetFrame` answers, and a probe frame pushed through the mirror
 * must come back from `PyEval_GetFrame` as a frame of the probe's code at
 * the probe's line. Called once per process, from the first exec.
 */
static void pycc_frames_probe(void)
{
    PyObject *types;
    PyThreadState *tstate;
    PyccIFrame *top;
    PyFrameObject *expect;

    if (pycc_frames_state != 0) {
        return;
    }
    pycc_frames_state = -1;
    if ((Py_Version >> 16) != 0x030D) {
        return;
    }
    types = PyImport_ImportModule("types");
    if (types == NULL) {
        PyErr_Clear();
        return;
    }
    pycc_frames_function_type = PyObject_GetAttrString(types, "FunctionType");
    /* The template, the code of `def f(): pass` (`RESUME 0`, `RETURN_CONST
     * 0`, its constants `(None,)`), made with `types.CodeType` rather than
     * compiled: `compile()` cost more than the rest of the probe. */
    {
        static const char code[] = {(char)PYCC_OP_RESUME, 0, (char)PYCC_OP_RETURN_CONST, 0};
        /* Each instruction a no-column entry: line 1, then line 2. */
        static const char lines[] = {(char)0xE8, 0x00, (char)0xE8, 0x02};
        PyObject *code_type = PyObject_GetAttrString(types, "CodeType");

        if (code_type != NULL) {
            pycc_frames_template = PyObject_CallFunction(
                code_type, "iiiiiiy#(O)()()sssiy#y#()()", 0, 0, 0, 0, 1,
                PYCC_CO_OPTIMIZED | PYCC_CO_NEWLOCALS, code, (Py_ssize_t)sizeof code, Py_None,
                "<pycc>", "f", "f", 1, lines, (Py_ssize_t)sizeof lines, "", (Py_ssize_t)0);
            Py_DECREF(code_type);
        }
    }
    Py_DECREF(types);
    PyErr_Clear();
    if (pycc_frames_function_type == NULL) {
        return;
    }
    if (pycc_frames_template == NULL
        || pycc_code_base(pycc_frames_template)[0] != PYCC_OP_RESUME) {
        return;
    }
    {
        PyObject *flags = PyObject_GetAttrString(pycc_frames_template, "co_flags");
        long expected = flags != NULL ? PyLong_AsLong(flags) : -1;

        Py_XDECREF(flags);
        PyErr_Clear();
        if (expected != *(const int *)((const char *)pycc_frames_template + PYCC_CODE_FLAGS)) {
            return;
        }
    }
    tstate = PyThreadState_Get();
    top = *pycc_frames_current(tstate);
    while (top != NULL && pycc_iframe_incomplete(top)) {
        top = top->previous;
    }
    expect = PyEval_GetFrame();
    if ((expect == NULL) != (top == NULL)
        || (expect != NULL && ((PyccFrameObject *)expect)->f_frame != top)) {
        PyErr_Clear();
        return;
    }
    pycc_frames_state = 1;
    {
        static const uint32_t probe_positions[1][4] = {{7, 7, 4, 9}};
        PyccFnInfo probe = {NULL, NULL, NULL, NULL, NULL, "probe", "probe", "<pycc>",
                            probe_positions, 1, 5, 0, 0, NULL};
        PyObject *f;

        /* A namespace of its own: the probe belongs to no module. */
        probe.globals = PyDict_New();
        f = pycc_frame_fabricate(&probe, 0);
        int ok = 0;

        if (f != NULL && PyFrame_GetLineNumber((PyFrameObject *)f) == 7) {
            PyCodeObject *code = PyFrame_GetCode((PyFrameObject *)f);

            ok = (PyObject *)code == probe.code;
            Py_XDECREF((PyObject *)code);
        }
        Py_XDECREF(f);
        Py_XDECREF(probe.code);
        Py_XDECREF(probe.func);
        Py_XDECREF(probe.globals);
        Py_XDECREF(probe.builtins);
        PyErr_Clear();
        if (!ok) {
            pycc_frames_state = -1;
        }
    }
}

/*
 * Adds the compiled frame `info` at position `pos` to the traceback of the
 * CPython exception set now: a module body's CPython operation that failed
 * the import directly, with no pycc exception in between to carry a record.
 */
void pycc_ext_tb_here_raised(PyccFnInfo *info, uint32_t pos)
{
    PyObject *raised;
    PyObject *f;

    if (pycc_frames_state != 1 || !PyErr_Occurred()) {
        return;
    }
    raised = PyErr_GetRaisedException();
    f = pycc_frame_fabricate(info, pos);
    PyErr_SetRaisedException(raised);
    if (f != NULL) {
        (void)PyTraceBack_Here((PyFrameObject *)f);
        Py_DECREF(f);
    }
}

/* `PyccIFrame.owner` of a frame `pycc_ext_frame_enter` did not link. */
#define PYCC_FRAME_INACTIVE 0x7F

extern void pycc_rt_tb_here_frame(const PyccFnInfo *info, uint32_t pos, PyObject *frame,
                                  const void *activation);
extern int pycc_rt_tb_seen(const PyccFnInfo *info, uint32_t pos, const void *activation);

/*
 * A compiled function's prologue: links `frame` (its own storage) for
 * `info` onto the thread's frame chain, at position 0. The thread state and
 * the top it replaced are kept in `localsplus[0]` and `[1]`, which CPython
 * never reads (`stacktop` is 0).
 */
void pycc_ext_frame_enter(PyccIFrame *frame, PyccFnInfo *info)
{
    PyThreadState *tstate;

    if (info->code == NULL && !pycc_fninfo_ready(info)) {
        frame->owner = PYCC_FRAME_INACTIVE;
        return;
    }
    tstate = PyThreadState_Get();
    frame->localsplus[1] = (PyObject *)pycc_iframe_push(tstate, frame, info, 0);
    frame->localsplus[0] = (PyObject *)tstate;
}

/* Its epilogue, on every return: unlinks `frame`. */
void pycc_ext_frame_exit(PyccIFrame *frame)
{
    if (frame->owner == PYCC_FRAME_INACTIVE) {
        return;
    }
    pycc_iframe_pop((PyThreadState *)frame->localsplus[0], frame,
                    (PyccIFrame *)frame->localsplus[1]);
}

/*
 * The pending pycc exception reached position `pos` of the live frame
 * `frame` of `info`: records it with that frame's own frame object when
 * something made one already (`sys._getframe`, `inspect`), so the
 * traceback shows the frame they showed. Otherwise the record's frame is
 * made only if the exception reaches CPython (`pycc_ext_tb_sync`): most
 * exceptions compiled code raises are caught there, and making a frame
 * object for each costs more than the raise.
 */
void pycc_ext_tb_here_live(const PyccFnInfo *info, uint32_t pos, PyccIFrame *frame)
{
    PyObject *f = NULL;

    if (pycc_rt_tb_seen(info, pos, frame)) {
        return;
    }
    if (frame->owner != PYCC_FRAME_INACTIVE && frame->frame_obj != NULL) {
        frame->instr_ptr = info->ip0 + pos;
        f = Py_NewRef(frame->frame_obj);
    }
    pycc_rt_tb_here_frame(info, pos, f, frame);
}
