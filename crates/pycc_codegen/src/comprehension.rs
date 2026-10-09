//! Comprehension emission (PR-12 Tasks 5a/5b, D-117): one loop builder for
//! the list, set and dict comprehension forms.
//!
//! Carved out of `lib.rs` per AGENTS.md's "Keep source files decomposable"
//! (D-185 tracking issue #545). The three `MirStmt::*CompAssign` arms were
//! near-identical inline copies of one another; each now calls
//! [`emit_comprehension`] and binds the returned container to its target.
//!
//! **Shape.** A comprehension is a *fourth* intentional copy of `ForRange`'s
//! loop-building shape (`ForList`'s doc comment names `ForList`/`ForDict`/
//! `ForSet` as the first three). It differs from every `For*` arm in
//! (a) allocating the produced container as a free-standing SSA pointer
//! before the loop starts and handing it back only once the loop has
//! finished, (b) branching *internally* on `source`'s kind for which
//! per-iteration `_get`/`_len` FFI pair backs the loop test and body
//! (mirroring `MirExpr::Subscript`'s "one MIR node, one codegen arm, branch
//! internally on the resolved kind" precedent, PR-11b), and (c) running a
//! conditionally executed element step instead of arbitrary user statements.
//!
//! **Target binding is the caller's job, and it comes last.** Python fully
//! evaluates an assignment's right-hand side before rebinding its target; a
//! comprehension's "right-hand side" is the entire loop. `source`, `cond` and
//! the elements may all name the target (`xs = [x for x in xs if x > 2]`),
//! and until the loop finishes the target must still resolve to whatever it
//! already held. Storing the fresh container early once made a
//! self-referential comprehension read its own emptied slot (a confirmed
//! regression -- see `a_list_sourced_list_comprehension_that_rebinds_its_own_
//! source_name_reads_the_pre_existing_value` and its neighbours in the
//! crate's `tests` module). The container needs no slot to stay live across
//! the loop: it is defined in a block that dominates every block the loop
//! creates, so each element step references it as a plain SSA value.
//!
//! **No terminator guard on the back edge.** Every `For*` arm checks
//! `get_terminator().is_none()` before its increment because a `Return` in an
//! arbitrary user `body` can already have terminated the body block. A
//! comprehension's only "body" is `cond` and the element expressions, plain
//! `MirExpr` trees with no `MirStmt::Return` to reach, so the block the
//! builder sits in after the element step is always unterminated, and the
//! guard's "already terminated" side could never be exercised.
//!
//! Block names carry a `listcomp_`/`setcomp_`/`dictcomp_` prefix purely for
//! readable IR dumps; the strings have no observable behaviour.

use super::bigint_rc::{
    BigIntRefcount, emit_bigint_refcount_call, int_temporary_word, release_if_int_temporary,
    release_scalar_if_int_temporary,
};
use super::rt_fns::RtFns;
use super::set_instance::{SetEmitter, set_element_scalar};
use super::{
    Scalar, StorageSlot, UserFunction, build_at_entry_block, build_dict_len, build_dict_set,
    build_int_list_append, build_int_list_get, build_int_list_len, build_int_set_add,
    build_int_set_check_not_resized, build_int_set_get, build_int_set_len, build_untag_checked,
    emit_assign, emit_dict_name_read, emit_expr, emit_list_name_read,
    emit_range_operands_with_exception_safety, emit_set_name_read, guard_statement_effects,
    incref_if_str_duplicate, to_encoded_int, truthy, ty_to_basic_type,
};
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{FunctionValue, IntValue, PhiValue, PointerValue};
use pycc_mir::{CompSource, MirCompElt, MirComprehension, MirExpr, SetElementOps, Ty};
use std::collections::HashMap;

