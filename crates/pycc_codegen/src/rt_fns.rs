//! Declarations of every `pycc_rt` runtime function this crate calls.
//!
//! Cohesion-driven carve out of `lib.rs` under AGENTS.md's decomposability
//! rule (issue #545 Part 2): the `RtFns` handle struct here, and the single
//! routine that populates it in [`declare`], relocated unchanged.

use super::*;

mod declare;
pub(super) use declare::declare_rt_functions;

/// Every `pycc_rt` function this crate calls, declared once in
/// `compile_to_object` and threaded through `emit_stmt`/`emit_expr`.
/// Extended (never replaced) by Tasks 8/9/10 as they add more `pycc_rt`
/// declarations.
pub(super) struct RtFns<'ctx> {
    /// The generic tuple, dict and set objects' functions.
    pub(super) containers: crate::containers::ContainerFns<'ctx>,
    /// The generator objects' functions and coroutine intrinsics.
    pub(super) gens: crate::generator::GenFns<'ctx>,
    pub(super) closures: crate::closure::ClosureFns<'ctx>,
    /// The `int | float` functions (`crate::num`).
    pub(super) num: crate::num::NumFns<'ctx>,
    /// The coroutine state of the generator function being emitted, if any.
    pub(super) gen_frame: std::cell::RefCell<Option<crate::generator::GenFrame<'ctx>>>,
    /// The LLVM context, for emitters that receive only a builder.
    pub(super) context: &'ctx Context,
    pub(super) int_from_i64: FunctionValue<'ctx>,
    /// #1331: `pycc_rt_hash_int`/`pycc_rt_hash_tuple`, CPython's `hash()`
    /// of an encoded int and of a tuple's already-hashed lanes.
    pub(super) hash_int: FunctionValue<'ctx>,
    pub(super) hash_tuple: FunctionValue<'ctx>,
    /// `pycc_rt_hash_str`/`pycc_rt_hash_float`, a tuple lane of those types.
    pub(super) hash_str: FunctionValue<'ctx>,
    pub(super) hash_float: FunctionValue<'ctx>,
    /// `pycc_rt_hash_tuple_object`, a tuple held as a tuple object.
    pub(super) hash_tuple_object: FunctionValue<'ctx>,
    /// #1335: `pycc_rt_hash_pointer`/`pycc_rt_hash_slot_int`, an
    /// instance's identity hash and a user `__hash__`'s `int` result
    /// reduced as CPython's `slot_tp_hash` does.
    pub(super) hash_pointer: FunctionValue<'ctx>,
    pub(super) hash_slot_int: FunctionValue<'ctx>,
    pub(super) int_add: FunctionValue<'ctx>,
    pub(super) int_sub: FunctionValue<'ctx>,
    pub(super) int_mul: FunctionValue<'ctx>,
    pub(super) int_floordiv: FunctionValue<'ctx>,
    pub(super) int_floormod: FunctionValue<'ctx>,
    pub(super) int_pow: FunctionValue<'ctx>,
    /// #1210: `pycc_rt_int_lshift`/`_rshift`/`_and`/`_or`/`_xor`, the
    /// encoded-word shift and bitwise operators. The shifts may raise
    /// (D-173); `&`, `|` and `^` never do.
    pub(super) int_lshift: FunctionValue<'ctx>,
    pub(super) int_rshift: FunctionValue<'ctx>,
    pub(super) int_and: FunctionValue<'ctx>,
    pub(super) int_or: FunctionValue<'ctx>,
    pub(super) int_xor: FunctionValue<'ctx>,
    pub(super) int_cmp: FunctionValue<'ctx>,
    /// `pycc_rt_int_truediv(a, b) -> f64`: `int / int`, correctly rounded at
    /// any magnitude.
    pub(super) int_truediv: FunctionValue<'ctx>,
    /// `pycc_rt_int_float_cmp(a, f) -> i32`: exact `int`-vs-`float`
    /// ordering, `2` when `f` is NaN.
    pub(super) int_float_cmp: FunctionValue<'ctx>,
    /// `pycc_rt_int_from_float(f) -> i64`: `int(f)`.
    pub(super) int_from_float: FunctionValue<'ctx>,
    /// `pycc_rt_int_mod_noraise(a, b) -> i64`: `divmod`'s remainder, which
    /// leaves the zero-divisor raise to the quotient.
    pub(super) int_mod_noraise: FunctionValue<'ctx>,
    /// `pycc_rt_int_abs(a) -> i64`: `abs(a)`.
    pub(super) int_abs: FunctionValue<'ctx>,
    /// `llvm.{sadd,ssub,smul}.with.overflow.i64`, the `int` fast paths'
    /// overflow detectors (`int_fast`).
    pub(super) sadd_overflow: FunctionValue<'ctx>,
    pub(super) ssub_overflow: FunctionValue<'ctx>,
    pub(super) smul_overflow: FunctionValue<'ctx>,
    pub(super) int_truthy: FunctionValue<'ctx>,
    pub(super) range_continue: FunctionValue<'ctx>,
    /// `pycc_rt_range_sized_stop(start, stop, step) -> i64`: the bound a
    /// `list(range(...))` materialization iterates to, raising CPython's
    /// `OverflowError` for a length past a C `ssize_t`.
    pub(super) range_sized_stop: FunctionValue<'ctx>,
    /// #147 (D-179): `pycc_rt_range_normalize_operand`, the encoded-word-in,
    /// encoded-word-out normalizer every `range()` operand passes through.
    /// It maps D-141's `False`/`True` markers to the ordinary smallints
    /// `0`/`1` (a range produces integer objects rather than forwarding its
    /// argument object) and passes a smallint or a heap bigint through
    /// unchanged. It replaces the `int_untag_checked` call this position
    /// used to emit, which rejected every bigint.
    pub(super) range_normalize_operand: FunctionValue<'ctx>,
    pub(super) int_to_float: FunctionValue<'ctx>,
    pub(super) float_div: FunctionValue<'ctx>,
    pub(super) float_floordiv: FunctionValue<'ctx>,
    pub(super) float_floormod: FunctionValue<'ctx>,
    pub(super) float_pow: FunctionValue<'ctx>,
    /// libm's own `pow`, the function `pycc_rt_float_pow` (and CPython's
    /// `float_pow`) computes with; `float_fast` calls it directly on the
    /// domain where no Python exception is possible.
    pub(super) libm_pow: FunctionValue<'ctx>,
    /// `llvm.sqrt.f64` (correctly rounded), for `float_fast`'s proof that
    /// `pow(x, 0.5) == sqrt(x)`.
    pub(super) sqrt_f64: FunctionValue<'ctx>,
    pub(super) str_from_literal: FunctionValue<'ctx>,
    pub(super) str_concat: FunctionValue<'ctx>,
    /// #575 (Part 2 of #123): `pycc_rt_str_repeat`, the `str * int` /
    /// `int * str` primitive. Declared beside `str_concat` because the
    /// two share the same shape apart from the count: one `str` pointer
    /// in, one brand-new `str` pointer out. The count parameter is a
    /// **raw** `i64`, already decoded by `build_untag_checked`, matching
    /// every other raw runtime counter this table declares (list index,
    /// slice bound) rather than pushing D-141's classifier into a second
    /// place. `range` operands left that group in #147 (D-179): they now
    /// stay encoded and go through `range_normalize_operand`.
    pub(super) str_repeat: FunctionValue<'ctx>,
    pub(super) str_cmp: FunctionValue<'ctx>,
    pub(super) str_truthy: FunctionValue<'ctx>,
    pub(super) str_incref: FunctionValue<'ctx>,
    pub(super) str_decref: FunctionValue<'ctx>,
    /// The shim's `Py_XINCREF`/`Py_XDECREF` (`pycc_ext_obj_retain`/
    /// `pycc_ext_obj_release`): how `heap_rc` counts an `object` value.
    /// Only referenced by code that handles a `Ty::Object`, which exists
    /// only in a build that links the CPython shim.
    pub(super) obj_retain: FunctionValue<'ctx>,
    pub(super) obj_release: FunctionValue<'ctx>,
    /// #146 Part 1: `str`'s D-060 refcounting pair, generalized to D-141's
    /// encoded `int` word. Both take the raw `i64` word rather than a
    /// pointer, and both are only ever called under the inline
    /// `(word & 3) == 0 && word != 0` guard `emit_bigint_refcount_call`
    /// wraps them in, so a smallint loop never reaches a runtime call.
    pub(super) bigint_retain: FunctionValue<'ctx>,
    pub(super) bigint_release: FunctionValue<'ctx>,
    pub(super) int_to_str: FunctionValue<'ctx>,
    pub(super) float_to_str: FunctionValue<'ctx>,
    pub(super) bool_to_str: FunctionValue<'ctx>,
    /// `str()` of an `Optional[int|float|bool]`: the payload and the
    /// present flag (`pycc_rt::format`).
    pub(super) optional_to_str: [FunctionValue<'ctx>; 3],
    pub(super) print_write_str: FunctionValue<'ctx>,
    pub(super) print_space: FunctionValue<'ctx>,
    pub(super) print_newline: FunctionValue<'ctx>,
    pub(super) print_none: FunctionValue<'ctx>,
    /// `pycc_rt_print_flush` (#1340): empties compiled code's stdout before
    /// `print` asks CPython for an object argument's `str()`.
    pub(super) print_flush: FunctionValue<'ctx>,
    /// D-141's checked numeric decoder. Container ingress calls it for
    /// validation and keeps the original encoded word; index, slice, and
    /// `str`-repeat-count sites use its raw `0`/`1`/smallint result as an
    /// implementation counter. `range` operands no longer call it at all
    /// since #147 (D-179) -- see `range_normalize_operand`. It rejects
    /// bigint and malformed words in `pycc_rt`, which owns the
    /// representation classifier.
    pub(super) int_untag_checked: FunctionValue<'ctx>,
    /// `list[T]`'s generic runtime object (`pycc_rt::list`). The hot reads
    /// and stores are inlined by `crate::list` against the object's
    /// `#[repr(C)]` layout; these are the out-of-line operations.
    /// `round(float, ndigits)` and `round(float)` (`pycc_rt::round`).
    pub(super) float_round: FunctionValue<'ctx>,
    pub(super) float_round_to_int: FunctionValue<'ctx>,
    pub(super) list_new: FunctionValue<'ctx>,
    pub(super) list_append: FunctionValue<'ctx>,
    pub(super) list_reserve: FunctionValue<'ctx>,
    pub(super) list_reset: FunctionValue<'ctx>,
    /// The list the next list comprehension fills instead of a new one
    /// (`extend_scratch`), taken when its container is made.
    pub(super) comprehension_into: std::cell::Cell<Option<inkwell::values::PointerValue<'ctx>>>,
    pub(super) list_index_error: FunctionValue<'ctx>,
    pub(super) tuple_index_error: FunctionValue<'ctx>,
    pub(super) list_set: FunctionValue<'ctx>,
    pub(super) list_pop: FunctionValue<'ctx>,
    pub(super) list_insert: FunctionValue<'ctx>,
    pub(super) list_slice: FunctionValue<'ctx>,
    pub(super) list_concat: FunctionValue<'ctx>,
    pub(super) list_repeat: FunctionValue<'ctx>,
    pub(super) list_contains: FunctionValue<'ctx>,
    pub(super) list_repr: FunctionValue<'ctx>,
    pub(super) dict_repr: FunctionValue<'ctx>,
    /// Part 2 of #1027's bounds-checked buffer element load
    /// (`pycc_rt_buffer_f64_get`): takes the `PyccExtBufferView` pointer a
    /// `pycc build --ext` wrapper passed for a `memoryview` parameter plus an
    /// already-untagged raw `i64` index, and returns the `f64` element.
    ///
    /// The bounds check lives in `pycc_rt`, not in emitted IR -- one call and
    /// no arithmetic here, exactly as `list_get` does it. An index outside
    /// `[0, len)` leaves the D-173 pending `IndexError` set and returns a
    /// `0.0` sentinel, which the caller's `exception_exit` block checks for.
    pub(super) buffer_f64_get: FunctionValue<'ctx>,
    /// Part 1 of #1142's bounds-checked buffer element store
    /// (`pycc_rt_buffer_f64_set`): the same pointer and already-untagged
    /// raw `i64` index `buffer_f64_get` takes, plus the `f64` to write, and
    /// no return value.
    ///
    /// Being `void` is what makes its D-173 check a *statement*-level one:
    /// there is no result for `expression_can_set_exception` to guard, so
    /// the emitting arm calls `guard_statement_effects` itself.
    pub(super) buffer_f64_set: FunctionValue<'ctx>,
    /// #1116's buffer element count (`pycc_rt_buffer_len`): takes the same
    /// `PyccExtBufferView` pointer and returns the `len` word as a **raw,
    /// untagged** `i64`, exactly as `list_len` below does for a list.
    ///
    /// Unlike `buffer_f64_get` above it cannot fail, so the emitted call
    /// carries no D-173 exception check.
    pub(super) buffer_len: FunctionValue<'ctx>,
    /// #1165's artifact-owned buffer allocation
    /// (`pycc_rt_buffer_f64_alloc`): takes an already-untagged raw `i64`
    /// element count and returns a **fresh** `PyccExtBufferView` pointer --
    /// exactly the same opaque pointer type the three helpers above already
    /// take, so a produced buffer and a `memoryview` parameter are one shape
    /// to every consumer.
    ///
    /// Unlike `buffer_len` it can fail: a negative count leaves the D-173
    /// pending `ValueError` set and returns null, which is why
    /// `expression_can_set_exception` answers `true` for `MirExpr::BufferAlloc`.
    pub(super) buffer_f64_alloc: FunctionValue<'ctx>,
    /// #1165's non-aborting decoder for the `ndarray(n)` length word
    /// (`pycc_rt_buffer_alloc_untag_len`). `int_untag_checked` above
    /// `panic!`s on a bigint, and that panic becomes a process abort at its
    /// own `extern "C"` boundary -- the host interpreter's, for a D-244
    /// `ext` artifact. This decoder raises `OverflowError` (D-173) and
    /// returns the sentinel `0` instead, which the
    /// `guard_statement_effects` emitted between it and `buffer_f64_alloc`
    /// consumes before the allocator can see it.
    pub(super) buffer_alloc_untag_len: FunctionValue<'ctx>,
    /// #1165's release for storage `buffer_f64_alloc` produced
    /// (`pycc_rt_buffer_f64_free`): takes the view pointer and returns
    /// nothing.
    ///
    /// A documented no-op on null, which is what makes the generated
    /// epilogue safe over a buffer slot a path never assigned -- the slot is
    /// null-initialized at entry, exactly as an owned `str` slot is.
    pub(super) buffer_f64_free: FunctionValue<'ctx>,
    /// PR-11 Task 5's own new `pycc_rt_dict_*` declarations, mirroring the
    /// `list_*` cluster immediately above one-for-one: `dict_new` (no
    /// pre-sizing entry point, same reasoning as `list_new`),
    /// `dict_set` (insert-or-update, D-123 -- this crate's one dict
    /// "growable op", playing `list_append`'s role), `dict_get`
    /// (read, panics on a missing key), `dict_len`, and `dict_key_at`
    /// (`ForDict`'s own per-iteration key read, playing `list_get`'s
    /// `ForList`-iteration role).
    pub(super) dict_new: FunctionValue<'ctx>,
    pub(super) dict_set: FunctionValue<'ctx>,
    pub(super) dict_get: FunctionValue<'ctx>,
    /// PR-12 Task 11's own new `pycc_rt_dict_get_or_default` declaration
    /// (`dict.get(key, default)`, D-119) -- mirrors `dict_get`'s own shape
    /// plus one extra encoded-value `i64` `default` parameter, matching
    /// `pycc_rt_dict_get_or_default`'s real Rust signature
    /// (`fn(*mut PyDictObj, *mut PyStrObj, i64) -> i64`) exactly.
    pub(super) dict_get_or_default: FunctionValue<'ctx>,
    pub(super) dict_len: FunctionValue<'ctx>,
    pub(super) dict_key_at: FunctionValue<'ctx>,
    /// `pycc_rt_dict_check_not_resized(len, initial)`: `for k in d`'s
    /// per-step size check.
    pub(super) dict_check_not_resized: FunctionValue<'ctx>,
    /// The instance-set constructor (`int_set_new`, uncounted elements),
    /// and the length and element read every set shares (`int_set_len`,
    /// `int_set_get`: `pycc_rt_set_len`/`pycc_rt_set_get` serve both
    /// `set.rs` and `int_set.rs` objects).
    pub(super) int_set_new: FunctionValue<'ctx>,
    pub(super) int_set_len: FunctionValue<'ctx>,
    pub(super) int_set_get: FunctionValue<'ctx>,
    /// `frozenset(s)` of an instance set: a copy keeping its hashes.
    pub(super) int_set_copy: FunctionValue<'ctx>,
    /// `ForSet`'s own loop-test check (Task 11 review fix, P1): panics if
    /// the set's freshly re-read length no longer matches the length
    /// captured once in the loop's preheader -- see
    /// `pycc_rt_int_set_check_not_resized`'s own doc comment for why
    /// `set.add()` made this reachable.
    pub(super) int_set_check_not_resized: FunctionValue<'ctx>,
    /// #1343: a set of user-class instances (`set_instance.rs`). `candidate`
    /// finds the next stored entry with an equal hash, `push` appends a new
    /// entry with its hash, and `add_identity` is the whole insert for a
    /// class whose `__eq__` is identity.
    pub(super) obj_set_candidate: FunctionValue<'ctx>,
    pub(super) obj_set_push: FunctionValue<'ctx>,
    pub(super) obj_set_add_identity: FunctionValue<'ctx>,
    /// D-154 (Part 1 of #375): `pycc_rt::instance`'s own function
    /// cluster -- `instance_new` (allocates a fresh instance with the
    /// class's own declared slot count, every slot unassigned, and -- since
    /// #1388 -- its layout descriptor); `instance_get_slot_checked`, the cold
    /// path of an inline `base.attr` read that raises `AttributeError` for an
    /// unassigned slot (#1388, see `attr_slot`); and `instance_set_slot`,
    /// which only enum member initialization still calls (attribute stores
    /// are inline).
    pub(super) instance_new: FunctionValue<'ctx>,
    pub(super) instance_get_slot_checked: FunctionValue<'ctx>,
    pub(super) instance_set_slot: FunctionValue<'ctx>,
    /// `pycc_rt_instance_class_is(instance, name, len) -> bool`: the
    /// run-time class test of a dispatched method call (v0.9,
    /// `pycc_mir::dispatch`).
    pub(super) instance_class_is: FunctionValue<'ctx>,
    /// `pycc_rt_raise_name_error(name, len)`: makes CPython's
    /// `NameError: name '<name>' is not defined` the pending exception and
    /// returns. Issue #22's call-before-`def` path raises it and unwinds
    /// (`ext_thunk::emit_fnptr_dispatch_guard`); an unbound global read
    /// (`expr::name_read`) finds this same declaration.
    pub(super) raise_name_error: FunctionValue<'ctx>,
    /// The runtime's `pycc_rt_exc_pending` `i8` global, the pending-exception
    /// flag a native program loads inline (`int_fast::emit_exception_pending`).
    pub(super) exc_pending: inkwell::values::GlobalValue<'ctx>,
    /// The runtime's `pycc_rt_exc_pending_threads` (`usize`): how many
    /// threads have an exception pending. An `ext` module loads it inline
    /// and calls `exception_active` only when it is non-zero
    /// (`int_fast::emit_exception_pending`).
    pub(super) exc_pending_threads: inkwell::values::GlobalValue<'ctx>,
    /// `pycc_rt_exception_active() -> i8`, the thread-local pending state.
    /// An `ext` module checks through it instead of `exc_pending`, because
    /// CPython may call into the module from several threads.
    pub(super) exception_active: FunctionValue<'ctx>,
    /// Set for an `ext` build: check the pending state with
    /// `exception_active` rather than the process-wide `exc_pending` load.
    pub(super) thread_safe_exception_check: std::cell::Cell<bool>,
    /// Set when the target's libm is glibc, whose documented `pow` error
    /// bound lets `float_fast` prove `pow(x, 0.5) == sqrt(x)` for most `x`.
    pub(super) glibc_libm: std::cell::Cell<bool>,
    /// The attribute slots proven always assigned (`assigned_slots`).
    pub(super) assigned_slots: std::cell::RefCell<crate::assigned_slots::AssignedSlots>,
    /// Whether the function being emitted is a constructor, whose stores
    /// set the assigned flags the proof relies on.
    pub(super) emitting_ctor: std::cell::Cell<bool>,
    /// The functions that can never raise (`nothrow`): a call to one needs
    /// no pending-exception guard.
    pub(super) nothrow: std::cell::RefCell<std::collections::HashSet<String>>,
    /// `exception_value` returns a pointer to the current exception object.
    pub(super) exception_value: FunctionValue<'ctx>,
    /// `exception_take` clears the pending state and returns the exception
    /// it held, with the state's reference (null when nothing was pending).
    pub(super) exception_take: FunctionValue<'ctx>,
    /// `pycc_rt_exception_incref`/`_decref`: an exception object's count.
    pub(super) exc_incref: FunctionValue<'ctx>,
    pub(super) exc_decref: FunctionValue<'ctx>,
    /// `exception_alloc(type_tag: u8, name: *const u8, name_len: usize,
    /// message: *mut PyStrObj) -> *mut PyExceptionObj`. Part 2 of #541
    /// (D-189) added the class name: user-defined exception classes carry
    /// module-assigned tags the runtime cannot map back to a name.
    pub(super) exception_alloc: FunctionValue<'ctx>,
    /// `exception_raise(obj: *mut PyExceptionObj)` — sets pending state (void).
    pub(super) exception_raise: FunctionValue<'ctx>,
    /// `exception_restore(obj: *mut PyExceptionObj)` (void): makes a taken
    /// exception pending again; nothing for null (`object_release`).
    pub(super) exception_restore: FunctionValue<'ctx>,
    /// `exception_set_frame(obj: *mut PyExceptionObj, frame_function: *const
    /// u8, frame_function_len: usize)` (void, #707): records the enclosing
    /// function's source name on the exception object being raised, for
    /// `exception_print_and_exit`'s traceback rendering. Called once, at the
    /// `raise`/`raise ... from ...` statement itself, on the raised
    /// exception only -- never on a `from` clause's cause, which keeps
    /// whatever frame it already carried (see
    /// `pycc_mir::MirStmt::RaiseFrom::frame_function`'s doc comment).
    pub(super) exception_set_frame: FunctionValue<'ctx>,
    /// `exception_raise_with_cause(obj, cause: *mut PyExceptionObj)` (void).
    pub(super) exception_raise_with_cause: FunctionValue<'ctx>,
    /// `exception_type_matches(obj: *mut PyExceptionObj, type_tag: u8) -> i8`.
    pub(super) exception_type_matches: FunctionValue<'ctx>,
    /// `exception_message(obj: *mut PyExceptionObj) -> *mut PyStrObj` (Part 3A
    /// of #541, #736): the exception's own message alone (`str(e)`
    /// semantics), never `exception_print_and_exit`'s `"{type}: {message}"`
    /// uncaught-exception format.
    pub(super) exception_message: FunctionValue<'ctx>,
    /// `exception_print_and_exit(obj: *mut PyExceptionObj) -> !` (noreturn).
    pub(super) exception_print_and_exit: FunctionValue<'ctx>,
    /// Part 3 of #382 (#542, PEP 654): `exception_group_alloc(type_tag: u8,
    /// name: *const u8, name_len: usize, message: *mut PyStrObj, exceptions:
    /// *const *mut PyExceptionObj, exceptions_len: usize) -> *mut
    /// PyExceptionObj`. Builds an `ExceptionGroup`/`BaseExceptionGroup`
    /// instance from a literal member array, mirroring `exception_alloc`
    /// with two extra parameters for the member pointers.
    pub(super) exception_group_alloc: FunctionValue<'ctx>,
    /// Part 3 of #382 (#542, PEP 654): `exception_group_partition(group:
    /// *mut PyExceptionObj, tags: *const u8, tags_len: usize,
    /// group_type_tag: u8, group_name: *const u8, group_name_len: usize,
    /// matched_out: *mut *mut PyExceptionObj, rest_out: *mut *mut
    /// PyExceptionObj)` (void). Splits `group`'s members by `tags` into two
    /// output slots -- `emit_try_star` calls this once per `except*`
    /// clause, feeding `rest_out` from one call into the next clause's
    /// `group` input, exactly like `Try`'s own sequential
    /// `exception_type_matches` chain.
    pub(super) exception_group_partition: FunctionValue<'ctx>,
    /// Lexically enclosing `except` handler values used by bare `raise`.
    /// Each handler owns an LLVM local slot. A stack is required because a
    /// nested handler must not overwrite the exception saved by its outer
    /// handler.
    pub(super) exceptions: ExceptionCodegenState<'ctx>,
    /// Compiled frames' records and traceback entries (`frames`).
    pub(crate) frames: crate::frames::FrameState<'ctx>,
    /// Each typed array local's header snapshot (`ndarray`), keyed by
    /// function and variable name.
    pub(super) nd_caches: crate::ndarray::NdCaches<'ctx>,
    /// The enclosing loops' `break`/`continue` targets; `loop_control.rs`
    /// owns it.
    pub(super) loops: crate::loop_control::LoopControlState<'ctx>,
    /// `pycc_rt_list_free`/`pycc_rt_instance_free`: the cold call an inline
    /// decrement makes when a count reaches zero (`heap_rc`).
    pub(super) list_free: FunctionValue<'ctx>,
    pub(super) instance_free: FunctionValue<'ctx>,
    /// The out-of-line counts of a dict and a set (`heap_rc`): their
    /// decrements free the object and release its keys or instances.
    pub(super) dict_incref: FunctionValue<'ctx>,
    pub(super) dict_decref: FunctionValue<'ctx>,
    pub(super) set_incref: FunctionValue<'ctx>,
    pub(super) set_decref: FunctionValue<'ctx>,
    /// `pycc_rt_obj_set_new`: an empty set owning its instance elements.
    pub(super) obj_set_new: FunctionValue<'ctx>,
    /// `pycc_rt_obj_set_discard_candidate`: releases an element an
    /// instance-set insertion did not keep.
    pub(super) obj_set_discard_candidate: FunctionValue<'ctx>,
    /// Every reference-counted class of the module (`pycc_hir::
    /// is_refcounted_class`), keyed by its MIR name, with the slot kind
    /// string its layout descriptor records (`pycc_hir::slot_rc_kind`). A
    /// class absent here is an exception class (`exception_classes`).
    pub(super) rc_class_kinds: std::cell::RefCell<HashMap<String, String>>,
    /// Every exception class an instance type can name: the module's own
    /// and the builtin ones. Their instances are the runtime's exception
    /// objects, counted through `exc_incref`/`exc_decref`.
    pub(super) exception_classes: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Every class's record (`crate::class_record`), keyed by MIR name.
    pub(super) class_records:
        std::cell::RefCell<HashMap<String, crate::class_record::Record<'ctx>>>,
    /// The current function's locals that hold borrowed references
    /// (`borrowed_locals`).
    pub(super) borrowed_locals: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The current function's `object` local slots that may hold a tagged
    /// word, a small `int` or an ASCII character, instead of a reference
    /// (`tagged_locals`).
    pub(super) tagged_slots: std::cell::RefCell<std::collections::HashSet<PointerValue<'ctx>>>,
    /// Per function, the entry-block slot holding what its first direct
    /// call's recursion check answered (`direct_call`).
    pub(super) direct_entered: std::cell::RefCell<HashMap<FunctionValue<'ctx>, PointerValue<'ctx>>>,
    /// The names of the current function's `int` parameters and locals,
    /// whose slots its epilogue releases (#625): `return n` of one moves
    /// the slot's reference out instead of retaining a second one.
    pub(super) owned_int_names: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The ranges proven for the current function's `int` locals
    /// (`int_range`).
    pub(super) int_ranges: std::cell::RefCell<crate::int_range::IntRanges>,
    /// The current function's hidden frame slots holding a `for` loop's
    /// own reference to the list it iterates, keyed by the `ForList`
    /// statement's address: the owned-slot epilogue releases one a
    /// `return` or an exception left set.
    pub(super) loop_list_slots: std::cell::RefCell<HashMap<usize, PointerValue<'ctx>>>,
}
