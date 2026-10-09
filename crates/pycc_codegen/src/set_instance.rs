//! Set helpers and the insert of a set of user-class instances (#1343,
//! Part 1 of #1336).
//!
//! The `PyIntSetObj` length, read, resize-check and `int` insert helpers
//! moved here from `lib.rs` unchanged, under AGENTS.md's decomposition rule.
//! The new code emits an instance insert, which the runtime cannot do on
//! its own because it cannot call a user `__hash__` or `__eq__`:
//!
//! - [`emit_set_element_hash`] computes the element's hash once, as
//!   CPython does: by identity, or through a direct call of the user
//!   `__hash__` reduced like `slot_tp_hash`.
//! - [`emit_set_insert`] then either lets `pycc_rt_obj_set_add_identity` do
//!   the whole insert (identity `__eq__`), or drives the probe: for each
//!   stored entry with an equal hash, an identity check and then
//!   `stored.__eq__(new)`, CPython's own argument order; no match pushes.
//! - [`emit_instance_set_literal`] evaluates up to
//!   [`CPYTHON_BUILD_SET_MAX`] elements before inserting any, as CPython's
//!   `BUILD_SET` does, and interleaves evaluation and insertion beyond it.
//!
//! Every user call is followed by `guard_statement_effects`, so a raising
//! `__hash__` or `__eq__` unwinds to the innermost exception target and the
//! partially built set leaks, as every container does (D-124).

use super::*;
use crate::hash::{hash_bool, hash_identity, hash_slot_int_word};

/// Extracts a `PyIntSetObj` pointer from an already-evaluated operand that
/// every upstream check says must be a `set[T]`: `len`'s argument and
/// `emit_set_name_read`'s named local. `what` names the offending operand
/// for the message. Mirrors `expect_list_pointer`/`expect_dict_pointer`
/// exactly, for the identical reason (see `expect_list_pointer`'s own doc
/// comment): one shared helper rather than a `let Scalar::Set(..) = ..
/// else` at each site, so the check stays genuinely covered by the site
/// that is naturally reachable with a non-set operand (a non-set local
/// named by `for`, and a non-set argument to `len`).
pub(super) fn expect_set_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
    let Scalar::Set(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: {what} did not evaluate to a set -- \
             pycc_types::check (T0033/T0037/T0038) should have rejected this before codegen"
        )
    };
    ptr
}

/// Inserts one already-validated encoded D-141 value into a `PyIntSetObj`,
/// shared by `MirExpr::SetLiteral`'s per-element construction and
/// `MirExpr::SetAdd`'s own user-facing `s.add(value)` call (PR-12 Task 11,
/// D-119 -- the second call site; `SetLiteral`'s per-element construction
/// was the first and, until this task, only one). Returns nothing:
/// `pycc_rt_int_set_add` is declared `void`, exactly like
/// `build_int_list_append`/`build_dict_set` above. The dedup check that
/// makes a repeated element collapse to one (D-121) lives entirely inside
/// `pycc_rt_int_set_add` itself -- both callers just call it per value,
/// unconditionally, with no dedup logic of their own.
pub(super) fn build_int_set_add<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    set_ptr: PointerValue<'ctx>,
    encoded_value: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.int_set_add,
            &[set_ptr.into(), encoded_value.into()],
            "set_add",
        )
        .expect("build_call should not fail for a well-formed set add");
}

/// A `PyIntSetObj`'s current element count, as a raw `i64` counter, shared
/// by the `len(s)` builtin's `Ty::Set` branch, `MirStmt::ForSet`'s own loop
/// bound and `frozenset::set_truthy` -- mirrors `build_int_list_len`/
/// `build_dict_len` exactly, for the identical reason.
pub(super) fn build_int_set_len<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    set_ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.int_set_len, &[set_ptr.into()], "set_len")
        .expect("build_call should not fail for a well-formed set length read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_set_len returns a non-void i64")
        .into_int_value()
}

/// Panics (via `pycc_rt_int_set_check_not_resized`) if `current_len` no
/// longer matches `expected_len` -- called once per `ForSet` loop-test
/// evaluation, comparing a freshly re-read length against the length
/// captured once in the preheader. See that runtime function's own doc
/// comment for why `set.add()` (PR-12, D-119) made this reachable.
pub(super) fn build_int_set_check_not_resized<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    current_len: IntValue<'ctx>,
    expected_len: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.int_set_check_not_resized,
            &[current_len.into(), expected_len.into()],
            "set_check_not_resized",
        )
        .expect("build_call should not fail for a well-formed set resize check");
}

/// Reads one element out of a `PyIntSetObj` by insertion-order index, used
/// only by `MirStmt::ForSet`'s own per-iteration element read. The index is
/// a raw counter and the result is an encoded D-141 value, mirroring
/// `build_int_list_get`.
pub(super) fn build_int_set_get<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    set_ptr: PointerValue<'ctx>,
    raw_index: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(
            rt.int_set_get,
            &[set_ptr.into(), raw_index.into()],
            "set_get",
        )
        .expect("build_call should not fail for a well-formed set read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_set_get returns a non-void i64")
        .into_int_value()
}