/// The element expressions of one comprehension, by produced container.
#[derive(Clone, Copy)]
pub(super) enum CompElts<'a> {
    /// `[elt for ...]`, producing `list[int]` (T0034).
    List(&'a MirExpr),
    /// `{elt for ...}`, producing `set[int]` (T0038) or, with `Some` ops, a
    /// `set[C]` of user-class instances (#1344, D-255): the ops are the
    /// element class's `__hash__`/`__eq__`, and the insertion is
    /// `SetEmitter::add`'s, exactly as for a set literal or `.add(...)`.
    Set(&'a MirExpr, Option<&'a SetElementOps>),
    /// `{key: value for ...}`, producing `dict[str, int]` (T0036).
    Dict(&'a MirExpr, &'a MirExpr),
}

impl CompElts<'_> {
    /// The IR block-name prefix and the container word used in value names.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            CompElts::List(_) => ("listcomp", "list"),
            CompElts::Set(..) => ("setcomp", "set"),
            CompElts::Dict(..) => ("dictcomp", "dict"),
        }
    }
}

/// The codegen context every emitter in this module threads through.
pub(super) struct CompCx<'a, 'ctx> {
    pub(super) context: &'ctx Context,
    pub(super) builder: &'a Builder<'ctx>,
    pub(super) module: &'a Module<'ctx>,
    pub(super) rt: &'a RtFns<'ctx>,
    pub(super) user_functions: &'a HashMap<&'a str, UserFunction<'ctx>>,
    pub(super) locals: &'a HashMap<String, StorageSlot<'ctx>>,
}

/// How the shared increment step adds the phi's back-edge value and
/// branches back to the loop test.
enum CompLoopTail<'ctx> {
    /// A `range` source: a tagged `pycc_rt_int_add` step.
    Range {
        induction: PhiValue<'ctx>,
        current: IntValue<'ctx>,
        step_v: IntValue<'ctx>,
    },
    /// A container source: a raw `i64` index step.
    Indexed {
        induction: PhiValue<'ctx>,
        current: IntValue<'ctx>,
    },
}

/// The loop skeleton [`open_loop`] leaves behind for the shared tail.
struct CompLoop<'ctx> {
    test_bb: BasicBlock<'ctx>,
    after_bb: BasicBlock<'ctx>,
    tail: CompLoopTail<'ctx>,
    /// Freshly built `range` bounds this loop must release at `after_bb`.
    owned_range_operands: Vec<IntValue<'ctx>>,
}

