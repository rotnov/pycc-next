//! Tagged `object` locals: a slot of a generic body that holds a small
//! `int` or an ASCII character as a word of its own, not as an object.
//!
//! In a scanning loop (`while True: char = src[pos] ... pos += 1`) every
//! step otherwise reads `pos`'s `int` object (its type, its tag, its digit),
//! writes the next value back into it ([`crate::fast_ops::Fast::int_in_place`]),
//! and checks `char`'s type, state and length again at each comparison.
//! A tagged slot holds instead:
//!
//! - a reference (an aligned pointer, the low three bits zero), or null;
//! - a small `int` `v` as `v << 1 | 1` (D-061's smallint word), for
//!   `-2**62 <= v < 2**62`;
//! - an ASCII character `c` as `c << 3 | 2` ([`CHAR_TAG`]).
//!
//! A tagged word is made only where a value is produced: `x = x + y` or
//! `x = x - y` ([`emit_rebinding`]) and `c = s[i]` of an exact ASCII `str`
//! ([`emit_char_load`]). It is consumed as a word only where its value is
//! read: an index ([`emit_subscript`]), a comparison with a one-character
//! literal ([`emit_eq_char`]), `c in frozenset` ([`emit_contains`]), and the
//! two producers' operands. Every other read is an object read
//! ([`read_object`]): the word is boxed -- `int(v)` (a cached object for
//! `-5..=256`, as CPython's own result), or CPython's cached
//! one-character `str` -- and the box is written back into the slot, which
//! owns it, so every later read of the same value sees the same object, as
//! a CPython local would. A release of a tagged slot (a store, the
//! function's epilogue) releases nothing for a tagged word
//! ([`release_word`]).
//!
//! Which locals are tagged ([`tagged_locals`]): an owning `object` local or
//! parameter of a function that is not a generator, with at least one
//! producer, and whose every mention in the body is a read
//! (`MirExpr::Name`), an assignment target or a `ReleaseName`. The last
//! condition is checked by counting: the name's mentions found by the
//! structural walk must be all its mentions in the body's `Debug` text, so
//! any other kind of mention (a `for` or `except ... as` target, a walrus, a
//! closure's capture, a node the walk does not visit, or a string equal to
//! the name) leaves the local untagged. Compiled frames have no `f_locals`,
//! so nothing outside the body reads a local's slot.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use inkwell::builder::Builder;
use inkwell::values::BasicValueEnum;
use pycc_mir::{BinOpKind, CmpOpKind, Ty};
use std::collections::HashSet;

/// The low bits of a word that mark it as a tag, not a reference.
pub(crate) const TAG_MASK: u64 = 7;
/// The low bits of an ASCII character's word.
pub(crate) const CHAR_TAG: u64 = 2;
/// A small `int`'s value must lie in `-BOUND..BOUND` to be tagged.
const INT_BOUND: u64 = 1 << 62;

/// The tagged locals among `candidates` of the function `function` (module
/// docs). `PYCC_DEBUG_TAGGED` prints each candidate's verdict to stderr.
pub(crate) fn tagged_locals<'a>(
    function: &str,
    body: &[MirStmt],
    candidates: impl Iterator<Item = &'a String>,
) -> HashSet<String> {
    let mut candidates: Vec<&String> = candidates.collect();
    candidates.sort();
    if candidates.is_empty() {
        return HashSet::new();
    }
    let debug = std::env::var_os("PYCC_DEBUG_TAGGED").is_some();
    let text = format!("{body:?}");
    candidates
        .into_iter()
        .filter(|name| {
            let mut mentions = Mentions::default();
            mentions.stmts(body, name);
            let all = text.matches(&format!("{name:?}")).count();
            let tagged = mentions.producer && mentions.count == all;
            if debug {
                eprintln!(
                    "tagged_locals: {function}.{name}: producer={} walked={} mentions={all} -> {tagged}",
                    mentions.producer, mentions.count
                );
            }
            tagged
        })
        .cloned()
        .collect()
}

