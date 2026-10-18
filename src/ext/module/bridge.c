/*
 * The fixed half of a pycc `--ext` artifact (D-244, #1025 Part 1).
 *
 * This file is tracked, hand-written, and never generated. `pycc` embeds it
 * with `include_str!` (see `src/ext_build.rs`) and writes it into the build
 * scratch directory next to a small generated companion,
 * `pycc_ext_exports.inc`, which supplies the module name and the per-export
 * wrappers. Both are compiled as one translation unit by the same `cc`
 * invocation that links the artifact -- never during an ordinary
 * `cargo build`, which is what keeps CPython headers off the ordinary build
 * path entirely.
 *
 * Embedded rather than located by source-tree path on purpose: an installed
 * `pycc` has no source tree to look in, so a path-based lookup would work in
 * a checkout and break the moment the compiler is installed.
 *
 * Why C at all, rather than emitting `PyModuleDef`/`PyMethodDef` as LLVM IR:
 * those are exactly the struct layouts the stable ABI exists to hide. Hand-
 * modelling them in IR would re-introduce the version coupling
 * `Py_LIMITED_API` removes.
 *
 * Division of labour with `pycc_rt` (D-244 rule 2): every *decision* about
 * how a value is represented -- the inline-integer range, D-141's bool
 * marker words, the four-way classification of an encoded word, the pending
 * exception's type and message -- belongs to `pycc_rt::ext_bridge` and
 * `pycc_rt::exception`. This file only moves `PyObject*`s according to those
 * answers. That is what keeps `libpycc_rt.a` free of CPython symbols, which
 * matters because the same archive is linked into every `native` executable,
 * where no interpreter exists.
 */

#define Py_LIMITED_API 0x030D0000
#include <Python.h>
#include <string.h>

/*
 * Inline reference counting (v0.8). Under `Py_LIMITED_API >= 3.12` the
 * headers make every `Py_INCREF`/`Py_DECREF` a call into libpython
 * (`_Py_IncRef`/`_Py_DecRef`), so the immortal-object check can change
 * without rebuilding extensions. The count's place is not what may change:
 * `ob_refcnt` at offset 0 of every object is part of the stable ABI, which
 * still runs abi3 extensions built against the 3.2-3.11 limited API, whose
 * macros update it inline and call `_Py_Dealloc` (an `abi_only` stable-ABI
 * function, exported by `python3.dll` too) at zero. These are CPython
 * 3.13's own non-limited inline forms for a 64-bit GIL build: an increment
 * that wraps the low 32 bits to zero is an immortal object's and is
 * skipped, and a decrement skips an object whose low 32 bits are negative
 * (immortal). A free-threaded build loads no abi3 extension; a debug build
 * (`Py_REF_DEBUG`, whose total-count bookkeeping these would skip) and a
 * 32-bit or big-endian one keep the calls. The generated code inlines the
 * same two forms (`pycc_codegen::object_rc`). A closing brace here carries a
 * comment: `shim_split` reads a bare column-0 `}` as a shim function's end.
 */
#if !defined(Py_GIL_DISABLED) && !defined(Py_REF_DEBUG) && SIZEOF_VOID_P == 8 && !PY_BIG_ENDIAN
#define PYCC_INLINE_RC 1
Py_LOCAL_INLINE(void) pycc_incref(PyObject *op) {
    uint32_t *low = (uint32_t *)op;
    uint32_t next = *low + 1;
    if (next != 0) {
        *low = next;
    }
} /* pycc_incref */
Py_LOCAL_INLINE(void) pycc_decref(PyObject *op) {
    Py_ssize_t *count = (Py_ssize_t *)op;
    if ((int32_t)*count < 0) {
        return;
    }
    if (--*count == 0) {
        _Py_Dealloc(op);
    }
} /* pycc_decref */
Py_LOCAL_INLINE(void) pycc_xincref(PyObject *op) {
    if (op != NULL) {
        pycc_incref(op);
    }
} /* pycc_xincref */
Py_LOCAL_INLINE(void) pycc_xdecref(PyObject *op) {
    if (op != NULL) {
        pycc_decref(op);
    }
} /* pycc_xdecref */
Py_LOCAL_INLINE(PyObject *) pycc_newref(PyObject *op) {
    pycc_incref(op);
    return op;
} /* pycc_newref */
#undef Py_INCREF
#undef Py_DECREF
#undef Py_XINCREF
#undef Py_XDECREF
#undef Py_NewRef
#define Py_INCREF(op) pycc_incref(_PyObject_CAST(op))
#define Py_DECREF(op) pycc_decref(_PyObject_CAST(op))
#define Py_XINCREF(op) pycc_xincref(_PyObject_CAST(op))
#define Py_XDECREF(op) pycc_xdecref(_PyObject_CAST(op))
#define Py_NewRef(op) pycc_newref(_PyObject_CAST(op))
#endif

