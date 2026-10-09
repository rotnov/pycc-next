//! D-244's hosted `ext` artifact mode, as codegen sees it.
//!
//! Everything here answers one question -- *who calls the synthetic
//! module-body entry point, and what does it return when the body raises* --
//! and nothing here touches LLVM. `native` mode's answer is "the process
//! loader, and `main` never comes back from an uncaught exception"; `ext`'s
//! is "the C shim's `Py_mod_exec` slot, and it returns `-1` with the pending
//! exception handed to CPython". The option struct that selects between them
//! and the two symbol-name predicates that implement the distinction live
//! together so the naming invariant cannot drift across a 9000-line file
//! (AGENTS.md's "Keep source files decomposable"; the tracker for the rest
//! of `lib.rs` is #545).
//!
//! #1050 added a second, closely related question -- *what shape does a
//! public export present to the generated C wrapper* -- and the answer is
//! the `pycc_ext_thunk_<name>` convention below. It lives here rather than
//! in `ext_thunk.rs` because the driver (`src/ext_build.rs`) renders C
//! against exactly this convention and must agree with it symbol for symbol
//! and width for width, while `ext_thunk.rs` is the LLVM emission that
//! implements it.

use pycc_mir::Ty;

/// Everything `compile_to_object` needs beyond the MIR and the output path.
///
/// Additive by construction: `compile_to_object` has hundreds of call sites
/// across this crate's own tests and the workspace's integration targets, so
/// `ext` (D-244's hosted CPython extension-module mode) arrives as a field on
/// a `Default`-constructible struct behind a second entry point rather than
/// as a new positional parameter on the existing one. `..Default::default()`
/// then keeps a later mode from churning those call sites either.
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// `None` builds for the host's own default target; `Some(triple)`
    /// cross-compiles. Owned rather than borrowed so the struct can be
    /// stored and passed without threading a lifetime through every caller.
    pub target_triple: Option<String>,
    /// `true` runs LLVM's `"default<O3>"` pipeline (D-094).
    pub release: bool,
    /// `true` emits an object destined for a CPython extension-module
    /// artifact rather than a native executable (D-244). Two things change,
    /// both of them about *who calls the module body and what happens when
    /// it raises*: the synthetic module-body entry point is named
    /// [`EXT_MODULE_EXEC_SYMBOL`] instead of `main`, so the artifact exports
    /// no stray `main` and the fixed C shim's `Py_mod_exec` slot has a
    /// symbol to call; and an uncaught module-scope exception returns `-1`
    /// after handing the pending state to the host (see
    /// `pycc_rt_ext_pending_type`) instead of calling
    /// `pycc_rt_exception_print_and_exit`, which would terminate the
    /// interpreter process instead of failing the import.
    pub ext: bool,
    /// `true` skips the export thunks `ext` mode would otherwise emit
    /// (`ext_thunk::emit_export_thunks`), and means nothing without `ext`.
    ///
    /// Set by the embedded-executable mode (Part 1 of #1028), which compiles
    /// with `ext` for its module-body entry point and failure edge but
    /// exports no function to a host: the thunk set comes from the MIR
    /// (`ext_thunk_required`), not from the driver's export list, and in
    /// `--ext` mode only the driver's `collect_exports` guarantees a thunked
    /// signature is admissible, which an embedded build never runs. The
    /// polarity is deliberate -- `false`, the `Default`, is today's
    /// behaviour, so every `ext: true, ..CompileOptions::default()` literal
    /// keeps its meaning.
    pub suppress_export_thunks: bool,
}

/// The symbol the module body is emitted under in `ext` mode: the fixed C
/// shim's `Py_mod_exec` slot calls exactly this name, and it returns `0` on
/// success or `-1` with the pending exception already handed to CPython.
///
/// Deliberately *not* `main`: a CPython extension module that exports `main`
/// would collide with the host interpreter's own entry point.
pub const EXT_MODULE_EXEC_SYMBOL: &str = "pycc_ext_module_exec";

/// What [`EXT_MODULE_EXEC_SYMBOL`] returns when the module body raised: the
/// `Py_mod_exec` slot's own failure convention (`-1` with the exception
/// already set), which is *not* the per-export wrapper's (`NULL`).
pub const EXT_MODULE_EXEC_FAILED: i64 = -1;

/// The fixed C shim's per-definition publication entry point (#1199):
/// `int pycc_ext_publish(const char *name)` binds the export named `name`
/// on the executing module and returns `0`, or `0` without touching any
/// state for a name the generated tables do not know, or `-1` with the
/// CPython exception set. The module body calls it right after a top-level
/// `def` or class statement has bound its function-pointer slots
/// (`ext_publish.rs`), so an export becomes visible when its definition
/// executes, as in CPython.
///
/// Kept here, beside [`EXT_MODULE_EXEC_SYMBOL`], so the driver's shim
/// parity test can assert it: the `--ext` link resolves an undefined symbol
/// lazily, so a misspelling on either side would be a crash at first call
/// rather than a link error.
pub const EXT_PUBLISH_SYMBOL: &str = "pycc_ext_publish";

/// The name the synthetic module-body entry point carries in each mode.
/// One function so the `add_function` call and the `MirStmt::Return`
/// invariant that pins the name can never drift apart.
pub(crate) fn entry_fn_name(ext: bool) -> &'static str {
    if ext { EXT_MODULE_EXEC_SYMBOL } else { "main" }
}

/// Whether `name` is the symbol the synthetic module-body entry point was
/// emitted under, in *either* mode. The `MirStmt::Return` invariant below
/// asks this rather than comparing against `main` directly: `ext` builds
/// rename that entry point (see [`CompileOptions::ext`]), and an invariant
/// that still only recognized `main` would stop firing there -- silently, and
/// exactly in the mode where a module-level `return` reaching codegen would
/// corrupt the `Py_mod_exec` slot's own return value.
pub(crate) fn is_module_entry_symbol(name: &[u8]) -> bool {
    name == b"main" || name == EXT_MODULE_EXEC_SYMBOL.as_bytes()
}

/// The prefix every scalar-only `ext` export thunk's symbol carries.
///
/// Kept beside [`EXT_MODULE_EXEC_SYMBOL`] for the same reason: the driver's
/// generated C declares this symbol by name, and a prefix spelled twice is a
/// link-time-deferred crash rather than a compile error (the `--ext` link
/// passes `-undefined dynamic_lookup` on Mach-O and `-Bsymbolic` on ELF, so
/// an undefined thunk resolves to nothing and dies at call time).
pub const EXT_THUNK_PREFIX: &str = "pycc_ext_thunk_";

/// The fixed C shim's module-import helper (Part 1 of #1026): it takes a
/// NUL-terminated module name and returns a *new* reference to the imported
/// module, or `NULL` with the CPython exception already set.
///
/// Kept here for exactly the reason [`EXT_THUNK_PREFIX`] is: the symbol is
/// defined in `src/ext/pycc_ext_module.c` and declared by LLVM in
/// `foreign_import.rs`, and the `--ext` link resolves an undefined symbol
/// lazily (`-undefined dynamic_lookup` on Mach-O, `-Bsymbolic` on ELF), so
/// a misspelling on either side is a crash at first call rather than a link
/// error. One constant, referenced by both sides, makes that impossible.
pub const EXT_OBJ_IMPORT_SYMBOL: &str = "pycc_ext_obj_import";