/// Whether every mention of `name` in `body` is a read (`MirExpr::Name`):
/// a parameter that is only read keeps the argument it was passed, never
/// null.
pub(crate) fn read_only(body: &[MirStmt], name: &str) -> bool {
    let mut mentions = Mentions::default();
    mentions.stmts(body, name);
    mentions.count == mentions.reads && mentions.count == format!("{body:?}").matches(&format!("{name:?}")).count()
}

/// Tells LLVM that `pointer` is not null (`llvm.assume`).
pub(crate) fn assume_nonnull<'ctx>(
    module: &inkwell::module::Module<'ctx>,
    builder: &Builder<'ctx>,
    pointer: PointerValue<'ctx>,
) {
    let assume = inkwell::intrinsics::Intrinsic::find("llvm.assume")
        .expect("llvm.assume exists")
        .get_declaration(module, &[])
        .expect("llvm.assume takes no type parameters");
    let present = builder.build_is_not_null(pointer, "param_present").expect("icmp");
    builder
        .build_call(assume, &[present.into()], "")
        .expect("build_call should not fail for llvm.assume");
}

/// A local's mentions found by the structural walk.
#[derive(Default)]
struct Mentions {
    count: usize,
    reads: usize,
    producer: bool,
}

impl Mentions {
    fn stmts(&mut self, body: &[MirStmt], name: &str) {
        for stmt in body {
            match stmt {
                MirStmt::Assign { target, value } if target == name => {
                    self.count += 1;
                    self.producer |= rebinding(name, value).is_some() || char_load(value).is_some();
                }
                MirStmt::ReleaseName(released) if released == name => self.count += 1,
                _ => {}
            }
            for expr in stmt.exprs() {
                self.expr(expr, name);
            }
            for inner in stmt.bodies() {
                self.stmts(inner, name);
            }
        }
    }

    fn expr(&mut self, expr: &MirExpr, name: &str) {
        if matches!(expr, MirExpr::Name { name: read, .. } if read == name) {
            self.count += 1;
            self.reads += 1;
        }
        for child in expr.children() {
            self.expr(child, name);
        }
    }
}

/// The operator and right operand of `name = name op right`, `op` one
/// [`emit_rebinding`] takes, with `right` a literal or a name.
fn rebinding<'e>(name: &str, value: &'e MirExpr) -> Option<(BinOpKind, &'e MirExpr)> {
    let MirExpr::ObjBinOp { op, left, right, .. } = value else {
        return None;
    };
    let reads_name = matches!(left.as_ref(), MirExpr::Name { name: read, ty: Ty::Object } if read == name);
    let simple = matches!(right.as_ref(), MirExpr::IntLiteral(_) | MirExpr::Name { ty: Ty::Object, .. });
    (reads_name && simple && matches!(op, BinOpKind::Add | BinOpKind::Sub)).then_some((*op, right))
}

/// The base and index of `base[index]` read from names (or an index
/// literal), which [`emit_char_load`] takes.
fn char_load(value: &MirExpr) -> Option<(&MirExpr, &MirExpr)> {
    let MirExpr::ObjSubscript { base, index } = value else {
        return None;
    };
    let base_name = matches!(base.as_ref(), MirExpr::Name { ty: Ty::Object, name } if name != crate::ext::FALLBACK_NS_NAME);
    let index_simple = matches!(index.as_ref(), MirExpr::IntLiteral(_) | MirExpr::Name { ty: Ty::Object, .. });
    (base_name && index_simple).then_some((base.as_ref(), index.as_ref()))
}

/// The slot `expr` reads when it is a read of a tagged local.
pub(crate) fn tagged_read<'ctx>(
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Option<StorageSlot<'ctx>> {
    let MirExpr::Name { name, ty: Ty::Object } = expr else {
        return None;
    };
    tagged_slot(rt, locals, name)
}

/// The slot of `name` when it is a tagged local.
fn tagged_slot<'ctx>(
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    name: &str,
) -> Option<StorageSlot<'ctx>> {
    locals
        .get(name)
        .filter(|slot| rt.tagged_slots.borrow().contains(&slot.ptr))
        .cloned()
}

fn function_of<'ctx>(builder: &Builder<'ctx>) -> FunctionValue<'ctx> {
    builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a tagged local is used inside a function body")
}