/// Emits the whole comprehension loop and returns the produced container
/// (`Scalar::List`/`Set`/`Dict`). The builder is left at the end of the
/// loop's `after` block (past its exception checkpoint, for a set source).
/// `var` is the D-117 synthesized loop variable of type `var_ty`, whose
/// slot must already exist in `cx.locals`.
pub(super) fn emit_comprehension<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    var: &str,
    var_ty: &Ty,
    source: &CompSource,
    cond: Option<&MirExpr>,
    elts: CompElts<'_>,
) -> Scalar<'ctx> {
    let (prefix, word) = elts.names();
    let function = cx.builder.get_insert_block().unwrap().get_parent().unwrap();
    let (new_fn, what) = match elts {
        CompElts::List(_) => (cx.rt.int_list_new, "pycc_rt_int_list_new"),
        CompElts::Set(..) => (cx.rt.int_set_new, "pycc_rt_int_set_new"),
        CompElts::Dict(..) => (cx.rt.dict_new, "pycc_rt_dict_new"),
    };
    let container = cx
        .builder
        .build_call(new_fn, &[], &format!("comp_{word}_new"))
        .expect("build_call should not fail for a well-formed container allocation")
        .try_as_basic_value()
        .expect_basic(&format!("{what} returns a non-void pointer"))
        .into_pointer_value();

    let lp = open_loop(cx, function, prefix, var, var_ty, source);

    // The filter: with `cond`, a small `if_taken`/`if_skip` block pair
    // (mirroring `MirStmt::If`'s two-block shape) where `if_skip` doubles as
    // the join point; without it, the element step runs unconditionally.
    match cond {
        Some(cond_expr) => {
            let cond_scalar = emit_expr(
                cx.context,
                cx.builder,
                cx.module,
                cx.rt,
                cx.user_functions,
                cx.locals,
                cond_expr,
            );
            // A produced CPython-object filter (`if o.ready()`) is held across
            // its truth test and released after it (Part 1 of #1092).
            let held =
                crate::object_release::hold(cx.context, cx.module, cx.rt, cond_expr, &cond_scalar);
            let cond_i1 = truthy(cx.context, cx.builder, cx.module, cx.rt, cond_scalar);
            held.release(cx.builder, cx.rt);
            // #146 Part 2 (D-181): released after `truthy`, which reads a
            // bigint operand's limbs.
            release_scalar_if_int_temporary(cx.context, cx.builder, cx.rt, cond_expr, &cond_scalar);
            let if_taken_bb = cx
                .context
                .append_basic_block(function, &format!("{prefix}_if_taken"));
            let if_skip_bb = cx
                .context
                .append_basic_block(function, &format!("{prefix}_if_skip"));
            cx.builder
                .build_conditional_branch(cond_i1, if_taken_bb, if_skip_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");
            cx.builder.position_at_end(if_taken_bb);
            emit_element(cx, prefix, container, elts);
            cx.builder.build_unconditional_branch(if_skip_bb).expect(
                "build_unconditional_branch should not fail on a block with no terminator yet",
            );
            cx.builder.position_at_end(if_skip_bb);
        }
        None => emit_element(cx, prefix, container, elts),
    }

    close_loop(cx, prefix, lp);
    if matches!(source, CompSource::Set(_)) {
        // #1344: a set source's loop also exits early on a pending
        // exception (see `open_loop`'s `CompSource::Set` arm), so the
        // container must not reach the enclosing statement or expression
        // while that exception is pending -- the same checkpoint
        // `MirStmt::ForSet` takes at its own `after` block.
        guard_statement_effects(cx.context, cx.builder, cx.rt);
    }

    match elts {
        CompElts::List(_) => Scalar::List(container),
        CompElts::Set(..) => Scalar::Set(container),
        CompElts::Dict(..) => Scalar::Dict(container),
    }
}

/// `emit_expr`'s `MirExpr::Comprehension` arm (#1254, D-250): a
/// comprehension in any expression position.
///
/// The loop variable gets a slot of its own, hoisted to the top of the
/// function's entry block by [`build_at_entry_block`] so a comprehension
/// inside a loop does not grow the stack, and visible only through a clone
/// of `locals` -- the enclosing scope never gains a binding for it, which is
/// also why it can never become a module global (#1237). The zero word
/// stored beside the `alloca` runs once per call; for an `int` variable it
/// is what makes `emit_assign`'s release-before-store a no-op on the first
/// iteration.
///
/// After the loop the `int` variable's last value is released and the slot
/// zeroed again, so the next run of the same comprehension (inside a loop,
/// or after a caught exception) starts from the same invariant: the slot
/// holds `0` or one word it owns. A `str` variable (a dict source's keys)
/// keeps `MirStmt::ForDict`'s own leak-only binding, like the statement
/// form.
pub(super) fn emit_comprehension_expr<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    comp: &MirComprehension,
) -> Scalar<'ctx> {
    let function = cx
        .builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a comprehension is always emitted inside a function body");
    let slot_ty = ty_to_basic_type(cx.context, comp.var_ty.clone());
    let ptr = build_at_entry_block(cx.builder, function, |b| {
        let ptr = b
            .build_alloca(slot_ty, &comp.var)
            .expect("build_alloca should not fail for a supported loop-variable type");
        b.build_store(ptr, slot_ty.const_zero())
            .expect("build_store should not fail immediately after its own alloca");
        ptr
    });
    let mut scoped = cx.locals.clone();
    scoped.insert(
        comp.var.clone(),
        StorageSlot {
            ptr,
            ty: comp.var_ty.clone(),
            initialized: None,
        },
    );
    let inner = CompCx {
        locals: &scoped,
        ..*cx
    };
    let elts = match &comp.elt {
        MirCompElt::List(elt) => CompElts::List(elt),
        MirCompElt::Set(elt, ops) => CompElts::Set(elt, ops.as_deref()),
        MirCompElt::Dict { key, value } => CompElts::Dict(key, value),
    };
    // Part 1 of #1255: a CPython iterable builds a CPython list or set, and
    // its loop variable is a borrowed object pointer with nothing to release.
    if let CompSource::Object(iterable) = &comp.source {
        return super::object_comprehension::emit_object_comprehension(
            &inner,
            ptr,
            iterable,
            comp.cond.as_ref(),
            elts,
        );
    }
    let container = emit_comprehension(
        &inner,
        &comp.var,
        &comp.var_ty,
        &comp.source,
        comp.cond.as_ref(),
        elts,
    );
    if comp.var_ty == Ty::Int {
        let last = cx
            .builder
            .build_load(cx.context.i64_type(), ptr, "comp_var_last")
            .expect("build_load should not fail for this function's own alloca")
            .into_int_value();
        emit_bigint_refcount_call(cx.context, cx.builder, cx.rt, last, BigIntRefcount::Release);
        cx.builder
            .build_store(ptr, cx.context.i64_type().const_zero())
            .expect("build_store should not fail for this function's own alloca");
    }
    container
}

