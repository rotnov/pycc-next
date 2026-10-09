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
extern int pycc_rt_ext_int_encode(long long value, long long *out);
extern long long pycc_rt_ext_bool_encode(int value);
extern int pycc_rt_ext_int_classify(long long encoded);
extern long long pycc_rt_ext_int_decode(long long encoded);
extern int pycc_rt_ext_pending_type(void);
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
 * There is deliberately no matching free. Compiled code never releases an
 * instance either (D-107's arena model, narrowed by D-154), so a
 * `pycc_rt_instance_free` called from `tp_dealloc` would be the only
 * deallocation in the program and could release storage a compiled
 * function still aliases -- attribute reads hand out interior pointers.
 * The inner object therefore outlives its carrier; see
 * `pycc_ext_instance_dealloc`. */
extern void *pycc_rt_instance_new(long long slot_count, const char *layout, size_t layout_len);
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
 * Part 1 of #1499 (#1501): the number of this artifact's compiled
 * activations that are live right now, on any thread -- every
 * `pycc_ext_exec_module` body and every generated wrapper (an export, a
 * method, a slot, `Py_tp_init`) between entering compiled code and
 * returning to CPython. The bridge watermark below is already the one
 * bracket every such frame takes before compiled code runs, so
 * `pycc_ext_bridge_mark` counts a frame in. The frame is counted out
 * separately, by `pycc_ext_activation_exit` (or `_status`) wrapped around
 * each of its returns -- not by `pycc_ext_bridge_release_to`, which runs
 * earlier (`src/ext_build_tests/bridge_watermark.rs` pins both pairings).
 * A wrapper stays counted until its return value holds its own host
 * reference and every cleanup that can run a finalizer (the watermark
 * release, a `PyBuffer_Release`) has finished: a finalizer may switch the
 * GIL, and a suspended activation counted out early would let another
 * thread's rebind release the very object this wrapper is about to return.
 *
 * A file static, so it is per artifact: each artifact compiles its own copy
 * of this shim, exactly as it owns its own module-global slots, and every
 * module object created from one shared object (the same artifact imported
 * under a second name) shares both. Read and written only with the GIL
 * held: a free-threaded build is refused at compile time, because
 * CPython's `Python.h` rejects `Py_LIMITED_API` under `Py_GIL_DISABLED`,
 * and a free-threaded interpreter loading this abi3 artifact is refused at
 * import, by the `Py_GetVersion()` guard in `PyInit_`.
 * A frame that released the GIL inside a foreign call is still counted.
 */
static Py_ssize_t pycc_ext_live_activations = 0;

/*
 * The watermark: this thread's current entry count, 0 when it has no
 * table. Taken before a frame runs compiled code, which also counts the
 * frame as a live activation (see `pycc_ext_live_activations`).
 */
static Py_ssize_t pycc_ext_bridge_mark(void)
{
    pycc_ext_bridge_table *table = pycc_ext_bridge_current();

    pycc_ext_live_activations++;
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
 * never sees a released entry. It only truncates the table: the frame is
 * still a live activation while these releases run their finalizers, and
 * is counted out later, by `pycc_ext_activation_exit`. Every entry it drops
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
 * Counts a frame out of `pycc_ext_live_activations` and hands back its
 * return value unchanged. A generated wrapper returns through it on every
 * exit after its watermark mark, so C evaluates the argument -- the pack
 * that gives the return value its own host reference -- before the count
 * drops, and every cleanup before the `return` already ran while counted.
 * Exactly one call per `pycc_ext_bridge_mark`.
 */
static PyObject *pycc_ext_activation_exit(PyObject *result)
{
    pycc_ext_live_activations--;
    return result;
}

/* The same for a frame whose exit is a status (`Py_tp_init`, the module
 * exec slot). */
static int pycc_ext_activation_exit_status(int status)
{
    pycc_ext_live_activations--;
    return status;
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
 *  - The inline-range gate *after* CPython's own overflow check, never
 *    against `i64`: `pycc_rt`'s inline-integer range is [-2^62, 2^62-1], so
 *    an `i64`-shaped check would admit 4611686018427387905 -- a fully
 *    conforming call -- and the compiled body would then abort the process
 *    on it. See D-244's dated amendment and #1040.
 */
static int pycc_ext_unpack_int_at(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                  Py_ssize_t element, long long *out)
{
    long long raw;
    int overflow = 0;
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
    raw = PyLong_AsLongLongAndOverflow(obj, &overflow);
    if (raw == -1 && PyErr_Occurred()) {
        return -1;
    }
    if (overflow != 0 || pycc_rt_ext_int_encode(raw, out) != 0) {
        PyErr_Format(PyExc_OverflowError,
                     "%s() argument %zd%s: int is outside the inline-integer range "
                     "[-2**62, 2**62-1] this pycc version's `ext` boundary supports "
                     "(see #1040)",
                     fn_name, index + 1,
                     pycc_ext_element_clause(where, sizeof where, element));
        return -1;
    }
    return 0;
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
    case PYCC_EXT_INT_BIGINT:
        /* The caller owns this reference (D-180 rule 6) and it dies here:
         * dropping it without releasing would leak the `BigIntObj` on every
         * call the D-244 amendment turns into an `OverflowError`. */
        pycc_rt_bigint_release(encoded);
        PyErr_Format(PyExc_OverflowError,
                     "%s() produced an int outside the inline-integer range "
                     "[-2**62, 2**62-1] this pycc version's `ext` boundary supports "
                     "(see #1040)",
                     fn_name);
        return NULL;
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
 * The bytes belong to `obj` and are copied by `pycc_rt_str_from_literal`, so
 * the caller need not keep them alive. That call hands back a fresh
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
    *out = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);
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
    PyObject *packed;

    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "str result was NULL");
        return NULL;
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
 * The compiled body receives a *strong* reference it never releases:
 * RUNTIME.md's #1092 rule has a function body add no reference-count
 * traffic, and a body may store the parameter somewhere that outlives this
 * call (`self.x = x`), so a borrowed pointer would dangle once the host
 * dropped its own reference. The reference is deliberately leaked, exactly
 * as every other object the body produces is, until the object lifetime
 * model lands; nothing is owed on a later argument's bail path either.
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
 * (`f(x) is x`). Under #1092 a compiled return hands the pointer through
 * unchanged with no reference of its own, so the boundary takes the new
 * reference the CPython calling convention requires of a return value.
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
    return Py_NewRef((PyObject *)result);
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

/*
 * Part 1 of #1026: the module-import helper compiled code calls for a
 * foreign `import numpy`.
 *
 * A thin `PyImport_ImportModule` wrapper and nothing more -- it returns a
 * new reference to the imported module, or NULL with the CPython exception
 * (a `ModuleNotFoundError` for a module that is not on the interpreter's
 * `sys.path`) already set, which is exactly the convention the generated
 * call site checks for. Every *decision* about the import stays with
 * CPython, keeping this file's "only moves `PyObject*`s" division of labour
 * with `pycc_rt` intact.
 *
 * Not `static`, unlike every other helper here: LLVM-generated code
 * declares and calls it by this name (`EXT_OBJ_IMPORT_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 *
 * The generated module body stores the returned reference in a
 * module-level global, which owns it: since Part 1 of #1499 a re-import or
 * rebind of that global releases the previous value, and the last value is
 * never released, because the artifact has no teardown hook to release it
 * from; the module object is in the interpreter's `sys.modules` for that
 * whole lifetime anyway. `docs/RUNTIME.md` records the rule.
 */
PyObject *pycc_ext_obj_import(const char *name)
{
    return PyImport_ImportModule(name);
}

/*
 * #1278: raise CPython's `IMPORT_FROM` `ImportError` for `name` missing from
 * `module`, and return NULL. `pycc_ext_obj_import_from` below documents the
 * contract; this only builds the exception.
 */
static PyObject *pycc_ext_import_from_error(PyObject *module, PyObject *name)
{
    PyObject *pkgname = NULL;
    PyObject *shown = NULL;
    PyObject *pkgpath = NULL;
    PyObject *message = NULL;
    PyObject *raised = NULL;

    if (PyObject_GetOptionalAttrString(module, "__name__", &pkgname) < 0) {
        return NULL;
    }
    if (pkgname != NULL && !PyUnicode_Check(pkgname)) {
        Py_CLEAR(pkgname);
    }
    shown = (pkgname != NULL) ? Py_NewRef(pkgname)
                              : PyUnicode_FromString("<unknown module name>");
    if (shown == NULL) {
        goto done;
    }
    if (PyModule_Check(module)) {
        pkgpath = PyModule_GetFilenameObject(module);
        if (pkgpath == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_SystemError)) {
                goto done;
            }
            /* A module without `__file__`, such as the builtin `itertools`. */
            PyErr_Clear();
        }
    }
    if (pkgpath == NULL || !PyUnicode_Check(pkgpath)) {
        Py_CLEAR(pkgpath);
        message = PyUnicode_FromFormat("cannot import name %R from %R (unknown location)",
                                       name, shown);
    } else {
        message = PyUnicode_FromFormat("cannot import name %R from %R (%S)",
                                       name, shown, pkgpath);
    }
    if (message == NULL) {
        goto done;
    }
    PyErr_SetImportError(message, pkgname, pkgpath);
    /*
     * `PyErr_SetImportError` leaves `name_from` unset; CPython's own
     * `IMPORT_FROM` sets it, and its "Did you mean" suggestion reads it.
     */
    raised = PyErr_GetRaisedException();
    if (raised != NULL && PyObject_SetAttrString(raised, "name_from", name) < 0) {
        Py_DECREF(raised);
        goto done;
    }
    PyErr_SetRaisedException(raised);
done:
    Py_XDECREF(message);
    Py_XDECREF(pkgpath);
    Py_XDECREF(shown);
    Py_XDECREF(pkgname);
    return NULL;
}

/*
 * One CPython `IMPORT_FROM` of `name` from `module` (#1278, shared with
 * #1381's dotted walk): a new reference to the attribute when `module` has
 * one; on a missing attribute only, the `sys.modules` entry
 * `<module.__name__>.<name>`; otherwise CPython's `cannot import name`
 * `ImportError` (`pycc_ext_import_from_error`). Any other attribute-lookup
 * failure propagates unchanged. Returns NULL with the exception set on
 * failure; borrows both arguments.
 */
static PyObject *pycc_ext_import_from_step(PyObject *module, PyObject *name)
{
    PyObject *pkgname = NULL;
    PyObject *fullname = NULL;
    PyObject *value = NULL;

    if (PyObject_GetOptionalAttr(module, name, &value) != 0) {
        /* Found (1) or a non-`AttributeError` failure (-1): done either way. */
        return value;
    }
    if (PyObject_GetOptionalAttrString(module, "__name__", &pkgname) < 0) {
        return NULL;
    }
    if (pkgname != NULL && PyUnicode_Check(pkgname)) {
        fullname = PyUnicode_FromFormat("%U.%U", pkgname, name);
        if (fullname == NULL) {
            goto done;
        }
        value = PyImport_GetModule(fullname);
        if (value != NULL || PyErr_Occurred()) {
            goto done;
        }
    }
    pycc_ext_import_from_error(module, name);
done:
    Py_XDECREF(fullname);
    Py_XDECREF(pkgname);
    return value;
}

/*
 * #1278: the helper compiled code calls for each name of a foreign
 * `from itertools import product, chain`.
 *
 * It mirrors CPython 3.14's `IMPORT_NAME` with a fromlist followed by one
 * `IMPORT_FROM`, and returns a new reference to the value bound to
 * `fromlist[index]`, or NULL with the CPython exception set:
 *
 * 1. The import runs `builtins.__import__(module, None, None, fromlist, 0)`
 *    with the statement's *whole* fromlist, as `IMPORT_NAME` does, so a
 *    package whose submodule import sets another listed attribute still
 *    works. It runs once per name rather than once per statement; that
 *    repeats no binding and raises nothing new, because
 *    `_handle_fromlist` skips every name already bound. The two
 *    observable differences -- an overridden `__import__` sees N calls,
 *    and a failed finder lookup for a missing `pkg.<name>` repeats -- are
 *    recorded in `docs/RUNTIME.md`.
 * 2. `IMPORT_FROM`: the attribute when the module has one. On a missing
 *    attribute only, the `sys.modules` entry `<mod.__name__>.<name>`, so a
 *    submodule the fromlist import just loaded (`from xml import dom`) is
 *    found. Any other attribute-lookup failure propagates unchanged.
 * 3. Otherwise CPython's own `ImportError`: `cannot import name 'n' from
 *    'm' (<__file__>)`, or `(unknown location)` when the module has no
 *    file, with `.name`, `.path` and `.name_from` set as CPython sets them.
 *    The message, `.name` and the fallback key come from the module
 *    object's `__name__`, not from the requested string, as CPython's do.
 *    CPython's circular-import and stdlib-shadowing variants of the
 *    message are not reproduced; `docs/RUNTIME.md` records both.
 *
 * `level` is `0` for an absolute import, which passes `None` as the
 * globals. A relative one (#1366; `level` is its dot count, and `module` is
 * the name as written after the dots, `""` for `from . import x`) is only
 * emitted under `pycc build --ext --foreign-relative-imports`, and only as a
 * module-body item, so it runs inside `Py_mod_exec`: it passes the executing
 * module's own dict as the globals, which CPython's `_calc___package__`
 * reads the package from, so a missing sibling, a climb beyond the
 * top-level package and a top-level import with no parent package raise
 * exactly what the `.py` form raises. Outside an exec (never emitted) it
 * raises `SystemError` rather than resolve against the wrong package.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_IMPORT_FROM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 * `module` and every `fromlist` entry are NUL-terminated UTF-8 constants
 * the artifact owns; `index` is always inside `[0, nfrom)` because the
 * compiler emits both. The returned reference is owned by the module
 * global that binds it, on the same rule as `pycc_ext_obj_import`: a
 * re-import or rebind releases it (Part 1 of #1499), and the last value is
 * never released.
 */