fn i64_const<'ctx>(context: &'ctx Context, value: u64) -> IntValue<'ctx> {
    context.i64_type().const_int(value, false)
}

fn ptr_type(context: &Context) -> inkwell::types::PointerType<'_> {
    context.ptr_type(inkwell::AddressSpace::default())
}

/// The word in a tagged slot.
fn load_word<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>, slot: &StorageSlot<'ctx>) -> IntValue<'ctx> {
    let pointer = builder
        .build_load(ptr_type(context), slot.ptr, "tagged_word")
        .expect("build_load should not fail for a declared slot")
        .into_pointer_value();
    builder
        .build_ptr_to_int(pointer, context.i64_type(), "tagged_bits")
        .expect("ptrtoint")
}

/// `word & TAG_MASK == bits` (`word & 1 != 0` for a small `int`'s tag),
/// as an `i1`.
fn has_tag<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>, word: IntValue<'ctx>, bits: u64) -> IntValue<'ctx> {
    if bits == 1 {
        let odd = builder.build_and(word, i64_const(context, 1), "tagged_odd").expect("and");
        return builder
            .build_int_compare(inkwell::IntPredicate::NE, odd, i64_const(context, 0), "tagged_is_int")
            .expect("icmp");
    }
    let low = builder
        .build_and(word, i64_const(context, TAG_MASK), "tagged_low")
        .expect("and");
    builder
        .build_int_compare(inkwell::IntPredicate::EQ, low, i64_const(context, bits), "tagged_kind")
        .expect("icmp")
}

/// Releases `ptr`, a word a tagged slot held: a reference, or nothing.
pub(crate) fn release_word<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ptr: PointerValue<'ctx>,
) {
    let word = builder
        .build_ptr_to_int(ptr, context.i64_type(), "tagged_released")
        .expect("ptrtoint");
    let reference = has_tag(context, builder, word, 0);
    let function = function_of(builder);
    let release = context.append_basic_block(function, "tagged_release");
    let done = context.append_basic_block(function, "tagged_released");
    builder
        .build_conditional_branch(reference, release, done)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(release);
    builder
        .build_call(rt.obj_release, &[ptr.into()], "")
        .expect("build_call should not fail for pycc_ext_obj_release");
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(done);
}

/// Stores `word` (a tag, or an owned reference as an integer) into a
/// tagged slot, then releases what it held (the `Py_XSETREF` order).
fn store_word<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    word: IntValue<'ctx>,
) {
    let slot = &locals[target];
    let old = heap_rc::take_slot_old(context, builder, rt, &slot.ty, slot.ptr);
    let pointer = builder
        .build_int_to_ptr(word, ptr_type(context), "tagged_store")
        .expect("inttoptr");
    emit_assign(context, builder, rt, locals, target, Scalar::Object(pointer));
    heap_rc::release_old(context, builder, rt, old);
}

/// The object a tagged word stands for, a new reference, or `NULL` with
/// the exception set: `int(word >> 1)` or the one-character `str`.
fn box_tag<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    word: IntValue<'ctx>,
) -> PointerValue<'ctx> {
    let ptr = ptr_type(context);
    let i64_type = context.i64_type();
    let function = function_of(builder);
    let int_block = context.append_basic_block(function, "box_int");
    let char_block = context.append_basic_block(function, "box_char");
    let done = context.append_basic_block(function, "boxed");
    let is_int = has_tag(context, builder, word, 1);
    builder
        .build_conditional_branch(is_int, int_block, char_block)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(int_block);
    let value = builder
        .build_right_shift(word, i64_const(context, 1), true, "box_value")
        .expect("ashr");
    let pack = crate::foreign_pack::shim_fn(
        module,
        crate::fast_ops::EXT_OBJ_PACK_I64_SYMBOL,
        ptr.fn_type(&[i64_type.into()], false),
    );
    let int_object = builder
        .build_call(pack, &[value.into()], "box_int_object")
        .expect("build_call should not fail for pycc_ext_obj_pack_i64")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_pack_i64 returns PyObject *")
        .into_pointer_value();
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(char_block);
    let code = builder
        .build_right_shift(word, i64_const(context, 3), false, "box_code")
        .expect("lshr");
    let code = builder
        .build_int_truncate(code, context.i32_type(), "box_code32")
        .expect("trunc");
    let from_ordinal = crate::foreign_pack::shim_fn(
        module,
        "PyUnicode_FromOrdinal",
        ptr.fn_type(&[context.i32_type().into()], false),
    );
    let char_object = builder
        .build_call(from_ordinal, &[code.into()], "box_char_object")
        .expect("build_call should not fail for PyUnicode_FromOrdinal")
        .try_as_basic_value()
        .expect_basic("PyUnicode_FromOrdinal returns PyObject *")
        .into_pointer_value();
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(done);
    let phi = builder.build_phi(ptr, "boxed").expect("phi");
    phi.add_incoming(&[(&int_object, int_block), (&char_object, char_block)]);
    phi.as_basic_value().into_pointer_value()
}