/// CPython's `STACK_USE_GUIDELINE`: a set display of at most this many
/// elements compiles to one `BUILD_SET`, which evaluates every element
/// before hashing any. A longer display adds elements one at a time, so
/// each element is hashed and inserted before the next is evaluated.
pub(super) const CPYTHON_BUILD_SET_MAX: usize = 30;

/// The code state an instance set insertion emits into. Bundles the
/// arguments every direct user call needs.
pub(super) struct SetEmitter<'a, 'ctx> {
    pub(super) context: &'ctx Context,
    pub(super) builder: &'a inkwell::builder::Builder<'ctx>,
    pub(super) module: &'a inkwell::module::Module<'ctx>,
    pub(super) rt: &'a RtFns<'ctx>,
    pub(super) user_functions: &'a HashMap<&'a str, UserFunction<'ctx>>,
    pub(super) locals: &'a HashMap<String, StorageSlot<'ctx>>,
}

impl<'ctx> SetEmitter<'_, 'ctx> {
    /// Calls the mangled user method `callee` with `leading` as its only
    /// arguments, then guards: a raising method unwinds to the innermost
    /// exception target before its result is used.
    fn call_user_method(
        &self,
        callee: &str,
        leading: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        // MIR resolved `callee` from the class's own method table, which
        // codegen declares in full before any body, so indexing cannot miss.
        let function = &self.user_functions[callee];
        let value = build_call_to_with_leading_args(
            self.context,
            self.builder,
            self.module,
            self.rt,
            self.user_functions,
            self.locals,
            function,
            callee,
            leading,
            &[],
        )
        .try_as_basic_value()
        .expect_basic("a set element `__hash__`/`__eq__` returns a value");
        guard_statement_effects(self.context, self.builder, self.rt);
        value
    }

    /// The element's raw `i64` hash, computed once per insertion as CPython
    /// does. A `bool` result is its own hash; an `int` result is reduced
    /// like `slot_tp_hash`, then its birth reference is retired: a call
    /// result is owned, and there is no source expression to consult.
    pub(super) fn element_hash(
        &self,
        element: PointerValue<'ctx>,
        op: &pycc_mir::SetHashOp,
    ) -> IntValue<'ctx> {
        match op {
            pycc_mir::SetHashOp::Identity => hash_identity(self.builder, self.rt, element),
            pycc_mir::SetHashOp::Method { callee, ret } => {
                let result = self
                    .call_user_method(callee, &[element.into()])
                    .into_int_value();
                if *ret == pycc_mir::Ty::Bool {
                    return hash_bool(self.context, self.builder, result);
                }
                let hash = hash_slot_int_word(self.builder, self.rt, result);
                emit_bigint_refcount_call(
                    self.context,
                    self.builder,
                    self.rt,
                    result,
                    BigIntRefcount::Release,
                );
                hash
            }
        }
    }

    /// Inserts `element` into the instance set `set` unless an equal one is
    /// stored. An identity `__eq__` is one runtime call. A user `__eq__`
    /// drives the probe here, since the runtime cannot call user code: for
    /// each stored entry with an equal hash, in insertion order, an identity
    /// check and then `stored.__eq__(element)`; no match pushes. The index
    /// lives in an entry-block slot because the guards and the release move
    /// the builder to fresh blocks.
    pub(super) fn insert(
        &self,
        set: PointerValue<'ctx>,
        element: PointerValue<'ctx>,
        ops: &pycc_mir::SetElementOps,
    ) {
        let (context, builder, rt) = (self.context, self.builder, self.rt);
        let i64_type = context.i64_type();
        let hash = self.element_hash(element, &ops.hash);
        let word = builder
            .build_ptr_to_int(element, i64_type, "set_elem_word")
            .expect("build_ptr_to_int should not fail for an instance pointer");
        let callee = match &ops.eq {
            pycc_mir::SetEqOp::Identity => {
                builder
                    .build_call(
                        rt.obj_set_add_identity,
                        &[set.into(), word.into(), hash.into()],
                        "",
                    )
                    .expect("build_call should not fail for pycc_rt_obj_set_add_identity");
                return;
            }
            pycc_mir::SetEqOp::Method { callee } => callee,
        };
        let function = builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .expect("a set insertion is emitted inside a function");
        let index_slot = build_at_entry_block(builder, function, |b| {
            b.build_alloca(i64_type, "set_probe_index")
                .expect("build_alloca should not fail")
        });
        let candidate = |from: IntValue<'ctx>| {
            builder
                .build_call(
                    rt.obj_set_candidate,
                    &[set.into(), hash.into(), from.into()],
                    "set_candidate",
                )
                .expect("build_call should not fail for pycc_rt_obj_set_candidate")
                .try_as_basic_value()
                .expect_basic("pycc_rt_obj_set_candidate returns an i64")
                .into_int_value()
        };
        builder
            .build_store(index_slot, candidate(i64_type.const_zero()))
            .expect("build_store should not fail");
        let test_bb = context.append_basic_block(function, "set_probe_test");
        let probe_bb = context.append_basic_block(function, "set_probe");
        let call_bb = context.append_basic_block(function, "set_probe_eq");
        let next_bb = context.append_basic_block(function, "set_probe_next");
        let push_bb = context.append_basic_block(function, "set_probe_push");
        let done_bb = context.append_basic_block(function, "set_probe_done");
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail");

        builder.position_at_end(test_bb);
        let index = builder
            .build_load(i64_type, index_slot, "set_probe_i")
            .expect("build_load should not fail")
            .into_int_value();
        let exhausted = builder
            .build_int_compare(
                IntPredicate::EQ,
                index,
                i64_type.const_all_ones(),
                "set_probe_exhausted",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(exhausted, push_bb, probe_bb)
            .expect("build_conditional_branch should not fail");

        builder.position_at_end(probe_bb);
        let stored = build_int_set_get(builder, rt, set, index);
        let same = builder
            .build_int_compare(IntPredicate::EQ, stored, word, "set_probe_same")
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(same, done_bb, call_bb)
            .expect("build_conditional_branch should not fail");

        builder.position_at_end(call_bb);
        let stored_ptr = builder
            .build_int_to_ptr(
                stored,
                context.ptr_type(inkwell::AddressSpace::default()),
                "set_stored",
            )
            .expect("build_int_to_ptr should not fail");
        let equal = self
            .call_user_method(callee, &[stored_ptr.into(), element.into()])
            .into_int_value();
        let is_equal = builder
            .build_int_compare(
                IntPredicate::NE,
                equal,
                equal.get_type().const_zero(),
                "set_probe_equal",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(is_equal, done_bb, next_bb)
            .expect("build_conditional_branch should not fail");

        builder.position_at_end(next_bb);
        let from = builder
            .build_int_add(index, i64_type.const_int(1, false), "set_probe_from")
            .expect("build_int_add should not fail");
        builder
            .build_store(index_slot, candidate(from))
            .expect("build_store should not fail");
        builder
            .build_unconditional_branch(test_bb)
            .expect("build_unconditional_branch should not fail");

        builder.position_at_end(push_bb);
        builder
            .build_call(rt.obj_set_push, &[set.into(), word.into(), hash.into()], "")
            .expect("build_call should not fail for pycc_rt_obj_set_push");
        builder
            .build_unconditional_branch(done_bb)
            .expect("build_unconditional_branch should not fail");
        builder.position_at_end(done_bb);
    }

    /// A fresh, empty set.
    pub(super) fn new_set(&self) -> PointerValue<'ctx> {
        self.builder
            .build_call(self.rt.int_set_new, &[], "set_new")
            .expect("build_call should not fail for a well-formed set construction")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_set_new returns a non-void pointer")
            .into_pointer_value()
    }

    /// Evaluates one element expression to its instance pointer.
    fn element(&self, element: &MirExpr) -> PointerValue<'ctx> {
        let scalar = emit_expr(
            self.context,
            self.builder,
            self.module,
            self.rt,
            self.user_functions,
            self.locals,
            element,
        );
        expect_instance_pointer(scalar, "a set element")
    }

    /// `{e1, e2, ...}` of instances, in CPython's evaluation order: up to
    /// [`CPYTHON_BUILD_SET_MAX`] elements are all evaluated before the first
    /// is hashed; a longer display evaluates and inserts one at a time.
    pub(super) fn literal(
        &self,
        elements: &[MirExpr],
        ops: &pycc_mir::SetElementOps,
    ) -> PointerValue<'ctx> {
        let set = self.new_set();
        if elements.len() <= CPYTHON_BUILD_SET_MAX {
            let pointers: Vec<PointerValue<'ctx>> = elements
                .iter()
                .map(|element| self.element(element))
                .collect();
            for pointer in pointers {
                self.insert(set, pointer, ops);
            }
        } else {
            for element in elements {
                let pointer = self.element(element);
                self.insert(set, pointer, ops);
            }
        }
        set
    }

    /// `s.add(value)` of an instance: evaluate, then insert.
    pub(super) fn add(
        &self,
        set: PointerValue<'ctx>,
        value: &MirExpr,
        ops: &pycc_mir::SetElementOps,
    ) {
        let pointer = self.element(value);
        self.insert(set, pointer, ops);
    }
}

/// The `Scalar` a set iteration binds for one stored word: the word itself
/// for a `set[int]`, or the instance pointer it holds for a `set[C]`.
pub(super) fn set_element_scalar<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    element_ty: &pycc_mir::Ty,
    word: IntValue<'ctx>,
) -> Scalar<'ctx> {
    if !matches!(element_ty, pycc_mir::Ty::Instance(..)) {
        return Scalar::Int(word);
    }
    Scalar::Instance(
        builder
            .build_int_to_ptr(
                word,
                context.ptr_type(inkwell::AddressSpace::default()),
                "set_elem_instance",
            )
            .expect("build_int_to_ptr should not fail for a stored instance word"),
    )
}
