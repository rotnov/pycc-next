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
extern int pycc_rt_ext_pending_type(void);
extern int pycc_rt_ext_pending_is_unbound_local(void);
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
/* The `str` boundary (Part 2 of #1037, #1049). `pycc_rt`'s own `i64` length
 * is `long long` here and its `usize` is `size_t`; `PyStrObj` stays an opaque
 * `void *` on this side, exactly as it is in the generated wrapper. */
extern void *pycc_rt_str_from_literal(const unsigned char *ptr, long long len);
extern void pycc_rt_str_decref(void *s);
extern const unsigned char *pycc_rt_ext_str_bytes(void *s, size_t *len);
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
extern void pycc_rt_exception_set_str_form(void *obj, void *text);

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
 * The bridge table: each bridged pycc exception object paired with a strong
 * reference to the CPython exception it was translated from. When the pycc
 * exception escapes unchanged (unmatched, re-raised with a bare `raise`, or
 * re-raised after `finally`), `pycc_ext_raise_pending` finds its pointer
 * here and re-raises the *original*, so the host still sees `.name`,
 * `.path`, the exact class and the traceback.
 *
 * Pointer identity is sound because a `PyExceptionObj` is never freed
 * (`pycc_rt::exception`'s leak-only rule), so no other exception can ever
 * reuse a bridged one's address. A table rather than one slot because a
 * handler can bridge a second failure (`try: import b` inside `except
 * ImportError:`) before re-raising the first.
 *
 * Two bridges fill it: the failed-import bridge (#1293), during module exec
 * only, and the foreign-operation bridge (#1316), from any compiled
 * function. The second makes it unbounded by static sites -- a host may
 * call an exported function any number of times, and every *caught*
 * bridged exception would otherwise stay here forever -- so it is bounded
 * by a watermark instead: every frame that can return to CPython (each
 * generated wrapper, and `pycc_ext_exec_module`) takes
 * `pycc_ext_bridge_mark()` before running compiled code and calls
 * `pycc_ext_bridge_release_to(mark)` on every exit, after
 * `pycc_ext_raise_pending` has looked up the escaping entry. So the table
 * holds at most the entries created during one top-level host call. That is
 * a known limitation, not a per-catch bound: a loop that keeps catching a
 * bridged exception inside one long call keeps every original (and its
 * traceback) alive until that call returns, and an embedded program (D-248)
 * is one `pycc_ext_exec_module` call, so there the table grows for the
 * program's lifetime (docs/RUNTIME.md).
 *
 * Per thread, not process-wide: pycc's pending exception is thread-local,
 * and a foreign call can release the GIL, so a second thread's wrapper exit
 * must never release the first thread's live entries. The key is heap
 * allocated because `Py_tss_t` is an incomplete type under this file's
 * limited API. It is created once per process by `pycc_ext_exec_module`
 * and never deleted (the module is never unloaded); each thread's table is
 * allocated lazily on its first bridge and, TSS having no destructor, is
 * leaked with its buffer when the thread ends. No CPython object leaks
 * with it: the watermark has emptied it by then.
 */
typedef struct {
    void *pycc;
    PyObject *orig;
} pycc_ext_bridge_entry;

typedef struct {
    pycc_ext_bridge_entry *entries;
    Py_ssize_t len;
    Py_ssize_t cap;
} pycc_ext_bridge_table;

static Py_tss_t *pycc_ext_bridge_key = NULL;

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
 * exec never touches it. Created once per process next to
 * `pycc_ext_bridge_key`, on the same never-replaced rule.
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

/* This thread's table, or NULL when it has never bridged. */
static pycc_ext_bridge_table *pycc_ext_bridge_current(void)
{
    if (pycc_ext_bridge_key == NULL) {
        return NULL;
    }
    return (pycc_ext_bridge_table *)PyThread_tss_get(pycc_ext_bridge_key);
}

/*
 * This thread's table with room for one more entry, allocating the table on
 * the thread's first bridge. NULL when it cannot -- no key yet, or an
 * allocation failed -- with CPython's error indicator untouched.
 */
static pycc_ext_bridge_table *pycc_ext_bridge_reserve(void)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_current();

    if (table == NULL) {
        if (pycc_ext_bridge_key == NULL) {
            return NULL;
        }
        table = PyMem_Calloc(1, sizeof(pycc_ext_bridge_table));
        if (table == NULL) {
            return NULL;
        }
        if (PyThread_tss_set(pycc_ext_bridge_key, table) != 0) {
            PyMem_Free(table);
            return NULL;
        }
    }
    if (table->len == table->cap) {
        Py_ssize_t cap = table->cap == 0 ? 4 : table->cap * 2;
        pycc_ext_bridge_entry *grown =
            PyMem_Realloc(table->entries, (size_t)cap * sizeof(pycc_ext_bridge_entry));
        if (grown == NULL) {
            return NULL;
        }
        table->entries = grown;
        table->cap = cap;
    }
    return table;
}

/*
 * The watermark: this thread's current entry count, 0 when it has no
 * table. Taken before a frame runs compiled code.
 */
static Py_ssize_t pycc_ext_bridge_mark(void)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_current();

    return table == NULL ? 0 : table->len;
}

/*
 * Releases every entry above `mark`, which clamps: a mark at or above the
 * current length releases nothing. Called on a frame's exits, after
 * `pycc_ext_raise_pending` has looked up the escaping entry. The caller may
 * hold a set CPython error, which a finalizer run by a released original
 * must not clobber, so it is set aside across the releases and restored
 * afterwards (a NULL round-trips). Each entry leaves the table before its
 * release, so a finalizer that bridges again (and may grow the buffer)
 * never sees a released entry. Every entry it drops
 * is an original the escaping-exception lookup has already passed over, so
 * nothing after this call -- the pack included -- needs one.
 */
static void pycc_ext_bridge_release_to(Py_ssize_t mark)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_current();
    PyObject *saved;

    if (table == NULL || mark >= table->len) {
        return;
    }
    saved = PyErr_GetRaisedException();
    while (table->len > mark) {
        PyObject *orig;
        table->len--;
        orig = table->entries[table->len].orig;
        Py_XDECREF(orig);
    }
    PyErr_SetRaisedException(saved);
}