/// The fixed C shim's from-import helper (#1278): `PyObject
/// *pycc_ext_obj_import_from(const char *module, const char *const
/// *fromlist, long long nfrom, long long index, long long level)` runs
/// CPython's `IMPORT_NAME` with the statement's whole fromlist, then
/// `IMPORT_FROM` of `fromlist[index]`, and returns a *new* reference to that
/// object, or `NULL` with the CPython exception (an `ImportError` for a
/// missing name) already set. `level` is `0` for an absolute import; a
/// relative one (#1366, `pycc build --ext --foreign-relative-imports`)
/// passes its dot count, and the shim resolves it against the executing
/// module's own dict. Spelled once here for exactly the reason
/// `EXT_OBJ_IMPORT_SYMBOL` is.
pub const EXT_OBJ_IMPORT_FROM_SYMBOL: &str = "pycc_ext_obj_import_from";

/// The fixed C shim's plain dotted-import helper (#1381, Part 3 of #1138):
/// `PyObject *pycc_ext_obj_import_dotted(const char *name, long long
/// bind_root)` runs CPython's `IMPORT_NAME` of the dotted `name` with no
/// fromlist and returns a *new* reference to the root package when
/// `bind_root` is non-zero (`import a.b` binds `a`), or else to the leaf it
/// reaches with one `IMPORT_FROM` per remaining segment (`import a.b as c`
/// binds `a.b`), or `NULL` with the CPython exception already set. An
/// undotted `import a` keeps [`EXT_OBJ_IMPORT_SYMBOL`]. Spelled once here
/// for exactly the reason `EXT_OBJ_IMPORT_SYMBOL` is.
pub const EXT_OBJ_IMPORT_DOTTED_SYMBOL: &str = "pycc_ext_obj_import_dotted";

/// The fixed C shim's failed-import bridge (#1293, Part 3 of #1282): called
/// on the `NULL` edge of a foreign import nested in a module-level
/// `if`/`try` block, it returns `1` after translating CPython's pending
/// `ImportError` into a pending pycc exception, or `0` with CPython's
/// exception left as it was.
///
/// Spelled once here for exactly the reason [`EXT_OBJ_IMPORT_SYMBOL`]
/// directly above is. Deliberately not prefixed `pycc_ext_obj_import`:
/// tests count that symbol's occurrences in the emitted IR.
pub const EXT_IMPORT_ERROR_BRIDGE_SYMBOL: &str = "pycc_ext_import_error_bridge";

/// The fixed C shim's foreign-operation bridge (#1316): `int
/// pycc_ext_obj_error_bridge(void)` translates CPython's pending exception
/// (a failed attribute load, call, `len`, ...) into a pending pycc
/// exception and keeps the original for the host. Unlike
/// [`EXT_IMPORT_ERROR_BRIDGE_SYMBOL`] it is total: it always returns `1`
/// with a pycc exception pending and CPython's indicator clear, so a
/// failure edge branches straight to its innermost handler. Module exec
/// calls it too inside a module-level `try`, and keeps its direct `-1`
/// edge only outside every one (Part 1 of #1096, `foreign_fail.rs`).
pub const EXT_OBJ_ERROR_BRIDGE_SYMBOL: &str = "pycc_ext_obj_error_bridge";

/// The fixed C shim's read-before-import error (#1316): `void
/// pycc_ext_name_error(const unsigned char *name, long long len)` raises
/// CPython's `NameError("name '<name>' is not defined")` and bridges it
/// through [`EXT_OBJ_ERROR_BRIDGE_SYMBOL`]. Emitted on the unbound branch of
/// a function body's read of a module-level foreign name.
pub const EXT_NAME_ERROR_SYMBOL: &str = "pycc_ext_name_error";

/// The fixed C shim's attribute-load helper (Part 2 of #1026): it takes a
/// borrowed `PyObject *` and a NUL-terminated attribute name, and returns a
/// *new* reference to the attribute's value, or `NULL` with the CPython
/// exception already set.
///
/// Spelled once here for exactly the reason [`EXT_OBJ_IMPORT_SYMBOL`]
/// directly above is: the symbol is defined in `src/ext/pycc_ext_module.c`
/// and declared by LLVM in `lib.rs`'s `MirExpr::ObjAttrGet` arm, and the
/// `--ext` link resolves an undefined symbol lazily, so a misspelling on
/// either side is a crash at first call rather than a link error.
pub const EXT_OBJ_GETATTR_SYMBOL: &str = "pycc_ext_obj_getattr";

/// The fixed C shim's method-call helper (Part 2 of #1026, PR 2b of #1081):
/// it takes a borrowed `PyObject *`, a NUL-terminated method name, an array
/// of `nargs` *owned* argument references, and returns a *new* reference to
/// the call's result, or `NULL` with the CPython exception already set. It
/// consumes every argument reference on every path.
///
/// Fused rather than "getattr then call" so the bound method object never
/// becomes a pycc value and the whole operation has one failure edge; the C
/// side's own comment carries the full rationale. Spelled once here for
/// exactly the reason [`EXT_OBJ_IMPORT_SYMBOL`] is.
pub const EXT_OBJ_CALL_SYMBOL: &str = "pycc_ext_obj_call";

/// The fixed C shim's direct-call helper (#1313): `f(args)` where `f` is
/// any name typed `object` -- a foreign binding, such as `from itertools
/// import product` followed by `product("ab", "cd")`, or a `for` loop
/// target bound to an object. Same argument contract as
/// [`EXT_OBJ_CALL_SYMBOL`] -- `nargs` *owned* argument references, each
/// consumed on every path -- but the callee is *borrowed*: it is a
/// reference the caller keeps (a retained module global, or a `for` loop
/// target's slot), so the helper takes its own reference before delegating
/// to the consuming `pycc_ext_obj_call`, which releases one.
/// Returns a new reference or `NULL` with the CPython exception set.
/// Spelled once here for exactly the reason [`EXT_OBJ_IMPORT_SYMBOL`] is.
pub const EXT_OBJ_CALL_BORROWED_SYMBOL: &str = "pycc_ext_obj_call_borrowed";

/// The fixed C shim's keyword-call helper (Part 8 of #1371):
/// `o.method(x, key=v)`, `f(a, b=c)` or `Cls(arg, flag=True)` on a CPython
/// object. It takes the callable, an array of `nargs + nkw` *owned*
/// argument references -- the positional arguments, then the keyword values
/// -- the positional count `nargs`, an array of `nkw` NUL-terminated keyword
/// names and `nkw` itself. It builds the `kwnames` tuple and calls
/// `PyObject_Vectorcall`, which is CPython's own keyword-call protocol and
/// observably equivalent to `PyObject_Call` with a `kwargs` dict.
///
/// Like [`EXT_OBJ_CALL_SYMBOL`] it **consumes** the callable (a bound
/// method or another freshly produced reference) and every argument
/// reference on every path. Returns a new reference or `NULL` with the
/// CPython exception set. Spelled once here for exactly the reason
/// [`EXT_OBJ_IMPORT_SYMBOL`] is.
pub const EXT_OBJ_CALL_KW_SYMBOL: &str = "pycc_ext_obj_call_kw";