/// Evaluates the element expressions, runs the same validation and
/// identity-preserving storage as `ListAppend`/`SetAdd`/`DictSet`, and adds
/// the element to `container`.
fn emit_element<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    prefix: &str,
    container: PointerValue<'ctx>,
    elts: CompElts<'_>,
) {
    let emit_one = |expr: &MirExpr| {
        emit_expr(
            cx.context,
            cx.builder,
            cx.module,
            cx.rt,
            cx.user_functions,
            cx.locals,
            expr,
        )
    };
    match elts {
        CompElts::Set(elt, Some(ops)) => {
            let emitter = SetEmitter {
                context: cx.context,
                builder: cx.builder,
                module: cx.module,
                rt: cx.rt,
                user_functions: cx.user_functions,
                locals: cx.locals,
            };
            emitter.add(container, elt, ops);
        }
        CompElts::List(elt) | CompElts::Set(elt, None) => {
            let encoded = to_encoded_int(cx.context, cx.builder, emit_one(elt));
            let _ = build_untag_checked(
                cx.builder,
                cx.rt,
                encoded,
                &format!("{prefix}_validate_elt"),
            );
            if matches!(elts, CompElts::List(_)) {
                build_int_list_append(cx.builder, cx.rt, container, encoded);
            } else {
                build_int_set_add(cx.builder, cx.rt, container, encoded);
            }
        }
        CompElts::Dict(key, value) => {
            // `pycc_rt_dict_set` adopts the key pointer as the dict's own
            // permanent reference without incref'ing it, so a duplicate key
            // reference (a bare `Name`, `{k: 1 for k in d}`) is incref'd first,
            // exactly as `MirStmt::DictSet` does (D-124). A no-op for a key
            // that is already fresh, such as an f-string.
            let key_scalar = incref_if_str_duplicate(cx.builder, cx.rt, key, emit_one(key));
            let Scalar::Str(key_ptr) = key_scalar else {
                panic!(
                    "pycc_codegen: internal error: dict comprehension key did not evaluate \
                     to str -- pycc_types::check (T0036) should have rejected this before \
                     codegen"
                )
            };
            let encoded = to_encoded_int(cx.context, cx.builder, emit_one(value));
            let _ = build_untag_checked(
                cx.builder,
                cx.rt,
                encoded,
                &format!("{prefix}_validate_value"),
            );
            build_dict_set(cx.builder, cx.rt, container, key_ptr, encoded);
        }
    }
}

