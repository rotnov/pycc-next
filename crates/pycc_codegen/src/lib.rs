use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::FileType;
use inkwell::types::BasicType;
use inkwell::values::{FloatValue, FunctionValue, IntValue, PointerValue};
use pycc_mir::{EXCEPTION_GROUP_TYPE_TAG, MirExceptionValue, MirExpr, MirItem, MirModule, MirStmt};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod exception;
mod fallthrough;
use exception::{ExceptionCodegenState, expression_can_set_exception, guard_statement_effects};
mod exception_value;
use exception_value::{emit_exception_set_frame, emit_exception_value};
mod attr_slot;
use attr_slot::{
    decref_str_attr_slot_before_store, release_int_attr_slot_before_store, scalar_to_slot_word,
    slot_word_to_scalar,
};
mod bigint_rc;
/// `<< >> & | ^` operand encoding and the `bool`-result arm (#1210).
mod binop;
/// `and`/`or` short circuit and join (#1211).
mod boolop;
mod call_result;
mod compare;
mod compare_chain;
mod comprehension;
mod copy_slots;
mod if_exp;
use bigint_rc::{
    BigIntRefcount, emit_bigint_refcount_call, pop_pending_int_release,
    push_pending_int_release_if_scalar_temporary, push_pending_int_release_if_temporary,
    release_if_int_temporary, release_int_slot_before_store,
    release_optional_int_slot_before_store, release_scalar_if_int_temporary,
    retain_if_int_duplicate, retain_if_int_duplicate_and_track_for_exception_edge,
};
use comprehension::{CompCx, CompElts, emit_comprehension, emit_comprehension_expr};
mod int_const;
use int_const::{emit_int_constant, tag_smallint_const};
mod exception_isinstance;
mod exception_render;
use exception_render::emit_exception_message;
mod str_rc;
use str_rc::{decref_str_slot_before_store, incref_if_str_duplicate};
mod rt_fns;
use rt_fns::{RtFns, declare_rt_functions};
mod ext;
mod ext_publish;
mod ext_thunk;
mod foreign_attr;
mod foreign_call;
mod foreign_call_emit;
mod foreign_compare;
mod foreign_fail;
mod foreign_import;
mod foreign_len;
mod foreign_pack;
mod foreign_raise;
mod foreign_slice;
mod foreign_store;
mod foreign_type;
mod foreign_unpack;
/// `frozenset(...)` construction and set truthiness (Part 1 of #1319).
mod frozenset;
mod hash;
mod object_box;
mod object_comprehension;
mod object_release;
mod object_return;
mod object_slot;
mod object_unbox;
mod sequence;
/// Set insertion, length and iteration helpers, and the insert of a set of
/// user-class instances (#1343, Part 1 of #1336).
mod set_instance;
use set_instance::{
    SetEmitter, build_int_set_add, build_int_set_check_not_resized, build_int_set_get,
    build_int_set_len, expect_set_pointer, set_element_scalar,
};

/// One Part 4 conversion emitter in `foreign_len.rs` (PR 4b of #1083).
///
/// Named so `emit_expr`'s `int`/`str` arm can pick between
/// [`foreign_len::emit_to_int`] and [`foreign_len::emit_to_str`] by value
/// instead of duplicating the arity and operand backstops around each. The
/// `for<'a>` binder is what lets one alias serve both, since the emitters
/// are generic over the LLVM context's lifetime rather than over a type.
type ObjectConversionEmitter = for<'a> fn(
    &'a Context,
    &inkwell::builder::Builder<'a>,
    &inkwell::module::Module<'a>,
    &RtFns<'a>,
    Scalar<'a>,
) -> Scalar<'a>;
mod slot_bindings;
mod string_render;
#[cfg(test)]
use slot_bindings::collect_expr_bindings;
use slot_bindings::{collect_module_bindings, collect_stmt_bindings};
mod target_machine;
pub use ext::{
    CompileOptions, EXT_MODULE_EXEC_FAILED, EXT_MODULE_EXEC_SYMBOL, EXT_OBJ_IMPORT_FROM_SYMBOL,
    EXT_PUBLISH_SYMBOL, EXT_THUNK_PREFIX, body_returns_buffer_slice, buffer_slice_out_names,
    ext_boundary_slots, ext_thunk_out_tys, ext_thunk_param_tys, ext_thunk_required,
    ext_thunk_symbol, is_ext_exportable_name, mangle_ext_name,
};
use ext::{
    EXT_NAME_ERROR_SYMBOL, EXT_OBJ_BUILD_LIST_SYMBOL, EXT_OBJ_CALL_BORROWED_SYMBOL,
    EXT_OBJ_CALL_KW_BORROWED_SYMBOL, EXT_OBJ_CALL_KW_SYMBOL, EXT_OBJ_CALL_SYMBOL,
    EXT_OBJ_COLLECT_SYMBOL, EXT_OBJ_CONTAINS_SYMBOL, EXT_OBJ_DELATTR_SYMBOL,
    EXT_OBJ_DELSLICE_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL, EXT_OBJ_FORMAT_SYMBOL,
    EXT_OBJ_GET_ITER_SYMBOL, EXT_OBJ_GETATTR_SYMBOL, EXT_OBJ_GETITEM_SYMBOL,
    EXT_OBJ_GETSLICE_SYMBOL, EXT_OBJ_IMPORT_SYMBOL, EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL,
    EXT_OBJ_ISINSTANCE_SYMBOL, EXT_OBJ_ITER_NEXT_SYMBOL, EXT_OBJ_LEN_SYMBOL,
    EXT_OBJ_NEW_COLLECTION_SYMBOL, EXT_OBJ_NONE_SYMBOL, EXT_OBJ_NOT_IMPLEMENTED_SYMBOL,
    EXT_OBJ_PACK_BOOL_SYMBOL, EXT_OBJ_PACK_FLOAT_SYMBOL, EXT_OBJ_PACK_INSTANCE_SYMBOL,
    EXT_OBJ_PACK_INT_SYMBOL, EXT_OBJ_PACK_OBJECT_SYMBOL, EXT_OBJ_PACK_STR_SYMBOL,
    EXT_OBJ_RAISE_SYMBOL, EXT_OBJ_RICHCOMPARE_SYMBOL, EXT_OBJ_SETATTR_SYMBOL,
    EXT_OBJ_TO_FLOAT_SYMBOL, EXT_OBJ_TO_INT_SYMBOL, EXT_OBJ_TO_STR_SYMBOL, EXT_OBJ_TRUTHY_SYMBOL,
    EXT_OBJ_TYPE_SYMBOL, EXT_OBJ_UNBOX_BOOL_SYMBOL, EXT_OBJ_UNBOX_FLOAT_SYMBOL,
    EXT_OBJ_UNBOX_INSTANCE_SYMBOL, EXT_OBJ_UNBOX_INT_SYMBOL, EXT_OBJ_UNBOX_STR_SYMBOL,
    EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL, ObjCollectionKind, entry_fn_name, is_module_entry_symbol,
};
#[cfg(test)]
mod tests;
pub use pycc_artifact_layout as artifact_layout;

const RELEASE_PASS_PIPELINE: &str = "default<O3>";
type CodegenObserver<'observer> =
    dyn for<'ctx> FnMut(&inkwell::module::Module<'ctx>, Option<&'static str>) + 'observer;

/// One MIR-level value during codegen. Extended (never replaced) by later
/// tasks: `Str` (Task 7) is a pointer to an opaque `pycc_rt::PyStrObj` --
/// `pycc_codegen` never inspects its layout (D-059's inline/heap
/// representation is entirely `pycc_rt`'s own concern), only ever passing
/// it through to a `pycc_rt_str_*` call. `Ty::None` uses the same LLVM `i8`
/// carrier shape as `Bool`, but always with the value zero: a distinct enum
/// variant would not add information because MIR's static `Ty` determines
/// whether that carrier means Python's unit value or a real boolean. Function
/// returns of `None` remain LLVM `void`.
///
/// `Copy` because every payload is an inkwell SSA handle (itself `Copy`) --
/// a `Scalar` names a value the module already holds, so duplicating the
/// handle duplicates nothing. #146 Part 2 (D-181) needs a site to both hand
/// a value onward and still classify the word it carried.
#[derive(Clone, Copy)]
enum Scalar<'ctx> {
    /// Tagged per D-061. Always LLVM `i64`.
    Int(IntValue<'ctx>),
    /// `0`/`1`, LLVM `i8` -- not `i1` (D-061's ABI note: this project has
    /// already hit real cross-platform storage/parameter footguns for
    /// sub-byte types, see D-027/D-028/D-029; `i1` is used only
    /// transiently for a `br` condition or an `icmp`/`fcmp` result,
    /// immediately zero-extended to `i8` before it's stored anywhere).
    Bool(IntValue<'ctx>),
    /// A plain, untagged LLVM `f64` -- unlike `int`, `float` needs no
    /// tagging scheme (D-061's tagged-fixnum representation is specific
    /// to `int`'s own overflow/bigint-promotion story); every `float`
    /// value is exactly one `f64`, always (Task 6).
    Float(FloatValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyStrObj` (D-059/D-060,
    /// Task 7) -- always refcounted, never inspected directly by this
    /// crate (see this enum's own doc comment).
    Str(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyIntListObj` (D-105,
    /// Task 10) -- like `Str`, opaque to this crate, which only ever
    /// stores it, passes it to a `pycc_rt_int_list_*` call, or marshals it
    /// across a function boundary.
    ///
    /// Its own variant rather than a reuse of `Str`'s (D-107): the two
    /// runtime objects have entirely different layouts, and `truthy`/
    /// `to_str` are exhaustive matches that would otherwise hand a
    /// `PyIntListObj` pointer straight to a `pycc_rt_str_*` function --
    /// reachable from ordinary type-checked source (`if xs:`, `print(xs)`)
    /// the moment list values become constructible. Keeping them distinct
    /// makes every operation `list[T]` has no v0.2 semantics for a compile
    /// error until it is answered deliberately, instead of silently
    /// misreading memory.
    ///
    /// Refcounting is deliberately *not* wired for this variant in v0.2
    /// (D-107): `pycc_rt_int_list_incref`/`_decref` are never called, so a
    /// list's backing allocation leaks for the process's lifetime. That is
    /// leak-only -- never a premature free or a double free -- because
    /// nothing frees a list value early either.
    List(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyDictObj` (PR-11 Task 2/5,
    /// D-121/D-123) -- like `List`, opaque to this crate, which only ever
    /// stores it, passes it to a `pycc_rt_dict_*` call, or marshals it
    /// across a function boundary.
    ///
    /// Its own variant rather than a reuse of `List`'s or `Str`'s (D-107's
    /// reasoning, extended to this new container by D-124): `PyDictObj` and
    /// `PyIntListObj` have entirely different layouts, and every exhaustive
    /// `Scalar` match (`truthy`/`to_str`/`to_numeric_encoded_int`/`to_float`/
    /// `emit_assign`/argument marshalling) would otherwise hand a
    /// `PyDictObj` pointer straight to a `pycc_rt_int_list_*` or
    /// `pycc_rt_str_*` function -- reachable from ordinary type-checked
    /// source (`if x:`, `print(x)`) the moment dict values become
    /// constructible (this task). Keeping it distinct makes every operation
    /// `dict[K, V]` has no v0.2 semantics for a compile error until it is
    /// answered deliberately, instead of silently misreading memory.
    ///
    /// Refcounting is deliberately *not* wired for this variant in v0.2
    /// (D-124, extending D-107's exact reasoning): `pycc_rt_dict_incref`/
    /// `_decref` are never called on a dict value itself, so a dict's
    /// backing allocation leaks for the process's lifetime, identically to
    /// `List`. That is leak-only -- never a premature free or a double free
    /// -- because nothing frees a dict value early either.
    Dict(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyIntSetObj` (PR-11 Task 6,
    /// D-121/D-124) -- like `List`/`Dict`, opaque to this crate, which only
    /// ever stores it, passes it to a `pycc_rt_int_set_*` call, or marshals
    /// it across a function boundary.
    ///
    /// Its own variant rather than a reuse of `List`'s, `Dict`'s, or
    /// `Str`'s (D-107's reasoning, extended to this new container by
    /// D-124, exactly as `Dict`'s own doc comment already extends it):
    /// `PyIntSetObj` has its own layout, distinct from `PyIntListObj`,
    /// `PyDictObj`, and `PyStrObj`, and every exhaustive `Scalar` match
    /// (`truthy`/`to_str`/`to_numeric_encoded_int`/`to_float`/`emit_assign`/argument
    /// marshalling) would otherwise hand a `PyIntSetObj` pointer straight to
    /// a `pycc_rt_int_list_*`, `pycc_rt_dict_*`, or `pycc_rt_str_*`
    /// function -- reachable from ordinary type-checked source (`if s:`,
    /// `print(s)`) the moment set values become constructible (PR-11 Task
    /// 9). Keeping it distinct makes every operation `set[int]` has no v0.2
    /// semantics for a compile error until it is answered deliberately,
    /// instead of silently misreading memory.
    ///
    /// Refcounting is deliberately *not* wired for this variant in v0.2
    /// (D-124, extending D-107's exact reasoning, identically to `List`/
    /// `Dict`): `pycc_rt_int_set_incref`/`_decref` are never called on a set
    /// value itself, so a set's backing allocation leaks for the process's
    /// lifetime. That is leak-only -- never a premature free or a double
    /// free -- because nothing frees a set value early either.
    Set(PointerValue<'ctx>),
    /// An LLVM `struct` held **by value** -- an SSA aggregate register, not
    /// a `PointerValue` to a heap or stack allocation, unlike every other
    /// container variant above (D-115). Each field is one of `Ty::Int`/
    /// `Ty::Bool`/`Ty::Float`'s own existing scalar representation (D-116
    /// admits no other element type in v0.2, so no field is ever itself a
    /// pointer or a refcounted value), and the arity is fixed at compile
    /// time -- which is exactly why no heap object, no `pycc_rt` type, and
    /// no refcounting policy accompanies this variant at all. There is
    /// nothing to leak (contrast `List`/`Dict`/`Set`, whose D-107/D-124
    /// leak-only policy exists because they *do* allocate).
    ///
    /// Its own variant rather than a reuse of any pointer-holding
    /// variant's shape (D-107/D-124's exact reasoning, extended): every
    /// exhaustive `Scalar` match (`truthy`/`to_str`/`to_numeric_encoded_int`/
    /// `to_float`) would otherwise have no way to reject a struct value
    /// passed where a pointer or a tagged int is expected. Unlike those
    /// four, `emit_assign`, `build_call_to`'s argument marshalling, and
    /// `MirStmt::Return` treat this exactly like every other variant's own
    /// pass-through case (D-116): moving a `StructValue` needs no
    /// container-specific logic, since `inkwell`'s `BasicValueEnum` and
    /// `BasicMetadataValueEnum` both already include a `StructValue` arm.
    ///
    /// Field values are stored exactly as the corresponding `Scalar`
    /// carries them. D-141 makes that identical to the current
    /// `PyIntListObj`/`PyDictObj`/`PyIntSetObj` value contract: an int field
    /// or container element is already an int-compatible encoded word,
    /// including a bool-identity marker. A tuple still crosses no runtime
    /// boundary and needs no ingress validation call.
    Tuple(inkwell::values::StructValue<'ctx>),
    /// `T | None` (PEP 604, D-197, #763, Part 1 of #747). An LLVM `struct`
    /// held **by value** -- an SSA aggregate register, exactly like `Tuple`
    /// immediately above and for the identical reason (D-115's reasoning,
    /// extended): `{ inner, i8 }` (payload, present-flag), the explicit
    /// present/absent tag `ty_to_basic_type`'s own `Ty::Optional` arm
    /// documents (not a niche sentinel -- see that arm's doc comment for
    /// why). Only `Optional[int]` is ever constructed by real,
    /// type-checked source in this PR (`pycc_types`' `T0049` gate rejects
    /// every other inner type pre-MIR-lowering, mirroring `list[int]`'s own
    /// `T0034` gate), but this variant's own shape does not assume that --
    /// matching `Tuple`'s own not-narrowed-to-what's-reachable-today
    /// precedent immediately above.
    ///
    /// Every exhaustive `Scalar` match this variant did not already need to
    /// answer for (`truthy`/`to_str`/`to_numeric_encoded_int`/`to_float`) is
    /// a defensive, provably-unreachable-by-real-source panic, for the same
    /// reason `Tuple`'s own arms in those functions are: `pycc_types`
    /// accepts neither printing, truthiness-testing, nor arithmetic on an
    /// `Optional`-typed value in this PR (only construction, `is`/`is not`,
    /// and narrowed-and-then-unwrapped use, which produces a *different*,
    /// non-`Optional` `Ty` once narrowed) -- see this PR's ADR.
    Optional(inkwell::values::StructValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyInstanceObj` (D-154, Part 1
    /// of #375) -- like `List`/`Dict`/`Set`, opaque to this crate, which
    /// only ever stores it, passes it to a `pycc_rt_instance_*` call, or
    /// marshals it across a function boundary. Never `GEP`'d into directly
    /// (the class-instance-layout ADR's own opaque-accessor decision):
    /// every attribute read/write goes through
    /// `pycc_rt_instance_get_slot`/`_set_slot` with a compile-time-resolved
    /// slot index, exactly like `List`'s own `pycc_rt_int_list_*` calls.
    ///
    /// Its own variant rather than a reuse of `List`'s/`Dict`'s/`Set`'s
    /// (D-107/D-124's exact reasoning, extended identically): `PyInstanceObj`
    /// has its own layout, and every exhaustive `Scalar` match (`truthy`/
    /// `to_str`/`to_numeric_encoded_int`/`to_float`/`emit_assign`/argument
    /// marshalling) would otherwise hand an instance pointer straight to a
    /// `pycc_rt_int_list_*`/`pycc_rt_dict_*`/`pycc_rt_str_*` function.
    ///
    /// Refcounting is deliberately not wired for this variant either (see
    /// `pycc_rt::instance`'s own doc comment): leak-only, identically to
    /// `List`/`Dict`/`Set`.
    Instance(PointerValue<'ctx>),
    /// A borrowed `PyObject *` owned by the hosting CPython interpreter
    /// (D-244, Part 2 of #1026) -- the runtime carrier for [`Ty::Object`],
    /// the opaque foreign-object type a `pycc build --ext` artifact obtains
    /// from `pycc_ext_obj_import` and, from this part on, from
    /// `pycc_ext_obj_getattr`. Entirely opaque to this crate: it is never
    /// `GEP`'d into, never inspected, and never handed to any `pycc_rt_*`
    /// function -- the only operations on it are the `pycc_ext_obj_*`
    /// shims in `src/ext/pycc_ext_module.c`, which are the sole place a
    /// CPython C-API call may appear.
    ///
    /// Its own variant rather than a reuse of `Instance`'s or any other
    /// pointer-holding variant's (D-107's rule, applied unchanged: a new
    /// pointer kind gets a new variant): a `PyObject *` obeys CPython's
    /// refcounting protocol and has no `pycc_rt` layout at all, so every
    /// exhaustive `Scalar` match (`truthy`/`to_str`/`to_numeric_encoded_int`/
    /// `to_float`/`coerce_scalar_to_type`/`emit_assign`/slot marshalling)
    /// would otherwise hand it straight to a `pycc_rt_str_*` or
    /// `pycc_rt_instance_*` function. Those arms were all refusals when
    /// this variant was added -- `pycc_types` refused every operation on a
    /// `Ty::Object` value except the attribute load Part 2 added -- and
    /// each later operation turned its own arm into a real one (the store
    /// in `emit_assign` is #1325's, for a module-level binding).
    ///
    /// Ownership: a producer such as `pycc_ext_obj_getattr` returns a new
    /// reference. An unbound temporary is released by its consumer
    /// (`object_release.rs`, #1092), and a module global owns the value
    /// bound to it and releases it on rebind (`object_slot.rs`, Part 1 of
    /// #1499). A value bound to a frame slot or an instance slot, passed to
    /// a user function, or returned is still never released (the later
    /// parts of #1499), so such a leak inside a loop is trip-count-linear;
    /// see `docs/RUNTIME.md`'s "Foreign imports in the module body"
    /// ownership subsection.
    Object(PointerValue<'ctx>),
    /// A pointer to the `PyccExtBufferView` -- `pycc_rt`'s `{ ptr, len }`
    /// pair -- that a `pycc build --ext` wrapper passed for a `memoryview`
    /// parameter (Part 1/Part 2 of #1027). Opaque to this crate, exactly as
    /// `Object` is: the only thing generated code may do with it is hand it
    /// to `pycc_rt_buffer_f64_get`. No GEP, no load of either field, no
    /// arithmetic -- the bounds check and the element read both live behind
    /// that one call.
    ///
    /// Its own variant rather than a reuse of `Object`'s or any container's
    /// (D-107/D-124's "a new pointer kind gets a new variant"): the pointee
    /// is a wrapper-owned C struct, not a `PyObject *` and not a `pycc_rt`
    /// heap object, so every exhaustive `Scalar` match would otherwise hand
    /// it to a `pycc_ext_obj_*` or `pycc_rt_*` function that would misread
    /// it. Keeping it distinct makes each such operation a deliberate
    /// compile error until it is answered.
    ///
    /// No refcounting, and nothing to leak: the buffer is acquired and
    /// released entirely wrapper-side, around the compiled call. Its
    /// lifetime strictly contains the value's.
    MemoryView(PointerValue<'ctx>),
}

struct UserFunction<'ctx> {
    param_tys: Vec<pycc_mir::Ty>,
    /// Issue #22: the global function-pointer slot for this function name.
    /// Initialized to null; set to the current definition's address when
    /// the `def` executes at module level. Calls load from this slot and
    /// dispatch indirectly, so a call before the `def` has executed sees
    /// null and aborts with `pycc_rt_name_error`, and a redefinition
    /// updates the slot so later calls see the new function.
    /// `None` for monomorphized generic specializations (`0gen_...` names),
    /// which are compiler-generated, not user-defined: they have no
    /// top-level `def` whose execution order matters, so they dispatch
    /// directly through `direct_value` instead.
    fn_ptr_global: Option<inkwell::values::GlobalValue<'ctx>>,
    /// The LLVM function type (parameter types + return type) for this
    /// function name. All definitions of the same name share one type
    /// (the type checker resolves to one signature per name). Needed to
    /// type the indirect call through `fn_ptr_global`.
    fn_type: inkwell::types::FunctionType<'ctx>,
    /// Issue #22: a global string constant holding the function name as a
    /// null-terminated C string, passed to `pycc_rt_name_error` on the
    /// null-pointer (call-before-`def`) path. Created once per function
    /// name in the declaration pass and reused at every call site, rather
    /// than recreating a duplicate-named global on each call.
    /// `None` for monomorphized specializations (no call-before-`def`
    /// path exists for compiler-generated functions).
    name_global: Option<inkwell::values::GlobalValue<'ctx>>,
    /// Issue #22: for monomorphized generic specializations (`0gen_...`
    /// names), the LLVM function value to call directly. `None` for
    /// ordinary user-defined functions, which dispatch indirectly through
    /// `fn_ptr_global` to preserve Python execution order.
    direct_value: Option<FunctionValue<'ctx>>,
}

/// Part 2b of #1142 (#1164): the `locals` key under which a
/// `memoryview`-returning frame records the buffer pointer a `return`
/// statement has made pending.
///
/// A reserved key rather than a threaded parameter: the two sites that need
/// it -- `MirStmt::Return` and [`free_buffer_slot_before_store`] -- already
/// both receive `locals`, and nothing in this crate ever walks that map by
/// entry, so a key no Python identifier can spell is invisible to every
/// other reader. The leading digit is the same "compiler-generated, not
/// user-spellable" convention `0gen_` monomorphization names already use.
///
/// Seeded only when the frame both returns a `memoryview` and owns at least
/// one buffer slot; every other frame's IR is unchanged by its existence.
const PENDING_RETURN_BUFFER_KEY: &str = "0pending_return_buffer";

/// Part 2b of #1142 (#1164): the companion flag to
/// [`PENDING_RETURN_BUFFER_KEY`], set when a rebinding of the pending
/// buffer's own name leaves [`PENDING_RETURN_BUFFER_KEY`] as the pointer's
/// *sole* owner.
///
/// Without it the epilogue could not tell "the pending pointer is still in
/// some local slot, which will release it" from "the pending pointer was
/// rebound away and nothing else names it". Freeing unconditionally would
/// double-free the first; never freeing leaks the second.
const PENDING_RETURN_ORPHANED_KEY: &str = "0pending_return_orphaned";

/// Part 2 of #1175 (#1179), review round 2: the three trailing
/// out-pointers a buffer-sub-range-returning frame was widened by,
/// `has_slice`/`start`/`stop` in [`crate::ext::ext_thunk_out_tys`]' own
/// order, seeded into the frame's `locals` at entry.
///
/// A reserved key rather than a threaded parameter for the same reason
/// [`PENDING_RETURN_BUFFER_KEY`] is one, and the rationale documented there
/// transfers verbatim: the sites that need them -- `MirStmt::Return` and
/// `MirStmt::ReturnBufferSlice` -- already both receive `locals`, `emit_stmt`
/// is recursive and also called from `exception.rs`, and no Python
/// identifier can spell a leading digit.
///
/// Presence *is* the "this frame was widened" predicate: every return in a
/// widened frame must describe itself, because the wrapper reads one pair of
/// out-slots for whichever return actually reached it and a frame can execute
/// more than one `return` (an abandoned one whose `finally` raised, then a
/// later one after an enclosing handler swallowed it).
const BUFFER_SLICE_HAS_SLICE_KEY: &str = "0buffer_slice_has_slice";

/// The `start` out-pointer companion to [`BUFFER_SLICE_HAS_SLICE_KEY`].
const BUFFER_SLICE_START_KEY: &str = "0buffer_slice_start";

/// The `stop` out-pointer companion to [`BUFFER_SLICE_HAS_SLICE_KEY`].
const BUFFER_SLICE_STOP_KEY: &str = "0buffer_slice_stop";

/// #382 (PR-22 Part 2): A pending `finally` target that `return`
/// statements inside a `try` body must route through before completing.
/// When a `MirStmt::Return` is emitted and the `finally_stack` is
/// non-empty, the return value is stored to `ret_slot` (if non-`None`),
/// the `is_returning` flag is set to `1`, and control branches to
/// `finally_bb` instead of emitting `ret` directly. After the finally
/// body runs, the codegen checks `is_returning`: if set, it loads the
/// return value and emits `ret` (or propagates to an enclosing finally).
#[derive(Clone)]
struct FinallyTarget<'ctx> {
    /// The finally block to branch to instead of emitting `ret`.
    finally_bb: inkwell::basic_block::BasicBlock<'ctx>,
    /// Alloca holding the return value (`None` for `Ty::None` / void
    /// functions, where `build_return(None)` is emitted instead).
    ret_slot: Option<PointerValue<'ctx>>,
    /// Alloca i8 flag: `1` = a `return` was intercepted, `0` = normal
    /// completion (fall-through to finally).
    is_returning: PointerValue<'ctx>,
}

#[derive(Clone)]
struct StorageSlot<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: pycc_mir::Ty,
    /// Module globals and non-parameter locals can have storage before
    /// execution reaches an assignment. Their separate flag prevents an
    /// unexecuted control-flow path from exposing an LLVM initializer, `undef`,
    /// or null as a fabricated Python value. Parameters are initialized at
    /// entry and therefore need no flag; #118 separately owns static
    /// definite-assignment diagnostics.
    initialized: Option<PointerValue<'ctx>>,
}

fn ty_to_basic_type(context: &Context, ty: pycc_mir::Ty) -> inkwell::types::BasicTypeEnum<'_> {
    match ty {
        pycc_mir::Ty::Int => context.i64_type().into(),
        pycc_mir::Ty::Bool => context.i8_type().into(),
        pycc_mir::Ty::Float => context.f64_type().into(),
        pycc_mir::Ty::Str => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // LLVM `void` cannot be a parameter type. v0.1 therefore carries
        // Python's singleton unit value across user-function parameter and
        // assignment-storage boundaries as the canonical `i8 0`; a `None`
        // return is still emitted as LLVM `void` by `compile_to_object`.
        pycc_mir::Ty::None => context.i8_type().into(),
        // `list[T]`'s runtime object (Task 11) is heap-allocated and always
        // referenced by pointer -- exactly the same storage/parameter
        // representation `Str` already gets above. The element type `T`
        // only affects what Task 11's runtime does with the pointee, never
        // this decision, so every `List(_)` is a pointer regardless of `T`
        // (D-105 restricts real *codegen* for non-`int` elements elsewhere,
        // not this representation choice).
        pycc_mir::Ty::List(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // `dict[K, V]`'s runtime object (PR-11 Task 5) is heap-allocated
        // and always referenced by pointer -- exactly the same
        // storage/parameter representation `List(_)` gets immediately
        // above, for the identical reason (the key/value types only affect
        // what this crate's `pycc_rt_dict_*` calls do with the pointee,
        // never this representation choice). Only `Ty::Dict(Box::new((Ty::
        // Str, Ty::Int)))` ever reaches this arm today (`pycc_types`'
        // T0036 gate rejects every other key/value combination before
        // codegen runs), but the arm itself is not narrowed to that one
        // combination, matching `List(_)`'s own element-type-agnostic
        // shape.
        pycc_mir::Ty::Dict(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // `set[T]`'s runtime object (PR-11 Task 9) is heap-allocated and
        // always referenced by pointer -- exactly the same
        // storage/parameter representation `List(_)`/`Dict(_)` get
        // immediately above, for the identical reason (the element type
        // only affects what this crate's `pycc_rt_int_set_*` calls do with
        // the pointee, never this representation choice). An `int` or,
        // since #1343, a user-class instance element reaches this arm
        // (`pycc_types`' T0038 gate rejects every other element type before
        // codegen runs); the arm is element-agnostic, matching
        // `List(_)`/`Dict(_)`'s own element/key/value-agnostic shape.
        pycc_mir::Ty::Set(_) | pycc_mir::Ty::FrozenSet(_) => {
            context.ptr_type(inkwell::AddressSpace::default()).into()
        }
        // A class instance's runtime object (`pycc_rt::instance::PyInstanceObj`,
        // D-154, Part 1 of #375) is heap-allocated and always referenced by
        // pointer -- exactly the same storage/parameter representation
        // `List(_)`/`Dict(_)`/`Set(_)` get above, for the identical reason
        // (the class's own declared shape only affects what this crate's
        // `pycc_rt_instance_*` calls do with the pointee, never this
        // representation choice).
        pycc_mir::Ty::Instance(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // `tuple[...]`'s v0.2 representation (D-115), and the one place
        // this function departs from every container arm above: a real
        // LLVM `struct` built positionally from each element's own
        // `ty_to_basic_type`, *not* a pointer. A tuple is the only
        // container this crate holds by value rather than by reference,
        // because it is the only one whose full shape is known at compile
        // time -- fixed arity, and (per D-116) every element a fixed-width
        // scalar -- so it needs no heap allocation and therefore no
        // runtime object to point at.
        //
        // Recursive rather than flat, matching `Ty::Tuple`'s own recursive
        // shape. D-116 admits only `int`/`bool`/`float` elements today, so
        // in practice every field resolves to `i64`/`i8`/`f64` -- but this
        // arm is deliberately not narrowed to those three, matching the
        // element-type-agnostic shape `List(_)`/`Dict(_)`/`Set(_)` already
        // use above.
        pycc_mir::Ty::Tuple(elems) => {
            let field_types: Vec<inkwell::types::BasicTypeEnum> = elems
                .iter()
                .map(|elem_ty| ty_to_basic_type(context, elem_ty.clone()))
                .collect();
            context.struct_type(&field_types, false).into()
        }
        // `T | None` (PEP 604, D-197, #763, Part 1 of #747). Only
        // `Optional[int]` reaches this arm from real, codegen-eligible
        // source today (`pycc_types`' `T0049` gate rejects every other
        // inner type before MIR-lowering, mirroring `list[int]`'s own
        // `T0034` gate) -- but this representation function is deliberately
        // element-type-agnostic here too, matching `Tuple`'s own recursive,
        // not-narrowed-to-what's-reachable-today shape immediately above.
        //
        // Representation: `{ inner, i8 }` (payload, present-flag) -- an
        // *explicit* present/absent tag, not a niche sentinel bit pattern.
        // A sentinel scheme was the original hypothesis (see this PR's ADR),
        // but `Ty::Int`'s own representation just above is a plain LLVM
        // `i64` occupying its full 64-bit range (this crate's separate
        // tagged smallint/bool-marker/bigint-pointer encoding in
        // `bigint_rc.rs` is a *different* representation, used only for
        // polymorphic container storage, not for a scalar `int`-typed local
        // or parameter) -- there is no bit pattern of a plain `i64` that is
        // guaranteed to never be a legitimate `int` value, so no sentinel is
        // safe. An explicit flag is therefore correct where `Optional[int]`
        // would otherwise be described as "niche-optimized."
        pycc_mir::Ty::Optional(inner) => {
            let payload_ty = ty_to_basic_type(context, (*inner).clone());
            context
                .struct_type(&[payload_ty, context.i8_type().into()], false)
                .into()
        }
        // An opaque CPython object (Part 1 of #1026): a `PyObject *`, the
        // same pointer representation `Str`/`List`/`Instance` already get
        // above. pycc knows nothing about the pointee -- every operation
        // on an `object`-typed value is refused by `pycc_types` (`I0404`)
        // -- so the representation only has to be wide enough to hold the
        // module handle `pycc_ext_obj_import` returns (a *new* reference
        // the artifact retains for its whole lifetime; `docs/RUNTIME.md`
        // owns that rule).
        pycc_mir::Ty::Object => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // A `memoryview` parameter at a `pycc build --ext` boundary (Part 1
        // of #1027, D-244): a pointer to the wrapper's own stack-resident
        // `PyccExtBufferView` -- the `{ void *ptr, long long len }` pair the
        // generated C fills from the acquired `Py_buffer` and keeps alive
        // for the whole call. Compiled code never dereferences CPython's
        // `Py_buffer` itself, so the pointee layout is pycc's own and the
        // representation is just the pointer `Str`/`Object` already get.
        pycc_mir::Ty::MemoryView => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // Deviation from the task brief: the brief's own version of this
        // catch-all's message read "(only int/float/bool/str/list[int] do)"
        // -- but that parenthetical is inaccurate twice over. This function
        // already gives `Ty::None` a representation too (the `i8 0` carrier
        // above), and the `List(_)`/`Dict(_)`/`Set(_)`/`Tuple(_)` arms
        // above aren't specific to `list[int]`/`dict[str, int]`/`set[int]`:
        // each produces the same representation for any element/key/value
        // type, not just those. Worded to match what this function actually
        // does -- `Ty::Infer` is the one variant left with no arm (see
        // `an_infer_typed_return_type_is_not_supported`).
        other => panic!(
            "pycc_codegen: {} has no LLVM representation yet (int/float/bool/str/None/list[_]/dict[_, _]/set[_]/tuple[...] do)",
            other.name()
        ),
    }
}

/// #380 (PR-20): Returns a zero/null default `BasicValueEnum` for the given
/// type, used only for abstract method bodies (which are never executed).
/// The value itself is irrelevant — it just needs to be well-typed so the
/// LLVM verifier accepts the function's `ret` instruction.
fn default_value_for_type<'ctx>(
    context: &'ctx Context,
    ty: pycc_mir::Ty,
) -> inkwell::values::BasicValueEnum<'ctx> {
    match ty {
        pycc_mir::Ty::Int => context.i64_type().const_zero().into(),
        pycc_mir::Ty::Bool => context.i8_type().const_zero().into(),
        pycc_mir::Ty::Float => context.f64_type().const_zero().into(),
        pycc_mir::Ty::Str
        | pycc_mir::Ty::List(_)
        | pycc_mir::Ty::Dict(_)
        | pycc_mir::Ty::Set(_)
        | pycc_mir::Ty::FrozenSet(_)
        | pycc_mir::Ty::Instance(_)
        | pycc_mir::Ty::Protocol(_)
        | pycc_mir::Ty::Object
        | pycc_mir::Ty::MemoryView => context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into(),
        pycc_mir::Ty::Tuple(elems) => {
            let field_types: Vec<inkwell::types::BasicTypeEnum> = elems
                .iter()
                .map(|elem_ty| ty_to_basic_type(context, elem_ty.clone()))
                .collect();
            let struct_ty = context.struct_type(&field_types, false);
            struct_ty.const_zero().into()
        }
        // `Optional[_]` (D-197, #763; widened to `float`/`bool` payloads by
        // #809): an absent (`present == 0`) `{ inner, i8 }` struct, matching
        // every other container arm above for the `present`/tag field --
        // but field 0 (the payload) is NOT unconditionally a raw zero word
        // the way `struct_ty.const_zero()` would give it. This function's
        // two call sites are not both the "never executed" case its
        // neighbours are: the abstract-method `Return(None)` site truly
        // never runs the returned value, but the *other* call site (this
        // function's exceptional-exit default, `emit_function`'s own `if
        // *return_ty == pycc_mir::Ty::None` sibling) IS reached by a real,
        // executing `int | None`-returning function that raises mid-body --
        // the same invariant `coerce_scalar_to_type`'s own
        // placeholder-building arm documents (an `Optional`'s payload field
        // must always be a *valid* payload for its inner type regardless of
        // the present flag, because `truthy`'s branch-free AND reads it
        // unconditionally for `Ty::Int`) applies here too. For `Ty::Int`
        // specifically, a raw `0` word would trip `classify_encoded_int`'s
        // fail-closed panic the moment any caller's own branch-free code
        // touches this exceptional return value's payload field before ever
        // consulting the exception flag, so that case keeps using
        // `tag_smallint_const` to encode `0` exactly as `to_encoded_int`
        // would. `Ty::Float`/`Ty::Bool` carry no analogous tagged encoding
        // (their `truthy` arms below branch on the `Ty::Optional` shape
        // explicitly rather than reading a D-141-encoded int), so a plain
        // per-type zero from `default_value_for_type` itself is a valid
        // placeholder for them.
        pycc_mir::Ty::Optional(ref inner) => {
            let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
            let payload = match inner.as_ref() {
                pycc_mir::Ty::Int => tag_smallint_const(context, 0).into(),
                other_inner => default_value_for_type(context, other_inner.clone()),
            };
            struct_ty
                .const_named_struct(&[payload, context.i8_type().const_zero().into()])
                .into()
        }
        // `Infer`, `Param`, and `None` never produce a real value at
        // runtime — `Infer` and `Param` are resolved away before MIR,
        // and `None` is represented as a zero i8.  Grouping them here
        // (instead of a `panic!` catch-all) avoids a permanently-
        // uncovered defensive region under D-014's 100 %-coverage gate.
        pycc_mir::Ty::None | pycc_mir::Ty::Infer | pycc_mir::Ty::Param(_) => {
            context.i8_type().const_zero().into()
        }
    }
}

/// Converts a numeric `Int`/`Bool` scalar to D-141's encoded-int carrier.
/// An existing `Int` passes through unchanged, whether it contains an odd
/// ordinary smallint, a bool-identity marker, or a bigint pointer. A
/// standalone `Bool` is being consumed numerically at these call sites, so
/// it becomes an ordinary D-061 smallint via a zero-extend and shift-and-or
/// matching `pycc_rt::tag_smallint`. Panics for `Float`, which is never
/// `int`-coercible -- `pycc_types`'
/// `numeric_result_type` always promotes an expression with any `float`
/// operand to `Ty::Float`, so no real MIR can reach this arm with a
/// `Float` operand (see this task's own defensive-panic test exercising
/// it via deliberately malformed MIR, matching this file's existing
/// convention for such arms).
fn to_numeric_encoded_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Int(v) => v,
        Scalar::Bool(v) => {
            let widened = builder
                .build_int_z_extend(v, context.i64_type(), "bool_to_i64")
                .expect("build_int_z_extend should not fail widening i8 to i64");
            let shifted = builder
                .build_left_shift(widened, context.i64_type().const_int(1, false), "tag_shl")
                .expect("build_left_shift should not fail for a constant shift amount");
            builder
                .build_or(shifted, context.i64_type().const_int(1, false), "tag_or")
                .expect("build_or should not fail for two i64 operands")
        }
        Scalar::Float(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got float")
        }
        Scalar::Str(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got str")
        }
        // Defensive, exactly like the two arms above -- not a feature gap
        // (D-107): `pycc_types`' `numeric_result_type` maps no `Ty::List`
        // to a numeric type, so any arithmetic with a list operand is
        // already rejected as `T0021` before codegen runs. Its own arm
        // rather than folding into `Str`'s, so the message names the type
        // it actually got.
        Scalar::List(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got list")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List` arm directly above, extended to `dict[K, V]` (D-107's
        // reasoning, per D-124): no arithmetic operand is ever `Ty::Dict`,
        // so this is never reached by real, type-checked source.
        Scalar::Dict(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got dict")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict` arms above, extended to `set[T]` (D-107's
        // reasoning, per D-124): no arithmetic operand is ever `Ty::Set`,
        // so this is never reached by real, type-checked source.
        Scalar::Set(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got set")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set` arms above, extended to `tuple[...]` (D-107's
        // reasoning, per D-116): no arithmetic operand is ever `Ty::Tuple`,
        // so this is never reached by real, type-checked source.
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got tuple")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set`/`Tuple` arms above, extended to a class
        // instance (D-154, Part 1 of #375, mirroring D-107/D-124's
        // reasoning): `pycc_types` rejects arithmetic on `Ty::Instance` as
        // `T0021` before codegen runs (see `numeric_result_type`'s own
        // `as_numeric` closure), so this is never reached by real,
        // type-checked source.
        Scalar::Instance(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got instance")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // arms above, extended to `Optional[int]` (D-197, #763, Part 1 of
        // #747, extending D-107/D-124's reasoning): `as_numeric` maps no
        // `Ty::Optional` to a numeric type either, so any arithmetic with
        // an `Optional[int]` operand is already rejected as `T0021` before
        // codegen runs -- unwrapping an `Optional[int]` to use its payload
        // numerically requires narrowing (Part 2+ of #747), which produces
        // a plain `Ty::Int` value, not this type.
        Scalar::Optional(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got optional")
        }
        // Defensive for the same reason as every arm above, extended to a
        // foreign CPython object (D-244, Part 2 of #1026): `pycc_types`
        // refuses arithmetic on a `Ty::Object` operand at the consumer
        // sites the Part 2 refusal migration added, so no type-checked
        // program reaches this arm.
        Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got object")
        }
        // Defensive for the same reason as every arm above, extended to a
        // `memoryview` (Part 2 of #1027): `numeric_result_type` maps no
        // `Ty::MemoryView` to a numeric type, and the bare name an
        // arithmetic operand would have to be is refused whichever
        // provenance it has -- `reject_memoryview_read` for a wrapper-
        // borrowed parameter, and since Part 2a of #1142 (#1165)
        // `pycc_types::buffer::owned_buffer_use_unsupported` for
        // artifact-owned storage. Its own arm rather than folding into
        // `Object`'s, so the message names the type it actually got.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got memoryview")
        }
    }
}

/// Converts an int-compatible scalar for storage at a statically-`int`
/// boundary while preserving whether the source object was `False` or
/// `True`. Ordinary integer inputs already use the D-061 encoding. The two
/// even marker words are disjoint from odd smallints and aligned bigint
/// pointers; `pycc_rt` owns their interpretation.
fn to_encoded_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Int(value) => value,
        Scalar::Bool(value) => {
            let widened = builder
                .build_int_z_extend(value, context.i64_type(), "bool_identity_i64")
                .expect("build_int_z_extend should not fail widening i8 to i64");
            let shifted = builder
                .build_left_shift(
                    widened,
                    context.i64_type().const_int(2, false),
                    "bool_identity_shl",
                )
                .expect("build_left_shift should not fail for a constant shift amount");
            builder
                .build_or(
                    shifted,
                    context.i64_type().const_int(2, false),
                    "bool_identity_or",
                )
                .expect("build_or should not fail for two i64 operands")
        }
        other => to_numeric_encoded_int(context, builder, other),
    }
}

/// Tags an already-known-in-range raw counter as an ordinary smallint.
/// D-141 leaves this only for user-visible lengths -- `range()` operands
/// stopped using it in #147 (D-179), where normalization moved into
/// `pycc_rt_range_normalize_operand` so a bigint operand survives it.
/// Container elements/values carry their encoded words unchanged and never
/// use this helper on read.
fn raw_i64_to_tagged_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    raw: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let shifted = builder
        .build_left_shift(raw, context.i64_type().const_int(1, false), "list_tag_shl")
        .expect("build_left_shift should not fail for a constant shift amount");
    builder
        .build_or(
            shifted,
            context.i64_type().const_int(1, false),
            "list_tag_or",
        )
        .expect("build_or should not fail for two i64 operands")
}

/// Extracts a `PyIntListObj` pointer from an already-evaluated operand that
/// every upstream check says must be a `list[T]`: `len`'s argument,
/// `MirExpr::Subscript`'s base, and `emit_list_name_read`'s named local.
/// `what` names the offending operand for the message.
///
/// One shared helper rather than a `let Scalar::List(..) = .. else` at each
/// of the three sites, for this file's established
/// no-permanently-uncoverable-region reason (see `emit_string_literal`'s own
/// doc comment): `Subscript`'s base in particular can only be reached with a
/// non-list `Scalar` through deliberately self-inconsistent MIR (a
/// `MirExpr` whose `ty()` says `list[T]` while `emit_expr` returns something
/// else), so an inline arm there would be a region D-014's gate could never
/// legitimately exercise. Funnelling all three through one helper makes the
/// check genuinely covered by the site that *is* naturally reachable -- a
/// non-list local named by `.append()`/`for`, and a non-list argument to
/// `len` (see the crate's `tests` module for both).
fn expect_list_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
    let Scalar::List(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: {what} did not evaluate to a list -- \
             pycc_types::check (T0033) should have rejected this before codegen"
        )
    };
    ptr
}

/// D-154 (Part 1 of #375): mirrors `expect_list_pointer` exactly, for
/// `MirExpr::AttrGet`/`MirStmt::AttrSet`'s own base -- `pycc_types::check`
/// (`T0043`) already rejects a non-instance `AttrGet`/`AttrSet`/`MethodCall`
/// base before codegen runs, so a mismatch here can only mean malformed MIR.
fn expect_instance_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
    let Scalar::Instance(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: {what} did not evaluate to a class instance -- \
             pycc_types::check (T0043) should have rejected this before codegen"
        )
    };
    ptr
}

/// Calls D-141's runtime-owned classifier/decoder. Container-value ingress
/// uses the call only to validate bigint exclusion and then stores the
/// original encoded word; index, slice, and `str`-repeat-count sites consume
/// its decoded raw result. `range` operands stopped calling it in #147
/// (D-179). Keeping classification in `pycc_rt` prevents codegen from
/// duplicating or partially interpreting the ABI.
fn build_untag_checked<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    tagged: IntValue<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.int_untag_checked, &[tagged.into()], name)
        .expect("build_call should not fail for a well-formed untag")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_untag_checked returns a non-void i64")
        .into_int_value()
}

/// Calls #1165's non-aborting length decoder
/// (`pycc_rt_buffer_alloc_untag_len`) for the `ndarray(n)` producer.
///
/// `build_untag_checked` above cannot serve this one site:
/// `pycc_rt_int_untag_checked` `panic!`s on a bigint or malformed word, and
/// that panic is caught at its own `extern "C"` boundary and turned into a
/// process abort -- which for a D-244 `ext` artifact aborts the *host*
/// CPython interpreter. This decoder raises `OverflowError` (D-173) and
/// returns the type-valid sentinel `0`, which the `guard_statement_effects`
/// the call site emits immediately afterwards branches away on, so the
/// sentinel never reaches `pycc_rt_buffer_f64_alloc`.
///
/// Every *other* `build_untag_checked` site keeps the aborting decoder on
/// purpose: they consume the decoded word with nothing guarding a sentinel,
/// so a silent `0` there would be a wrong answer rather than a loud stop.
fn build_buffer_alloc_untag_len<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    tagged: IntValue<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.buffer_alloc_untag_len, &[tagged.into()], name)
        .expect("build_call should not fail for a well-formed untag")
        .try_as_basic_value()
        .expect_basic("pycc_rt_buffer_alloc_untag_len returns a non-void i64")
        .into_int_value()
}

/// Reads one encoded element out of a `PyIntListObj`. The positional index
/// is a raw runtime counter; the returned word is already a user-visible
/// D-141 int-compatible value and is forwarded unchanged.
fn build_int_list_get<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list_ptr: PointerValue<'ctx>,
    raw_index: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(
            rt.int_list_get,
            &[list_ptr.into(), raw_index.into()],
            "list_get",
        )
        .expect("build_call should not fail for a well-formed list read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_list_get returns a non-void i64")
        .into_int_value()
}

/// Removes and returns the list's own last element (`list.pop()`, PR-12
/// Task 11, D-119). Mirrors `build_int_list_get`'s own one-`build_call`
/// shape exactly, minus the `index` parameter -- `.pop()` always targets
/// the last element, so there is nothing else to pass. The returned value
/// is already an encoded D-141 int-compatible word, exactly like
/// `build_int_list_get`'s own return value.
fn build_int_list_pop<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list_ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.int_list_pop, &[list_ptr.into()], "list_pop")
        .expect("build_call should not fail for a well-formed list pop")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_list_pop returns a non-void i64")
        .into_int_value()
}

/// A `PyIntListObj`'s current element count, as a raw `i64` counter. The
/// `len(x)` builtin tags this before handing it back as a
/// `Ty::Int` expression value; `MirStmt::ForList` uses it directly as its
/// own loop bound and deliberately does not.
fn build_int_list_len<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list_ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.int_list_len, &[list_ptr.into()], "list_len")
        .expect("build_call should not fail for a well-formed list length read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_list_len returns a non-void i64")
        .into_int_value()
}

/// Returns a **new** `PyIntListObj` holding the clamped, strided sub-range
/// `[start, stop)` of `list_ptr`'s elements, stepping by `step`
/// (`base[start:stop:step]`, PR-12 Task 9, D-118). All three bound operands
/// must already be raw, untagged `i64`s with every default/untag conversion
/// already applied by the caller (`MirExpr::Slice`'s own `emit_expr` arm
/// below) -- this helper is a thin one-`build_call` wrapper, exactly like
/// `build_int_list_get` above, not a place that itself interprets D-141's
/// encoded elements or D-118's defaulting rules.
fn build_int_list_slice<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list_ptr: PointerValue<'ctx>,
    start: IntValue<'ctx>,
    stop: IntValue<'ctx>,
    step: IntValue<'ctx>,
) -> PointerValue<'ctx> {
    builder
        .build_call(
            rt.int_list_slice,
            &[list_ptr.into(), start.into(), stop.into(), step.into()],
            "list_slice",
        )
        .expect("build_call should not fail for a well-formed list slice")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_list_slice returns a non-void pointer")
        .into_pointer_value()
}

/// Appends one already-validated encoded D-141 value to a `PyIntListObj`, shared by
/// `MirExpr::ListLiteral`'s per-element construction and
/// `MirExpr::ListAppend`. Returns nothing: `pycc_rt_int_list_append` is
/// declared `void`, so unlike every other `pycc_rt_int_list_*` helper above
/// there is no `try_as_basic_value()` result to extract.
fn build_int_list_append<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list_ptr: PointerValue<'ctx>,
    encoded_value: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.int_list_append,
            &[list_ptr.into(), encoded_value.into()],
            "list_append",
        )
        .expect("build_call should not fail for a well-formed list append");
}

/// Extracts a `PyDictObj` pointer from an already-evaluated operand that
/// every upstream check says must be a `dict[K, V]`: `len`'s argument,
/// `MirExpr::DictGet`'s `dict` operand, and `emit_dict_name_read`'s named
/// local. `what` names the offending operand for the message. Mirrors
/// `expect_list_pointer` exactly, for the identical reason (see that
/// function's own doc comment): one shared helper rather than a `let
/// Scalar::Dict(..) = .. else` at each site, so the check stays genuinely
/// covered by the site that is naturally reachable with a non-dict operand
/// (a non-dict local named by `d[k] = v`/`for`, and a non-dict argument to
/// `len`) rather than an unreachable one (`DictGet`'s own `dict` operand can
/// only be non-dict through deliberately self-inconsistent MIR, exactly like
/// `Subscript`'s base).
fn expect_dict_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
    let Scalar::Dict(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: {what} did not evaluate to a dict -- \
             pycc_types::check (T0033/T0035/T0036) should have rejected this before codegen"
        )
    };
    ptr
}

/// Inserts or updates one already-validated encoded D-141 value under `key`
/// in a `PyDictObj`, shared by `MirExpr::DictLiteral`'s per-pair
/// construction and `MirStmt::DictSet`'s own `d[k] = v` (D-123's
/// insert-or-update operation -- `pycc_rt_dict_set` itself decides which of
/// the two this is, by whether `key` already compares equal to a stored
/// key). Returns nothing, exactly like `build_int_list_append` above:
/// `pycc_rt_dict_set` is declared `void`.
fn build_dict_set<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    dict_ptr: PointerValue<'ctx>,
    key_ptr: PointerValue<'ctx>,
    encoded_value: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.dict_set,
            &[dict_ptr.into(), key_ptr.into(), encoded_value.into()],
            "dict_set",
        )
        .expect("build_call should not fail for a well-formed dict set");
}

/// Returns the encoded value stored for `key_ptr`, or `encoded_default` if absent
/// (`dict.get(key, default)`, PR-12 Task 11, D-119). Mirrors
/// `build_int_list_pop`'s own one-`build_call` shape exactly, with one
/// extra `i64` operand for the default. Both the default and returned value
/// are D-141 int-compatible encoded words and retain bool identity.
fn build_dict_get_or_default<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    dict_ptr: PointerValue<'ctx>,
    key_ptr: PointerValue<'ctx>,
    encoded_default: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(
            rt.dict_get_or_default,
            &[dict_ptr.into(), key_ptr.into(), encoded_default.into()],
            "dict_get_or_default",
        )
        .expect("build_call should not fail for a well-formed dict get-or-default")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_get_or_default returns a non-void i64")
        .into_int_value()
}

/// A `PyDictObj`'s current entry count, as a raw `i64` counter, shared by
/// the `len(d)` builtin's `Ty::Dict` branch and
/// `MirStmt::ForDict`'s own loop bound -- mirrors `build_int_list_len`
/// exactly, for the identical reason.
fn build_dict_len<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    dict_ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.dict_len, &[dict_ptr.into()], "dict_len")
        .expect("build_call should not fail for a well-formed dict length read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_len returns a non-void i64")
        .into_int_value()
}

/// Applies the representation-changing assignment conversions accepted by
/// this type system: standalone `i8` bool to D-141's identity-preserving
/// int-compatible `i64`, and (D-197, #763, Part 1 of #747) a bare
/// `inner`-typed value or `None` widening into an `Optional[inner]` slot's
/// `{ inner, i8 }` present/absent struct. All other assignable pairs already
/// share a representation.
///
/// This is the *only* place `Optional[inner]`'s representation is built --
/// driven entirely by the target slot's own declared type, not by anything
/// MIR-lowering-time. That single-site design (rather than an `IntBoundary`-
/// style MIR wrapper node) is deliberate: it uniformly covers both the
/// first, `AnnAssign`-introduced binding (`x: int | None = 5`) and every
/// later plain reassignment to the same name (`x = None`), which
/// `pycc_mir::stmt::lower_stmt`'s `Assign` arm never wraps -- see that
/// arm's own doc comment on why a wrapper node is unnecessary.
fn coerce_scalar_to_type<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
    target_ty: pycc_mir::Ty,
) -> Scalar<'ctx> {
    match (target_ty, scalar) {
        (pycc_mir::Ty::Int, Scalar::Bool(value)) => {
            Scalar::Int(to_encoded_int(context, builder, Scalar::Bool(value)))
        }
        // A `Scalar::Optional` arriving here is either a *real*
        // `Optional[inner]` value (e.g. `w: int | None = y` where `y` is
        // itself `Optional[int]`, read back out via `MirExpr::Name`'s own
        // `Ty::Optional` arm), or `MirExpr::NoneLiteral`'s own `{ i8, i8 }`
        // all-zero placeholder standing in for the bare `None` literal
        // (`emit_expr_unchecked`'s `MirExpr::NoneLiteral` arm has no target
        // `inner` type to build a real struct against). Distinguished by
        // LLVM struct type, not by any Rust-level tag: an anonymous struct
        // type is uniqued per-`Context` by its field types, so the real
        // `{ i64, i8 }` (or whatever `inner` actually is) and the
        // placeholder's fixed `{ i8, i8 }` compare unequal whenever they
        // are not the exact same shape, and equal (and therefore pass
        // through unchanged) whenever the source already has the target's
        // exact representation.
        //
        // #809 risk, verified: for `inner == Ty::Bool`, `ty_to_basic_type`'s
        // `Ty::Optional` arm produces the real `Optional[bool]` shape `{
        // i8, i8 }` -- structurally, and therefore (LLVM struct types being
        // uniqued per-field-type-list within a `Context`) *literally the
        // same `StructType` value as* the `NoneLiteral` placeholder's own
        // fixed `{ i8, i8 }`. So this arm's `v.get_type() == struct_ty`
        // check cannot distinguish "this is genuinely the untyped None
        // placeholder" from "this is a real, already-correctly-shaped
        // `Optional[bool]` value" when `inner == Ty::Bool` -- both take the
        // `v.get_type() == struct_ty` branch and pass `v` through as-is.
        // This is harmless rather than a bug: taking that branch for the
        // placeholder returns `{ payload: 0, present: 0 }` unchanged, which
        // is *exactly* the correct representation for an absent
        // `Optional[bool]` too -- `Ty::Bool`'s payload carries no tagged
        // encoding analogous to `Ty::Int`'s D-141 word (see `truthy`'s and
        // `MirExpr::OptionalUnwrap`'s own `Ty::Bool` handling below/in
        // `lib.rs`, both of which read the payload only as a plain `0`/`1`
        // and never as anything requiring a specific "valid" bit pattern
        // the way `classify_encoded_int` does for `Ty::Int`), so an
        // arbitrary (here, zero) payload byte is always a safe stand-in
        // when `present == 0`. Every consumer of an `Optional[bool]`
        // payload (`truthy`'s `Scalar::Optional` arm, `OptionalUnwrap`'s
        // codegen arm) is itself only ever reached with `present == 1`
        // guarding real use of the payload's *value* (narrowing proves
        // presence before an unwrap; `truthy` ANDs the payload's
        // truthiness with `present` rather than trusting it alone) -- so no
        // observer can tell the difference between "the None placeholder,
        // never coerced" and "a real, coerced, present `Optional[bool]`"
        // for the one case (`present == 0`) where the collision could ever
        // matter. `optional_bool_none_placeholder_and_real_absent_value_are_the_same_llvm_struct_type`
        // and
        // `optional_bool_absent_value_truthiness_and_narrowed_unwrap_are_both_correct`
        // (`pycc_codegen::tests`) pin both halves of this finding: the
        // literal type-identity collision, and that it produces correct
        // end-to-end behavior for `x: bool | None = None` through both
        // `truthy` and an `if x is not None:` narrowed unwrap.
        (pycc_mir::Ty::Optional(inner), Scalar::Optional(v)) => {
            let struct_ty =
                ty_to_basic_type(context, pycc_mir::Ty::Optional(inner.clone())).into_struct_type();
            if v.get_type() == struct_ty {
                Scalar::Optional(v)
            } else {
                // The placeholder's field 0 must be a value of field 0's
                // *actual* declared type (`struct_ty`'s first field, sized
                // by `inner`), not unconditionally a D-141-encoded int:
                // `struct_ty.get_undef()` is `inner`-shaped (`{ i64, i8 }`
                // for `Ty::Int`, `{ f64, i8 }` for `Ty::Float`, `{ i8, i8 }`
                // for `Ty::Bool`), and `build_insert_value` performs no
                // type-matching validation of its own -- inserting a
                // mismatched-type constant (e.g. an `i64` into a `Ty::Float`
                // struct's `f64` field 0) builds a malformed
                // `ConstantStruct` that both `module.verify()` and LLVM's
                // own IR printer accept without complaint (neither
                // re-validates a `ConstantAggregate`'s element types against
                // its declared struct type), so this was previously a
                // latent miscompile rather than a build-time or
                // verification failure. For `Ty::Int`,
                // `tag_smallint_const(context, 0)` encodes `0` the same way
                // `to_encoded_int` would (`truthy`'s own `Scalar::Optional`
                // arm unconditionally calls `pycc_rt_int_truthy` on field 0
                // and ANDs the result with the present flag rather than
                // branching around it, so an absent-but-invalid payload
                // would trip `classify_encoded_int`'s fail-closed panic even
                // though the AND makes the payload's truth value
                // irrelevant). `Ty::Float` and `Ty::Bool` carry no such
                // tagged encoding, so a plain zero of the field's own type
                // is exact.
                let payload0: inkwell::values::BasicValueEnum = match inner.as_ref() {
                    pycc_mir::Ty::Int => tag_smallint_const(context, 0).into(),
                    pycc_mir::Ty::Float => context.f64_type().const_zero().into(),
                    pycc_mir::Ty::Bool => context.i8_type().const_zero().into(),
                    // `T0049` (`crates/pycc_hir/src/func.rs`) rejects every
                    // `Optional[T]` annotation for `T` outside `{int, float,
                    // bool}` before this value could ever be constructed.
                    other => panic!(
                        "pycc_codegen: internal error: an Optional[_] placeholder targeted an unsupported inner type ({other:?}) -- pycc_types::check (T0049) should have rejected this before codegen"
                    ),
                };
                let with_payload = builder
                    .build_insert_value(struct_ty.get_undef(), payload0, 0, "opt_none_payload")
                    .expect(
                        "build_insert_value should not fail inserting field 0 of a fresh struct",
                    )
                    .into_struct_value();
                let with_flag = builder
                    .build_insert_value(
                        with_payload,
                        context.i8_type().const_zero(),
                        1,
                        "opt_none_flag",
                    )
                    .expect(
                        "build_insert_value should not fail inserting field 1 of a fresh struct",
                    )
                    .into_struct_value();
                Scalar::Optional(with_flag)
            }
        }
        // A bare `inner`-typed (or `inner`-assignable, e.g. `bool` under
        // `Optional[int]`) value widening into an `Optional[inner]` slot:
        // recurse to apply any further representation change the payload
        // itself needs (e.g. `bool -> int`, the arm above), then wrap the
        // result as the struct's present (`i8 = 1`) payload field.
        (pycc_mir::Ty::Optional(inner), bare) => {
            let coerced = coerce_scalar_to_type(context, builder, bare, (*inner).clone());
            // #809 (Part 3 of #747): `T0049`'s widened gate
            // (`crates/pycc_hir/src/func.rs`) now also admits
            // `Optional[float]`/`Optional[bool]`, so this match is keyed on
            // *both* the declared inner type and the coerced `Scalar`
            // variant, not the `Scalar` variant alone -- matching on the
            // variant alone would accept e.g. a stray `Scalar::Float`
            // widening into an `Optional[int]` slot (that combination
            // falls through `coerce_scalar_to_type`'s own recursive call
            // unchanged, since there is no `float -> int` widening arm),
            // silently building a struct whose field 0 both has the wrong
            // LLVM type for `Ty::Optional(Int)`'s `{ i64, i8 }` shape and
            // is never a value any real `Optional[int]`-typed source could
            // produce (`pycc_types` never assigns a bare `float` to an
            // `int`-annotated slot). Pairing the target inner type with the
            // coerced variant keeps each of the three real widenings exact
            // and every mismatched combination falling to the panic below.
            let payload: inkwell::values::BasicValueEnum = match (inner.as_ref(), coerced) {
                (pycc_mir::Ty::Int, Scalar::Int(v)) => v.into(),
                // Each passes through as-is: `ty_to_basic_type`'s own
                // `Ty::Optional` arm already sizes field 0 from the inner
                // type, so no further representation change is needed here.
                (pycc_mir::Ty::Float, Scalar::Float(v)) => v.into(),
                (pycc_mir::Ty::Bool, Scalar::Bool(v)) => v.into(),
                // `T0049` (`crates/pycc_hir/src/func.rs`) rejects every
                // `Optional[T]` annotation for `T` outside `{int, float,
                // bool}` before this value could ever be constructed, and
                // `pycc_types::check`'s own assignability rule only ever
                // widens a bare value into the *matching* inner type (or
                // `bool -> int`, handled by the recursive call above), so
                // an inner/coerced-variant mismatch reaching here means one
                // of those upstream gates accepted something it should
                // have rejected.
                _ => panic!(
                    "pycc_codegen: internal error: an Optional[int|float|bool] assignment's payload did not evaluate to int, float, or bool -- pycc_types::check (T0049) should have rejected this before codegen"
                ),
            };
            let struct_ty =
                ty_to_basic_type(context, pycc_mir::Ty::Optional(inner)).into_struct_type();
            let with_payload = builder
                .build_insert_value(struct_ty.get_undef(), payload, 0, "opt_payload")
                .expect("build_insert_value should not fail inserting field 0 of a fresh struct")
                .into_struct_value();
            let with_flag = builder
                .build_insert_value(
                    with_payload,
                    context.i8_type().const_int(1, false),
                    1,
                    "opt_present",
                )
                .expect("build_insert_value should not fail inserting field 1 of a fresh struct")
                .into_struct_value();
            Scalar::Optional(with_flag)
        }
        (_, scalar) => scalar,
    }
}

/// Prepares one `range()` operand for the induction phi: the result is an
/// *encoded* int-compatible word (D-061/D-141), normalized so the
/// bool-identity markers become the ordinary smallints `0`/`1` while a
/// smallint or a heap bigint passes through unchanged.
///
/// Renamed from `range_operand_to_tagged_int` in #147 (D-179): the old name
/// described the old mechanism, which decoded to a raw `i64` and re-tagged,
/// and therefore could not represent a bigint operand at all.
fn range_operand_to_normalized_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
    position: &str,
) -> IntValue<'ctx> {
    match scalar {
        scalar @ (Scalar::Int(_) | Scalar::Bool(_)) => {
            let encoded = to_encoded_int(context, builder, scalar);
            // #147 (D-179): normalize instead of decode-and-re-tag. The old
            // `build_untag_checked` + `raw_i64_to_tagged_int` pair enforced
            // D-141's bool normalization by round-tripping through a raw
            // `i64`, which made every bigint operand abort. The runtime
            // normalizer keeps that bool contract and lets a bigint through.
            builder
                .build_call(rt.range_normalize_operand, &[encoded.into()], "range_norm")
                .expect("build_call should not fail for a declared runtime function")
                .try_as_basic_value()
                .expect_basic("pycc_rt_range_normalize_operand returns a non-void i64")
                .into_int_value()
        }
        // `List`/`Dict`/`Set`/`Tuple` join this arm's existing or-pattern
        // rather than getting their own (D-107, extended to dict/set by
        // D-124 and to tuple by D-116): unlike `to_numeric_encoded_int`/`to_float`
        // above and below, this message never names the offending type, so
        // it stays exactly as honest for a list, dict, set, or tuple
        // operand as for a `float` or `str` one -- and folding adds no
        // separate, permanently-unexecutable region under this crate's
        // 100%-region gate (D-014). `range()` arguments are type-checked by
        // `pycc_types` before codegen, so this whole arm is defensive
        // either way.
        Scalar::Float(_)
        | Scalar::Str(_)
        | Scalar::List(_)
        | Scalar::Dict(_)
        | Scalar::Set(_)
        | Scalar::Tuple(_)
        // D-154 (Part 1 of #375): a class instance joins this same
        // or-pattern for the identical reason `List`/`Dict`/`Set`/`Tuple`
        // already do -- no new instrumented region.
        | Scalar::Instance(_)
        // D-197, #763, Part 1 of #747: `Optional[int]` joins this same
        // or-pattern for the identical `numeric_result_type`/`as_numeric`
        // reason -- `range()` operands are type-checked as plain numeric
        // types before codegen, and an `Optional[int]` is never one.
        | Scalar::Optional(_)
        // D-244, Part 2 of #1026: a foreign CPython object joins this same
        // or-pattern for the identical reason -- `range()` operands are
        // type-checked as plain numeric types before codegen, and a
        // `Ty::Object` never is. Folded in rather than given its own arm
        // so it adds no separate, permanently-unexecutable region.
        // Part 2 of #1027: a `memoryview` joins this same or-pattern for
        // the identical reason -- `range()` operands are type-checked as
        // plain numeric types before codegen, and a `Ty::MemoryView` never
        // is, under either provenance (Part 2a of #1142, #1165).
        | Scalar::MemoryView(_)
        | Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: range() {position} did not evaluate to int")
        }
    }
}

/// Evaluates a `range()`-shaped triple of bounds (`MirStmt::ForRange` and
/// its three comprehension-tail copies all share this exact preheader
/// shape) with #638 (D-208) exception-edge protection: `start`'s word is
/// protected across `stop` and `step`'s own evaluation, and `stop`'s word is
/// protected across `step`'s, mirroring the two-operand push/pop pattern
/// `BinOp`/`Compare` use above but generalized to three operands evaluated
/// in sequence. Per D-179, these bounds are legitimately owned, live
/// `IntValue`s held across each other's evaluation (unlike `Slice`/
/// `Subscript`/container-literal bounds, which abort the process on a
/// bigint before a sibling is ever reached) -- see the #638 decision
/// entry's affected-site inventory for the full boundary.
///
/// Returns the three normalized `IntValue`s; callers keep their own
/// existing retain/release and `owned_range_operands` bookkeeping unchanged
/// -- this helper only factors out the shared evaluation-with-protection
/// step, not the ownership contract built on top of it.
#[allow(clippy::too_many_arguments)]
fn emit_range_operands_with_exception_safety<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    start: &MirExpr,
    stop: &MirExpr,
    step: &MirExpr,
) -> (IntValue<'ctx>, IntValue<'ctx>, IntValue<'ctx>) {
    let start_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, start),
        "start",
    );
    let pending_start = push_pending_int_release_if_temporary(rt, start, start_v);
    let stop_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, stop),
        "stop",
    );
    let pending_stop = push_pending_int_release_if_temporary(rt, stop, stop_v);
    let step_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, step),
        "step",
    );
    // LIFO pop order, matching the LIFO push order above.
    pop_pending_int_release(rt, pending_stop);
    pop_pending_int_release(rt, pending_start);
    (start_v, stop_v, step_v)
}

/// Promotes any numeric `Scalar` to `f64`: an existing `Float` passes
/// through; `Int` goes through `pycc_rt_int_to_float` (never a raw LLVM
/// cast -- the value is D-141 encoded, so only `pycc_rt` may interpret its
/// bits); `Bool` uses a plain unsigned-int-to-float conversion
/// (unambiguous for a 0/1 value, no tagging involved).
fn to_float<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> FloatValue<'ctx> {
    match scalar {
        Scalar::Float(v) => v,
        Scalar::Int(v) => builder
            .build_call(rt.int_to_float, &[v.into()], "int_to_float")
            .expect("build_call should not fail for a well-formed conversion")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_to_float returns a non-void f64")
            .into_float_value(),
        Scalar::Bool(v) => builder
            .build_unsigned_int_to_float(v, context.f64_type(), "bool_to_float")
            .expect("build_unsigned_int_to_float should not fail for an i8 0/1 value"),
        Scalar::Str(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got str")
        }
        // Defensive for the same `numeric_result_type` reason as
        // `to_numeric_encoded_int`'s own `List` arm above (D-107), and separate from
        // `Str`'s for the same message-honesty reason.
        Scalar::List(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got list")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List` arm directly above, extended to `dict[K, V]` (D-107's
        // reasoning, per D-124).
        Scalar::Dict(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got dict")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict` arms above, extended to `set[T]` (D-107's
        // reasoning, per D-124).
        Scalar::Set(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got set")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set` arms above, extended to `tuple[...]` (D-107's
        // reasoning, per D-116).
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got tuple")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set`/`Tuple` arms above, extended to a class
        // instance (D-154, Part 1 of #375).
        Scalar::Instance(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got instance")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // arms above, extended to `Optional[int]` (D-197, #763, Part 1 of
        // #747).
        Scalar::Optional(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got optional")
        }
        // Defensive for the same reason as every arm above, extended to a
        // foreign CPython object (D-244, Part 2 of #1026): `pycc_types`
        // refuses arithmetic on a `Ty::Object` operand.
        Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got object")
        }
        // Defensive for the same reason as every arm above, extended to a
        // `memoryview` (Part 2 of #1027). A buffer *element* is a `float`
        // and reaches this function freely; the buffer itself never does,
        // whether it is a wrapper-borrowed parameter or the artifact-owned
        // storage Part 2a of #1142 (#1165) added.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got memoryview")
        }
    }
}

/// Shared by the `BinOp` arm's `FloorDiv`/`Mod`/`Pow` cases under a
/// `Ty::Float` result: each of these three (unlike `Add`/`Sub`/`Mul`/`Div`,
/// which map directly to an LLVM float instruction) needs a `pycc_rt_float_*`
/// runtime call instead -- this is the one piece of call-building logic all
/// three share, parameterized only by which `RtFns` field to call.
fn build_float_rt_binop<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt_fn: FunctionValue<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
) -> FloatValue<'ctx> {
    builder
        .build_call(rt_fn, &[l.into(), r.into()], "float_binop")
        .expect("build_call should not fail for a well-formed float binop")
        .try_as_basic_value()
        .expect_basic("pycc_rt_float_* functions all return a non-void f64")
        .into_float_value()
}

/// Converts any scalar to a fresh, owned `str` object matching CPython's
/// `str(x)` for that value (Task 8) -- reused unchanged by Task 10's
/// `print`. `str` itself passes through (already a `str`, and per this
/// function's own contract already an owned reference by the time it gets
/// here -- see `emit_expr`'s `FString` arm, the only caller); every other
/// type goes through its own `pycc_rt_*_to_str` conversion.
///
/// Two deviations from the task brief, both here:
///
/// 1. The brief's own version of this function took a `context: &'ctx
///    Context` parameter (matching `to_numeric_encoded_int`/`to_float`'s own
///    shape) -- but no branch below actually needs it: unlike `to_float`'s
///    `Bool` case (which builds a real `context`-dependent conversion
///    instruction), every branch here only ever picks which already-built
///    `RtFns` field and value to call `build_call` with, and `build_call`
///    itself needs no `Context` at all. An unused parameter would fail
///    this crate's `-D warnings` (`unused_variables` applies to
///    parameters, not just local bindings). Dropped rather than
///    underscore-prefixed, since it's not merely unused in *this* body but
///    structurally unneeded by this function's own job -- matching this
///    file's established convention of removing what's provably
///    unnecessary rather than working around it (see e.g. `emit_expr`'s
///    own `module`-parameter history note).
/// 2. The brief's own version of this function's final `build_call` ended
///    `.try_as_basic_value().left().expect(msg)` -- but (per the doc
///    comment on `emit_expr`'s `BinOp` arm) this inkwell version's
///    `try_as_basic_value()` returns its own `ValueKind` enum, not
///    `either::Either`, so `.left()` doesn't exist on it and the brief's
///    code as written doesn't compile. Fixed to `.expect_basic(msg)`, the
///    same fix already applied throughout this file (e.g.
///    `build_float_rt_binop` above).
fn to_str<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let (rt_fn, arg): (FunctionValue<'ctx>, inkwell::values::BasicMetadataValueEnum) = match scalar
    {
        Scalar::Str(v) => return v,
        Scalar::Int(v) => (rt.int_to_str, v.into()),
        Scalar::Float(v) => (rt.float_to_str, v.into()),
        Scalar::Bool(v) => (rt.bool_to_str, v.into()),
        // A real, reachable feature gap rather than a defensive arm
        // (D-107): `pycc_types` accepts any argument type for `print`, so
        // `print(xs)` for a `list[int]` local type-checks today and lands
        // here -- and so does f-string interpolation (`f"{xs}"`, the
        // interpolation arm in `emit_expr`), a second, independent reachable
        // route into this same arm (`emit_eval_print_arg` and that interpolation
        // arm both call into this one shared `to_str` helper). v0.2 has no
        // `str(list)`/list-printing semantics (D-105), and there is no
        // `pycc_rt_list_to_str` to call -- so this panics honestly instead
        // of handing a `PyIntListObj` pointer to a `pycc_rt_*_to_str`
        // function that would read it as a `PyStrObj`.
        Scalar::List(_) => {
            panic!("pycc_codegen: string conversion of a list[T] value is not supported yet")
        }
        // A real, reachable feature gap, identical in kind to the `List`
        // arm directly above: `pycc_types` places no type restriction on
        // `print`'s argument or an f-string interpolation other than the
        // instance/protocol gate (#977, D-237), so `print(x)`/
        // `f"{x}"` for a `dict[str, int]` local type-checks today and
        // lands here. v0.2 has no `str(dict)`/dict-printing semantics
        // (D-123), and there is no `pycc_rt_dict_to_str` to call -- so
        // this panics honestly instead of handing a `PyDictObj` pointer to
        // a `pycc_rt_*_to_str` function that would read it as a
        // `PyStrObj`.
        Scalar::Dict(_) => {
            panic!("pycc_codegen: string conversion of a dict[K, V] value is not supported yet")
        }
        // No longer reachable from source: since Part 1 of #1319,
        // `pycc_types` refuses `print(s)`/`f"{s}"` for a `set[int]` or
        // `frozenset[int]` value as `C0001` before codegen. v0.2 has no
        // `str(set)`/set-printing semantics (D-124), and there is no
        // `pycc_rt_int_set_to_str` to call -- so this stays an honest
        // panic backstop instead of handing a `PyIntSetObj` pointer to a
        // `pycc_rt_*_to_str` function that would read it as a `PyStrObj`.
        Scalar::Set(_) => {
            panic!("pycc_codegen: string conversion of a set[T] value is not supported yet")
        }
        // A real, reachable feature gap -- but NOT "identical in kind" to
        // the `List`/`Dict`/`Set` arms directly above in one respect:
        // `list[T]`'s own `print(xs)`/`f"{xs}"` reachability predates this
        // whole PR-11 effort entirely (established back in PR-10, D-107);
        // `dict`/`set`'s own reachability, while more recent (PR-11a's own
        // HIR literal lowering), was already in place before this PR
        // (PR-11b) started -- neither is something this PR's own diff
        // turned from a clean diagnostic into a panic. `tuple[...]`'s
        // reachability here IS exactly that: it is new as of this PR's own
        // Task 2 (`HirExpr::TupleLiteral` lowering, `crates/pycc_hir/src/
        // lib.rs`) -- before that commit, any program containing a tuple
        // literal failed to lower at all and got a clean `C0001`
        // ("expression kind not supported yet") diagnostic instead of ever
        // reaching this function. `pycc_types` places no type restriction
        // on `print`'s argument or an f-string interpolation other than the
        // instance/protocol gate (#977, D-237), so
        // `print(t)`/`f"{t}"` for a `tuple[...]` local type-checks today
        // and lands here. D-116 ships only construction and literal-index
        // reads, so v0.2 has no tuple string-conversion semantics -- and
        // unlike the three container arms above there is not even a
        // runtime object to hand to a conversion function, since a tuple is
        // a bare LLVM struct with no `pycc_rt` type at all (D-115). Panics
        // honestly instead of reinterpreting the struct's first field as a
        // `PyStrObj` pointer. See the D-116 deferred-capability list in
        // `docs/decisions/D-116-tuple-v0-2-scope-int-bool-float-elements-only.md`
        // and `docs/ROADMAP.md`'s matching follow-up for this
        // new-as-of-PR-11b reachability.
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: string conversion of a tuple[...] value is not supported yet")
        }
        // #378 (PR-18): a class instance with a `__repr__` method is
        // converted to a string at the MIR level -- `rewrite_instance_to_repr`
        // in `pycc_mir` rewrites instance-typed f-string interpolations and
        // `print` arguments to `__repr__` calls, so the codegen's `to_str`
        // receives a `str` scalar from the `__repr__` call, never an
        // `Instance` scalar. Since #977 (D-237) `pycc_types` rejects every
        // `print` argument and f-string interpolation that neither that
        // rewrite nor `rewrite_exception_to_message` renders with `C0001`,
        // so no type-checked program reaches this arm; it stays as
        // defence-in-depth for a hand-built MIR (see `tests.rs`) and
        // panics honestly, matching the pre-#378 behavior.
        Scalar::Instance(_) => {
            panic!(
                "pycc_codegen: string conversion of a class instance without `__repr__` is not supported yet"
            )
        }
        // A real, reachable feature gap, identical in kind to the `List`/
        // `Dict`/`Set` arms above (D-197, #763, Part 1 of #747):
        // `pycc_types` places no type restriction on `print`'s argument or
        // an f-string interpolation other than the instance/protocol gate
        // (#977, D-237), so `print(x)`/`f"{x}"` for an `Optional[int]`
        // local type-checks today and lands here. This PR
        // ships no `str(Optional[int])`/`Optional`-printing semantics (CPython's
        // own `str(None)` is `"None"` and `str(5)` is `"5"`, which this
        // representation *could* support, but doing so is out of this PR's
        // scope -- see this PR's ADR), and there is no
        // `pycc_rt_optional_int_to_str` to call -- so this panics honestly
        // instead of reinterpreting the struct's first field as an `i64` or
        // `PyStrObj` pointer.
        Scalar::Optional(_) => {
            panic!("pycc_codegen: string conversion of an Optional[int] value is not supported yet")
        }
        // Defensive rather than a reachable feature gap, unlike the
        // container arms above: since #1340 both callers convert a CPython
        // object *before* reaching here -- `print` through the shim's
        // `PyObject_Str` helper in its write phase and an f-string
        // interpolation through its `PyObject_Format` helper
        // (`string_render.rs`) -- because rendering a foreign object needs a
        // `pycc_ext_obj_*` shim call and its failure edge, neither of which
        // this plain runtime-call helper can emit.
        Scalar::Object(_) => {
            panic!(
                "pycc_codegen: internal error: string conversion of a CPython object value must \
                 go through the shim -- string_render should have converted it first"
            )
        }
        // Defensive, unlike the `List`/`Dict` arms above and like
        // `Object`'s: `print(b)` and `f"{b}"` are both bare reads of a
        // `memoryview`-bound name, refused with `C0001` before codegen runs
        // by `reject_memoryview_read` for a wrapper-borrowed parameter and
        // by `owned_buffer_use_unsupported` for the artifact-owned storage
        // Part 2a of #1142 (#1165) added. Part 2 of #1027 admits `b[i]`,
        // #1116 `len(b)` and Part 1 of #1142 `b[i] = v`; nothing else.
        Scalar::MemoryView(_) => {
            panic!(
                "pycc_codegen: internal error: string conversion of a memoryview value is not \
                 supported yet -- pycc_types should have refused this before codegen"
            )
        }
    };
    builder
        .build_call(rt_fn, &[arg], "to_str")
        .expect("build_call should not fail for a well-formed conversion")
        .try_as_basic_value()
        .expect_basic("every pycc_rt_*_to_str function returns a non-void pointer")
        .into_pointer_value()
}

/// Builds a `str` object from a compile-time literal's bytes (Task 7),
/// embedding them as a private constant global and calling
/// `pycc_rt_str_from_literal` -- never null-terminated, since a Python
/// `str` can contain an embedded `\0`, so the byte length is always passed
/// explicitly rather than relying on termination.
///
/// Extracted (Task 8) out of `emit_expr`'s own `MirExpr::StringLiteral` arm
/// below so `MirExpr::FString`'s `Literal`-part case can build the exact
/// same `str` object directly, as a `PointerValue`, without going through
/// `emit_expr`'s general `&MirExpr -> Scalar` dispatch just to immediately
/// pattern-match the `Scalar::Str` back out again. That defensive
/// pattern-match (the task brief's own version used a `let Scalar::Str(ptr)
/// = emit_expr(..., &MirExpr::StringLiteral(s.clone())) else { unreachable!
/// (...) }`) is *provably* dead code given this function's own contract --
/// `MirExpr::StringLiteral` always evaluates to `Scalar::Str`, unconditionally,
/// nothing else ever calls this helper with a different result type -- and
/// `cargo llvm-cov`'s region coverage confirmed it empirically: that
/// `unreachable!()` arm's region never executed under any test, an
/// uncovered region under this project's 100%-line-and-region coverage gate
/// (D-014). Removed by sharing this typed-`PointerValue`-returning helper
/// instead, the same "no redundant impossible-to-cover branch" convention
/// this file already applies elsewhere (see `emit_expr`'s `Name` arm's own
/// doc comment).
fn emit_string_literal<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    s: &str,
) -> PointerValue<'ctx> {
    let bytes = s.as_bytes();
    let array_ty = context.i8_type().array_type(bytes.len() as u32);
    let global = module.add_global(array_ty, None, "str_lit");
    global.set_initializer(&context.const_string(bytes, false));
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    let ptr = global.as_pointer_value();
    let len = context.i64_type().const_int(bytes.len() as u64, false);
    builder
        .build_call(
            rt.str_from_literal,
            &[ptr.into(), len.into()],
            "str_lit_obj",
        )
        .expect("build_call should not fail for a well-formed string literal construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_str_from_literal returns a non-void pointer")
        .into_pointer_value()
}

fn emit_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let value = emit_expr_unchecked(context, builder, module, rt, user_functions, locals, expr);
    // Recursive calls come back through this wrapper. Consequently a node
    // that can set the pending state stops evaluation before its parent can
    // evaluate the next operand, call argument, or other sub-expression.
    // Pure literals, reads, comparisons, and ordinary arithmetic skip the
    // TLS call and branch entirely; D-173's original unconditional guard at
    // every node made tight numeric loops several times slower.
    if expression_can_set_exception(expr) {
        guard_statement_effects(context, builder, rt);
    }
    value
}

#[allow(clippy::too_many_arguments)]
fn emit_expr_unchecked<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    // Tasks 3-6 only ever passed this to `emit_expr`'s own recursive calls
    // (clippy's `only_used_in_recursion` lint, part of `-D warnings`,
    // required an underscore-prefixed `_module` name for that shape). Task
    // 7's `MirExpr::StringLiteral` arm below is the first to read it
    // directly (`module.add_global`, to embed a literal's bytes as a
    // constant), so it's no longer only-used-in-recursion -- dropping the
    // underscore.
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    use pycc_mir::Ty;
    match expr {
        MirExpr::IntLiteral(n) => Scalar::Int(emit_int_constant(context, builder, rt, *n)),
        // #1346: a receiver evaluated for its effects, then the attribute.
        MirExpr::Sequence { discard, value } => sequence::emit_sequence(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            discard,
            value,
        ),
        // #1254 (D-250): a comprehension in any expression position.
        MirExpr::Comprehension(comp) => emit_comprehension_expr(
            &CompCx {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            },
            comp,
        ),
        MirExpr::FloatLiteral(f) => Scalar::Float(context.f64_type().const_float(*f)),
        MirExpr::IntBoundary(value) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            Scalar::Int(to_encoded_int(context, builder, scalar))
        }
        // Part 2 of #1387: a native value boxed into an `object` binding,
        // rebinding or attribute slot -- see `object_box`.
        MirExpr::ObjectBox(value) => Scalar::Object(object_box::emit_boxed(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            value,
        )),
        // The bare literal `None` (D-197, #763, Part 1 of #747), standing
        // alone rather than already known to be flowing into a
        // predeclared `Optional[inner]` slot -- that context-sensitive
        // wrapping happens at the assignment site
        // (`coerce_scalar_to_type`, driven by the target slot's own
        // declared type), not here, so this arm has no target `inner` type
        // to build a real `{ inner, i8 }` struct against. A minimal `{ i8,
        // i8 }` all-zero placeholder is emitted instead: `pycc_types`
        // accepts a bare `None` expression only as an `Optional[_]`
        // initializer or an `is`/`is not` operand in this PR (never
        // printed, compared for equality, or used arithmetically), and
        // both of those consumers rebuild the correctly-typed struct
        // themselves (`coerce_scalar_to_type` for assignment,
        // `MirExpr::Compare`'s `Is`/`IsNot` codegen below reads the
        // struct's `i8` flag field only, which is at the same offset
        // regardless of the payload's own width) -- see this PR's ADR.
        MirExpr::NoneLiteral => {
            let placeholder_ty = context.struct_type(&[context.i8_type().into(); 2], false);
            Scalar::Optional(placeholder_ty.const_zero())
        }
        // CPython's `NotImplemented` singleton (#1418), a borrowed pointer
        // from the fixed shim -- only an `ext` module's admitted
        // `return NotImplemented` builds this node.
        MirExpr::NotImplemented => {
            let ptr = context.ptr_type(inkwell::AddressSpace::default());
            let accessor = foreign_pack::shim_fn(
                module,
                EXT_OBJ_NOT_IMPLEMENTED_SYMBOL,
                ptr.fn_type(&[], false),
            );
            Scalar::Object(
                builder
                    .build_call(accessor, &[], "not_implemented")
                    .expect("build_call should not fail for pycc_ext_obj_not_implemented")
                    .try_as_basic_value()
                    .expect_basic("pycc_ext_obj_not_implemented returns PyObject *")
                    .into_pointer_value(),
            )
        }
        // `OptionalWrap` (D-197, #763, Part 1 of #747) exists purely to fix
        // `.ty()` for `collect_stmt_bindings`'s slot-type derivation (see
        // its own doc comment in `pycc_mir`) -- the actual `{ inner, i8 }`
        // struct-building work is `coerce_scalar_to_type`'s, called here
        // with the wrapper's own declared `Ty::Optional(inner)` as the
        // target so it applies uniformly whether the wrapped value is a
        // bare `inner`-typed payload or `MirExpr::NoneLiteral`'s
        // placeholder.
        MirExpr::OptionalWrap(value, inner) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            // #770 review: `value`'s classification must be checked here,
            // against the raw `Scalar` it just produced, *before*
            // `coerce_scalar_to_type` below wraps it into a
            // `Scalar::Optional` struct. `MirStmt::Assign`'s own
            // `retain_if_int_duplicate` call runs on the *outer*
            // `OptionalWrap` expression and its already-wrapped
            // `Scalar::Optional` result, which can never retain anything
            // (`retain_if_int_duplicate`'s `if let Scalar::Int(word) =
            // scalar` guard never matches a struct) and
            // `int_value_is_a_duplicate_reference` classifies
            // `OptionalWrap` itself as owning for exactly that reason --
            // by design, it never looks inside the wrapper. So a borrowed
            // `int` payload (e.g. `x: int | None = n` for a heap-bigint
            // `n`) reached this arm with no retain anywhere in the
            // pipeline before this fix, letting `x` hold a second,
            // unretained reference to `n`'s bigint: `n`'s later
            // reassignment/death still fires `release_int_slot_before_store`
            // unconditionally, freeing the object out from under `x`'s
            // stored payload despite `x` still being live. Reusing the
            // same classification here (rather than duplicating it) keeps
            // both the direct-assignment and the `OptionalWrap` paths
            // agreeing on which shapes are borrowed.
            let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
            coerce_scalar_to_type(
                context,
                builder,
                scalar,
                pycc_mir::Ty::Optional(inner.clone()),
            )
        }
        // Issue #769 (Part 2 of #747): the read-side counterpart of
        // `OptionalWrap` immediately above. `pycc_types::check` (via its
        // own `narrow` overlay) has already proven this particular read is
        // reachable only when the value is present, so `value` below
        // always evaluates to a real `Scalar::Optional { payload, present }`
        // struct (never the `MirExpr::NoneLiteral` all-zero placeholder --
        // that placeholder is only ever a `coerce_scalar_to_type` target,
        // never something a narrowing test could recognize as
        // `Optional`). `T0049` (`crates/pycc_hir/src/func.rs`) restricts
        // every `Optional[T]` annotation to `T = int`, so `inner` here is
        // always `Ty::Int` and field 0 is always an already-D-141-encoded
        // int word -- extracting it and returning `Scalar::Int` needs no
        // decode/re-encode step, exactly like `Ty::Optional(_)`'s own
        // `MirExpr::Name` read arm above returns the struct as-is with no
        // transformation of its own fields.
        //
        // Refcount reasoning (bigint case, `n: int | None = <heap bigint>`
        // then `if n is not None: use(n)`): this arm performs a plain
        // *read* of `n`'s slot (via the inner `Name` -> `Ty::Optional`
        // load above), extracting the payload word without incrementing
        // any refcount -- exactly mirroring a bare (non-`Optional`) `int`
        // local's own `MirExpr::Name` read arm, which likewise loads and
        // returns `Scalar::Int` with no retain. Both are borrowed reads:
        // ownership transfer (and therefore the retain that must accompany
        // it) happens only where a value is *stored* into a new owning
        // slot -- `MirStmt::Assign`'s `retain_if_int_duplicate` call, or
        // (for the wrap direction) `OptionalWrap`'s own arm just above.
        // `use(n)` here passing the unwrapped `Scalar::Int` to a function
        // call follows the same already-correct borrowed-argument
        // convention every other bare `int` argument uses; nothing about
        // narrowing changes that convention, so no extra retain belongs in
        // this arm specifically. See `pycc_codegen::bigint_rc`'s
        // `is_owning_producer`/`int_value_is_a_duplicate_reference`
        // classification of this node (mirroring `MirExpr::Name`, not
        // `OptionalWrap`) for the corresponding compile-time classification.
        // #1476 (Part 3 of #1387): an `object` read an `isinstance` guard
        // narrowed -- see `object_unbox`.
        MirExpr::ObjectUnbox(object, inner) => {
            let object = emit_expr(context, builder, module, rt, user_functions, locals, object);
            object_unbox::emit_unboxed(context, builder, module, rt, object, inner)
        }
        MirExpr::OptionalUnwrap(value, inner) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            let Scalar::Optional(v) = scalar else {
                panic!(
                    "pycc_codegen: internal error: OptionalUnwrap's operand did not evaluate to Scalar::Optional -- pycc_mir should only ever wrap a Ty::Optional-scoped Name read"
                );
            };
            let payload = builder
                .build_extract_value(v, 0, "narrowed_payload")
                .expect(
                    "build_extract_value should not fail extracting field 0 of an Optional struct",
                );
            // #809 (Part 3 of #747): `T0049`'s widened gate now admits
            // `Ty::Float`/`Ty::Bool` inner types alongside the pre-existing
            // `Ty::Int`, so the narrowed payload must become the matching
            // `Scalar` variant instead of always `Scalar::Int` -- a plain
            // `f64`/`i8` payload handed to an `int`-typed consumer as
            // `Scalar::Int` would misinterpret it as a D-141-encoded word.
            match inner.as_ref() {
                pycc_mir::Ty::Float => Scalar::Float(payload.into_float_value()),
                pycc_mir::Ty::Bool => Scalar::Bool(payload.into_int_value()),
                // `Ty::Int`, the pre-existing shape -- and every other
                // `Ty` is unreachable here: `T0049`
                // (`crates/pycc_hir/src/func.rs`) restricts every
                // `Ty::Optional` inner type to `{int, float, bool}` before
                // an `OptionalUnwrap` node can ever be constructed.
                _ => Scalar::Int(payload.into_int_value()),
            }
        }
        MirExpr::StringLiteral(s) => {
            Scalar::Str(emit_string_literal(context, builder, module, rt, s))
        }
        // D-136: `math.pi` is a compile-time float constant, not a runtime
        // value bound to any local slot -- Task 4's own thin slice emits
        // the literal immediate directly, matching the ADR's explicit "no
        // runtime call at all" design for this one symbol. `std::f64::consts::PI`
        // is Rust's own IEEE-754 double-precision constant for pi, bit-for-bit
        // the same value CPython's `math.pi` uses (both are the nearest
        // representable `f64`/C `double` to the true mathematical constant).
        MirExpr::Name {
            name,
            ty: Ty::Float,
        } if name == "math.pi" => {
            Scalar::Float(context.f64_type().const_float(std::f64::consts::PI))
        }
        // PEP 634-636 (#381, PR-21): `None` singleton in a match pattern
        // comparison. `None` is not a bound variable — it is a constant
        // `i8 0` carrier (D-075), emitted inline exactly like `math.pi`
        // above rather than looked up in `locals`.
        MirExpr::Name { name, ty: Ty::None } if name == "None" => {
            Scalar::Bool(context.i8_type().const_zero())
        }
        MirExpr::Name { name, ty } => {
            let slot = locals.get(name).unwrap_or_else(|| {
                panic!("pycc_codegen: internal error: `{name}` has no local slot")
            });
            debug_assert_eq!(
                &slot.ty, ty,
                "pycc_codegen: internal error: local type drifted"
            );
            if let Some(initialized_ptr) = slot.initialized {
                let initialized = builder
                    .build_load(context.i8_type(), initialized_ptr, "global_initialized")
                    .expect("build_load should not fail for a declared global flag")
                    .into_int_value();
                let is_initialized = builder
                    .build_int_compare(
                        IntPredicate::NE,
                        initialized,
                        context.i8_type().const_zero(),
                        "global_is_initialized",
                    )
                    .expect("build_int_compare should not fail for two i8 values");
                let function = builder
                    .get_insert_block()
                    .expect("name reads are always emitted inside a basic block")
                    .get_parent()
                    .expect("the current basic block always belongs to a function");
                let ready = context.append_basic_block(function, "global_ready");
                let unbound = context.append_basic_block(function, "global_unbound");
                builder
                    .build_conditional_branch(is_initialized, ready, unbound)
                    .expect("build_conditional_branch should not fail for an i1 condition");
                builder.position_at_end(unbound);
                // #1316 / Part 1 of #1333: an `object` global read in a
                // function body before its import or assignment ran raises
                // a catchable `NameError`, as CPython does; every other
                // unbound read keeps the trap.
                if !foreign_fail::emit_unbound_object_read(context, builder, module, rt, ty, name) {
                    builder
                        .build_call(rt.trap, &[], "unbound_global")
                        .expect("build_call should not fail for llvm.trap");
                    builder
                        .build_unreachable()
                        .expect("build_unreachable should not fail in a fresh block");
                }
                builder.position_at_end(ready);
            }
            // Deviation from the task brief: the brief's version matched
            // `ty` twice -- once to pick `load_ty` (a `BasicTypeEnum`) for
            // `build_load`, once more to wrap the loaded value in the
            // right `Scalar` variant -- with the second match's `Ty::Int`/
            // `Ty::Bool` arms mirroring the first's and a trailing
            // `_ => unreachable!("handled above")`. That second match's
            // catch-all is provably dead code, not merely hard to test:
            // both matches inspect the exact same `ty` value, so any input
            // that isn't `Ty::Int`/`Ty::Bool` already panics in the *first*
            // match (this arm's own "not supported yet" case, still
            // present just below) -- the second match's `_` arm can never
            // be reached by any input, well-formed or not. Merged into one
            // match (matching `ty` once) so there's no redundant
            // impossible-to-cover branch left over.
            match ty {
                Ty::Int => {
                    let loaded = builder
                        .build_load(context.i64_type(), slot.ptr, "load")
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Int(loaded.into_int_value())
                }
                Ty::Bool => {
                    let loaded = builder
                        .build_load(context.i8_type(), slot.ptr, "load")
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Bool(loaded.into_int_value())
                }
                Ty::Float => {
                    let loaded = builder
                        .build_load(context.f64_type(), slot.ptr, "load")
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Float(loaded.into_float_value())
                }
                Ty::Str => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Str(loaded.into_pointer_value())
                }
                Ty::None => {
                    let loaded = builder
                        .build_load(context.i8_type(), slot.ptr, "load_none")
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    // The static `Ty::None` retains the semantic distinction
                    // from a bool even though both use an LLVM `i8` carrier.
                    Scalar::Bool(loaded.into_int_value())
                }
                // Same pointer-slot read as `Ty::Str` immediately above --
                // `ty_to_basic_type`'s own `List(_)` arm already allocated
                // this slot as a pointer, so reading it back is identical
                // regardless of the element type.
                //
                // Task 5 (D-089) originally carried the loaded pointer in
                // `Scalar::Str`, safe only for as long as no `MirExpr`
                // could construct a `list[T]` value, and flagged as a
                // Task-11 tripwire for `truthy`/`to_str` specifically.
                // Task 11a (D-107) retired that reuse: the pointer is now
                // `Scalar::List`, so every exhaustive `Scalar` match had to
                // answer for `list[T]` explicitly instead of silently
                // treating it as a `PyStrObj`.
                //
                // `str_value_is_a_duplicate_reference` stays gated on
                // `ty: Ty::Str` (Task 5's own fix for the same reuse). That
                // gate is now redundant with the variant split for this
                // arm, but it is still the correct contract for that
                // function -- and `incref_if_str_duplicate` needs no
                // `List` arm at all, since its `if let Scalar::Str(..)
                // else { scalar }` shape already passes a list through
                // untouched, which is exactly D-107's leak-only policy.
                Ty::List(_) => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::List(loaded.into_pointer_value())
                }
                // Same pointer-slot read as `Ty::List(_)` immediately
                // above (PR-11 Task 5) -- `ty_to_basic_type`'s own
                // `Dict(_)` arm already allocated this slot as a pointer,
                // so reading it back is identical regardless of the
                // key/value types. Every real read of a `dict[str, int]`
                // local -- `len(x)`, `x[k]`, `x[k] = v`'s own dict operand,
                // `for k in x:` -- goes through this arm (directly, or via
                // `emit_dict_name_read`'s synthetic `MirExpr::Name`), so
                // without it every one of those would fall through to the
                // catch-all below and panic on a real, type-checked
                // program instead of reading the value.
                Ty::Dict(_) => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Dict(loaded.into_pointer_value())
                }
                // Same pointer-slot read as `Ty::List(_)`/`Ty::Dict(_)`
                // immediately above (PR-11 Task 9) -- `ty_to_basic_type`'s
                // own `Set(_)` arm already allocated this slot as a
                // pointer, so reading it back is identical regardless of
                // the element type. Every real read of a `set[int]`
                // local -- `len(x)`, `for v in x:` -- goes through this
                // arm (directly, or via `emit_set_name_read`'s synthetic
                // `MirExpr::Name`), so without it every one of those would
                // fall through to the catch-all below and panic on a real,
                // type-checked program instead of reading the value.
                Ty::Set(_) | Ty::FrozenSet(_) => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Set(loaded.into_pointer_value())
                }
                // NOT a pointer read, unlike the four container arms above
                // (PR-11b Task 5, D-115): a tuple slot holds the struct
                // itself, so this loads the whole aggregate back out as an
                // SSA value. The load type must be computed from `ty`
                // rather than hardcoded like every other arm here, since a
                // tuple's LLVM type depends on its own element types.
                //
                // Not optional, and not merely a nicety: `t[0]` -- the only
                // tuple operation D-116 ships besides construction -- lowers
                // to a `MirExpr::Subscript` whose base is exactly this
                // `MirExpr::Name`. Without this arm every literal-index read
                // of a tuple *variable* (as opposed to an inline
                // `(1, 2)[0]`) would fall through to the catch-all below and
                // panic on a real, type-checked program, which is why the
                // module-level and function-local smoke programs for this
                // task both exercise it.
                Ty::Tuple(_) => {
                    let loaded = builder
                        .build_load(
                            ty_to_basic_type(context, ty.clone()).into_struct_type(),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Tuple(loaded.into_struct_value())
                }
                // Same pointer-slot read as `Ty::List(_)`/`Ty::Dict(_)`/
                // `Ty::Set(_)` above (D-154, Part 1 of #375) --
                // `ty_to_basic_type`'s own `Instance(_)` arm already
                // allocated this slot as a pointer, so reading it back is
                // identical regardless of the class. Every real read of a
                // class-instance local -- `p.x`, `p.bump()`, passing `p` as
                // an argument -- goes through this arm, so without it every
                // one of those would fall through to the catch-all below
                // and panic on a real, type-checked program instead of
                // reading the value.
                Ty::Instance(_) => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Instance(loaded.into_pointer_value())
                }
                // NOT a pointer read, identical in kind to `Ty::Tuple(_)`
                // above (D-197, #763, Part 1 of #747): an `Optional[int]`
                // slot holds the `{ int, i8 }` struct itself, so this loads
                // the whole aggregate back out as an SSA value. Every real
                // read of an `Optional[int]` local -- `x is None`,
                // `if x:`, passing `x` as an argument, `return x` -- lowers
                // to exactly this `MirExpr::Name`, so without it every one
                // of those would fall through to the catch-all below and
                // panic on a real, type-checked program instead of reading
                // the value.
                Ty::Optional(_) => {
                    let loaded = builder
                        .build_load(
                            ty_to_basic_type(context, ty.clone()).into_struct_type(),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Optional(loaded.into_struct_value())
                }
                // Part 2 of #1026 (#1081). The same pointer-slot read as
                // `Ty::Instance(_)` above, for the global a foreign
                // `import numpy` binds. The plan for this PR expected the
                // catch-all below to stay the only `Ty::Object` answer here
                // -- `pycc_types` refused binding a CPython object to a
                // name, so no *local* could carry one -- but a foreign
                // import's module global is a storage slot like any other
                // (and since #1325 so is a module-level `x = <object>`),
                // and `MirExpr::ObjAttrGet`'s base -- like
                // `MirExpr::ObjMethodCall`'s, added by PR 2b -- is a plain
                // `MirExpr::Name` read of it. Without this arm the one
                // shape PR 2a actually compiles (`numpy.pi`) panics here
                // before reaching `foreign_attr::emit`.
                //
                // No reference-count traffic accompanies the read: the
                // global owns the reference the import created
                // (`docs/RUNTIME.md`), so a read is a borrow with nothing to
                // balance. A `x = <object>` global owns its reference the
                // same way (#1325). Since Part 1 of #1499 a rebind or
                // re-import releases the previous value
                // (`object_slot.rs`).
                Ty::Object => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::Object(loaded.into_pointer_value())
                }
                // Part 2 of #1027: the same pointer-slot read again, for a
                // `pycc build --ext` export's `memoryview` parameter. Its
                // slot holds the `PyccExtBufferView *` the generated wrapper
                // passed; `ty_to_basic_type` already maps `Ty::MemoryView`
                // to a plain pointer, so only this load was missing.
                //
                // The one expression that reaches it is a `BufferGet` base,
                // which is the whole of the admitted read surface -- every
                // other read of the name is `reject_memoryview_read`'s
                // `C0001`. No refcount traffic: the buffer is borrowed and
                // wrapper-released, so the read owns nothing.
                Ty::MemoryView => {
                    let loaded = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            slot.ptr,
                            "load",
                        )
                        .expect(
                            "build_load should not fail for a slot this function itself allocated",
                        );
                    Scalar::MemoryView(loaded.into_pointer_value())
                }
                other => {
                    panic!(
                        "pycc_codegen: reading a `{}`-typed local is not supported yet",
                        other.name()
                    )
                }
            }
        }
        MirExpr::BinOp {
            op,
            left,
            right,
            ty,
        } => {
            // This inkwell version's `try_as_basic_value()` returns its own
            // `ValueKind` enum (not `either::Either` as in older inkwell
            // releases the task brief's original code was written against
            // -- ".left()" doesn't exist on this type); `.expect_basic(msg)`
            // is the direct equivalent, panicking with `msg` if the callee
            // turned out to be void instead of returning a value.
            let l = emit_expr(context, builder, module, rt, user_functions, locals, left);
            // #638 (D-208): `l` may already be an owned, not-yet-released
            // `Ty::Int` birth reference (e.g. `x + x` promoting to a fresh
            // heap word). Protect it across `right`'s evaluation, which may
            // recurse into an exception-settable node and branch away
            // before this arm's own `release_if_int_temporary(left, l)`
            // call below is ever reached. The pop on the fallthrough path
            // immediately precedes that unchanged release call, so at most
            // one of {exception-edge release, fallthrough release} ever
            // executes for this word.
            let pending_l = push_pending_int_release_if_scalar_temporary(rt, left, &l);
            let r = emit_expr(context, builder, module, rt, user_functions, locals, right);
            pop_pending_int_release(rt, pending_l);
            match ty {
                Ty::Int => {
                    // `to_numeric_encoded_int` promotes a `bool` operand instead of
                    // rejecting it -- Python's `bool` is an `int` subtype
                    // (e.g. `True + 1 == 2`), a case `pycc_types` already
                    // legitimately types `Ty::Int` (see its own
                    // `a_binop_treats_bool_as_int` test); this is normal
                    // Python arithmetic-promotion semantics being handled,
                    // not an internal invariant violation (this codegen's
                    // own earlier, Task 3-era version mislabeled the
                    // now-removed rejection of exactly this case as an
                    // "internal error" -- see this task's own
                    // `adding_a_bool_left_operand_to_an_int_promotes_
                    // bool_to_int` test).
                    // `&`, `|` and `^` keep D-141's bool markers instead
                    // (#1210), which `binop::encode_int_operand` decides.
                    let l = binop::encode_int_operand(context, builder, *op, l);
                    let r = binop::encode_int_operand(context, builder, *op, r);
                    let rt_fn = match op {
                        pycc_mir::BinOpKind::Add => rt.int_add,
                        pycc_mir::BinOpKind::Sub => rt.int_sub,
                        pycc_mir::BinOpKind::Mul => rt.int_mul,
                        pycc_mir::BinOpKind::FloorDiv => rt.int_floordiv,
                        pycc_mir::BinOpKind::Mod => rt.int_floormod,
                        pycc_mir::BinOpKind::Pow => rt.int_pow,
                        pycc_mir::BinOpKind::LShift => rt.int_lshift,
                        pycc_mir::BinOpKind::RShift => rt.int_rshift,
                        pycc_mir::BinOpKind::BitAnd => rt.int_and,
                        pycc_mir::BinOpKind::BitOr => rt.int_or,
                        pycc_mir::BinOpKind::BitXor => rt.int_xor,
                        pycc_mir::BinOpKind::Div => unreachable!(
                            "pycc_types/pycc_mir always type true division as Ty::Float"
                        ),
                    };
                    let result = builder
                        .build_call(rt_fn, &[l.into(), r.into()], "int_binop")
                        .expect("build_call should not fail for a well-formed int binop")
                        .try_as_basic_value()
                        .expect_basic("pycc_rt_int_* functions all return a non-void `i64`");
                    // #146 Part 2 (D-181): both operands are dead now. Every
                    // `pycc_rt_int_*` above builds a bigint result through
                    // `tag_bigint(BigIntObj::new(..))`, never by handing an
                    // operand's own word back, so releasing here cannot free
                    // the value just computed -- an invariant that arithmetic
                    // export's own doc comment states and
                    // `an_int_operation_never_returns_an_operand_s_own_word`
                    // pins.
                    release_if_int_temporary(context, builder, rt, left, l);
                    release_if_int_temporary(context, builder, rt, right, r);
                    Scalar::Int(result.into_int_value())
                }
                Ty::Float => {
                    let l = to_float(context, builder, rt, l);
                    let r = to_float(context, builder, rt, r);
                    match op {
                        pycc_mir::BinOpKind::Add => Scalar::Float(
                            builder
                                .build_float_add(l, r, "fadd")
                                .expect("build_float_add should not fail for two f64 operands"),
                        ),
                        pycc_mir::BinOpKind::Sub => Scalar::Float(
                            builder
                                .build_float_sub(l, r, "fsub")
                                .expect("build_float_sub should not fail for two f64 operands"),
                        ),
                        pycc_mir::BinOpKind::Mul => Scalar::Float(
                            builder
                                .build_float_mul(l, r, "fmul")
                                .expect("build_float_mul should not fail for two f64 operands"),
                        ),
                        pycc_mir::BinOpKind::Div => {
                            Scalar::Float(build_float_rt_binop(builder, rt.float_div, l, r))
                        }
                        // Each arm binds its own `rt_fn` directly (rather
                        // than a shared `FloorDiv | Mod | Pow` arm re-matching
                        // `op` to pick one) so there is no redundant,
                        // provably unreachable `_` fallback arm left over --
                        // this outer `match op` already guarantees exactly
                        // one of these three, and Rust's own exhaustiveness
                        // checker (not a defensive catch-all) is what proves
                        // that here (same reasoning as this file's other
                        // documented dead-code removals, e.g.
                        // `emit_expr`'s `Name` arm above).
                        pycc_mir::BinOpKind::FloorDiv => {
                            Scalar::Float(build_float_rt_binop(builder, rt.float_floordiv, l, r))
                        }
                        pycc_mir::BinOpKind::Mod => {
                            Scalar::Float(build_float_rt_binop(builder, rt.float_floormod, l, r))
                        }
                        pycc_mir::BinOpKind::Pow => {
                            Scalar::Float(build_float_rt_binop(builder, rt.float_pow, l, r))
                        }
                        other => unreachable!("pycc_types rejects a float `{other:?}` (T0021)"),
                    }
                }
                // #1210: `&`, `|` and `^` over two `bool` operands.
                Ty::Bool => binop::emit_bool_bitwise(builder, *op, l, r),
                Ty::Str => {
                    // #575 (Part 2 of #123): string repetition. `pycc_types`
                    // accepts `str * int` / `int * str` (with `bool` as the
                    // count, since `bool <: int`) and
                    // `pycc_mir::binop_result_ty` types it `str`; this arm
                    // now emits it rather than stopping at Part 1's named
                    // D-072 boundary. MIR needs no repetition node of its
                    // own: `BinOp { op: Mul, ty: Ty::Str }` *is* the
                    // repetition, and this `match ty` is what distinguishes
                    // it from numeric multiplication.
                    //
                    // Handled *before* the two operand destructures below on
                    // purpose: the count evaluates to a `Scalar::Int`/
                    // `Scalar::Bool`, so leaving it to fall through would
                    // report the destructures' "internal error" message.
                    if *op == pycc_mir::BinOpKind::Mul {
                        // Both operand orders land here -- whichever side is
                        // the `str` becomes the operand, the other the count.
                        //
                        // Review note: a `str * str` pair (unreachable from
                        // any real pipeline, since `pycc_types` rejects it
                        // with `T0021`) matches the first arm and then fails
                        // inside `to_numeric_encoded_int` with its own
                        // "expected an int-or-bool operand, got str" message
                        // rather than the `str {op} str` message below. That
                        // shadowing is accepted, exactly as Part 1 accepted
                        // it, rather than worked around for a shape no
                        // type-checked program can produce.
                        let (operand, count) = match (l, r) {
                            (Scalar::Str(operand), count) => (operand, count),
                            (count, Scalar::Str(operand)) => (operand, count),
                            _ => panic!(
                                "pycc_codegen: internal error: a str-result `*` had no str operand"
                            ),
                        };
                        // `to_numeric_encoded_int` promotes a `bool` count to
                        // the same D-141 encoding an `int` count already
                        // carries (`"ab" * True` is `"ab"`), and
                        // `build_untag_checked` decodes it to the raw counter
                        // `pycc_rt_str_repeat` expects -- keeping bigint
                        // rejection in D-141's single runtime-owned
                        // classifier instead of duplicating it here.
                        let encoded = to_numeric_encoded_int(context, builder, count);
                        let raw = build_untag_checked(builder, rt, encoded, "str_repeat_count");
                        let result = builder
                            .build_call(rt.str_repeat, &[operand.into(), raw.into()], "str_repeat")
                            .expect("build_call should not fail for a well-formed repetition")
                            .try_as_basic_value()
                            .expect_basic("pycc_rt_str_repeat returns a non-void pointer");
                        return Scalar::Str(result.into_pointer_value());
                    }
                    let Scalar::Str(l) = l else {
                        panic!(
                            "pycc_codegen: internal error: str BinOp operand did not evaluate to str"
                        )
                    };
                    let Scalar::Str(r) = r else {
                        panic!(
                            "pycc_codegen: internal error: str BinOp operand did not evaluate to str"
                        )
                    };
                    if *op != pycc_mir::BinOpKind::Add {
                        panic!(
                            "pycc_codegen: `str {op:?} str` is not supported yet (only concatenation is)"
                        );
                    }
                    let result = builder
                        .build_call(rt.str_concat, &[l.into(), r.into()], "str_concat")
                        .expect("build_call should not fail for a well-formed concatenation")
                        .try_as_basic_value()
                        .expect_basic("pycc_rt_str_concat returns a non-void pointer");
                    Scalar::Str(result.into_pointer_value())
                }
                // No container type supports any `BinOpKind` in this plan's
                // own scope -- not even `+` for list concatenation, which
                // D-105 defers past v0.2. This arm is a pure diagnostic-
                // message improvement over the generic `other` catch-all
                // below (naming the specific container type via `.name()`
                // and calling out that it's the *operator* that's
                // unsupported, not just the result type), not new
                // capability.
                Ty::List(_) | Ty::Dict(..) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_) => {
                    panic!(
                        "pycc_codegen: binary operators are not supported on {} yet",
                        ty.name()
                    )
                }
                other => panic!("pycc_codegen: a `{other:?}`-result BinOp is not supported yet"),
            }
        }
        MirExpr::Compare {
            op, left, right, ..
        } => {
            let l = emit_expr(context, builder, module, rt, user_functions, locals, left);
            // #638 (D-208): same protection as `BinOp`'s `Ty::Int` arm --
            // `l` must survive `right`'s evaluation intact so its release
            // below is guaranteed to see it, even when `right`'s evaluation
            // branches away on an exception.
            let pending_l = push_pending_int_release_if_scalar_temporary(rt, left, &l);
            let r = emit_expr(context, builder, module, rt, user_functions, locals, right);
            pop_pending_int_release(rt, pending_l);
            let as_bool =
                compare::emit_compare_values(context, builder, rt, *op, left, l, right, r);
            // #146 Part 2 (D-181): no comparison retains either operand, so
            // both are dead the moment the comparison has been made. #1212
            // moved this out of the int-only branch, which also releases an
            // `int` temporary compared against a `float`
            // (`pycc_rt_int_to_float` reads its operand and consumes
            // nothing).
            release_scalar_if_int_temporary(context, builder, rt, left, &l);
            release_scalar_if_int_temporary(context, builder, rt, right, &r);
            Scalar::Bool(as_bool)
        }
        // #1212 (Part 4 of #1018): chained comparisons. See
        // `compare_chain.rs`.
        MirExpr::CompareChain { first, links } => compare_chain::emit_compare_chain(
            &boolop::Emitter {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            },
            first,
            links,
        ),
        MirExpr::BoolLiteral(b) => Scalar::Bool(context.i8_type().const_int(u64::from(*b), false)),
        // #604 (Part 3 of #573): `not x`. Reuses `truthy`, the exact same
        // helper an `if`/`while` condition's own test already calls (see
        // `MirStmt::If` above), then inverts the resulting `i1` -- so a
        // `not` over any operand `truthy` can classify (`bool`/`int`/
        // `float`/`str`/`Optional`, and a CPython object since Part 10 of
        // #1371; `pycc_types::unop::unary_result_type` rejects every other
        // operand before this ever runs) gets
        // `if`/`while`'s exact truthiness semantics for free, including
        // the `float`'s `UNE`-not-`ONE` NaN handling and `Optional`'s
        // present/payload AND. Every other truthy-call site releases any
        // int temporary the operand produced *after* `truthy` reads it
        // (#146 Part 2, D-181) -- this one follows the identical sequence.
        // #1211 (Part 3 of #1018): `and`/`or`. See `boolop.rs`.
        MirExpr::BoolOp {
            op,
            left,
            right,
            ty,
            truth_only,
        } => boolop::emit_bool_op(
            &boolop::Emitter {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            },
            *op,
            left,
            right,
            ty,
            *truth_only,
        ),
        // #1395: `body if test else orelse`. See `if_exp.rs`.
        MirExpr::IfExp {
            test,
            body,
            orelse,
            ty,
        } => if_exp::emit_if_exp(
            &boolop::Emitter {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            },
            test,
            body,
            orelse,
            ty,
        ),
        MirExpr::Not(operand) => {
            let operand_scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                operand,
            );
            let held = object_release::hold(context, module, rt, operand, &operand_scalar);
            let truthy_cond = truthy(context, builder, module, rt, operand_scalar);
            held.release(builder, rt);
            release_scalar_if_int_temporary(context, builder, rt, operand, &operand_scalar);
            let inverted = builder
                .build_not(truthy_cond, "not_truthy")
                .expect("build_not should not fail inverting a well-formed i1");
            let as_bool = builder
                .build_int_z_extend(inverted, context.i8_type(), "bool_from_not")
                .expect("build_int_z_extend should not fail widening i1 to i8");
            Scalar::Bool(as_bool)
        }
        MirExpr::Call { callee, args, ty } => {
            if callee == "print" {
                panic!(
                    "pycc_codegen: using print()'s result as a nested expression is not supported yet"
                );
            }
            // D-136 Task 4: `math.sqrt(x: float) -> float` is this PR's one
            // lowered stdlib function, calling straight into the platform
            // libm `sqrt` symbol -- the same C ABI this crate's existing
            // float codegen already links against for `**`'s `pow` call
            // (see `main.rs`'s `add_linux_system_libs` doc comment: macOS
            // folds libm into libSystem and Windows's UCRT bundles it, so
            // only Linux needs an explicit `-lm`, already added there).
            // Declared once per module and reused on every call site
            // (`get_function` first, matching `declare_rt_functions`'s own
            // idempotent-declare precedent) rather than redeclaring per
            // call -- LLVM rejects a duplicate `declare` with a different
            // linkage/signature, and there is no reason to risk that for a
            // fixed, known-once signature.
            if callee == "math.sqrt" {
                let [arg] = args.as_slice() else {
                    panic!(
                        "pycc_codegen: internal error: `math.sqrt` takes exactly 1 argument, got {} \
                         -- pycc_types::check (T0021) should have rejected this before codegen",
                        args.len()
                    )
                };
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
                let f64_type = context.f64_type();
                let sqrt_fn = module.get_function("sqrt").unwrap_or_else(|| {
                    module.add_function(
                        "sqrt",
                        f64_type.fn_type(&[f64_type.into()], false),
                        Some(inkwell::module::Linkage::External),
                    )
                });
                // `pycc_types::std_qualified_symbol`'s call-site check
                // already rejects any argument whose static type isn't
                // `Ty::Float` (T0021) before codegen runs -- this match
                // is a defensive backstop against malformed MIR, mirroring
                // `len`'s/`float`'s own internal-error convention above,
                // not a real dispatch over legitimate source.
                let Scalar::Float(arg_value) = scalar else {
                    panic!(
                        "pycc_codegen: internal error: `math.sqrt`'s argument was not a float \
                         -- pycc_types::check (T0021) should have rejected this before codegen"
                    )
                };
                let call_site = builder
                    .build_call(sqrt_fn, &[arg_value.into()], "math_sqrt")
                    .expect("build_call should not fail for the libm `sqrt` declaration");
                return Scalar::Float(
                    call_site
                        .try_as_basic_value()
                        .expect_basic("libm `sqrt` is declared to return a double")
                        .into_float_value(),
                );
            }
            // `len` is the second hand-recognized builtin (D-105 point 3),
            // dispatched here for the same reason `print` is: it has no
            // `user_functions` entry, so it must be claimed before the
            // lookup below turns it into an "undefined function" panic.
            // Mirrors `pycc_types`' own `callee == "len"` arm and
            // `pycc_mir`'s, which already type it `Ty::Int` -- so this
            // returns a `Scalar::Int` directly rather than falling through
            // to the declared-return-type dispatch at the end of this arm.
            if callee == "len" {
                let [list_arg] = args.as_slice() else {
                    panic!(
                        "pycc_codegen: internal error: `len` takes exactly 1 argument, got {} \
                         -- pycc_types::check (T0033) should have rejected this before codegen",
                        args.len()
                    )
                };
                let scalar = emit_expr(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    list_arg,
                );
                // D-123 relaxed `len()` to also accept a `dict[K, V]`
                // argument alongside `list[T]` (PR-11 Task 5), and PR-11
                // Task 9 relaxed it once more to also accept `set[T]`,
                // dispatched on the argument's own static type -- mirrors
                // `pycc_types`'/`pycc_mir`'s identical relaxation at their
                // own hand-recognized `len` dispatch points.
                let raw_len = match list_arg.ty() {
                    Ty::Dict(_) => {
                        let dict_ptr = expect_dict_pointer(scalar, "`len`'s argument");
                        build_dict_len(builder, rt, dict_ptr)
                    }
                    Ty::Set(_) | Ty::FrozenSet(_) => {
                        let set_ptr = expect_set_pointer(scalar, "`len`'s argument");
                        build_int_set_len(builder, rt, set_ptr)
                    }
                    _ => {
                        let list_ptr = expect_list_pointer(scalar, "`len`'s argument");
                        build_int_list_len(builder, rt, list_ptr)
                    }
                };
                // The raw count becomes a user-visible
                // `Ty::Int` expression value here (`print(len(x))`,
                // `n = len(x)`), so it is re-tagged -- unlike
                // `MirStmt::ForList`'s own use of the same runtime call,
                // which keeps it raw as a private loop bound.
                return Scalar::Int(raw_i64_to_tagged_int(context, builder, raw_len));
            }
            if callee == "float" && !user_functions.contains_key(callee.as_str()) {
                // Hand-recognized builtin -- but, unlike `len` above, only when
                // no `user_functions` entry claims the name first (mirrors
                // `pycc_types`/`pycc_mir`'s own identical guard; see
                // `pycc_types::infer_expr_in`'s comment for why `float`, unlike
                // `len`/`print`, needs one). Reuses `to_float`, which already dispatches
                // `Scalar::Int`/`Scalar::Bool`/`Scalar::Float` correctly (the same
                // helper arithmetic-promotion already calls) and already panics
                // correctly for `Scalar::Str`/`List`/`Dict`/`Set`/`Tuple` -- no new conversion
                // logic, only a new caller. `pycc_types` already rejects a
                // non-numeric or mis-arity call with T0021 before codegen ever runs;
                // this arity check is a defensive backstop against malformed MIR,
                // mirroring `len`'s own internal-error convention immediately above.
                let [arg] = args.as_slice() else {
                    panic!(
                        "pycc_codegen: internal error: `float` takes exactly 1 argument, got {} \
                         -- pycc_types::check (T0021) should have rejected this before codegen",
                        args.len()
                    )
                };
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
                // Part 4 of #1026 (PR 4a of #1083): a CPython object cannot go
                // through `to_float`, which knows only pycc's own scalars and
                // panics on `Scalar::Object`. It takes the shim helper instead,
                // which runs CPython's own `PyNumber_Float` protocol -- an
                // explicit conversion the author asked for by naming the
                // destination type, not the implicit thunk-seam crossing D-244
                // rule 7 closes (see `foreign_len::emit_to_float`).
                if matches!(scalar, Scalar::Object(_)) {
                    let held = object_release::hold(context, module, rt, arg, &scalar);
                    let result = foreign_len::emit_to_float(context, builder, module, rt, scalar);
                    held.release(builder, rt);
                    return result;
                }
                return Scalar::Float(to_float(context, builder, rt, scalar));
            }
            // #1331: `hash(x)`, guarded like `float` above; `pycc_types`
            // admits exactly one argument. A class instance never arrives
            // here: `pycc_mir` lowers it to `MirExpr::InstanceHash` (#1335).
            if let ("hash", [arg]) = (callee.as_str(), args.as_slice())
                && !user_functions.contains_key(callee.as_str())
            {
                return hash::emit_hash(context, builder, module, rt, user_functions, locals, arg);
            }
            if callee == "bool" && !user_functions.contains_key(callee.as_str()) {
                // Part 4 of #1026 (PR 4a of #1083). `pycc_types` admits the
                // `bool` builtin for a `Ty::Object` argument *only* -- every
                // other argument type keeps its C0001 refusal -- and only when
                // no user `def bool` claims the name, which is the same guard
                // `float` above carries and for the same reason. So this arm
                // sees exactly one shape, and both backstops below are against
                // malformed MIR rather than against legitimate source.
                //
                // No new shim symbol: `bool(o)` is the truth test PR 3a already
                // emits for an `if`/`while` condition, widened from `i1` to the
                // `i8` a `Scalar::Bool` carries -- exactly what `truthy`'s own
                // `Scalar::Object` arm does.
                let [arg] = args.as_slice() else {
                    panic!(
                        "pycc_codegen: internal error: `bool` takes exactly 1 argument, got {} \
                         -- pycc_types::check (T0021) should have rejected this before codegen",
                        args.len()
                    )
                };
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
                let Scalar::Object(ptr) = scalar else {
                    panic!(
                        "pycc_codegen: internal error: `bool` takes a CPython object argument \
                         -- pycc_types::check (C0001) should have rejected this before codegen"
                    )
                };
                let held = object_release::hold(context, module, rt, arg, &scalar);
                let bit = foreign_len::emit_truthy(context, builder, module, rt, ptr);
                held.release(builder, rt);
                return Scalar::Bool(
                    builder
                        .build_int_z_extend(bit, context.i8_type(), "bool_from_object")
                        .expect("build_int_z_extend should not fail widening i1 to i8"),
                );
            }
            // Part 4 of #1026 (PR 4b of #1083): `int(o)` and `str(o)`. Both
            // are the `bool` arm above in every structural respect -- admitted
            // by `pycc_types` for a `Ty::Object` argument *only*, under the
            // same user-defined-function guard, so each sees exactly one shape
            // and both backstops below are against malformed MIR -- and they
            // differ only in which shim helper they reach and what it answers.
            // One arm dispatching on the emitter rather than two near-identical
            // arms, because at this layer the emission genuinely is the same
            // code; `pycc_types` is where the two names diverge, and it already
            // has.
            let object_conversion: Option<ObjectConversionEmitter> = match callee.as_str() {
                "int" => Some(foreign_len::emit_to_int),
                "str" => Some(foreign_len::emit_to_str),
                _ => None,
            };
            if let Some(emit) = object_conversion
                && !user_functions.contains_key(callee.as_str())
            {
                let [arg] = args.as_slice() else {
                    panic!(
                        "pycc_codegen: internal error: `{callee}` takes exactly 1 argument, got {} \
                         -- pycc_types::check (T0021) should have rejected this before codegen",
                        args.len()
                    )
                };
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
                if !matches!(scalar, Scalar::Object(_)) {
                    panic!(
                        "pycc_codegen: internal error: `{callee}` takes a CPython object argument \
                         -- pycc_types::check (C0001) should have rejected this before codegen"
                    )
                }
                let held = object_release::hold(context, module, rt, arg, &scalar);
                let result = emit(context, builder, module, rt, scalar);
                held.release(builder, rt);
                return result;
            }
            // Unlike `emit_stmt`'s void-call arm below, there is no
            // `Result` here to propagate a clean, user-facing error
            // through -- `emit_expr` returns a `Scalar` unconditionally, so
            // an undefined callee can only be this crate's own internal
            // error. Real `pycc_types` already rejects any call to an
            // undefined function (T0021) long before codegen runs, so this
            // is a defensive backstop, not a rejection of legitimate
            // source (see `calling_an_undefined_function_as_a_nested_
            // expression_is_an_internal_error` below).
            let user_function = user_functions.get(callee.as_str()).unwrap_or_else(|| {
                panic!(
                    "pycc_codegen: internal error: call to undefined function `{callee}` \
                     should have been rejected by pycc_types before reaching codegen"
                )
            });
            let _ = user_function; // validated above; build_call_to looks up by name
            let call_site = build_call_to(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                callee,
                args,
            );
            crate::call_result::call_result_scalar(context, call_site, ty)
        }
        MirExpr::FString(parts) => {
            // Two deviations from the task brief, both here:
            //
            // 1. The brief's own version of this arm's `str_concat` call
            //    ended `.try_as_basic_value().left().expect(msg)` -- same
            //    `.left()`-doesn't-exist-on-this-inkwell-version issue
            //    documented on `to_str` and `BinOp` above, fixed the same
            //    way (`.expect_basic(msg)`).
            // 2. The brief's own `Literal`-part case called `emit_expr`
            //    recursively on a synthetic `MirExpr::StringLiteral(s.
            //    clone())` and then defensively pattern-matched the
            //    `Scalar::Str` back out with a `let ... else {
            //    unreachable!(...) }`. That `unreachable!()` arm is
            //    *provably* dead code (confirmed by `cargo llvm-cov`: an
            //    uncovered region under D-014's 100% gate) -- `MirExpr::
            //    StringLiteral` always evaluates to `Scalar::Str`, nothing
            //    else. Fixed by calling `emit_string_literal` (extracted
            //    from `emit_expr`'s own `StringLiteral` arm) directly,
            //    which returns the `PointerValue` itself with no `Scalar`
            //    round-trip and so no impossible arm to cover -- see that
            //    helper's own doc comment.
            let mut acc: Option<PointerValue<'ctx>> = None;
            for part in parts {
                let part_str = match part {
                    pycc_mir::MirFStringPart::Literal(s) => {
                        emit_string_literal(context, builder, module, rt, s)
                    }
                    pycc_mir::MirFStringPart::Interpolation(inner) => {
                        string_render::emit_fstring_interpolation(
                            context,
                            builder,
                            module,
                            rt,
                            user_functions,
                            locals,
                            inner,
                        )
                    }
                };
                acc = Some(match acc {
                    None => part_str,
                    Some(prev) => {
                        let joined = builder
                            .build_call(
                                rt.str_concat,
                                &[prev.into(), part_str.into()],
                                "fstring_concat",
                            )
                            .expect("build_call should not fail for a well-formed concatenation")
                            .try_as_basic_value()
                            .expect_basic("pycc_rt_str_concat returns a non-void pointer")
                            .into_pointer_value();
                        builder
                            .build_call(rt.str_decref, &[prev.into()], "fstring_decref_prev")
                            .expect("build_call should not fail for a well-formed decref");
                        builder
                            .build_call(rt.str_decref, &[part_str.into()], "fstring_decref_part")
                            .expect("build_call should not fail for a well-formed decref");
                        joined
                    }
                });
            }
            Scalar::Str(acc.unwrap_or_else(|| {
                // An empty `FString(vec![])` never actually reaches this arm --
                // `pycc_hir`'s own f-string lowering always produces at least
                // one `Literal` part (an empty Python f-string `f""` still
                // lowers to `FString(vec![FStringPart::Literal("")])`, never a
                // truly empty `Vec`) -- but guard it explicitly rather than
                // silently returning a dangling/null pointer if that assumption
                // is ever wrong.
                panic!("pycc_codegen: internal error: an f-string with zero parts should not be reachable")
            }))
        }
        // `[e1, e2, ...]` (D-105): an empty `pycc_rt_int_list_new()` object
        // followed by one `pycc_rt_int_list_append` per element, in source
        // order. No pre-sizing call: `PyIntListObj`'s payload is a
        // `Vec<i64>` whose own amortized-doubling `push` already handles
        // growth (see that struct's doc comment), so a reserve entry point
        // would be new runtime surface for no behavioral difference.
        //
        // D-141: each element becomes an int-compatible encoded word. The
        // checked decoder validates bigint exclusion, but storage keeps the
        // original word so a bool marker survives the round-trip.
        //
        // `to_encoded_int` rather than a `let Scalar::Int(..) else` match, for
        // the same reason `emit_expr`'s own `BinOp`/`Ty::Int` arm uses it:
        // Python's `bool` is an `int` subtype, so widening is the correct
        // response to a `Scalar::Bool` element rather than an error -- and
        // its `Float`/`Str`/`List` arms already panic honestly.
        // #1021: a resolved empty `[]`. Structurally `ListLiteral` with no
        // elements -- the same `pycc_rt_int_list_new()` object, with no
        // `append` calls after it. It is a separate arm only because the
        // variant carries its element type instead of deriving one.
        MirExpr::EmptyList(_) => Scalar::List(
            builder
                .build_call(rt.int_list_new, &[], "list_new")
                .expect("build_call should not fail for a well-formed list construction")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_list_new returns a non-void pointer")
                .into_pointer_value(),
        ),
        // #1021: a resolved empty `{}`, the dict counterpart of
        // `EmptyList` immediately above.
        MirExpr::EmptyDict(_) => Scalar::Dict(
            builder
                .build_call(rt.dict_new, &[], "dict_new")
                .expect("build_call should not fail for a well-formed dict construction")
                .try_as_basic_value()
                .expect_basic("pycc_rt_dict_new returns a non-void pointer")
                .into_pointer_value(),
        ),
        MirExpr::ListLiteral(elements) => {
            let list_ptr = builder
                .build_call(rt.int_list_new, &[], "list_new")
                .expect("build_call should not fail for a well-formed list construction")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_list_new returns a non-void pointer")
                .into_pointer_value();
            for element in elements {
                let scalar = emit_expr(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    element,
                );
                let encoded = to_encoded_int(context, builder, scalar);
                let _ = build_untag_checked(builder, rt, encoded, "list_validate_element");
                build_int_list_append(builder, rt, list_ptr, encoded);
            }
            Scalar::List(list_ptr)
        }
        // `base[index]`, read-only, over two structurally different bases
        // (PR-11b Task 5). A `list[T]` base is a runtime-indexed read into a
        // heap object; a `tuple[...]` base is a compile-time-indexed field
        // extraction from an SSA aggregate (D-115). They share only their
        // surface syntax, so this arm dispatches between them rather than
        // treating one as a special case of the other. `dict[K, V]` never
        // reaches here at all -- `pycc_mir`'s `lower_expr` routes a dict
        // base to `MirExpr::DictGet` instead.
        MirExpr::Subscript { base, index } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            // Matched as a *pair* rather than branching on `base.ty()` alone
            // (PR-11b Task 5). Branching on the type alone would need a
            // `let Scalar::Tuple(..) = base_scalar else { panic!(..) }`
            // inside the tuple arm -- and that arm would be permanently
            // uncoverable under D-014's 100%-region gate, because every
            // expression whose `ty()` is `Ty::Tuple` already evaluates to
            // `Scalar::Tuple` by construction: `TupleLiteral` returns one
            // directly, `Name` returns one unconditionally from its own
            // `Ty::Tuple(_)` arm (which dispatches on the very `ty` field
            // `base.ty()` just read, so the two cannot disagree -- this
            // holds on its own, without relying on that arm's
            // `debug_assert_eq!`, which is compiled out in release), and a
            // `Ty::Tuple`-typed `Call` returns one from its own arm as
            // well, since #925 gave the container return annotations their
            // codegen call-result arms -- that arm dispatches on the same
            // declared return type `base.ty()` reports, so the two cannot
            // disagree there either. (Before #925 the case was vacuous for
            // a different reason: such a `Call` panicked at the container
            // catch-all in this same function.)
            // Pairing the two instead lets that impossible
            // combination fall into the list path's already-covered
            // `expect_list_pointer` check, adding no new branch that no
            // test could ever legitimately reach -- the same
            // "no redundant impossible-to-cover branch" convention
            // `emit_string_literal`'s own doc comment records.
            match (base.ty(), base_scalar) {
                // D-115: the field is read straight off the loaded
                // `StructValue` with `extractvalue` -- no pointer, no
                // `build_struct_gep`, because the aggregate is an SSA
                // register rather than something in memory.
                (pycc_mir::Ty::Tuple(_), Scalar::Tuple(struct_value)) => {
                    let MirExpr::IntLiteral(literal_index) = index.as_ref() else {
                        panic!(
                            "pycc_codegen: internal error: a tuple subscript index is not a \
                             literal int -- pycc_types::check (T0040) should have rejected this \
                             before codegen"
                        )
                    };
                    // Delegates the non-negative and in-range validation to
                    // `MirExpr::ty()`, which already re-derives exactly this
                    // positional element type from the same literal index
                    // and panics on either failure (see `pycc_mir`'s own
                    // `Subscript`/`Ty::Tuple` arm). Re-checking here would
                    // duplicate that logic *and* add two more panic regions
                    // this crate would then have to cover itself.
                    let elem_ty = expr.ty();
                    let field_value = builder
                        .build_extract_value(struct_value, *literal_index as u32, "tuple_extract")
                        .expect("build_extract_value should not fail for a validated index");
                    // No output conversion: like D-141's container payloads,
                    // a tuple field already holds the encoded value that
                    // `TupleLiteral` inserted. Re-tagging would corrupt every
                    // ordinary `int` element and erase bool identity.
                    match elem_ty {
                        Ty::Int => Scalar::Int(field_value.into_int_value()),
                        Ty::Bool => Scalar::Bool(field_value.into_int_value()),
                        Ty::Float => Scalar::Float(field_value.into_float_value()),
                        // Defensive: `pycc_types`' T0039 gate (D-116)
                        // rejects every other element type before codegen.
                        other => panic!(
                            "pycc_codegen: reading a `{}`-typed tuple element is not supported yet",
                            other.name()
                        ),
                    }
                }
                // `base[index]`, read-only (D-105 scope cut 2). The index is
                // decoded to a raw positional counter; the D-141 element
                // word read back out is already user-visible and passes
                // through unchanged.
                //
                // An out-of-range (including any negative) index is
                // `pycc_rt_int_list_get`'s own honest runtime panic, not
                // something this crate can check -- the index is only known
                // at runtime. That is the opposite of the tuple arm above,
                // where the index is a compile-time literal `pycc_types`
                // has already bounds-checked.
                (_, base_scalar) => {
                    let base_ptr = expect_list_pointer(base_scalar, "the subscripted value");
                    let index_scalar =
                        emit_expr(context, builder, module, rt, user_functions, locals, index);
                    let encoded_index = to_numeric_encoded_int(context, builder, index_scalar);
                    let raw_index =
                        build_untag_checked(builder, rt, encoded_index, "list_untag_index");
                    let encoded_element = build_int_list_get(builder, rt, base_ptr, raw_index);
                    Scalar::Int(encoded_element)
                }
            }
        }
        // `list.append(value)` (D-105 point 3). Same D-141 validation and
        // identity-preserving storage as `ListLiteral` above. `list` is a
        // plain variable name or (#1263) an attribute read, resolved through
        // `emit_list_receiver` *before* `value` is evaluated -- CPython
        // evaluates the bound method's receiver before its arguments.
        //
        // Python's `list.append` evaluates to `None`; the canonical `i8 0`
        // unit carrier this crate uses for every other `None`-valued
        // expression is what comes back (identical to `emit_expr`'s `Call`
        // arm's own `Ty::None` case).
        MirExpr::ListAppend { list, value } => {
            let list_ptr =
                emit_list_receiver(context, builder, module, rt, user_functions, locals, list);
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            let encoded = to_encoded_int(context, builder, scalar);
            let _ = build_untag_checked(builder, rt, encoded, "list_validate_appended");
            build_int_list_append(builder, rt, list_ptr, encoded);
            Scalar::Bool(context.i8_type().const_int(0, false))
        }
        // `{k1: v1, k2: v2, ...}` (PR-11 Task 5, D-123): an empty
        // `pycc_rt_dict_new()` object followed by one `pycc_rt_dict_set`
        // per pair, in source order -- mirrors `MirExpr::ListLiteral`'s own
        // shape exactly (no pre-sizing call: `PyDictObj`'s own payload
        // already handles growth itself, same reasoning as
        // `ListLiteral`'s own doc comment).
        //
        // D-141's encoded-value contract applies to the value half only:
        // the key is a `Ty::Str` expression that crosses into `PyDictObj`
        // unchanged (a dict key is a raw pointer either way, no tagging
        // scheme), while each value is validated against bigint storage and
        // then stored with its original encoded word.
        MirExpr::DictLiteral(pairs) => {
            let dict_ptr = builder
                .build_call(rt.dict_new, &[], "dict_new")
                .expect("build_call should not fail for a well-formed dict construction")
                .try_as_basic_value()
                .expect_basic("pycc_rt_dict_new returns a non-void pointer")
                .into_pointer_value();
            for (key, value) in pairs {
                let key_scalar =
                    emit_expr(context, builder, module, rt, user_functions, locals, key);
                // `PyDictObj` adopts the key pointer it is given as its own
                // permanent reference, without incref'ing it itself (D-124:
                // "the stored key pointer is neither increfed on insert nor
                // decrefed ... ever"). A bare-`Name` key (`{k: 1}`) is a
                // *duplicate* reference to whatever `PyStrObj` `k`'s own
                // slot already owns, exactly the shape `incref_if_str_
                // duplicate` exists to protect everywhere else a `str`
                // value crosses an ownership-taking boundary (`MirStmt::
                // Assign`, `build_call_to`'s argument marshalling,
                // `MirStmt::Return`) -- without this call, a later `k = ...`
                // reassignment would decref (and potentially free) the same
                // `PyStrObj` this dict still points to, a real premature
                // free, not this project's accepted list/dict leak-only
                // policy (D-107/D-124 only ever accept *never freeing*, not
                // freeing too early). A string-literal key (`{"a": 1}`)
                // freshly constructs its own owned reference every time, so
                // `str_value_is_a_duplicate_reference` correctly leaves it
                // untouched here.
                let key_scalar = incref_if_str_duplicate(builder, rt, key, key_scalar);
                // Every `Ty::Dict` value that survives `pycc_types`' own
                // T0036 gate has key type exactly `Ty::Str` (see that
                // gate's own comment), so this is a defensive backstop,
                // not a real feature gap -- the same inline
                // `let Scalar::Str(..) = .. else { panic!(..) }` shape
                // `emit_expr`'s own `BinOp`/`Compare` arms already use for
                // their `Ty::Str` operands, for the identical reason (no
                // shared helper: see this file's established convention of
                // not extracting a single-use-per-arm check into its own
                // function).
                let Scalar::Str(key_ptr) = key_scalar else {
                    panic!(
                        "pycc_codegen: internal error: dict literal key did not evaluate to str \
                         -- pycc_types::check (T0036) should have rejected this before codegen"
                    )
                };
                let value_scalar =
                    emit_expr(context, builder, module, rt, user_functions, locals, value);
                let encoded = to_encoded_int(context, builder, value_scalar);
                let _ = build_untag_checked(builder, rt, encoded, "dict_validate_literal_value");
                build_dict_set(builder, rt, dict_ptr, key_ptr, encoded);
            }
            Scalar::Dict(dict_ptr)
        }
        // `dict[key]`, read-only (PR-11 Task 5, D-123 -- `d[k] = v` is a
        // separate, statement-level operation, `MirStmt::DictSet` below).
        // Mirrors `MirExpr::Subscript`'s own shape: the key is a `Ty::Str`
        // expression crossing in unchanged (a dict key is never tagged),
        // and the encoded value read back out is forwarded unchanged.
        //
        // A missing key makes `pycc_rt_dict_get` set D-173's pending
        // `KeyError` state and return a neutral carrier. The enclosing
        // `emit_expr` wrapper guards this node before that carrier can be
        // consumed.
        MirExpr::DictGet { dict, key } => {
            let dict_scalar = emit_expr(context, builder, module, rt, user_functions, locals, dict);
            let dict_ptr = expect_dict_pointer(dict_scalar, "the dict subscripted value");
            let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
            let Scalar::Str(key_ptr) = key_scalar else {
                panic!(
                    "pycc_codegen: internal error: dict subscript key did not evaluate to str \
                     -- pycc_types::check (T0021) should have rejected this before codegen"
                )
            };
            let encoded_value = builder
                .build_call(rt.dict_get, &[dict_ptr.into(), key_ptr.into()], "dict_get")
                .expect("build_call should not fail for a well-formed dict read")
                .try_as_basic_value()
                .expect_basic("pycc_rt_dict_get returns a non-void i64")
                .into_int_value();
            Scalar::Int(encoded_value)
        }
        // `{e1, e2, ...}` (PR-11 Task 9, D-123/D-121): an empty
        // `pycc_rt_int_set_new()` object followed by one
        // `pycc_rt_int_set_add` per element, in source order -- mirrors
        // `MirExpr::ListLiteral`'s own shape exactly (no pre-sizing call:
        // `PyIntSetObj`'s own payload already handles growth itself, same
        // reasoning as `ListLiteral`'s own doc comment), except that
        // `set[int]` has no string-keyed counterpart to `DictLiteral`'s own
        // per-pair key handling -- every element is an encoded `i64`, so there is
        // no refcounting concern here at all (unlike `DictLiteral`'s key,
        // D-123's own T0038 gate means every `set[int]` element is exactly
        // `Ty::Int`).
        //
        // D-141 applies to every element: validate bigint exclusion, then
        // store the original encoded word. The dedup check that makes a repeated element
        // collapse to one (D-121) lives entirely inside
        // `pycc_rt_int_set_add` itself (see `build_int_set_add`'s own doc
        // comment) -- this arm calls it per element, unconditionally, with
        // no dedup logic of its own.
        //
        // A set of user-class instances (#1343) carries `ops` and is built
        // by `set_instance.rs`, which drives the user `__hash__`/`__eq__`.
        MirExpr::SetLiteral {
            elements,
            ops: Some(ops),
        } => {
            let emitter = SetEmitter {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            };
            Scalar::Set(emitter.literal(elements, ops))
        }
        MirExpr::SetLiteral {
            elements,
            ops: None,
        } => {
            let set_ptr = builder
                .build_call(rt.int_set_new, &[], "set_new")
                .expect("build_call should not fail for a well-formed set construction")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_set_new returns a non-void pointer")
                .into_pointer_value();
            for element in elements {
                let scalar = emit_expr(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    element,
                );
                let encoded = to_encoded_int(context, builder, scalar);
                let _ = build_untag_checked(builder, rt, encoded, "set_validate_element");
                build_int_set_add(builder, rt, set_ptr, encoded);
            }
            Scalar::Set(set_ptr)
        }
        // `(e1, e2, ...)` (PR-11b Task 5, D-115). The one container literal
        // in this file that calls into no `pycc_rt` constructor at all:
        // where `ListLiteral`/`DictLiteral`/`SetLiteral` each allocate a
        // heap object and then fill it, this builds a pure SSA aggregate --
        // `get_undef()` for the struct shape, then one
        // `build_insert_value` (LLVM's `insertvalue`) per element in source
        // order, each returning a *new* aggregate value rather than
        // mutating one in place. No alloca, no pointer, and no allocation
        // of the aggregate itself -- but D-107/D-124's leak-only policy
        // does apply to what the fields hold: under D-182 a borrowed `int`
        // element is retained on the way in (see the retain below) and
        // nothing releases it when the tuple dies.
        //
        // Deliberately no `build_untag_checked` per element: a tuple has no
        // runtime storage boundary to validate. Like D-141 containers, a
        // tuple field stores exactly the `Scalar` it was given. An `int`
        // element therefore goes in as its D-141 encoded word (ordinary
        // smallint, bool marker, or bigint pointer), and `MirExpr::Subscript`'s
        // tuple branch returns that word unchanged, so the two sides never
        // disagree and no conversion happens on either.
        MirExpr::TupleLiteral(elements) => {
            let elem_tys: Vec<pycc_mir::Ty> = elements.iter().map(MirExpr::ty).collect();
            let struct_ty = ty_to_basic_type(context, pycc_mir::Ty::Tuple(Box::new(elem_tys)))
                .into_struct_type();
            let mut aggregate = struct_ty.get_undef();
            // #638 (D-208), sixth site: a fresh (non-duplicate) `Ty::Int`
            // element's ownership transfers into the aggregate's own field
            // on the normal path -- there is no release call for it
            // anywhere in this arm, exactly like `build_call_to_with_leading_args`'s
            // argument-marshalling loop. If a *later* sibling element's own
            // evaluation raises before the loop below completes, that
            // transfer never happens and an earlier element's birth
            // reference would otherwise be orphaned. `mark` records the
            // stack depth before this loop so the truncate below is
            // recursion-safe: an element expression can itself contain a
            // nested call or tuple literal whose own loop pushes and pops
            // its own entries while evaluating this loop's own element,
            // and this loop's earlier pending entries must stay untouched
            // by that nested truncation.
            let mark = rt.exceptions.pending_int_releases.borrow().len();
            for (index, element) in elements.iter().enumerate() {
                let scalar = emit_expr(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    element,
                );
                // D-182: a tuple field takes its own reference at ingress.
                // Only a *borrowed* element word is retained here -- that is
                // what `retain_if_int_duplicate_and_track_for_exception_edge`
                // classifies -- because an owning element (a fresh `BinOp`
                // result, an out-of-range literal) already arrives with the
                // single reference this field will hold. Retaining
                // unconditionally would leave rc at 2 with only one owner,
                // which a future `Ty::Tuple` slot-death release under D-124
                // could never balance.
                //
                // `emit_bigint_refcount_call`'s documented postcondition:
                // on the non-constant path it leaves the builder positioned
                // in a fresh continuation block, so `get_insert_block()`
                // must be re-read unconditionally after this call. The
                // `build_insert_value` below is block-agnostic and takes
                // part in no phi, so nothing here depends on the block the
                // element was evaluated in.
                let scalar = retain_if_int_duplicate_and_track_for_exception_edge(
                    context, builder, rt, element, scalar,
                );
                // Push *after* the retain-and-track call, matching
                // `build_call_to_with_leading_args`'s own call order:
                // `int_temporary_word` (via
                // `push_pending_int_release_if_scalar_temporary`) excludes
                // a duplicate reference by construction, so this is a
                // no-op for that case regardless of ordering -- the
                // retain-and-track call above already pushed a *duplicate*
                // element's retained word itself, if any. #834 is closed:
                // a borrowed/duplicate element's extra retain is now
                // protected on this exception edge exactly like an owning
                // element's own word already was, not merely D-180
                // residual 3 (which covers a retain that eventually
                // transfers to a real owner, not one abandoned before
                // transfer completes). See
                // https://github.com/rotnov/pycc/issues/834.
                push_pending_int_release_if_scalar_temporary(rt, element, &scalar);
                let field_value: inkwell::values::BasicValueEnum = match scalar {
                    Scalar::Int(v) => v.into(),
                    Scalar::Bool(v) => v.into(),
                    Scalar::Float(v) => v.into(),
                    // A defensive backstop, not a feature gap: `pycc_types`'
                    // own T0039 gate (D-116) rejects every tuple element
                    // type but `int`/`bool`/`float` before codegen runs, so
                    // no type-checked program reaches this. Named in prose
                    // rather than `{other:?}` because `Scalar` deliberately
                    // derives no `Debug` -- matching `expect_list_pointer`'s
                    // own message style.
                    _ => panic!(
                        "pycc_codegen: internal error: a tuple element evaluated to a \
                         non-int/bool/float value -- pycc_types::check (T0039) should have \
                         rejected this before codegen"
                    ),
                };
                aggregate = builder
                    .build_insert_value(aggregate, field_value, index as u32, "tuple_insert")
                    .expect("build_insert_value should not fail for an in-range tuple field")
                    .into_struct_value();
            }
            // #638 (D-208): every element above evaluated successfully --
            // this Rust-level line is reached unconditionally at codegen
            // (emission) time regardless of what the *compiled* program's
            // exception-edge control flow does, since `guard_statement_effects`
            // only emits a runtime branch instruction, never diverts the
            // emitter itself (see `guard_statement_effects`'s own doc
            // comment). Truncate back to `mark` *without* releasing:
            // ownership of every entry pushed above has now transferred to
            // the aggregate's own fields via `build_insert_value`. Getting
            // this backwards (releasing instead of truncating) would
            // double-free every owning element on this normal,
            // non-exception path.
            rt.exceptions
                .pending_int_releases
                .borrow_mut()
                .truncate(mark);
            Scalar::Tuple(aggregate)
        }
        // `base[start:stop:step]` (PR-12 Task 9, D-118). Evaluates `base`,
        // then each present bound left to right, exactly matching Python's
        // own sub-expression evaluation order. `pycc_types` already
        // validated `base` is `list[int]` and every present bound is
        // `int`-assignable (Task 7); this arm applies D-118's own runtime
        // behavior (default/clamp/panic) that `pycc_mir`'s purely
        // structural lowering (Task 8) deliberately left to codegen.
        //
        // Deliberate deviation from this task's own originating plan
        // sketch: `base`'s length (needed only when `stop` is omitted) is
        // read *after* every present bound has already been evaluated, not
        // immediately after `base`. `len()` itself has no side effect, but
        // a present `start`/`step` expression might mutate `base` before
        // yielding its own value (e.g. a helper that appends to the same
        // list `base` names, then returns the bound) -- reading the length
        // this late means an omitted `stop` always reflects `base`'s state
        // as of just before the slice actually runs, matching CPython's own
        // "build the slice object from every sub-expression, then apply it"
        // order, where the length lookup happens once, after `start`/
        // `stop`/`step` have all already been evaluated. This costs nothing
        // in either code size or coverage: both the `Some`/`None` arms for
        // `stop` already need their own tests regardless of where the
        // length read sits.
        //
        // `xs = xs[1:3]` is safe from the identical self-referential-rebind
        // hazard `MirStmt::ListCompAssign`'s own doc comment documents at
        // length (Task 5a's confirmed regression): the only way this arm's
        // result reaches a name is through `MirStmt::Assign`, which fully
        // evaluates this whole expression to a `Scalar::List` -- a pointer
        // to a **freshly allocated** result object `pycc_rt_int_list_slice`
        // returns, never `base_ptr` itself -- before it ever calls
        // `emit_assign` to rebind `target`'s own slot. This arm never
        // writes to `locals` at all, so there is no premature-rebind window
        // here the way an earlier `ListCompAssign` draft had for
        // comprehensions. The original list is left untouched either way
        // (D-107's leak-only policy: no incref/decref of `base_ptr`, and no
        // special handling for the new result beyond what
        // `pycc_rt_int_list_slice` itself already does).
        MirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let base_ptr = expect_list_pointer(base_scalar, "the sliced value");

            let start_i64 = match start {
                Some(e) => {
                    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
                    let encoded = to_numeric_encoded_int(context, builder, scalar);
                    build_untag_checked(builder, rt, encoded, "slice_untag_start")
                }
                None => context.i64_type().const_int(0, false),
            };
            let stop_raw = match stop {
                Some(e) => {
                    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
                    let encoded = to_numeric_encoded_int(context, builder, scalar);
                    Some(build_untag_checked(
                        builder,
                        rt,
                        encoded,
                        "slice_untag_stop",
                    ))
                }
                None => None,
            };
            let step_i64 = match step {
                Some(e) => {
                    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
                    let encoded = to_numeric_encoded_int(context, builder, scalar);
                    build_untag_checked(builder, rt, encoded, "slice_untag_step")
                }
                None => context.i64_type().const_int(1, false),
            };
            // Read only now: after `base` and every present bound have
            // already been evaluated (see this arm's own doc comment
            // above).
            let stop_i64 = match stop_raw {
                Some(v) => v,
                None => build_int_list_len(builder, rt, base_ptr),
            };

            let result_ptr =
                build_int_list_slice(builder, rt, base_ptr, start_i64, stop_i64, step_i64);
            Scalar::List(result_ptr)
        }
        // `list.pop()` (PR-12 Task 11, D-119): removes and returns `list`'s
        // own last element. Mirrors `MirExpr::ListAppend`'s own shape (a
        // plain-name base read through `emit_list_name_read` and a direct
        // encoded-value return, exactly like `MirExpr::Subscript` and
        // `MirExpr::DictGet`) rather than `MirExpr::Slice`'s -- there
        // is no sub-expression to recursively evaluate here at all, unlike
        // `Slice`'s bounds.
        //
        // Safe under repeated self-reference within one statement (e.g.
        // `xs = [xs.pop(), xs.pop()]`, this task's own brief flags this
        // shape): `emit_list_name_read` always re-reads `xs`'s *current*
        // slot value, which does not change mid-statement -- `MirStmt::
        // Assign`'s own arm (see that arm's own call site) evaluates this
        // whole right-hand-side expression to a finished `Scalar` *before*
        // ever rebinding `xs`'s slot, exactly the ordering `MirExpr::Slice`'s
        // own doc comment above already establishes is what keeps `xs =
        // xs[1:3]` safe from Task 5a's confirmed self-referential-rebind
        // regression. Each `.pop()` mutates the *pointee* `PyIntListObj`
        // in place (via `pycc_rt_int_list_pop`'s own `Cell<Vec<i64>>`), not
        // `xs`'s own slot, so two `.pop()`s emitted in source order run in
        // that same order at runtime -- ordinary sequential `call`
        // instructions, nothing deferred -- and each observes the
        // previous one's mutation, matching CPython's own left-to-right
        // evaluation of `[xs.pop(), xs.pop()]`. Verified empirically, not
        // just by this reasoning: see the crate's `tests` module and its
        // `pop_twice_on_the_same_list_in_one_statement_removes_in_order`
        // end-to-end test.
        MirExpr::ListPop { list, .. } => {
            let list_ptr =
                emit_list_receiver(context, builder, module, rt, user_functions, locals, list);
            let encoded = build_int_list_pop(builder, rt, list_ptr);
            Scalar::Int(encoded)
        }
        // `dict.get(key, default)` (PR-12 Task 11, D-119): returns the
        // stored value, or `default` if `key` is absent -- never panics on
        // a missing key, unlike `MirExpr::DictGet`'s own `d[key]`. `dict`
        // is a plain variable name or (#1263) an attribute read, resolved
        // through `emit_dict_receiver` first; `key` and `default` are both
        // arbitrary sub-expressions, evaluated after it and left to right,
        // matching Python's own receiver-then-arguments evaluation order.
        //
        // `key`'s `Scalar::Str` is extracted with **no**
        // `incref_if_str_duplicate` call, unlike `MirStmt::DictSet`'s own
        // key -- following `MirExpr::DictGet`'s own no-incref precedent
        // above, not `DictSet`'s: a read-only lookup never stores the key
        // pointer anywhere persistent, so there is no new owning reference
        // to protect (D-124's incref requirement exists only where a
        // pointer is *adopted*, e.g. `pycc_rt_dict_set`'s own key
        // parameter).
        //
        // `default` is read-only too -- unlike `key`, it never reaches
        // `pycc_rt` as a pointer (a dict value is an encoded `i64`), so no
        // incref question arises for it at all. Safe under
        // nested self-reference within one statement (e.g. `d =
        // d.get("k", d.get("j", 0))`, this task's own brief flags this
        // shape): both the outer and inner `.get()` read `d`'s slot via
        // `emit_dict_name_read`, and neither `.get()` call ever mutates
        // `d`'s pointee (`pycc_rt_dict_get_or_default` is a pure lookup, no
        // `Cell::set` on any success or failure path) -- so, unlike
        // `ListPop`'s own mutating case just above, there is no ordering
        // hazard to reason about here at all: every read observes the same
        // unchanged dict. Verified empirically: see the crate's `tests`
        // module and its
        // `dict_get_or_default_nested_in_its_own_default_argument_resolves_correctly`
        // end-to-end test.
        MirExpr::DictGetOrDefault {
            dict, key, default, ..
        } => {
            let dict_ptr =
                emit_dict_receiver(context, builder, module, rt, user_functions, locals, dict);
            let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
            let Scalar::Str(key_ptr) = key_scalar else {
                panic!(
                    "pycc_codegen: internal error: dict.get() key did not evaluate to str -- \
                     pycc_types::check (T0021) should have rejected this before codegen"
                )
            };
            let default_scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                default,
            );
            let encoded_default = to_encoded_int(context, builder, default_scalar);
            let _ = build_untag_checked(builder, rt, encoded_default, "dict_get_validate_default");
            let encoded =
                build_dict_get_or_default(builder, rt, dict_ptr, key_ptr, encoded_default);
            Scalar::Int(encoded)
        }
        // `set.add(value)` (PR-12 Task 11, D-119): mirrors `MirExpr::
        // ListAppend`'s own shape exactly (a plain-name base read through
        // `emit_set_name_read`, D-141 validation on `value`, the canonical
        // `Ty::None` `i8 0` result
        // carrier) -- `set` is read only once, `build_int_set_add` is
        // `MirExpr::SetLiteral`'s own already-existing helper (this is
        // its second call site, not a new declaration), and the dedup
        // check that makes a repeated `.add()` of an already-present value
        // a no-op lives entirely inside `pycc_rt_int_set_add` itself, not
        // here (see that function's own doc comment). A separate
        // `s.add(x)` statement whose value is already a member needs no
        // special reasoning at all: each `.add()` is its own fully
        // independent `MirStmt`, executed strictly in source order, each
        // re-reading `s`'s current
        // (unchanged-by-`.add()`-itself-except-for-dedup-mutation) pointer
        // -- verified empirically: see the crate's `tests` module and its
        // `set_add_grows_the_set_and_a_repeated_value_still_dedups_codegens_and_runs`
        // end-to-end test.
        MirExpr::SetAdd { set, value, ops } => {
            let set_ptr =
                emit_set_name_read(context, builder, module, rt, user_functions, locals, set);
            if let Some(ops) = ops {
                // An instance element (#1343): `set_instance.rs` drives the
                // user `__hash__`/`__eq__`.
                let emitter = SetEmitter {
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                };
                emitter.add(set_ptr, value, ops);
                return Scalar::Bool(context.i8_type().const_int(0, false));
            }
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let encoded = to_encoded_int(context, builder, value_scalar);
            let _ = build_untag_checked(builder, rt, encoded, "set_validate_added");
            build_int_set_add(builder, rt, set_ptr, encoded);
            Scalar::Bool(context.i8_type().const_int(0, false))
        }
        // `ClassName(args)` (D-154, Part 1 of #375): allocate a fresh
        // instance whose slots are all unassigned (`pycc_rt_instance_new`,
        // given the class's own already-resolved slot count and, since
        // #1388, its interned layout descriptor), then call the
        // mangled `__init__` with that pointer as `self`, followed by
        // `args` -- see `MirExpr::Instantiate`'s own doc comment for why
        // this needs `build_call_to_with_leading_args` rather than the
        // ordinary `build_call_to` every ordinary/method call above uses.
        MirExpr::Instantiate(inst) => {
            let pycc_mir::InstantiateExpr {
                ctor,
                class_name,
                slot_names,
                args,
                ..
            } = inst.as_ref();
            let count = context.i64_type().const_int(slot_names.len() as u64, false);
            let (layout, layout_len) =
                attr_slot::instance_layout_constant(context, module, class_name, slot_names);
            let instance_ptr = builder
                .build_call(
                    rt.instance_new,
                    &[count.into(), layout.into(), layout_len.into()],
                    "instance_new",
                )
                .expect("build_call should not fail for a well-formed instance allocation")
                .try_as_basic_value()
                .expect_basic("pycc_rt_instance_new returns a non-void pointer")
                .into_pointer_value();
            let ctor_function = user_functions.get(ctor.as_str()).unwrap_or_else(|| {
                panic!(
                    "pycc_codegen: internal error: constructor `{ctor}` should have been \
                     registered as an ordinary user function -- pycc_hir::class::lower_class \
                     mangles every method, including `__init__`, into HirModule::items"
                )
            });
            build_call_to_with_leading_args(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                ctor_function,
                ctor,
                &[instance_ptr.into()],
                args,
            );
            Scalar::Instance(instance_ptr)
        }
        // `base.attr` (D-154, Part 1 of #375): read the raw slot word via
        // the opaque `pycc_rt_instance_get_slot_checked` accessor (never a
        // direct `GEP`, per the class-instance-layout ADR) -- which raises
        // `AttributeError` for a slot not yet assigned (#1388), the raise
        // `expression_can_set_exception` guards -- then reinterpret it as
        // the attribute's own declared `Ty` -- see `slot_word_to_scalar`'s
        // own doc comment for the conversion and its own reachable-`Ty`
        // scope.
        MirExpr::AttrGet { base, slot, ty } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let base_ptr = expect_instance_pointer(base_scalar, "attribute access base");
            let slot_index = context.i64_type().const_int(*slot as u64, false);
            let raw = builder
                .build_call(
                    rt.instance_get_slot_checked,
                    &[base_ptr.into(), slot_index.into()],
                    "instance_get_slot",
                )
                .expect("build_call should not fail for a well-formed attribute read")
                .try_as_basic_value()
                .expect_basic("pycc_rt_instance_get_slot_checked returns a non-void i64")
                .into_int_value();
            slot_word_to_scalar(context, builder, raw, ty)
        }
        // Part 2 of #1026: the string-keyed runtime sibling of the
        // compile-time-slot `AttrGet` directly above. `foreign_attr::emit`
        // carries the whole contract, including the `NULL` check it emits,
        // the foreign failure edge that check branches to (`foreign_fail.rs`),
        // and which
        // side owns the "CPython raised" transition.
        MirExpr::ObjAttrGet { base, attr, .. } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_attr::emit(context, builder, module, rt, base_scalar, attr);
            held.release(builder, rt);
            result
        }
        // Part 2 of #1026 (PR 2b of #1081): the call counterpart of
        // `ObjAttrGet` directly above. The order below is CPython's own --
        // base, then the callable, then each argument left to right -- and
        // resolving the callable *here*, rather than inside the call shim,
        // is the whole reason `foreign_call` exposes two functions:
        // `obj.missing(1 // 0)` must raise `AttributeError`, not
        // `ZeroDivisionError`. `foreign_call` carries the rest of the
        // contract, including the argument marshalling, the ownership rule,
        // and the two `NULL` checks that route a failure to the
        // foreign failure edge (`foreign_fail.rs`).
        MirExpr::ObjMethodCall {
            base, method, args, ..
        } => foreign_call_emit::emit_method_call(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            base,
            method,
            args,
        ),
        // #1313: `callee(args)` on an `object`-typed name (a foreign
        // binding or a `for` loop target), or since Part 2a of #1371 on an
        // object subscript result (`table[k](args)`). CPython's order -- the
        // callee, then each argument left to right -- and no lookup step.
        // `foreign_call::emit_object_call` decides from the callee's MIR
        // shape whether it is a borrow (a name read; the shim helper takes
        // its own reference) or a produced new reference the call consumes.
        MirExpr::ObjCall { callee, args } => foreign_call_emit::emit_direct_call(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            callee,
            args,
        ),
        // Part 8 of #1371: either call above with keyword arguments.
        // `foreign_call_emit` carries the order and the ownership split.
        MirExpr::ObjKeywordCall(call) => foreign_call_emit::emit_keyword_call(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            call,
        ),
        // Part 3 of #1026 (PR 3a of #1082): `len(o)`. `foreign_len` carries
        // the contract -- why the D-141 encode happens inside the shim
        // rather than here, why that leaves exactly one failure edge, and
        // why the out-parameter's `alloca` is hoisted to the entry block.
        MirExpr::ObjLen { base } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_len::emit_len(context, builder, module, rt, base_scalar);
            held.release(builder, rt);
            result
        }
        // Part 11 of #1371: `type(o)`. `foreign_type` carries the contract.
        MirExpr::ObjType { base } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_type::emit_type(context, builder, module, rt, base_scalar);
            held.release(builder, rt);
            result
        }
        // Part 3 of #1026 (PR 3b of #1082): `o[k]`. The evaluation order
        // below is CPython's own -- base, then key -- and
        // `foreign_call::emit_subscript` carries the rest of the contract,
        // including why the packed key needs no `NULL` check of its own and
        // why that leaves exactly one foreign failure edge.
        MirExpr::ObjSubscript { base, index } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held_base = object_release::hold(context, module, rt, base, &base_scalar);
            let index_scalar = object_unbox::emit_pack_operand(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                index,
            );
            let held_index = object_release::hold(context, module, rt, index, &index_scalar);
            let result = foreign_call::emit_subscript(
                context,
                builder,
                module,
                rt,
                base_scalar,
                index_scalar,
            );
            held_index.release(builder, rt);
            held_base.release(builder, rt);
            result
        }
        // Part 1 of #1371: a comparison or identity test with a CPython
        // object operand. Left, then right -- CPython's own order -- with
        // the `Compare` arm's #638 protection of an `int` temporary across
        // the right operand's evaluation; a `None` literal operand of an
        // identity test is not evaluated at all (`foreign_compare` names
        // CPython's `None` instead). `foreign_compare` carries the rest.
        MirExpr::ObjCompare { op, left, right } => {
            let operand = |expr: &MirExpr| {
                (!matches!(expr, MirExpr::NoneLiteral)).then(|| {
                    object_unbox::emit_pack_operand(
                        context,
                        builder,
                        module,
                        rt,
                        user_functions,
                        locals,
                        expr,
                    )
                })
            };
            let hold = |expr: &MirExpr, scalar: Option<Scalar<'ctx>>| {
                scalar.map(|scalar| object_release::hold(context, module, rt, expr, &scalar))
            };
            let l = operand(left);
            let held_l = hold(left, l);
            let pending_l =
                l.and_then(|l| push_pending_int_release_if_scalar_temporary(rt, left, &l));
            let r = operand(right);
            let held_r = hold(right, r);
            pop_pending_int_release(rt, pending_l);
            let result = foreign_compare::emit_compare(context, builder, module, rt, *op, l, r);
            for held in [held_r, held_l].into_iter().flatten() {
                held.release(builder, rt);
            }
            for (expr, scalar) in [(left, l), (right, r)] {
                if let Some(scalar) = scalar {
                    release_scalar_if_int_temporary(context, builder, rt, expr, &scalar);
                }
            }
            result
        }
        // Part 2b of #1371: `item in container`. Item, then container --
        // CPython's own order -- with the `ObjCompare` arm's #638
        // protection of an `int` temporary item across the container's
        // evaluation. `foreign_compare::emit_contains` carries the rest.
        MirExpr::ObjContains {
            negate,
            item,
            container,
        } => {
            let item_scalar = object_unbox::emit_pack_operand(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                item,
            );
            let held_item = object_release::hold(context, module, rt, item, &item_scalar);
            let pending = push_pending_int_release_if_scalar_temporary(rt, item, &item_scalar);
            let container_scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                container,
            );
            let held_container =
                object_release::hold(context, module, rt, container, &container_scalar);
            pop_pending_int_release(rt, pending);
            let result = foreign_compare::emit_contains(
                context,
                builder,
                module,
                rt,
                *negate,
                item_scalar,
                container_scalar,
            );
            held_container.release(builder, rt);
            held_item.release(builder, rt);
            release_scalar_if_int_temporary(context, builder, rt, item, &item_scalar);
            result
        }
        // Part 2b of #1371: `o[a:b:c]`. `foreign_slice` owns the operand
        // order, the `int`-temporary protection and the call.
        MirExpr::ObjSlice {
            base,
            start,
            stop,
            step,
        } => foreign_slice::emit_with_operands(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            |base, bounds| foreign_slice::emit_slice(context, builder, module, rt, base, bounds),
        ),
        // Part 2d of #1371: a list display bound to an object slot. Each
        // element in source order, every evaluated `int` temporary protected
        // across the evaluations after it and released after the call, the
        // `ObjSlice` arm's discipline. `foreign_call::emit_list` carries the
        // rest.
        MirExpr::ObjList { elements } => {
            let mut pendings = Vec::with_capacity(elements.len());
            let mut scalars = Vec::with_capacity(elements.len());
            let mut holds = Vec::with_capacity(elements.len());
            for element in elements {
                let scalar = object_unbox::emit_pack_operand(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    element,
                );
                holds.push(object_release::hold(context, module, rt, element, &scalar));
                pendings.push(push_pending_int_release_if_scalar_temporary(
                    rt, element, &scalar,
                ));
                scalars.push(scalar);
            }
            for pending in pendings.into_iter().rev() {
                pop_pending_int_release(rt, pending);
            }
            let result = foreign_call::emit_list(context, builder, module, rt, &scalars);
            for held in holds.into_iter().rev() {
                held.release(builder, rt);
            }
            for (element, scalar) in elements.iter().zip(&scalars) {
                release_scalar_if_int_temporary(context, builder, rt, element, scalar);
            }
            result
        }
        MirExpr::ObjIsInstance { value, class } => {
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let held_value = object_release::hold(context, module, rt, value, &value_scalar);
            let mut held_class = None;
            let class = match class {
                pycc_mir::ObjIsInstanceClass::Object(class) => {
                    let scalar =
                        emit_expr(context, builder, module, rt, user_functions, locals, class);
                    held_class = Some(object_release::hold(context, module, rt, class, &scalar));
                    foreign_compare::IsInstanceClass::Object(scalar)
                }
                pycc_mir::ObjIsInstanceClass::Builtin(builtin) => {
                    foreign_compare::IsInstanceClass::Builtin(builtin.shim_code())
                }
                pycc_mir::ObjIsInstanceClass::Compiled(name) => {
                    foreign_compare::IsInstanceClass::Compiled(name.clone())
                }
            };
            let result =
                foreign_compare::emit_isinstance(context, builder, module, rt, value_scalar, class);
            if let Some(held) = held_class {
                held.release(builder, rt);
            }
            held_value.release(builder, rt);
            result
        }
        // Part 2 of #1027: `b[i]` on a `pycc build --ext` export's
        // `memoryview` parameter. Base then index, CPython's own order and
        // the list-subscript arm's own, and then a single call: the bounds
        // check, the D-173 `IndexError` raise and the element read all live
        // inside `pycc_rt_buffer_f64_get`. No GEP and no comparison is
        // emitted here, deliberately -- an inline check would be IR this
        // crate's own tests could only observe as text, where the runtime
        // helper is directly unit-testable in `pycc_rt`.
        //
        // The index is decoded exactly as a list index is: `Ty::Int` (or
        // `bool`, per D-086) arrives D-141 encoded, and `build_untag_checked`
        // is what rejects a bigint or malformed word. The raise this node
        // can leave pending is declared by `expression_can_set_exception`,
        // which emits the guard that reads it.
        MirExpr::BufferGet { base, index } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let Scalar::MemoryView(base_ptr) = base_scalar else {
                panic!(
                    "pycc_codegen: internal error: a buffer element load's base did not evaluate \
                     to a memoryview"
                )
            };
            let index_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, index);
            let encoded_index = to_numeric_encoded_int(context, builder, index_scalar);
            let raw_index = build_untag_checked(builder, rt, encoded_index, "buffer_untag_index");
            let element = builder
                .build_call(
                    rt.buffer_f64_get,
                    &[base_ptr.into(), raw_index.into()],
                    "buffer_get",
                )
                .expect("build_call should not fail for a declared runtime function")
                .try_as_basic_value()
                .expect_basic("pycc_rt_buffer_f64_get returns a non-void f64")
                .into_float_value();
            Scalar::Float(element)
        }
        // #1116: `len(b)`. One call and no arithmetic, exactly as the load
        // above -- and no D-173 exception check after it, because
        // `pycc_rt_buffer_len` cannot raise (`expression_can_set_exception`
        // answers `false` for this node, so no guard block is emitted).
        MirExpr::BufferLen { base } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let Scalar::MemoryView(base_ptr) = base_scalar else {
                panic!(
                    "pycc_codegen: internal error: a buffer length read's base did not evaluate \
                     to a memoryview"
                )
            };
            let raw_len = builder
                .build_call(rt.buffer_len, &[base_ptr.into()], "buffer_len")
                .expect("build_call should not fail for a declared runtime function")
                .try_as_basic_value()
                .expect_basic("pycc_rt_buffer_len returns a non-void i64")
                .into_int_value();
            // D-141: the raw element count becomes a user-visible `Ty::Int`
            // expression value here, so it is tagged exactly as the scalar
            // `len` arm tags `pycc_rt_int_list_len`'s own raw count.
            Scalar::Int(raw_i64_to_tagged_int(context, builder, raw_len))
        }
        // #1165: `ndarray(n)` / `NDArray(n)`. One call and no arithmetic,
        // exactly as the two arms above -- `pycc_rt` owns the zero-fill, the
        // negative-length refusal and the allocator pairing.
        //
        // `Ty::Int` (or `bool`, per D-086) arrives D-141 encoded, so the
        // length is decoded before the allocator sees it -- but *not* the
        // way a buffer index is. `build_untag_checked` calls
        // `pycc_rt_int_untag_checked`, which `panic!`s on a bigint or
        // malformed word, and that panic is turned into a process abort at
        // its own `extern "C"` boundary: for a D-244 `ext` artifact, an
        // abort of the *host* CPython interpreter, which no host code can
        // catch. This arm therefore decodes through
        // `build_buffer_alloc_untag_len` instead, whose runtime function
        // raises `OverflowError` (D-173) and returns the type-valid
        // sentinel `0`. The
        // D-173 raise a negative length leaves pending is declared by
        // `expression_can_set_exception`, which emits the guard that reads
        // it; the null view the refusal returns is safe for the epilogue,
        // whose free is a documented no-op on null. A second guard is emitted
        // *before* the allocator call, for an exception that was already
        // pending when this arm was reached -- see it in place below.
        MirExpr::FrozenSetFrom { source } => frozenset::emit_frozenset_from(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            source.as_deref(),
        ),
        MirExpr::BufferAlloc { len } => {
            let len_scalar = emit_expr(context, builder, module, rt, user_functions, locals, len);
            let encoded_len = to_numeric_encoded_int(context, builder, len_scalar);
            let raw_len =
                build_buffer_alloc_untag_len(builder, rt, encoded_len, "buffer_untag_alloc_len");
            // #1166 round 11, finding 1: retire the length's own birth
            // reference here, unconditionally, before anything below can
            // branch away.
            //
            // `n + 1` with `n == 2 ** 62 - 1` evaluates to a freshly owned
            // `BigIntObj`, and `to_numeric_encoded_int` passes that pointer
            // through unchanged. Every edge out of this arm that is reached
            // *after* the decode either abandons the value or consumes it:
            //
            //   * the decoder's own `OverflowError` on a bigint or malformed
            //     word -- the reviewer's case, and the only one a real
            //     program reaches with a live bigint, since promotion is the
            //     only way a bigint length exists here at all;
            //   * an exception already pending when this arm was reached,
            //     which the guard below short-circuits on;
            //   * the allocator's own negative-length `ValueError` and
            //     unreservable-length `RuntimeError`, both raised after
            //     `raw_len` is computed;
            //   * and the ordinary fallthrough.
            //
            // Retiring the reference *before* the guard is what covers the
            // first two as well as the last two: a release emitted after
            // `guard_statement_effects` would be skipped by exactly the
            // edges that leak. Repeatedly catching the `OverflowError`
            // leaked one `BigIntObj` per call in the host process before
            // this call existed.
            //
            // Unconditional rather than exception-edge-only, and a bare
            // release rather than a `push_pending_int_release_if_temporary`:
            // `raw_len` (or the decoder's sentinel `0`) is the only thing
            // read below, so the word is dead on *every* edge -- and
            // `rt.exceptions.pending_int_releases` is documented to carry an
            // *enclosing* node's operand, "never this call site's own
            // operand" (see `guard_statement_effects` and each push site).
            // Pushing this arm's own operand there would have the unwind
            // block release a word this arm must also retire on the normal
            // path, which is a double release rather than a leak.
            //
            // `release_if_int_temporary`'s own classification keeps a
            // borrowed length (`a = ndarray(n)`, where `n` names the value)
            // untouched, so only a birth reference is retired.
            release_if_int_temporary(context, builder, rt, len, encoded_len);
            // #1166 review finding F5: check the pending state *before*
            // allocating, not only after.
            //
            // `emit_expr`'s own guard runs after `emit_expr_unchecked`
            // returns, so an exception already pending when this arm is
            // reached -- set by an operation `expression_can_set_exception`
            // classifies `false` and therefore leaves unguarded, such as the
            // `MirExpr::ListPop` in `a = ndarray(xs.pop())` -- allowed the
            // allocation to succeed and *then* branched to the handler,
            // before `MirStmt::Assign` could store the pointer into the
            // frame's owned slot. The slot kept its entry null, so neither
            // the unwind nor the epilogue freed the view: one leaked buffer
            // per call, which `pycc_rt_buffer_live_views` counts.
            //
            // Guarding here instead short-circuits to the same exception
            // target without allocating, which is what makes the arm's
            // "a refused allocation leaves nothing for the unwind path to
            // release" claim hold for a *stale* pending exception as well as
            // for the allocator's own negative-length raise. It is also what
            // makes `build_buffer_alloc_untag_len` above safe: that decoder's
            // bigint refusal sets the pending state and returns the sentinel
            // `0`, and `0` is a *valid* length, so without this guard the
            // allocator would succeed on it and produce a zero-length view
            // that `MirStmt::Assign` never gets to store -- the same leak
            // shape as the stale-exception case above, reached through the
            // decoder instead. `decode_inline_or_raise`'s own contract
            // requires exactly this: return the sentinel before reaching any
            // further `raise_builtin`, since `raise_builtin` installs
            // unconditionally and would otherwise report over this raise.
            guard_statement_effects(context, builder, rt);
            let view = builder
                .build_call(rt.buffer_f64_alloc, &[raw_len.into()], "buffer_alloc")
                .expect("build_call should not fail for a declared runtime function")
                .try_as_basic_value()
                .expect_basic("pycc_rt_buffer_f64_alloc returns a non-void pointer")
                .into_pointer_value();
            // The identical `Scalar::MemoryView` a `memoryview` parameter
            // carries (see the parameter's own binding site), which is what
            // lets `BufferGet`, `BufferLen` and `MirStmt::BufferSet` operate
            // on an owned buffer with no arm of their own.
            Scalar::MemoryView(view)
        }
        MirExpr::InstanceHash { operand, via } => hash::emit_instance_hash(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            operand,
            *via,
        ),
        // Part 4 of #1026 (PR 4c of #1083): `x: tuple[float, float, float] =
        // o` at module scope, and the same at any other fixed arity -- the
        // PEP 585 variadic `tuple[float, ...]` stays refused and never
        // reaches here. `foreign_len::emit_unpack_float_tuple` carries the
        // contract -- why the out-slot is an array rather than a scalar,
        // why the struct is reassembled here with `build_insert_value`, and
        // why the arity travels as a call argument.
        MirExpr::ObjUnpackFloatTuple { base, arity } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_len::emit_unpack_float_tuple(
                context,
                builder,
                module,
                rt,
                base_scalar,
                *arity,
            );
            held.release(builder, rt);
            result
        }
        MirExpr::ObjUnpack { value, arity } => {
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let held = object_release::hold(context, module, rt, value, &value_scalar);
            let result =
                foreign_unpack::emit_unpack(context, builder, module, rt, value_scalar, *arity);
            held.release(builder, rt);
            result
        }
        MirExpr::NullInstance { .. } => {
            let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
            let null_ptr = ptr_type.const_null();
            Scalar::Instance(null_ptr)
        }
        // Part 3A of #541 (#736): `pycc_mir::class::rewrite_exception_to_message`
        // is this node's sole constructor, applied only to an exception-typed
        // expression -- `base` therefore always evaluates to a
        // `Scalar::Instance` pointer to a live `PyExceptionObj`.
        MirExpr::ExceptionMessage(base) => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            emit_exception_message(builder, rt, base_scalar)
        }
        // #1337 (WI-6a): `isinstance` on a caught builtin exception value.
        MirExpr::ExceptionTypeTest { obj, tags } => {
            let obj_scalar = emit_expr(context, builder, module, rt, user_functions, locals, obj);
            exception_isinstance::emit_exception_type_test(context, builder, rt, obj_scalar, tags)
        }
        // PEP 572 (#774): `target := value`. `name`'s storage slot is
        // already predeclared by `collect_expr_bindings` (this node's own
        // slot-scanning counterpart to `collect_stmt_bindings`'s
        // `MirStmt::Assign` handling), exactly the way `MirStmt::Assign`'s
        // own target is predeclared -- so this mirrors that statement's own
        // codegen (`emit_stmt`'s `MirStmt::Assign` arm) almost exactly:
        // evaluate `value`, retain it if it is a borrowed bigint duplicate
        // (T0050 restricts a walrus's value to non-reference-counted scalar
        // types, so `incref_if_str_duplicate` never applies here -- there is
        // no `str`/container/instance case to retain), then store via the
        // same `emit_assign` this statement uses. Unlike a plain assignment
        // statement, this node must also yield a value -- routed through a
        // synthetic `MirExpr::Name` read of the slot it just stored into
        // (mirroring `emit_list_name_read`'s own "round-trip through the
        // `Name` arm" precedent just below), so the result gets the exact
        // same load/definite-assignment/refcount-borrow treatment an
        // ordinary later read of `name` would get -- which is also why
        // `bigint_rc::int_value_is_a_duplicate_reference` classifies
        // `NamedExpr { ty: Ty::Int, .. }` as borrowed exactly like `Name`:
        // the value this arm yields is a second read of the same slot, not
        // a second owned reference.
        MirExpr::NamedExpr { name, value, ty } => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
            emit_assign(context, builder, rt, locals, name, scalar);
            emit_expr_unchecked(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                &MirExpr::Name {
                    name: name.clone(),
                    ty: ty.clone(),
                },
            )
        }
    }
}

/// Resolves the receiver of `MirExpr::ListAppend`/`ListPop` to its list
/// pointer (#1263): a bare name through [`emit_list_name_read`], exactly as
/// before, and an attribute receiver (a slot read or a `@property` getter
/// call) through an ordinary `emit_expr`. Containers are leak-only (D-107,
/// D-124), so the pointer an attribute read yields needs no retain or
/// release around the mutation.
#[allow(clippy::too_many_arguments)]
fn emit_list_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    receiver: &pycc_mir::MirContainerReceiver,
) -> PointerValue<'ctx> {
    match receiver {
        pycc_mir::MirContainerReceiver::Name(name) => {
            emit_list_name_read(context, builder, module, rt, user_functions, locals, name)
        }
        pycc_mir::MirContainerReceiver::Attr(read) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, read);
            expect_list_pointer(scalar, "an attribute container receiver")
        }
    }
}

/// The `dict` counterpart of [`emit_list_receiver`], for
/// `MirExpr::DictGetOrDefault`'s receiver (#1263).
#[allow(clippy::too_many_arguments)]
fn emit_dict_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    receiver: &pycc_mir::MirContainerReceiver,
) -> PointerValue<'ctx> {
    match receiver {
        pycc_mir::MirContainerReceiver::Name(name) => {
            emit_dict_name_read(context, builder, module, rt, user_functions, locals, name)
        }
        pycc_mir::MirContainerReceiver::Attr(read) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, read);
            expect_dict_pointer(scalar, "an attribute container receiver")
        }
    }
}

/// Reads a `list[T]`-typed local by name. A bare-name `MirExpr::ListAppend`/
/// `ListPop` receiver and `MirStmt::ForList`'s `list` both carry their list
/// as a plain variable name rather than a sub-expression (mirroring
/// `HirExpr::ListAppend`/`HirStmt::ForList`, D-105), so neither has a
/// `MirExpr` to hand to `emit_expr` directly.
///
/// Routes the read through `emit_expr`'s own `Name` arm (via a synthetic
/// `MirExpr::Name` carrying the slot's own recorded type) rather than
/// loading `slot.ptr` here, so these two reads get exactly the same
/// definite-assignment guard every other name read gets -- without it,
/// `if flag: xs = [1]` followed by an unconditional `xs.append(2)` would
/// dereference an uninitialized slot instead of trapping. Unlike
/// `emit_string_literal`'s own documented removal of a synthetic-`MirExpr`
/// round-trip, this one introduces no permanently-uncoverable arm: the
/// non-list case goes through the shared `expect_list_pointer` above, and a
/// non-list local is exactly the naturally reachable way to cover it (see
/// this file's own `appending_to_a_non_list_local_is_an_internal_error`).
#[allow(clippy::too_many_arguments)]
fn emit_list_name_read<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    name: &str,
) -> PointerValue<'ctx> {
    // `let ... else`, not the `unwrap_or_else(|| panic!(..))` shape
    // `emit_expr`'s own `Name` arm uses for the same lookup: a panicking
    // closure compiles to its own function record, which `cargo llvm-cov`
    // reports as an "Unexecuted instantiation" in whichever crate
    // instantiation never reaches it (observed here for the copy the
    // integration-test binaries link, which has no reason to construct an
    // unbound list name). Writing the check inline keeps its counts inside
    // the enclosing function, where they merge across instantiations.
    let Some(slot) = locals.get(name) else {
        panic!("pycc_codegen: internal error: `{name}` has no local slot")
    };
    let ty = slot.ty.clone();
    let scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &MirExpr::Name {
            name: name.to_string(),
            ty,
        },
    );
    expect_list_pointer(scalar, &format!("`{name}`"))
}

/// Reads a `dict[K, V]`-typed local by name. `MirStmt::DictSet`'s `dict` and
/// `MirStmt::ForDict`'s `dict` both carry their dict as a plain variable
/// name rather than a sub-expression (mirroring `HirStmt::DictSet`/
/// `HirStmt::ForList`'s dict-typed case, D-123), so neither has a `MirExpr`
/// to hand to `emit_expr` directly. Mirrors `emit_list_name_read` exactly,
/// for the identical reason (see that function's own doc comment,
/// including its `let ... else` shape over `unwrap_or_else(|| panic!(..))`).
#[allow(clippy::too_many_arguments)]
fn emit_dict_name_read<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    name: &str,
) -> PointerValue<'ctx> {
    let Some(slot) = locals.get(name) else {
        panic!("pycc_codegen: internal error: `{name}` has no local slot")
    };
    let ty = slot.ty.clone();
    let scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &MirExpr::Name {
            name: name.to_string(),
            ty,
        },
    );
    expect_dict_pointer(scalar, &format!("`{name}`"))
}

/// Reads a `set[T]`-typed local by name. `MirStmt::ForSet`'s `set` carries
/// its set as a plain variable name rather than a sub-expression (mirroring
/// `HirStmt::ForList`'s set-typed case, D-123), so it has no `MirExpr` to
/// hand to `emit_expr` directly. Mirrors `emit_list_name_read`/
/// `emit_dict_name_read` exactly, for the identical reason (see
/// `emit_list_name_read`'s own doc comment, including its `let ... else`
/// shape over `unwrap_or_else(|| panic!(..))`).
#[allow(clippy::too_many_arguments)]
fn emit_set_name_read<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    name: &str,
) -> PointerValue<'ctx> {
    let Some(slot) = locals.get(name) else {
        panic!("pycc_codegen: internal error: `{name}` has no local slot")
    };
    let ty = slot.ty.clone();
    let scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &MirExpr::Name {
            name: name.to_string(),
            ty,
        },
    );
    expect_set_pointer(scalar, &format!("`{name}`"))
}

/// Evaluates every entry in `args` (via `emit_expr`, so each argument is
/// itself an arbitrary expression -- nested calls included, which is
/// exactly what makes recursion with real arguments work) and emits the
/// `call` instruction to the already-resolved `f`. Shared between
/// `emit_expr`'s `Call` arm (a value-producing call used inside a larger
/// expression) and `emit_stmt`'s void-call arm below (a call whose
/// declared return type is `None`, used as a bare statement). A
/// `None`-returning call has no LLVM return value; `emit_expr` separately
/// materializes its canonical unit carrier only when a surrounding
/// expression needs one. This is the one piece both call sites need
/// regardless of whether a value comes back afterward.
///
/// Deliberately does *not* also do the `user_functions` lookup for
/// `callee`: the two call sites disagree on how a missing function should
/// be reported. `emit_stmt`'s arm still has a `Result` to propagate a
/// clean, user-facing error through (matching this crate's pre-Task-5
/// behavior, just generalized from zero-arg-only to any arity); `emit_expr`
/// does not (see its own `Call` arm's comment) -- so each resolves `f`
/// itself, with its own error-handling policy, before calling this helper.
#[allow(clippy::too_many_arguments)]
fn build_call_to<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    callee_name: &str,
    args: &[MirExpr],
) -> inkwell::values::CallSiteValue<'ctx> {
    let user_function = &user_functions[callee_name];
    build_call_to_with_leading_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        user_function,
        callee_name,
        &[],
        args,
    )
}

/// Generalizes [`build_call_to`] with a `leading_args` prefix -- values
/// already computed by the caller (never marshalled through `emit_expr`,
/// since they have no `MirExpr` of their own) that go first in the emitted
/// `call`'s argument list, before `args`' own marshalled values. Used by
/// `MirExpr::Instantiate`'s codegen (D-154, Part 1 of #375) to pass the
/// freshly allocated instance pointer as `__init__`'s own `self` argument --
/// `build_call_to`'s ordinary path has no `MirExpr` to build that pointer
/// from, since it is a codegen-only value with no HIR/MIR node of its own.
/// `leading_args` is skipped when zipping `args` against
/// `user_function.param_tys`, so `args[0]` still lines up with the *first
/// non-leading* declared parameter type, exactly like `build_call_to`'s own
/// contract for every ordinary call (`leading_args` empty).
#[allow(clippy::too_many_arguments)]
fn build_call_to_with_leading_args<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    user_function: &UserFunction<'ctx>,
    _callee_name: &str,
    leading_args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    args: &[MirExpr],
) -> inkwell::values::CallSiteValue<'ctx> {
    let mut arg_values: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = leading_args.to_vec();
    // #638 (D-208): a fresh (non-duplicate) `Ty::Int` argument's ownership
    // *transfers* to the callee's parameter slot on the normal path --
    // there is no release call for it anywhere in this function, unlike
    // every other affected site in the inventory. If a later sibling
    // argument's own evaluation raises before `build_call`/
    // `build_indirect_call` below is ever reached, that transfer never
    // happens and the reference is orphaned. `mark` records the stack
    // depth before this loop so the truncate below is recursion-safe: an
    // argument expression can itself contain a nested call whose own
    // marshalling loop pushes and pops its own arguments' words while
    // evaluating this loop's own argument, and this loop's earlier
    // pending entries must stay untouched by that nested truncation.
    let mark = rt.exceptions.pending_int_releases.borrow().len();
    let marshalled_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args
        .iter()
        .zip(&user_function.param_tys[leading_args.len()..])
        .map(|(a, param_ty)| {
            // Part 2 of #1387: a native argument to an `object` parameter
            // is boxed (`object_box`). The packer borrows the value and the
            // box is a new `PyObject *` reference, so none of the
            // `str`/`int` ownership steps below applies to it.
            if object_box::boxes_into(a, param_ty) {
                return object_box::emit_boxed(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    a,
                )
                .into();
            }
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, a);
            let scalar = incref_if_str_duplicate(builder, rt, a, scalar);
            let scalar = retain_if_int_duplicate_and_track_for_exception_edge(
                context, builder, rt, a, scalar,
            );
            // Push *after* the retain-and-track call: `int_temporary_word`
            // (via `push_pending_int_release_if_scalar_temporary`)
            // excludes a duplicate reference by construction, so this is
            // a no-op for that case regardless of ordering -- the
            // retain-and-track call above already pushed a *duplicate*
            // argument's retained word itself, if any. #834 is closed: a
            // borrowed/duplicate argument's extra retain is now protected
            // on this exception edge exactly like an owning argument's own
            // word already was, not merely D-180 residual 3 (which covers
            // a retain that eventually transfers to a real owner, not one
            // abandoned before transfer completes). See
            // https://github.com/rotnov/pycc/issues/834.
            push_pending_int_release_if_scalar_temporary(rt, a, &scalar);
            let scalar = coerce_scalar_to_type(context, builder, scalar, param_ty.clone());
            match scalar {
                Scalar::Int(v) => v.into(),
                Scalar::Bool(v) => v.into(),
                Scalar::Float(v) => v.into(),
                Scalar::Str(v) => v.into(),
                // Pass-through, identical to `Str`'s arm directly above: a
                // `list[T]` parameter is an opaque pointer at the ABI
                // level exactly like a `str` one (`ty_to_basic_type` gives
                // both the same LLVM type), so argument marshalling needs
                // no list-specific handling at all.
                Scalar::List(v) => v.into(),
                // Pass-through, identical to `List`'s arm directly above: a
                // `dict[K, V]` parameter is an opaque pointer at the ABI
                // level exactly like a `str`/`list[T]` one
                // (`ty_to_basic_type` gives all three the same LLVM type),
                // so argument marshalling needs no dict-specific handling
                // at all.
                Scalar::Dict(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s arms
                // directly above: a `set[T]` parameter is an opaque pointer
                // at the ABI level exactly like a `str`/`list[T]`/
                // `dict[K, V]` one (`ty_to_basic_type` gives all four the
                // same LLVM type), so argument marshalling needs no
                // set-specific handling at all.
                Scalar::Set(v) => v.into(),
                // Pass-through like the three arms above, but by VALUE
                // rather than by pointer (D-115): a `tuple[...]` argument
                // is an LLVM struct at the ABI level, not an opaque
                // pointer. It still needs no tuple-specific marshalling
                // code, because `BasicMetadataValueEnum` already has a
                // `StructValue` arm -- the calling convention's own
                // by-value aggregate handling is LLVM's job, not this
                // crate's.
                Scalar::Tuple(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s/`Set`'s arms
                // above: a class-instance parameter (D-154, Part 1 of #375
                // -- `self`, or any other class-typed parameter a future PR
                // might add) is an opaque pointer at the ABI level exactly
                // like a `str`/`list[T]`/`dict[K, V]`/`set[T]` one.
                Scalar::Instance(v) => v.into(),
                // Pass-through by VALUE, identical in kind to `Tuple`'s arm
                // above (D-197, #763, Part 1 of #747): an `Optional[int]`
                // argument is an LLVM `{ i64, i8 }` struct at the ABI level
                // -- `coerce_scalar_to_type` immediately above already
                // built it against `param_ty`, so this arm needs no
                // further conversion, only the same `Into` `BasicMetadataValueEnum`
                // has for any `StructValue`.
                Scalar::Optional(v) => v.into(),
                // Pass-through (Part 1 of #1333): a `Ty::Object` argument
                // reaches an unannotated private helper's solver-inferred
                // `object` parameter or, since Part 1 of #1367, a parameter
                // annotated with a class a foreign import binds -- either
                // way the callee is another pycc function taking the same
                // `ptr` (a public one called from the host gets the object
                // through its `--ext` thunk instead, #1397/#1386). The
                // borrowed pointer is passed through with no refcount
                // change: the callee never releases it (#1092's leak-only
                // rule, `docs/RUNTIME.md`).
                Scalar::Object(v) => v.into(),
                // Defensive (Part 2 of #1027): passing `b` to another
                // function is a bare read of the name, which
                // `reject_memoryview_read` refuses with `C0001` for a
                // wrapper-borrowed parameter and
                // `owned_buffer_use_unsupported` for the artifact-owned
                // storage of Part 2a of #1142 (#1165). A `memoryview`
                // parameter exists only on an `--ext` export, which CPython
                // calls, never compiled code.
                Scalar::MemoryView(_) => {
                    panic!(
                        "pycc_codegen: internal error: a memoryview argument is not supported \
                         yet -- pycc_types should have refused this before codegen"
                    )
                }
            }
        })
        .collect();
    // #638 (D-208): every argument above evaluated successfully -- this
    // Rust-level line is reached unconditionally at codegen (emission)
    // time regardless of what the *compiled* program's exception-edge
    // control flow does, since `guard_statement_effects` only emits a
    // runtime branch instruction, never diverts the emitter itself (see
    // `guard_statement_effects`'s own doc comment). Truncate back to
    // `mark` *without* releasing: ownership of every entry pushed above
    // has now transferred to the callee's parameter slots via the
    // `build_call`/`build_indirect_call` below.
    rt.exceptions
        .pending_int_releases
        .borrow_mut()
        .truncate(mark);
    arg_values.extend(marshalled_args);
    // Issue #22: dispatch indirectly through the function-pointer slot.
    // Load the current binding; if null, the function hasn't been defined
    // yet at this point in execution -- abort with a runtime NameError.
    // Otherwise, call through the loaded pointer.
    // Monomorphized generic specializations (`0gen_...` names) have no
    // `fn_ptr_global` -- they dispatch directly through `direct_value`
    // since they are compiler-generated, not user-defined.
    if let Some(ref direct_value) = user_function.direct_value {
        return builder
            .build_call(*direct_value, &arg_values, "call_user_fn")
            .expect("build_call should not fail for a well-formed direct call");
    }
    // The load, the null check and its `NameError` path are shared with
    // #1050's `ext` export thunks, which dispatch through the same slot:
    // see `ext_thunk::emit_fnptr_dispatch_guard`. It returns with the
    // builder positioned on the non-null path.
    let fn_ptr = ext_thunk::emit_fnptr_dispatch_guard(context, builder, rt, user_function);
    builder
        .build_indirect_call(user_function.fn_type, fn_ptr, &arg_values, "call_user_fn")
        .expect("build_indirect_call should not fail for a well-formed indirect call")
}

/// Turns any supported `Scalar` into an LLVM `i1` for use as a `br`
/// condition -- the shared truthiness check behind `if`/`while` (Task 4),
/// now including `str` (Task 7): `False` only for the empty string,
/// delegated to `pycc_rt_str_truthy` (D-059's representation is opaque to
/// this crate).
/// `module` is threaded in for the `Scalar::Object` arm alone (Part 3 of
/// #1026, PR 3a of #1082): that arm declares and calls the shim's
/// `pycc_ext_obj_truthy`, and inkwell 0.9's `FunctionValue` exposes no
/// `get_parent`, so the LLVM module cannot be recovered from the builder's
/// own insertion point the way the enclosing *function* can.
fn truthy<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> inkwell::values::IntValue<'ctx> {
    let as_i8 = match scalar {
        Scalar::Bool(v) => v,
        Scalar::Int(v) => builder
            .build_call(rt.int_truthy, &[v.into()], "int_truthy")
            .expect("build_call should not fail for a well-formed truthiness check")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_truthy returns a non-void i8")
            .into_int_value(),
        // Python's `bool(x)` for a `float` is `False` only for exactly
        // `0.0`/`-0.0` -- `NaN` is truthy -- so this needs the
        // *unordered*-or-not-equal predicate, `UNE`, not `ONE` (same
        // distinction as the `Compare` arm's `NotEq` predicate above).
        Scalar::Float(v) => {
            let zero = context.f64_type().const_float(0.0);
            let cond = builder
                .build_float_compare(FloatPredicate::UNE, v, zero, "float_truthy")
                .expect("build_float_compare should not fail for two f64 operands");
            builder
                .build_int_z_extend(cond, context.i8_type(), "bool_from_float_truthy")
                .expect("build_int_z_extend should not fail widening i1 to i8")
        }
        Scalar::Str(v) => builder
            .build_call(rt.str_truthy, &[v.into()], "str_truthy")
            .expect("build_call should not fail for a well-formed truthiness check")
            .try_as_basic_value()
            .expect_basic("pycc_rt_str_truthy returns a non-void i8")
            .into_int_value(),
        // A real, reachable feature gap rather than a defensive arm
        // (D-107): `pycc_types` places no type restriction on an `if`/
        // `while` condition, so `if xs:` for a `list[int]` local
        // type-checks today and lands here. v0.2 has no `bool(list)`
        // semantics (D-105 ships only `len(x)`/`x[i]`/iteration/
        // `.append()`), and there is no `pycc_rt_int_list_truthy` to call
        // -- so this panics honestly instead of calling
        // `pycc_rt_str_truthy` on a `PyIntListObj` pointer, whose layout
        // has nothing in common with `PyStrObj`'s.
        Scalar::List(_) => {
            panic!("pycc_codegen: truthiness of a list[T] value is not supported yet")
        }
        // A real, reachable feature gap, identical in kind to the `List`
        // arm directly above: `pycc_types` places no type restriction on an
        // `if`/`while` condition, so `if x:` for a `dict[str, int]` local
        // type-checks today and lands here. v0.2 has no `bool(dict)`
        // semantics (D-123 ships only `len(x)`/`x[k]`/`x[k] = v`/
        // iteration), and there is no `pycc_rt_dict_truthy` to call -- so
        // this panics honestly instead of calling `pycc_rt_str_truthy` on
        // a `PyDictObj` pointer, whose layout has nothing in common with
        // `PyStrObj`'s.
        Scalar::Dict(_) => {
            panic!("pycc_codegen: truthiness of a dict[K, V] value is not supported yet")
        }
        // Part 1 of #1319: a `set[int]`/`frozenset[int]` value is truthy
        // exactly when it is non-empty, which its runtime length answers.
        Scalar::Set(ptr) => frozenset::set_truthy(context, builder, rt, ptr),
        // A real, reachable feature gap -- but NOT "identical in kind" to
        // the panicking `List`/`Dict` arms above in one respect (the `Set`
        // arm stopped panicking when Part 1 of #1319 gave it real
        // truthiness): `list[T]`'s own `if xs:`/`while xs:` reachability
        // predates this whole PR-11 effort entirely (established back in
        // PR-10, D-107); `dict`'s own reachability, while more recent (PR-11a's own
        // HIR literal lowering), was already in place before this PR
        // (PR-11b) started -- neither is something this PR's own diff
        // turned from a clean diagnostic into a panic. `tuple[...]`'s
        // reachability here IS exactly that: it is new as of this PR's own
        // Task 2 (`HirExpr::TupleLiteral` lowering, `crates/pycc_hir/src/
        // lib.rs`) -- before that commit, any program containing a tuple
        // literal failed to lower at all and got a clean `C0001`
        // ("expression kind not supported yet") diagnostic instead of ever
        // reaching this function. `pycc_types` places no type restriction
        // on an `if`/`while` condition, so `if t:` for a `tuple[...]` local
        // type-checks today and lands here. D-116 ships only construction
        // and literal-index reads, so v0.2 has no tuple truthiness
        // semantics -- and CPython's own rule (a tuple is falsey only when
        // empty) is not derivable from this representation for free
        // anyway, since D-116 admits no empty tuple in the first place.
        // Panics honestly rather than guessing. See the D-116
        // deferred-capability list in
        // `docs/decisions/D-116-tuple-v0-2-scope-int-bool-float-elements-only.md`
        // and `docs/ROADMAP.md`'s matching follow-up for this
        // new-as-of-PR-11b reachability.
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: truthiness of a tuple[...] value is not supported yet")
        }
        // Unlike every container arm above, a class instance's truthiness
        // (D-154, Part 1 of #375) needs no runtime call and no honest-panic
        // deferral: this PR ships no `__bool__`/`__len__` (both explicitly
        // out of scope), so CPython's own default-object rule applies
        // unconditionally -- `bool(x)` is `True` for any instance with
        // neither, with no data to inspect and no way for it to ever be
        // `False`. A plain constant `1` correctly implements that rule
        // rather than deferring it.
        Scalar::Instance(_) => context.i8_type().const_int(1, false),
        // `Optional[int | float | bool]` (D-197, #763, Part 1 of #747;
        // widened to `float`/`bool` by #809): unlike every
        // container/instance arm above, this one is real rather than a
        // deferred panic -- CPython's `bool(x)` for `x: T | None` is
        // `False` for `None` and otherwise `bool(<the T value>)` (so a
        // present-but-zero/false payload is still falsy), and both halves
        // are directly readable from this representation: field 1 is the
        // present/absent flag, and field 0 (when present) is a payload
        // whose own truthiness this function already knows how to compute
        // for every inner type `T0049` admits -- `Ty::Int`'s D-141-encoded
        // `int` via `pycc_rt_int_truthy` (`Scalar::Int`'s own arm above),
        // `Ty::Float`'s plain `f64` via the *unordered*-not-equal-to-zero
        // compare (`Scalar::Float`'s own arm above, mirrored exactly
        // below), and `Ty::Bool`'s plain `i8` `0`/`1`, which already *is*
        // its own truthiness with no conversion needed (`Scalar::Bool`'s
        // own arm above).
        //
        // This function receives no `Ty` for the inner type -- `Scalar::
        // Optional` carries only the LLVM struct value -- so the inner
        // type is recovered from field 0's own LLVM type instead, which
        // `ty_to_basic_type`'s `Ty::Optional` arm makes unambiguous per
        // inner type: `f64` for `Ty::Float`, `i64` for `Ty::Int`, `i8` for
        // `Ty::Bool`. Every producer of a `Scalar::Optional` reaching this
        // function (`coerce_scalar_to_type`, `OptionalWrap`'s codegen, a
        // `Name` read of an `Optional`-typed local) already builds the
        // struct with the target's own inner-typed payload, so this
        // dispatch is exact, not a heuristic.
        Scalar::Optional(v) => {
            let present = builder
                .build_extract_value(v, 1, "opt_present")
                .expect("build_extract_value should not fail reading field 1 of a 2-field struct")
                .into_int_value();
            let payload = builder
                .build_extract_value(v, 0, "opt_payload")
                .expect("build_extract_value should not fail reading field 0 of a 2-field struct");
            let payload_truthy = match payload {
                inkwell::values::BasicValueEnum::FloatValue(fv) => {
                    // `Ty::Float` inner: mirrors `Scalar::Float`'s own arm
                    // above exactly (same `UNE` predicate, same
                    // zero-extend), rather than inventing a new mechanism.
                    let zero = context.f64_type().const_float(0.0);
                    let cond = builder
                        .build_float_compare(
                            FloatPredicate::UNE,
                            fv,
                            zero,
                            "opt_payload_float_truthy",
                        )
                        .expect("build_float_compare should not fail for two f64 operands");
                    builder
                        .build_int_z_extend(
                            cond,
                            context.i8_type(),
                            "opt_payload_bool_from_float_truthy",
                        )
                        .expect("build_int_z_extend should not fail widening i1 to i8")
                }
                // `i8`-width payload: `Ty::Bool` inner. Already the exact
                // `0`/`1` truthiness value -- `Scalar::Bool`'s own arm
                // above (`Scalar::Bool(v) => v`) treats a bare bool
                // identically, with no runtime call.
                inkwell::values::BasicValueEnum::IntValue(iv)
                    if iv.get_type().get_bit_width() == 8 =>
                {
                    iv
                }
                // Any other int width: `Ty::Int` inner, a D-141-encoded
                // `i64` -- the pre-existing behavior, delegated to
                // `pycc_rt_int_truthy` exactly like `Scalar::Int`'s own arm
                // above.
                inkwell::values::BasicValueEnum::IntValue(iv) => builder
                    .build_call(rt.int_truthy, &[iv.into()], "opt_payload_truthy")
                    .expect("build_call should not fail for a well-formed truthiness check")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_int_truthy returns a non-void i8")
                    .into_int_value(),
                // Deliberately printing the payload's *type* rather than the
                // payload value itself (e.g. via `{other:?}`, which reaches
                // inkwell's `Value::Debug` impl and calls
                // `LLVMPrintValueToString`): either printer returns an
                // `inkwell::support::LLVMString`, whose `Drop` impl calls
                // `LLVMDisposeMessage` and crashes on Windows for this LLVM
                // release (D-029) unless routed through
                // `llvm_string_to_owned`. Printing the type instead of the
                // value additionally avoids handing the printer a value
                // freshly extracted from a constant struct inside a
                // function whose block is not yet terminated (see PR #812).
                other => panic!(
                    "pycc_codegen: internal error: an Optional[_] payload had an unsupported LLVM representation ({}) -- pycc_types::check (T0049) should have rejected this inner type before codegen",
                    llvm_string_to_owned(other.get_type().print_to_string())
                ),
            };
            builder
                .build_and(present, payload_truthy, "opt_truthy")
                .expect("build_and should not fail for two i8 operands")
        }
        // D-244, Part 3 of #1026 (PR 3a of #1082): CPython's own `bool(x)`
        // for an arbitrary object consults `__bool__`/`__len__`, which is
        // arbitrary Python code, so this delegates to the shim's
        // `pycc_ext_obj_truthy` -- and takes `foreign_fail.rs`'s failure
        // edge when that code raises. This arm is what closed Part 2's R11 hole:
        // `pycc_types` used to refuse a `Ty::Object` condition at ten
        // condition-position sites *because* there was no answer here, and
        // PR 3a deleted all ten (`crates/pycc_types/src/foreign.rs`).
        Scalar::Object(ptr) => {
            let bit = foreign_len::emit_truthy(context, builder, module, rt, ptr);
            builder
                .build_int_z_extend(bit, context.i8_type(), "bool_from_object_truthy")
                .expect("build_int_z_extend should not fail widening i1 to i8")
        }
        // Defensive, not a capability gap (Part 2 of #1027): `if b:` is a
        // bare read of a `memoryview`-bound name, refused with `C0001` by
        // `reject_memoryview_read` for a wrapper-borrowed parameter and by
        // `owned_buffer_use_unsupported` for the artifact-owned storage of
        // Part 2a of #1142 (#1165). CPython would answer `len(b) != 0`;
        // `len(b)` is implemented (#1116) but the bare read is not, so this
        // arm stays unreachable.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: truthiness of a memoryview value is not supported yet")
        }
    };
    builder
        .build_int_compare(
            IntPredicate::NE,
            as_i8,
            context.i8_type().const_int(0, false),
            "truthy",
        )
        .expect("build_int_compare should not fail comparing two i8 operands")
}

/// #382 (PR-22 Part 2): If the current block's terminator is an
/// `unreachable` (placed by `MirStmt::Raise`/`RaiseFrom`/`Reraise`),
/// erase it so the caller can append an exception check (conditional
/// branch) in this block. Returns `true` if an `unreachable` was erased
/// (meaning a raise happened and the exception is active), `false`
/// otherwise. After erasing, the block has no terminator and the caller
/// can build its exception-check conditional branch normally.
fn erase_unreachable_if_present<'ctx>(builder: &inkwell::builder::Builder<'ctx>) -> bool {
    if let Some(term) = builder.get_insert_block().unwrap().get_terminator()
        && term.get_opcode() == inkwell::values::InstructionOpcode::Unreachable
    {
        term.erase_from_basic_block();
        return true;
    }
    false
}

/// Runs `build` with the builder positioned at the *top* of `function`'s
/// entry block, then puts the builder back at the end of the block it was
/// emitting into, and returns whatever `build` produced.
///
/// This is the one place an expression that needs its own stack slot hoists
/// the `alloca`: an `alloca` is reclaimed only when its function returns, so
/// one emitted inside a loop grows the stack without bound, and LLVM's own
/// convention is that every `alloca` belongs in the entry block. The top of
/// the block, not its end, because by the time an expression is emitted the
/// entry block may already have been terminated -- a comprehension after an
/// `if`, a `while` or a `try` (#1254) is emitted into a later block, and
/// appending after the entry block's branch would be invalid IR. An entry
/// block with no instructions yet is appended to instead.
///
/// `foreign_call.rs`'s `alloca_in_entry_block` and `foreign_len.rs`'s
/// `out_slot_in_entry_block` build on this for their argument arrays and
/// out-slots; `comprehension.rs` uses it for an expression comprehension's
/// loop-variable slot.
fn build_at_entry_block<'ctx, T>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: inkwell::values::FunctionValue<'ctx>,
    build: impl FnOnce(&inkwell::builder::Builder<'ctx>) -> T,
) -> T {
    let resume_at = builder
        .get_insert_block()
        .expect("the builder is positioned inside a block");
    let entry_block = function
        .get_first_basic_block()
        .expect("a function being emitted into has an entry block");
    match entry_block.get_first_instruction() {
        Some(first) => builder.position_before(&first),
        None => builder.position_at_end(entry_block),
    }
    let value = build(builder);
    builder.position_at_end(resume_at);
    value
}

/// Allocates a local slot in the current function's entry block, which
/// dominates every branch, loop, and merge that may read it. String slots are
/// initialized to null and `int` slots to the word `0` (#146 Part 1), so the
/// first emitted decref/release is a safe no-op rather than a read of
/// uninitialized stack. `0` is deliberately *not* a valid encoded int -- it
/// is `classify_encoded_int`'s fail-closed pattern -- which is exactly why
/// `pycc_rt_bigint_release` returns on it before classifying. A guarded
/// non-parameter slot also receives an `i8` initialized flag, because #118's
/// static definite-assignment joins have not landed yet and a syntactically
/// present assignment may not execute at runtime.
fn storage_slot_at_entry<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ty: pycc_mir::Ty,
    name: &str,
    guard_reads: bool,
) -> StorageSlot<'ctx> {
    let current_block = builder.get_insert_block().expect(
        "builder is always positioned inside some block while a statement is being emitted",
    );
    let function = current_block
        .get_parent()
        .expect("the block builder is currently positioned in always belongs to a function");
    let entry_block = function.get_first_basic_block().expect(
        "compile_to_object always appends a function's entry block before emitting its body",
    );
    builder.position_at_end(entry_block);
    let ptr = builder
        .build_alloca(ty_to_basic_type(context, ty.clone()), name)
        .expect("build_alloca should not fail for a supported local type");
    if ty == pycc_mir::Ty::Str || ty == pycc_mir::Ty::MemoryView {
        // Part 2a of #1142 (#1165) joins `Ty::Str` on this arm rather than
        // adding one of its own: both are owning pointer slots released on
        // the way out, and both runtime releases are documented no-ops on
        // null. Without the null-init a `MemoryView` slot holds garbage, and
        // the epilogue's free and the first free-before-overwrite store
        // would each read uninitialized alloca memory. Gating those on the
        // `initialized` flag below is *not* an alternative: that flag guards
        // reads, and the very first store happens before any read.
        builder
            .build_store(
                ptr,
                context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null(),
            )
            .expect("build_store should not fail immediately after this function's own alloca");
    } else if ty == pycc_mir::Ty::Int {
        builder
            .build_store(ptr, context.i64_type().const_zero())
            .expect("build_store should not fail immediately after this function's own alloca");
    } else if matches!(&ty, pycc_mir::Ty::Optional(inner) if **inner == pycc_mir::Ty::Int) {
        // #770 review: mirrors the `Ty::Int` zero-init immediately above,
        // but with the same valid-payload placeholder
        // `default_value_for_type`'s own `Ty::Optional(_)` arm builds
        // (`{ tag_smallint_const(0), 0 }`, never a raw `{0, 0}` struct) --
        // required so `release_optional_int_slot_before_store` can release
        // this slot's payload unconditionally, including at the very first
        // store, without ever reading uninitialized alloca memory or
        // tripping `classify_encoded_int`'s fail-closed panic on a raw
        // zero word.
        let placeholder = default_value_for_type(context, ty.clone());
        builder
            .build_store(ptr, placeholder)
            .expect("build_store should not fail immediately after this function's own alloca");
    }
    let initialized = if guard_reads {
        let initialized_ptr = builder
            .build_alloca(context.i8_type(), &format!("{name}_initialized"))
            .expect("build_alloca should not fail for an i8 initialization flag");
        builder
            .build_store(initialized_ptr, context.i8_type().const_zero())
            .expect("build_store should not fail for a fresh initialization flag");
        Some(initialized_ptr)
    } else {
        None
    };
    builder.position_at_end(current_block);
    StorageSlot {
        ptr,
        ty,
        initialized,
    }
}

/// Reuses the predeclared slot backing `target`. The slot's established type
/// wins over the current expression type, including the accepted
/// `bool`-to-`int` assignment that must store a D-141 encoded i64 rather than raw i8.
///
/// #146 Part 1: an `Ty::Int` slot releases its previous word here, before
/// the store, for *every* caller rather than at each call site
/// individually. `str`'s own release deliberately stays at `MirStmt::
/// Assign` (`decref_str_slot_before_store`): only a small subset of this
/// function's callers bind a `str`, whereas thirteen of them bind an `int`
/// (the four `range`-induction binds, the eight container-element binds,
/// and ordinary assignment), and a per-site release is exactly the shape
/// where one missed site ships a silent leak that no value assertion can
/// observe. Centralizing it here makes "released before every int store" a
/// property of the function rather than of an audit.
///
/// The release is gated on the *slot's* declared type, never on the value's:
/// `x: int = True` reaches here as a `Scalar::Bool` that
/// `coerce_scalar_to_type` (below) turns into a D-141 encoded word, and
/// value-type gating would skip the release for exactly that case.
// PEP 572 (#774): loosened from `&mut HashMap` to `&HashMap`. The body only
// ever reads the map (`locals.get(target)`), never inserts or removes a key
// -- the actual mutation this function performs is a `build_store` into the
// slot's own already-allocated pointer, not a change to the map of slots
// itself. Every pre-existing call site sits inside `emit_stmt`, which holds
// `locals: &mut HashMap<...>`; passing that through to a `&HashMap`
// parameter is an ordinary reborrow coercion, so this loosening is
// behavior-preserving for all of them. It is required, not merely tidier,
// for `MirExpr::NamedExpr`'s own codegen (`emit_expr_unchecked`'s new arm
// below): that function only ever holds a read-only `locals: &HashMap<...>`
// (expression lowering never needs to declare a *new* slot, only read
// existing ones), yet the walrus's "evaluate value, store to the bound
// local" behavior needs exactly this store logic to run mid-expression.
fn emit_assign<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: Scalar<'ctx>,
) {
    let slot = locals
        .get(target)
        .cloned()
        .expect("every assignment target must have a predeclared storage slot");
    if slot.ty == pycc_mir::Ty::Int {
        release_int_slot_before_store(context, builder, rt, &slot);
    } else if matches!(&slot.ty, pycc_mir::Ty::Optional(inner) if **inner == pycc_mir::Ty::Int) {
        // #770 review: the release-side half of the retain
        // `MirExpr::OptionalWrap`'s codegen now performs on a borrowed
        // `int` payload -- without this, that retain would be a permanent
        // per-reassignment leak of the slot's previous payload instead of
        // a balanced reference (D-197's only supported `Optional[T]`
        // payload is `int`, so this is the only `Optional` shape that can
        // ever hold a bigint word to release).
        release_optional_int_slot_before_store(context, builder, rt, &slot);
    }
    let value = coerce_scalar_to_type(context, builder, value, slot.ty);
    let basic_value: inkwell::values::BasicValueEnum = match value {
        Scalar::Int(v) => v.into(),
        Scalar::Bool(v) => v.into(),
        Scalar::Float(v) => v.into(),
        Scalar::Str(v) => v.into(),
        // Pass-through, identical to `Str`'s arm directly above: storing a
        // `list[T]` value is storing one opaque pointer into a slot
        // `ty_to_basic_type` already allocated as a pointer. No refcount
        // traffic accompanies it -- D-107 keeps `list[T]` leak-only for
        // v0.2, so unlike `Str` there is deliberately no incref here and
        // no `decref_str_slot_before_store` counterpart, and unlike
        // `Ty::Int` the slot-type guard above skips this arm entirely.
        Scalar::List(v) => v.into(),
        // Pass-through, identical to `List`'s arm directly above: storing
        // a `dict[K, V]` value is storing one opaque pointer into a slot
        // `ty_to_basic_type` already allocated as a pointer. No refcount
        // traffic accompanies it -- D-124 keeps `dict[K, V]` leak-only for
        // v0.2 (extending D-107's exact reasoning), so unlike `Str` there
        // is deliberately no incref here and no
        // `decref_str_slot_before_store` counterpart.
        Scalar::Dict(v) => v.into(),
        // Pass-through, identical to `List`'s/`Dict`'s arms directly
        // above: storing a `set[T]` value is storing one opaque pointer
        // into a slot `ty_to_basic_type` already allocated as a pointer.
        // No refcount traffic accompanies it -- D-124 keeps `set[T]`
        // leak-only for v0.2 (extending D-107's exact reasoning),
        // identically to `List`/`Dict`.
        Scalar::Set(v) => v.into(),
        // Pass-through like the three arms above, but storing a whole
        // struct by value rather than one opaque pointer (D-115) -- into a
        // slot `ty_to_basic_type` already allocated at exactly that struct
        // type, so the store is type-correct without any tuple-specific
        // code at the `build_store` call below. The store itself emits no
        // refcount traffic, but that is no longer because a tuple holds
        // nothing worth counting: under D-182 a tuple field can hold an
        // ingress-retained reference to a borrowed `int` element, so
        // overwriting this slot drops the struct without releasing those
        // fields. That is the accepted, documented leak -- a slot-death
        // release for tuple fields is deferred to D-124's container
        // refcounting, not simply absent because there is nothing to free.
        Scalar::Tuple(v) => v.into(),
        // Pass-through, identical to `List`'s/`Dict`'s/`Set`'s arms above
        // (D-154, Part 1 of #375): storing a class-instance value is
        // storing one opaque pointer into a slot `ty_to_basic_type`
        // already allocated as a pointer. No refcount traffic accompanies
        // it -- `pycc_rt::instance` is leak-only, mirroring `List`/`Dict`/
        // `Set` (see that module's own doc comment).
        Scalar::Instance(v) => v.into(),
        // Pass-through by VALUE, identical in kind to `Tuple`'s arm above
        // (D-197, #763, Part 1 of #747): `coerce_scalar_to_type` above
        // already built the correctly-typed `{ int, i8 }` struct against
        // `slot.ty`, so this arm needs only the store, exactly like
        // `Tuple`'s own struct-by-value store above -- no refcount traffic
        // accompanies it either, for the identical D-182-acknowledged
        // reason `Tuple`'s own comment already gives.
        Scalar::Optional(v) => v.into(),
        // A pass-through since #1325: storing one `PyObject *` into a slot
        // `ty_to_basic_type` already allocated as a pointer, with no
        // refcount traffic. Since Part 1 of #1499 only a *frame* slot -- a
        // function local, reached since Part 1 of #1333 (#1362) -- stores
        // here: `MirStmt::Assign` routes a module-global `object` target
        // through `object_slot::assign`, which owns its reference. A frame
        // slot keeps #1092's leak-only rule, where a rebinding overwrites
        // and leaks the previous reference. A release here would be a
        // use-after-free, not a fix: a parameter or a local alias holds a
        // borrowed pointer without an incref, so freeing the old value
        // could free an object the caller still names. Part 2 (#1502)
        // gives frame slots the ownership model.
        Scalar::Object(v) => v.into(),
        // A pass-through since Part 2a of #1142 (#1165), where it was a
        // panic (as `Object`'s arm above was until #1325): storing one
        // `PyccExtBufferView *` into a slot `ty_to_basic_type` already
        // allocated as a pointer.
        //
        // The value is always artifact-owned storage, never the host's. The
        // two shapes that would put a *borrowed* view here are both refused
        // before codegen: `c = b` is a bare read of a buffer parameter
        // (`reject_memoryview_read`'s `C0001`), and `b = ndarray(n)` on a
        // parameter name is `buffer_parameter_rebinding`'s. What makes the
        // store safe rather than a leak is the pair around it --
        // `free_buffer_slot_before_store` releases whatever the slot already
        // held (D-074), and the frame's owned-slot epilogue releases this one
        // on every return path -- so unlike `List`/`Dict`/`Set` above, this
        // arm is deliberately *not* on the D-107 leak-only precedent.
        Scalar::MemoryView(v) => v.into(),
    };
    builder
        .build_store(slot.ptr, basic_value)
        .expect("build_store should not fail for a slot this function itself allocated");
    if let Some(initialized_ptr) = slot.initialized {
        builder
            .build_store(initialized_ptr, context.i8_type().const_int(1, false))
            .expect("build_store should not fail for a declared global flag");
    }
}

/// Mirror of [`decref_str_slot_before_store`] for an artifact-owned buffer
/// slot (Part 2a of #1142, #1165): loads the target's predeclared slot and
/// frees its current storage before the new view pointer overwrites it.
///
/// D-074's rule, applied to the second owning slot type this crate has.
/// Without it `a = ndarray(3); a = ndarray(4)` leaks the first allocation
/// and `for i in range(n): a = ndarray(3)` leaks `n - 1` of them -- an
/// unbounded leak inside someone else's process, which is why the D-107
/// leak precedent for `list`/`dict`/`set` is deliberately *not* followed
/// here. Buffer slots are null-initialized by `storage_slot_at_entry`, and
/// `pycc_rt_buffer_f64_free` is a documented no-op on null, so the same path
/// is correct for the first assignment and for every later one.
fn free_buffer_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
) {
    let slot = &locals[target];
    if slot.ty != pycc_mir::Ty::MemoryView {
        panic!(
            "pycc_codegen: internal error: buffer assignment target `{target}` has a non-buffer storage slot"
        );
    }
    let old = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            slot.ptr,
            "old_buffer",
        )
        .expect("build_load should not fail for this function's own alloca")
        .into_pointer_value();
    // Part 2b of #1142 (#1164): a rebinding that happens while a `return` of
    // this same pointer is pending must not release it -- the value is on
    // its way to the host. That is reachable: `try: return a` followed by
    // `finally: a = ndarray(n)` runs this store *after* the return statement
    // made `a`'s pointer pending, so an unguarded free here would hand the
    // host a view over storage this frame had already released.
    //
    // Skipping the free makes the pending slot the pointer's sole owner, so
    // the orphan flag is raised in the same breath; the frame's owned-slot
    // epilogue reads it to decide whether it must release the pending
    // pointer itself (when the return was abandoned by a raising finalizer)
    // or leave it to the host (when the return proceeded). Branch-free on
    // the `select` model the epilogue uses, so the emitted block keeps the
    // straight-line shape #1165 gave it.
    let release = match locals.get(PENDING_RETURN_BUFFER_KEY) {
        None => old,
        Some(pending) => {
            let pending_ptr = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    pending.ptr,
                    "pending_return_buffer",
                )
                .expect("build_load should not fail for the pending-return slot")
                .into_pointer_value();
            // Both halves are load-bearing. The equality alone is true for
            // the *first* allocation into a never-assigned slot, where `old`
            // and the pending record are both the entry null: that would
            // raise the orphan flag on a frame that never rebound anything,
            // and the epilogue would then release a pointer its own slot
            // loop had already released -- a double free on every
            // exception exit, which is what an unguarded equality actually
            // produced here before this conjunction was added.
            let is_live = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    old,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                    "buffer_store_old_is_live",
                )
                .expect("build_int_compare should not fail for two pointers");
            let is_same = builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    old,
                    pending_ptr,
                    "buffer_store_matches_pending",
                )
                .expect("build_int_compare should not fail for two pointers");
            let is_pending = builder
                .build_and(is_live, is_same, "buffer_store_is_pending")
                .expect("build_and should not fail for two i1 values");
            let orphaned = &locals[PENDING_RETURN_ORPHANED_KEY];
            let previously = builder
                .build_load(
                    context.i8_type(),
                    orphaned.ptr,
                    "pending_return_was_orphaned",
                )
                .expect("build_load should not fail for the pending-return orphan flag")
                .into_int_value();
            let raised = builder
                .build_select(
                    is_pending,
                    context.i8_type().const_int(1, false),
                    previously,
                    "pending_return_orphaned",
                )
                .expect("build_select should not fail for two i8 flags");
            builder
                .build_store(orphaned.ptr, raised)
                .expect("build_store should not fail for the pending-return orphan flag");
            builder
                .build_select(
                    is_pending,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                    old,
                    "buffer_free_old_release",
                )
                .expect("build_select should not fail for two pointers")
                .into_pointer_value()
        }
    };
    builder
        .build_call(rt.buffer_f64_free, &[release.into()], "buffer_free_old")
        .expect("build_call should not fail for a well-formed buffer free");
}

/// Emits every statement in `body` in order, stopping early the moment the
/// current block already ends in a terminator.
///
/// **History (Tasks 3/4 removed this exact check as unreachable, Task 5
/// re-adds it as their own doc comments predicted it eventually would
/// need to):** Task 3 (only `ExprStmt`/`Assign` existed) and Task 4 (which
/// added `If`/`While`/`ForRange`, every arm of which always finishes by
/// repositioning the builder at a fresh, never-yet-terminated continuation
/// block before returning) both proved this check was unreachable in their
/// own scope, confirmed empirically by `cargo llvm-cov`, and removed it
/// rather than carry dead code -- while flagging that a future `Return`
/// arm would be the first `MirStmt` shape whose codegen terminates the
/// *current* block without repositioning the builder afterward (nothing is
/// left to emit into). Task 5 adds exactly that `Return` arm, so a body
/// with a `Return` followed by further (legal, if dead, Python) statements
/// now really can leave this loop's *next* iteration trying to emit into
/// an already-terminated block -- without this check, that would build a
/// second terminator onto the same block, invalid LLVM IR that
/// `module.verify()` (correctly) rejects. `emit_body_then_branch` below
/// and `ForRange`'s own inline copy in `emit_stmt` need the exact same
/// reasoning applied to their own trailing branch, and both re-add their
/// own guard for the same reason (see each one's own doc comment).
#[allow(clippy::too_many_arguments)]
fn emit_body<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    for stmt in body {
        emit_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        if erase_unreachable_if_present(builder) {
            let exception_target = rt
                .exceptions
                .targets
                .borrow()
                .last()
                .copied()
                .expect("emit_body is always called inside an exception target");
            builder
                .build_unconditional_branch(exception_target)
                .expect("build_unconditional_branch should route an explicit raise");
            break;
        }
        if builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_some()
        {
            // A return already terminated this suite. Raising expressions
            // and complete try statements route themselves; explicit raise
            // uses the `unreachable` path handled directly above.
            break;
        }
    }
    Ok(())
}

/// Emits `body` (via `emit_body`, which may now leave the current block
/// already terminated -- see its own doc comment), then an unconditional
/// branch to `dest` *unless* the block is already terminated (a `Return`
/// reached inside `body`, in which case there is nothing left to branch
/// from and doing so anyway would build an invalid second terminator).
/// Used by `If`'s `then`/`else` arms and `While`'s body; `ForRange`'s body
/// needs its own variant inline (it has extra post-body work --
/// incrementing the loop variable -- to do before branching back to the
/// loop test), so does not reuse this helper, but re-adds the identical
/// guard around its own trailing branch for the same reason.
#[allow(clippy::too_many_arguments)]
fn emit_body_then_branch<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    dest: inkwell::basic_block::BasicBlock<'ctx>,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<bool, String> {
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty,
        finally_stack,
    )?;
    let falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if falls_through {
        builder
            .build_unconditional_branch(dest)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }
    Ok(falls_through)
}

fn declare_module_globals<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    bindings: &BTreeMap<String, pycc_mir::Ty>,
) -> BTreeMap<String, StorageSlot<'ctx>> {
    bindings
        .iter()
        .map(|(name, ty)| {
            let (storage_ty, initializer): (
                inkwell::types::BasicTypeEnum,
                inkwell::values::BasicValueEnum,
            ) = match ty {
                pycc_mir::Ty::Int => (
                    context.i64_type().into(),
                    tag_smallint_const(context, 0).into(),
                ),
                pycc_mir::Ty::Bool => (
                    context.i8_type().into(),
                    context.i8_type().const_zero().into(),
                ),
                // D-075's canonical unit carrier extends to ordinary
                // assignment storage: `None` has no payload, so an `i8 0`
                // is sufficient. The separate `initialized` flag created
                // below, not this zero initializer, distinguishes a binding
                // whose assignment has executed from an uninitialized one.
                pycc_mir::Ty::None => (
                    context.i8_type().into(),
                    context.i8_type().const_zero().into(),
                ),
                pycc_mir::Ty::Float => (
                    context.f64_type().into(),
                    context.f64_type().const_zero().into(),
                ),
                pycc_mir::Ty::Str => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage to `Ty::Str` directly above: an opaque
                // pointer, null until the first assignment stores a real
                // `PyIntListObj` into it, with the separate `initialized`
                // flag below (which every module global gets) trapping any
                // read that reaches it first.
                //
                // Task 5 (D-089) deliberately left this arm out and let a
                // module-level `list[int]` binding hit the catch-all below,
                // on the grounds that no real source could construct a list
                // value at all yet; its own report flagged the interaction
                // for Task 11 to re-derive. Task 11b makes `x = [1, 2, 3]`
                // constructible from real source, and D-105's first scope
                // cut names module scope as one of the two places a
                // `list[int]` value is expected to live -- so leaving it out
                // now would turn that documented, supported form into an
                // internal compiler panic. No exit-time decref accompanies
                // it (contrast the `Ty::Str` loop in `compile_to_object`):
                // D-107 keeps `list[T]` leak-only for v0.2.
                pycc_mir::Ty::List(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage and reasoning to `Ty::List(_)`
                // directly above (PR-11 Task 5): an opaque pointer, null
                // until the first assignment stores a real `PyDictObj`
                // into it, with the separate `initialized` flag below
                // trapping any read that reaches it first. D-123 names
                // module scope as one of the places a `dict[str, int]`
                // value is expected to live, and every one of this task's
                // own CLI repro programs assigns `x = {...}` at module
                // scope -- leaving this arm out would turn that documented,
                // supported form into an internal compiler panic. No
                // exit-time decref accompanies it (contrast the `Ty::Str`
                // loop in `compile_to_object`): D-124 keeps `dict[K, V]`
                // leak-only for v0.2, extending D-107's exact reasoning.
                pycc_mir::Ty::Dict(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage and reasoning to `Ty::List(_)`/
                // `Ty::Dict(_)` directly above (PR-11 Task 9): an opaque
                // pointer, null until the first assignment stores a real
                // `PyIntSetObj` into it, with the separate `initialized`
                // flag below trapping any read that reaches it first.
                // D-123 names module scope as one of the places a
                // `set[int]` value is expected to live, and every one of
                // this task's own CLI repro programs assigns `x = {...}` at
                // module scope -- leaving this arm out would turn that
                // documented, supported form into an internal compiler
                // panic. No exit-time decref accompanies it (contrast the
                // `Ty::Str` loop in `compile_to_object`): D-124 keeps
                // `set[T]` leak-only for v0.2, extending D-107's exact
                // reasoning.
                pycc_mir::Ty::Set(_) | pycc_mir::Ty::FrozenSet(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // `tuple[...]`'s own storage (PR-11b Task 5, D-115), and
                // the one arm here that is not a nullable pointer: the
                // struct type itself, stored inline in the global, zero-
                // initialized via `const_zero()` (which zero-fills every
                // field whatever the arity and field types -- `i64`/`i8`/
                // `f64` all have a well-defined zero).
                //
                // That zero is deliberately *not* a sentinel, unlike the
                // null pointer the four arms above use. A zeroed struct is
                // indistinguishable from a legitimately-zero tuple, so it
                // could never mark "unassigned" on its own. Nothing is
                // lost: read-before-first-assignment is trapped by the
                // separate `initialized` flag every module global already
                // gets just below -- which is the real guard for the
                // pointer arms too, the null merely being a redundant
                // second signal there. D-116 names module scope as a place
                // a tuple value is expected to live (`x = (1, 2)` at top
                // level is the canonical form), so omitting this arm would
                // turn that supported form into an internal compiler panic.
                // No exit-time decref accompanies it (contrast the
                // `Ty::Str` loop in `compile_to_object`): a tuple owns no
                // allocation to release.
                pycc_mir::Ty::Tuple(_) => {
                    let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
                    (struct_ty.into(), struct_ty.const_zero().into())
                }
                // Identical storage and reasoning to `Ty::List(_)`/
                // `Ty::Dict(_)`/`Ty::Set(_)` above (D-154, Part 1 of #375):
                // an opaque pointer, null until the first assignment (an
                // instantiation, or a call/attribute-read returning an
                // instance) stores a real `PyInstanceObj` into it, with the
                // separate `initialized` flag below trapping any read that
                // reaches it first. `p = Point(1, 2)` at module scope is
                // exactly the shape Task 8's own conformance fixture uses --
                // omitting this arm would turn that supported form into an
                // internal compiler panic. No exit-time decref accompanies
                // it: `pycc_rt::instance` is leak-only, mirroring `List`/
                // `Dict`/`Set`.
                pycc_mir::Ty::Instance(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Part 1 of #1026: identical storage and reasoning to
                // `Ty::Instance(_)` directly above -- an opaque pointer,
                // null until some store of a CPython object reference
                // fills it -- for example `pycc_ext_obj_import` (the
                // imported module), `pycc_ext_obj_import_from` (a
                // `from X import a` member, #1278), `emit_assign`'s object
                // store (a module-level `x = <object>`, #1325) or a
                // module-level `for` target's own store in the
                // `MirStmt::ForObject` emission -- with the separate `initialized`
                // flag below trapping any read that reaches it first. Since
                // Part 1 of #1499 the slot owns its one reference and a
                // rebind releases the old value (`object_slot.rs`). No
                // exit-time release accompanies it: the last value is
                // owned for the artifact's lifetime (see
                // `foreign_import.rs` and `docs/RUNTIME.md`).
                pycc_mir::Ty::Object => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // `int | None` (D-197, #763, Part 1 of #747): stored inline
                // like `Tuple`'s own struct arm immediately above, not as a
                // nullable pointer -- an `Optional[inner]` value is an LLVM
                // aggregate, exactly like a tuple, not a heap object. `x:
                // int | None = 5` at module scope is exactly the shape this
                // PR's own conformance fixture uses, so omitting this arm
                // would turn that supported form into an internal compiler
                // panic, the same reasoning `Tuple`'s own arm gives.
                //
                // Unlike `Tuple`'s plain `const_zero()`, the initializer's
                // payload field is built as a valid D-141-encoded smallint
                // `0` (`tag_smallint_const`), not a raw zero word -- for the
                // identical branch-free-masked-read reason
                // `default_value_for_type`'s own `Ty::Optional` arm and
                // `coerce_scalar_to_type`'s placeholder-normalization arm
                // are: `truthy`'s `Scalar::Optional` arm ANDs the payload's
                // truthiness with the present flag unconditionally rather
                // than branching around it, and a global read before its
                // first real assignment (guarded only by the separate
                // `initialized` flag below, not by never reaching this
                // value at all) must not hand that AND a raw zero word that
                // trips `classify_encoded_int`'s fail-closed panic. The
                // present flag itself is `0` either way, matching every
                // "unassigned" global's own zero-flag/null-pointer pattern
                // above.
                pycc_mir::Ty::Optional(inner) => {
                    let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
                    // #809: `default_value_for_type` (below) already draws
                    // this same `Ty::Int`-vs-other-inner-type distinction
                    // for exactly the same reason -- only `Ty::Int`'s
                    // payload field is the D-141 encoded-word
                    // representation `tag_smallint_const` produces; a
                    // `Float`/`Bool` inner type's payload field is a plain
                    // `f64`/`i8` value, and handing `tag_smallint_const`'s
                    // `i64` constant to `const_named_struct` for a struct
                    // whose first field is `f64`/`i8` is an LLVM constant
                    // type/size mismatch (the `Optional[bool]` case was
                    // reproduced crashing the LLVM backend with "invalid
                    // number of bytes" before this fix). Reuse
                    // `default_value_for_type` itself so this initializer
                    // and the per-local entry-block placeholder in
                    // `storage_slot_at_entry` can never drift apart again.
                    let payload = match inner.as_ref() {
                        pycc_mir::Ty::Int => tag_smallint_const(context, 0).into(),
                        other_inner => default_value_for_type(context, other_inner.clone()),
                    };
                    let zeroed = struct_ty
                        .const_named_struct(&[payload, context.i8_type().const_zero().into()]);
                    (struct_ty.into(), zeroed.into())
                }
                other => panic!(
                    "pycc_codegen: a `{}`-typed module binding is not supported yet",
                    other.name()
                ),
            };
            let global = module.add_global(storage_ty, None, &format!("pyglobal_{name}"));
            global.set_linkage(Linkage::Internal);
            global.set_initializer(&initializer);
            let initialized =
                module.add_global(context.i8_type(), None, &format!("pyglobal_init_{name}"));
            initialized.set_linkage(Linkage::Internal);
            initialized.set_initializer(&context.i8_type().const_zero());
            (
                name.clone(),
                StorageSlot {
                    ptr: global.as_pointer_value(),
                    ty: ty.clone(),
                    initialized: Some(initialized.as_pointer_value()),
                },
            )
        })
        .collect()
}

/// `target_triple`: `None` compiles for the host's own default target (the
/// common case). `Some(triple)` cross-compiles for a different Tier-1
/// target -- LLVM's codegen backend is inherently multi-target, so this
/// only requires `Target::initialize_all` (rather than
/// `Target::initialize_native`) plus the requested `TargetTriple` instead
/// of the host's default; producing an actual *linked binary* for a
/// foreign target is a separate concern the caller handles (see
/// `src/main.rs`'s `--target` handling and its doc comment on what's
/// actually achievable without bundling a full foreign sysroot).
///
/// `release`: `false` (the default, matching every build before this PR)
/// creates the target machine with `OptimizationLevel::None` and skips
/// LLVM's optimizer entirely -- today's only behavior. `true` selects
/// `OptimizationLevel::Aggressive` and additionally runs the `"default<O3>"`
/// whole-module pass pipeline via `Module::run_passes` before the object is
/// written (D-094). True cross-translation-unit LTO has no effect yet:
/// pycc emits exactly one LLVM module per compilation (single-file only
/// until v0.4's multi-file support), so this is "maximum whole-module
/// optimization," not literal cross-file link-time optimization.
///
/// # Concurrency
///
/// Safe to call concurrently from several threads of one process. LLVM's
/// process-global target registry is initialized exactly once, behind a
/// `OnceLock`, so no thread can observe the registry mid-write; see
/// `target_machine`'s module documentation for why that guard is needed
/// even though inkwell locks `Target::initialize_all` internally. Each
/// call otherwise owns its own `Context`, `Module` and `TargetMachine`,
/// and callers must still supply distinct `output_path`s.
pub fn compile_to_object(
    mir: &MirModule,
    output_path: &Path,
    target_triple: Option<&str>,
    release: bool,
) -> Result<(), String> {
    compile_to_object_with_options(
        mir,
        output_path,
        &CompileOptions {
            target_triple: target_triple.map(str::to_string),
            release,
            ..CompileOptions::default()
        },
    )
}

/// `compile_to_object` with the full option set (D-244's `ext` mode).
pub fn compile_to_object_with_options(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
) -> Result<(), String> {
    compile_to_object_with_observer(mir, output_path, options, None)
}

/// #379 (PR-19): Emit per-enum-member singleton init sequences. Each enum
/// member is a compile-time singleton instance that must be alive before
/// any top-level code reads it. For each enum class with members, and each
/// member in source order, allocate a fresh 2-slot instance
/// (`pycc_rt_instance_new(2, NULL, 0)` -- no layout descriptor, since both
/// slots are assigned right here, before any code can read them), set slot 0 to the member's `int` or `str`
/// value per its `EnumMemberValue` (#892 widened this from `int` alone),
/// set slot 1 to a string pointer containing the member name, and store
/// the instance pointer into the synthetic global
/// `<Class>.<Member>.enum_member`. Extracted from
/// `compile_to_object_with_observer` to isolate the enum-specific code
/// paths (see cargo-llvm-cov#276 for the coverage instantiation issue).
fn emit_enum_member_inits<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    mir: &MirModule,
    module_globals: &BTreeMap<String, StorageSlot<'ctx>>,
) {
    for (class_name, class_def) in &mir.class_defs {
        for (member_name, member_value) in &class_def.enum_members {
            let global_name = format!("{class_name}.{member_name}.enum_member");
            let slot = &module_globals[&global_name];
            // Allocate a fresh 2-slot instance.
            let count = context.i64_type().const_int(2, false);
            let no_layout = context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();
            let no_layout_len = context.i64_type().const_zero();
            let instance_ptr = builder
                .build_call(
                    rt.instance_new,
                    &[count.into(), no_layout.into(), no_layout_len.into()],
                    "enum_instance_new",
                )
                .expect("build_call should not fail for a well-formed enum member allocation")
                .try_as_basic_value()
                .expect_basic("pycc_rt_instance_new returns a non-void pointer")
                .into_pointer_value();
            // Set slot 0 to the member's own value literal, carried in
            // `HirClassDef.enum_members` from HIR through MIR to here.
            //
            // For an `int` member (`RED = 1` → 1), `emit_int_constant` folds
            // the value at compile time into the tagged-pointer
            // representation `pycc_rt` uses for small ints, or materializes
            // a heap bigint through `pycc_rt_int_from_i64` when the
            // discriminant is outside the tagged 63-bit range (D-178) --
            // once, here at module init.
            //
            // For a `str` member (#892: `RED = "red"`, or any member of an
            // `enum.StrEnum` subclass), `emit_string_literal` interns the
            // bytes as a private constant and calls
            // `pycc_rt_str_from_literal` -- the same helper slot 1 (the
            // member's `name`) already uses just below.
            let value_scalar = match member_value {
                pycc_mir::EnumMemberValue::Int(value) => {
                    Scalar::Int(emit_int_constant(context, builder, rt, *value))
                }
                pycc_mir::EnumMemberValue::Str(value) => {
                    Scalar::Str(emit_string_literal(context, builder, module, rt, value))
                }
            };
            let value_word = scalar_to_slot_word(context, builder, value_scalar);
            let slot0_index = context.i64_type().const_int(0, false);
            builder
                .build_call(
                    rt.instance_set_slot,
                    &[instance_ptr.into(), slot0_index.into(), value_word.into()],
                    "enum_set_value",
                )
                .expect("build_call should not fail for a well-formed enum value slot write");
            // Set slot 1 to a string pointer containing the member name.
            let name_ptr = emit_string_literal(context, builder, module, rt, member_name);
            let name_scalar = Scalar::Str(name_ptr);
            let name_word = scalar_to_slot_word(context, builder, name_scalar);
            let slot1_index = context.i64_type().const_int(1, false);
            builder
                .build_call(
                    rt.instance_set_slot,
                    &[instance_ptr.into(), slot1_index.into(), name_word.into()],
                    "enum_set_name",
                )
                .expect("build_call should not fail for a well-formed enum name slot write");
            // Store the instance pointer into the synthetic global and
            // mark it initialized. Module globals always have an
            // initialized flag (see `declare_module_globals`), so
            // `slot.initialized` is always `Some` here — `expect` makes
            // that invariant explicit without an unreachable `None` arm
            // that would create a permanently uncovered coverage region.
            builder
                .build_store(slot.ptr, instance_ptr)
                .expect("build_store should not fail for a declared enum member global");
            let initialized_ptr = slot
                .initialized
                .expect("module globals always have an initialized flag");
            builder
                .build_store(initialized_ptr, context.i8_type().const_int(1, false))
                .expect("build_store should not fail for a declared enum member init flag");
        }
    }
}

fn compile_to_object_with_observer(
    mir: &MirModule,
    output_path: &Path,
    options: &CompileOptions,
    mut observer: Option<&mut CodegenObserver<'_>>,
) -> Result<(), String> {
    let target_triple = options.target_triple.as_deref();
    let release = options.release;
    let context = Context::create();
    let module = context.create_module("pycc_module");
    let builder = context.create_builder();
    let i64_type = context.i64_type();
    let rt = declare_rt_functions(&context, &module);

    // First pass: declare every user-defined function -- with its real
    // parameter types and return type (Task 5), instead of Task 3/4's
    // placeholder zero-arg/void signature -- under a mangled name (never
    // the bare Python name) before emitting any body. Two reasons: this is
    // what lets a function call another function defined later in the
    // same module, or itself (recursion, now with real arguments and a
    // real return value flowing through it, since every signature is
    // already known before any body is lowered); and mangling is what
    // stops a Python-level function actually named `main` from colliding
    // with the real C-ABI entry point below, which must be literally
    // named `main` for the OS loader to find it. A def alone has no
    // runtime effect in Python regardless of its name -- something has to
    // call it, which is exactly the bug this pass structure fixes (see
    // git history: an earlier version treated a function merely named
    // `main` as auto-invoked, which doesn't match CPython at all).
    //
    // Issue #22: each `def` now gets a unique mangled name so redefinition
    // doesn't collide (`pyfn_{name}` for the first, `pyfn_{name}__redef_{n}`
    // for subsequent). A global function-pointer slot per unique name is
    // initialized to null; the top-level emission pass stores each def's
    // address into the slot when the def is "executed" in source order, and
    // all calls dispatch indirectly through the slot. This separates
    // LLVM symbol declaration (this pass, needed for the compiler to
    // generate call instructions) from Python name binding (the store,
    // which happens at the def's source position in top-level execution).
    let mut user_functions: HashMap<&str, UserFunction> = HashMap::new();
    // Maps function name to the number of definitions seen so far, for
    // unique mangled-name generation on redefinition.
    let mut def_counts: HashMap<&str, usize> = HashMap::new();
    // List of (function name, LLVM function value) in source order, for
    // the top-level binding pass.
    let mut function_defs_in_order: Vec<(&str, FunctionValue)> = Vec::new();
    // Part 2 of #1175 (#1179), review round 2: the buffer-sub-range
    // out-slot fact is resolved **per name, over every definition of it**,
    // not per `def`. Two `def f`s share one `fnptr_f` slot and one
    // `UserFunction::fn_type`, so giving them two arities makes the
    // indirect call ill-typed for whichever definition the slot does not
    // describe -- which is the `verify()` abort a redefinition used to hit
    // when only the slicing definition was widened. See
    // `crate::ext::buffer_slice_out_names` for why the union is the safe
    // direction and for the driver-side half that unions with it.
    let slice_widened = crate::ext::buffer_slice_out_names(&mir.items);
    for item in &mir.items {
        if let MirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        {
            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = params
                .iter()
                .map(|(_, ty)| ty_to_basic_type(&context, ty.clone()).into())
                .collect();
            // Part 2 of #1175 (#1179): a function whose body returns a
            // sub-range of a buffer parameter hands the sub-range's
            // `has_slice`/`start`/`stop` back through three trailing
            // out-pointers, because the bounds are produced inside this
            // frame while the returned value stays the parameter's own view
            // pointer -- which is what keeps the generated wrapper's
            // `result == &a{index}` identity test exact.
            //
            // The predicate is the body fact alone, deliberately not gated
            // on `options.ext` or on `ext_thunk_required`'s own
            // exportability test: this declaration and `emit_stmt`'s own
            // `MirStmt::ReturnBufferSlice` arm must never disagree about
            // the arity, and `pycc_types` already refuses the shape
            // everywhere a wrapper cannot exist.
            if slice_widened.contains(name.as_str()) {
                let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                for _ in crate::ext::ext_thunk_out_tys(return_ty, true) {
                    param_types.push(ptr_type.into());
                }
            }
            let fn_type = match return_ty {
                pycc_mir::Ty::None => context.void_type().fn_type(&param_types, false),
                other => ty_to_basic_type(&context, other.clone()).fn_type(&param_types, false),
            };
            let count = def_counts.entry(name.as_str()).or_insert(0);
            let mangled = if *count == 0 {
                format!("pyfn_{name}")
            } else {
                format!("pyfn_{name}__redef_{count}")
            };
            *count += 1;
            let f = module.add_function(&mangled, fn_type, None);
            function_defs_in_order.push((name.as_str(), f));
            // Only insert into user_functions on the first definition;
            // subsequent definitions update fn_ptr_global at their source
            // position but share the same slot and type info. The param_tys
            // and fn_type come from the first definition (the type checker
            // resolves one signature per name).
            user_functions.entry(name.as_str()).or_insert_with(|| {
                // Monomorphized generic specializations (`0gen_...` names)
                // are compiler-generated, not user-defined: they have no
                // top-level `def` whose execution order matters, so they
                // dispatch directly through `direct_value` instead of
                // through the indirect function-pointer slot.
                let is_monomorphized = name.starts_with("0gen_");
                if is_monomorphized {
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: None,
                        fn_type,
                        name_global: None,
                        direct_value: Some(f),
                    }
                } else {
                    let fn_ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                    // A method's `name` is dotted (`Grid.scale.static`), and
                    // this global is the one codegen emits that the generated
                    // C *declares* -- `extern void *fnptr_<name>;` in
                    // `pycc::ext_build`'s `wrapper_for`. A dotted C identifier
                    // is not accepted by any compiler, so the symbol carries
                    // the mangled spelling. inkwell 0.9 exposes no alias API
                    // and the workspace has no `llvm-sys` dependency, so this
                    // is a rename and not a C-safe alias beside the dotted
                    // one; nothing outside this workspace consumes the
                    // spelling. `mangle_ext_name` is the identity for a
                    // dot-free name, so every module-level function's global
                    // is byte-identical to what it was.
                    //
                    // This path is not gated on `CompileOptions::ext`, so a
                    // native build renames a method's global too, where no C
                    // ever declares it. That is deliberate: one spelling per
                    // symbol is cheaper to reason about than a mode-dependent
                    // one, and nothing outside this workspace reads it.
                    let fn_ptr_global = module.add_global(
                        fn_ptr_type,
                        None,
                        &format!("fnptr_{}", mangle_ext_name(name)),
                    );
                    fn_ptr_global.set_initializer(&fn_ptr_type.const_null());
                    // Deliberately *not* mangled, on both counts. The symbol
                    // is internal-linkage and never declared from C, so it
                    // needs no C-legal spelling; and its contents are the
                    // string `pycc_rt_name_error` prints, so the array size
                    // and initializer stay on the source dotted name --
                    // mangling them would make a method's `NameError` read
                    // `0m4_Grid5_scale6_static`.
                    let name_global = module.add_global(
                        context.i8_type().array_type(name.len() as u32 + 1),
                        None,
                        &format!("fnname_{name}"),
                    );
                    name_global.set_linkage(Linkage::Internal);
                    name_global.set_constant(true);
                    name_global.set_initializer(&context.const_string(name.as_bytes(), true));
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: Some(fn_ptr_global),
                        fn_type,
                        name_global: Some(name_global),
                        direct_value: None,
                    }
                }
            });
        }
    }

    // #1050: an `ext` artifact's generated C reaches an export whose
    // signature carries a `tuple` through a scalar-only thunk rather than
    // through `fnptr_<name>` directly, because pycc's aggregate calling
    // convention is not the platform C struct ABI. Emit those thunks now
    // that every callee's `fn_type` and `fnptr_` slot exist, and before the
    // builder is positioned in the module entry point below.
    if options.ext && !options.suppress_export_thunks {
        ext_thunk::emit_export_thunks(&context, &builder, &module, &rt, mir, &user_functions);
    }

    // Module bindings need process-wide storage because generated functions
    // can read them (D-041), including bindings whose assignment appears
    // after the function definition. Declare every slot before emitting either
    // the synthetic module entry point or any user function.
    let module_bindings = collect_module_bindings(mir);
    let module_globals = declare_module_globals(&context, &module, &module_bindings);

    let entry_fn_type = i64_type.fn_type(&[], false);
    let entry_fn = module.add_function(entry_fn_name(options.ext), entry_fn_type, None);
    let entry_block = context.append_basic_block(entry_fn, "entry");
    let top_exception_exit = context.append_basic_block(entry_fn, "top_exception_exit");
    builder.position_at_end(entry_block);
    // Part 1 of #1499: every module-global `object` slot's owned bit,
    // cleared before the first statement (`object_slot.rs`).
    object_slot::declare_owned_bits(&context, &builder, &module, &rt, &module_globals);
    // Top-level statements share one `locals` map across the synthetic
    // `main` entry block (module-level Python names are one shared
    // scope); each user function gets its own, fresh map below, since
    // Python function bodies don't see each other's locals.
    let mut top_level_locals: HashMap<_, _> = module_globals
        .iter()
        .map(|(name, binding)| (name.clone(), binding.clone()))
        .collect();
    // #379 (PR-19): emit per-enum-member singleton init sequences BEFORE
    // the top-level statement loop.
    emit_enum_member_inits(&context, &builder, &module, &rt, mir, &module_globals);
    // Issue #22: iterate over ALL items in source order, not just
    // top-level statements. A `MirItem::Function` at its source position
    // represents a `def` statement's runtime binding effect: store the
    // function's address into the global function-pointer slot so calls
    // after this point dispatch to it. A call before the `def` (the slot
    // is still null) aborts with `pycc_rt_name_error` -- matching
    // CPython's `NameError: name 'foo' is not defined`.
    let mut def_iter = function_defs_in_order.iter().peekable();
    let copy_slots = copy_slots::CopySlots::new(mir);
    let publish_plan = ext_publish::PublishPlan::new(mir, &copy_slots, options.ext);
    let mut fn_ordinal = 0usize;
    rt.exceptions.targets.borrow_mut().push(top_exception_exit);
    // Part 1 of #1096: while this target is the innermost one, no
    // module-level `try` encloses the statement being emitted, so a foreign
    // failure keeps its direct module-exec `-1` edge (`foreign_fail.rs`).
    rt.exceptions.module_exec_exit.set(Some(top_exception_exit));
    for item in &mir.items {
        match item {
            MirItem::TopLevelStmt(stmt) => {
                emit_stmt(
                    &context,
                    &builder,
                    &module,
                    &rt,
                    &user_functions,
                    &mut top_level_locals,
                    stmt,
                    pycc_mir::Ty::None,
                    &mut Vec::new(),
                )?;
                // #382: After each top-level statement, check for an active
                // exception. A `raise` (or a converted runtime failure like
                // division by zero) sets the pending exception state and
                // terminates the block with `unreachable`. Erase the
                // `unreachable` so the exception check can run here. This
                // check intercepts it at the top level: if an exception is
                // active, print it and exit. Inside a `try`, the try's own
                // post-body check intercepts first. `emit_stmt` rejects a
                // `Return` whose parent is this synthetic `main`, so after
                // erasing an exception's `unreachable` the current block is
                // guaranteed to accept this check.
                erase_unreachable_if_present(&builder);
                let active = builder
                    .build_call(rt.exception_active, &[], "top_exc_active")
                    .expect("build_call should not fail for exception_active")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_exception_active returns i8")
                    .into_int_value();
                let has_exc = builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        active,
                        context.i8_type().const_zero(),
                        "top_has_exc",
                    )
                    .expect("build_int_compare should not fail");
                let cont_bb = context.append_basic_block(entry_fn, "top_exc_cont");
                builder
                    .build_conditional_branch(has_exc, top_exception_exit, cont_bb)
                    .expect("build_conditional_branch should not fail");
                builder.position_at_end(cont_bb);
            }
            // Part 1 of #1026: emit the import call here, at the item's
            // own position in the source-order loop, rather than as a
            // hoisted prologue -- see `foreign_import.rs`. `def_iter` is
            // deliberately not advanced: it pairs with
            // `MirItem::Function` items only.
            MirItem::ForeignImport {
                local_name,
                module_path,
                from,
            } => foreign_import::emit_item(
                &context,
                &builder,
                &module,
                &rt,
                entry_fn,
                options.ext,
                &module_globals,
                foreign_import::ForeignBinding {
                    local_name,
                    module_path,
                    from: from.as_ref(),
                },
            ),
            MirItem::Function { name, .. } => {
                // Store this definition's function pointer into the
                // global slot, representing the `def`'s runtime binding.
                // `def_iter` is in the same source order as `mir.items`,
                // so the next entry matches this function definition.
                // Monomorphized generic specializations (`0gen_...` names)
                // have no `fn_ptr_global` (they dispatch directly), so
                // skip the store for them.
                let &(_, f) = def_iter.next().expect(
                    "def_iter should have an entry for every MirItem::Function                      (the declaration pass populates function_defs_in_order                      from the same mir.items in the same order)",
                );
                let ordinal = fn_ordinal;
                fn_ordinal += 1;
                // #1337 (D-254): an inherited-method copy is bound with the
                // origin `def` it copies, not at its own (appended)
                // position -- see `copy_slots`.
                if copy_slots.is_copy(ordinal) {
                    continue;
                }
                let bound = std::iter::once((name.as_str(), f)).chain(
                    copy_slots
                        .bound_with(ordinal)
                        .iter()
                        .map(|&copy| function_defs_in_order[copy]),
                );
                for (name, f) in bound {
                    let uf = &user_functions[name];
                    if let Some(ref fn_ptr_global) = uf.fn_ptr_global {
                        let _ = builder.build_store(
                            fn_ptr_global.as_pointer_value(),
                            f.as_global_value().as_pointer_value(),
                        );
                    }
                }
                // #1199: publish what this `def` (and its copies) made
                // callable -- see `ext_publish`. A `def` is refused inside
                // any module-level block, so no module-level `try` encloses
                // this point, and a failed publish takes the direct
                // module-exec `-1` edge, as a foreign failure here does
                // (Part 1 of #1096).
                debug_assert_eq!(
                    rt.exceptions.targets.borrow().last(),
                    Some(&top_exception_exit)
                );
                ext_publish::emit(
                    &context,
                    &builder,
                    &module,
                    entry_fn,
                    publish_plan.names_at(ordinal),
                );
            }
        }
    }
    rt.exceptions.targets.borrow_mut().pop();
    rt.exceptions.module_exec_exit.set(None);
    // Module-level Python code has no `return` (T0024) -- every top-level
    // `str` local's single exit point is program completion right here, so
    // this is where its accepted refcounting scope (D-061's Task 7
    // addendum) decrefs it exactly once, before `main` itself returns.
    //
    // `ext` mode has no such exit point. The entry function is D-244's
    // `Py_mod_exec` slot, and the module object outlives it: every wrapper
    // the host may call afterwards still reads these globals, so decrefing
    // them here frees storage that stays reachable. `PyModuleDef.m_free` is
    // NULL, so nothing ever tears the module down -- holding the reference
    // for the process lifetime is the correct D-060 accounting, exactly as
    // the exceptional exit below already assumes.
    for slot in module_globals.values() {
        if slot.ty == pycc_mir::Ty::Str && !options.ext {
            let value = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "final_str",
                )
                .expect("build_load should not fail for this function's own alloca")
                .into_pointer_value();
            builder
                .build_call(rt.str_decref, &[value.into()], "str_decref_final")
                .expect("build_call should not fail for a well-formed decref");
        }
    }
    // `emit_stmt` rejects every `Return` whose parent is this synthetic
    // `main`; all exception-produced `unreachable` terminators are erased
    // before their explicit state check above. The current continuation is
    // therefore guaranteed to accept the process-success return below.
    // See the module-level comment block below for why these .expect()s
    // (this one included) are deliberate rather than Result-threaded: each
    // covers an operation that stays infallible given how this function
    // always calls it. Two calls below are genuine, externally-triggerable
    // failure modes and stay real Results the caller must handle:
    // Target::from_triple (a user-supplied --target can legitimately name
    // a triple LLVM doesn't recognize) and write_to_file, at the very end.
    builder
        .build_return(Some(&i64_type.const_int(0, false)))
        .expect(
            "build_return should not fail: builder is always freshly positioned before this call",
        );

    // Every exceptional module-scope edge converges here. Keeping one
    // target alive while top-level expressions are emitted lets recursive
    // expression guards stop later operands and effects immediately.
    builder.position_at_end(top_exception_exit);
    if options.ext {
        // D-244's `ext` mode: the module body runs inside the host
        // interpreter's `Py_mod_exec` slot, so an uncaught module-scope
        // exception must *fail the import*, not end the process.
        // `pycc_rt_exception_print_and_exit` is `-> !` and calls
        // `std::process::exit(1)`, which here would terminate the
        // interpreter that just imported this artifact. Return `-1`
        // instead, leaving the pending exception state set: the C shim
        // reads it through `pycc_rt_ext_pending_type`/`_message`, raises
        // the matching CPython exception, and returns `-1` from the slot.
        // `print_and_exit` stays `native` mode's handler, untouched.
        builder
            .build_return(Some(
                &i64_type.const_int(EXT_MODULE_EXEC_FAILED as u64, true),
            ))
            .expect("build_return should not fail on a freshly positioned builder");
    } else {
        let exc_val = builder
            .build_call(rt.exception_value, &[], "top_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_print_and_exit, &[exc_val.into()], "")
            .expect("build_call should not fail for exception_print_and_exit");
        builder
            .build_unreachable()
            .expect("build_unreachable should terminate a noreturn block");
    }

    // Second pass: fill in each user function's body, now that every
    // function (including ones a body might call) is already declared.
    // Each parameter is bound into `fn_locals` by allocating a fresh slot
    // for it and storing the incoming LLVM argument into it (Task 5) --
    // the same load/store-via-`alloca` model every other local already
    // uses (see `emit_assign`), so a parameter is fully ordinary once
    // bound: reassignable, and readable via `emit_expr`'s `Name` arm with
    // no special-casing.
    // Issue #22: use `function_defs_in_order` to get the correct LLVM
    // function value for each definition -- a redefined name has multiple
    // function values (one per def, with unique mangled names), and each
    // body must be emitted into its own function value, not the first
    // definition's.
    let mut body_def_iter = function_defs_in_order.iter();
    for item in &mir.items {
        if let MirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        {
            // Advance the iterator in lockstep with `mir.items`'s
            // Function items -- same source order, same count.
            let f = body_def_iter
                .next()
                .map(|&(_, f)| f)
                .expect("function body has matching declaration");
            let block = context.append_basic_block(f, "entry");
            builder.position_at_end(block);
            let mut fn_locals: HashMap<_, _> = module_globals
                .iter()
                .map(|(global_name, binding)| (global_name.clone(), binding.clone()))
                .collect();
            // #1054: the release list for this function's `str` epilogue.
            //
            // **Ownership invariant this epilogue depends on (plan §7 R2):
            // a `str` slot holds exactly one *owned* `PyStrObj` reference,
            // or null.** Every producer upholds it -- a parameter slot
            // receives an argument the caller already `pycc_rt_str_incref`d
            // (`emit_user_call_args`' own `incref_if_str_duplicate`) or a
            // freshly built object with refcount 1 (a literal, a
            // concatenation, the D-244 `ext` shim's `pycc_ext_unpack_str`);
            // a rebinding store releases the old value first
            // (`decref_str_slot_before_store`); a `return` of a bare name
            // increfs the duplicate it hands back. So exactly one
            // `pycc_rt_str_decref` per slot on the way out balances the
            // books, and the null a never-assigned local still holds is the
            // runtime's documented no-op.
            //
            // Deliberately a *snapshot* taken here rather than a walk of
            // `fn_locals` at exit: `fn_locals` is seeded above with this
            // module's globals (which this function does not own) and is
            // extended mid-function by `exception.rs`'s `except X as e`
            // bindings (whose slots are not in the entry block and so need
            // not dominate the epilogue).
            let mut owned_str_slots: Vec<PointerValue> = Vec::new();
            // Part 2a of #1142 (#1165): the same list for artifact-owned
            // buffer storage, kept separate from `owned_str_slots` because
            // the two release through different runtime helpers.
            //
            // **Only locals are ever pushed here, never parameters.** A
            // buffer parameter holds a `PyccExtBufferView *` the *host*
            // owns and the wrapper borrowed for one call; freeing it would
            // be an invalid free. `pycc_types`' `buffer_parameter_rebinding`
            // refuses the one shape (`b = ndarray(3)` on a parameter `b`)
            // that could make a parameter slot look like it needed
            // releasing, so this asymmetry with the `str` list above is a
            // checked invariant rather than an assumption.
            let mut owned_buffer_slots: Vec<PointerValue> = Vec::new();
            for (i, (param_name, ty)) in params.iter().enumerate() {
                // `.expect(...)`, not `.unwrap_or_else(|| panic!(...))`:
                // `f`'s own `fn_type` (built above, in the first pass) was
                // constructed from this exact `params` list, so
                // `get_nth_param` always succeeds for every `i` this loop
                // produces -- an unreachable "missing parameter" case
                // would be a defensive branch this crate's own coverage
                // gate (D-014) could never legitimately exercise, so it's
                // not introduced in the first place, the same reasoning
                // already applied elsewhere in this file (e.g.
                // `emit_body_then_branch`'s removed guard, Task 4).
                let incoming = f.get_nth_param(i as u32).expect(
                    "this function was declared with exactly `params.len()` parameters above",
                );
                let slot = storage_slot_at_entry(&context, &builder, ty.clone(), param_name, false);
                builder.build_store(slot.ptr, incoming).expect(
                    "build_store should not fail for a slot this function itself allocated",
                );
                if *ty == pycc_mir::Ty::Str {
                    owned_str_slots.push(slot.ptr);
                }
                fn_locals.insert(param_name.clone(), slot);
            }
            // Part 2 of #1175 (#1179), review round 2: bind this frame's
            // three out-pointers -- present exactly when the declaration
            // pass above widened this *name* -- so both `MirStmt::Return`
            // and `MirStmt::ReturnBufferSlice` reach them by name instead
            // of by `count_params()` arithmetic, and so a bare `return`
            // can tell that it owes the wrapper a `has_slice = 0`.
            if slice_widened.contains(name.as_str()) {
                let out_base = f.count_params() - 3;
                for (offset, key) in [
                    BUFFER_SLICE_HAS_SLICE_KEY,
                    BUFFER_SLICE_START_KEY,
                    BUFFER_SLICE_STOP_KEY,
                ]
                .into_iter()
                .enumerate()
                {
                    let ptr = f
                        .get_nth_param(out_base + offset as u32)
                        .expect("the declaration pass widened this signature by three out-pointers")
                        .into_pointer_value();
                    fn_locals.insert(
                        key.to_string(),
                        StorageSlot {
                            ptr,
                            ty: pycc_mir::Ty::Int,
                            initialized: None,
                        },
                    );
                }
            }
            let mut local_bindings = BTreeMap::new();
            for stmt in body {
                collect_stmt_bindings(stmt, &mut local_bindings);
            }
            for (param_name, _) in params {
                local_bindings.remove(param_name);
            }
            for (local_name, ty) in local_bindings {
                let is_str = ty == pycc_mir::Ty::Str;
                // Part 2a of #1142 (#1165): a buffer *local* is artifact-owned
                // storage and joins the release list; the parameter loop above
                // deliberately has no counterpart.
                let is_buffer = ty == pycc_mir::Ty::MemoryView;
                let slot = storage_slot_at_entry(&context, &builder, ty, &local_name, true);
                if is_str {
                    owned_str_slots.push(slot.ptr);
                }
                if is_buffer {
                    owned_buffer_slots.push(slot.ptr);
                }
                // A function-local target shadows a same-named module global
                // throughout the function (D-055), so this intentionally
                // replaces any global slot seeded above.
                fn_locals.insert(local_name, slot);
            }
            // Part 2b of #1142 (#1164): the pending-return record, seeded
            // only for a frame that can actually hand a buffer back and owns
            // at least one buffer slot to hand. Both allocas belong to the
            // entry block, where the builder still is, on the same model as
            // every storage slot above.
            //
            // Seeded *after* the binding loop above deliberately: that loop
            // is what fills `owned_buffer_slots`, and a synthetic entry
            // picked up as an ordinary `memoryview` local would join the
            // release list and make the epilogue free the returned pointer a
            // second time.
            let pending_return_slots =
                if *return_ty == pycc_mir::Ty::MemoryView && !owned_buffer_slots.is_empty() {
                    let pending = builder
                        .build_alloca(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            "pending_return_buffer_slot",
                        )
                        .expect("build_alloca should not fail for the pending-return slot");
                    builder
                        .build_store(
                            pending,
                            context
                                .ptr_type(inkwell::AddressSpace::default())
                                .const_null(),
                        )
                        .expect("build_store should not fail for the pending-return slot init");
                    let orphaned = builder
                        .build_alloca(context.i8_type(), "pending_return_orphaned_slot")
                        .expect("build_alloca should not fail for the pending-return orphan flag");
                    builder
                        .build_store(orphaned, context.i8_type().const_zero())
                        .expect("build_store should not fail for the orphan flag init");
                    fn_locals.insert(
                        PENDING_RETURN_BUFFER_KEY.to_string(),
                        StorageSlot {
                            ptr: pending,
                            ty: pycc_mir::Ty::MemoryView,
                            initialized: None,
                        },
                    );
                    fn_locals.insert(
                        PENDING_RETURN_ORPHANED_KEY.to_string(),
                        StorageSlot {
                            ptr: orphaned,
                            ty: pycc_mir::Ty::Bool,
                            initialized: None,
                        },
                    );
                    Some((pending, orphaned))
                } else {
                    None
                };
            // #1054: when this function owns at least one entry-block
            // `str` slot, every way out of it is routed through a single
            // synthetic outermost `finally` frame whose block releases
            // those slots. Reusing #382's existing `FinallyTarget`
            // machinery rather than inventing a landing pad means
            // `MirStmt::Return` and `exception.rs`'s own `try`/`try*`
            // return routing need no edit at all: they already store to
            // the innermost frame's `ret_slot` and branch to its
            // `finally_bb`, and a nested frame already propagates outward
            // to this one. Only the two paths that bypass `finally_stack`
            // entirely -- the implicit `return None` fall-through and the
            // `exception_exit` carrier return, both below -- are
            // redirected here.
            //
            // Gated on a non-empty snapshot so a `str`-free function emits
            // byte-identical IR to before this change, which is also what
            // keeps the "no outer `finally_stack` entry" arms of
            // `MirStmt::Return` and of `emit_try`/`emit_try_star`'s
            // `ret_bb` reachable (their tests build `str`-free functions).
            // The module entry point (`main` / `__pycc_ext_exec`) is a
            // separate emitter above and never gets this frame.
            let mut finally_stack: Vec<FinallyTarget> = Vec::new();
            // Part 2a of #1142 (#1165): the gate widens to "this function
            // owns *any* releasable entry-block slot". Left keyed on
            // `owned_str_slots` alone, a buffer-owning but `str`-free
            // function would get no epilogue block at all and leak one
            // allocation per call into a long-lived host process -- silently,
            // since nothing else in this emitter would notice.
            let owned_slot_epilogue_bb = if owned_str_slots.is_empty()
                && owned_buffer_slots.is_empty()
            {
                None
            } else {
                let is_returning = builder
                    .build_alloca(context.i8_type(), "str_epilogue_is_returning")
                    .expect("build_alloca should not fail for the str epilogue is_returning flag");
                builder
                    .build_store(is_returning, context.i8_type().const_zero())
                    .expect("build_store should not fail for the str epilogue is_returning init");
                let ret_slot = if *return_ty == pycc_mir::Ty::None {
                    None
                } else {
                    Some(
                        builder
                            .build_alloca(
                                ty_to_basic_type(&context, return_ty.clone()),
                                "str_epilogue_ret_slot",
                            )
                            .expect("build_alloca should not fail for the str epilogue ret_slot"),
                    )
                };
                let finally_bb = context.append_basic_block(f, "pycc_str_epilogue");
                finally_stack.push(FinallyTarget {
                    finally_bb,
                    ret_slot,
                    is_returning,
                });
                Some((finally_bb, ret_slot))
            };
            let exception_exit = context.append_basic_block(f, "exception_exit");
            rt.exceptions.targets.borrow_mut().push(exception_exit);
            emit_body(
                &context,
                &builder,
                &module,
                &rt,
                &user_functions,
                &mut fn_locals,
                body,
                return_ty.clone(),
                &mut finally_stack,
            )?;
            rt.exceptions.targets.borrow_mut().pop();
            // A `None`-returning function falling through its last
            // statement without an explicit `return` is ordinary, legal
            // Python (an implicit `return None`); a non-`None`-returning
            // function falling through is not -- `pycc_types`' T0024
            // fallthrough check rejects that HIR before it ever reaches
            // codegen, so seeing it here is this crate's own internal
            // error, not a user-facing rejection (see this task's own
            // `a_non_none_function_falling_through_is_an_internal_error_
            // not_bad_ir` test).
            match return_ty {
                pycc_mir::Ty::None => {
                    if builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        // #1054: the implicit `return None` never passes
                        // through `finally_stack`, so it is redirected to
                        // the owned-slot epilogue explicitly. No `ret_slot`
                        // store: a `None`-returning function's epilogue
                        // has none, and the epilogue does not read
                        // `is_returning` -- every path into it returns.
                        if let Some((epilogue_bb, _)) = owned_slot_epilogue_bb {
                            builder.build_unconditional_branch(epilogue_bb).expect(
                                "build_unconditional_branch should not fail for str epilogue routing",
                            );
                        } else {
                            builder.build_return(None).expect(
                                "build_return should not fail: builder is always freshly positioned before this call",
                            );
                        }
                    }
                }
                _ if builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none() =>
                {
                    if fallthrough::block_always_terminates(body) {
                        builder.build_unreachable().expect(
                            "build_unreachable should terminate a statically impossible continuation",
                        );
                    } else {
                        panic!(
                            "pycc_codegen: internal error: `{name}` is declared to return a \
                             non-`None` value but fell through without a `return` -- \
                             pycc_types::check (T0024) should have rejected this HIR before \
                             it reached codegen"
                        );
                    }
                }
                _ => {}
            }

            // A Python exception crosses the native function ABI via the
            // thread-local runtime flag. Return a neutral carrier here;
            // the caller's expression guard observes the still-active flag
            // before it can consume that carrier or evaluate another effect.
            builder.position_at_end(exception_exit);
            // #1054: the exceptional exit is the other path that bypasses
            // `finally_stack`. The slots are released here too -- an
            // in-flight exception does not make the references this
            // function owns somebody else's problem.
            match (owned_slot_epilogue_bb, *return_ty == pycc_mir::Ty::None) {
                (Some((epilogue_bb, _)), true) => {
                    builder.build_unconditional_branch(epilogue_bb).expect(
                        "build_unconditional_branch should not fail for an exceptional None exit",
                    );
                }
                (Some((epilogue_bb, ret_slot)), false) => {
                    let default = default_value_for_type(&context, return_ty.clone());
                    let slot = ret_slot.expect(
                        "a non-`None`-returning function's str epilogue always has a ret_slot",
                    );
                    builder
                        .build_store(slot, default)
                        .expect("build_store should not fail for the exceptional carrier");
                    builder.build_unconditional_branch(epilogue_bb).expect(
                        "build_unconditional_branch should not fail for an exceptional value exit",
                    );
                }
                (None, true) => {
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for an exceptional None exit");
                }
                (None, false) => {
                    let default = default_value_for_type(&context, return_ty.clone());
                    builder
                        .build_return(Some(&default))
                        .expect("build_return should not fail for an exceptional value exit");
                }
            }

            // #1054: emitted last, after every branch into it exists. One
            // `pycc_rt_str_decref` per owned `str` entry-block slot -- a
            // null slot (a local on a path that never assigned it) is the
            // runtime's documented no-op -- then, since Part 2a of #1142
            // (#1165), one `pycc_rt_buffer_f64_free` per owned buffer slot
            // on the same model, and finally the function's single real
            // `ret`. The IR value names below keep their `str_` prefix so a
            // `str`-owning function still emits byte-identical IR.
            if let Some((epilogue_bb, ret_slot)) = owned_slot_epilogue_bb {
                builder.position_at_end(epilogue_bb);
                for slot_ptr in &owned_str_slots {
                    let live = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            *slot_ptr,
                            "str_epilogue_live",
                        )
                        .expect("build_load should not fail for a str slot this function allocated")
                        .into_pointer_value();
                    builder
                        .build_call(rt.str_decref, &[live.into()], "str_epilogue_decref")
                        .expect("build_call should not fail for pycc_rt_str_decref");
                }
                // Part 2a of #1142 (#1165): one `pycc_rt_buffer_f64_free`
                // per artifact-owned buffer slot, on exactly the `str`
                // model above -- the null a never-assigned local still
                // holds is the runtime's documented no-op, which is what
                // covers a buffer allocated in only one arm of an `if`.
                //
                // Part 2b of #1142 (#1164): **this block is where ownership
                // of a returned buffer transfers**, and it is the only
                // place it can correctly happen. A `return` statement makes
                // a value *pending*; it is not the point control leaves the
                // frame. Between the two, `finally_stack` interposes every
                // user finalizer -- which may read the returned name, raise,
                // or return a different buffer of its own -- so a transfer
                // performed at the `return` statement is wrong for all three
                // (a finalizer reading a cleared slot dereferences null and
                // aborts the hosting interpreter; a finalizer that raises or
                // overrides the return abandons the pointer the statement
                // already disowned). Every exit routes through this one
                // block with the value that is actually being returned
                // already stored in `ret_slot`, so the transfer is decided
                // here by identity against that value.
                //
                // Branch-free by construction: `select` replaces the
                // returned pointer with the null `pycc_rt_buffer_f64_free`
                // documents as a no-op, so the release loop keeps the single
                // straight-line shape #1165 gave it. No two owned slots can
                // hold one pointer -- a bare read of an owned buffer name is
                // refused by `owned_buffer_use_unsupported` everywhere but
                // this return position, so `b = a` cannot alias -- which is
                // what makes "skip the one slot whose value is being
                // returned" release each allocation exactly once.
                let returned_buffer =
                    if owned_buffer_slots.is_empty() || *return_ty != pycc_mir::Ty::MemoryView {
                        None
                    } else {
                        let slot = ret_slot.expect(
                        "a `memoryview`-returning function's owned-slot epilogue has a ret_slot",
                    );
                        Some(
                            builder
                                .build_load(
                                    context.ptr_type(inkwell::AddressSpace::default()),
                                    slot,
                                    "buffer_epilogue_returned",
                                )
                                .expect("build_load should not fail for the epilogue ret_slot")
                                .into_pointer_value(),
                        )
                    };
                for slot_ptr in &owned_buffer_slots {
                    let live = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            *slot_ptr,
                            "buffer_epilogue_live",
                        )
                        .expect(
                            "build_load should not fail for a buffer slot this function allocated",
                        )
                        .into_pointer_value();
                    let release = match returned_buffer {
                        None => live,
                        Some(returned) => {
                            let transferred = builder
                                .build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    live,
                                    returned,
                                    "buffer_epilogue_transferred",
                                )
                                .expect("build_int_compare should not fail for two pointers");
                            builder
                                .build_select(
                                    transferred,
                                    context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .const_null(),
                                    live,
                                    "buffer_epilogue_release",
                                )
                                .expect("build_select should not fail for two pointers")
                                .into_pointer_value()
                        }
                    };
                    builder
                        .build_call(
                            rt.buffer_f64_free,
                            &[release.into()],
                            "buffer_epilogue_free",
                        )
                        .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                }
                // Part 2b of #1142 (#1164): the one pointer the loop above
                // cannot see. A finalizer that rebound the returned name left
                // the pending record as that pointer's sole owner, so it is
                // released here -- unless it is the value actually being
                // returned, in which case it belongs to the host now.
                //
                // Gated on the orphan flag rather than on the record alone:
                // unorphaned, the pending pointer is still in some local slot
                // and the loop above already released it, so freeing it again
                // here would be a double free on every exception exit.
                if let (Some((pending_ptr, orphaned_ptr)), Some(returned)) =
                    (pending_return_slots, returned_buffer)
                {
                    let pending = builder
                        .build_load(
                            context.ptr_type(inkwell::AddressSpace::default()),
                            pending_ptr,
                            "buffer_epilogue_pending",
                        )
                        .expect("build_load should not fail for the pending-return slot")
                        .into_pointer_value();
                    let orphaned = builder
                        .build_load(context.i8_type(), orphaned_ptr, "buffer_epilogue_orphaned")
                        .expect("build_load should not fail for the pending-return orphan flag")
                        .into_int_value();
                    let was_orphaned = builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            orphaned,
                            context.i8_type().const_zero(),
                            "buffer_epilogue_was_orphaned",
                        )
                        .expect("build_int_compare should not fail for an i8 flag");
                    let transferred = builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            pending,
                            returned,
                            "buffer_epilogue_pending_transferred",
                        )
                        .expect("build_int_compare should not fail for two pointers");
                    let null = context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    let unless_transferred = builder
                        .build_select(transferred, null, pending, "buffer_epilogue_pending_keep")
                        .expect("build_select should not fail for two pointers")
                        .into_pointer_value();
                    let release = builder
                        .build_select(
                            was_orphaned,
                            unless_transferred,
                            null,
                            "buffer_epilogue_pending_release",
                        )
                        .expect("build_select should not fail for two pointers")
                        .into_pointer_value();
                    builder
                        .build_call(
                            rt.buffer_f64_free,
                            &[release.into()],
                            "buffer_epilogue_pending_free",
                        )
                        .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                }
                match ret_slot {
                    Some(slot) => {
                        let ret_val = builder
                            .build_load(
                                ty_to_basic_type(&context, return_ty.clone()),
                                slot,
                                "str_epilogue_ret_val",
                            )
                            .expect("build_load should not fail for the str epilogue ret_slot");
                        builder
                            .build_return(Some(&ret_val))
                            .expect("build_return should not fail for the str epilogue");
                    }
                    None => {
                        builder
                            .build_return(None)
                            .expect("build_return should not fail for a void str epilogue");
                    }
                }
            }
        }
    }

    verify_module(&module);

    // See `target_machine::target_machine_for` for the LLVM target-registry
    // initialization contract this call satisfies, and for the D-029 and
    // D-073 reasoning behind the triple and relocation-model choices.
    let target_machine = target_machine::target_machine_for(target_triple, release)?;
    // `--release`'s whole-module optimization pipeline (D-094). This is
    // "maximum whole-module optimization," not literal cross-translation-
    // unit LTO: pycc emits exactly one LLVM module per compilation today
    // (single-file only until v0.4's multi-file support), so there is only
    // ever one module for `"default<O3>"` to optimize.
    //
    // Deliberately `.expect(..)`, not `.map_err(llvm_string_to_owned)?` like
    // `Target::from_triple` (in `target_machine`)/`write_to_file` below:
    // those two are genuine,
    // externally-triggerable failure modes with their own dedicated tests
    // and reachable `Err` paths this crate's 100% region-coverage gate
    // (D-014) can actually exercise. `run_passes` here runs a fixed,
    // always-valid pipeline string against a module `verify_module` has
    // already accepted (skipped only on Windows per D-029's own "no
    // Windows-specific IR-building path exists" reasoning, which applies
    // here too) -- there is no way to make this fail that a test could
    // construct, so this stays infallible given how this function always
    // calls it, the same treatment `create_target_machine` gets in
    // `target_machine` and `module.verify()` gets in `verify_module` below.
    // On the
    // vanishingly narrow chance this ever legitimately panics on Windows,
    // the `LLVMString`'s `Drop` would run during unwinding (D-029's crash)
    // -- an accepted, currently unreachable risk, not a silently ignored one.
    let applied_pipeline = if release {
        module
            .run_passes(
                RELEASE_PASS_PIPELINE,
                &target_machine,
                PassBuilderOptions::create(),
            )
            .expect(
                "LLVM's \"default<O3>\" pipeline should never fail against a module this \
                 function has already verified, using a fixed, always-valid pipeline string",
            );
        Some(RELEASE_PASS_PIPELINE)
    } else {
        None
    };
    if let Some(observer) = observer.as_mut() {
        observer(&module, applied_pipeline);
    }
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(llvm_string_to_owned)
}

/// See D-029: every message inkwell hands back as an `LLVMString` --
/// `Target::from_triple`'s and `TargetMachine::write_to_file`'s error
/// paths, on top of `TargetTriple` itself -- shares the same broken `Drop`
/// (`LLVMDisposeMessage`) on Windows against this LLVM release. Converts
/// to an owned `String` (a real copy, safe to keep and format past this
/// call) and forgets the original rather than letting it drop. General
/// fix at the one place `LLVMString` crosses into this crate's error
/// values, not a patch per call site -- so a future one isn't missed.
fn llvm_string_to_owned(message: inkwell::support::LLVMString) -> String {
    let owned = message.to_string();
    std::mem::forget(message);
    owned
}

/// Skipped on Windows: `module.verify()` crashes there with an access
/// violation when linked against the official prebuilt LLVM 22.1.1 release
/// -- isolated with stderr checkpoints bracketing every call from the end of
/// IR building through object emission (D-029): every other call completed,
/// consistently, across every test that reached this point; only this one
/// never returned. Root cause not further isolated -- no Windows debugger
/// available in this environment to get an exact crash address/frame -- so
/// this is a targeted skip of a *pure internal sanity check* (a failure
/// here would mean a pycc_codegen bug, never a rejection of legitimate user
/// code -- see the non-Windows body's own message), not a change to what
/// gets compiled. The identical IR-building code already runs verified on
/// macOS/Linux for every test in this suite, which narrows the residual,
/// Windows-only risk to "a bug that only produces malformed IR on a
/// Windows-specific code path" -- and no such path exists yet, since IR
/// building above has no platform-conditional logic at all.
#[cfg(windows)]
fn verify_module(_module: &inkwell::module::Module<'_>) {}

#[cfg(not(windows))]
fn verify_module(module: &inkwell::module::Module<'_>) {
    module.verify().expect(
        "generated IR should always be well-formed for this fixed instruction shape; \
         a verify() failure here means a bug in pycc_codegen itself, not bad user input",
    );
}

/// Handles every `MirStmt` shape (this match is exhaustive over
/// `MirStmt`, no catch-all arm): a `print()` call of any number of
/// `int`/`float`/`bool`/`str` arguments plus any supported materializable
/// non-`print()` `None` expression, including direct user-function,
/// `ListAppend`, and `SetAdd` results, a D-075 parameter, or ordinary
/// assignment storage (space-separated, one trailing newline, matching
/// CPython's `print(*args)` -- all arguments are now evaluated
/// left-to-right before any output is emitted, matching CPython's own
/// argument-evaluation semantics (fixes #145; see
/// `emit_eval_print_arg`/`emit_write_print_arg`); D-072 still excludes
/// using `print()` itself as a nested expression), any
/// other bare expression statement (a user-function call with any number of
/// arguments included -- see `emit_expr`'s `Call` arm, which this now
/// delegates to uniformly instead of special-casing zero-arg calls here), a
/// local-variable assignment, `If`/`While`/
/// `ForRange` control flow (Task 4) -- real basic blocks, conditional
/// branches, and loop back-edges, using `truthy` for the shared `if`/
/// `while` truthiness check and `emit_body_then_branch`/an inline
/// equivalent for the terminator-safety this introduces (see both
/// helpers' own doc comments) -- `Return` (Task 5), terminating
/// the current block with the evaluated value (or none, for a bare
/// `return`) -- and `ForList` (v0.2, D-105/Task 11b), reusing `ForRange`'s
/// own loop/branch-building infrastructure parametrized over a runtime
/// `pycc_rt_int_list_len` call instead of a static bound. `ForList` is a
/// v0.2 addition, not part of v0.1's own original shape set.
#[allow(clippy::too_many_arguments)]
fn emit_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    match stmt {
        MirStmt::ExprStmt(MirExpr::Call { callee, args, .. }) if callee == "print" => {
            // Two-phase evaluate-then-output (fixes #145): evaluate *all*
            // arguments left-to-right before emitting any output, so a
            // later argument's side effects (e.g. a user function that
            // itself calls `print`) complete before this `print`'s own
            // output begins -- matching CPython's left-to-right
            // argument-evaluation semantics. See `emit_eval_print_arg`/
            // `emit_write_print_arg` for the phase split and the
            // `cargo llvm-cov` region-attribution artifact that keeps
            // them as extracted helpers.
            string_render::emit_print_call(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                args,
            );
            Ok(())
        }
        // A user-function call whose declared return type is `None`, used
        // as a bare statement (e.g. `main()`) -- must go through
        // `build_call_to` directly rather than the general `ExprStmt(expr)`
        // arm below: the call itself returns LLVM `void`, and the general
        // expression path's canonical unit carrier is unnecessary when the
        // result is discarded. Matched by `ty`
        // alone (no `callee != "print"` guard needed): the arm above
        // already claims every `print` call via its own guard regardless
        // of `ty`, so only a non-`print` call ever reaches this one.
        //
        // Generalizes this crate's pre-Task-5 zero-arg-only special case
        // (which this task's brief called "redundant" and removed) to a
        // call of any arity: unlike `emit_expr`'s `Call` arm, this arm
        // *does* still have a `Result` to propagate a clean, user-facing
        // error through for an undefined callee, exactly like the
        // zero-arg case always did -- so it keeps doing that, rather than
        // switching to `emit_expr`'s "internal error" panic, preserving
        // both the existing user-facing behavior and the `?`-propagation
        // coverage every nested-body call site below relies on.
        MirStmt::ExprStmt(MirExpr::Call {
            callee,
            args,
            ty: pycc_mir::Ty::None,
        }) => {
            let user_function = user_functions.get(callee.as_str()).ok_or_else(|| {
                format!("pycc_codegen v0.1: call to undefined function `{callee}`")
            })?;
            let _ = user_function; // validated above; build_call_to looks up by name
            build_call_to(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                callee,
                args,
            );
            guard_statement_effects(context, builder, rt);
            Ok(())
        }
        MirStmt::ExprStmt(expr) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, expr);
            // #146 Part 2 (D-181): a statement-position expression's value
            // is discarded outright, so nothing else will ever retire the
            // reference it was born with.
            release_scalar_if_int_temporary(context, builder, rt, expr, &scalar);
            // Part 1 of #1092: the same for a produced CPython object
            // (`o.update()`), whose reference nothing else will retire.
            object_release::release_if_produced(context, builder, module, expr, &scalar);
            Ok(())
        }
        MirStmt::Assign { target, value } => {
            let ty = value.ty();
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            // Part 1 of #1499: a module-global `object` slot owns its
            // reference (`object_slot.rs`); every other slot keeps
            // `emit_assign`'s store below.
            if object_slot::assign(context, builder, module, rt, locals, target, value, scalar) {
                return Ok(());
            }
            let scalar = incref_if_str_duplicate(builder, rt, value, scalar);
            let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
            if ty == pycc_mir::Ty::Str {
                decref_str_slot_before_store(context, builder, rt, locals, target);
            }
            // Part 2a of #1142 (#1165): D-074's free-before-overwrite, for
            // the second owning slot type. Keyed on `ty` exactly like the
            // `str` arm above; unlike `int`, a buffer slot's declared type
            // and the value's type can never disagree, because the producer
            // is the only expression of the type a local can be bound to.
            if ty == pycc_mir::Ty::MemoryView {
                free_buffer_slot_before_store(context, builder, rt, locals, target);
            }
            // #146 Part 1: the matching `int` release is *not* here. It
            // lives inside `emit_assign`, gated on the target slot's own
            // declared type rather than on `ty` above -- `x: int = True`
            // reaches this arm with `ty == Ty::Bool` and would otherwise
            // skip the release of the bigint the slot still holds.
            emit_assign(context, builder, rt, locals, target, scalar);
            Ok(())
        }
        MirStmt::NoOp => Ok(()),
        MirStmt::ForeignImport { bindings } => {
            foreign_import::emit_stmt(context, builder, module, rt, locals, bindings);
            Ok(())
        }
        MirStmt::Unreachable => {
            builder
                .build_unreachable()
                .expect("build_unreachable should terminate a statically impossible match path");
            Ok(())
        }
        MirStmt::If { test, body, orelse } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            let cond = {
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, test);
                let held = object_release::hold(context, module, rt, test, &scalar);
                let cond = truthy(context, builder, module, rt, scalar);
                held.release(builder, rt);
                // #146 Part 2 (D-181): released *after* `truthy`, which
                // reads a bigint operand's limbs -- releasing first could
                // free the very word being tested.
                release_scalar_if_int_temporary(context, builder, rt, test, &scalar);
                cond
            };
            let then_bb = context.append_basic_block(function, "if_then");
            let merge_bb = context.append_basic_block(function, "if_merge");
            let else_bb = if orelse.is_empty() {
                merge_bb
            } else {
                context.append_basic_block(function, "if_else")
            };
            builder
                .build_conditional_branch(cond, then_bb, else_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(then_bb);
            let then_falls_through = emit_body_then_branch(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                merge_bb,
                expected_return_ty.clone(),
                finally_stack,
            )?;

            let else_falls_through = if orelse.is_empty() {
                true
            } else {
                builder.position_at_end(else_bb);
                emit_body_then_branch(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    orelse,
                    merge_bb,
                    expected_return_ty,
                    finally_stack,
                )?
            };

            builder.position_at_end(merge_bb);
            if !then_falls_through && !else_falls_through {
                builder
                    .build_unreachable()
                    .expect("build_unreachable should terminate a merge with no predecessors");
            }
            Ok(())
        }
        MirStmt::While { test, body } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            let test_bb = context.append_basic_block(function, "while_test");
            let body_bb = context.append_basic_block(function, "while_body");
            let after_bb = context.append_basic_block(function, "while_after");

            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let cond = {
                let scalar = emit_expr(context, builder, module, rt, user_functions, locals, test);
                let held = object_release::hold(context, module, rt, test, &scalar);
                let cond = truthy(context, builder, module, rt, scalar);
                held.release(builder, rt);
                // #146 Part 2 (D-181): released *after* `truthy`, which
                // reads a bigint operand's limbs -- releasing first could
                // free the very word being tested.
                release_scalar_if_int_temporary(context, builder, rt, test, &scalar);
                cond
            };
            builder
                .build_conditional_branch(cond, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(body_bb);
            emit_body_then_branch(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                test_bb,
                expected_return_ty,
                finally_stack,
            )?;

            builder.position_at_end(after_bb);
            Ok(())
        }
        MirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            let (start_v, stop_v, step_v) = emit_range_operands_with_exception_safety(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                start,
                stop,
                step,
            );
            // #146 Part 1, the `ForRange` ownership contract.
            //
            // `range_operand_to_normalized_int` passes a bigint operand
            // through `pycc_rt_range_normalize_operand` *unchanged*, so
            // `start_v`/`stop_v`/`step_v` alias whatever the source
            // expression already owned -- for `for i in range(b, b, b)`,
            // one single heap object under three names. None of the three
            // has been retained by `retain_if_int_duplicate`: that helper
            // is deliberately not applied to `range` operands, and the
            // three retains below are the loop's own.
            //
            // WARNING for a later change: do *not* route `range`'s operands
            // through `retain_if_int_duplicate` as well. `start_v` would
            // then be retained twice while still being released once, by
            // the ordinary `current` machinery -- a permanent leak of every
            // named `range` bound, invisible to every value assertion.
            //
            // `stop_v` and `step_v` are read on every trip through
            // `for_test` and are released once in `for_after`. `start_v` is
            // retained here as `current`'s *first owner*, not as a third
            // independent operand: on the first trip the induction phi's
            // incoming value *is* `start_v`, so the ordinary per-iteration
            // `current` release below (or `for_after`'s final one, for an
            // empty range) is what balances this retain. Adding a third
            // `for_after` release for `start_v` would free a word the
            // source name still holds.
            //
            // The whole invariant, for `n` executed iterations: `n + 1`
            // owned `current` values (`start_v` plus one `pycc_rt_int_add`
            // result per iteration), matched by `n` per-iteration releases
            // plus one `for_after` release. Separately, each of the `n`
            // binds of the visible target retains `current`, matched by the
            // *next* bind's release-before-store inside `emit_assign`, with
            // the final one matched whenever that slot is next overwritten.
            emit_bigint_refcount_call(context, builder, rt, start_v, BigIntRefcount::Retain);
            emit_bigint_refcount_call(context, builder, rt, stop_v, BigIntRefcount::Retain);
            emit_bigint_refcount_call(context, builder, rt, step_v, BigIntRefcount::Retain);
            // #146 Part 2 (D-181): retire the *source expression's* birth
            // reference for any operand that was freshly built rather than
            // borrowed (`range(a + a, ...)`, or a literal outside D-061's
            // tagged range). Safe immediately, and only immediately after
            // the loop's own retains directly above: the loop is already an
            // owner by this point, so the object survives to `for_after`.
            // Deliberately does *not* conditionalize those retains -- D-180
            // warns that making the loop's ownership depend on the operand's
            // shape is what breaks the `n + 1`/`n + 1` release arithmetic.
            release_if_int_temporary(context, builder, rt, start, start_v);
            release_if_int_temporary(context, builder, rt, stop, stop_v);
            release_if_int_temporary(context, builder, rt, step, step_v);
            // Re-read *after* the retains above: each one splits the block,
            // and the phi's incoming edge must name the block that actually
            // branches into `for_test`.
            let preheader = builder.get_insert_block().unwrap();

            let test_bb = context.append_basic_block(function, "for_test");
            let body_bb = context.append_basic_block(function, "for_body");
            let after_bb = context.append_basic_block(function, "for_after");

            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let induction = builder
                .build_phi(context.i64_type(), "for_current")
                .expect("build_phi should not fail in a fresh loop-test block");
            induction.add_incoming(&[(&start_v, preheader)]);
            let current = induction.as_basic_value().into_int_value();
            let cont = builder
                .build_call(
                    rt.range_continue,
                    &[current.into(), stop_v.into(), step_v.into()],
                    "range_continue",
                )
                .expect("build_call should not fail for a well-formed range_continue check")
                .try_as_basic_value()
                .expect_basic("pycc_rt_range_continue returns a non-void i8")
                .into_int_value();
            let cont_i1 = builder
                .build_int_compare(
                    IntPredicate::NE,
                    cont,
                    context.i8_type().const_int(0, false),
                    "for_cont",
                )
                .expect("build_int_compare should not fail comparing two i8 operands");
            builder
                .build_conditional_branch(cont_i1, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(body_bb);
            // Python binds a `for` target only after an element actually
            // exists. The visible target is separate from the hidden SSA
            // induction value so an empty range leaves it unbound, the final
            // target remains the last element, and body reassignment cannot
            // corrupt the next iteration.
            // Retain before `emit_assign`, which releases the slot's
            // previous word: when the slot already holds this very object
            // (a body that does not reassign the target cannot produce
            // that, but an aliasing source such as `b` can), releasing
            // first would free the word this bind is about to store.
            emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Retain);
            emit_assign(context, builder, rt, locals, var, Scalar::Int(current));
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                expected_return_ty,
                finally_stack,
            )?;
            // `ForRange`'s own inline copy of `emit_body_then_branch`'s
            // terminator-safety guard (see that function's own doc comment
            // for why): a `Return` reached inside `body` already terminates
            // `body_bb`, so the increment-and-branch-back below must be
            // skipped in that case -- building it anyway would try to add a
            // second terminator onto an already-terminated block, which is
            // invalid LLVM IR.
            if builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                let next = builder
                    .build_call(rt.int_add, &[current.into(), step_v.into()], "for_next")
                    .expect("build_call should not fail for a well-formed int add")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_int_add returns a non-void i64")
                    .into_int_value();
                // This iteration's `current` is dead once `next` exists.
                // A `Return` inside `body` skips this release (and both
                // `for_after` releases below) and leaks -- an accepted,
                // documented Part 1 concession, not an oversight.
                emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
                // Re-read after the release: it splits the block, and the
                // phi's incoming edge must name the block that branches.
                let body_end = builder.get_insert_block().unwrap();
                induction.add_incoming(&[(&next, body_end)]);
                builder.build_unconditional_branch(test_bb).expect(
                    "build_unconditional_branch should not fail on a block with no terminator yet",
                );
            }

            builder.position_at_end(after_bb);
            // The `current` that failed `range_continue` was never bound to
            // the visible target and has no per-iteration release; plus the
            // two operand retains from the preheader.
            emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
            emit_bigint_refcount_call(context, builder, rt, stop_v, BigIntRefcount::Release);
            emit_bigint_refcount_call(context, builder, rt, step_v, BigIntRefcount::Release);
            Ok(())
        }
        // An intentional inline duplicate of `ForRange`'s loop-building
        // logic directly above -- exactly as that arm is itself a
        // deliberate inline copy of `emit_body_then_branch`'s (see both
        // their own comments). Not factored into a shared helper: the two
        // arms differ in their bound, their induction step, and their
        // per-iteration prologue, and this file's established position is
        // that a third consumer, not a second, is what justifies extracting
        // shared loop-building machinery.
        //
        // Three real differences from `ForRange`, all of them consequences
        // of iterating a container rather than an arithmetic range:
        //
        // 1. The induction variable is a plain, raw LLVM `i64` index, never
        //    a D-061-tagged `Ty::Int` -- so the increment is
        //    `build_int_add`, not `pycc_rt_int_add`, and the loop test is a
        //    plain `icmp slt`, not `pycc_rt_range_continue`. Both of those
        //    runtime functions take *tagged* operands (D-061); calling
        //    either with this raw counter would silently compute on the
        //    wrong values.
        // 2. The bound is `pycc_rt_int_list_len`, re-read on every
        //    iteration inside the test block rather than hoisted into the
        //    preheader. This matches CPython, whose list iterator compares
        //    its cursor against the list's *current* length on every
        //    `__next__` -- so `for v in xs: xs.append(v)` runs forever
        //    there, and would terminate here if the length were hoisted.
        // 3. Each iteration prepends one `pycc_rt_int_list_get` read.
        //
        // D-141 leaves the induction variable and raw `len` as private LLVM
        // counters. The element read is already encoded and is assigned to
        // the user-visible `Ty::Int` loop target unchanged.
        MirStmt::ForList { var, list, body } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            // Read once, in the preheader, not per iteration: Python binds
            // its iterator to the object the `for` statement evaluated, so
            // a body-level rebinding of the same name (`xs = [9]`) must not
            // retarget the loop.
            //
            // That makes `list_ptr` a *borrowed* reference held across
            // arbitrary body code without an incref, which is sound only
            // because D-107 keeps `list[T]` leak-only in v0.2 -- nothing
            // frees a list. Whichever future PR wires D-107's own
            // reassignment-cleanup site must give this read an incref/decref
            // pair at the same time, or exactly the rebinding shape above
            // would free the object out from under the loop.
            let list_ptr =
                emit_list_name_read(context, builder, module, rt, user_functions, locals, list);
            let preheader = builder.get_insert_block().unwrap();

            let test_bb = context.append_basic_block(function, "for_list_test");
            let body_bb = context.append_basic_block(function, "for_list_body");
            let after_bb = context.append_basic_block(function, "for_list_after");

            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let induction = builder
                .build_phi(context.i64_type(), "for_list_index")
                .expect("build_phi should not fail in a fresh loop-test block");
            let zero = context.i64_type().const_zero();
            induction.add_incoming(&[(&zero, preheader)]);
            let current = induction.as_basic_value().into_int_value();
            let len = build_int_list_len(builder, rt, list_ptr);
            let cont = builder
                .build_int_compare(IntPredicate::SLT, current, len, "for_list_cont")
                .expect("build_int_compare should not fail comparing two i64 operands");
            builder
                .build_conditional_branch(cont, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(body_bb);
            // Same Python target-binding semantics `ForRange`'s own
            // comment describes: the visible target is a separate storage
            // slot written once per iteration, so the final target keeps
            // the last element and body reassignment cannot corrupt the
            // next iteration (both pinned by
            // `list_targets_keep_the_last_element_and_ignore_body_reassignment`
            // in `tests/slice1_codegen_depth.rs`). `ForRange`'s third
            // property -- an empty sequence leaving the target unbound --
            // holds here structurally for the same reason, and since #1021
            // (D-245) it *is* reachable from real source: `xs: list[int] =
            // []` now resolves its element type from the annotation instead
            // of failing, so `for x in xs` over an empty list compiles and
            // runs zero iterations (pinned by
            // `iterating_an_inferred_empty_list_runs_zero_iterations` in
            // `tests/issue_1021_empty_container_inference.rs`).
            let encoded_element = build_int_list_get(builder, rt, list_ptr, current);
            let element = encoded_element;
            emit_assign(context, builder, rt, locals, var, Scalar::Int(element));
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                expected_return_ty,
                finally_stack,
            )?;
            // `ForRange`'s own terminator-safety guard, for the identical
            // reason (see that arm's comment): a `Return` inside `body`
            // already terminated `body_bb`, and adding the increment and
            // back-edge anyway would build a second terminator on it.
            if builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                let next = builder
                    .build_int_add(
                        current,
                        context.i64_type().const_int(1, false),
                        "for_list_next",
                    )
                    .expect("build_int_add should not fail for two i64 operands");
                let body_end = builder.get_insert_block().unwrap();
                induction.add_incoming(&[(&next, body_end)]);
                builder.build_unconditional_branch(test_bb).expect(
                    "build_unconditional_branch should not fail on a block with no terminator yet",
                );
            }

            builder.position_at_end(after_bb);
            Ok(())
        }
        // PR 3c of #1082: `for x in o.attr:` / `for x in o.method(...):`.
        // `foreign_call::emit_iter_loop` builds the preheader and header --
        // the `pycc_ext_obj_get_iter` call with its NULL failure edge, and
        // the `pycc_ext_obj_iter_next` call with the three-way switch whose
        // default is a second failure edge -- and leaves the builder in the
        // body block with this iteration's item loaded. Only the target
        // store, the body and the back-edge are emitted here.
        MirStmt::ForObject { var, iter, body } => {
            let iterable = emit_expr(context, builder, module, rt, user_functions, locals, iter);
            let held = object_release::hold(context, module, rt, iter, &iterable);
            let (loop_blocks, iterator) =
                foreign_call::emit_iter_loop(context, builder, module, rt, iterable, held);
            // The item is `pycc_ext_obj_iter_next`'s new reference. Part 1
            // of #1499: a module-global target owns it, so each trip
            // releases the previous item (or the value bound before the
            // loop) and the slot keeps the last one, as in CPython. A frame
            // slot keeps the plain, leak-only store until Part 2 (#1502).
            let slot = locals
                .get(var)
                .expect("every for-target must have a predeclared storage slot");
            object_slot::store_new_reference(context, builder, module, rt, slot, loop_blocks.item);
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                expected_return_ty,
                finally_stack,
            )?;
            // `ForList`'s own terminator-safety guard, for the identical
            // reason: a `Return` inside `body` already terminated the block
            // and a second terminator would be invalid IR.
            if builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                builder.build_unconditional_branch(loop_blocks.header_bb).expect(
                    "build_unconditional_branch should not fail on a block with no terminator yet",
                );
            }
            builder.position_at_end(loop_blocks.after_bb);
            // Part 3 of #1092: the normal exit releases the iterator; every
            // exception edge already did through its cleanup target.
            iterator.exit(builder, rt);
            Ok(())
        }
        // Part 2 of #1175 (#1179): `return b[start:stop]` for a
        // `memoryview` **parameter** `b`.
        //
        // The two bounds are evaluated here, in the compiled frame, and
        // written to the three trailing out-pointers the declaration pass
        // gave this function. The returned *value* is the parameter's own
        // whole view, byte-identical to what a bare `return b` returns, so
        // the generated wrapper's `result == &a{index}` identity chain
        // stays exact and Part 1's PEP 688 `held`-vs-probe check still runs
        // against the whole window before the host derives the sub-range.
        //
        // `has_slice` is stored **last**, after both bounds, so a bound
        // whose evaluation raises can never leave the flag set; on that
        // path the frame leaves through its exception target and the
        // wrapper's pending-exception bail returns before any acquire.
        //
        // An absent `start` is `0` and an absent `stop` is `i64::MAX`, both
        // of which CPython's own `PySlice_New`/`PyObject_GetItem` clamp
        // exactly as Python does; no slice arithmetic exists on this side.
        // A bigint bound aborts in `build_untag_checked`, the same
        // pre-existing D-141 boundary `MirStmt::BufferSet`'s index has.
        MirStmt::ReturnBufferSlice { name, start, stop } => {
            let bound_or = |builder: &inkwell::builder::Builder<'ctx>,
                            locals: &mut HashMap<String, StorageSlot<'ctx>>,
                            bound: &Option<MirExpr>,
                            default: u64,
                            label: &str| match bound {
                Some(expr) => {
                    let scalar =
                        emit_expr(context, builder, module, rt, user_functions, locals, expr);
                    let encoded = to_numeric_encoded_int(context, builder, scalar);
                    build_untag_checked(builder, rt, encoded, label)
                }
                None => context.i64_type().const_int(default, false),
            };
            let start_i64 = bound_or(builder, locals, start, 0, "buffer_slice_untag_start");
            let stop_i64 = bound_or(
                builder,
                locals,
                stop,
                i64::MAX as u64,
                "buffer_slice_untag_stop",
            );
            // The three out-pointers were bound at function entry, under
            // the reserved keys, exactly when the declaration pass widened
            // this name -- which it did, or this statement could not be
            // here.
            //
            // `has_slice` is **taken** out of `locals` rather than read
            // through it, for the duration of the delegated bare `return`
            // below: that arm reads the key's presence as "this frame owes
            // the wrapper a `has_slice = 0`", and this return's own answer
            // is 1. Restored immediately afterwards, so a later return in
            // the same frame still describes itself.
            let has_slice = locals.remove(BUFFER_SLICE_HAS_SLICE_KEY).expect(
                "a frame containing a buffer sub-range return was widened by the declaration pass",
            );
            builder
                .build_store(locals[BUFFER_SLICE_START_KEY].ptr, start_i64)
                .expect("build_store should not fail through a buffer-slice out-pointer");
            builder
                .build_store(locals[BUFFER_SLICE_STOP_KEY].ptr, stop_i64)
                .expect("build_store should not fail through a buffer-slice out-pointer");
            builder
                .build_store(has_slice.ptr, context.i64_type().const_int(1, false))
                .expect("build_store should not fail through a buffer-slice out-pointer");
            // Delegated rather than duplicated: the value half of this
            // statement *is* a bare `return b`, including its `finally`
            // routing and its interaction with the frame's owned-slot
            // epilogue, and a second copy of that arm is exactly the drift
            // this node exists to avoid.
            let emitted = emit_stmt(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                &MirStmt::Return(Some(MirExpr::Name {
                    name: name.clone(),
                    ty: pycc_mir::Ty::MemoryView,
                })),
                expected_return_ty,
                finally_stack,
            );
            locals.insert(BUFFER_SLICE_HAS_SLICE_KEY.to_string(), has_slice);
            emitted
        }
        MirStmt::Return(value) => {
            if is_module_entry_symbol(
                builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap()
                    .get_name()
                    .to_bytes(),
            ) {
                panic!(
                    "pycc_codegen: internal error: a top-level statement terminated `main`'s \
                     entry block -- pycc_types::check (T0024) should have rejected a module-level \
                     `return` before it reached codegen"
                );
            }
            // Part 2 of #1175 (#1179), review round 2: in a frame the
            // declaration pass widened, **every** return describes itself.
            // This one hands the whole view back, so its answer is
            // `has_slice = 0`.
            //
            // Without this store the out-slots would describe "the last
            // return that executed a store" rather than "the return that
            // reached the wrapper", and the wrapper's own zero-init of the
            // flag would be load-bearing in a way it cannot be: a frame can
            // execute more than one `return`. `try: return b[1:3]` /
            // `finally: raise` / an enclosing `except` that swallows it /
            // `return b` left the abandoned sub-range's bounds in the slots
            // and applied them to the whole-buffer return.
            //
            // Stored before the value is evaluated, so a returned
            // expression that raises cannot leave a stale 1 behind either;
            // the wrapper bails on the pending exception before reading
            // either way, and 0 is the safe answer regardless.
            //
            // Absent for every frame that was not widened, which is every
            // frame in a build that has no buffer sub-range return at all.
            if let Some(has_slice) = locals.get(BUFFER_SLICE_HAS_SLICE_KEY) {
                builder
                    .build_store(has_slice.ptr, context.i64_type().const_zero())
                    .expect("build_store should not fail through a buffer-slice out-pointer");
            }
            // #382 (PR-22 Part 2): If inside a try-with-finally, route the
            // return through the finally block instead of emitting `ret`
            // directly. Store the return value to the finally's ret_slot,
            // set the is_returning flag, and branch to the finally block.
            // After the finally body runs, the codegen emits the `ret`.
            let finally_target = finally_stack.last().cloned();
            // #1387: in a function returning `object` (D-258), a bare
            // `return` means `return None` -- see `object_return` -- and,
            // since Part 2 of #1387, a native returned value (`None`
            // included) is boxed into the `object` the host receives -- see
            // `object_box`.
            let bare_none = MirExpr::NoneLiteral;
            let value = object_return::object_return_value(&expected_return_ty, value, &bare_none);
            match value {
                Some(expr) => {
                    let scalar = if object_box::boxes_into(expr, &expected_return_ty) {
                        Scalar::Object(object_box::emit_boxed(
                            context,
                            builder,
                            module,
                            rt,
                            user_functions,
                            locals,
                            expr,
                        ))
                    } else {
                        emit_expr(context, builder, module, rt, user_functions, locals, expr)
                    };
                    let scalar = incref_if_str_duplicate(builder, rt, expr, scalar);
                    let scalar = retain_if_int_duplicate(context, builder, rt, expr, scalar);
                    let scalar =
                        coerce_scalar_to_type(context, builder, scalar, expected_return_ty.clone());
                    // Part 2b of #1142 (#1164): a returned artifact-owned
                    // buffer leaves through the frame's owned-slot
                    // epilogue, and **nothing about ownership happens
                    // here**. This statement only makes the value pending:
                    // when `finally_stack` is non-empty the value is stored
                    // to the innermost frame's `ret_slot` and control is
                    // routed through every interposed user finalizer before
                    // it reaches the epilogue. Clearing the slot here --
                    // which is what this arm used to do -- made the slot
                    // stop naming live storage while those finalizers were
                    // still running, so `finally: a[0] = 42.0` dereferenced
                    // null and aborted the hosting interpreter, and a
                    // finalizer that raised or returned a buffer of its own
                    // abandoned the pointer this statement had disowned.
                    //
                    // The transfer therefore lives in the epilogue, which is
                    // the one point control actually leaves the frame; see
                    // the `buffer_epilogue_returned` block there for how it
                    // decides which slot not to release.
                    //
                    // What this statement does record is that the pointer is
                    // now *pending*: a finalizer that rebinds the name it
                    // came from would otherwise release it out from under the
                    // host, so `free_buffer_slot_before_store` reads this
                    // record to keep its hands off. Written for every
                    // `memoryview` return shape rather than only a bare
                    // `Name`, since the epilogue's own identity test makes a
                    // record that no slot ever held a harmless no-op.
                    //
                    // `(pending, orphaned)` is **one two-field state** with a
                    // coupling invariant: `orphaned != 0` means the pending
                    // record is the pointer's *sole* owner, so nothing else
                    // will release it. The record therefore has exactly two
                    // mutators, and each must leave that invariant standing.
                    // `free_buffer_slot_before_store` is the other one, and it
                    // raises the flag in the same breath that it makes the
                    // record the sole owner.
                    //
                    // This one supersedes the record, so its transition is
                    // total: release the predecessor when this frame is its
                    // only owner, store the new pointer, and clear the flag.
                    // A raw store here -- which is what this arm used to do --
                    // left both halves of the invariant broken whenever one
                    // frame reached a second `return`: the superseded
                    // pointer's only record was gone, leaking one buffer per
                    // call, and the surviving flag then described a pointer
                    // that was no longer pending, so the epilogue's trailing
                    // release freed the *new* pointer that the owned-slot loop
                    // had already released -- a double free that aborts the
                    // hosting interpreter.
                    //
                    // A second `return` is still reachable after review round
                    // 5 narrowed the egress admission
                    // (`pycc_types::buffer::buffer_return_inside_finally`
                    // now refuses a buffer egress from any function with a
                    // `return` inside a `finally`). That narrowing removes
                    // only the *simultaneous* case -- two returns in flight at
                    // once -- because that requires the inner `return` to sit
                    // lexically inside a `finally` the outer one's exit path
                    // runs. The *sequential* case survives and is what this
                    // transition serves: `try: return a` / `finally: raise`,
                    // caught by an enclosing handler, then `a = ndarray(n)`
                    // (which orphans the abandoned pointer) and a second
                    // `return a`. `cancelled_then_rebind` and
                    // `cancelled_then_rebind_then_raises` in
                    // `tests/issue_1164_memoryview_egress.rs` are that shape;
                    // each kills one half of this transition, the release and
                    // the flag clear respectively.
                    //
                    // With one invariant-preserving mutator, "how many
                    // syntactic paths reach a second `return`" stops being a
                    // question correctness depends on: the state is restored at
                    // every write, so a third and fourth write are the same
                    // transition applied again.
                    //
                    // Clearing the flag unconditionally is sound because the
                    // new pointer is, at this instant, held by a live local
                    // slot: an owned buffer originates only at `a = ndarray(n)`
                    // bound to a simple local name, and the read admitted at
                    // this return position loads it back out of that slot.
                    // "Not orphaned" is therefore true by construction of
                    // every pending store.
                    if let Scalar::MemoryView(returned) = scalar
                        && let Some(pending) = locals.get(PENDING_RETURN_BUFFER_KEY)
                    {
                        let superseded = builder
                            .build_load(
                                context.ptr_type(inkwell::AddressSpace::default()),
                                pending.ptr,
                                "superseded_return_buffer",
                            )
                            .expect("build_load should not fail for the pending-return slot")
                            .into_pointer_value();
                        let orphaned = &locals[PENDING_RETURN_ORPHANED_KEY];
                        let was_orphaned = builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                builder
                                    .build_load(
                                        context.i8_type(),
                                        orphaned.ptr,
                                        "superseded_return_orphaned",
                                    )
                                    .expect(
                                        "build_load should not fail for the pending-return orphan flag",
                                    )
                                    .into_int_value(),
                                context.i8_type().const_zero(),
                                "superseded_return_was_orphaned",
                            )
                            .expect("build_int_compare should not fail for an i8 flag");
                        // Both halves again. Gating on the flag rather than on
                        // "a different pointer" is what keeps an unorphaned
                        // predecessor -- still held by its own slot, which the
                        // epilogue's loop will release -- from being freed
                        // twice; a `return a` abandoned by a raising
                        // finalizer, caught, and followed by a second `return
                        // b` on a still-slot-held `b` is that shape. The
                        // inequality makes
                        // the symmetric state unreachable rather than merely
                        // improbable: an orphaned record is by definition held
                        // by no slot, so no later `return` can load it back and
                        // name it again, but one `icmp` removes the dependence
                        // on that argument entirely.
                        let differs = builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                superseded,
                                returned,
                                "superseded_return_differs",
                            )
                            .expect("build_int_compare should not fail for two pointers");
                        let stale = builder
                            .build_and(was_orphaned, differs, "superseded_return_is_stale")
                            .expect("build_and should not fail for two i1 values");
                        // Branch-free on the `select`-plus-null-free model the
                        // epilogue and the free-before-store both already use,
                        // so a single-`return` frame keeps the straight-line
                        // shape it has today and pays one extra no-op free.
                        let release = builder
                            .build_select(
                                stale,
                                superseded,
                                context
                                    .ptr_type(inkwell::AddressSpace::default())
                                    .const_null(),
                                "superseded_return_release",
                            )
                            .expect("build_select should not fail for two pointers")
                            .into_pointer_value();
                        builder
                            .build_call(
                                rt.buffer_f64_free,
                                &[release.into()],
                                "superseded_return_free",
                            )
                            .expect("build_call should not fail for pycc_rt_buffer_f64_free");
                        builder
                            .build_store(pending.ptr, returned)
                            .expect("build_store should not fail for the pending-return slot");
                        builder
                            .build_store(orphaned.ptr, context.i8_type().const_zero())
                            .expect(
                                "build_store should not fail for the pending-return orphan flag",
                            );
                    }
                    if expected_return_ty == pycc_mir::Ty::None {
                        // `None` parameters, call results, and stored names
                        // use a canonical `i8 0` carrier inside expressions,
                        // but a function
                        // declared to return `None` has an LLVM `void`
                        // signature. Evaluating above preserves any call or
                        // name-load side effects; the carrier itself is
                        // intentionally discarded here. Returning it as an
                        // `i8` previously built invalid IR that only the
                        // non-Windows verifier caught (D-029).
                        if let Some(ft) = finally_target {
                            builder
                                .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                                .expect("build_store should not fail for is_returning");
                            builder.build_unconditional_branch(ft.finally_bb).expect(
                                "build_unconditional_branch should not fail for finally routing",
                            );
                        } else {
                            builder
                                .build_return(None)
                                .expect("build_return should not fail for a None return value");
                        }
                        return Ok(());
                    }
                    let basic_value: inkwell::values::BasicValueEnum = match scalar {
                        Scalar::Int(v) => v.into(),
                        Scalar::Bool(v) => v.into(),
                        Scalar::Float(v) => v.into(),
                        Scalar::Str(v) => v.into(),
                        // Pass-through, identical to `Str`'s arm directly
                        // above: returning a `list[T]` returns one opaque
                        // pointer, and `ty_to_basic_type` already gave the
                        // function's LLVM signature the same pointer
                        // return type it gives a `str`-returning one.
                        Scalar::List(v) => v.into(),
                        // Pass-through, identical to `List`'s arm directly
                        // above: returning a `dict[K, V]` returns one
                        // opaque pointer, and `ty_to_basic_type` already
                        // gave the function's LLVM signature the same
                        // pointer return type it gives a `str`/`list[T]`-
                        // returning one.
                        Scalar::Dict(v) => v.into(),
                        // Pass-through, identical to `List`'s/`Dict`'s arms
                        // directly above: returning a `set[T]` returns one
                        // opaque pointer, and `ty_to_basic_type` already
                        // gave the function's LLVM signature the same
                        // pointer return type it gives a
                        // `str`/`list[T]`/`dict[K, V]`-returning one.
                        Scalar::Set(v) => v.into(),
                        // Pass-through like the three arms above, but
                        // returning a whole struct by value rather than one
                        // opaque pointer (D-115) -- and
                        // `ty_to_basic_type` already gave this function's
                        // LLVM signature that same struct return type, so
                        // the `ret` is well-typed with no tuple-specific
                        // code here.
                        Scalar::Tuple(v) => v.into(),
                        // Pass-through, identical to `List`'s/`Dict`'s/
                        // `Set`'s arms above (D-154, Part 1 of #375):
                        // returning a class instance returns one opaque
                        // pointer, and `ty_to_basic_type` already gave the
                        // function's LLVM signature the same pointer return
                        // type it gives a `str`/`list[T]`/`dict[K, V]`/
                        // `set[T]`-returning one.
                        Scalar::Instance(v) => v.into(),
                        // Pass-through by VALUE, identical in kind to
                        // `Tuple`'s arm above (D-197, #763, Part 1 of
                        // #747): `coerce_scalar_to_type` above already
                        // built the correctly-typed struct against
                        // `expected_return_ty`, and `ty_to_basic_type`
                        // already gave the function's LLVM signature that
                        // same struct return type.
                        Scalar::Optional(v) => v.into(),
                        // Pass-through, identical to `List`'s/`Dict`'s/
                        // `Set`'s/`Instance`'s arms above (D-244, Part 2 of
                        // #1026): returning a foreign CPython object
                        // returns one opaque `PyObject *`, and
                        // `ty_to_basic_type`'s own `Ty::Object` arm already
                        // gave the function's LLVM signature the same
                        // pointer return type. This arm is genuinely
                        // reachable: an unannotated private
                        // helper whose body is `return numpy.pi` infers a
                        // `Ty::Object` return type, which is the second
                        // producer shape Part 2's refusal migration admits.
                        // No refcount traffic accompanies it: a returned
                        // object is still leak-only (Part 2 of #1499), see
                        // `Scalar`'s own `Object` doc comment.
                        Scalar::Object(v) => v.into(),
                        // Part 2b of #1142 (#1164): pass-through by
                        // pointer, identical in kind to `List`'s and
                        // `Instance`'s arms above -- `ty_to_basic_type`'s
                        // own `Ty::MemoryView` arm already gave the
                        // function's LLVM signature the same pointer return
                        // type, and the value is the `PyccExtBufferView *`
                        // `pycc_rt_buffer_f64_alloc` produced.
                        //
                        // Two provenances reach here. *Artifact-owned*
                        // storage is the original one. Part 1 of #1175
                        // (#1178) adds the *caller-owned* one: a
                        // `memoryview` **parameter** returned by name,
                        // whose value is the address of the wrapper's own
                        // `PyccExtBufferView` local. The earlier refusal of
                        // that shape was justified here as a dangling view
                        // -- the host's exporter being released on the way
                        // out -- and that justification was wrong: the
                        // wrapper acquires a second, independent buffer
                        // export on the argument object before releasing
                        // its own, and an export is what pins the storage.
                        // See `pycc_types::buffer::admits_buffer_egress`.
                        //
                        // Nothing below has to distinguish the two. A
                        // parameter never enters `owned_buffer_slots` (see
                        // the comment at the slot-collection site), and a
                        // local buffer slot only ever holds null or a
                        // `pycc_rt_buffer_f64_alloc` heap pointer, never a
                        // parameter's address -- so the orphan flag cannot
                        // fire against a caller-owned pending return and
                        // the epilogue cannot free one. This arm stores
                        // nothing into the
                        // slot: the ownership transfer happens later, in
                        // the frame's owned-slot epilogue, where the
                        // `buffer_epilogue_returned` identity test skips
                        // exactly the pointer being returned and releases
                        // every other owned slot. Clearing the slot here
                        // instead is what made a `finally` block observe a
                        // null name and abort the host.
                        Scalar::MemoryView(v) => v.into(),
                    };
                    if let Some(ft) = finally_target {
                        // Route through finally: store the return value,
                        // set the is_returning flag, and branch to finally.
                        let slot = ft
                            .ret_slot
                            .expect("a value return routed through finally has a return slot");
                        builder
                            .build_store(slot, basic_value)
                            .expect("build_store should not fail for ret_slot");
                        builder
                            .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                            .expect("build_store should not fail for is_returning");
                        builder.build_unconditional_branch(ft.finally_bb).expect(
                            "build_unconditional_branch should not fail for finally routing",
                        );
                    } else {
                        builder
                            .build_return(Some(&basic_value))
                            .expect("build_return should not fail for a well-formed return value");
                    }
                }
                None => {
                    // #380 (PR-20): an abstract method has a `Return(None)`
                    // body but a non-`None` declared return type. The type
                    // checker skips body checking for abstract methods, so
                    // this is the only case where `Return(None)` reaches
                    // codegen with a non-`None` `expected_return_ty`. Emit
                    // a default value of the correct type so the LLVM IR
                    // is well-typed; the abstract method is never called,
                    // so the value doesn't matter.
                    if expected_return_ty == pycc_mir::Ty::None {
                        if let Some(ft) = finally_target {
                            builder
                                .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                                .expect("build_store should not fail for is_returning");
                            builder.build_unconditional_branch(ft.finally_bb).expect(
                                "build_unconditional_branch should not fail for finally routing",
                            );
                        } else {
                            builder
                                .build_return(None)
                                .expect("build_return should not fail for a bare `return`");
                        }
                    } else {
                        let default = default_value_for_type(context, expected_return_ty.clone());
                        if let Some(ft) = finally_target {
                            let slot = ft.ret_slot.expect(
                                "a non-None default return routed through finally has a return slot",
                            );
                            builder
                                .build_store(slot, default)
                                .expect("build_store should not fail for ret_slot");
                            builder
                                .build_store(ft.is_returning, context.i8_type().const_int(1, false))
                                .expect("build_store should not fail for is_returning");
                            builder.build_unconditional_branch(ft.finally_bb).expect(
                                "build_unconditional_branch should not fail for finally routing",
                            );
                        } else {
                            builder
                                .build_return(Some(&default))
                                .expect("build_return should not fail for a default return value");
                        }
                    }
                }
            }
            Ok(())
        }
        // `b[i] = v` on a `memoryview` parameter (Part 1 of #1142): the
        // store counterpart of `MirExpr::BufferGet` above, sharing its
        // base extraction and its index decoding verbatim so the load and
        // the store can never disagree about either.
        //
        // The `guard_statement_effects` call is load-bearing and is
        // deliberately *not* copied from `MirStmt::DictSet` below, which
        // has none: `pycc_rt_dict_set` cannot raise, while
        // `pycc_rt_buffer_f64_set` raises `IndexError` exactly as the load
        // does. The load lets `expression_can_set_exception` guard the
        // expression that consumes its `0.0` sentinel; a `void` setter has
        // no consumer, so the statement-level guard is the *only* thing
        // that ever observes the raise. Omit it and an out-of-range store
        // as a function's last statement would return normally. The
        // precedent copied here is `MirStmt::Call`'s own guard above.
        //
        // A *sub-expression* that raises is already handled before the
        // write: `emit_expr` guards any operand for which
        // `expression_can_set_exception` holds -- a `b[0] = b[99]` load
        // among them -- and branches away before this arm reaches
        // `buffer_f64_set`, so no write lands on a sentinel operand. The
        // one unguarded case is the bigint index, which
        // `build_untag_checked` aborts on: that is the pre-existing D-141
        // boundary `MirExpr::BufferGet` and `MirStmt::DictSet` already sit
        // on (`docs/RUNTIME.md`'s object-model "still aborts" line), not
        // something the store introduces.
        MirStmt::BufferSet { base, index, value } => {
            // CPython's assignment order, which is *not* left to right: for
            // `b[i] = v` the interpreter evaluates the right-hand side
            // first, then the subscription target, then the index. A
            // `d[i()] = v()` probe on CPython prints `v`'s effect before
            // `i`'s, so emitting the index first would let a raising or
            // side-effecting index expression suppress a value expression
            // CPython would already have run. The three `emit_expr` calls
            // below are therefore in value, base, index order.
            //
            // The neighbouring `MirStmt::DictSet` arm has the same ordering
            // defect -- it emits its key before its value. That is
            // pre-existing and out of scope here, and is worth its own issue
            // rather than a silent fix in this pull request, so it is left
            // exactly as it stands.
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            // `pycc_types` admits only a `Ty::Float` value here, so this is
            // the identity arm of `to_float`; it is used rather than an
            // inline `Scalar::Float` destructure so the store carries no
            // panic the load does not.
            let element = to_float(context, builder, rt, value_scalar);
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let Scalar::MemoryView(base_ptr) = base_scalar else {
                panic!(
                    "pycc_codegen: internal error: a buffer element store's base did not \
                     evaluate to a memoryview"
                )
            };
            let index_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, index);
            let encoded_index = to_numeric_encoded_int(context, builder, index_scalar);
            let raw_index = build_untag_checked(builder, rt, encoded_index, "buffer_untag_index");
            builder
                .build_call(
                    rt.buffer_f64_set,
                    &[base_ptr.into(), raw_index.into(), element.into()],
                    "buffer_set",
                )
                .expect("build_call should not fail for a declared runtime function");
            guard_statement_effects(context, builder, rt);
            Ok(())
        }
        // `d[k] = v` (PR-11 Task 5, D-123): insert-or-update --
        // `pycc_rt_dict_set` itself decides which, by whether `key`
        // already compares equal to a stored key. `dict` is read by name
        // (like a bare-name `MirExpr::ListAppend` receiver), `key` crosses
        // in as a `Ty::Str` expression unchanged, exactly like
        // `MirExpr::DictGet`'s key, and `value` gets D-141 validation plus
        // identity-preserving encoded storage.
        MirStmt::DictSet { dict, key, value } => {
            let dict_ptr =
                emit_dict_name_read(context, builder, module, rt, user_functions, locals, dict);
            let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
            // Same `incref_if_str_duplicate` requirement as `MirExpr::
            // DictLiteral`'s own per-pair key above, and for the identical
            // reason (see that arm's own comment): `pycc_rt_dict_set`
            // adopts whatever key pointer it is given as `d`'s own
            // permanent reference without incref'ing it itself (D-124), so
            // a bare-`Name` key (`d[k] = v`) must be incref'd here first,
            // or a later reassignment of `k` would decref -- and
            // potentially free -- the same `PyStrObj` `d` still points to.
            let key_scalar = incref_if_str_duplicate(builder, rt, key, key_scalar);
            let Scalar::Str(key_ptr) = key_scalar else {
                panic!(
                    "pycc_codegen: internal error: dict item-assignment key did not evaluate to \
                     str -- pycc_types::check (T0021) should have rejected this before codegen"
                )
            };
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let encoded = to_encoded_int(context, builder, value_scalar);
            let _ = build_untag_checked(builder, rt, encoded, "dict_validate_set_value");
            build_dict_set(builder, rt, dict_ptr, key_ptr, encoded);
            Ok(())
        }
        // Part 2c of #1371: `del o[a:b:c]`, the statement twin of the
        // `MirExpr::ObjSlice` arm, sharing its operand order and packing.
        MirStmt::ObjDelSlice {
            base,
            start,
            stop,
            step,
        } => {
            foreign_slice::emit_with_operands(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                base,
                [start.as_ref(), stop.as_ref(), step.as_ref()],
                |base, bounds| {
                    foreign_slice::emit_del_slice(context, builder, module, rt, base, bounds)
                },
            );
            Ok(())
        }
        // #1457 (Part 2 of #1443): `o.x = v` and `del o.x` on a CPython
        // object; `foreign_store` owns the evaluation order and packing.
        MirStmt::ObjAttrSet { base, attr, value } => {
            foreign_store::emit_set_attr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                base,
                attr,
                value,
            );
            Ok(())
        }
        MirStmt::ObjDelAttr { base, attr } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            foreign_store::emit_del_attr(context, builder, module, rt, base_scalar, attr);
            held.release(builder, rt);
            Ok(())
        }
        // `base.attr = value` (D-154, Part 1 of #375): writes the raw slot
        // word via the opaque `pycc_rt_instance_set_slot` accessor -- the
        // mirror image of `MirExpr::AttrGet`'s read, using
        // `scalar_to_slot_word` for the identical encoding
        // `slot_word_to_scalar` decodes.
        //
        // For a `Ty::Str` attribute, mirrors `MirStmt::Assign`'s own two
        // refcount obligations exactly (D-154 Part 1's own post-merge
        // review finding -- the first version of this arm had neither):
        // `incref_if_str_duplicate` before storing, since a source value
        // `str_value_is_a_duplicate_reference` classifies as borrowed is a
        // *duplicate* reference whose original owner keeps its own copy; and
        // `decref_str_attr_slot_before_store` before overwriting, to
        // release whatever the slot held previously (a no-op on a fresh
        // instance's unassigned slot, which the unchecked getter reads as
        // `0`, exactly like a local's
        // null-initialized string slot). Without both, a `str` attribute
        // read twice, or reassigned, use-after-frees the first `PyStrObj`.
        MirStmt::AttrSet { base, slot, value } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let base_ptr = expect_instance_pointer(base_scalar, "attribute assignment base");
            let value_ty = value.ty();
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let value_scalar = incref_if_str_duplicate(builder, rt, value, value_scalar);
            let value_scalar = retain_if_int_duplicate(context, builder, rt, value, value_scalar);
            // Part 1 of #1499: a borrowed object takes its own reference
            // before the slot word keeps it, because a module global it was
            // read from now releases its old value on rebind. The replaced
            // word is still never released (Part 4, #1504).
            let value_scalar = match value_scalar {
                Scalar::Object(pointer) => Scalar::Object(object_slot::retain_if_borrowed(
                    context, builder, module, value, pointer,
                )),
                other => other,
            };
            let slot_index = context.i64_type().const_int(*slot as u64, false);
            if value_ty == pycc_mir::Ty::Str {
                decref_str_attr_slot_before_store(context, builder, rt, base_ptr, slot_index);
            }
            // Unlike a local's storage slot, an instance attribute's
            // declared `Ty` is not reachable from `MirStmt::AttrSet` (it
            // carries only a slot *index*), so this release is gated on
            // the *value's* type rather than the slot's.
            //
            // #146 Part 1 recorded one shape that diverged: a `bool` value
            // assigned into an `int`-declared attribute reported
            // `Ty::Bool` here, skipping the release of a bigint the
            // attribute still held -- a leak, never a use-after-free
            // (D-180 Consequences item 6). #627 closed that divergence in
            // MIR instead of here: `pycc_mir`'s `HirStmt::AttrSet` arm now
            // wraps such a value in `MirExpr::IntBoundary`, whose `ty()`
            // is `Ty::Int`, so the release below fires and the store
            // encodes a D-141 word. `tests/issue_146_bigint_release.rs`
            // pins the observable half and `bigint_rc.rs`'s own
            // `an_int_attribute_slot_store_of_a_bool_emits_a_guarded_release`
            // pins this release. D-187 records the correction.
            //
            // The gate stays literally value-typed and only *coincides*
            // with D-180 Decision item 4's slot-typed invariant because
            // `bool` -> `int` is the sole widening `pycc_types` permits at
            // an attribute boundary (`bool` into a `float` attribute is
            // rejected with `error[T0021]`). A future widening would break
            // the coincidence silently; threading the declared type
            // through from `mir.class_defs` remains out of scope.
            if value_ty == pycc_mir::Ty::Int {
                release_int_attr_slot_before_store(context, builder, rt, base_ptr, slot_index);
            }
            let word = scalar_to_slot_word(context, builder, value_scalar);
            builder
                .build_call(
                    rt.instance_set_slot,
                    &[base_ptr.into(), slot_index.into(), word.into()],
                    "instance_set_slot",
                )
                .expect("build_call should not fail for a well-formed attribute write");
            Ok(())
        }
        // `for k in d:` (PR-11 Task 5, D-123): an intentional inline
        // duplicate of `MirStmt::ForList`'s own loop-building logic
        // directly above, for the identical "a third consumer, not a
        // second, justifies a shared helper" reason that arm's own comment
        // already gives for its own `ForRange` duplication.
        //
        // Three differences from `ForList`, all consequences of iterating
        // a dict's keys rather than a list's elements:
        // 1. The bound is `pycc_rt_dict_len`, still re-read on every
        //    iteration inside the test block rather than hoisted into the
        //    preheader, for the identical CPython-mutation-during-iteration
        //    reason `ForList`'s own comment gives: this compiler has no
        //    `del`/removal for a dict yet, but `d[k] = v` inside the loop
        //    body can still grow it, and a hoisted bound would silently
        //    stop tracking that.
        // 2. Each iteration reads the current *key*, via
        //    `pycc_rt_dict_key_at`, not an element via
        //    `pycc_rt_int_list_get`.
        // 3. The loop variable is bound `Scalar::Str`, not `Scalar::Int` --
        //    and, unlike every `ForList`/`ForRange` induction target (never
        //    refcounted, D-061), a dict key genuinely is a refcounted
        //    `PyStrObj` that `d` itself still holds a live, non-incref'd
        //    pointer to (D-124: "`PyDictObj`'s own keys ... are stored
        //    without incref on insert"). Without the `pycc_rt_str_incref`
        //    below, an ordinary body-level reassignment of the loop
        //    variable (`for k in d:\n    k = "z"\n`) would go through
        //    `emit_stmt`'s own `Assign` arm, whose
        //    `decref_str_slot_before_store` call fires unconditionally for
        //    any `Ty::Str` target -- decref'ing `d`'s *own* only reference
        //    to that key and freeing it while `d` still holds the
        //    now-dangling pointer, a real premature free this project's own
        //    leak-only containers are never supposed to cause (see
        //    `Scalar::List`'s and `Scalar::Dict`'s own doc comments: "leak-
        //    only -- never a premature free"). The incref gives the loop
        //    variable's own slot a reference distinct from `d`'s, so that
        //    decref (or the next iteration's own overwrite, which bypasses
        //    it entirely -- see `emit_assign` below) only ever brings the
        //    *duplicate* back down, never below `d`'s own copy. The
        //    resulting extra, never-brought-back-down increment on the
        //    loop's last key is exactly D-124's existing leak-only policy
        //    playing out one level lower: an unbalanced incref only makes
        //    an already-permanent leak larger, never a premature free.
        MirStmt::ForDict { var, dict, body } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            // Read once, in the preheader, not per iteration -- identical
            // reasoning to `ForList`'s own preheader read (see that arm's
            // comment): Python binds its iterator to the object the `for`
            // statement evaluated, so a body-level rebinding of `dict`'s
            // own name must not retarget the loop. Sound leak-only for the
            // same reason `ForList`'s is: nothing ever frees a dict value,
            // so a borrowed reference held across the body without its own
            // incref cannot go stale.
            let dict_ptr =
                emit_dict_name_read(context, builder, module, rt, user_functions, locals, dict);
            let preheader = builder.get_insert_block().unwrap();

            let test_bb = context.append_basic_block(function, "for_dict_test");
            let body_bb = context.append_basic_block(function, "for_dict_body");
            let after_bb = context.append_basic_block(function, "for_dict_after");

            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let induction = builder
                .build_phi(context.i64_type(), "for_dict_index")
                .expect("build_phi should not fail in a fresh loop-test block");
            let zero = context.i64_type().const_zero();
            induction.add_incoming(&[(&zero, preheader)]);
            let current = induction.as_basic_value().into_int_value();
            let len = build_dict_len(builder, rt, dict_ptr);
            let cont = builder
                .build_int_compare(IntPredicate::SLT, current, len, "for_dict_cont")
                .expect("build_int_compare should not fail comparing two i64 operands");
            builder
                .build_conditional_branch(cont, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(body_bb);
            let key_ptr = builder
                .build_call(
                    rt.dict_key_at,
                    &[dict_ptr.into(), current.into()],
                    "dict_key_at",
                )
                .expect("build_call should not fail for a well-formed dict key read")
                .try_as_basic_value()
                .expect_basic("pycc_rt_dict_key_at returns a non-void pointer")
                .into_pointer_value();
            // See this arm's own doc comment, point 3: gives the loop
            // variable's slot a reference distinct from `d`'s own, so a
            // body-level reassignment of `var` cannot free a key `d`
            // itself still points to.
            builder
                .build_call(rt.str_incref, &[key_ptr.into()], "for_dict_key_incref")
                .expect("build_call should not fail for a well-formed incref");
            // Same Python target-binding semantics `ForList`'s own comment
            // describes: a separate storage slot written once per
            // iteration (via `emit_assign` directly, not through
            // `emit_stmt`'s `Assign` arm -- so, like `ForList`'s own
            // per-iteration bind, this specific write never itself calls
            // `decref_str_slot_before_store`).
            emit_assign(context, builder, rt, locals, var, Scalar::Str(key_ptr));
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                expected_return_ty,
                finally_stack,
            )?;
            // `ForList`'s own terminator-safety guard, for the identical
            // reason (see that arm's comment): a `Return` inside `body`
            // already terminated `body_bb`, and adding the increment and
            // back-edge anyway would build a second terminator on it.
            if builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                let next = builder
                    .build_int_add(
                        current,
                        context.i64_type().const_int(1, false),
                        "for_dict_next",
                    )
                    .expect("build_int_add should not fail for two i64 operands");
                let body_end = builder.get_insert_block().unwrap();
                induction.add_incoming(&[(&next, body_end)]);
                builder.build_unconditional_branch(test_bb).expect(
                    "build_unconditional_branch should not fail on a block with no terminator yet",
                );
            }

            builder.position_at_end(after_bb);
            Ok(())
        }
        // `for x in s:` (PR-11 Task 9, D-123): an intentional inline
        // duplicate of `MirStmt::ForList`'s own loop-building logic above,
        // for the identical "a third consumer, not a second, justifies a
        // shared helper" reason that arm's own comment already gives for
        // its own `ForRange` duplication (and `MirStmt::ForDict`'s own
        // comment gives for its `ForList` duplication).
        //
        // Structurally this is closer to `ForList` than to `ForDict`: a
        // set's elements, like a list's, are encoded `i64` values with no
        // refcounting concern (unlike `ForDict`'s own key, which is a
        // refcounted `PyStrObj` needing its own `pycc_rt_str_incref` before
        // the loop variable's per-iteration bind -- see that arm's own doc
        // comment). Two differences from `ForList`, both consequences of
        // iterating a set rather than a list:
        // 1. The bound is `pycc_rt_int_set_len`, still re-read on every
        //    iteration inside the test block rather than hoisted into the
        //    preheader, for the identical CPython-mutation-during-iteration
        //    reason `ForList`'s own comment gives -- kept identical to
        //    `ForList`/`ForDict`'s own shape. Unlike `ForDict` (D-123
        //    accepts silently visiting newly-inserted keys as a bounded
        //    divergence), a mid-loop `set.add()` (D-119, this same PR) is
        //    checked against the length captured once in the preheader and
        //    raises `RuntimeError` (D-173) on any change -- see
        //    `pycc_rt_int_set_check_not_resized`'s own doc comment for why
        //    silently extending the iteration is not safe to accept here:
        //    `for x in s: s.add(x + 1)` would never terminate.
        // 2. Each iteration reads the current element via
        //    `pycc_rt_int_set_get`, not `pycc_rt_int_list_get`.
        //
        // D-141 mirrors `ForList`: the induction variable and `len` are raw
        // private counters, while the element is an encoded user value.
        MirStmt::ForSet {
            var,
            var_ty,
            set,
            body,
        } => {
            let function = builder.get_insert_block().unwrap().get_parent().unwrap();
            // Read once, in the preheader, not per iteration -- identical
            // reasoning to `ForList`'s own preheader read (see that arm's
            // comment): Python binds its iterator to the object the `for`
            // statement evaluated, so a body-level rebinding of `set`'s own
            // name must not retarget the loop. Sound leak-only for the same
            // reason `ForList`'s is: nothing ever frees a set value, so a
            // borrowed reference held across the body without its own
            // incref cannot go stale.
            let set_ptr =
                emit_set_name_read(context, builder, module, rt, user_functions, locals, set);
            // Captured once, in the preheader -- P1 review fix (PR-12,
            // `pycc_rt_int_set_check_not_resized`'s own doc comment):
            // `set.add()` existing in this same commit made it possible for
            // the loop body to grow `set_ptr` out from under this loop's own
            // `len` re-read below. Comparing every iteration's fresh read
            // against this snapshot, rather than silently visiting whatever
            // `len` grows to, matches CPython's own `RuntimeError` on
            // set-changed-size-during-iteration.
            let initial_len = build_int_set_len(builder, rt, set_ptr);
            let preheader = builder.get_insert_block().unwrap();

            let test_bb = context.append_basic_block(function, "for_set_test");
            let body_bb = context.append_basic_block(function, "for_set_body");
            let after_bb = context.append_basic_block(function, "for_set_after");

            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let induction = builder
                .build_phi(context.i64_type(), "for_set_index")
                .expect("build_phi should not fail in a fresh loop-test block");
            let zero = context.i64_type().const_zero();
            induction.add_incoming(&[(&zero, preheader)]);
            let current = induction.as_basic_value().into_int_value();
            let len = build_int_set_len(builder, rt, set_ptr);
            build_int_set_check_not_resized(builder, rt, len, initial_len);
            let in_range = builder
                .build_int_compare(IntPredicate::SLT, current, len, "for_set_in_range")
                .expect("build_int_compare should not fail comparing two i64 operands");
            // Part B of #1038 (#1064): `pycc_rt_int_set_check_not_resized`
            // now *raises* `RuntimeError: Set changed size during iteration`
            // (D-173) instead of aborting the process, and a D-173 raise
            // returns normally. Without this conjunct the loop-continue
            // condition would still be `current < len` -- and `len` is
            // exactly the value the body just grew -- so a `for x in s:
            // s.add(...)` loop would spin forever rather than reporting the
            // error. Reading the pending flag here, rather than returning a
            // status from the check function, keeps the runtime ABI
            // unchanged and is strictly more correct under D-173: the loop
            // also stops when anything in the *body* raised.
            //
            // What this cannot do is cut a loop short after a raise that was
            // already *handled*: `exception.rs` clears the pending state
            // before an `except` handler body runs and before a `finally`
            // body runs (restoring it afterwards), so a `for` loop in either
            // position iterates in full. What it can do -- deliberately -- is
            // exit at zero iterations when an *unhandled* raise is still
            // pending from an earlier statement that was not itself a
            // checkpoint, since `expression_can_set_exception` does not
            // classify every call as one. That is the same
            // raise-observed-at-the-next-checkpoint residual D-244's
            // 2026-09-13 amendments record, and exiting is the correct
            // response to it: a real exception is pending and is about to be
            // reported, so running the loop body would be the bug.
            //
            // Note the emission order: the resize check above runs *before*
            // this read, yet cannot relabel a body's own exception, because
            // `check_set_len_unchanged` returns early when
            // `pycc_rt_exception_active()` is already non-zero (review round
            // 2 of #1064). Suppressing inside the runtime rather than
            // branching around the call here keeps this block a single
            // straight-line test with no extra basic block, and the two
            // placements are observationally identical: the conjunct below
            // terminates the loop on this same iteration either way.
            let exc_active = builder
                .build_call(rt.exception_active, &[], "for_set_exc_active")
                .expect("build_call should not fail for exception_active")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_active returns i8")
                .into_int_value();
            let no_exception = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    exc_active,
                    exc_active.get_type().const_zero(),
                    "for_set_no_exc",
                )
                .expect("build_int_compare should not fail comparing an i8 against zero");
            let cont = builder
                .build_and(in_range, no_exception, "for_set_cont")
                .expect("build_and should not fail for two i1 operands");
            builder
                .build_conditional_branch(cont, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");

            builder.position_at_end(body_bb);
            // Same Python target-binding semantics `ForList`'s own comment
            // describes: a separate storage slot written once per
            // iteration.
            let encoded_element = build_int_set_get(builder, rt, set_ptr, current);
            emit_assign(
                context,
                builder,
                rt,
                locals,
                var,
                set_element_scalar(context, builder, var_ty, encoded_element),
            );
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                body,
                expected_return_ty,
                finally_stack,
            )?;
            // `ForList`'s own terminator-safety guard, for the identical
            // reason (see that arm's comment): a `Return` inside `body`
            // already terminated `body_bb`, and adding the increment and
            // back-edge anyway would build a second terminator on it.
            if builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                let next = builder
                    .build_int_add(
                        current,
                        context.i64_type().const_int(1, false),
                        "for_set_next",
                    )
                    .expect("build_int_add should not fail for two i64 operands");
                let body_end = builder.get_insert_block().unwrap();
                induction.add_incoming(&[(&next, body_end)]);
                builder.build_unconditional_branch(test_bb).expect(
                    "build_unconditional_branch should not fail on a block with no terminator yet",
                );
            }

            builder.position_at_end(after_bb);
            // Review round 3 of #1064. `after_bb` is reached by two edges
            // now: the ordinary "the loop ran out of elements" one, and the
            // exception conjunct's own early exit above. Falling through
            // without a checkpoint would let the statement *after* the loop
            // run with a real exception already pending -- measured: a
            // `for x in s: s.add(...)` loop followed by `print("BAD")`
            // inside a function printed `BAD` before the `RuntimeError`
            // reached its handler. That is not the
            // raise-observed-at-the-next-checkpoint residual D-244's
            // 2026-09-13 amendments accept, which is a property of
            // `expression_can_set_exception`'s classification of *operands*;
            // this is a statement-level edge this same commit introduced, so
            // it is closed here rather than recorded. `guard_statement_effects`
            // is the project's one mechanism for that edge (it also unwinds
            // the D-208 pending-release stack), so the loop's exceptional
            // exit routes through exactly the block every other fallible
            // statement does.
            guard_statement_effects(context, builder, rt);
            Ok(())
        }
        // `target = <comprehension>` (PR-12 Tasks 5a/5b, D-117): the loop
        // itself lives in `comprehension::emit_comprehension` (see its module
        // docs for the shape and for why `target` is bound only after the
        // whole loop has run, never before).
        MirStmt::ListCompAssign {
            target,
            var,
            var_ty,
            source,
            cond,
            elt,
        } => {
            let container = emit_comprehension(
                &CompCx {
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                },
                var,
                var_ty,
                source,
                cond.as_deref(),
                CompElts::List(elt),
            );
            emit_assign(context, builder, rt, locals, target, container);
            Ok(())
        }
        MirStmt::SetCompAssign {
            target,
            var,
            var_ty,
            source,
            cond,
            elt,
            ops,
        } => {
            let container = emit_comprehension(
                &CompCx {
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                },
                var,
                var_ty,
                source,
                cond.as_deref(),
                CompElts::Set(elt, ops.as_deref()),
            );
            emit_assign(context, builder, rt, locals, target, container);
            Ok(())
        }
        MirStmt::DictCompAssign {
            target,
            var,
            var_ty,
            source,
            cond,
            key,
            value,
        } => {
            let container = emit_comprehension(
                &CompCx {
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                },
                var,
                var_ty,
                source,
                cond.as_deref(),
                CompElts::Dict(key, value),
            );
            emit_assign(context, builder, rt, locals, target, container);
            Ok(())
        }
        MirStmt::Seq(stmts) => emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmts,
            expected_return_ty,
            finally_stack,
        ),
        // #382 (PR-22 Part 1): `raise ExceptionType("msg")` — allocate an
        // exception object, set the pending exception state, then terminate
        // this block. `emit_body` routes that explicit raise directly to the
        // nearest installed exception target (a `try` handler, the caller's
        // exceptional exit, or the top-level exit).
        MirStmt::Raise {
            exception,
            frame_function,
        } => {
            let exc_obj = emit_exception_value(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                exception,
                "exception",
            )?;
            emit_exception_set_frame(context, builder, module, rt, exc_obj, frame_function);
            builder
                .build_call(rt.exception_raise, &[exc_obj.into()], "")
                .expect("build_call should not fail for exception_raise");
            // Terminate the block with `unreachable` — the exception is
            // already set in the pending state. Do NOT position at a new
            // block after the unreachable: `emit_body` recognizes this
            // marker and replaces it with a branch to the current exception
            // target, while an unreachable block correctly does not fall
            // through in other structural checks.
            builder
                .build_unreachable()
                .expect("build_unreachable should not fail after raise");
            Ok(())
        }
        // Part 9 of #1371: `raise o` with a CPython object `o`.
        MirStmt::ObjRaise { value } => {
            foreign_raise::emit_obj_raise(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                value,
            );
            Ok(())
        }
        // #382: `raise ExceptionType("msg") from CauseType("cause")` —
        // allocate both exception and cause objects, then raise with cause.
        // Same `unreachable` approach as `Raise`.
        MirStmt::RaiseFrom {
            exception,
            cause,
            frame_function,
        } => {
            let exc_obj = emit_exception_value(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                exception,
                "exception",
            )?;
            let cause_obj = emit_exception_value(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                cause,
                "cause",
            )?;
            emit_exception_set_frame(context, builder, module, rt, exc_obj, frame_function);
            builder
                .build_call(
                    rt.exception_raise_with_cause,
                    &[exc_obj.into(), cause_obj.into()],
                    "",
                )
                .expect("build_call should not fail for exception_raise_with_cause");
            // Terminate the block with `unreachable` — the exception is
            // already set in the pending state. See `Raise`'s comment for
            // the rationale (same approach).
            builder
                .build_unreachable()
                .expect("build_unreachable should not fail after raise_from");
            Ok(())
        }
        // #382: Bare `raise` (re-raise) — load the lexically enclosing
        // handler's exception value, re-raise it, then terminate the block with
        // `unreachable`. The enclosing check point will detect the active
        // exception and handle it.
        MirStmt::Reraise => {
            let saved_exc = *rt
                .exceptions
                .reraise_values
                .borrow()
                .last()
                .expect("pycc_types rejects bare raise outside an except handler");
            let exc_val = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    saved_exc,
                    "saved_exc_val",
                )
                .expect("build_load should not fail for a handler exception slot")
                .into_pointer_value();
            builder
                .build_call(rt.exception_raise, &[exc_val.into()], "")
                .expect("build_call should not fail for exception_raise");
            // Terminate the block with `unreachable` — the exception is
            // already set in the pending state. See `Raise`'s comment for
            // the rationale (same approach).
            builder
                .build_unreachable()
                .expect("build_unreachable should not fail after reraise");
            Ok(())
        }
        // #382 (PR-22 Part 1): `try`/`except`/`else`/`finally` codegen.
        // The D-173 model uses explicit check-and-branch: after the try
        // body completes, generated code checks `pycc_rt_exception_active`
        // and, if an exception is pending, dispatches to the handler chain.
        // Each handler checks `pycc_rt_exception_type_matches` against its
        // declared type (or catches all for a bare `except:`). The handler
        // clears the exception state before running its body. The `else`
        // body runs only if no exception was raised. The `finally` body
        // always runs (implemented as a shared merge block that both the
        // normal-completion and exception paths branch to before exiting).
        MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => exception::emit_try(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            body,
            handlers,
            orelse,
            finalbody,
            expected_return_ty,
            finally_stack,
        ),
        // Part 3 of #382 (#542, PEP 654, D-202): `try`/`except*`/`else`/
        // `finally` codegen -- see `exception::emit_try_star`'s own doc
        // comment for how its handler dispatch differs from `Try`'s above.
        MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => exception::emit_try_star(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            body,
            handlers,
            orelse,
            finalbody,
            expected_return_ty,
            finally_stack,
        ),
    }
}