/// [`EXT_OBJ_CALL_KW_SYMBOL`] with a *borrowed* callable (Part 8 of #1371),
/// the keyword counterpart of [`EXT_OBJ_CALL_BORROWED_SYMBOL`]: the helper
/// takes its own reference to the callable before delegating, so a module
/// global such as a foreign class keeps its reference. The argument
/// references are consumed exactly as for the consuming helper.
pub const EXT_OBJ_CALL_KW_BORROWED_SYMBOL: &str = "pycc_ext_obj_call_kw_borrowed";

/// The shim's `int` argument packer: a D-141 encoded int word in, a new
/// `PyObject *` reference out, or `NULL` with an `OverflowError` set for a
/// bigint (#1040). Borrows its argument -- see the C side's own comment.
pub const EXT_OBJ_PACK_INT_SYMBOL: &str = "pycc_ext_obj_pack_int";

/// The shim's `float` argument packer (`PyFloat_FromDouble`).
pub const EXT_OBJ_PACK_FLOAT_SYMBOL: &str = "pycc_ext_obj_pack_float";

/// The shim's `bool` argument packer (`PyBool_FromLong`, so the result is
/// one of the two interned singletons).
pub const EXT_OBJ_PACK_BOOL_SYMBOL: &str = "pycc_ext_obj_pack_bool";

/// The shim's `str` argument packer: a borrowed `PyStrObj` in, an
/// independent CPython `str` out.
pub const EXT_OBJ_PACK_STR_SYMBOL: &str = "pycc_ext_obj_pack_str";

/// The shim's `object` argument packer (Part 2a of #1371): a borrowed
/// `PyObject *` in, one new reference to the same object out (`Py_INCREF`),
/// so a consuming helper can release it without touching the operand's own
/// reference.
pub const EXT_OBJ_PACK_OBJECT_SYMBOL: &str = "pycc_ext_obj_pack_object";

/// The shim's class-instance argument packer (#1435): a borrowed
/// `PyInstanceObj *` in, a new reference to the `PyccExtInstance` carrier
/// standing for it out -- the instance's live carrier when it has one, so
/// identity survives the crossing, otherwise a fresh carrier of its
/// run-time class. `pycc_types` admits an instance only as a call
/// argument.
pub const EXT_OBJ_PACK_INSTANCE_SYMBOL: &str = "pycc_ext_obj_pack_instance";

/// The fixed C shim's `len` helper (Part 3 of #1026): it takes a borrowed
/// `PyObject *` and an out-pointer, writes the D-141 encoded `int` word for
/// `PyObject_Size(o)` through it and returns `0`, or returns `-1` with a
/// CPython exception already set. It does not touch the operand's refcount.
///
/// The `PyObject_Size` call and the `pycc_rt_ext_int_encode` call are fused
/// inside the shim deliberately: either can fail, and folding both into one
/// `-1` return lets codegen emit exactly *one* foreign failure edge for
/// `len` instead of two. (The encode failure is unreachable for a real
/// container -- a length never leaves D-141's inline range -- so the second
/// branch exists only as defence in depth.) Spelled once here for exactly
/// the reason [`EXT_OBJ_IMPORT_SYMBOL`] is: the `--ext` link resolves an
/// undefined symbol lazily, so a misspelling is a crash at first call rather
/// than a link error.
pub const EXT_OBJ_LEN_SYMBOL: &str = "pycc_ext_obj_len";

/// The fixed C shim's truth-testing helper (Part 3 of #1026): it takes a
/// borrowed `PyObject *` and returns `1` for a truthy operand, `0` for a
/// falsy one, or `-1` with a CPython exception already set (`PyObject_IsTrue`
/// can run arbitrary `__bool__`/`__len__` code, so it really can raise). It
/// does not touch the operand's refcount.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_TRUTHY_SYMBOL: &str = "pycc_ext_obj_truthy";

/// The fixed C shim's `type(o)` helper (Part 11 of #1371): it takes a
/// borrowed `PyObject *` and returns a *new* reference to its class
/// (`PyObject_Type`), or `NULL` for a `NULL` operand -- the
/// defence-in-depth guard [`EXT_OBJ_LEN_SYMBOL`]'s helper documents. It does not touch the operand's refcount.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_TYPE_SYMBOL: &str = "pycc_ext_obj_type";

/// The fixed C shim's object-temporary release (Part 1 of #1092): `void
/// pycc_ext_obj_release(PyObject *o)` is `Py_XDECREF`. Generated code calls
/// it for a shim producer's new reference once the borrowing operation that
/// consumed it is done, or on the failure edge that leaves that operation
/// (`object_release.rs`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_RELEASE_SYMBOL: &str = "pycc_ext_obj_release";

/// The fixed C shim's object retain (Part 1 of #1499): `void
/// pycc_ext_obj_retain(PyObject *o)` is `Py_XINCREF`. Generated code calls
/// it before a borrowed object moves into a slot that owns its reference --
/// a module-global `object` slot, or a compiled instance's `object`
/// attribute (`object_slot.rs`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_RETAIN_SYMBOL: &str = "pycc_ext_obj_retain";

/// The fixed C shim's rebind gate (Part 1 of #1499, #1501): `int
/// pycc_ext_obj_rebind_may_release(void)` answers non-zero only when the
/// module-exec body calling it is the artifact's only live compiled
/// activation. A module-global rebind releases the replaced value only
/// then, and otherwise leaks it (`object_slot.rs`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_REBIND_MAY_RELEASE_SYMBOL: &str = "pycc_ext_obj_rebind_may_release";

/// The fixed C shim's subscript-load helper (Part 3 of #1026, PR 3b of
/// #1082): it takes a borrowed `PyObject *` and an *owned* key reference
/// produced by one of the `pycc_ext_obj_pack_*` helpers above, and returns a
/// *new* reference to `o[k]`, or `NULL` with the CPython exception already
/// set.
///
/// **It consumes the key on every path**, including the one where `o` or
/// `k` is itself `NULL`. That is deliberately the same arg-slot contract
/// [`EXT_OBJ_CALL_SYMBOL`] already imposes on the values the packers
/// produce, so the packer contract stays one rule rather than two: whatever
/// a packer creates is handed to a shim helper and is that helper's to
/// release. Folding the failed-packer test into the helper as well is what
/// leaves this operation with exactly *one* foreign failure edge, for
/// the reason [`EXT_OBJ_LEN_SYMBOL`] records for its own fused encode.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_GETITEM_SYMBOL: &str = "pycc_ext_obj_getitem";

/// The fixed C shim's rich-comparison helper (Part 1 of #1371): it takes two
/// `PyObject *` operands, CPython's `Py_LT`..`Py_GE` selector and an
/// ownership mask (bit 0 the left operand, bit 1 the right) naming which
/// operands a `pycc_ext_obj_pack_*` helper produced, and returns a *new*
/// reference to `PyObject_RichCompare(l, r, op)`, or `NULL` with the
/// CPython exception already set. It consumes every packed operand on
/// every path, for the reason [`EXT_OBJ_GETITEM_SYMBOL`] records.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_RICHCOMPARE_SYMBOL: &str = "pycc_ext_obj_richcompare";

/// The fixed C shim's membership helper (Part 2b of #1371): it takes a
/// borrowed container and a *packed* item, and returns
/// `PySequence_Contains(container, item)`'s `1`/`0`, or `-1` with the
/// CPython exception already set. It consumes the item on every path, for
/// the reason [`EXT_OBJ_GETITEM_SYMBOL`] records, so a failed packer's
/// `NULL` item is tested there and needs no failure edge of its own.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_CONTAINS_SYMBOL: &str = "pycc_ext_obj_contains";