/*
 * `pycc_rt`'s boundary, declared by hand. `pycc_rt` exposes no C header and
 * needs none: these are plain `extern "C"` symbols with fixed signatures.
 */
extern long long pycc_rt_int_from_i64(long long value);
extern long long pycc_rt_ext_int_from_hex(const char *text, size_t len);
extern char *pycc_rt_ext_bigint_hex(long long encoded);
extern void pycc_rt_ext_cstr_free(char *text);
extern long long pycc_rt_ext_bool_encode(int value);
extern int pycc_rt_ext_int_classify(long long encoded);
extern long long pycc_rt_ext_int_decode(long long encoded);
extern int pycc_rt_ext_byte_of_word(long long encoded);
extern int pycc_rt_ext_list_bytes(void *list, unsigned char *out);
extern int pycc_rt_ext_pending_type(void);
extern int pycc_rt_ext_pending_is_unbound_local(void);
extern int pycc_rt_ext_pending_key_is_repr(void);
extern const unsigned char *pycc_rt_ext_pending_message(size_t *len);
extern void pycc_rt_exception_clear(void);
/* D-180 rule 6: a compiled function's scalar return value arrives retained,
 * so a heap-bigint result this boundary refuses still has to be released.
 * A *tuple* element does not arrive retained -- the aggregate return path
 * takes no per-field retain -- so a tuple egress takes its own reference
 * with `pycc_rt_bigint_retain` before handing the word to the packer that
 * discharges one. Both are no-ops for a smallint, a bool marker, and the
 * word `0`, so the pairing stays balanced on every classification. */
extern void pycc_rt_bigint_retain(long long word);
extern void pycc_rt_bigint_release(long long word);
/* v0.6 `int | float` words (`pycc_rt::num`). */
extern long long pycc_rt_num_from_float(double value);
extern double pycc_rt_num_to_float(long long word);
extern signed char pycc_rt_num_is_float(long long word);
extern void pycc_rt_num_release(long long word);
/* The `str` boundary (Part 2 of #1037, #1049). `pycc_rt`'s own `i64` length
 * is `long long` here and its `usize` is `size_t`; `PyStrObj` stays an opaque
 * `void *` on this side, exactly as it is in the generated wrapper. */
extern void *pycc_rt_str_from_literal(const unsigned char *ptr, long long len);
extern void pycc_rt_str_decref(void *s);
extern const unsigned char *pycc_rt_ext_str_bytes(void *s, size_t *len);
/* The CPython `str` a native `str` shares (`PyStrObj::foreign`): borrowed
 * from the getter, adopted by the setter. */
extern void *pycc_rt_str_foreign(void *s);
extern void pycc_rt_str_set_foreign(void *s, void *object);
/* A native `str` of an exact CPython `str`'s UTF-8 text, adopting the
 * reference passed (`pycc_rt_str_adopt_foreign`): no text is copied. */
extern void *pycc_rt_str_adopt_foreign(const unsigned char *ptr, long long len, void *object);
/* The same for a frozenset (`PySetObj::foreign`). */
extern void *pycc_rt_set_foreign(void *s);
extern void pycc_rt_set_set_foreign(void *s, void *object);
extern void *pycc_rt_set_freeze(void *s);
/* The instance boundary (#1145). `pycc_rt_instance_new` allocates a
 * `PyInstanceObj` with `slot_count` unassigned attribute slots and returns
 * it as the opaque `void *` a compiled `Ty::Instance` parameter is. It is
 * the same call `MirExpr::Instantiate` emits for a native `Grid(3, 4)`, so
 * a host-constructed instance and a natively constructed one have
 * identical layout. `layout`/`layout_len` is the class's static,
 * NUL-separated name-then-slot-names descriptor, which words the
 * `AttributeError` of a slot read before its assignment (#1388).
 *
 * Instances are reference counted (`pycc_rt::rc`): a carrier owns one
 * reference to the instance it holds and releases it in its `tp_dealloc`;
 * an instance handed to a compiled function is a reference the function's
 * parameter slot owns, and an instance a compiled function returns is one
 * the caller owns. */