/// A read of a tagged local as an object, borrowed from its slot: a tagged
/// word is boxed on a cold path and the box stored into the slot, which
/// owns it from then on (module docs).
pub(crate) fn read_object<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
) -> PointerValue<'ctx> {
    let ptr = ptr_type(context);
    let word = load_word(context, builder, slot);
    let as_pointer = builder
        .build_int_to_ptr(word, ptr, "tagged_reference")
        .expect("inttoptr");
    let reference = has_tag(context, builder, word, 0);
    let function = function_of(builder);
    let entry = builder.get_insert_block().expect("positioned");
    let boxing = context.append_basic_block(function, "tagged_box");
    let done = context.append_basic_block(function, "tagged_read");
    let branch = builder
        .build_conditional_branch(reference, done, boxing)
        .expect("build_conditional_branch should not fail for an i1");
    crate::int_fast::weight_branch(context, branch, true);
    builder.position_at_end(boxing);
    let edge = ForeignFailEdge::for_current(builder);
    let boxed = box_tag(context, builder, module, word);
    route_null(context, builder, module, rt, edge, boxed, "tagged_box");
    builder
        .build_store(slot.ptr, boxed)
        .expect("build_store should not fail for a declared slot");
    let boxed_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(done);
    let phi = builder.build_phi(ptr, "tagged_object").expect("phi");
    phi.add_incoming(&[(&as_pointer, entry), (&boxed, boxed_end)]);
    phi.as_basic_value().into_pointer_value()
}

/// The `i64` value of an `int` operand on `fast`'s path: a literal, a
/// tagged local's word, or a compact `int` object read from a name.
#[allow(clippy::too_many_arguments)]
fn int_operand<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    fast: &crate::fast_ops::Fast<'_, 'ctx>,
    operand: &MirExpr,
    name: &str,
) -> IntValue<'ctx> {
    match operand {
        MirExpr::IntLiteral(literal) => context.i64_type().const_int(*literal as u64, true),
        _ => match tagged_read(rt, locals, operand) {
            Some(slot) => fast.tagged_int(load_word(context, builder, &slot), name),
            None => {
                let object = expect_object(emit_expr(context, builder, module, rt, user_functions, locals, operand));
                fast.compact_int(object, name)
            }
        },
    }
}