/// The fixed C shim's slice-load helper (Part 2b of #1371): it takes a
/// borrowed base, three *packed* bounds and a presence mask (bit 0 start,
/// bit 1 stop, bit 2 step; an absent bound's pointer is ignored and passed
/// to `PySlice_New` as `None`), and returns a *new* reference to
/// `PyObject_GetItem(base, slice(start, stop, step))`, or `NULL` with the
/// CPython exception already set. It consumes every present bound on every
/// path, for the reason [`EXT_OBJ_GETITEM_SYMBOL`] records.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_GETSLICE_SYMBOL: &str = "pycc_ext_obj_getslice";

/// The fixed C shim's list-display helper (Part 2d of #1371): it takes an
/// array of `n` *packed* elements and returns a *new* reference to a fresh
/// CPython `list` holding them in order, or `NULL` with the CPython
/// exception already set. It consumes every element on every path -- a
/// `NULL` element (a failed packer) and a failed `PyList_New` included --
/// for the reason [`EXT_OBJ_GETITEM_SYMBOL`] records.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_BUILD_LIST_SYMBOL: &str = "pycc_ext_obj_build_list";

/// The fixed C shim's slice-deletion helper (Part 2c of #1371), the
/// statement twin of [`EXT_OBJ_GETSLICE_SYMBOL`] with the same arguments
/// and the same consumption of every present bound on every path. It
/// performs `PyObject_DelItem(base, slice(start, stop, step))` and returns
/// `0`, or `-1` with the CPython exception already set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_DELSLICE_SYMBOL: &str = "pycc_ext_obj_delslice";

/// The fixed C shim's attribute-store helper (#1457, Part 2 of #1443):
/// `int pycc_ext_obj_setattr(PyObject *o, const char *name, PyObject *value)`
/// with `o` borrowed and `value` a packer's new reference the helper
/// consumes on every path, a `NULL` from a failed packer included -- which
/// it reports as `-1` without calling `PyObject_SetAttrString`, whose `NULL`
/// value would *delete* the attribute. Returns `0`, or `-1` with the CPython
/// exception already set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_SETATTR_SYMBOL: &str = "pycc_ext_obj_setattr";

/// The fixed C shim's attribute-deletion helper (#1457):
/// `int pycc_ext_obj_delattr(PyObject *o, const char *name)` with `o`
/// borrowed. Returns `0`, or `-1` with the CPython exception already set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_DELATTR_SYMBOL: &str = "pycc_ext_obj_delattr";

/// The fixed C shim's `raise o` helper (Part 9 of #1371): `void
/// pycc_ext_obj_raise(PyObject *o)` with `o` borrowed. It decides what is
/// raised the way CPython's own `raise` does -- an exception class is
/// instantiated, an instance is raised as it is, and anything else raises
/// `TypeError` -- then always bridges the CPython exception into a pending
/// pycc exception, so the caller ends the block exactly like a native
/// `raise`.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_RAISE_SYMBOL: &str = "pycc_ext_obj_raise";

/// The fixed C shim's `None` accessor (Part 1 of #1371): a *borrowed*
/// pointer to CPython's immortal `None`, the right-hand side of `o is None`
/// and, since Part 8 of #1371, the value a `None` call argument is packed
/// from (`foreign_pack::none_pointer`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_NONE_SYMBOL: &str = "pycc_ext_obj_none";

/// The fixed C shim's `NotImplemented` accessor (#1418): a *borrowed*
/// pointer to CPython's `NotImplemented` singleton, the value of an
/// admitted `return NotImplemented` in a comparison method
/// (`MirExpr::NotImplemented`). A compiled return hands the borrowed
/// singleton back as is (it is immortal on CPython 3.13+); when the result
/// crosses the export boundary, the C shim's `pycc_ext_pack_object` takes
/// the strong reference the host caller owns.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_NOT_IMPLEMENTED_SYMBOL: &str = "pycc_ext_obj_not_implemented";

/// The fixed C shim's `isinstance` helper (Part 1 of #1371): it takes a
/// borrowed object, a borrowed class (or `NULL`, which selects a builtin
/// class by the third argument, `pycc_mir::ObjBuiltinClass::shim_code`) and
/// returns `PyObject_IsInstance`'s `1`/`0`, or `-1` with the CPython
/// exception already set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_ISINSTANCE_SYMBOL: &str = "pycc_ext_obj_isinstance";

/// The fixed C shim's compiled-class `isinstance` helper (Part 7 of #1371):
/// it takes a borrowed object and the NUL-terminated name of a class
/// compiled in this module, and returns `1`/`0`, or `-1` with the CPython
/// exception already set. The shim forwards to the artifact's generated
/// `pycc_ext_compiled_class_isinstance`, which tests the object against the
/// host type object of every published class whose MRO contains the named
/// one and answers `0` when no published class does.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL: &str = "pycc_ext_obj_isinstance_compiled";

/// The fixed C shim's iterator-acquisition helper (Part 3 of #1026, PR 3c
/// of #1082): it takes a borrowed `PyObject *` and returns a *new*
/// reference to `iter(o)` -- `PyObject_GetIter` -- or `NULL` with the
/// CPython exception already set (a non-iterable operand raises
/// `TypeError` there, which is exactly the behaviour pycc wants to
/// surface).
///
/// The iterator is read once, in the loop preheader, and released when the
/// loop ends or fails (Part 3 of #1092). The iterable
/// it was taken from is released right after this call when it is a produced
/// temporary (Part 1 of #1092, `object_release.rs`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_GET_ITER_SYMBOL: &str = "pycc_ext_obj_get_iter";

/// The fixed C shim's iterator-advance helper (Part 3 of #1026, PR 3c of
/// #1082). Unlike every other helper here it is *three-valued*: given a
/// borrowed iterator and an out-parameter, it returns `1` having written a
/// *new* reference to the next item through `*out`, `0` on clean
/// exhaustion, or `-1` with a CPython exception already set.
///
/// **The discrimination lives in C, not in LLVM IR.** `PyIter_Next`
/// signals both exhaustion and failure with `NULL`, and only
/// `PyErr_Occurred()` tells the two apart; open-coding that in emitted IR
/// would put a second, independently-maintained copy of a CPython calling
/// convention into this crate. Collapsing the two into one value here is
/// what keeps exhaustion off the module-exec failure edge: a `for` loop
/// that simply ends is not a failure.
///
/// Each item written through `*out` is a new reference. A module-global
/// `for` target owns it and releases it on the next trip (Part 1 of #1499);
/// a comprehension's loop variable does not own it, so the comprehension
/// leaks one item per trip (#1499, `docs/RUNTIME.md`).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_ITER_NEXT_SYMBOL: &str = "pycc_ext_obj_iter_next";

/// The fixed C shim's collection constructor (Part 1 of #1255): given a
/// [`ObjCollectionKind`] code it returns a *new* reference to an empty
/// CPython `list` (`0`) or `set` (`1`), or `NULL` with the CPython
/// exception already set. A list or set comprehension over a CPython object
/// builds its result in it; the reference is the comprehension's value,
/// released by its consumer when unbound (Part 3 of #1092), owned by a
/// module global that binds it (Part 1 of #1499), and leaked otherwise.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_NEW_COLLECTION_SYMBOL: &str = "pycc_ext_obj_new_collection";