PyObject *pycc_ext_obj_import_from(const char *module_name, const char *const *fromlist,
                                   long long nfrom, long long index, long long level)
{
    PyObject *globals = Py_None;
    PyObject *import = NULL;
    PyObject *names = NULL;
    PyObject *module = NULL;
    PyObject *name = NULL;
    PyObject *value = NULL;
    long long i;
    int found;

    if (level > 0) {
        PyObject *target = NULL;

        if (pycc_ext_exec_target_key != NULL) {
            target = (PyObject *)PyThread_tss_get(pycc_ext_exec_target_key);
        }
        if (target == NULL) {
            PyErr_SetString(PyExc_SystemError, "pycc: relative import outside Py_mod_exec");
            return NULL;
        }
        /* Borrowed: the module outlives its own exec. */
        globals = PyModule_GetDict(target);
        if (globals == NULL) {
            return NULL;
        }
    }
    found = PyDict_GetItemStringRef(PyEval_GetBuiltins(), "__import__", &import);
    if (found < 0) {
        return NULL;
    }
    if (found == 0) {
        PyErr_SetString(PyExc_ImportError, "__import__ not found");
        return NULL;
    }
    names = PyTuple_New((Py_ssize_t)nfrom);
    if (names == NULL) {
        goto done;
    }
    for (i = 0; i < nfrom; i++) {
        PyObject *item = PyUnicode_FromString(fromlist[i]);
        if (item == NULL) {
            goto done;
        }
        /* Steals `item`, even on failure. */
        if (PyTuple_SetItem(names, (Py_ssize_t)i, item) < 0) {
            goto done;
        }
    }
    module = PyObject_CallFunction(import, "sOOOi", module_name, globals, Py_None, names,
                                   (int)level);
    if (module == NULL) {
        goto done;
    }
    name = PyUnicode_FromString(fromlist[index]);
    if (name == NULL) {
        goto done;
    }
    value = pycc_ext_import_from_step(module, name);
done:
    Py_XDECREF(name);
    Py_XDECREF(module);
    Py_XDECREF(names);
    Py_XDECREF(import);
    return value;
}

/*
 * #1381 (Part 3 of #1138): the helper compiled code calls for a foreign
 * plain dotted import, `import a.b.c` or `import a.b.c as d`. An undotted
 * `import a` keeps `pycc_ext_obj_import`.
 *
 * It mirrors CPython 3.14's bytecode for both forms and returns a new
 * reference to the bound object, or NULL with the CPython exception set:
 *
 * 1. `IMPORT_NAME`: `builtins.__import__(name, None, None, None, 0)`. With
 *    no fromlist, `__import__` imports every package on the way to the leaf
 *    and returns the *root* package, which `import a.b.c` binds to `a`
 *    (`bind_root` non-zero): the call's result is returned as is.
 * 2. Otherwise `import a.b.c as d` walks from the root with one
 *    `IMPORT_FROM` per remaining segment (`b`, then `c`), exactly as the
 *    compiled `IMPORT_FROM`/`SWAP`/`POP_TOP` chain does, through the step
 *    `pycc_ext_obj_import_from` shares: the attribute, else the
 *    `sys.modules` entry `<__name__>.<segment>`, else CPython's `cannot
 *    import name` `ImportError`. So a package whose `__init__` rebinds a
 *    submodule's name binds that attribute, as in CPython.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_IMPORT_DOTTED_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 * `name` is a NUL-terminated UTF-8 constant the artifact owns, always
 * holding at least one dot. The returned reference is owned by the module
 * global that binds it, on the same rule as `pycc_ext_obj_import`: a
 * re-import or rebind releases it (Part 1 of #1499), and the last value is
 * never released.
 */
PyObject *pycc_ext_obj_import_dotted(const char *name, long long bind_root)
{
    PyObject *import = NULL;
    PyObject *current = NULL;
    PyObject *segment = NULL;
    const char *rest;
    int found;

    found = PyDict_GetItemStringRef(PyEval_GetBuiltins(), "__import__", &import);
    if (found < 0) {
        return NULL;
    }
    if (found == 0) {
        PyErr_SetString(PyExc_ImportError, "__import__ not found");
        return NULL;
    }
    current = PyObject_CallFunction(import, "sOOOi", name, Py_None, Py_None, Py_None, 0);
    Py_DECREF(import);
    if (current == NULL || bind_root) {
        return current;
    }
    rest = strchr(name, '.');
    while (rest != NULL) {
        const char *start = rest + 1;
        PyObject *next;

        rest = strchr(start, '.');
        segment = (rest != NULL)
                      ? PyUnicode_FromStringAndSize(start, (Py_ssize_t)(rest - start))
                      : PyUnicode_FromString(start);
        if (segment == NULL) {
            Py_DECREF(current);
            return NULL;
        }
        next = pycc_ext_import_from_step(current, segment);
        Py_DECREF(segment);
        Py_DECREF(current);
        if (next == NULL) {
            return NULL;
        }
        current = next;
    }
    return current;
}

/*
 * #1293 (Part 3 of #1282): the failure edge of a foreign import nested in a
 * module-level `if`/`try` block, called only after `pycc_ext_obj_import`
 * returned NULL with CPython's exception set.
 *
 * An `ImportError` (a `ModuleNotFoundError` or a subclass of it becomes
 * pycc's tag 27, any other `ImportError` tag 26) is translated into a
 * pending pycc exception whose message is CPython's own `str(exc)`, and the
 * original is kept in the bridge table (see `pycc_ext_bridge_restore`).
 * Returns 1 with CPython's error indicator clear, and the generated code
 * branches to the enclosing handler exactly as an explicit `raise` does.
 *
 * Anything else -- a module body's own `ValueError`, a `SyntaxError`, a
 * `BaseException` -- is left exactly as CPython set it, and so is an
 * `ImportError` this function cannot translate (the table cannot grow, or
 * `str(exc)` fails): it returns 0, and the generated code takes the module
 * body's foreign failure edge (Part 1 of #1096): `pycc_ext_obj_error_bridge`
 * inside a module-level `try`, and the direct `-1` edge out of
 * `Py_mod_exec` outside every one. Every fallible step
 * runs before any pycc state is touched, so a 0 never leaves a pycc
 * exception pending, and a 1 never leaves a CPython one set.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_IMPORT_ERROR_BRIDGE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_import_error_bridge(void)
{
    PyObject *exc = PyErr_GetRaisedException();
    unsigned char tag;
    const char *class_name;
    size_t class_name_len;

    if (exc == NULL) {
        return 0;
    }
    if (!PyErr_GivenExceptionMatches(exc, PyExc_ImportError)) {
        PyErr_SetRaisedException(exc);
        return 0;
    }
    if (PyErr_GivenExceptionMatches(exc, PyExc_ModuleNotFoundError)) {
        tag = PYCC_EXT_TAG_MODULE_NOT_FOUND_ERROR;
        class_name = PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR;
        class_name_len = sizeof(PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR) - 1;
    } else {
        tag = PYCC_EXT_TAG_IMPORT_ERROR;
        class_name = PYCC_EXT_NAME_IMPORT_ERROR;
        class_name_len = sizeof(PYCC_EXT_NAME_IMPORT_ERROR) - 1;
    }
    if (!pycc_ext_bridge_store(exc, tag, class_name, class_name_len)) {
        PyErr_SetRaisedException(exc);
        return 0;
    }
    return 1;
}

/*
 * #1316: the foreign-operation bridge's class mapping. Each line maps one
 * CPython class to the pycc builtin tag and class-name literal a bridged
 * instance of it carries, and the first match wins, so a subclass is
 * listed before its base: `ModuleNotFoundError` before `ImportError`, the
 * four `ConnectionError` children before `ConnectionError`, every PEP 3151
 * subclass before `OSError`. A CPython class pycc does not model maps to
 * its nearest modelled base, and anything else under `Exception` to tag 0.
 * That is sound for compiled code -- `except NameError` is compile-time
 * `T0021`, so no pycc handler can tell -- and
 * the host always sees the original through the bridge table.
 *
 * `ext_build_tests`' `the_c_shims_foreign_error_mapping_names_its_classes`
 * parses these lines against `pycc_hir::BUILTIN_EXCEPTION_CLASSES` and
 * `builtin_exception_parent`, so a renumbered tag, a drifted name, or a
 * base listed ahead of its subclass fails there.
 */
#define PYCC_EXT_OBJ_TAG(py_class, tag_value, literal)                                           \
    if (PyErr_GivenExceptionMatches(exc, py_class)) {                                            \
        *class_name = literal;                                                                   \
        *class_name_len = sizeof(literal) - 1;                                                   \
        return tag_value;                                                                        \
    }

