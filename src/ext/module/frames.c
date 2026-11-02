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
 * made by linking a CPython 3.13 or 3.15 `_PyInterpreterFrame` for it onto the
 * thread's frame chain, asking `PyEval_GetFrame()` for its object and
 * copying the frame into that object before unlinking it, as CPython's own
 * `take_ownership` does for a returning Python frame.
 *
 * Those two structures, `PyThreadState.current_frame` and `PyCodeObject`'s
 * bytecode offset are not part of the stable ABI. They are mirrored here
 * for CPython 3.13 and 3.15 (`PyccFramesLayout`), and used only after `pycc_frames_probe` has
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

/*
 * `_PyInterpreterFrame` (Include/internal/pycore_frame.h in CPython 3.13,
 * pycore_interpframe_structs.h in 3.15) for a 64-bit GIL build. Its first
 * eight words are the same in both versions (in 3.15 `f_executable` and
 * `f_funcobj` are `_PyStackRef`s, a strong reference being the untagged
 * pointer); what follows differs, so it is read and written through
 * `pycc_frames_layout`:
 *
 *  - 3.13: `int stacktop` at 64, `uint16_t return_offset` at 68, `char
 *    owner` at 70, `localsplus` at 72;
 *  - 3.15: `_PyStackRef *stackpointer` at 64, `return_offset` at 72,
 *    `owner` at 74, `uint8_t visited` at 75, `localsplus` at 80.
 *
 * Two `localsplus` words follow, which CPython never reads (the frame's
 * stack is empty): twelve words in all, what codegen allocates
 * (`pycc_codegen::frames`).
 */
typedef struct PyccIFrame {
    PyObject *f_executable;
    struct PyccIFrame *previous;
    PyObject *f_funcobj;
    PyObject *f_globals;
    PyObject *f_builtins;
    PyObject *f_locals;
    PyObject *frame_obj;
    uint16_t *instr_ptr;
    char tail[32];
} PyccIFrame;

/* `_frameowner` (3.15's `FRAME_OWNED_BY_INTERPRETER` is 3.13's
 * `FRAME_OWNED_BY_CSTACK`, a frame no traceback shows). */
#define PYCC_FRAME_OWNED_BY_THREAD 0
#define PYCC_FRAME_OWNED_BY_GENERATOR 1
#define PYCC_FRAME_OWNED_BY_FRAME_OBJECT 2
#define PYCC_FRAME_OWNED_BY_CSTACK 3

/* The head of `struct _frame` (`PyFrameObject`), the same in 3.13 and
 * 3.15; its `_f_frame_data` is at `pycc_frames_layout.frame_data`. */
typedef struct PyccFrameObject {
    PyObject_HEAD
    struct PyccFrameObject *f_back;
    PyccIFrame *f_frame;
} PyccFrameObject;

/* `PyThreadState.current_frame`, the same in 3.13 and 3.15. */
#define PYCC_TSTATE_CURRENT_FRAME 72

/*
 * What differs between the two versions compiled frames know: the frame's
 * tail (above), `PyFrameObject._f_frame_data`, `PyCodeObject`'s
 * `_co_firsttraceable` and `co_code_adaptive`, and the opcodes of the
 * synthetic code objects -- 3.13 ends one with `RETURN_CONST 0`, 3.15 with
 * `LOAD_CONST 0` and `RETURN_VALUE`, and its `RESUME` has one cache unit.
 * Chosen by `pycc_frames_probe` from `Py_Version`, then checked.
 */
typedef struct PyccFramesLayout {
    unsigned version;
    size_t owner;
    size_t localsplus;
    size_t frame_data;
    size_t code_firsttraceable;
    size_t code_adaptive;
    /* Code units of `RESUME` with its caches: where the first NOP is. */
    unsigned resume_units;
    unsigned char op_nop;
    unsigned char op_resume;
    unsigned char op_return_const; /* 3.13 only */
    unsigned char op_load_const;   /* 3.15 only */
    unsigned char op_return_value; /* 3.15 only */
} PyccFramesLayout;

static const PyccFramesLayout pycc_frames_layout_313 = {0x030D, 70, 72, 64, 184, 200, 1, 30, 149, 103, 0, 0};
static const PyccFramesLayout pycc_frames_layout_315 = {0x030F, 74, 80, 72, 192, 208, 2, 25, 128, 0, 81, 33};
static const PyccFramesLayout *pycc_frames_layout = &pycc_frames_layout_313;

/* Whether the running interpreter's frames are 3.15's (0 for 3.13's),
 * read by codegen's inline frame link (`pycc_codegen::frames`); set by
 * `pycc_frames_probe` before any frame is linked inline. */
static unsigned char pycc_ext_frames_v315 = 0;