extern void *pycc_rt_instance_new(long long slot_count, const char *layout, size_t layout_len);
extern void pycc_rt_instance_incref(void *instance);
extern void pycc_rt_instance_decref(void *instance);
/* The read half of the instance boundary (#1442): the generated read-only
 * `Py_tp_getset` getters read a slot word with the same checked accessor a
 * compiled `self.x` read uses, so an unassigned slot raises the same
 * `AttributeError` (#1388) through the pending-exception channel. A `str`
 * slot keeps its own reference, so its getter takes one with
 * `pycc_rt_str_incref` before `pycc_ext_pack_str` discharges it. */
extern long long pycc_rt_instance_get_slot_checked(void *instance, long long slot);
extern void pycc_rt_str_incref(void *s);

/* The instance egress (#1435). The class an instance was constructed as
 * (field 0 of its layout descriptor; empty for an instance without one), and
 * the weak back-pointer to the `PyccExtInstance` carrier currently standing
 * for it. `pycc_rt` stores that pointer opaquely and never dereferences it;
 * only this file reads or writes it. See `pycc_ext_obj_pack_instance`. */
extern const unsigned char *pycc_rt_ext_instance_class(void *instance, size_t *len);
extern void *pycc_rt_ext_instance_carrier(void *instance);
extern void pycc_rt_ext_instance_set_carrier(void *instance, void *carrier);

/* `copy.copy` of a carrier (#1455). The clone of an instance with the slot
 * references a per-class kind string calls for (`s`/`i` taken by `pycc_rt`,
 * `o` by `pycc_ext_instance_copy` itself), and the unchecked slot read that
 * finds the `o` words; it answers 0 for an unassigned slot. */
extern void *pycc_rt_ext_instance_copy(void *instance, const char *kinds, size_t kinds_len);
extern long long pycc_rt_instance_get_slot(void *instance, long long slot);

/* A host store into, and `del` of, a slot (Part 1 of #1443), called by the
 * generated slot setters (`src/ext_build/getset/setter.rs`). The kind byte is
 * the copy's: the replaced or deleted word's `s`/`i` reference is released by
 * `pycc_rt`; `o`/`w` release nothing. The delete answers -1 with the checked
 * read's `AttributeError` pending for a slot not assigned. */
extern void pycc_rt_ext_instance_store_slot(void *instance, long long slot, unsigned char kind,
                                            long long word);
extern int pycc_rt_ext_instance_delete_slot(void *instance, long long slot, unsigned char kind);

/* `pycc_rt::ext_bridge`'s classification codes. */
#define PYCC_EXT_INT_SMALLINT 0
#define PYCC_EXT_INT_FALSE 1
#define PYCC_EXT_INT_TRUE 2
#define PYCC_EXT_INT_BIGINT 3

/*
 * The compiled module body, emitted by `pycc_codegen` under
 * `EXT_MODULE_EXEC_SYMBOL`. Returns 0 on success, -1 with the pycc-side
 * exception state pending. Deliberately not named `main`: an extension
 * module that exported `main` would collide with the host interpreter's own.
 */
extern long long pycc_ext_module_exec(void);

/*
 * The pending tag's synthesized user exception class, or NULL when the tag
 * names no class this artifact registered. Defined in the generated
 * companion, which is `#include`d far below this point -- hence the forward
 * declaration here. The returned pointer is *borrowed*: the generated
 * file-scope cache owns the strong reference, exactly as every `PyExc_*`
 * below is a borrowed immortal.
 */
static PyObject *pycc_ext_user_exception_class(unsigned char tag);