/// Builds the loop's test/body/after blocks for `source`, positions the
/// builder at the start of the body and binds `var` there. Each source kind
/// mirrors the matching `For*` arm's preheader/test/body shape exactly.
fn open_loop<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    function: FunctionValue<'ctx>,
    prefix: &str,
    var: &str,
    var_ty: &Ty,
    source: &CompSource,
) -> CompLoop<'ctx> {
    let (context, builder, rt) = (cx.context, cx.builder, cx.rt);
    match source {
        CompSource::Object(_) => panic!(
            "pycc_codegen: internal error: a comprehension over a CPython object reached the \
             native comprehension loop -- `emit_comprehension_expr` routes it to \
             `object_comprehension`, and pycc_hir never lowers it to a statement form"
        ),
        CompSource::Range { start, stop, step } => {
            // Mirrors `MirStmt::ForRange`'s own shape exactly.
            let (start_v, stop_v, step_v) = emit_range_operands_with_exception_safety(
                context,
                builder,
                cx.module,
                rt,
                cx.user_functions,
                cx.locals,
                start,
                stop,
                step,
            );
            // #146 Part 1: same ownership contract as `MirStmt::ForRange`,
            // minus its `stop_v`/`step_v` retain/release pair -- this loop
            // only ever *reads* those, so not retaining and not releasing
            // them is already balanced. `start_v` becomes the first
            // `current`, which the per-iteration and `after` releases retire,
            // so it is retained here. Do not additionally route these
            // operands through `retain_if_int_duplicate`.
            emit_bigint_refcount_call(context, builder, rt, start_v, BigIntRefcount::Retain);
            // #146 Part 2 (D-181): `start_v`'s birth reference is retired at
            // once, exactly as in `MirStmt::ForRange`. `stop_v`/`step_v`
            // cannot be: `pycc_rt_range_continue` re-reads both on every
            // iteration, so they are carried to `after` -- past the last
            // read -- and released there.
            release_if_int_temporary(context, builder, rt, start, start_v);
            let owned_range_operands: Vec<IntValue<'ctx>> = [
                int_temporary_word(stop, stop_v),
                int_temporary_word(step, step_v),
            ]
            .into_iter()
            .flatten()
            .collect();
            // Re-read after the retain: it splits the block.
            let preheader = builder.get_insert_block().unwrap();
            let (test_bb, body_bb, after_bb) = append_loop_blocks(cx, function, prefix);
            builder
                .build_unconditional_branch(test_bb)
                .expect("build_unconditional_branch should not fail entering the loop test");
            builder.position_at_end(test_bb);
            let induction = builder
                .build_phi(context.i64_type(), &format!("{prefix}_current"))
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
                    &format!("{prefix}_cont"),
                )
                .expect("build_int_compare should not fail comparing two i8 operands");
            builder
                .build_conditional_branch(cont_i1, body_bb, after_bb)
                .expect("build_conditional_branch should not fail for a well-formed i1 condition");
            builder.position_at_end(body_bb);
            // Retain before `emit_assign`'s release-before-store, exactly as
            // in `MirStmt::ForRange`.
            emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Retain);
            emit_assign(context, builder, rt, cx.locals, var, Scalar::Int(current));
            CompLoop {
                test_bb,
                after_bb,
                tail: CompLoopTail::Range {
                    induction,
                    current,
                    step_v,
                },
                owned_range_operands,
            }
        }
        CompSource::List(name) => {
            // Mirrors `MirStmt::ForList`'s own shape exactly.
            let list_ptr = emit_list_name_read(
                context,
                builder,
                cx.module,
                rt,
                cx.user_functions,
                cx.locals,
                name,
            );
            // CPython lets a list grow while it is iterated, so the length is
            // re-read on every trip with no resize guard.
            let (lp, current) = open_indexed_loop(cx, function, prefix, None, |b| {
                build_int_list_len(b, rt, list_ptr)
            });
            let encoded_element = build_int_list_get(builder, rt, list_ptr, current);
            emit_assign(
                context,
                builder,
                rt,
                cx.locals,
                var,
                Scalar::Int(encoded_element),
            );
            lp
        }
        CompSource::Dict(name) => {
            // Mirrors `MirStmt::ForDict`'s own shape exactly, including its
            // `pycc_rt_str_incref` on the read key before the per-iteration
            // bind: that keeps `var`'s reference alive across the iteration
            // without corrupting the source dict's own key. Like `ForDict`,
            // this write never calls `decref_str_slot_before_store`. A
            // type-checked program never routes a `Dict` source into a list
            // or set comprehension (T0034/T0038 need an `int` element and no
            // `str`-to-`int` builtin exists), but the binding is correct
            // regardless -- see `a_dict_sourced_list_comprehension_binds_its_
            // key_without_crashing`, which reaches it through hand-built MIR.
            let dict_ptr = emit_dict_name_read(
                context,
                builder,
                cx.module,
                rt,
                cx.user_functions,
                cx.locals,
                name,
            );
            // A dict grown during the loop is not guarded yet (#1361).
            let (lp, current) = open_indexed_loop(cx, function, prefix, None, |b| {
                build_dict_len(b, rt, dict_ptr)
            });
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
            builder
                .build_call(
                    rt.str_incref,
                    &[key_ptr.into()],
                    &format!("{prefix}_dict_key_incref"),
                )
                .expect("build_call should not fail for a well-formed incref");
            emit_assign(context, builder, rt, cx.locals, var, Scalar::Str(key_ptr));
            lp
        }
        CompSource::Set(name) => {
            // Mirrors `MirStmt::ForSet`'s own shape, including its binding
            // of the element word through `set_element_scalar` -- an `int`
            // or, for a `set[C]`, the instance pointer (#1344) -- and its
            // resize guard: the length is snapshotted here, in the
            // preheader, and a set grown by the element step (a user
            // `__hash__`, `__eq__` or call) raises CPython's `RuntimeError:
            // Set changed size during iteration` instead of looping forever.
            let set_ptr = emit_set_name_read(
                context,
                builder,
                cx.module,
                rt,
                cx.user_functions,
                cx.locals,
                name,
            );
            let initial_len = build_int_set_len(builder, rt, set_ptr);
            let (lp, current) = open_indexed_loop(cx, function, prefix, Some(initial_len), |b| {
                build_int_set_len(b, rt, set_ptr)
            });
            let encoded_element = build_int_set_get(builder, rt, set_ptr, current);
            emit_assign(
                context,
                builder,
                rt,
                cx.locals,
                var,
                set_element_scalar(context, builder, var_ty, encoded_element),
            );
            lp
        }
    }
}