/// The fixed C shim's collection insert (Part 1 of #1255): given a borrowed
/// collection from [`EXT_OBJ_NEW_COLLECTION_SYMBOL`], the same kind code and
/// a *packed* item, it appends the item to the list or adds it to the set
/// and returns `0`, or returns `-1` with the CPython exception already set
/// (an unhashable set item raises `TypeError` there).
///
/// **Ownership.** The item is a new reference a `pycc_ext_obj_pack_*`
/// helper produced, and it is consumed on every path, exactly as
/// `pycc_ext_obj_call` consumes its arguments: a `NULL` item is a packer
/// that already set the exception, and is reported as a failure.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_COLLECT_SYMBOL: &str = "pycc_ext_obj_collect";

/// Which CPython collection a comprehension over an object builds (Part 1
/// of #1255): the code [`EXT_OBJ_NEW_COLLECTION_SYMBOL`] and
/// [`EXT_OBJ_COLLECT_SYMBOL`] take. Spelled once here so the two helpers'
/// shared numbering cannot drift between their callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjCollectionKind {
    /// A `list`, from `[elt for x in <object>]`.
    List,
    /// A `set`, from `{elt for x in <object>}`.
    Set,
}

impl ObjCollectionKind {
    /// The kind code the shim helpers switch on.
    pub fn shim_code(self) -> u64 {
        match self {
            ObjCollectionKind::List => 0,
            ObjCollectionKind::Set => 1,
        }
    }
}

/// The fixed C shim's `float(o)` conversion helper (Part 4 of #1026, PR 4a
/// of #1083): it takes a borrowed `PyObject *` and a `double *`
/// out-parameter, writes the converted value and returns `0`, or returns
/// `-1` with the CPython exception already set.
///
/// **This is an explicit conversion, not an implicit boundary crossing.**
/// D-244 rule 7 keeps the type boundary closed at the *thunk export seam*,
/// where a value crosses implicitly and the annotation is the whole
/// contract. `float(o)` in user source names its destination type, so
/// running CPython's own `PyNumber_Float` protocol -- the operand's
/// `__float__`, `__index__` or string parse -- is precisely what the author
/// asked for. `docs/TYPE_SYSTEM.md`'s `object` row and the helper's own C
/// comment record the same distinction.
///
/// **Ownership.** The helper releases the temporary `PyNumber_Float`
/// produces on *every* exit, including the failing one, and nothing but a
/// `double` escapes into compiled code -- so unlike an attribute load or a
/// subscript, this operation adds nothing to the #1092 leak-only set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_TO_FLOAT_SYMBOL: &str = "pycc_ext_obj_to_float";

/// The fixed C shim's `int(o)` conversion helper (Part 4 of #1026, PR 4b
/// of #1083): it takes a borrowed `PyObject *` and a `long long *`
/// out-parameter, writes a **D-141 encoded** integer word and returns `0`,
/// or returns `-1` with the CPython exception already set.
///
/// **This is an explicit conversion, not an implicit boundary crossing.**
/// The distinction [`EXT_OBJ_TO_FLOAT_SYMBOL`] records applies unchanged:
/// D-244 rule 7 closes the type boundary at the *thunk export seam*, and
/// `int(o)` in user source names its destination type, so running CPython's
/// own `PyNumber_Long` protocol is what the author asked for. It is
/// therefore *not* `pycc_ext_unpack_int_at`, whose `PyBool_Check` and
/// `PyLong_Check` guards exist precisely because that seam is closed.
///
/// **Overflow.** The encode is fused into the helper, exactly as
/// [`EXT_OBJ_LEN_SYMBOL`]'s is and for its reason (one failure edge rather
/// than two). A value outside pycc's inline-integer range
/// `[-2**62, 2**62-1]` raises `OverflowError` citing #1040 -- there is no
/// bigint path across this boundary.
///
/// **Ownership.** The helper releases the `PyNumber_Long` temporary on
/// *every* exit, including the `OverflowError` path that still holds it, so
/// this operation adds nothing to the #1092 leak-only set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_TO_INT_SYMBOL: &str = "pycc_ext_obj_to_int";

/// The fixed C shim's `str(o)` conversion helper (Part 4 of #1026, PR 4b of
/// #1083): it takes a borrowed `PyObject *` and a `void **` out-parameter,
/// writes a pycc `PyStrObj *` at refcount 1 and returns `0`, or returns `-1`
/// with the CPython exception already set.
///
/// **This is an explicit conversion, not an implicit boundary crossing.**
/// See [`EXT_OBJ_TO_FLOAT_SYMBOL`]; `PyObject_Str` *is* `str()`, so no other
/// answer is defensible. Unlike `pycc_ext_unpack_str` it does not
/// `PyUnicode_Check` its operand -- refusing a non-`str` is exactly what an
/// explicit conversion must not do.
///
/// **Ownership.** The handle written through the out-parameter is produced
/// by `pycc_rt_str_from_literal`, the same call a `str` literal's own
/// emission uses, and arrives as compiled code's own reference -- so this
/// crate needs no new rule for it. On the C side the copy must complete
/// *before* the `PyObject_Str` result is released, because
/// `PyUnicode_AsUTF8AndSize` points into that result's buffer; the helper's
/// own comment records why that ordering is load-bearing. The temporary is
/// released on every exit, so nothing joins the #1092 leak-only set.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_TO_STR_SYMBOL: &str = "pycc_ext_obj_to_str";

/// The fixed C shim's f-string interpolation helper (#1340): it takes a
/// borrowed `PyObject *` and a `void **` out-parameter, writes a pycc
/// `PyStrObj *` at refcount 1 and returns `0`, or returns `-1` with the
/// CPython exception already set.
///
/// [`EXT_OBJ_TO_STR_SYMBOL`]'s contract, ownership and copy-before-release
/// ordering exactly, with `PyObject_Format(o, NULL)` in place of
/// `PyObject_Str(o)`: CPython renders `f"{value}"` as `format(value, '')`,
/// which reaches the value's `__format__`, not its `__str__`. `print(o)`
/// writes `str(o)` and so reaches [`EXT_OBJ_TO_STR_SYMBOL`] instead.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_FORMAT_SYMBOL: &str = "pycc_ext_obj_format";

/// The fixed C shim's narrowed-read helpers (#1476, Part 3 of #1387): the
/// read of an `object` name an `isinstance(o, int|float|bool|str)` guard
/// narrowed (`pycc_mir::MirExpr::ObjectUnbox`). Each takes a borrowed
/// `PyObject *` and an out-parameter of the native ABI type (`long long`
/// D-141 word, `double`, one-byte `char`, `PyStrObj *`), writes the value
/// and returns `0`, or returns `-1` with the CPython exception set.
///
/// The read is an implicit crossing, so each helper keeps D-244 rule 7's
/// closed type check rather than CPython's conversion protocol: a `bool`
/// under an `int` guard keeps its D-141 marker word, an `int` outside the
/// inline range raises `OverflowError` citing #1040, and a `str` subclass
/// is copied into a plain pycc `str` at refcount 1 (the ownership
/// [`EXT_OBJ_TO_STR_SYMBOL`] documents). No CPython reference is taken.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_UNBOX_INT_SYMBOL: &str = "pycc_ext_obj_unbox_int";
/// See [`EXT_OBJ_UNBOX_INT_SYMBOL`].
pub const EXT_OBJ_UNBOX_FLOAT_SYMBOL: &str = "pycc_ext_obj_unbox_float";
/// See [`EXT_OBJ_UNBOX_INT_SYMBOL`].
pub const EXT_OBJ_UNBOX_BOOL_SYMBOL: &str = "pycc_ext_obj_unbox_bool";
/// See [`EXT_OBJ_UNBOX_INT_SYMBOL`].
pub const EXT_OBJ_UNBOX_STR_SYMBOL: &str = "pycc_ext_obj_unbox_str";
/// The narrowed read under `isinstance(o, C)` for a regular class `C`
/// compiled in this module (#1476): a borrowed `PyObject *`, the class's
/// NUL-terminated name and a `void **` out-parameter; writes the compiled
/// instance the carrier holds, borrowed (compiled instances are never
/// freed), and returns `0`, or returns `-1` with a `TypeError` set for an
/// uninitialized carrier or an object that is not a carrier of this module.
pub const EXT_OBJ_UNBOX_INSTANCE_SYMBOL: &str = "pycc_ext_obj_unbox_instance";

