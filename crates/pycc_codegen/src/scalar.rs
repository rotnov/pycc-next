//! Codegen value and storage representations: `Scalar`, `StorageSlot`,
//! `UserFunction`, finally targets, and the `Ty` -> LLVM type mapping.

use super::*;

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
pub(crate) enum Scalar<'ctx> {
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
    /// `int | float`: `pycc_rt::num`'s NaN-boxed word, LLVM `i64`. Only
    /// `pycc_rt_num_*` functions interpret it; a word holding a heap
    /// bigint owns one reference, like an `int` word (`crate::num`).
    Num(IntValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyStrObj` (D-059/D-060,
    /// Task 7) -- always refcounted, never inspected directly by this
    /// crate (see this enum's own doc comment).
    Str(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyListObj`, the one runtime
    /// object of every `list[T]`. `crate::list` passes it to the
    /// `pycc_rt_list_*` calls and inlines the hot reads and stores against
    /// its `#[repr(C)]` `{ len, data }` prefix.
    ///
    /// Its own variant rather than a reuse of `Str`'s (D-107): the two
    /// runtime objects have entirely different layouts, and `truthy`/
    /// `to_str` are exhaustive matches that would otherwise hand a
    /// `PyListObj` pointer straight to a `pycc_rt_str_*` function --
    /// reachable from ordinary type-checked source (`if xs:`, `print(xs)`)
    /// the moment list values become constructible. Keeping them distinct
    /// makes every operation `list[T]` has no v0.2 semantics for a compile
    /// error until it is answered deliberately, instead of silently
    /// misreading memory.
    ///
    /// A list is reference counted (`heap_rc`), and owns one reference to
    /// each `str`, list or instance element (`pycc_rt::list`).
    List(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyDictObj` (PR-11 Task 2/5,
    /// D-121/D-123) -- like `List`, opaque to this crate, which only ever
    /// stores it, passes it to a `pycc_rt_dict_*` call, or marshals it
    /// across a function boundary.
    ///
    /// Its own variant rather than a reuse of `List`'s or `Str`'s (D-107's
    /// reasoning, extended to this new container by D-124): `PyDictObj` and
    /// `PyListObj` have entirely different layouts, and every exhaustive
    /// `Scalar` match (`truthy`/`to_str`/`to_numeric_encoded_int`/`to_float`/
    /// `emit_assign`/argument marshalling) would otherwise hand a
    /// `PyDictObj` pointer straight to a `pycc_rt_list_*` or
    /// `pycc_rt_str_*` function -- reachable from ordinary type-checked
    /// source (`if x:`, `print(x)`) the moment dict values become
    /// constructible (this task). Keeping it distinct makes every operation
    /// `dict[K, V]` has no v0.2 semantics for a compile error until it is
    /// answered deliberately, instead of silently misreading memory.
    ///
    /// Reference counted like every other heap value (`heap_rc`): a freed
    /// dict releases the keys it owns.
    Dict(PointerValue<'ctx>),
    /// A pointer to a heap-allocated `pycc_rt::PyIntSetObj` (PR-11 Task 6,
    /// D-121/D-124) -- like `List`/`Dict`, opaque to this crate, which only
    /// ever stores it, passes it to a `pycc_rt_int_set_*` call, or marshals
    /// it across a function boundary.
    ///
    /// Its own variant rather than a reuse of `List`'s, `Dict`'s, or
    /// `Str`'s (D-107's reasoning, extended to this new container by
    /// D-124, exactly as `Dict`'s own doc comment already extends it):
    /// `PyIntSetObj` has its own layout, distinct from `PyListObj`,
    /// `PyDictObj`, and `PyStrObj`, and every exhaustive `Scalar` match
    /// (`truthy`/`to_str`/`to_numeric_encoded_int`/`to_float`/`emit_assign`/argument
    /// marshalling) would otherwise hand a `PyIntSetObj` pointer straight to
    /// a `pycc_rt_list_*`, `pycc_rt_dict_*`, or `pycc_rt_str_*`
    /// function -- reachable from ordinary type-checked source (`if s:`,
    /// `print(s)`) the moment set values become constructible (PR-11 Task
    /// 9). Keeping it distinct makes every operation `set[int]` has no v0.2
    /// semantics for a compile error until it is answered deliberately,
    /// instead of silently misreading memory.
    ///
    /// Reference counted like every other heap value (`heap_rc`): a freed
    /// set of counted instances releases its elements.
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
    /// `PyListObj`/`PyDictObj`/`PyIntSetObj` value contract: an int field
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
    /// of #375). Attribute reads and writes address its slots directly at a
    /// compile-time-resolved slot index, through `pycc_rt::instance`'s
    /// documented `#[repr(C)]` layout (see `attr_slot`); everything else
    /// about it goes through `pycc_rt_instance_*` calls.
    ///
    /// Its own variant rather than a reuse of `List`'s/`Dict`'s/`Set`'s
    /// (D-107/D-124's exact reasoning, extended identically): `PyInstanceObj`
    /// has its own layout, and every exhaustive `Scalar` match (`truthy`/
    /// `to_str`/`to_numeric_encoded_int`/`to_float`/`emit_assign`/argument
    /// marshalling) would otherwise hand an instance pointer straight to a
    /// `pycc_rt_list_*`/`pycc_rt_dict_*`/`pycc_rt_str_*` function.
    ///
    /// An instance of a regular class is reference counted (`heap_rc`); an
    /// exception object, which shares this variant, is not.
    Instance(PointerValue<'ctx>),
    /// A borrowed `PyObject *` owned by the hosting CPython interpreter
    /// (D-244, Part 2 of #1026) -- the runtime carrier for [`Ty::Object`],
    /// the opaque foreign-object type a `pycc build --ext` artifact obtains
    /// from `pycc_ext_obj_import` and, from this part on, from
    /// `pycc_ext_obj_getattr`. Entirely opaque to this crate: it is never
    /// `GEP`'d into, never inspected, and never handed to any `pycc_rt_*`
    /// function -- the only operations on it are the `pycc_ext_obj_*`
    /// shims in `src/ext/module/`, which are the sole place a
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
    /// Ownership is `heap_rc`'s (`HeapKind::Object`): every slot -- a local,
    /// a parameter, a module global, an instance attribute -- owns one
    /// reference, a producer such as `pycc_ext_obj_getattr` returns a new
    /// one, and an unbound temporary is released by its consumer
    /// (`object_release.rs`).
    /// A generator object (`pycc_rt::generator::PyGenObj *`), counted
    /// through its runtime incref/decref.
    Generator(PointerValue<'ctx>),
    /// A function value (`pycc_rt::closure::PyClosureObj *`), counted
    /// through its runtime incref/decref (`closure`).
    Closure(PointerValue<'ctx>),
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

#[derive(Clone)]
pub(crate) struct UserFunction<'ctx> {
    pub(crate) param_tys: Vec<pycc_mir::Ty>,
    /// Issue #22: the global function-pointer slot for this function name.
    /// Initialized to null; set to the current definition's address when
    /// the `def` executes at module level. Calls load from this slot and
    /// dispatch indirectly, so a call before the `def` has executed sees
    /// null and raises `NameError`, and a redefinition
    /// updates the slot so later calls see the new function.
    /// `None` for monomorphized generic specializations (`0gen_...` names),
    /// which are compiler-generated, not user-defined: they have no
    /// top-level `def` whose execution order matters, so they dispatch
    /// directly through `direct_value` instead.
    pub(crate) fn_ptr_global: Option<inkwell::values::GlobalValue<'ctx>>,
    /// The LLVM function type (parameter types + return type) for this
    /// function name. All definitions of the same name share one type
    /// (the type checker resolves to one signature per name). Needed to
    /// type the indirect call through `fn_ptr_global`.
    pub(crate) fn_type: inkwell::types::FunctionType<'ctx>,
    /// Issue #22: a global string constant holding the function name as a
    /// null-terminated C string, passed to `pycc_rt_raise_name_error` on the
    /// null-pointer (call-before-`def`) path. Created once per function
    /// name in the declaration pass and reused at every call site, rather
    /// than recreating a duplicate-named global on each call.
    /// `None` for monomorphized specializations (no call-before-`def`
    /// path exists for compiler-generated functions).
    pub(crate) name_global: Option<inkwell::values::GlobalValue<'ctx>>,
    /// Issue #22: for monomorphized generic specializations (`0gen_...`
    /// names), the LLVM function value to call directly. `None` for
    /// ordinary user-defined functions, which dispatch indirectly through
    /// `fn_ptr_global` to preserve Python execution order.
    pub(crate) direct_value: Option<FunctionValue<'ctx>>,
    /// The one definition `fn_ptr_global` can ever hold, when the module
    /// has exactly one `def` of this name and never binds the name as a
    /// variable: the slot is then either null (the `def` has not run yet)
    /// or this function. Calls keep the slot's null check, for the
    /// `NameError`, but call this function directly so LLVM can inline and
    /// specialize it; a call from inside the function itself skips even the
    /// check, since the function can only be running once its `def` has.
    pub(crate) single_def: Option<FunctionValue<'ctx>>,
    /// `single_def`'s definition is bound before any code of the module can
    /// run: every item before its `def` is another `def` (no top-level
    /// statement, foreign import or enum singleton runs earlier). Every call
    /// therefore finds the slot set, and skips the null check too.
    pub(crate) bound_before_code: bool,
    /// For the function a module of a package calls in place of another
    /// module's compiled function: that function, and how to tell whether
    /// the call may go to it (`crate::sibling_call`).
    pub(crate) sibling: Option<crate::sibling_call::SiblingCallee<'ctx>>,
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
pub(crate) const PENDING_RETURN_BUFFER_KEY: &str = "0pending_return_buffer";

/// Part 2b of #1142 (#1164): the companion flag to
/// [`PENDING_RETURN_BUFFER_KEY`], set when a rebinding of the pending
/// buffer's own name leaves [`PENDING_RETURN_BUFFER_KEY`] as the pointer's
/// *sole* owner.
///
/// Without it the epilogue could not tell "the pending pointer is still in
/// some local slot, which will release it" from "the pending pointer was
/// rebound away and nothing else names it". Freeing unconditionally would
/// double-free the first; never freeing leaks the second.
pub(crate) const PENDING_RETURN_ORPHANED_KEY: &str = "0pending_return_orphaned";

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
pub(crate) const BUFFER_SLICE_HAS_SLICE_KEY: &str = "0buffer_slice_has_slice";

/// The `start` out-pointer companion to [`BUFFER_SLICE_HAS_SLICE_KEY`].
pub(crate) const BUFFER_SLICE_START_KEY: &str = "0buffer_slice_start";

/// The `stop` out-pointer companion to [`BUFFER_SLICE_HAS_SLICE_KEY`].
pub(crate) const BUFFER_SLICE_STOP_KEY: &str = "0buffer_slice_stop";

/// #382 (PR-22 Part 2): A pending `finally` target that `return`
/// statements inside a `try` body must route through before completing.
/// When a `MirStmt::Return` is emitted and the `finally_stack` is
/// non-empty, the return value is stored to `ret_slot` (if non-`None`),
/// the `is_returning` flag is set to `1`, and control branches to
/// `finally_bb` instead of emitting `ret` directly. After the finally
/// body runs, the codegen checks `is_returning`: if set, it loads the
/// return value and emits `ret` (or propagates to an enclosing finally).
#[derive(Clone)]
pub(crate) struct FinallyTarget<'ctx> {
    /// The finally block to branch to instead of emitting `ret`.
    pub(crate) finally_bb: inkwell::basic_block::BasicBlock<'ctx>,
    /// Alloca holding the return value (`None` for `Ty::None` / void
    /// functions, where `build_return(None)` is emitted instead).
    pub(crate) ret_slot: Option<PointerValue<'ctx>>,
    /// Alloca i8 flag: `1` = a `return` was intercepted, `0` = normal
    /// completion (fall-through to finally).
    pub(crate) is_returning: PointerValue<'ctx>,
    /// The number of live exception slots (`exception::owned_slots`) when
    /// the `try` was entered: a `return` routed here releases the slots
    /// above it, and this `finally` may still read the ones below.
    pub(crate) exc_depth: usize,
    /// The number of enclosing `for x in <object>:` loops
    /// (`ExceptionCodegenState::loop_iterators`) when the `try` was
    /// entered: a `return` routed here releases the iterators of the loops
    /// above it, and those below stay live while this `finally` runs.
    pub(crate) loop_depth: usize,
}

#[derive(Clone)]
pub(crate) struct StorageSlot<'ctx> {
    pub(crate) ptr: PointerValue<'ctx>,
    pub(crate) ty: pycc_mir::Ty,
    /// Module globals and non-parameter locals can have storage before
    /// execution reaches an assignment. Their separate flag prevents an
    /// unexecuted control-flow path from exposing an LLVM initializer, `undef`,
    /// or null as a fabricated Python value. Parameters are initialized at
    /// entry and therefore need no flag; #118 separately owns static
    /// definite-assignment diagnostics.
    pub(crate) initialized: Option<PointerValue<'ctx>>,
}

pub(crate) fn ty_to_basic_type(
    context: &Context,
    ty: pycc_mir::Ty,
) -> inkwell::types::BasicTypeEnum<'_> {
    match ty {
        pycc_mir::Ty::Int | pycc_mir::Ty::Num => context.i64_type().into(),
        pycc_mir::Ty::Bool => context.i8_type().into(),
        pycc_mir::Ty::Float => context.f64_type().into(),
        pycc_mir::Ty::Str => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // LLVM `void` cannot be a parameter type. v0.1 therefore carries
        // Python's singleton unit value across user-function parameter and
        // assignment-storage boundaries as the canonical `i8 0`; a `None`
        // return is still emitted as LLVM `void` by `compile_to_object`.
        pycc_mir::Ty::None => context.i8_type().into(),
        // `list[T]`'s runtime object is heap-allocated and always
        // referenced by pointer -- exactly the same storage/parameter
        // representation `Str` already gets above, whatever `T` is.
        pycc_mir::Ty::List(_) | pycc_mir::Ty::VarTuple(_) => {
            context.ptr_type(inkwell::AddressSpace::default()).into()
        }
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
        // A boxed tuple (any element other than int/bool/float) is a
        // pointer to its runtime object (`containers.rs`).
        pycc_mir::Ty::Tuple(elems) if pycc_hir::tuple_is_boxed(&elems) => {
            context.ptr_type(inkwell::AddressSpace::default()).into()
        }
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
        // A pointer-represented `T | None` is the bare pointer, null for
        // `None` ([`pointer_optional_inner`]).
        pycc_mir::Ty::Optional(inner) if pycc_hir::is_pointer_optional_inner(&inner) => {
            ty_to_basic_type(context, *inner)
        }
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
        // A generator object (`pycc_rt::generator::PyGenObj *`).
        pycc_mir::Ty::Generator(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        // A function value (`pycc_rt::closure::PyClosureObj *`).
        pycc_mir::Ty::Callable(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
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
pub(crate) fn default_value_for_type<'ctx>(
    context: &'ctx Context,
    ty: pycc_mir::Ty,
) -> inkwell::values::BasicValueEnum<'ctx> {
    match ty {
        pycc_mir::Ty::Int => context.i64_type().const_zero().into(),
        // The `int` `0`, a valid `int | float` word holding no reference.
        pycc_mir::Ty::Num => context.i64_type().const_int(1, false).into(),
        pycc_mir::Ty::Bool => context.i8_type().const_zero().into(),
        pycc_mir::Ty::Float => context.f64_type().const_zero().into(),
        pycc_mir::Ty::Str
        | pycc_mir::Ty::List(_)
        | pycc_mir::Ty::VarTuple(_)
        | pycc_mir::Ty::Dict(_)
        | pycc_mir::Ty::Set(_)
        | pycc_mir::Ty::FrozenSet(_)
        | pycc_mir::Ty::Instance(_)
        | pycc_mir::Ty::Protocol(_)
        | pycc_mir::Ty::Object
        | pycc_mir::Ty::MemoryView
        | pycc_mir::Ty::NdArray(_)
        | pycc_mir::Ty::Generator(_)
        | pycc_mir::Ty::Callable(_) => context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into(),
        pycc_mir::Ty::ShapedList(_) => {
            unreachable!("pycc_types erases a shaped list to its tuple")
        }
        pycc_mir::Ty::Tuple(elems) if pycc_hir::tuple_is_boxed(&elems) => context
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
        pycc_mir::Ty::Optional(ref inner) if pycc_hir::is_pointer_optional_inner(inner) => {
            context.ptr_type(inkwell::AddressSpace::default()).const_null().into()
        }
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

/// The `T` of a `T | None` held as the bare `T` pointer, null for `None`
/// (`pycc_hir::is_pointer_optional_inner`): a value of such a type is the
/// `Scalar` of `T` itself, and every reference-counting path treats it as a
/// `T` whose pointer may be null (each `incref`/`decref` is null-safe).
pub(crate) fn pointer_optional_inner(ty: &pycc_mir::Ty) -> Option<&pycc_mir::Ty> {
    match ty {
        pycc_mir::Ty::Optional(inner) if pycc_hir::is_pointer_optional_inner(inner) => Some(inner),
        _ => None,
    }
}

/// The pointer a pointer-valued `scalar` holds (a `str`, a list or tuple
/// object, a dict, a set, an instance, a CPython object, a closure or a
/// generator), or `None` for a value scalar.
pub(crate) fn scalar_pointer<'ctx>(scalar: &Scalar<'ctx>) -> Option<inkwell::values::PointerValue<'ctx>> {
    match scalar {
        Scalar::Str(p)
        | Scalar::List(p)
        | Scalar::Dict(p)
        | Scalar::Set(p)
        | Scalar::Instance(p)
        | Scalar::Object(p)
        | Scalar::Closure(p)
        | Scalar::Generator(p) => Some(*p),
        _ => None,
    }
}