static unsigned char pycc_ext_obj_error_tag(PyObject *exc, const char **class_name,
                                            size_t *class_name_len)
{
    PYCC_EXT_OBJ_TAG(PyExc_ModuleNotFoundError, 27, "ModuleNotFoundError")
    PYCC_EXT_OBJ_TAG(PyExc_ImportError, 26, "ImportError")
    PYCC_EXT_OBJ_TAG(PyExc_OverflowError, 25, "OverflowError")
    PYCC_EXT_OBJ_TAG(PyExc_AssertionError, 28, "AssertionError")
    PYCC_EXT_OBJ_TAG(PyExc_AttributeError, 29, "AttributeError")
    PYCC_EXT_OBJ_TAG(PyExc_BrokenPipeError, 19, "BrokenPipeError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionAbortedError, 20, "ConnectionAbortedError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionRefusedError, 21, "ConnectionRefusedError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionResetError, 22, "ConnectionResetError")
    PYCC_EXT_OBJ_TAG(PyExc_BlockingIOError, 8, "BlockingIOError")
    PYCC_EXT_OBJ_TAG(PyExc_ChildProcessError, 9, "ChildProcessError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionError, 10, "ConnectionError")
    PYCC_EXT_OBJ_TAG(PyExc_FileExistsError, 11, "FileExistsError")
    PYCC_EXT_OBJ_TAG(PyExc_FileNotFoundError, 12, "FileNotFoundError")
    PYCC_EXT_OBJ_TAG(PyExc_InterruptedError, 13, "InterruptedError")
    PYCC_EXT_OBJ_TAG(PyExc_IsADirectoryError, 14, "IsADirectoryError")
    PYCC_EXT_OBJ_TAG(PyExc_NotADirectoryError, 15, "NotADirectoryError")
    PYCC_EXT_OBJ_TAG(PyExc_PermissionError, 16, "PermissionError")
    PYCC_EXT_OBJ_TAG(PyExc_ProcessLookupError, 17, "ProcessLookupError")
    PYCC_EXT_OBJ_TAG(PyExc_TimeoutError, 18, "TimeoutError")
    PYCC_EXT_OBJ_TAG(PyExc_OSError, 7, "OSError")
    PYCC_EXT_OBJ_TAG(PyExc_ZeroDivisionError, 5, "ZeroDivisionError")
    PYCC_EXT_OBJ_TAG(PyExc_KeyError, 3, "KeyError")
    PYCC_EXT_OBJ_TAG(PyExc_IndexError, 4, "IndexError")
    PYCC_EXT_OBJ_TAG(PyExc_ValueError, 1, "ValueError")
    PYCC_EXT_OBJ_TAG(PyExc_TypeError, 2, "TypeError")
    PYCC_EXT_OBJ_TAG(PyExc_RuntimeError, 6, "RuntimeError")
    PYCC_EXT_OBJ_TAG(PyExc_Exception, 0, "Exception")
    /* Not an `Exception` at all: `SystemExit`, `KeyboardInterrupt`,
     * `GeneratorExit`, or a `BaseExceptionGroup` that is not an
     * `ExceptionGroup`. `except Exception` must not catch it. */
    *class_name = "BaseException";
    *class_name_len = sizeof("BaseException") - 1;
    return PYCC_EXT_TAG_FOREIGN_BASE;
}

#undef PYCC_EXT_OBJ_TAG

/*
 * #1316: the foreign-operation bridge. Called by compiled code in any
 * function body, and in the module body inside a module-level `try`
 * (Part 1 of #1096), when a foreign operation
 * (`pycc_ext_obj_getattr`, `_call`, `_len`, ...) failed with CPython's
 * exception set. It translates that exception into a pending pycc one and
 * keeps the original in the bridge table, so an enclosing `try` in the
 * compiled function runs exactly as CPython's would, and an exception that
 * escapes unchanged reaches the host as the original object.
 *
 * Total, unlike `pycc_ext_import_error_bridge`: on return a pycc exception
 * is always pending and CPython's error indicator is always clear, because
 * the generated code branches straight to the innermost handler and has no
 * other edge. Its degraded paths:
 *
 *  - No exception set. The shim that failed broke CPython's contract;
 *    bridge a `SystemError` saying so rather than raising nothing.
 *  - The table cannot grow, or `str(exc)` fails. Raise a pycc exception of
 *    the mapped class whose message is that class's name, and drop the
 *    original. The mapped tag, not tag 0, is kept deliberately, so a
 *    degraded `SystemExit` still escapes `except Exception`.
 *
 * Returns 1 always; the `int` return mirrors the import bridge's shape.
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_ERROR_BRIDGE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_obj_error_bridge(void)
{
    PyObject *exc = PyErr_GetRaisedException();
    const char *class_name;
    size_t class_name_len;
    unsigned char tag;
    void *message;

    if (exc == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc: foreign operation failed without setting an exception");
        exc = PyErr_GetRaisedException();
    }
    tag = pycc_ext_obj_error_tag(exc, &class_name, &class_name_len);
    if (pycc_ext_bridge_store(exc, tag, class_name, class_name_len)) {
        return 1;
    }
    /* Defensive: reached only when an allocation or `str(exc)` fails. */
    message = pycc_rt_str_from_literal((const unsigned char *)class_name,
                                       (long long)class_name_len);
    pycc_rt_exception_raise(pycc_rt_exception_alloc(
        tag, (const unsigned char *)class_name, class_name_len, message));
    Py_DECREF(exc);
    PyErr_Clear();
    return 1;
}

/*
 * #1316: a compiled function read a module-level foreign name before the
 * module body bound it -- `f()` called above `import copy`, which D-041's
 * checker cannot rule out. CPython raises `NameError` there, so this does
 * too, and bridges it like any other failed foreign operation. The caller
 * then branches to its innermost handler.
 *
 * `name` is the binding's local name (`c` for `from copy import copy as
 * c`), `len` bytes of UTF-8, not NUL-terminated. Not `static`: LLVM-
 * generated code calls it (`EXT_NAME_ERROR_SYMBOL`).
 */
void pycc_ext_name_error(const unsigned char *name, long long len)
{
    PyErr_Format(PyExc_NameError, "name '%.*s' is not defined", (int)len, (const char *)name);
    (void)pycc_ext_obj_error_bridge();
}

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
 * when it is consumed unbound, is owned by a module global that binds it
 * (Part 1 of #1499), and is otherwise leaked until a later part of #1499;
 * `docs/RUNTIME.md` records all three.
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
PyObject *pycc_ext_obj_getattr(PyObject *obj, const char *name)
{
    if (obj == NULL) {
        return NULL;
    }
    return PyObject_GetAttrString(obj, name);
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
        PyErr_SetString(PyExc_OverflowError,
                        "an int argument to a CPython object's method is outside the "
                        "inline-integer range [-2**62, 2**62-1] this pycc version's `ext` "
                        "boundary supports (see #1040)");
        return NULL;
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

    if (value == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "a str argument to a CPython object's method was NULL");
        return NULL;
    }
    bytes = pycc_rt_ext_str_bytes(value, &len);
    return PyUnicode_FromStringAndSize((const char *)bytes, (Py_ssize_t)len);
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
 * temporary (Part 1 of #1092), a module global that binds it owns it
 * (Part 1 of #1499), and otherwise compiled code leaks it, on the rule
 * `docs/RUNTIME.md` records for this boundary.
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

    for (i = 0; i < total; i++) {
        if (args[i] == NULL) {
            packed = 0;
        }
    }
    if (callable != NULL && packed) {
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
    if (pycc_rt_ext_int_encode((long long)size, out) != 0) {
        PyErr_NoMemory();
        return -1;
    }
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
 * owned by a module global that binds it (Part 1 of #1499), leaked
 * otherwise.
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
 * Part 1 of #1092: release one CPython object temporary
 * (`EXT_OBJ_RELEASE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Generated code calls this exactly once for every new reference a shim
 * producer handed it that is consumed by a borrowing operation and bound
 * nowhere -- the operand of an attribute load, a call argument, a
 * discarded statement value, a condition -- after that operation, or on
 * the failure edge that leaves it (`crates/pycc_codegen/src/object_release.rs`
 * owns the classification). Since Part 1 of #1499 it also releases the
 * value a module-global `object` slot held when that slot is rebound
 * (`crates/pycc_codegen/src/object_slot.rs`). `Py_XDECREF` rather than
 * `Py_DECREF` because a producer that failed left NULL in the value the
 * failure edge releases.
 */
void pycc_ext_obj_release(PyObject *o)
{
    Py_XDECREF(o);
}

/*
 * Part 1 of #1499: take a new reference to a borrowed CPython object
 * (`EXT_OBJ_RETAIN_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * A module-global `object` slot owns the reference it holds, so a store of
 * a *borrowed* value -- another global's (`y = x`), a parameter, `None` --
 * retains it first, and the slot's later rebind releases it without
 * freeing an object something else still names. A compiled-instance
 * attribute store of a borrowed object retains it the same way, since the
 * module global it was read from may now release it. `Py_XINCREF` mirrors
 * `pycc_ext_obj_release`'s NULL tolerance.
 */
void pycc_ext_obj_retain(PyObject *o)
{
    Py_XINCREF(o);
}

/*
 * Part 1 of #1499 (#1501): whether a module-global `object` rebind may
 * release the value it replaced (`EXT_OBJ_REBIND_MAY_RELEASE_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). Only the module-exec body rebinds a
 * global, so the rebinding exec is always one of the live activations; the
 * answer is 1 only when it is the *only* one. Any other live activation of
 * this artifact -- a nested exec (the shared object imported under a
 * second name, or re-imported from its own body), an exec on another
 * thread, or a compiled function a host entered through a wrapper, on this
 * thread or on one that released the GIL mid-call -- may hold the old value
 * borrowed, so the rebind leaks it instead, the direction that can never
 * free an object still in use.
 */
int pycc_ext_obj_rebind_may_release(void)
{
    return pycc_ext_live_activations == 1;
}

/*
 * Part 3 of #1026 (PR 3b of #1082): `o[k]` on a CPython object value
 * (`EXT_OBJ_GETITEM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed, exactly as `pycc_ext_obj_getattr` borrows its own base.
 * `k` is an *owned* reference produced by one of the `pycc_ext_obj_pack_*`
 * helpers above, and this function releases it on every path -- the same
 * arg-slot contract `pycc_ext_obj_call` imposes on the values those helpers
 * produce. Keeping one rule for every packed value is the point: generated
 * code creates a packed reference and hands it to a shim helper, and the
 * shim helper is what releases it, so codegen never has to.
 *
 * The result is a *new* reference, owned as `pycc_ext_obj_call`'s result
 * is (released when an unbound temporary, Part 1 of #1092) --
 * `PyObject_GetItem` hands back a new reference exactly as
 * `PyObject_GetAttrString` does, and the result is the only thing that
 * escapes into compiled code as an `object` value.
 *
 * A NULL `k` means the packer already failed with a CPython exception set
 * (an out-of-range `int`, #1040). Testing for that here rather than in LLVM
 * keeps the emitted code one straight line for the key and leaves the whole
 * operation with one failure edge instead of two, which is the identical
 * argument `pycc_ext_obj_call`'s own NULL scan records. A NULL `o` is
 * defence in depth: `PyObject_GetItem` dereferences `Py_TYPE(o)` with no
 * guard of its own, and the caller's own NULL check already routed a failed
 * producer to the foreign failure edge with its exception set, so
 * returning NULL without setting a second one leaves exactly one exception
 * pending.
 */
PyObject *pycc_ext_obj_getitem(PyObject *o, PyObject *k)
{
    PyObject *result;

    result = (o == NULL || k == NULL) ? NULL : PyObject_GetItem(o, k);
    Py_XDECREF(k);
    return result;
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
 * Part 1 of #1371: `isinstance(o, cls)` with an object `o`
 * (`EXT_OBJ_ISINSTANCE_SYMBOL`). Both operands are borrowed. A `NULL`
 * `cls` selects a builtin class by `builtin`: 0 `int`, 1 `str`, 2 `float`,
 * 3 `bool`, and since Part 7 of #1371 4 `list`, 5 `dict`, 6 `tuple`
 * (`pycc_mir::ObjBuiltinClass::shim_code`). Returns 1, 0, or -1
 * with the exception set (`PyObject_IsInstance` raises `TypeError` for a
 * `cls` that is not a class, and propagates a raising
 * `__instancecheck__`).
 */
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
    return PyObject_IsInstance(o, cls);
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
 * Part 2b of #1371: `item in container` with a CPython object container
 * (`EXT_OBJ_CONTAINS_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `PySequence_Contains` is CPython's own `in`: `__contains__`, then
 * iteration, then the old `__getitem__` protocol, raising `TypeError` when
 * the container supports none of them. Returns 1, 0, or -1 with the
 * exception set; compiled code flips the answer for `not in`.
 *
 * The container is borrowed. The item is always a packed value -- an
 * object item too, through `pycc_ext_obj_pack_object`'s new reference -- and
 * is consumed on every path, a `NULL` one included, exactly as
 * `pycc_ext_obj_getitem` consumes its packed key: a `NULL` item is a failed
 * packer whose exception is already set. A `NULL` container is defence in
 * depth for the reason `pycc_ext_obj_getitem` records.
 */
int pycc_ext_obj_contains(PyObject *container, PyObject *item)
{
    int result;

    result = (container == NULL || item == NULL) ? -1 : PySequence_Contains(container, item);
    Py_XDECREF(item);
    return result;
}

/*
 * Parts 2b and 2c of #1371: builds the `slice` object for `o[start:stop:step]`
 * and `del o[start:stop:step]`, the shared half of `pycc_ext_obj_getslice`
 * and `pycc_ext_obj_delslice`.
 *
 * `present` says which bounds the source spelled (bit 0 start, bit 1 stop,
 * bit 2 step). A present bound is a packed value, consumed here on every
 * path -- a `NULL` one included, which is a failed packer whose exception is
 * already set (an `int` bound outside the packer's range raises
 * `OverflowError` there, #1040) -- and an absent bound's pointer is ignored
 * and handed to `PySlice_New` as `NULL`, which it reads as `None`, the
 * value CPython's own `o[:b]` builds. A `NULL` base is defence in depth for
 * the reason `pycc_ext_obj_getitem` records. Returns a new reference to the
 * slice, or `NULL` with the exception set (or, for a `NULL` base, with
 * whatever exception produced it).
 */
static PyObject *pycc_ext_obj_slice_of(PyObject *o, PyObject *start, PyObject *stop,
                                       PyObject *step, int present)
{
    PyObject *bounds[3];
    PyObject *slice = NULL;
    int failed = (o == NULL);
    int i;

    bounds[0] = (present & 1) ? start : NULL;
    bounds[1] = (present & 2) ? stop : NULL;
    bounds[2] = (present & 4) ? step : NULL;
    for (i = 0; i < 3; i++) {
        if ((present & (1 << i)) && bounds[i] == NULL) {
            failed = 1;
        }
    }
    if (!failed) {
        slice = PySlice_New(bounds[0], bounds[1], bounds[2]);
    }
    for (i = 0; i < 3; i++) {
        Py_XDECREF(bounds[i]);
    }
    return slice;
}

/*
 * Part 2b of #1371: `o[start:stop:step]` with a CPython object `o`
 * (`EXT_OBJ_GETSLICE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * The bounds follow `pycc_ext_obj_slice_of`'s contract, and the base is
 * borrowed. The `slice` object is released after the load. The result is a
 * *new* reference, owned as `pycc_ext_obj_call`'s result is (released when
 * an unbound temporary, Part 1 of #1092), or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_getslice(PyObject *o, PyObject *start, PyObject *stop, PyObject *step,
                                int present)
{
    PyObject *slice = pycc_ext_obj_slice_of(o, start, stop, step, present);
    PyObject *result;

    if (slice == NULL) {
        return NULL;
    }
    result = PyObject_GetItem(o, slice);
    Py_DECREF(slice);
    return result;
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
 * Part 2c of #1371: `del o[start:stop:step]` with a CPython object `o`
 * (`EXT_OBJ_DELSLICE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`), the
 * statement twin of `pycc_ext_obj_getslice`.
 *
 * The bounds follow `pycc_ext_obj_slice_of`'s contract, and the base is
 * borrowed. `PyObject_DelItem` runs the base's own `__delitem__` with the
 * `slice`, which is released afterwards; nothing else outlives the call.
 * Returns `0`, or `-1` with the exception set: a `tuple` raises
 * `TypeError`, and a `dict` raises `KeyError` for the (hashable since
 * 3.12) slice key, exactly as CPython's own `del` does.
 */
int pycc_ext_obj_delslice(PyObject *o, PyObject *start, PyObject *stop, PyObject *step,
                          int present)
{
    PyObject *slice = pycc_ext_obj_slice_of(o, start, stop, step, present);
    int status;

    if (slice == NULL) {
        return -1;
    }
    status = PyObject_DelItem(o, slice);
    Py_DECREF(slice);
    return status;
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
 * Each item written through `*out` is a new reference. A module-global
 * `for` target owns it and releases it on the next trip (Part 1 of #1499);
 * a target that does not own it leaks it, which makes the leak
 * trip-count-linear rather than a fixed cost per statement (#1092).
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

/*
 * Part 1 of #1255: the empty result of a list or set comprehension over a
 * CPython object (`EXT_OBJ_NEW_COLLECTION_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). `kind` is `ObjCollectionKind`'s code
 * there: `0` builds a `list`, `1` a `set`; any other code is a code
 * generator defect and raises `SystemError` rather than guessing.
 *
 * The result is a *new* reference -- the comprehension's value -- released
 * by its consumer when unbound and on the comprehension's failure edges
 * (Part 3 of #1092), owned by a module global that binds it (Part 1 of
 * #1499) and still leaked when bound anywhere else, or `NULL` with the
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
 * `__index__` -- are precisely the contract. Only the *overflow tail* is
 * shared, and it is shared verbatim: the inline-range gate runs after
 * CPython's own overflow check and never against `i64`, because pycc's
 * inline-integer range is [-2**62, 2**62-1] (D-244's 2026-09-12 amendment,
 * #1040). There is deliberately no bigint path here.
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
    long long raw;
    int overflow = 0;

    if (o == NULL || out == NULL) {
        return -1;
    }
    converted = PyNumber_Long(o);
    if (converted == NULL) {
        return -1;
    }
    raw = PyLong_AsLongLongAndOverflow(converted, &overflow);
    Py_DECREF(converted);
    /* `PyLong_AsLongLongAndOverflow` signals an out-of-range value through
     * `overflow` *without* setting an exception, so this arm sees only a real
     * failure -- `pycc_ext_unpack_int_at`'s own ordering, unchanged. */
    if (raw == -1 && PyErr_Occurred()) {
        return -1;
    }
    if (overflow != 0 || pycc_rt_ext_int_encode(raw, out) != 0) {
        PyErr_SetString(PyExc_OverflowError,
                        "int() of a CPython object is outside the inline-integer range "
                        "[-2**62, 2**62-1] this pycc version's `ext` boundary supports "
                        "(see #1040)");
        return -1;
    }
    return 0;
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
int pycc_ext_obj_to_str(PyObject *o, void **out)
{
    PyObject *converted;
    const char *utf8;
    Py_ssize_t size;
    void *copied;

    if (o == NULL || out == NULL) {
        return -1;
    }
    converted = PyObject_Str(o);
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
 * `int` has no bigint path: a value outside pycc's inline-integer range
 * `[-2**62, 2**62-1]` raises `OverflowError` citing #1040, as at every other
 * `ext` crossing. A `str` (or `str` subclass) is copied into a fresh pycc
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
    long long raw;
    int overflow = 0;

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
    raw = PyLong_AsLongLongAndOverflow(o, &overflow);
    if (raw == -1 && PyErr_Occurred()) {
        return -1;
    }
    if (overflow != 0 || pycc_rt_ext_int_encode(raw, out) != 0) {
        PyErr_SetString(PyExc_OverflowError,
                        "an int narrowed by isinstance() is outside the inline-integer range "
                        "[-2**62, 2**62-1] this pycc version's `ext` boundary supports "
                        "(see #1040)");
        return -1;
    }
    return 0;
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
 * returned reference is bound to the unpacking temporary: leaked in a
 * function body on the #1092 rule for a bound object value, owned and
 * released on rebind by a module-global temporary (Part 1 of #1499). `PyTuple_New` plus `PyTuple_SetItem` (which steals) build the
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
 * The shared `Py_tp_dealloc` for every constructible class -- one function
 * rather than one per class, because none of it is class-dependent.
 *
 * The documented heap-type pattern: a `PyType_FromSpec` type is a heap type,
 * every instance of it holds a reference to it, and `tp_dealloc` is what
 * discharges that reference. Omitting the `Py_DECREF(tp)` leaks the type
 * object invisibly -- nothing observable fails, the module just never
 * releases its own types. `Py_tp_free` is fetched through `PyType_GetSlot`
 * because the limited API exposes no other way to reach the type's
 * deallocator, and it is called *before* the type reference is released so
 * `tp` is still live while the object is freed.
 *
 * The inner `PyInstanceObj` is deliberately not freed; the `extern` above
 * carries the reason.
 */
static void pycc_ext_instance_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst = ((PyccExtInstance *)self)->inst;
    /* #1435: an instance that outlives its carrier must not keep naming it.
     * Cleared only when the back-pointer is this carrier: a carrier that a
     * later `tp_init` superseded (see `pycc_ext_carrier_bind`) no longer
     * owns the link. */
    if (inst != NULL && pycc_rt_ext_instance_carrier(inst) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(inst, NULL);
    }
    freefunc tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    tp_free(self);
    Py_DECREF(tp);
}

/*
 * #1435: the carrier type of each class, keyed by the class name its
 * instances' layout descriptors carry. A published class's own type object,
 * and the hidden carrier type of an unpublished class with comparison slots
 * (#1427) or field descriptors (#1448), is entered by the generated
 * `pycc_ext_register_method_types` (`pycc_ext_carrier_register`); any other
 * class -- one with neither -- gets a carrier type whose only method is the
 * shared `__copy__` (#1455) the first time one of its instances crosses
 * (`pycc_ext_carrier_type`, defined after the companion because it needs
 * the module name). The dict holds a strong reference to every type for the
 * rest of the process, which the shim's refusal of subinterpreters licenses
 * exactly as it licenses every other file-scope static here.
 */
static PyObject *pycc_ext_carrier_types = NULL;

static int pycc_ext_carrier_types_ready(void)
{
    if (pycc_ext_carrier_types == NULL) {
        pycc_ext_carrier_types = PyDict_New();
    }
    return pycc_ext_carrier_types == NULL ? -1 : 0;
}

/* Enters a published class's type object as its instances' carrier type.
 * Called from the generated registration only, before any compiled code
 * runs, so a later crossing finds the type that carries the class's
 * methods rather than creating one with only `__copy__`. Returns 0, or -1
 * with a CPython exception set. */
static int pycc_ext_carrier_register(const char *class_name, PyObject *type)
{
    if (pycc_ext_carrier_types_ready() < 0) {
        return -1;
    }
    return PyDict_SetItemString(pycc_ext_carrier_types, class_name, type);
}

/* Stores the instance a successful `tp_init` built and records `self` as
 * its carrier (#1435), so `self` handed out by a method of a
 * host-constructed object is that very object. A re-run `__init__`
 * (`obj.__init__(...)`) replaces the instance; the one it replaces stops
 * naming `self`, because nothing would clear that link once `self` dies and
 * the old instance may still be reachable from compiled code. */
static void pycc_ext_carrier_bind(PyObject *self, void *inst)
{
    void *previous = ((PyccExtInstance *)self)->inst;
    if (previous != NULL && pycc_rt_ext_instance_carrier(previous) == (void *)self) {
        pycc_rt_ext_instance_set_carrier(previous, NULL);
    }
    ((PyccExtInstance *)self)->inst = inst;
    pycc_rt_ext_instance_set_carrier(inst, self);
}

/* Generated companion: module name macros, per-export wrappers, method table. */
/*
 * Part 2b of #1142 (#1164): the buffer egress, defined below but called from
 * the generated wrappers in the companion `#include`d next -- the same
 * forward-declaration shape `pycc_ext_module_exec` already uses in the other
 * direction. The definition cannot move above the include: the exporter
 * type's `PyType_Spec` names `PYCC_EXT_MODULE_NAME_STR`, which the companion
 * defines.
 */
static PyObject *pycc_ext_pack_memoryview(PyccExtBufferView *view);
/*
 * Part 1 of #1175: the caller-owned half of the buffer egress, declared here
 * for the same reason its sibling above is -- the generated wrappers in the
 * companion `#include`d next are its only callers. Unlike its sibling it has
 * no dependency on the exporter type, so only the symmetry keeps the two
 * definitions together.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed(PyObject *owner, const Py_buffer *held,
                                                   const char *fn_name, Py_ssize_t index,
                                                   int writable);
/*
 * Part 2 of #1175 (#1179): the sub-range form of the same egress, declared
 * here for the same reason. A separate entry point rather than three more
 * parameters on the whole-view one, so that every wrapper Part 1 generates
 * keeps generating byte-for-byte what it generated before -- only an export
 * that really can return `b[start:stop]` calls this.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed_slice(PyObject *owner, const Py_buffer *held,
                                                         const char *fn_name, Py_ssize_t index,
                                                         int writable, long long start,
                                                         long long stop);
/*
 * Part 1 of #1447 (#1449): the two halves of a same-module instance
 * crossing, declared here because the generated wrappers in the companion
 * `#include`d next are their only callers and defined after it because both
 * need what the companion defines -- the ingress asks the generated
 * `pycc_ext_carrier_class_isinstance`, and the egress reuses #1435's
 * `pycc_ext_obj_pack_instance`, which needs the module name.
 */
static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                    const char *class_name, void **out);
static PyObject *pycc_ext_pack_instance(void *result);
static int pycc_ext_slot_operand_is(PyObject *other, const char *class_name);
/*
 * #1455: the `__copy__` every carrier type carries, declared here because
 * the generated per-class method tables in the companion name it and defined
 * after it because it calls the generated `pycc_ext_carrier_class_copy_kinds`.
 */
static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused);

/*
 * #1461: keyword arguments at the host boundary, bound by name as CPython
 * binds a Python function's positional-or-keyword parameters
 * (`initialize_locals`): the positional arguments fill the leading slots,
 * then each keyword, in call order, fills the slot it names -- an unknown
 * name and an already filled slot are the `TypeError`s CPython words the
 * same way -- and finally every required slot must be filled. There is no
 * "too many positional arguments" step: with more positional arguments than
 * slots every slot is already filled, so the first keyword raises "multiple
 * values" or "unexpected", which is also what CPython reports.
 *
 * Only a generated wrapper or `Py_tp_init` whose export the driver made
 * keyword-enabled calls these, and only when the host passed at least one
 * keyword; a keyword-free call never enters them. `slots` receives borrowed
 * references only -- the caller's arguments and keyword values -- and a slot
 * left NULL is a defaulted parameter the generated caller fills from its
 * own default object. `names` holds the `arity` parameter names as UTF-8.
 */
static int pycc_ext_kw_place(const char *fn_name, const char *const *names, Py_ssize_t arity,
                             PyObject **slots, PyObject *key, PyObject *value)
{
    Py_ssize_t i;
    if (!PyUnicode_Check(key)) {
        PyErr_Format(PyExc_TypeError, "%s() keywords must be strings", fn_name);
        return -1;
    }
    for (i = 0; i < arity; i++) {
        if (PyUnicode_EqualToUTF8(key, names[i])) {
            if (slots[i] != NULL) {
                PyErr_Format(PyExc_TypeError, "%s() got multiple values for argument '%s'",
                             fn_name, names[i]);
                return -1;
            }
            slots[i] = value;
            return 0;
        }
    }
    PyErr_Format(PyExc_TypeError, "%s() got an unexpected keyword argument '%U'", fn_name, key);
    return -1;
}

/*
 * The required-argument check, worded as CPython's `format_missing`:
 * `'a'`, `'a' and 'b'`, `'a', 'b', and 'c'`.
 */
static int pycc_ext_kw_finish(const char *fn_name, const char *const *names,
                              Py_ssize_t required, PyObject **slots)
{
    Py_ssize_t i;
    Py_ssize_t missing = 0;
    Py_ssize_t listed = 0;
    PyObject *list;
    for (i = 0; i < required; i++) {
        if (slots[i] == NULL) {
            missing++;
        }
    }
    if (missing == 0) {
        return 0;
    }
    list = PyUnicode_FromString("");
    for (i = 0; i < required && list != NULL; i++) {
        PyObject *longer;
        const char *separator;
        if (slots[i] != NULL) {
            continue;
        }
        if (listed == 0) {
            separator = "";
        } else if (missing == 2) {
            separator = " and ";
        } else if (listed == missing - 1) {
            separator = ", and ";
        } else {
            separator = ", ";
        }
        longer = PyUnicode_FromFormat("%U%s'%s'", list, separator, names[i]);
        Py_DECREF(list);
        list = longer;
        listed++;
    }
    if (list == NULL) {
        return -1;
    }
    PyErr_Format(PyExc_TypeError, "%s() missing %zd required positional argument%s: %U",
                 fn_name, missing, missing == 1 ? "" : "s", list);
    Py_DECREF(list);
    return -1;
}

/* The `METH_FASTCALL | METH_KEYWORDS` form: keyword values follow the
 * `nargs` positional ones in `args`, named by the `kwnames` tuple. */
static int pycc_ext_kw_bind_fastcall(const char *fn_name, const char *const *names,
                                     Py_ssize_t arity, Py_ssize_t required, PyObject **slots,
                                     PyObject *const *args, Py_ssize_t nargs, PyObject *kwnames)
{
    Py_ssize_t i;
    Py_ssize_t count = PyTuple_Size(kwnames);
    for (i = 0; i < arity; i++) {
        slots[i] = i < nargs ? args[i] : NULL;
    }
    for (i = 0; i < count; i++) {
        if (pycc_ext_kw_place(fn_name, names, arity, slots, PyTuple_GetItem(kwnames, i),
                              args[nargs + i]) < 0) {
            return -1;
        }
    }
    return pycc_ext_kw_finish(fn_name, names, required, slots);
}

/* The `Py_tp_init` form: a positional tuple and a keyword dict, whose
 * iteration order is the call's keyword order. */
static int pycc_ext_kw_bind_dict(const char *fn_name, const char *const *names, Py_ssize_t arity,
                                 Py_ssize_t required, PyObject **slots, PyObject *args,
                                 PyObject *kwds)
{
    Py_ssize_t i;
    Py_ssize_t nargs = PyTuple_Size(args);
    Py_ssize_t position = 0;
    PyObject *key;
    PyObject *value;
    for (i = 0; i < arity; i++) {
        slots[i] = i < nargs ? PyTuple_GetItem(args, i) : NULL;
    }
    while (PyDict_Next(kwds, &position, &key, &value)) {
        if (pycc_ext_kw_place(fn_name, names, arity, slots, key, value) < 0) {
            return -1;
        }
    }
    return pycc_ext_kw_finish(fn_name, names, required, slots);
}

/*
 * #1427: the `Py_tp_hash` of a carrier type whose class compiles
 * `__hash__`, finishing what the generated `pycc_ext_hash_<Class>` started:
 * `res` is the compiled method's packed result (a new reference) or NULL
 * with the exception it raised. The conversion is CPython's own
 * `slot_tp_hash`: a non-`int` result is a `TypeError`, an `int` too large
 * for `Py_hash_t` hashes as that `int` does, and `-1` -- the error
 * sentinel -- becomes `-2`.
 */
static Py_hash_t pycc_ext_finish_hash(PyObject *res)
{
    Py_hash_t h;

    if (res == NULL) {
        return -1;
    }
    if (!PyLong_Check(res)) {
        Py_DECREF(res);
        PyErr_SetString(PyExc_TypeError, "__hash__ method should return an integer");
        return -1;
    }
    h = PyLong_AsSsize_t(res);
    if (h == -1 && PyErr_Occurred()) {
        PyErr_Clear();
        h = PyObject_Hash(res);
    } else if (h == -1) {
        h = -2;
    }
    Py_DECREF(res);
    return h;
}

/*
 * #1427: `object`'s identity hash, for a carrier type that installs a
 * `Py_tp_richcompare` while its class binds neither `__eq__` nor
 * `__hash__` (an `__lt__`-only class). `inherit_slots` never inherits
 * `tp_hash` next to an overridden `tp_richcompare`, so without this the
 * type would turn unhashable where CPython's class stays hashable.
 */
static Py_hash_t pycc_ext_identity_hash(PyObject *self)
{
    hashfunc base = (hashfunc)PyType_GetSlot(&PyBaseObject_Type, Py_tp_hash);
    return base(self);
}

/*
 * #1427: the `Py_tp_hash` row of a carrier type whose class binds `__eq__`
 * without `__hash__`. It is a marker: `pycc_ext_unhashable_slots` swaps it
 * for `PyObject_HashNotImplemented` before `PyType_FromSpec` reads the
 * array, because `PyType_Ready` publishes `__hash__ = None` only for that
 * exact address, and on Windows a static initializer naming a function
 * imported from `python3.dll` holds the import thunk's address instead.
 * Should it ever be called, it answers exactly as the real slot does.
 */
static Py_hash_t pycc_ext_unhashable(PyObject *self)
{
    return PyObject_HashNotImplemented(self);
}

static void pycc_ext_unhashable_slots(PyType_Slot *slots)
{
    for (; slots->slot != 0; ++slots) {
        if (slots->slot == Py_tp_hash && slots->pfunc == (void *)pycc_ext_unhashable) {
            slots->pfunc = (void *)PyObject_HashNotImplemented;
        }
    }
}

#include "pycc_ext_exports.inc"

/*
 * #1455: `copy.copy(x)` of a carrier. CPython's `copy.copy` calls a type's
 * `__copy__` before any reduce protocol, so this is the whole copy: a new
 * carrier of `type(x)` wrapping a clone of `x`'s compiled instance. The
 * generated table, keyed by the instance's run-time class, decides whether
 * the class copies at all -- a class whose namespace takes part in the copy
 * protocol, or a PEP 695 template whose specializations share one layout
 * name, is refused with a `TypeError` naming why -- and hands the clone one
 * kind byte per slot.
 *
 * A never-initialized carrier (`mod.C.__new__(mod.C)`, `inst` NULL) has no
 * instance to read a class from; only a published type can produce one, and
 * its name is its class name, so the table is consulted by that name and an
 * accepted copy is another never-initialized carrier.
 *
 * The original is not touched: its reference count is unchanged, as in
 * CPython. The clone is never freed (D-107, D-154), so the references it
 * holds, including the `o` slot references taken here, outlive the copy.
 */
static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused)
{
    PyTypeObject *tp = Py_TYPE(self);
    void *inst = ((PyccExtInstance *)self)->inst;
    const unsigned char *cls;
    size_t len = 0;
    const char *kinds = NULL;
    size_t nkinds = 0;
    const char *refused = NULL;
    PyObject *name = NULL;
    void *clone = NULL;
    allocfunc tp_alloc;
    PyObject *carrier;
    int found;
    size_t i;

    (void)unused;
    if (inst == NULL) {
        Py_ssize_t name_len = 0;
        name = PyType_GetName(tp);
        if (name == NULL) {
            return NULL;
        }
        cls = (const unsigned char *)PyUnicode_AsUTF8AndSize(name, &name_len);
        if (cls == NULL) {
            Py_DECREF(name);
            return NULL;
        }
        len = (size_t)name_len;
    }
    else {
        cls = pycc_rt_ext_instance_class(inst, &len);
    }
    found = pycc_ext_carrier_class_copy_kinds(cls, len, &kinds, &nkinds, &refused);
    Py_XDECREF(name);
    if (found < 0) {
        PyErr_SetString(PyExc_SystemError, "pycc: no copy table entry for a carrier's class");
        return NULL;
    }
    if (found == 0) {
        PyObject *qualified = PyType_GetFullyQualifiedName(tp);
        if (qualified != NULL) {
            PyErr_Format(PyExc_TypeError, "cannot copy '%U' object: %s", qualified, refused);
            Py_DECREF(qualified);
        }
        return NULL;
    }
    if (inst != NULL) {
        clone = pycc_rt_ext_instance_copy(inst, kinds, nkinds);
        if (clone == NULL) {
            PyErr_SetString(PyExc_SystemError,
                            "pycc: copy kind table does not match the instance layout");
            return NULL;
        }
        for (i = 0; i < nkinds; i++) {
            if (kinds[i] == 'o') {
                Py_XINCREF((PyObject *)(intptr_t)pycc_rt_instance_get_slot(clone, (long long)i));
            }
        }
    }
    tp_alloc = (allocfunc)PyType_GetSlot(tp, Py_tp_alloc);
    carrier = tp_alloc(tp, 0);
    if (carrier == NULL) {
        return NULL;
    }
    ((PyccExtInstance *)carrier)->inst = clone;
    if (clone != NULL) {
        pycc_rt_ext_instance_set_carrier(clone, carrier);
    }
    return carrier;
}

/*
 * #1455: the method table of a carrier type created on demand -- the shared
 * `__copy__` alone. A class with no published type still copies like a
 * Python object; the generated table refuses the ones that must not.
 */
static PyMethodDef pycc_ext_carrier_methods[] = {
    {"__copy__", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL},
};

/*
 * #1435: the slots of a carrier type created on demand -- the shared
 * deallocator and, since #1455, the shared `__copy__`; no other method, and
 * no descriptor or comparison slot, because a class that has either gets a
 * generated hidden carrier type instead (#1427, #1448). The
 * spec's `Py_TPFLAGS_DISALLOW_INSTANTIATION` keeps the host from calling it,
 * since no `tp_init` exists to build the instance it would carry.
 */
static PyType_Slot pycc_ext_carrier_slots[] = {
    {Py_tp_dealloc, pycc_ext_instance_dealloc},
    {Py_tp_methods, pycc_ext_carrier_methods},
    {0, NULL},
};

/*
 * The carrier type for the class named `name`/`len` (a borrowed reference
 * the cache owns), creating `<module>.<name>` on the first crossing of a
 * class no published type stands for. Returns NULL with a CPython
 * exception set on failure.
 *
 * An empty name is an instance without a layout descriptor. Only an enum
 * member is allocated that way, and `pycc_types` refuses one as a call
 * argument, so this is a `SystemError` rather than a carrier of no class.
 *
 * The type's full name is copied into storage that is never released:
 * `tp_name` may point at the spec's name, and the type itself is never
 * released either.
 */
static PyObject *pycc_ext_carrier_type(const unsigned char *name, size_t len)
{
    PyObject *key;
    PyObject *type;
    size_t prefix = strlen(PYCC_EXT_MODULE_NAME_STR);
    char *full;
    PyType_Spec spec;
    int found;

    if (len == 0) {
        PyErr_SetString(PyExc_SystemError,
                        "an instance without a class descriptor cannot cross into CPython");
        return NULL;
    }
    if (pycc_ext_carrier_types_ready() < 0) {
        return NULL;
    }
    key = PyUnicode_FromStringAndSize((const char *)name, (Py_ssize_t)len);
    if (key == NULL) {
        return NULL;
    }
    found = PyDict_GetItemRef(pycc_ext_carrier_types, key, &type);
    if (found != 0) {
        Py_DECREF(key);
        if (found < 0) {
            return NULL;
        }
        Py_DECREF(type);
        return type;
    }
    full = (char *)PyMem_Malloc(prefix + 1 + len + 1);
    if (full == NULL) {
        Py_DECREF(key);
        return PyErr_NoMemory();
    }
    memcpy(full, PYCC_EXT_MODULE_NAME_STR, prefix);
    full[prefix] = '.';
    memcpy(full + prefix + 1, name, len);
    full[prefix + 1 + len] = '\0';
    spec.name = full;
    spec.basicsize = (int)sizeof(PyccExtInstance);
    spec.itemsize = 0;
    spec.flags = Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE;
    spec.slots = pycc_ext_carrier_slots;
    type = PyType_FromSpec(&spec);
    if (type == NULL || PyDict_SetItem(pycc_ext_carrier_types, key, type) < 0) {
        Py_XDECREF(type);
        Py_DECREF(key);
        return NULL;
    }
    Py_DECREF(key);
    Py_DECREF(type);
    return type;
}

/*
 * #1435: the packer for a pycc class instance passed as an argument to a
 * call on a CPython object (`EXT_OBJ_PACK_INSTANCE_SYMBOL`). The packer
 * contract of its scalar neighbours above: it borrows the instance and
 * returns a new reference, which the consuming call helper releases.
 *
 * Identity follows CPython. The instance's live carrier, if it has one, is
 * returned again -- the object the host constructed, or the one an earlier
 * crossing made and the host still holds -- so two crossings of one
 * instance are `is`-equal. Otherwise a fresh carrier of the instance's
 * *run-time* class (its layout descriptor, not the argument's static type)
 * is allocated and linked; its `tp_dealloc` unlinks it. The instance is
 * never freed (D-107, D-154), so the carrier cannot outlive it.
 *
 * A NULL operand is a `@classmethod`'s `cls`, which `MirExpr::NullInstance`
 * lowers to a null pointer and `pycc_types` refuses as an argument; it is
 * answered with a `SystemError`, which the consuming helpers tolerate.
 */
PyObject *pycc_ext_obj_pack_instance(void *inst)
{
    PyObject *carrier;
    PyObject *type;
    const unsigned char *name;
    size_t len = 0;
    allocfunc tp_alloc;

    if (inst == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "an instance argument to a CPython object's call was NULL");
        return NULL;
    }
    carrier = (PyObject *)pycc_rt_ext_instance_carrier(inst);
    if (carrier != NULL) {
        Py_INCREF(carrier);
        return carrier;
    }
    name = pycc_rt_ext_instance_class(inst, &len);
    type = pycc_ext_carrier_type(name, len);
    if (type == NULL) {
        return NULL;
    }
    tp_alloc = (allocfunc)PyType_GetSlot((PyTypeObject *)type, Py_tp_alloc);
    carrier = tp_alloc((PyTypeObject *)type, 0);
    if (carrier == NULL) {
        return NULL;
    }
    ((PyccExtInstance *)carrier)->inst = inst;
    pycc_rt_ext_instance_set_carrier(inst, carrier);
    return carrier;
}

/*
 * Part 7 of #1371: `isinstance(o, C)` with an object `o` and a class `C`
 * compiled in this module (`EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL`). `o` is
 * borrowed; `name` is the class's NUL-terminated name, a constant string
 * compiled code owns. Same 1/0/-1 contract as `pycc_ext_obj_isinstance`.
 *
 * #1435: a carrier of a compiled instance -- an object whose type
 * deallocates through `pycc_ext_instance_dealloc`, which only the carrier
 * types use and which nothing can subclass -- is answered from the
 * instance's run-time class by the generated
 * `pycc_ext_carrier_class_isinstance`, so a carrier of a class that
 * publishes no method, or of an unpublished subclass of a published one,
 * answers as CPython does for the same source. Every other object,
 * including a published type's object no `tp_init` filled, goes to the
 * generated `pycc_ext_compiled_class_isinstance` declared above, which
 * carries the published-family rule. Defined after the companion include,
 * which defines the generated function it calls first.
 */
int pycc_ext_obj_isinstance_compiled(PyObject *o, const char *name)
{
    void *inst;
    const unsigned char *cls;
    size_t len = 0;

    if (o == NULL || name == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_isinstance_compiled: NULL operand");
        return -1;
    }
    if (PyType_GetSlot(Py_TYPE(o), Py_tp_dealloc) == (void *)pycc_ext_instance_dealloc) {
        inst = ((PyccExtInstance *)o)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            return pycc_ext_carrier_class_isinstance(cls, len, name);
        }
    }
    return pycc_ext_compiled_class_isinstance(o, name);
}

/*
 * Part 1 of #1447 (#1449): unpacks one argument at a parameter annotated
 * with a regular class compiled in this module. Returns 0 with `*out` the
 * compiled instance the carrier holds, or -1 with a `TypeError` set.
 *
 * Only a carrier of this module is admitted -- an object whose type
 * deallocates through *this* file's `pycc_ext_instance_dealloc`, which a
 * carrier of another pycc extension does not, since each artifact links
 * its own copy of the static -- and only one whose instance's run-time
 * class has `class_name` on its MRO, answered by the same generated table
 * `isinstance` uses, so a subclass instance is admitted exactly as CPython
 * admits it. A carrier no `tp_init` filled (`mod.C.__new__(mod.C)`) holds
 * no instance and is refused with its own message.
 *
 * No reference is taken. The body receives the instance, not the carrier,
 * and the instance is never freed (D-107, D-154), so the carrier dying
 * after the call leaves nothing dangling; the carrier link the instance
 * holds is weak and cleared by the carrier's own `tp_dealloc`.
 */
static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                    const char *class_name, void **out)
{
    PyObject *type_name;
    void *inst = NULL;
    const unsigned char *cls;
    size_t len = 0;
    int carrier = PyType_GetSlot(Py_TYPE(obj), Py_tp_dealloc) == (void *)pycc_ext_instance_dealloc;

    if (carrier) {
        inst = ((PyccExtInstance *)obj)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            if (pycc_ext_carrier_class_isinstance(cls, len, class_name)) {
                *out = inst;
                return 0;
            }
        }
    }
    /* Fully qualified on both sides, so a carrier of another pycc module
     * that compiled a class of the same name reads `must be m.Conf, not
     * other.Conf` rather than `must be Conf, not Conf`. */
    type_name = PyType_GetFullyQualifiedName(Py_TYPE(obj));
    if (type_name == NULL) {
        return -1;
    }
    if (carrier && inst == NULL) {
        PyErr_Format(PyExc_TypeError,
                     "%s() argument %zd: the %U object is uninitialized (its __init__ never "
                     "ran)",
                     fn_name, index + 1, type_name);
    } else {
        PyErr_Format(PyExc_TypeError, "%s() argument %zd must be %s.%s, not %U", fn_name,
                     index + 1, PYCC_EXT_MODULE_NAME_STR, class_name, type_name);
    }
    Py_DECREF(type_name);
    return -1;
}

/*
 * #1476 (Part 3 of #1387): the read of an `object` name an
 * `isinstance(o, C)` guard narrowed to a regular class `C` compiled in this
 * module (`EXT_OBJ_UNBOX_INSTANCE_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`). Writes the compiled instance the
 * carrier holds through `*out` and answers `0`, or answers `-1` with a
 * `TypeError` set.
 *
 * `pycc_ext_unpack_instance`'s exact admission -- a carrier of *this*
 * module whose instance's run-time class has `class_name` on its MRO -- with
 * messages that name no argument position. The guard (`PyObject_IsInstance`
 * against the published carrier types) also holds for a carrier no
 * `tp_init` filled (`C.__new__(C)`), refused with a `TypeError` where
 * CPython would raise `AttributeError` at the first field read; and for a
 * non-carrier whose `__class__` property answers a published type (a mock,
 * a proxy), refused with a `TypeError` where CPython would run the guarded
 * body on it -- the same deliberate deviation as the scalar helpers'
 * (#1476). A host-side
 * Python subclass cannot reach here: a carrier type is not an acceptable
 * base type, so the dealloc-slot test only rejects it defensively.
 *
 * The instance is borrowed, as at the argument ingress: compiled instances
 * are never freed (D-107, D-154), and the carrier's link to it is weak.
 */
int pycc_ext_obj_unbox_instance(PyObject *o, const char *class_name, void **out)
{
    PyObject *type_name;
    void *inst = NULL;
    const unsigned char *cls;
    size_t len = 0;
    int carrier;

    if (o == NULL || class_name == NULL || out == NULL) {
        return -1;
    }
    carrier = PyType_GetSlot(Py_TYPE(o), Py_tp_dealloc) == (void *)pycc_ext_instance_dealloc;
    if (carrier) {
        inst = ((PyccExtInstance *)o)->inst;
        if (inst != NULL) {
            cls = pycc_rt_ext_instance_class(inst, &len);
            if (pycc_ext_carrier_class_isinstance(cls, len, class_name)) {
                *out = inst;
                return 0;
            }
        }
    }
    type_name = PyType_GetFullyQualifiedName(Py_TYPE(o));
    if (type_name == NULL) {
        return -1;
    }
    if (carrier && inst == NULL) {
        PyErr_Format(PyExc_TypeError,
                     "the narrowed %U object is uninitialized (its __init__ never ran)",
                     type_name);
    } else {
        PyErr_Format(PyExc_TypeError,
                     "narrowed object must be a compiled %s.%s instance, not %U",
                     PYCC_EXT_MODULE_NAME_STR, class_name, type_name);
    }
    Py_DECREF(type_name);
    return -1;
}

/*
 * #1427: whether a generated `tp_richcompare` calls its compiled comparison
 * on `other` when that method's parameter is annotated with the compiled
 * class `class_name`. An operand `pycc_ext_unpack_instance` would refuse as
 * the wrong type answers 0, and the slot returns `NotImplemented` -- what a
 * `@dataclass`'s own `__eq__` returns for another class -- instead of
 * raising that ingress `TypeError`, so `p == None` is `False` as in CPython.
 * A carrier of this module that no `tp_init` filled answers 1, so the
 * wrapper refuses it with its own `uninitialized` message.
 */
static int pycc_ext_slot_operand_is(PyObject *other, const char *class_name)
{
    void *inst;
    const unsigned char *cls;
    size_t len = 0;

    if (PyType_GetSlot(Py_TYPE(other), Py_tp_dealloc) != (void *)pycc_ext_instance_dealloc) {
        return 0;
    }
    inst = ((PyccExtInstance *)other)->inst;
    if (inst == NULL) {
        return 1;
    }
    cls = pycc_rt_ext_instance_class(inst, &len);
    return pycc_ext_carrier_class_isinstance(cls, len, class_name);
}

/*
 * Part 1 of #1447 (#1449): packs a result annotated with a regular class
 * compiled in this module through #1435's carrier egress, so a returned
 * instance that already has a live carrier -- `self`, or an argument the
 * host still holds -- is that very object (`o.m() is o`), and any other
 * instance gets a fresh carrier of its run-time class. Returns a new
 * reference, the CPython return-value convention.
 *
 * `result` is never NULL on this path -- the generated wrapper checks for a
 * pending pycc exception first, and a compiled function annotated with a
 * class cannot return `None` -- but, as in `pycc_ext_pack_object`, the
 * guard turns a would-be crash into a `SystemError` with a message that
 * names the return position rather than #1435's argument position.
 */
static PyObject *pycc_ext_pack_instance(void *result)
{
    if (result == NULL) {
        PyErr_SetString(PyExc_SystemError, "instance result was NULL");
        return NULL;
    }
    return pycc_ext_obj_pack_instance(result);
}

/*
 * Part 2b of #1142 (#1164): the artifact-owned buffer exporter.
 *
 * A compiled function that returns a buffer hands the wrapper the
 * `PyccExtBufferView *` `pycc_rt_buffer_f64_alloc` produced -- artifact-owned
 * storage with no CPython object anywhere in it. The host must receive a real
 * `memoryview`, and a `memoryview` only ever exists over an *exporter*, so
 * this type is that exporter: one refcounted object per returned buffer,
 * holding the view and freeing it in `tp_dealloc`.
 *
 * That is what moves the free off the compiled frame's own epilogue. A
 * returned buffer is the one buffer the frame must *not* free (see
 * `pycc_codegen`'s `MirStmt::Return`), because the host may hold the
 * `memoryview` arbitrarily long after the call returned; the exporter's
 * refcount is the lifetime that replaces the frame's.
 *
 * The type object itself is a file static rather than module state. `m_size`
 * is 0, so there is no module state to put it in, and
 * `Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED` plus the free-threading
 * refusal in `PyInit_` mean exactly one interpreter on one GIL ever reaches
 * it -- the same single-interpreter argument `pycc_rt`'s non-atomic refcounts
 * already rest on.
 */
typedef struct {
    PyObject_HEAD
    PyccExtBufferView *view;
    /*
     * `Py_buffer.shape` and `.strides` must outlive the `getbuffer` call, so
     * they live in the exporter rather than on its stack. Both are constant
     * for the object's whole life -- the storage is never resized -- so two
     * concurrent views sharing them observe the same values.
     */
    Py_ssize_t shape[1];
    Py_ssize_t strides[1];
} PyccExtBufferObj;

/*
 * The exporter type, created once in `pycc_ext_exec_module` and never
 * released: it has to outlive every `memoryview` the host still holds, and a
 * module reload runs the exec slot again against the same file static.
 */
static PyObject *pycc_ext_buffer_type = NULL;

/*
 * `Py_bf_getbuffer`, filled field by field.
 *
 * Every field is written on every call, including the ones a given `flags`
 * makes NULL: `PyObject_GetBuffer` does not zero the caller's `Py_buffer`, so
 * a field left alone is indeterminate storage the consumer would read.
 *
 * `readonly` is always 0. The storage is the artifact's own heap block, so
 * there is no host-imposed read-only view of it to respect, and a `memoryview`
 * the host cannot write would be a silently narrower object than the one
 * `ndarray(n)` describes.
 *
 * `view->obj` takes a reference to the exporter, which is what keeps the
 * storage alive for exactly as long as the consumer holds the buffer:
 * `PyBuffer_Release` drops it, and the last drop runs `tp_dealloc`.
 */
static int pycc_ext_buffer_getbuffer(PyObject *self, Py_buffer *view, int flags)
{
    PyccExtBufferObj *obj = (PyccExtBufferObj *)self;
    if (obj->view == NULL) {
        /*
         * `view->obj` is NULLed before the failure return because that is
         * what the buffer protocol requires of an exporter that refuses: a
         * consumer may still call `PyBuffer_Release` on the structure it
         * passed in, and a stale non-NULL `obj` would be decrefed.
         */
        view->obj = NULL;
        PyErr_SetString(PyExc_BufferError, "this buffer's storage has been released");
        return -1;
    }
    obj->shape[0] = (Py_ssize_t)obj->view->len;
    obj->strides[0] = (Py_ssize_t)sizeof(double);
    view->buf = obj->view->ptr;
    view->obj = Py_NewRef(self);
    view->len = (Py_ssize_t)obj->view->len * (Py_ssize_t)sizeof(double);
    view->itemsize = (Py_ssize_t)sizeof(double);
    view->readonly = 0;
    view->ndim = 1;
    /*
     * The three shape-describing fields are conditional on what the consumer
     * asked for, exactly as the buffer protocol requires: a `PyBUF_SIMPLE`
     * consumer treats a non-NULL `format` or `shape` as a contract violation.
     * The storage is C-contiguous and one-dimensional, so no request this
     * boundary can receive is refusable on its shape.
     */
    view->format = ((flags & PyBUF_FORMAT) == PyBUF_FORMAT) ? (char *)"d" : NULL;
    view->shape = ((flags & PyBUF_ND) == PyBUF_ND) ? obj->shape : NULL;
    view->strides = ((flags & PyBUF_STRIDES) == PyBUF_STRIDES) ? obj->strides : NULL;
    view->suboffsets = NULL;
    view->internal = NULL;
    return 0;
}

/*
 * `Py_bf_releasebuffer`, deliberately empty.
 *
 * `shape` and `strides` belong to the exporter, not to the view, so there is
 * nothing per-view to free; and the reference `getbuffer` put in `view->obj`
 * is released by `PyBuffer_Release` itself, *after* this slot runs. Dropping
 * it here as well would be a double decref.
 */
static void pycc_ext_buffer_releasebuffer(PyObject *self, Py_buffer *view)
{
    (void)self;
    (void)view;
}

/*
 * The storage's one release point. The heap-type pattern is
 * `pycc_ext_instance_dealloc`'s, and its comment carries the reasoning for
 * the `Py_tp_free`-then-`Py_DECREF(tp)` order; the difference here is the
 * `pycc_rt_buffer_f64_free` in front of it, which is what actually returns
 * the artifact-owned block.
 *
 * `view` is cleared before the object is freed so that the field is never
 * read again from a partially destroyed object, and so `pycc_rt_buffer_f64_free`
 * -- a documented no-op on NULL -- cannot be handed the same pointer twice.
 */
static void pycc_ext_buffer_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    PyccExtBufferObj *obj = (PyccExtBufferObj *)self;
    freefunc tp_free;
    pycc_rt_buffer_f64_free(obj->view);
    obj->view = NULL;
    tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    tp_free(self);
    Py_DECREF(tp);
}

static PyType_Slot pycc_ext_buffer_type_slots[] = {
    {Py_tp_dealloc, pycc_ext_buffer_dealloc},
    {Py_bf_getbuffer, pycc_ext_buffer_getbuffer},
    {Py_bf_releasebuffer, pycc_ext_buffer_releasebuffer},
    {0, NULL},
};

/*
 * `Py_TPFLAGS_DISALLOW_INSTANTIATION` for the same reason a non-constructible
 * class carries it: nothing in Python may produce one of these, because an
 * exporter with a NULL `view` describes no storage. The only producer is
 * `pycc_ext_pack_memoryview`. The leading underscore keeps the name out of the
 * space a user class could collide with, and it is never published as a
 * module attribute in any case.
 */
static PyType_Spec pycc_ext_buffer_type_spec = {
    PYCC_EXT_MODULE_NAME_STR "._buffer",
    sizeof(PyccExtBufferObj),
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE,
    pycc_ext_buffer_type_slots,
};

/*
 * The buffer egress: takes ownership of `view` unconditionally and returns a
 * new `memoryview` over it, or NULL with an exception set.
 *
 * **The ownership boundary is the successful `tp_alloc`.** Before it, nothing
 * else can reach the storage, so a failure frees it directly. After it, the
 * exporter owns it and `tp_dealloc` is the only thing that may free it -- so
 * every later failure path releases the *exporter* and frees nothing itself.
 * Freeing directly after the store would be a double free, and returning
 * without freeing before it would be a leak.
 *
 * Both calls that can fail are checked: `tp_alloc` and
 * `PyMemoryView_FromObject`. No Rust allocation runs on this path at all, so
 * nothing here can abort the host interpreter the way an infallible `Box::new`
 * across the `extern "C"` boundary would.
 */
static PyObject *pycc_ext_pack_memoryview(PyccExtBufferView *view)
{
    PyObject *exporter;
    PyObject *result;
    allocfunc tp_alloc;
    /*
     * Neither arm is reachable from a well-formed artifact: the wrapper
     * checks `pycc_rt`'s pending flag before calling this, so a failed
     * allocation has already returned, and the exec slot fails the import
     * outright when the type cannot be created. Both are still handled,
     * because the alternative to a `SystemError` here is a NULL dereference
     * inside the host.
     */
    if (view == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc: internal error: a buffer return produced no storage");
        return NULL;
    }
    if (pycc_ext_buffer_type == NULL) {
        pycc_rt_buffer_f64_free(view);
        PyErr_SetString(PyExc_SystemError,
                        "pycc: internal error: the buffer exporter type is not registered");
        return NULL;
    }
    /*
     * `PyType_GetSlot(tp, Py_tp_alloc)` because the limited API exposes no
     * other way to reach a heap type's allocator, exactly as
     * `pycc_ext_instance_dealloc` reaches `Py_tp_free`. It zeroes the object,
     * so `view` starts NULL and the `shape`/`strides` arrays start defined
     * even if `getbuffer` never runs; and, this being a heap type, it takes
     * the type reference that `pycc_ext_buffer_dealloc` discharges.
     */
    tp_alloc = (allocfunc)PyType_GetSlot((PyTypeObject *)pycc_ext_buffer_type, Py_tp_alloc);
    exporter = tp_alloc((PyTypeObject *)pycc_ext_buffer_type, 0);
    if (exporter == NULL) {
        pycc_rt_buffer_f64_free(view);
        return NULL;
    }
    ((PyccExtBufferObj *)exporter)->view = view;
    /*
     * `PyMemoryView_FromObject` takes its own reference to the exporter
     * through the buffer it acquires, so the wrapper's own reference is
     * released on both arms -- the failing one included, which is what makes
     * a failure here free the storage exactly once, through `tp_dealloc`.
     */
    result = PyMemoryView_FromObject(exporter);
    Py_DECREF(exporter);
    return result;
}

/*
 * Part 1 of #1175: the buffer egress when the returned view is the *host's*
 * own buffer -- the name a `memoryview` parameter binds, returned by name.
 *
 * This is the one packer on the buffer path that allocates nothing and frees
 * nothing. `owner` is the argument object, a borrowed reference the caller
 * holds for the whole call; this function takes no reference of its own and
 * releases none, and there is no artifact-owned block anywhere in the path,
 * so `pycc_rt_buffer_live_views` is untouched by it.
 *
 * What keeps the returned view valid is not a reference to `owner` but a
 * buffer **export** on it: `PyMemoryView_FromObject` acquires its own, held
 * for exactly as long as the `memoryview` lives, and independent of the
 * `Py_buffer` the generated wrapper acquired and releases on its way out. A
 * bare `Py_INCREF(owner)` plus a `{ptr, len}` pair would *not* do -- a strong
 * reference does not pin storage, and a resizable exporter such as
 * `array.array('d')` relocates its block under `extend`. Holding an export is
 * what makes that same `extend` raise `BufferError` instead.
 *
 * The wrapper acquires this view *before* releasing its own buffer, so the
 * host object never reaches zero outstanding exports across the boundary;
 * `src/ext_build/wrappers.rs`'s `caller_owned_buffer_acquire` carries the
 * PEP 688 argument for why that ordering is required rather than tidy.
 *
 * NULL with the exception set on failure, which the generated wrapper returns
 * as-is: it must never fall through to `pycc_ext_pack_memoryview`, whose
 * exporter would free the host's storage in `tp_dealloc`.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed(PyObject *owner, const Py_buffer *held,
                                                   const char *fn_name, Py_ssize_t index,
                                                   int writable)
{
    PyObject *view;
    Py_buffer probe;
    const char *held_format;
    const char *probe_format;
    int differs;

    /*
     * The second export, and the reason it must be checked. PEP 688 lets an
     * exporter answer a later `__buffer__` with a *different* window, and
     * `PyMemoryView_FromObject` performs exactly such a later call. Nothing
     * routes that window through `pycc_ext_unpack_memoryview`, so without the
     * comparison below the host could receive storage the compiled body never
     * saw -- or a shape and format D-244 statement (e) refuses outright.
     *
     * Transferring the *first* export into the returned object instead is not
     * available under `Py_LIMITED_API`: `PyMemoryView_FromBuffer` copies the
     * `Py_buffer` without taking ownership of it (measured: the exporter's
     * `__release_buffer__` never runs), so that route leaks the export
     * permanently rather than moving it.
     *
     * Refusing an exotic-but-legal exporter is this boundary's existing idiom
     * -- `pycc_ext_unpack_memoryview` already refuses a conforming 2-D or
     * non-'d' one. A returned view whose window is not the one the call
     * operated on is refused the same way.
     */
    view = PyMemoryView_FromObject(owner);
    if (view == NULL) {
        return NULL;
    }
    /*
     * Probing the `memoryview` rather than `owner` deliberately: a
     * `memoryview`'s own `bf_getbuffer` re-describes the buffer it already
     * holds, so this reads back export #2 without asking the exporter for a
     * third one.
     */
    if (PyObject_GetBuffer(view, &probe,
                           PyBUF_C_CONTIGUOUS | PyBUF_FORMAT |
                               (writable ? PyBUF_WRITABLE : 0)) != 0) {
        Py_DECREF(view);
        return NULL;
    }
    held_format = (held->format == NULL) ? "" : held->format;
    probe_format = (probe.format == NULL) ? "" : probe.format;
    differs = probe.buf != held->buf || probe.len != held->len ||
              probe.itemsize != held->itemsize || probe.ndim != held->ndim ||
              probe.readonly != held->readonly || strcmp(probe_format, held_format) != 0;
    PyBuffer_Release(&probe);
    if (differs) {
        Py_DECREF(view);
        PyErr_Format(PyExc_BufferError,
                     "%s() argument %zd: the exporter described a different buffer on its "
                     "second export, so the window this call operated on cannot be returned",
                     fn_name, index + 1);
        return NULL;
    }
    return view;
}

/*
 * Part 2 of #1175 (#1179): `return b[start:stop]` over a caller-owned buffer
 * parameter.
 *
 * The bounds arrive out of band, through the three trailing `long long *`
 * out-slots the compiled function writes (`has_slice`, `start`, `stop`) --
 * `src/ext_build/wrappers.rs` carries why they cannot be in band. The
 * compiled body still returns the *whole* view's `PyccExtBufferView *`, so
 * the wrapper's `result == &a{index}` provenance test stays an exact pointer
 * identity and this function sees the same `owner`/`held` pair the
 * whole-view path does.
 *
 * Deriving the sub-range host-side, from a `memoryview` CPython itself
 * sliced, is what keeps the semantics Python's rather than this boundary's:
 * negative bounds, a `stop` past the end, and a crossed pair (`b[3:1]`, an
 * empty view) are all CPython's own, already-tested behaviour. An absent
 * bound is encoded by the compiled body as `0` for `start` and `LLONG_MAX`
 * for `stop`, both of which `slice` clamps exactly as `b[:]` does.
 *
 * The whole-window PEP 688 check runs first, unchanged, via the sibling
 * above: the window whose identity must be proven is the one the compiled
 * call operated on, not the sub-range derived from it afterwards. The
 * resulting `memoryview` holds the second export exactly as the whole-view
 * path's does -- `PyObject_GetItem` on a `memoryview` returns a new
 * `memoryview` that holds its own reference to the sliced one -- so the
 * `pycc_rt_buffer_live_views` accounting and the `array.array` resize
 * `BufferError` behave identically for both.
 *
 * NULL with the exception set on failure, which the generated wrapper
 * returns as-is.
 */
static PyObject *pycc_ext_pack_memoryview_borrowed_slice(PyObject *owner, const Py_buffer *held,
                                                         const char *fn_name, Py_ssize_t index,
                                                         int writable, long long start,
                                                         long long stop)
{
    PyObject *whole;
    PyObject *start_obj;
    PyObject *stop_obj;
    PyObject *slice;
    PyObject *sub;

    whole = pycc_ext_pack_memoryview_borrowed(owner, held, fn_name, index, writable);
    if (whole == NULL) {
        return NULL;
    }
    start_obj = PyLong_FromLongLong(start);
    if (start_obj == NULL) {
        Py_DECREF(whole);
        return NULL;
    }
    stop_obj = PyLong_FromLongLong(stop);
    if (stop_obj == NULL) {
        Py_DECREF(start_obj);
        Py_DECREF(whole);
        return NULL;
    }
    /* `PySlice_New` borrows nothing: it takes its own references. */
    slice = PySlice_New(start_obj, stop_obj, NULL);
    Py_DECREF(start_obj);
    Py_DECREF(stop_obj);
    if (slice == NULL) {
        Py_DECREF(whole);
        return NULL;
    }
    sub = PyObject_GetItem(whole, slice);
    Py_DECREF(slice);
    Py_DECREF(whole);
    return sub;
}


/*
 * #1199: binds one top-level definition on the executing module, called by
 * LLVM-generated code right after the definition's `fnptr_` slot store (and
 * after the last slot of a class statement), so the name becomes a module
 * attribute exactly when CPython would bind it. Before #1199 every export
 * was published before the body ran, and a foreign module that imported
 * this one back mid-body could call a wrapper whose slot was still null.
 *
 * The name is resolved first, against `pycc_ext_methods[]` (module-level
 * functions; never the PEP 562 hook table, which the exec slot adds after
 * the body) and then the generated published-class lookup. A name in
 * neither -- a function the driver did not export, or anything at all in
 * the embedded launcher, whose tables are empty -- is not an error: codegen
 * publishes from the MIR alone and may name a superset of the exports, so
 * the answer is 0 without touching any state. Only a resolved name reads
 * the exec-target key, and a NULL target there (no `Py_mod_exec` running
 * on this thread) is a `SystemError`.
 *
 * A function becomes a `builtin_function_or_method` bound to the module,
 * exactly what `PyModule_AddFunctions` would have made of the same row.
 * Codegen publishes a redefined name only at its last definition, since
 * every definition shares that one's wrapper
 * (`crates/pycc_codegen/src/ext_publish.rs`); a second publish of one name
 * would still just replace the binding.
 * A class is bound from this exec's snapshot (`pycc_ext_exec_classes_key`)
 * of the `pycc_ext_type_object_<Class>` static that
 * `pycc_ext_register_method_types` filled before the body, not from the
 * static itself: the static is one per process, and a nested `Py_mod_exec`
 * of this artifact mid-body replaces it, so reading it here would bind the
 * inner exec's class on the outer module.
 *
 * Returns 0, or -1 with an exception set. Not `static`: LLVM-generated code
 * declares and calls it by this name (`EXT_PUBLISH_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_publish(const char *name)
{
    PyMethodDef *row = NULL;
    PyMethodDef *def;
    PyObject **class_slot = NULL;
    PyObject *module;
    PyObject *classes;
    PyObject *type;
    PyObject *modname;
    PyObject *func;
    int status;

    for (def = pycc_ext_methods; def->ml_name != NULL; def++) {
        if (strcmp(def->ml_name, name) == 0) {
            row = def;
            break;
        }
    }
    if (row == NULL) {
        class_slot = pycc_ext_publish_class_slot(name);
        if (class_slot == NULL) {
            return 0;
        }
    }
    module = pycc_ext_exec_target_key == NULL
        ? NULL
        : (PyObject *)PyThread_tss_get(pycc_ext_exec_target_key);
    if (module == NULL) {
        PyErr_Format(PyExc_SystemError, "pycc: cannot publish '%s' outside its module body", name);
        return -1;
    }
    if (row == NULL) {
        classes = (PyObject *)PyThread_tss_get(pycc_ext_exec_classes_key);
        type = classes == NULL ? NULL : PyDict_GetItemString(classes, name);
        if (type == NULL) {
            PyErr_Format(PyExc_SystemError, "pycc: class '%s' has no type object", name);
            return -1;
        }
        return PyModule_AddObjectRef(module, name, type);
    }
    modname = PyModule_GetNameObject(module);
    if (modname == NULL) {
        return -1;
    }
    func = PyCFunction_NewEx(row, module, modname);
    Py_DECREF(modname);
    if (func == NULL) {
        return -1;
    }
    status = PyModule_AddObjectRef(module, row->ml_name, func);
    Py_DECREF(func);
    return status;
}

/*
 * #1199's post-body safety net: binds every published class the body's
 * `pycc_ext_publish` calls did not, so a completed import still exposes
 * every type object it did before #1199. Codegen publishes a class once the
 * last slot any of its type object's methods calls is stored, and every
 * published class has at least one such slot (a method it declares, the
 * `__init__` D-225 synthesizes for a base-less class that declares none,
 * or an inherited method a subclass's type object calls), so today this
 * binds nothing. It exists so that a publication codegen ever misses
 * becomes late visibility rather than a class the host can never reach.
 * A name already in the module dict is left alone: either its publication
 * ran, or the body rebound the name itself. Runs before the PEP 562 hooks
 * are added, so the dict lookup sees the module's own bindings only. Binds
 * from `classes`, this exec's snapshot, for the reason `pycc_ext_publish`
 * does.
 */
static int pycc_ext_publish_unbound_classes(PyObject *module, PyObject *classes)
{
    PyObject *dict = PyModule_GetDict(module);
    const char *const *name;

    for (name = pycc_ext_publish_class_names; *name != NULL; name++) {
        PyObject *type = PyDict_GetItemString(classes, *name);
        if (type == NULL || PyDict_GetItemString(dict, *name) != NULL) {
            continue;
        }
        if (PyModule_AddObjectRef(module, *name, type) != 0) {
            return -1;
        }
    }
    return 0;
}

/*
 * #1199: the snapshot `pycc_ext_exec_classes_key` holds for one exec -- a
 * new dict holding a reference to every published class's type object as
 * `pycc_ext_register_method_types` just made it. NULL with an exception
 * set on failure.
 */
static PyObject *pycc_ext_snapshot_classes(void)
{
    PyObject *classes = PyDict_New();
    const char *const *name;

    if (classes == NULL) {
        return NULL;
    }
    for (name = pycc_ext_publish_class_names; *name != NULL; name++) {
        PyObject *type = *pycc_ext_publish_class_slot(*name);
        if (type != NULL && PyDict_SetItemString(classes, *name, type) != 0) {
            Py_DECREF(classes);
            return NULL;
        }
    }
    return classes;
}

/*
 * PEP 489 multi-phase initialization. The module body cannot run in
 * `PyInit_`: at that point no module object exists, so there is nothing to
 * execute against and no way to report a failure. It runs here, in the
 * `Py_mod_exec` slot CPython calls after creating the module object, whose
 * failure convention is -1 with the exception set.
 *
 * Running it at all is not optional. `pycc` binds each `def` by *storing*
 * its address into a module-level function-pointer slot at the def's own
 * source position, and (#1199) publishes the export only right after that
 * store, through `pycc_ext_publish` below: before the body has reached a
 * definition, its name is simply not an attribute of the module.
 */
static int pycc_ext_exec_module(PyObject *module)
{
    Py_ssize_t mark;
    void *saved_target;
    void *saved_classes;
    PyObject *classes;
    int status;
    long long exec_status;

    /*
     * The synthesized user exception classes are created and published as
     * module attributes before the module body runs, so a body that raises
     * one already finds it registered. This is the only place the module
     * object is reachable (`m_size = 0`, so there is no module state), and
     * a failure here fails the import loudly rather than importing a module
     * whose `except m.MyError:` silently never matches.
     */
    /*
     * Part 2b of #1142 (#1164): the buffer exporter type, before anything
     * that could call into compiled code, because a compiled function that
     * returns a buffer needs it the first time it is called. Created at most
     * once: a module reload runs this slot again against the same file
     * static, and creating a second type object would leave `memoryview`s
     * from before the reload exporting through a type nothing else holds.
     *
     * Not published as a module attribute -- it is not part of the module's
     * Python-visible surface, and `Py_TPFLAGS_DISALLOW_INSTANTIATION` means
     * a host that reached it anyway could do nothing with it.
     */
    if (pycc_ext_buffer_type == NULL) {
        pycc_ext_buffer_type = PyType_FromSpec(&pycc_ext_buffer_type_spec);
        if (pycc_ext_buffer_type == NULL) {
            return -1;
        }
    }
    if (pycc_ext_register_exception_classes(module) != 0) {
        return -1;
    }
    /*
     * One type object per class that exports a method or is constructible,
     * created here but not yet added to the module (#1199): it is kept in
     * its `pycc_ext_type_object_<Class>` static, and `pycc_ext_publish`
     * binds it under the bare class name once the body has run the class
     * statement (this exec snapshots it just below, and `pycc_ext_publish`
     * binds from the snapshot). PEP 489 puts the module in `sys.modules` before this slot
     * runs, so a foreign module the body imports can import this one back
     * and read its attributes mid-body; a class bound here would be visible
     * there before CPython would bind it. Created before the body anyway,
     * because a compiled function the body calls may box an instance of the
     * class into its carrier type, which must already exist. A class
     * carrying an exception tag never gets a type object at all: the
     * exception classes above already own that attribute. Same
     * -1-with-exception-set convention as everything else in this slot.
     */
    if (pycc_ext_register_method_types(module) != 0) {
        return -1;
    }
    /*
     * #1316: the bridge table's thread-specific-storage key, created once
     * per process. A re-import after `del sys.modules[...]` runs this slot
     * again, and must not replace a key that live wrappers on other threads
     * still read.
     */
    if (pycc_ext_bridge_key == NULL) {
        Py_tss_t *key = PyThread_tss_alloc();
        if (key == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        if (PyThread_tss_create(key) != 0) {
            PyThread_tss_free(key);
            PyErr_SetString(PyExc_RuntimeError, "pycc: cannot create the bridge-table key");
            return -1;
        }
        pycc_ext_bridge_key = key;
    }
    /* #1366: the exec-target key, on the same create-once rule. */
    if (pycc_ext_exec_target_key == NULL) {
        Py_tss_t *key = PyThread_tss_alloc();
        if (key == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        if (PyThread_tss_create(key) != 0) {
            PyThread_tss_free(key);
            PyErr_SetString(PyExc_RuntimeError, "pycc: cannot create the exec-target key");
            return -1;
        }
        pycc_ext_exec_target_key = key;
    }
    /* #1199: the exec-classes key, on the same create-once rule. */
    if (pycc_ext_exec_classes_key == NULL) {
        Py_tss_t *key = PyThread_tss_alloc();
        if (key == NULL) {
            PyErr_NoMemory();
            return -1;
        }
        if (PyThread_tss_create(key) != 0) {
            PyThread_tss_free(key);
            PyErr_SetString(PyExc_RuntimeError, "pycc: cannot create the exec-classes key");
            return -1;
        }
        pycc_ext_exec_classes_key = key;
    }
    /*
     * #1199: this exec's classes, taken now, before the body can run a
     * nested exec that replaces the type statics; installed and restored
     * on both exits alongside the #1366 target below, and released only
     * after the safety net has read them.
     */
    classes = pycc_ext_snapshot_classes();
    if (classes == NULL) {
        return -1;
    }
    /*
     * #1366: this module is the one a relative import in its body resolves
     * against, for exactly the duration of the body. The previous value is
     * restored on both exits, so an exec running beneath another one on
     * this thread (see `pycc_ext_exec_target_key`) hands the outer one's
     * target back.
     */
    saved_target = PyThread_tss_get(pycc_ext_exec_target_key);
    saved_classes = PyThread_tss_get(pycc_ext_exec_classes_key);
    if (PyThread_tss_set(pycc_ext_exec_target_key, module) != 0
        || PyThread_tss_set(pycc_ext_exec_classes_key, classes) != 0) {
        (void)PyThread_tss_set(pycc_ext_exec_target_key, saved_target);
        Py_DECREF(classes);
        PyErr_SetString(PyExc_RuntimeError, "pycc: cannot record the executing module");
        return -1;
    }
    /*
     * The watermark, not a whole-table clear: when this exec runs beneath a
     * live wrapper on the same thread (a handler whose foreign helper
     * re-imports the module), that wrapper's entries survive it. When exec
     * is the outermost frame the mark is 0, which empties the table. Taken
     * immediately before the body, after every early return above, so each
     * mark is released exactly once and this exec is counted as a live
     * activation (`pycc_ext_live_activations`) from the body through its
     * watermark release, whose finalizers run while it is still counted.
     */
    mark = pycc_ext_bridge_mark();
    exec_status = pycc_ext_module_exec();
    (void)PyThread_tss_set(pycc_ext_exec_target_key, saved_target);
    (void)PyThread_tss_set(pycc_ext_exec_classes_key, saved_classes);
    if (exec_status != 0) {
        Py_DECREF(classes);
        /*
         * The generic `ImportError` is a last resort, not the default. A
         * failing body reports through one of two channels: `pycc_rt`'s
         * thread-local pending state, which `pycc_ext_raise_pending` turns
         * into a live CPython exception, or -- for a body that called into
         * CPython directly -- an exception CPython already set, with no
         * pycc pending state at all. The first channel has a third case
         * since #1293: a pending exception the failed-import bridge made
         * from a CPython one, which `pycc_ext_raise_pending` finds in the
         * bridge table and re-raises as that original object.
         * `pycc_ext_raise_pending` returns 0 for the second channel, so
         * raising unconditionally here would replace
         * the real exception (a `ModuleNotFoundError` from a failed host
         * import, say) with a message that names neither the cause nor the
         * culprit. Check `PyErr_Occurred` before overwriting, and keep
         * `pycc_ext_raise_pending` as the left operand: it is the call that
         * sets the exception, so short-circuiting must not skip it.
         */
        if (!pycc_ext_raise_pending() && !PyErr_Occurred()) {
            PyErr_SetString(PyExc_ImportError, "pycc module body failed");
        }
        /* Only after the lookup above: any entry still here belongs to a
         * bridged exception that was caught, or replaced by another. */
        pycc_ext_bridge_release_to(mark);
        return pycc_ext_activation_exit_status(-1);
    }
    pycc_ext_bridge_release_to(mark);
    (void)pycc_ext_activation_exit_status(0);
    status = pycc_ext_publish_unbound_classes(module, classes);
    Py_DECREF(classes);
    if (status != 0) {
        return -1;
    }
    /*
     * #1467: the module's PEP 562 `__getattr__` / `__dir__`, published
     * only now, not at their `def` the way `pycc_ext_publish` binds every
     * other function (#1199). Once in the dict, `__getattr__` answers every
     * missing attribute -- including the ones importlib and a re-entrant
     * host probe while the body is still running -- so binding it at its
     * `def` would let it answer for names the rest of the body has yet to
     * bind. A re-import after `del sys.modules[...]` runs this again and
     * rebinds them.
     */
    if (PyModule_AddFunctions(module, pycc_ext_module_hooks) != 0) {
        return -1;
    }
    return 0;
}

static PyModuleDef_Slot pycc_ext_slots[] = {
    {Py_mod_exec, (void *)pycc_ext_exec_module},
    /*
     * `pycc_rt`'s refcounts are non-atomic `Cell<u32>` and its exception
     * state is thread-local, both of which are safe only under one GIL in
     * one interpreter. Refuse a subinterpreter explicitly rather than
     * corrupting state in one.
     */
    {Py_mod_multiple_interpreters, Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED},
    {0, NULL},
};

static struct PyModuleDef pycc_ext_moduledef = {
    PyModuleDef_HEAD_INIT,
    PYCC_EXT_MODULE_NAME_STR,
    NULL,
    0,
    /*
     * #1199: no `m_methods`. CPython would add every row before the body
     * runs; `pycc_ext_publish` adds each one as its `def` executes instead.
     */
    NULL,
    pycc_ext_slots,
    NULL,
    NULL,
    NULL,
};

#define PYCC_EXT_CONCAT_(a, b) a##b
#define PYCC_EXT_CONCAT(a, b) PYCC_EXT_CONCAT_(a, b)

PyMODINIT_FUNC PYCC_EXT_CONCAT(PyInit_, PYCC_EXT_MODULE_NAME)(void)
{
    /*
     * The free-threaded guard, and it has to be here -- before
     * `PyModuleDef_Init` -- because it must refuse the module outright.
     *
     * A free-threaded host's `EXTENSION_SUFFIXES` does include `.abi3.so`,
     * so its finder locates this artifact happily; with no guard the failure
     * is a bare `SystemError: init function of <mod> returned uninitialized
     * object`. `sys._is_gil_enabled()` is not a usable detector here: it
     * returned True on a free-threaded 3.14 at `PyInit_` time. The version
     * string is, verified across 3.13 GIL, 3.14 GIL and 3.14t.
     */
    if (strstr(Py_GetVersion(), "free-threading build") != NULL) {
        PyErr_SetString(PyExc_ImportError,
                        PYCC_EXT_MODULE_NAME_STR " was built for a GIL-enabled CPython "
                        "(stable ABI); this interpreter is a free-threaded build, which is "
                        "not supported");
        return NULL;
    }
    return PyModuleDef_Init(&pycc_ext_moduledef);
}