/// Appends the loop's test, body and after blocks.
fn append_loop_blocks<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    function: FunctionValue<'ctx>,
    prefix: &str,
) -> (BasicBlock<'ctx>, BasicBlock<'ctx>, BasicBlock<'ctx>) {
    (
        cx.context
            .append_basic_block(function, &format!("{prefix}_test")),
        cx.context
            .append_basic_block(function, &format!("{prefix}_body")),
        cx.context
            .append_basic_block(function, &format!("{prefix}_after")),
    )
}

/// The container-source loop skeleton: a raw `i64` index `phi` tested
/// against a length `len` re-reads on every trip. Leaves the builder at the
/// start of the body and returns the index for the caller's element read.
///
/// `set_resize_guard` is a set source's preheader length snapshot (#1344).
/// With it the test also mirrors `MirStmt::ForSet`'s: every fresh length is
/// checked against the snapshot (`pycc_rt_int_set_check_not_resized`, which
/// raises the D-173 `RuntimeError`), and the loop continues only while no
/// exception is pending -- without that conjunct a raise that returns
/// normally would leave `index < len` true forever. See `ForSet`'s own
/// comments for why the resize check cannot relabel a pending exception.
fn open_indexed_loop<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    function: FunctionValue<'ctx>,
    prefix: &str,
    set_resize_guard: Option<IntValue<'ctx>>,
    len: impl FnOnce(&Builder<'ctx>) -> IntValue<'ctx>,
) -> (CompLoop<'ctx>, IntValue<'ctx>) {
    let (context, builder) = (cx.context, cx.builder);
    let preheader = builder.get_insert_block().unwrap();
    let (test_bb, body_bb, after_bb) = append_loop_blocks(cx, function, prefix);
    builder
        .build_unconditional_branch(test_bb)
        .expect("build_unconditional_branch should not fail entering the loop test");
    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(context.i64_type(), &format!("{prefix}_index"))
        .expect("build_phi should not fail in a fresh loop-test block");
    let zero = context.i64_type().const_zero();
    induction.add_incoming(&[(&zero, preheader)]);
    let current = induction.as_basic_value().into_int_value();
    let len = len(builder);
    let in_range = builder
        .build_int_compare(IntPredicate::SLT, current, len, &format!("{prefix}_cont"))
        .expect("build_int_compare should not fail comparing two i64 operands");
    let cont = match set_resize_guard {
        None => in_range,
        Some(initial_len) => {
            build_int_set_check_not_resized(builder, cx.rt, len, initial_len);
            let exc_active = builder
                .build_call(cx.rt.exception_active, &[], &format!("{prefix}_exc_active"))
                .expect("build_call should not fail for exception_active")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_active returns i8")
                .into_int_value();
            let no_exception = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    exc_active,
                    exc_active.get_type().const_zero(),
                    &format!("{prefix}_no_exc"),
                )
                .expect("build_int_compare should not fail comparing an i8 against zero");
            builder
                .build_and(in_range, no_exception, &format!("{prefix}_cont_ok"))
                .expect("build_and should not fail for two i1 operands")
        }
    };
    builder
        .build_conditional_branch(cont, body_bb, after_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");
    builder.position_at_end(body_bb);
    (
        CompLoop {
            test_bb,
            after_bb,
            tail: CompLoopTail::Indexed { induction, current },
            // A container-iterating comprehension has no `range` bounds to
            // own.
            owned_range_operands: Vec::new(),
        },
        current,
    )
}