/// `target = target op right` (`op` `+` or `-`, `right` a literal or a
/// name) for a tagged `target`, when [`rebinding`] admits it: two small or
/// compact `int`s give a tagged word, with no object made or changed;
/// anything else is CPython's operator, its result stored. Returns whether
/// it emitted the statement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_rebinding<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: &MirExpr,
) -> bool {
    let Some(slot) = tagged_slot(rt, locals, target) else {
        return false;
    };
    let Some((op, right)) = rebinding(target, value) else {
        return false;
    };
    let MirExpr::ObjBinOp { left, in_place, .. } = value else {
        unreachable!("rebinding admitted it")
    };
    let in_range = |v: IntValue<'ctx>, label: &str| {
        let biased = builder
            .build_int_add(v, i64_const(context, INT_BOUND), label)
            .expect("add");
        builder
            .build_int_compare(inkwell::IntPredicate::ULT, biased, i64_const(context, 2 * INT_BOUND), label)
            .expect("icmp")
    };
    let word = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "tagged_rebind",
        |fast| {
            let a = fast.tagged_int(load_word(context, builder, &slot), "tagged_left");
            let b = int_operand(context, builder, module, rt, user_functions, locals, fast, right, "tagged_right");
            // Both operands are below 2**62 in magnitude (a tag's value, a
            // compact `int`'s, or a literal checked here), so the `i64`
            // sum cannot wrap.
            fast.require(in_range(b, "tagged_right_range"), "tagged_right_range");
            let result = match op {
                BinOpKind::Add => builder.build_int_add(a, b, "tagged_sum"),
                _ => builder.build_int_sub(a, b, "tagged_difference"),
            }
            .expect("an operation of two i64");
            fast.require(in_range(result, "tagged_result_range"), "tagged_result_range");
            let shifted = builder
                .build_left_shift(result, i64_const(context, 1), "tagged_shifted")
                .expect("shl");
            builder
                .build_or(shifted, i64_const(context, 1), "tagged_int_word")
                .expect("or")
                .into()
        },
        || {
            let object = expect_object(crate::foreign_arith::emit_binop(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                op,
                *in_place,
                left,
                right,
            ));
            builder
                .build_ptr_to_int(object, context.i64_type(), "tagged_rebound_object")
                .expect("ptrtoint")
                .into()
        },
    )
    .into_int_value();
    store_word(context, builder, rt, locals, target, word);
    true
}

/// `target = base[index]` for a tagged `target`, when [`char_load`] admits
/// the value: a compact ASCII `str` indexed by a small or compact `int` in
/// range gives the character's tagged word, with no object made or
/// counted; anything else is CPython's subscript, its result stored.
/// Returns whether it emitted the statement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_char_load<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: &MirExpr,
) -> bool {
    if tagged_slot(rt, locals, target).is_none() {
        return false;
    }
    let Some((base, index)) = char_load(value) else {
        return false;
    };
    let base_object = expect_object(emit_expr(context, builder, module, rt, user_functions, locals, base));
    let word = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "tagged_char",
        |fast| {
            let length = fast.ascii_str(base_object, "tagged_char_base");
            let at = int_operand(context, builder, module, rt, user_functions, locals, fast, index, "tagged_char_index");
            let at = fast.index_within(at, length, "tagged_char_index");
            let byte = fast.ascii_byte(base_object, at, "tagged_char_byte");
            let shifted = builder
                .build_left_shift(byte, i64_const(context, 3), "tagged_char_shifted")
                .expect("shl");
            builder
                .build_or(shifted, i64_const(context, CHAR_TAG), "tagged_char_word")
                .expect("or")
                .into()
        },
        || {
            let object = expect_object(emit_expr(context, builder, module, rt, user_functions, locals, value));
            builder
                .build_ptr_to_int(object, context.i64_type(), "tagged_loaded_object")
                .expect("ptrtoint")
                .into()
        },
    )
    .into_int_value();
    store_word(context, builder, rt, locals, target, word);
    true
}

/// Emits `on_tag(word)` where a tagged local's word carries the tag `tag`
/// (1 for a small `int`, [`CHAR_TAG`]) and `on_object(object)` elsewhere
/// (its object read), merging their values.
fn split<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
    tag: u64,
    on_tag: impl FnOnce(IntValue<'ctx>) -> BasicValueEnum<'ctx>,
    on_object: impl FnOnce(PointerValue<'ctx>) -> BasicValueEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    let word = load_word(context, builder, slot);
    let is_tag = has_tag(context, builder, word, tag);
    let function = function_of(builder);
    let tag_block = context.append_basic_block(function, "tagged_word_path");
    let object_block = context.append_basic_block(function, "tagged_object_path");
    let join = context.append_basic_block(function, "tagged_join");
    let branch = builder
        .build_conditional_branch(is_tag, tag_block, object_block)
        .expect("build_conditional_branch should not fail for an i1");
    crate::int_fast::weight_branch(context, branch, true);
    builder.position_at_end(tag_block);
    let tag_value = on_tag(word);
    let tag_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join).expect("branch");
    builder.position_at_end(object_block);
    let object = read_object(context, builder, module, rt, slot);
    let object_value = on_object(object);
    let object_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join).expect("branch");
    builder.position_at_end(join);
    let phi = builder.build_phi(tag_value.get_type(), "tagged_value").expect("phi");
    phi.add_incoming(&[(&tag_value, tag_end), (&object_value, object_end)]);
    phi.as_basic_value()
}