#define PYCC_IFRAME_OWNER(frame) (((char *)(frame))[pycc_frames_layout->owner])
#define PYCC_IFRAME_LOCALSPLUS(frame) ((PyObject **)((char *)(frame) + pycc_frames_layout->localsplus))
#define PYCC_FRAME_DATA(f) ((PyccIFrame *)((char *)(f) + pycc_frames_layout->frame_data))

/* An empty stack at the frame's `localsplus` and no call in progress:
 * 3.13's `stacktop` 0, 3.15's `stackpointer` at `localsplus` and
 * `visited` 0 (the GC marks a frame it walked); `owner` as given. */
static void pycc_iframe_set_tail(PyccIFrame *frame, char owner)
{
    char *base = (char *)frame;

    if (pycc_frames_layout->version == 0x030F) {
        *(PyObject ***)(base + 64) = PYCC_IFRAME_LOCALSPLUS(frame);
        *(uint16_t *)(base + 72) = 0;
        base[74] = owner;
        base[75] = 0;
    } else {
        *(int *)(base + 64) = 0;
        *(uint16_t *)(base + 68) = 0;
        base[70] = owner;
    }
}

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
    static PyObject *file_key = NULL;
    PyObject *key = pycc_ext_interned(&file_key, "__file__");
    PyObject *file = key == NULL ? NULL : PyDict_GetItemWithError(globals, key);
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
    if (file == NULL && PyErr_Occurred()) {
        return NULL;
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
    return (uint16_t *)((char *)code + pycc_frames_layout->code_adaptive);
}

static int pycc_code_firsttraceable(PyObject *code)
{
    return *(int *)((char *)code + pycc_frames_layout->code_firsttraceable);
}