/// Increments, branches back to the loop test, and positions the builder at
/// `after`, releasing what the loop still owns there.
fn close_loop<'ctx>(cx: &CompCx<'_, 'ctx>, prefix: &str, lp: CompLoop<'ctx>) {
    let (context, builder, rt) = (cx.context, cx.builder, cx.rt);
    // `Some(current)` for a `range` source: the final `current` (the one that
    // failed `range_continue`) was never bound to the loop variable and needs
    // its own release at `after`. A comprehension has no `return`, so this
    // release is unconditional.
    let unconsumed_current = match lp.tail {
        CompLoopTail::Range {
            induction,
            current,
            step_v,
        } => {
            let next = builder
                .build_call(
                    rt.int_add,
                    &[current.into(), step_v.into()],
                    &format!("{prefix}_next"),
                )
                .expect("build_call should not fail for a well-formed int add")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_add returns a non-void i64")
                .into_int_value();
            // This iteration's `current` is dead once `next` exists.
            emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
            // Re-read after the release: it splits the block.
            let body_end = builder.get_insert_block().unwrap();
            induction.add_incoming(&[(&next, body_end)]);
            builder.build_unconditional_branch(lp.test_bb).expect(
                "build_unconditional_branch should not fail on a block with no terminator yet",
            );
            Some(current)
        }
        CompLoopTail::Indexed { induction, current } => {
            let next = builder
                .build_int_add(
                    current,
                    context.i64_type().const_int(1, false),
                    &format!("{prefix}_next"),
                )
                .expect("build_int_add should not fail for two i64 operands");
            let body_end = builder.get_insert_block().unwrap();
            induction.add_incoming(&[(&next, body_end)]);
            builder.build_unconditional_branch(lp.test_bb).expect(
                "build_unconditional_branch should not fail on a block with no terminator yet",
            );
            // A container index is a raw `i64` counter, never a D-141
            // encoded word, so it owns nothing to release.
            None
        }
    };
    builder.position_at_end(lp.after_bb);
    if let Some(current) = unconsumed_current {
        emit_bigint_refcount_call(context, builder, rt, current, BigIntRefcount::Release);
    }
    // #146 Part 2 (D-181): the freshly built `range` bounds, past their last
    // `pycc_rt_range_continue` read.
    for word in lp.owned_range_operands {
        emit_bigint_refcount_call(context, builder, rt, word, BigIntRefcount::Release);
    }
}