/// The fixed C shim's fixed-arity all-`float` tuple unpack helper (Part 4
/// of #1026, PR 4c of #1083): it takes a borrowed `PyObject *`, the
/// declared arity and a `double *` out-array, writes that many converted
/// doubles and returns `0`, or returns `-1` with the CPython exception
/// already set.
///
/// **Strict container, converting elements.** The helper checks
/// `PyTuple_Check` with an *exact*-arity test and then converts each item
/// with `PyNumber_Float`. The two halves answer two different questions:
/// D-115/D-116 hold a tuple as a by-value LLVM struct of fixed width, so
/// there is no shape a `list`, a generator or a differently-sized tuple
/// could be written into -- while the elements' `float` is a type the
/// author wrote in the annotation, which makes running CPython's own
/// conversion protocol on them the same explicit-conversion case
/// [`EXT_OBJ_TO_FLOAT_SYMBOL`] records. It is therefore *not*
/// `pycc_ext_unpack_float_at`, whose `PyFloat_Check` refusal exists because
/// the thunk export seam is closed.
///
/// **The arity is a parameter, never a constant.** The admission rule is
/// any fixed arity with every element `float`, so neither this declaration
/// nor the shim may hard-code the three of `tuple[float, float, float]`.
///
/// **Ownership.** Each `PyNumber_Float` temporary is released inside the
/// same loop iteration that produced it, so the failing exit holds nothing
/// and this operation adds nothing to the #1092 leak-only set -- Part 4's
/// property, unchanged.
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL: &str = "pycc_ext_obj_unpack_float_tuple";

/// The fixed C shim's tuple-unpacking helper (Part 1 of #891): it takes a
/// borrowed `PyObject *` and the target count `n`, and returns a *new*
/// reference to a `tuple` of exactly `n` items taken from the object by
/// CPython's own unpack protocol, or `NULL` with CPython's own exception
/// set -- `TypeError` for a non-iterable, `ValueError` for too many or too
/// few values. The tuple is bound to the unpacking temporary: leaked on the
/// #1092 rule for a bound value in a function body, owned and released on
/// rebind by a module-global temporary (Part 1 of #1499).
///
/// Spelled once here for the same lazy-link reason as [`EXT_OBJ_LEN_SYMBOL`].
pub const EXT_OBJ_UNPACK_SYMBOL: &str = "pycc_ext_obj_unpack";

/// The C-legal spelling of a possibly-dotted pycc name.
///
/// A method reaches MIR under a dotted name (`Grid.scale.static`), and two
/// of the places that name is used are *C identifiers*: the
/// `extern void *fnptr_<name>;` declaration `pycc::ext_build`'s
/// `wrapper_for` emits, and the `pycc_ext_wrap_<name>` /
/// `pycc_ext_thunk_<name>` symbols. `extern void *fnptr_Grid.scale.static;`
/// is not accepted by any C compiler, so the dotted spelling has to be
/// encoded -- and `src/ext_build.rs`'s rebind-dedup comment records that a
/// *duplicate* export name makes clang reject the generated `.inc`
/// outright, so the encoding has to be injective as well as legal.
///
/// The encoding: a name with no `.` is returned unchanged; otherwise the
/// result is `"0m"` followed by, for each `.`-separated segment in order,
/// the segment's byte length in decimal, then `"_"`, then the segment. So
/// `f` stays `f` and `Grid.scale.static` becomes `0m4_Grid5_scale6_static`.
///
/// **It is injective, and that is argued rather than fixture-tested.**
/// Every dot-free name reaching this function is either a Python identifier
/// or carries the compiler-generated `0gen_` prefix
/// (`pycc_types::monomorphize`), and neither can begin with `0m` -- the
/// premise is stated this way rather than as "Python identifiers cannot
/// begin with a digit", because a dot-free name like `0gen_make__T_int` is
/// a real identity-branch input that *does* begin with a digit. So the
/// identity branch's outputs never collide with a `0m...` output. Within
/// the `0m` branch the encoding is length-prefixed and therefore uniquely
/// decodable, so two distinct dotted names cannot mangle alike. `.` -> `_`
/// and `.` -> `__` both fail this argument, the second one silently: a
/// module-level `def Grid__scale__static` would collide with
/// `Grid.scale.static`.
///
/// Every use site prefixes the result (`fnptr_`, `fnname_`,
/// `pycc_ext_thunk_`, `pycc_ext_wrap_`), so the leading digit never starts
/// a C identifier. Because the dot-free case is the identity, every symbol
/// the compiler emitted before methods became exportable is byte-identical.
///
/// This is the one canonical implementation: `pycc::ext_build` calls it
/// rather than reimplementing it, exactly as it already calls
/// [`ext_thunk_symbol`].
#[must_use]
pub fn mangle_ext_name(name: &str) -> String {
    if !name.contains('.') {
        return name.to_string();
    }
    let mut out = String::from("0m");
    for segment in name.split('.') {
        out.push_str(&segment.len().to_string());
        out.push('_');
        out.push_str(segment);
    }
    out
}

/// The external symbol `name`'s scalar-only `ext` export thunk is emitted
/// under.
///
/// `name` is mangled through [`mangle_ext_name`] first, so a method's thunk
/// is a legal C identifier the generated `extern` declaration can name. The
/// mangling is the identity for a dot-free name, so every module-level
/// function's thunk symbol is unchanged.
#[must_use]
pub fn ext_thunk_symbol(name: &str) -> String {
    format!("{EXT_THUNK_PREFIX}{}", mangle_ext_name(name))
}