/*
 * Moves the original CPython exception for the pending pycc exception `obj`
 * back into CPython's error indicator and removes its entry. Returns 1 on a
 * hit, 0 when `obj` was never bridged. `PyErr_SetRaisedException` steals
 * the table's reference, so the removed entry is not released again.
 *
 * The removal preserves order (#1316): a re-entrant inner frame restoring
 * its own entry must never move an outer frame's entry across that outer
 * frame's watermark.
 */
static int pycc_ext_bridge_restore(void *obj)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_current();
    Py_ssize_t i;

    if (table == NULL) {
        return 0;
    }
    for (i = 0; i < table->len; i++) {
        if (table->entries[i].pycc == obj) {
            PyObject *orig = table->entries[i].orig;
            memmove(&table->entries[i], &table->entries[i + 1],
                    (size_t)(table->len - i - 1) * sizeof(pycc_ext_bridge_entry));
            table->len--;
            PyErr_SetRaisedException(orig);
            return 1;
        }
    }
    return 0;
}

/*
 * Translates `exc` into a pending pycc exception of `tag`, whose message is
 * CPython's own `str(exc)`, and keeps `exc` in this thread's table. Shared
 * by both bridges. `class_name` must be a string literal: it outlives the
 * object as `pycc_rt_exception_alloc` requires, which a heap type's
 * `tp_name` does not.
 *
 * Returns 1 having taken over `exc`'s reference, with CPython's error
 * indicator clear. Returns 0 when the table cannot grow or `str(exc)`
 * fails, with `exc` still the caller's, no pycc exception pending, and the
 * error indicator clear. Every fallible step runs before any pycc state is
 * touched.
 */
static int pycc_ext_bridge_store(PyObject *exc, unsigned char tag, const char *class_name,
                                 size_t class_name_len)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_reserve();
    PyObject *text;
    const char *utf8;
    Py_ssize_t utf8_len = 0;
    void *message;
    void *obj;

    if (table == NULL) {
        return 0;
    }
    text = PyObject_Str(exc);
    if (text == NULL) {
        PyErr_Clear();
        return 0;
    }
    utf8 = PyUnicode_AsUTF8AndSize(text, &utf8_len);
    if (utf8 == NULL) {
        Py_DECREF(text);
        PyErr_Clear();
        return 0;
    }
    /* The copy completes before `text`, whose buffer `utf8` points into, is
     * released -- the order `pycc_ext_obj_to_str` documents. The fresh +1
     * is the message's owning reference, exactly as a string literal's is
     * in a compiled `raise ImportError("...")`. */
    message = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)utf8_len);
    Py_DECREF(text);
    /* `str(exc)` can run arbitrary code, and a `__str__` that itself failed
     * a foreign operation and bridged would have moved the buffer: take the
     * table again rather than trusting the pointer from before. The slot is
     * still free -- a nested bridge that grew it reserved its own. */
    table = pycc_ext_bridge_reserve();
    if (table == NULL) {
        pycc_rt_str_decref(message);
        return 0;
    }
    obj = pycc_rt_exception_alloc(tag, (const unsigned char *)class_name, class_name_len, message);
    /* The message already is CPython's `str(exc)`: a bridged `KeyError`
     * must not show the `repr` of it. */
    pycc_rt_exception_set_str_form(obj, message);
    pycc_rt_exception_raise(obj);
    table->entries[table->len].pycc = obj;
    table->entries[table->len].orig = exc;
    table->len++;
    return 1;
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
static int pycc_ext_raise_pending(void)
{
    int tag = pycc_rt_ext_pending_type();
    PyObject *exc_type;
    PyObject *user_class;
    PyObject *message;
    size_t len = 0;
    const unsigned char *bytes;

    if (tag < 0) {
        return 0;
    }
    /*
     * #1293: a pending exception the failed-import bridge produced, escaping
     * unchanged, re-raises CPython's own original object rather than a
     * rebuilt one. A miss -- anything pycc raised itself, including an
     * `except*` remainder's freshly allocated group -- takes the switch.
     */
    if (pycc_ext_bridge_restore(pycc_rt_exception_value())) {
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
        exc_type = pycc_rt_ext_pending_is_unbound_local() ? PyExc_UnboundLocalError
                                                         : PyExc_NameError;
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
     * the bridge-table lookup above normally re-raises the original -- and
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
    PyErr_SetObject(exc_type, message);
    Py_DECREF(message);
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