/*
 * The reverse: the tag of the most derived class this artifact registered
 * that `exc` is an instance of (a fallback class deriving from a compiled
 * exception class included), with its class-name literal, or -1. Defined
 * in the generated companion too.
 */
static int pycc_ext_user_exception_tag(PyObject *exc, const char **class_name, size_t *class_name_len);

/*
 * Part 7 of #1371: `isinstance(o, C)` for a class `C` compiled in this
 * module. Defined in the generated companion for the same reason as the
 * declaration above (`COMPILED_CLASS_ISINSTANCE_DECL` in `src/ext_build.rs`).
 * Returns 1 when `o` is an instance of the host type object of any
 * *published* class whose MRO contains `name`, -1 with the exception set
 * when `PyObject_IsInstance` raises, and 0 otherwise. A name no published
 * class descends from has no type object to test against, and is answered
 * by `pycc_ext_unpublished_class_isinstance` below. A carrier of a compiled
 * instance (#1435) never reaches it: `pycc_ext_obj_isinstance_compiled`
 * answers that from the instance's run-time class first.
 */
static int pycc_ext_compiled_class_isinstance(PyObject *o, const char *name);

/*
 * The failed-import bridge (#1293, Part 3 of #1282). A foreign import nested
 * in a module-level `if`/`try` block whose `PyImport_ImportModule` raised an
 * `ImportError` is translated into a pending pycc exception, so the block's
 * own `except`/`finally` runs as CPython would run it. `pycc_rt` keeps no
 * CPython dependency (D-244 rule 2): the translation lives here.
 *
 * The pycc exception object is allocated with the same entry points a
 * compiled `raise ModuleNotFoundError("...")` uses. `PyExceptionObj` is an
 * opaque `void *` on this side, as `PyStrObj` is. The tag is the Rust `u8`.
 */
extern void *pycc_rt_exception_value(void);
extern void *pycc_rt_exception_alloc(unsigned char type_tag,
                                     const unsigned char *name,
                                     size_t name_len,
                                     void *message);
extern void pycc_rt_exception_raise(void *obj);
extern void pycc_rt_exception_str_install(void *(*hook)(void *));

/*
 * The two builtin tags the bridge raises, with the class name each one's
 * pycc exception object carries. Named constants rather than `case` labels
 * on purpose: `ext_build_tests`' tag-table drift guard parses the raising
 * switch's `case N:` / `exc_type = PyExc_...;` lines, and its sibling
 * `the_c_shims_import_error_bridge_tags_still_name_their_classes` parses
 * these four `#define`s against `pycc_hir::BUILTIN_EXCEPTION_CLASSES`.
 */
#define PYCC_EXT_TAG_IMPORT_ERROR 26
#define PYCC_EXT_TAG_MODULE_NOT_FOUND_ERROR 27
#define PYCC_EXT_NAME_IMPORT_ERROR "ImportError"
#define PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR "ModuleNotFoundError"

/*
 * #1316: the reserved tag a bridged non-`Exception` `BaseException`
 * (`SystemExit`, `KeyboardInterrupt`, `GeneratorExit`, a non-`Exception`
 * `BaseExceptionGroup`) carries. It is `pycc_rt`'s
 * `EXCEPTION_TYPE_FOREIGN_BASE` and `pycc_hir`'s
 * `FOREIGN_BASE_EXCEPTION_TYPE_TAG`, the one tag above every user class's:
 * `except Exception` does not match it, exactly as in CPython, while a bare
 * `except:` and `finally` still run.
 */
#define PYCC_EXT_TAG_FOREIGN_BASE 255

/*
 * A bridged pycc exception carries the CPython exception it was translated
 * from (`pycc_rt_exception_set_foreign`), which `pycc_rt` releases through
 * the installed object hooks when the pycc exception's last reference goes.
 * When the pycc exception escapes unchanged (unmatched, re-raised with a
 * bare `raise`, or re-raised after `finally`), `pycc_ext_raise_pending`
 * re-raises that original, so the host still sees `.name`, `.path`, the
 * exact class and the traceback. A caught bridged exception -- and with it
 * its original and traceback -- is freed when its handler finishes, so a
 * loop that keeps catching one runs in constant memory.
 */