/// Whether `name` is a name D-244 rule 1 can export at all, disregarding
/// its signature.
///
/// The tests are exactly `pycc::ext_build::collect_exports`' own *lexical*
/// verdict, and this is their one canonical home so the two sides of the
/// seam cannot drift: a wrapper generated for a name codegen declined to
/// emit a thunk for links cleanly and crashes on the first call.
/// `src/ext_build_tests/exports.rs` carries a test pinning the two equal
/// over a shared table of names, because the drift is otherwise silent --
/// `wrapper_for` picks the thunk `extern` or the `fnptr_` `extern` from
/// [`ext_thunk_required`], so a disagreement emits the wrong C declaration
/// for a `tuple`-carrying method.
///
/// Two name classes are answered `true` beyond that lexical verdict: the
/// PEP 562 module hooks `__getattr__` and `__dir__` (#1467), which the
/// driver admits through `is_module_hook`, and `<Class>.<slot dunder>`
/// (#1427), which it admits through `is_slot_dunder_method`, neither through
/// `classify_export_name`. So the parity test compares this function with
/// `classify_export_name(name).is_some() || is_module_hook(name) ||
/// is_slot_dunder_method(name)` -- the last for #1427's comparison and
/// `__hash__` slots, admitted for any non-empty class segment.
///
/// The public-name test is D-038's predicate, spelled out rather than
/// delegated to `pycc_hir::is_public_name` because this crate deliberately
/// does not depend on `pycc_hir` -- it sees only `pycc_mir`'s re-export of
/// `Ty`. The body there is `!name.starts_with('_')` and nothing else; if it
/// ever grows a case, this copy must grow with it.
///
/// **Dotted names are no longer refused wholesale.** A method reaches MIR
/// under `<Class>.<method>` and the suffixed spellings
/// `<Class>.<method>.static`, `<Class>.<method>.classmethod` and
/// `<Class>.<property>.setter` (`pycc_hir::class`'s mangling). This admits
/// the `.static` and `.classmethod` spellings and -- since #1145 -- the
/// bare `<Class>.<method>` one, with every segment public.
/// `<Class>.<property>.setter` stays refused: a `@property` is attribute
/// syntax on the host side, never a method table entry.
///
/// The bare spelling covers the three `MethodKind`s `Regular`,
/// `PropertyGetter` and `AbstractMethod` at once, and nothing here can tell
/// them apart. Admitting it is deliberate rather than an approximation:
/// widening this mirror is what makes it a **superset** of the driver's
/// admitted set again, and a superset is the safe direction. The driver
/// narrows the bare spelling back down with filters that read
/// `HirModule::class_defs`; leaving this side refusing it would instead
/// make `ext_thunk_required` answer `false` for a `tuple`-carrying instance
/// method, so the wrapper would emit the `fnptr_` cast form -- measured to
/// fault (SIGBUS) on aarch64-apple-darwin for an out-pointer signature.
///
/// **This verdict is purely lexical, and must stay so.** The function
/// receives a bare `&str` and this crate cannot see `pycc_hir`, so a
/// verdict that consulted `HirModule::class_defs` would have no
/// mirror-comparable form here and the parity test would stop being
/// well-formed. The driver layers its exception-class exclusion *on top of*
/// this verdict rather than inside it, which makes this mirror a
/// **superset** of the driver's admitted set: at worst a thunk is emitted
/// for a name no wrapper calls, which is dead code -- never the link error
/// the drift above would be.
///
/// A monomorphized generic specialization carries the `0gen_` prefix and
/// has no `fnptr_` global to dispatch through, so it stays refused; that
/// test is applied to the whole name *before* the split and takes
/// precedence, as cheap defense in depth. It is not a live hazard: the real
/// specialization shapes put the substitution suffix last
/// (`0gen_<Class>.<method>__<P>_<C>`), so a `0gen_` name's last segment is
/// never `static`.
///
/// A receiver-exact copy of an inherited body (#1337, D-254) needs no case
/// of its own: a primary copy is spelled exactly as the receiver's own
/// definition would be (`C.m`, `C.k.classmethod`) and is judged as one, and
/// a `super()`-target copy (`C.m.0super_D`, plus a kind suffix for a setter
/// or classmethod) carries a third segment that is
/// neither `static` nor `classmethod`, so it is refused here and by the
/// driver alike -- it is only ever called from another copy, never hosted.
#[must_use]
pub fn is_ext_exportable_name(name: &str) -> bool {
    if name.starts_with("0gen_") {
        return false;
    }
    // #1467: the two PEP 562 module hooks, which the driver publishes from
    // the entry module although D-038's predicate refuses every dunder. The
    // driver admits only the entry module's own `def`, which this lexical
    // mirror cannot see, so a helper module's hook gets a dead thunk -- the
    // safe superset direction described above.
    if name == "__getattr__" || name == "__dir__" {
        return true;
    }
    if is_slot_dunder_method_name(name) {
        return true;
    }
    let mut segments = name.split('.');
    // `str::split` always yields at least one segment, so the fallback is
    // unreachable rather than a second refusal path; an empty first segment
    // is refused on the next line either way, which is what the driver's
    // mirror does with its own empty-segment guard.
    let first = segments.next().unwrap_or("");
    if first.starts_with('_') || first.is_empty() {
        return false;
    }
    let Some(second) = segments.next() else {
        // A dot-free name: a module-level function, admitted by D-038's
        // predicate alone.
        return true;
    };
    if second.starts_with('_') || second.is_empty() {
        return false;
    }
    match segments.next() {
        // `<Class>.<method>` -- `Regular`, `PropertyGetter` or
        // `AbstractMethod`, indistinguishable here and all admitted as the
        // superset the driver narrows (#1145).
        None => true,
        Some(kind) => {
            // The only four-segment name is a `super()`-target copy of a
            // setter or classmethod (`C.k.0super_D.classmethod`, D-254),
            // which is never hosted: a class nested in a class or a
            // function is refused by `pycc_hir` (`stmt.rs`, `class.rs`), so
            // no `A.B.method` name exists. Refusing every fourth segment is
            // the fail-closed reading.
            segments.next().is_none() && (kind == "static" || kind == "classmethod")
        }
    }
}

/// The special methods the `--ext` driver installs as a type slot rather
/// than a `PyMethodDef` row (#1427): the six rich comparisons and
/// `__hash__`. Mirrors `src/ext_build/richcompare.rs`'s `SLOT_DUNDERS`;
/// the parity test in `src/ext_build_tests/exports.rs` keeps the two equal.
const SLOT_DUNDERS: [&str; 7] = [
    "__lt__", "__le__", "__eq__", "__ne__", "__gt__", "__ge__", "__hash__",
];

/// #1427: whether `name` is `<Class>.<slot dunder>` -- a comparison or
/// `__hash__` method the driver wraps for a `tp_richcompare`/`tp_hash`
/// slot. The class segment may be private: an unpublished class whose
/// instance crosses gets a carrier type carrying the same slots, so its
/// wrapper needs the thunk exactly as a published one's does. A `0gen_`
/// name is refused by the caller before this runs.
fn is_slot_dunder_method_name(name: &str) -> bool {
    let mut segments = name.split('.');
    let class = segments.next().unwrap_or("");
    let Some(method) = segments.next() else {
        return false;
    };
    !class.is_empty() && segments.next().is_none() && SLOT_DUNDERS.contains(&method)
}

/// The boundary slots a value of type `ty` occupies when it crosses the
/// `ext` seam: a `tuple`'s elements, in order, or the type itself.
///
/// D-116 fixes a tuple's arity and restricts its elements to `int`, `bool`
/// and `float` (`pycc_hir`'s `check_tuple_element_ty` rejects anything else
/// with `T0039`, and `tuple[int, ...]`/`tuple[()]` with `T0053`), so the
/// returned slice is always non-empty and never itself contains a tuple.
#[must_use]
pub fn ext_boundary_slots(ty: &Ty) -> &[Ty] {
    match ty {
        Ty::Tuple(elems) => elems.as_slice(),
        _ => std::slice::from_ref(ty),
    }
}

/// The thunk's own parameter list: every declared parameter flattened in
/// place to the scalars it occupies.
#[must_use]
pub fn ext_thunk_param_tys(param_tys: &[Ty]) -> Vec<Ty> {
    param_tys
        .iter()
        .flat_map(|ty| ext_boundary_slots(ty).iter().cloned())
        .collect()
}