static int pycc_iframe_incomplete(PyccIFrame *frame)
{
    char owner = PYCC_IFRAME_OWNER(frame);

    if (owner >= PYCC_FRAME_OWNED_BY_CSTACK) {
        return 1;
    }
    return owner != PYCC_FRAME_OWNED_BY_GENERATOR
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
 * `NOP` per position, `RETURN_CONST 0` (3.15: `LOAD_CONST 0`,
 * `RETURN_VALUE`), located per Objects/locations.md
 * (a long entry per NOP, a no-column entry where the position has none).
 * 1 when ready; 0 when compiled frames are off or it could not be built
 * (with no CPython exception set).
 */
static PyObject *pycc_frames_builtins_key = NULL;

static int pycc_fninfo_ready(PyccFnInfo *info)
{
    PyObject *builtins_key;
    PyObject *bytecode = NULL, *table = NULL, *args = NULL;
    PyObject *code = NULL, *func = NULL, *globals, *filename = NULL;
    unsigned char *buf = NULL, *out;
    const PyccFramesLayout *layout = pycc_frames_layout;
    Py_ssize_t n = (Py_ssize_t)info->npos + 4, i;
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
    *out++ = layout->op_resume;
    *out++ = 0;
    for (i = 1; i < (Py_ssize_t)layout->resume_units; i++) {
        *out++ = 0; /* CACHE */
        *out++ = 0;
    }
    for (i = 0; i < (Py_ssize_t)info->npos; i++) {
        *out++ = layout->op_nop;
        *out++ = 0;
    }
    if (layout->op_return_const != 0) {
        *out++ = layout->op_return_const;
        *out++ = 0;
    } else {
        *out++ = layout->op_load_const;
        *out++ = 0;
        *out++ = layout->op_return_value;
        *out++ = 0;
    }
    bytecode = PyBytes_FromStringAndSize((const char *)buf, out - buf);
    out = buf;
    /* RESUME and its caches: the first line, no columns (code 13). */
    *out++ = (unsigned char)(0x80 | (13 << 3) | (layout->resume_units - 1));
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
    /* The return: no location (code 15). */
    *out++ = (unsigned char)(0x80 | (15 << 3) | (layout->op_return_const != 0 ? 0 : 1));
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
    builtins_key = pycc_ext_interned(&pycc_frames_builtins_key, "__builtins__");
    if (builtins_key == NULL
        || (PyDict_GetItemWithError(globals, builtins_key) == NULL
            && (PyErr_Occurred() || PyDict_SetItem(globals, builtins_key, PyEval_GetBuiltins()) != 0))) {
        goto done;
    }
    func = PyObject_CallFunctionObjArgs(pycc_frames_function_type, code, globals, NULL);
    if (func == NULL) {
        goto done;
    }
    info->builtins = PyEval_GetBuiltins();
    Py_XINCREF(info->builtins);
    info->ip0 = pycc_code_base(code) + layout->resume_units;
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
    return PYCC_IFRAME_OWNER(frame) == PYCC_FRAME_OWNED_BY_THREAD
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
    pycc_iframe_set_tail(frame, PYCC_FRAME_OWNED_BY_THREAD);
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
            PyccIFrame *copy = PYCC_FRAME_DATA(f);
            PyccIFrame *prev = frame->previous;
            PyObject *saved = PyErr_GetRaisedException();

            while (prev != NULL && pycc_iframe_incomplete(prev)) {
                prev = prev->previous;
            }
            memcpy(copy, frame, pycc_frames_layout->localsplus);
            Py_INCREF(copy->f_executable);
            Py_INCREF(copy->f_funcobj);
            Py_XINCREF(copy->f_locals);
            copy->previous = NULL;
            /* Its empty stack at the copy's own `localsplus` (3.15's
             * `frame_dealloc` clears from there to `stackpointer`). */
            pycc_iframe_set_tail(copy, PYCC_FRAME_OWNED_BY_FRAME_OBJECT);
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
 * or 3.15 and the mirror above matches it: inside the module body's own frame
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
    if (sizeof(void *) != 8) {
        return;
    }
    if ((Py_Version >> 16) == 0x030D) {
        pycc_frames_layout = &pycc_frames_layout_313;
    } else if ((Py_Version >> 16) == 0x030F) {
        pycc_frames_layout = &pycc_frames_layout_315;
    } else {
        return;
    }
    pycc_ext_frames_v315 = pycc_frames_layout->version == 0x030F;
    types = PyImport_ImportModule("types");
    if (types == NULL) {
        PyErr_Clear();
        return;
    }
    pycc_frames_function_type = PyObject_GetAttrString(types, "FunctionType");
    /* The template, the code of `def f(): pass` (`RESUME 0`, `RETURN_CONST
     * 0`; 3.15: `RESUME 0`, a cache, `LOAD_CONST 0`, `RETURN_VALUE`; its
     * constants `(None,)`), made with `types.CodeType` rather than
     * compiled: `compile()` cost more than the rest of the probe. */
    {
        const PyccFramesLayout *layout = pycc_frames_layout;
        const char code_313[] = {(char)layout->op_resume, 0, (char)layout->op_return_const, 0};
        const char code_315[] = {(char)layout->op_resume, 0, 0, 0, (char)layout->op_load_const, 0,
                                 (char)layout->op_return_value, 0};
        /* Each instruction a no-column entry: line 1, then line 2 (3.15:
         * each entry two code units). */
        static const char lines_313[] = {(char)0xE8, 0x00, (char)0xE8, 0x02};
        static const char lines_315[] = {(char)0xE9, 0x00, (char)0xE9, 0x02};
        int v315 = layout->version == 0x030F;
        PyObject *code_type = PyObject_GetAttrString(types, "CodeType");

        if (code_type != NULL) {
            pycc_frames_template = PyObject_CallFunction(
                code_type, "iiiiiiy#(O)()()sssiy#y#()()", 0, 0, 0, 0, 1,
                PYCC_CO_OPTIMIZED | PYCC_CO_NEWLOCALS, v315 ? code_315 : code_313,
                v315 ? (Py_ssize_t)sizeof code_315 : (Py_ssize_t)sizeof code_313, Py_None, "<pycc>",
                "f", "f", 1, v315 ? lines_315 : lines_313, (Py_ssize_t)sizeof lines_313, "",
                (Py_ssize_t)0);
            Py_DECREF(code_type);
        }
    }
    Py_DECREF(types);
    PyErr_Clear();
    if (pycc_frames_function_type == NULL) {
        return;
    }
    if (pycc_frames_template == NULL
        || (pycc_code_base(pycc_frames_template)[0] & 0xFF) != pycc_frames_layout->op_resume) {
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
        PYCC_IFRAME_OWNER(frame) = PYCC_FRAME_INACTIVE;
        return;
    }
    tstate = PyThreadState_Get();
    PYCC_IFRAME_LOCALSPLUS(frame)[1] = (PyObject *)pycc_iframe_push(tstate, frame, info, 0);
    PYCC_IFRAME_LOCALSPLUS(frame)[0] = (PyObject *)tstate;
}

/* Its epilogue, on every return: unlinks `frame`. */
void pycc_ext_frame_exit(PyccIFrame *frame)
{
    if (PYCC_IFRAME_OWNER(frame) == PYCC_FRAME_INACTIVE) {
        return;
    }
    pycc_iframe_pop((PyThreadState *)PYCC_IFRAME_LOCALSPLUS(frame)[0], frame,
                    (PyccIFrame *)PYCC_IFRAME_LOCALSPLUS(frame)[1]);
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
    if (PYCC_IFRAME_OWNER(frame) != PYCC_FRAME_INACTIVE && frame->frame_obj != NULL) {
        frame->instr_ptr = info->ip0 + pos;
        f = Py_NewRef(frame->frame_obj);
    }
    pycc_rt_tb_here_frame(info, pos, f, frame);
}