extern void pycc_rt_exception_set_foreign(void *obj, PyObject *foreign);
extern PyObject *pycc_rt_exception_foreign(void *obj);

/*
 * #1366: the module object whose `Py_mod_exec` body is running on this
 * thread, or NULL outside one. `pycc_ext_obj_import_from` reads it for a
 * relative import, whose package CPython computes from the importing
 * module's own globals (`__package__`, then `__spec__.parent`), exactly as
 * `IMPORT_NAME` passes the frame's globals. Only a build under
 * `--foreign-relative-imports` emits such an import, and only as a
 * module-body item, which runs inside `Py_mod_exec`.
 *
 * Per thread, and saved and restored around each exec rather than just set
 * and cleared, because one artifact's exec can run beneath itself: the same
 * shared object imported under a second name (`top.pkg.m` and `m` share
 * one dlopen handle, so one file static) or reloaded from its own body, and
 * a second thread can import it while the first has released the GIL
 * inside the body's own `__import__`. The key is file-static, and each
 * artifact compiles its own copy of this shim, so another pycc artifact's
 * exec never touches it. Created once per process and never replaced:
 * live wrappers on other threads may still read it.
 *
 * #1199: `pycc_ext_publish` reads it too, to find the module a top-level
 * definition is bound on -- but only after the name it was handed resolved
 * against this artifact's own export and class tables. The embedded
 * launcher runs its program as `__main__` with both tables empty, so every
 * publication call it makes is a no-op that never reads the key, and it
 * never emits a relative import either.
 */
static Py_tss_t *pycc_ext_exec_target_key = NULL;

/*
 * v0.8 accept 2: the module this artifact's body last ran as (a strong
 * reference, replaced by each exec). A relative import in a compiled
 * function runs after the body, outside every exec, and resolves against
 * it: the module whose globals the function's code reads in CPython.
 */
static PyObject *pycc_ext_home_module = NULL;

/*
 * #1199: this exec's own class snapshot, a dict from each published class
 * name to the type object `pycc_ext_register_method_types` made for this
 * `Py_mod_exec`, held for exactly the duration of the body and saved and
 * restored around it like `pycc_ext_exec_target_key`. The type statics are
 * one per process, so a nested exec of this artifact mid-body (the body's
 * cycle drops the module from `sys.modules` and imports it again) replaces
 * them; `pycc_ext_publish` reads the snapshot instead, so each module binds
 * its own exec's class, as every module did before #1199 bound them all
 * ahead of the body. The snapshot's references also keep an outer exec's
 * type alive after the inner exec drops the static's reference to it.
 * Created once per process, on the same never-replaced rule.
 */
static Py_tss_t *pycc_ext_exec_classes_key = NULL;

/*
 * Translates `exc` into a pending pycc exception of `tag`, whose text is
 * CPython's own `str(exc)` and which carries `exc` itself. Shared by both
 * bridges. `class_name` must be a string literal: it outlives the object
 * as `pycc_rt_exception_alloc` requires, which a heap type's `tp_name` does
 * not.
 *
 * Returns 1 having taken over `exc`'s reference, with CPython's error
 * indicator clear (it cannot fail: the 0 its callers handle is no longer
 * returned).
 */
static int pycc_ext_bridge_store(PyObject *exc, unsigned char tag, const char *class_name,
                                 size_t class_name_len)
{
    void *obj;

    /* No message: the text is CPython's own `str(exc)`, rendered from the
     * original only when compiled code reads it
     * (`pycc_ext_exception_str_hook`), since most bridged exceptions are
     * handled without showing it. */
    obj = pycc_rt_exception_alloc(tag, (const unsigned char *)class_name, class_name_len, NULL);
    pycc_rt_exception_set_foreign(obj, exc);
    pycc_rt_exception_raise(obj);
    return 1;
}

/*
 * The text of a bridged exception (`pycc_rt_exception_str_install`):
 * CPython's `str()` of the original `exc` (borrowed), as a new pycc `str`.
 * Whatever CPython exception is set stays set. A `str()` that fails gives
 * the class name, the text the bridge used to fall back to.
 */