/// The element types a `tuple` return is handed back through, one trailing
/// out-pointer each, appended after every flattened parameter. Empty for
/// every other return type, which stays a real return value.
///
/// Returning a tuple *by value* is not an option: pycc's own aggregate
/// calling convention is not the platform C struct ABI. Measured on
/// aarch64-apple-darwin, a pycc function returning `tuple[int, int, int,
/// int, int]` hands the five words back in `x0`-`x4` where clang's C ABI
/// would pass a hidden `sret` pointer -- so a C declaration of the compiled
/// function would disagree with it silently. The thunk exists precisely to
/// keep every aggregate on the LLVM side of the seam.
///
/// Part 2 of #1175 (#1179) adds the second producer of out-pointers, and it
/// is **not** a function of the declared return type: an export declared
/// `-> memoryview` carries three trailing `i64` out-slots only when its own
/// body returns a sub-range of a buffer parameter. `has_slice`, `start` and
/// `stop`, in that order. Keying this on `Ty::MemoryView` alone would flip
/// every existing buffer-returning export off the direct `fnptr_<name>`
/// cast path and onto the thunk path, changing generated C for exports this
/// change does not touch; hence the explicit per-body argument.
///
/// The third slot is load-bearing. `stop = -1` is not "absent", it is the
/// legal `b[0:-1]`, and `i64::MIN`/`i64::MAX` are legal and clamp
/// correctly, so no pair of `i64`s is available as an in-band
/// whole-view sentinel.
#[must_use]
pub fn ext_thunk_out_tys(return_ty: &Ty, returns_buffer_slice: bool) -> Vec<Ty> {
    match return_ty {
        Ty::Tuple(elems) => (**elems).clone(),
        _ if returns_buffer_slice => vec![Ty::Int, Ty::Int, Ty::Int],
        _ => Vec::new(),
    }
}

/// Whether `body` contains the admitted buffer sub-range `return`
/// (Part 2 of #1175, #1179).
///
/// The codegen-side half of a fact the driver computes independently from
/// HIR (`src/ext_build.rs`'s `ExtExport::returns_buffer_slice`). `ExtExport`
/// is a driver-only type that never reaches this crate, and the fact governs
/// both the compiled function's own LLVM signature and the generated C call
/// form, so each side must answer it from the IR it has. Because
/// `MirStmt::ReturnBufferSlice` exists precisely so the admitted shape has a
/// node of its own, this walk is an exact presence test rather than a second
/// copy of the admission rule -- the same mirror relationship
/// [`is_ext_exportable_name`] already has with the driver's own export
/// predicate, and pinned by the cross-crate parity test.
///
/// Exhaustive over every nested-body statement form, modelled on
/// `pycc_mir`'s own `set_frame_function`, so a `return b[i:j]` inside an
/// `if`, a loop or a `try` is found -- branching provenance is admitted and
/// must be.
#[must_use]
pub fn body_returns_buffer_slice(body: &[pycc_mir::MirStmt]) -> bool {
    use pycc_mir::MirStmt;
    body.iter().any(|stmt| match stmt {
        MirStmt::ReturnBufferSlice { .. } => true,
        MirStmt::If { body, orelse, .. } => {
            body_returns_buffer_slice(body) || body_returns_buffer_slice(orelse)
        }
        MirStmt::While { body, .. }
        | MirStmt::ForRange { body, .. }
        | MirStmt::ForList { body, .. }
        | MirStmt::ForObject { body, .. }
        | MirStmt::ForDict { body, .. }
        | MirStmt::ForSet { body, .. } => body_returns_buffer_slice(body),
        MirStmt::Seq(stmts) => body_returns_buffer_slice(stmts),
        MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_returns_buffer_slice(body)
                || handlers
                    .iter()
                    .any(|handler| body_returns_buffer_slice(&handler.body))
                || body_returns_buffer_slice(orelse)
                || body_returns_buffer_slice(finalbody)
        }
        MirStmt::ExprStmt(_)
        | MirStmt::Assign { .. }
        | MirStmt::NoOp
        | MirStmt::Unreachable
        | MirStmt::DictSet { .. }
        | MirStmt::BufferSet { .. }
        | MirStmt::ListCompAssign { .. }
        | MirStmt::DictCompAssign { .. }
        | MirStmt::SetCompAssign { .. }
        | MirStmt::Return(_)
        | MirStmt::AttrSet { .. }
        | MirStmt::ObjDelSlice { .. }
        | MirStmt::ObjAttrSet { .. }
        | MirStmt::ObjDelAttr { .. }
        | MirStmt::ObjRaise { .. }
        | MirStmt::Raise { .. }
        | MirStmt::RaiseFrom { .. }
        | MirStmt::ForeignImport { .. }
        | MirStmt::Reraise => false,
    })
}

/// Every function *name* whose LLVM signature must carry the three
/// buffer-sub-range out-pointers (Part 2 of #1175, #1179).
///
/// The fact is a property of the **name**, not of one `def`: every
/// definition of a name shares one `fnptr_<name>` slot and one
/// `UserFunction::fn_type`, so a redefinition cannot give two definitions
/// two different arities without making some indirect call ill-typed. This
/// therefore unions over every definition -- if *any* `def f` returns a
/// sub-range, every `def f` is widened, and the driver's own half of the
/// fact unions the same way at its dedup site (`ExtExport` in
/// `src/ext_build.rs`).
///
/// "Union" rather than "the last definition wins" is what keeps a
/// definition that slices from ever being the un-widened one: its
/// `MirStmt::ReturnBufferSlice` would then store through parameters that
/// do not exist. The cost is a widened definition whose own body only ever
/// does a bare `return b`, and that is exactly the case
/// `MirStmt::Return`'s own `has_slice = 0` store exists to describe -- the
/// two halves are load-bearing for each other.
#[must_use]
pub fn buffer_slice_out_names(items: &[pycc_mir::MirItem]) -> std::collections::BTreeSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            pycc_mir::MirItem::Function { name, body, .. } if body_returns_buffer_slice(body) => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

/// Whether a function needs a scalar-only export thunk emitted for it.
///
/// Only a signature that actually carries an aggregate does: a scalar-only
/// export's generated wrapper still reaches the compiled function through
/// the `fnptr_<name>` global directly, and emitting a thunk it would never
/// call would be dead weight in every artifact.
///
/// Part 2 of #1175 (#1179) adds one non-signature reason: an export whose
/// body returns a sub-range of a buffer parameter needs the three trailing
/// out-pointers [`ext_thunk_out_tys`] describes, so it can no longer be a
/// pure function of the declared signature. Every other `-> memoryview`
/// export keeps the direct cast path byte-for-byte, which is what the
/// `returns_buffer_slice` argument buys -- it is passed in rather than
/// inferred so the driver and codegen halves of the fact stay two
/// deliberately mirrored computations rather than three.
#[must_use]
pub fn ext_thunk_required(
    name: &str,
    param_tys: &[Ty],
    return_ty: &Ty,
    returns_buffer_slice: bool,
) -> bool {
    is_ext_exportable_name(name)
        && (param_tys.iter().any(|ty| matches!(ty, Ty::Tuple(_)))
            || matches!(return_ty, Ty::Tuple(_))
            || returns_buffer_slice)
}