fn expect_object(scalar: Scalar<'_>) -> PointerValue<'_> {
    let Scalar::Object(object) = scalar else {
        unreachable!("an object operation gives an object")
    };
    object
}

/// `base[index]` for a tagged `index`, `base` evaluated: a small `int`'s
/// word is a native `int` key, anything else the key's object.
pub(crate) fn emit_subscript<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    slot: &StorageSlot<'ctx>,
) -> Scalar<'ctx> {
    let subscript = |key: Scalar<'ctx>| -> BasicValueEnum<'ctx> {
        expect_object(crate::foreign_call::emit_subscript(context, builder, module, rt, base, key, false)).into()
    };
    let value = split(
        context,
        builder,
        module,
        rt,
        slot,
        1,
        |word| subscript(Scalar::Int(word)),
        |object| subscript(Scalar::Object(object)),
    );
    Scalar::Object(value.into_pointer_value())
}

/// The truth of `local == text` (`!=` when `op` is `NotEq`), `text` the
/// one ASCII character `byte`, for a tagged `local`, as an `i1`: a
/// character's word is compared with the literal's; anything else is
/// compared as an object by `object_truth`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_eq_char<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
    op: CmpOpKind,
    byte: u8,
    object_truth: impl FnOnce(PointerValue<'ctx>) -> IntValue<'ctx>,
) -> IntValue<'ctx> {
    split(
        context,
        builder,
        module,
        rt,
        slot,
        CHAR_TAG,
        |word| {
            let literal = (u64::from(byte) << 3) | CHAR_TAG;
            let predicate = if op == CmpOpKind::Eq {
                inkwell::IntPredicate::EQ
            } else {
                inkwell::IntPredicate::NE
            };
            builder
                .build_int_compare(predicate, word, i64_const(context, literal), "tagged_char_equal")
                .expect("icmp")
                .into()
        },
        |object| object_truth(object).into(),
    )
    .into_int_value()
}

/// `local in container` (`not in` when `negate`) for a tagged `local` and
/// an evaluated container, as a `Scalar::Bool`: a character's word against
/// an exact `frozenset` of exact `str`s is a bit of the set's ASCII mask
/// ([`crate::fast_ops::Fast::ascii_byte_member`]); anything else is
/// `foreign_compare::emit_contains` of the local's object.
pub(crate) fn emit_contains<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    slot: &StorageSlot<'ctx>,
    negate: bool,
    container: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let contains_object = |object: PointerValue<'ctx>| -> BasicValueEnum<'ctx> {
        let Scalar::Bool(answer) =
            crate::foreign_compare::emit_contains(context, builder, module, rt, negate, Scalar::Object(object), container)
        else {
            unreachable!("a membership test gives a bool")
        };
        answer.into()
    };
    let Scalar::Object(set) = container else {
        return Scalar::Bool(contains_object(read_object(context, builder, module, rt, slot)).into_int_value());
    };
    let answer = split(
        context,
        builder,
        module,
        rt,
        slot,
        CHAR_TAG,
        |word| {
            crate::fast_ops::guarded(
                context,
                builder,
                module,
                "tagged_contains",
                |fast| {
                    let byte = builder
                        .build_right_shift(word, i64_const(context, 3), false, "tagged_contains_byte")
                        .expect("lshr");
                    let member = fast.ascii_byte_member(set, byte, "tagged_mask");
                    let member = builder
                        .build_int_truncate(member, context.i8_type(), "tagged_member")
                        .expect("trunc");
                    builder
                        .build_xor(member, context.i8_type().const_int(u64::from(negate), false), "tagged_answer")
                        .expect("xor")
                        .into()
                },
                || contains_object(read_object(context, builder, module, rt, slot)),
            )
        },
        contains_object,
    );
    Scalar::Bool(answer.into_int_value())
}