static void *pycc_ext_exception_str_hook(void *exc)
{
    PyObject *saved = PyErr_GetRaisedException();
    PyObject *text = PyObject_Str((PyObject *)exc);
    const char *utf8 = NULL;
    Py_ssize_t utf8_len = 0;
    void *message;

    if (text != NULL) {
        utf8 = PyUnicode_AsUTF8AndSize(text, &utf8_len);
    }
    if (utf8 == NULL) {
        PyErr_Clear();
        Py_XDECREF(text);
        text = PyType_GetName(Py_TYPE((PyObject *)exc));
        utf8 = text == NULL ? NULL : PyUnicode_AsUTF8AndSize(text, &utf8_len);
        if (utf8 == NULL) {
            PyErr_Clear();
            utf8 = "";
            utf8_len = 0;
        }
    }
    /* The copy completes before `text`, whose buffer `utf8` points into, is
     * released -- the order `pycc_ext_obj_to_str` documents. */
    message = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)utf8_len);
    Py_XDECREF(text);
    PyErr_SetRaisedException(saved);
    return message;
}

/*
 * Translates a pending pycc exception into a CPython one and clears it.
 * Returns 1 when it raised, 0 when nothing was pending.
 *
 * This is what stops an `ext` artifact killing its host. `native` mode's
 * handler, `pycc_rt_exception_print_and_exit`, calls `exit(1)`; inside an
 * imported extension module that terminates the interpreter rather than
 * failing the call. Both `ext` callers use this instead, with the two
 * different conventions CPython imposes on them: a per-export wrapper then
 * returns NULL, and the `Py_mod_exec` slot returns -1.
 *
 * The tag values are fixed by `pycc_hir::exception`'s
 * `BUILTIN_EXCEPTION_CLASSES` array order, which this file cannot see from
 * C: tags 0..=6 are the flat seven, which `pycc_rt::exception` also names as
 * `EXCEPTION_TYPE_*` constants, and tags 7..=22 are the PEP 3151 `OSError`
 * family; 25..=29 are `OverflowError`, `ImportError`,
 * `ModuleNotFoundError`, `AssertionError` and `AttributeError`, and 23..=24 (the PEP 654
 * groups) fall to `default:`. Two tests are this switch's drift guard -- `ext_bridge`'s
 * `exception_type_tags_match_the_c_shims_hardcoded_switch` for the seven
 * constants, and `ext_build_tests`'
 * `every_exception_tag_the_c_shim_switches_on_still_names_that_class` for
 * the full array.
 */
/*
 * `KeyError(ast.literal_eval(text))` as a new exception object, or NULL (no
 * Python error set) when `text` is not a literal: the caller then keeps the
 * text itself as the argument.
 */
static PyObject *pycc_ext_key_error_from_repr(PyObject *text)
{
    PyObject *ast = PyImport_ImportModule("ast");
    PyObject *literal_eval = (ast == NULL) ? NULL : PyObject_GetAttrString(ast, "literal_eval");
    PyObject *key = (literal_eval == NULL) ? NULL : PyObject_CallFunctionObjArgs(literal_eval, text, NULL);
    PyObject *raised = (key == NULL) ? NULL : PyObject_CallFunctionObjArgs(PyExc_KeyError, key, NULL);
    Py_XDECREF(key);
    Py_XDECREF(literal_eval);
    Py_XDECREF(ast);
    if (raised == NULL) {
        PyErr_Clear();
    }
    return raised;
}

/* `frames.c`: a pycc exception's compiled-frame traceback entries. */
static void pycc_ext_tb_sync(void *native, PyObject *obj);

/* `pycc_ext_tb_sync` on the CPython exception raised now. */
static void pycc_ext_tb_sync_raised(void *native)
{
    PyObject *raised = PyErr_GetRaisedException();

    pycc_ext_tb_sync(native, raised);
    PyErr_SetRaisedException(raised);
}

static int pycc_ext_raise_pending(void)
{
    int tag = pycc_rt_ext_pending_type();
    PyObject *exc_type;
    PyObject *user_class;
    PyObject *message;
    PyObject *original;
    size_t len = 0;
    const unsigned char *bytes;

    if (tag < 0) {
        return 0;
    }
    /*
     * #1293: a pending exception a bridge produced, escaping unchanged,
     * re-raises CPython's own original object rather than a rebuilt one.
     * Anything pycc raised itself, including an `except*` remainder's
     * freshly allocated group, carries none and takes the switch.
     */
    original = pycc_rt_exception_foreign(pycc_rt_exception_value());
    if (original != NULL) {
        pycc_ext_tb_sync(pycc_rt_exception_value(), original);
        Py_INCREF(original);
        PyErr_SetRaisedException(original);
        pycc_rt_exception_clear();
        return 1;
    }
    switch (tag) {
    case 1:
        exc_type = PyExc_ValueError;
        break;
    case 2:
        exc_type = PyExc_TypeError;
        break;
    case 3:
        exc_type = PyExc_KeyError;
        break;
    case 4:
        exc_type = PyExc_IndexError;
        break;
    case 5:
        exc_type = PyExc_ZeroDivisionError;
        break;
    case 6:
        exc_type = PyExc_RuntimeError;
        break;
    /*
     * Tags 7..=22, the PEP 3151 `OSError` family, in
     * `BUILTIN_EXCEPTION_CLASSES` order. Every one of them is reachable from
     * an `int`-only exported function through a bare `raise`, so flattening
     * them to `Exception` would make `except FileNotFoundError:` on the
     * Python side silently stop matching. Each constructs from a single
     * message argument, which is what `PyErr_SetObject` passes below.
     */
    case 7:
        exc_type = PyExc_OSError;
        break;
    case 8:
        exc_type = PyExc_BlockingIOError;
        break;
    case 9:
        exc_type = PyExc_ChildProcessError;
        break;
    case 10:
        exc_type = PyExc_ConnectionError;
        break;
    case 11:
        exc_type = PyExc_FileExistsError;
        break;
    case 12:
        exc_type = PyExc_FileNotFoundError;
        break;
    case 13:
        exc_type = PyExc_InterruptedError;
        break;
    case 14:
        exc_type = PyExc_IsADirectoryError;
        break;
    case 15:
        exc_type = PyExc_NotADirectoryError;
        break;
    case 16:
        exc_type = PyExc_PermissionError;
        break;
    case 17:
        exc_type = PyExc_ProcessLookupError;
        break;
    case 18:
        exc_type = PyExc_TimeoutError;
        break;
    case 19:
        exc_type = PyExc_BrokenPipeError;
        break;
    case 20:
        exc_type = PyExc_ConnectionAbortedError;
        break;
    case 21:
        exc_type = PyExc_ConnectionRefusedError;
        break;
    case 22:
        exc_type = PyExc_ConnectionResetError;
        break;
    /* Part A of #1038 (#1063): `OverflowError`, appended past the PEP 654
     * groups so every earlier tag keeps its value. Unlike those two it has a
     * `PyExc_*` object that a lone message constructs, so it is switched on. */
    case 25:
        exc_type = PyExc_OverflowError;
        break;
    /* #1292 (Part 2 of #1282): `ImportError` and `ModuleNotFoundError`,
     * appended past `OverflowError` the same way. Both have a `PyExc_*`
     * object a lone message constructs (`PyExc_ModuleNotFoundError` is
     * available under this file's limited API since 3.6). */
    case 26:
        exc_type = PyExc_ImportError;
        break;
    case 27:
        exc_type = PyExc_ModuleNotFoundError;
        break;
    /* #1369: `AssertionError`, appended past `ModuleNotFoundError` the same
     * way -- the class a failing `assert` statement raises. */
    case 28:
        exc_type = PyExc_AssertionError;
        break;
    /* #1388: `AttributeError`, appended past `AssertionError` the same
     * way -- the class a read of a not-yet-assigned instance slot raises. */
    case 29:
        exc_type = PyExc_AttributeError;
        break;
    /* `EOFError`, appended past `AttributeError` the same way -- the class
     * `input()` raises at end of input. */
    case 30:
        exc_type = PyExc_EOFError;
        break;
    /* `NameError`, appended past `EOFError` the same way -- a function's
     * read of a module global not bound yet. */
    case 31:
        /* A function local read before its assignment ran raises CPython's
         * `UnboundLocalError`, which subclasses `NameError`. */
        exc_type = PyExc_NameError;
        if (pycc_rt_ext_pending_is_unbound_local()) {
            exc_type = PyExc_UnboundLocalError;
        }
        break;
    /* `MemoryError`, appended past `NameError` the same way. */
    case 32:
        exc_type = PyExc_MemoryError;
        break;
    /* `StopIteration`, appended past `MemoryError` the same way -- `next()`
     * of an exhausted generator. */
    case 33:
        exc_type = PyExc_StopIteration;
        break;
    /* `GeneratorExit`, appended past `StopIteration` the same way -- what a
     * closed generator raises at its suspended `yield`. */
    case 34:
        exc_type = PyExc_GeneratorExit;
        break;
    /* #1316: a bridged non-`Exception` `BaseException`. Only a fallback --
     * the original's re-raise above normally handles it -- and
     * a named label so the decimal-tag drift guard does not read it as a
     * `BUILTIN_EXCEPTION_CLASSES` index. */
    case PYCC_EXT_TAG_FOREIGN_BASE:
        exc_type = PyExc_BaseException;
        break;
    default:
        /*
         * Tag 0 is `Exception`. So, deliberately, are the two remaining
         * builtin tags -- a hole in the otherwise contiguous switched range,
         * since tags 25..=29 above sit past them -- and every user-defined class:
         *
         *  - tags 23..=24 are `BaseExceptionGroup`/`ExceptionGroup`. The C
         *    API exposes no `PyExc_ExceptionGroup` at all, and the type it
         *    does expose cannot be constructed from a lone message -- PEP
         *    654 requires `(msg, exceptions)`, so `PyErr_SetObject` would
         *    fail during normalization and surface a `TypeError` about the
         *    constructor instead of the program's own error. `Exception`
         *    with the right message is the more truthful of the two.
         *  - a user-defined exception class carries a module-assigned tag
         *    this shim knows nothing about, so the lookup below -- not any
         *    tag arithmetic here -- decides whether the artifact registered
         *    a class for it. A hit replaces `Exception` with the real
         *    class, restoring `except m.MyError:`, `except ValueError:` for
         *    a builtin subclass, and `type(e).__name__`. A miss keeps
         *    `Exception`, which is what leaves the two group tags above (and
         *    any class derived from them, which the table excludes for the
         *    same reason) on the honest fallback.
         */
        exc_type = PyExc_Exception;
        user_class = pycc_ext_user_exception_class((unsigned char)tag);
        if (user_class != NULL) {
            exc_type = user_class;
        }
        break;
    }
    bytes = pycc_rt_ext_pending_message(&len);
    /* Not NUL-terminated: a `PyStrObj` carries an explicit length. */
    message = (bytes == NULL) ? PyUnicode_FromString("")
                              : PyUnicode_FromStringAndSize((const char *)bytes, (Py_ssize_t)len);
    if (message == NULL) {
        /* A MemoryError is already set; keep it and drop the pycc-side one. */
        pycc_rt_exception_clear();
        return 1;
    }
    if (tag == 3 && pycc_rt_ext_pending_key_is_repr()) {
        /* A `KeyError` for a key that is not a `str` carries the key's
         * `repr` (an `int`, `float`, `bool` or a tuple of those): rebuild the
         * key so CPython's exception holds it (`KeyError: 0`, `e.args ==
         * (0,)`), not its text (`KeyError: '0'`). */
        PyObject *raised = pycc_ext_key_error_from_repr(message);
        if (raised != NULL) {
            Py_DECREF(message);
            PyErr_SetRaisedException(raised);
            pycc_ext_tb_sync_raised(pycc_rt_exception_value());
            pycc_rt_exception_clear();
            return 1;
        }
    }
    PyErr_SetObject(exc_type, message);
    Py_DECREF(message);
    pycc_ext_tb_sync_raised(pycc_rt_exception_value());
    pycc_rt_exception_clear();
    return 1;
}

/*
 * The longest `, element %zd` clause `pycc_ext_element_clause` can produce.
 * `Py_ssize_t` is 64-bit on every target this shim builds for, so the widest
 * rendering is `, element -9223372036854775808` at 30 bytes including the
 * terminator; 48 leaves room and is still a stack buffer.
 */
#define PYCC_EXT_ELEMENT_CLAUSE_MAX 48

