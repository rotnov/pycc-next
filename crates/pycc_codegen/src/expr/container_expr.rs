//! Native container expressions: literals, subscripts, slices and methods.

use super::*;
use pycc_mir::Ty;

pub(crate) fn emit_list_literal_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ListLiteral(elements) = expr else {
        unreachable!()
    };
    let elem = expr.ty();
    let elem = list::element_ty(&elem);
    let list_ptr = list::build_list_new(context, builder, rt, elem, elements.len() as u64);
    // The list under construction is released if an element raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_owned(rt, heap_rc::HeapKind::List, list_ptr);
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
        let word = list::element_word(context, builder, rt, elem, element, scalar);
        list::build_list_append(builder, rt, list_ptr, word);
    }
    heap_rc::truncate_pending(rt, mark);
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
pub(crate) fn emit_subscript_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Subscript { base, index } = expr else {
        unreachable!()
    };
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
        // A boxed tuple: the literal index's word, read inline and
        // borrowed like a list element (`containers.rs`).
        (pycc_mir::Ty::Tuple(elements), Scalar::List(tuple_ptr))
            if pycc_hir::tuple_is_boxed(&elements) =>
        {
            let MirExpr::IntLiteral(literal_index) = index.as_ref() else {
                panic!(
                    "pycc_codegen: internal error: a tuple subscript index is not a \
                     literal int"
                )
            };
            let element = containers::tuple_element(
                context,
                builder,
                &elements,
                Scalar::List(tuple_ptr),
                *literal_index as u64,
            );
            if !heap_rc::is_borrowed_read(base) {
                heap_rc::retain_scalar(context, builder, rt, &expr.ty(), &element);
                release_scalar_if_temporary(context, builder, rt, base, &Scalar::List(tuple_ptr));
            }
            element
        }
        // D-115: the field is read straight off the loaded
        // `StructValue` with `extractvalue` -- no pointer, no
        // `build_struct_gep`, because the aggregate is an SSA
        // register rather than something in memory.
        (pycc_mir::Ty::Tuple(elements), Scalar::Tuple(struct_value)) => {
            let MirExpr::IntLiteral(literal_index) = index.as_ref() else {
                // A computed index into a tuple of one scalar element
                // type (`pycc_types` admits no other).
                let index_scalar =
                    emit_expr(context, builder, module, rt, user_functions, locals, index);
                let raw_index = list::raw_index(context, builder, rt, index_scalar);
                return crate::tuple_index::emit_dynamic_tuple_element(
                    context,
                    builder,
                    rt,
                    &elements,
                    struct_value,
                    raw_index,
                );
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
        // `base[index]` on a list: the index is decoded to a raw
        // position, a negative one counts from the end, and an
        // out-of-range one raises `IndexError` from the inlined
        // bounds check (`list::build_list_get`) -- the opposite of
        // the tuple arm above, whose literal index `pycc_types` has
        // already bounds-checked.
        (_, base_scalar) => {
            let base_ptr = expect_list_pointer(base_scalar, "the subscripted value");
            // A fresh base is released if the index or the bounds
            // check raises.
            let mark = heap_rc::pending_mark(rt);
            heap_rc::hold_temporary(rt, base, &Scalar::List(base_ptr));
            let index_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, index);
            let raw_index = list::raw_index(context, builder, rt, index_scalar);
            let word = list::build_list_get(context, builder, rt, &expr.ty(), base_ptr, raw_index);
            heap_rc::truncate_pending(rt, mark);
            let element = list::word_element(context, builder, &expr.ty(), word);
            // As for `AttrGet`: an element of a fresh list is
            // retained before the list is released.
            if !heap_rc::is_borrowed_read(base) {
                heap_rc::retain_scalar(context, builder, rt, &expr.ty(), &element);
                release_scalar_if_temporary(context, builder, rt, base, &Scalar::List(base_ptr));
            }
            element
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
pub(crate) fn emit_list_append_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ListAppend { list, value } = expr else {
        unreachable!()
    };
    let receiver = emit_list_receiver(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        list,
        expr,
    );
    let list_ptr = receiver.0;
    let elem = value_list_element(list, locals);
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let word = list::element_word(context, builder, rt, &elem, value, scalar);
    list::build_list_append(builder, rt, list_ptr, word);
    release_list_receiver(context, builder, rt, receiver);
    Scalar::Bool(context.i8_type().const_int(0, false))
}

pub(crate) fn emit_list_insert_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ListInsert { list, index, value } = expr else {
        unreachable!()
    };
    let receiver = emit_list_receiver(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        list,
        expr,
    );
    let list_ptr = receiver.0;
    let elem = value_list_element(list, locals);
    let index_scalar = emit_expr(context, builder, module, rt, user_functions, locals, index);
    let raw = list::raw_index(context, builder, rt, index_scalar);
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let word = list::element_word(context, builder, rt, &elem, value, scalar);
    list::build_list_insert(builder, rt, list_ptr, raw, word);
    release_list_receiver(context, builder, rt, receiver);
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
pub(crate) fn emit_dict_literal_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::DictLiteral(pairs) = expr else {
        unreachable!()
    };
    let Ty::Dict(kv) = expr.ty() else {
        panic!("pycc_codegen: internal error: a dict display without a dict type")
    };
    let dict_ptr = containers::build_dict_new(context, builder, rt, &kv.0, &kv.1);
    // The dict under construction is released if a pair raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_owned(rt, heap_rc::HeapKind::Dict, dict_ptr);
    for (key, value) in pairs {
        // CPython evaluates each key before its value.
        let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
        let key_word = list::element_word(context, builder, rt, &kv.0, key, key_scalar);
        let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
        let value_word = list::element_word(context, builder, rt, &kv.1, value, value_scalar);
        build_dict_set(builder, rt, dict_ptr, key_word, value_word);
    }
    heap_rc::truncate_pending(rt, mark);
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
pub(crate) fn emit_dict_get_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::DictGet { dict, key } = expr else {
        unreachable!()
    };
    let dict_ty = dict.ty();
    let dict_scalar = emit_expr(context, builder, module, rt, user_functions, locals, dict);
    let dict_ptr = expect_dict_pointer(dict_scalar, "the dict subscripted value");
    // A fresh dict is released if the key raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_temporary(rt, dict, &dict_scalar);
    let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
    heap_rc::truncate_pending(rt, mark);
    let (key_ty, value_ty) = containers::dict_kv(&dict_ty);
    let key_coerced = coerce_scalar_to_type(context, builder, key_scalar, key_ty.clone());
    let (key_word, boxed) = containers::borrowed_word(context, builder, rt, &key_ty, key_coerced);
    let value_word = builder
        .build_call(rt.dict_get, &[dict_ptr.into(), key_word.into()], "dict_get")
        .expect("build_call should not fail for a well-formed dict read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_get returns a non-void i64")
        .into_int_value();
    containers::release_boxed(context, builder, rt, boxed);
    // The lookup borrows the key; the value is a new reference.
    release_scalar_if_temporary(context, builder, rt, key, &key_scalar);
    release_scalar_if_temporary(context, builder, rt, dict, &Scalar::Dict(dict_ptr));
    containers::owned_word_element(context, builder, rt, &value_ty, value_word)
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
pub(crate) fn emit_set_literal_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::SetLiteral {
        elements,
        ops: Some(ops),
    } = expr
    else {
        unreachable!()
    };
    let emitter = SetEmitter {
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
    };
    Scalar::Set(emitter.literal(&expr.ty(), elements, ops))
}

pub(crate) fn emit_set_literal_expr_2<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::SetLiteral {
        elements,
        ops: None,
    } = expr
    else {
        unreachable!()
    };
    let Ty::Set(elem) = expr.ty() else {
        panic!("pycc_codegen: internal error: a set display without a set type")
    };
    let set_ptr = set_instance::build_set_new(builder, rt, &expr.ty());
    // The set under construction is released if an element raises.
    let mark = heap_rc::pending_mark(rt);
    heap_rc::hold_owned(rt, heap_rc::HeapKind::Set, set_ptr);
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
        let word = list::element_word(context, builder, rt, &elem, element, scalar);
        containers::build_set_add(builder, rt, set_ptr, word);
    }
    heap_rc::truncate_pending(rt, mark);
    Scalar::Set(set_ptr)
}

pub(crate) fn emit_tuple_literal_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::TupleLiteral(elements) = expr else {
        unreachable!()
    };
    let elem_tys: Vec<pycc_mir::Ty> = elements.iter().map(MirExpr::ty).collect();
    let struct_ty =
        ty_to_basic_type(context, pycc_mir::Ty::Tuple(Box::new(elem_tys))).into_struct_type();
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
// to a **freshly allocated** result object `pycc_rt_list_slice`
// returns, never `base_ptr` itself -- before it ever calls
// `emit_assign` to rebind `target`'s own slot. This arm never
// writes to `locals` at all, so there is no premature-rebind window
// here the way an earlier `ListCompAssign` draft had for
// comprehensions. The original list is left untouched either way
// (D-107's leak-only policy: no incref/decref of `base_ptr`, and no
// special handling for the new result beyond what
// `pycc_rt_list_slice` itself already does).
pub(crate) fn emit_slice_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Slice {
        base,
        start,
        stop,
        step,
    } = expr
    else {
        unreachable!()
    };
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let base_ptr = expect_list_pointer(base_scalar, "the sliced value");

    let bound = |e: &Option<Box<MirExpr>>| {
        e.as_ref().map(|e| {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
            list::raw_index(context, builder, rt, scalar)
        })
    };
    let bounds = [bound(start), bound(stop), bound(step)];
    Scalar::List(list::build_list_slice(
        context, builder, rt, base_ptr, bounds,
    ))
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
// regression. Each `.pop()` mutates the *pointee* `PyListObj`
// in place (via `pycc_rt_list_pop`'s own `Cell<Vec<i64>>`), not
// `xs`'s own slot, so two `.pop()`s emitted in source order run in
// that same order at runtime -- ordinary sequential `call`
// instructions, nothing deferred -- and each observes the
// previous one's mutation, matching CPython's own left-to-right
// evaluation of `[xs.pop(), xs.pop()]`. Verified empirically, not
// just by this reasoning: see the crate's `tests` module and its
// `pop_twice_on_the_same_list_in_one_statement_removes_in_order`
// end-to-end test.
pub(crate) fn emit_list_pop_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::ListPop { list, index, ty } = expr else {
        unreachable!()
    };
    let receiver = emit_list_receiver(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        list,
        expr,
    );
    let list_ptr = receiver.0;
    let index = index.as_ref().map(|index| {
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, index);
        list::raw_index(context, builder, rt, scalar)
    });
    let word = list::build_list_pop(context, builder, rt, list_ptr, index);
    release_list_receiver(context, builder, rt, receiver);
    containers::owned_word_element(context, builder, rt, ty, word)
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
pub(crate) fn emit_dict_get_or_default_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::DictGetOrDefault {
        dict, key, default, ..
    } = expr
    else {
        unreachable!()
    };
    let dict_ty = match dict {
        pycc_mir::MirContainerReceiver::Name(name) => locals[name].ty.clone(),
        pycc_mir::MirContainerReceiver::Attr(read) => read.ty(),
    };
    let (key_ty, value_ty) = containers::dict_kv(&dict_ty);
    let receiver = emit_dict_receiver(context, builder, module, rt, user_functions, locals, dict);
    let dict_ptr = receiver.0;
    let key_scalar = emit_expr(context, builder, module, rt, user_functions, locals, key);
    let key_coerced = coerce_scalar_to_type(context, builder, key_scalar, key_ty.clone());
    let (key_word, boxed) = containers::borrowed_word(context, builder, rt, &key_ty, key_coerced);
    let default_scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        default,
    );
    let default_scalar = coerce_scalar_to_type(context, builder, default_scalar, value_ty.clone());
    // The runtime adopts the default's reference.
    let default_word = list::element_word(context, builder, rt, &value_ty, default, default_scalar);
    let value_word = build_dict_get_or_default(builder, rt, dict_ptr, key_word, default_word);
    containers::release_boxed(context, builder, rt, boxed);
    release_scalar_if_temporary(context, builder, rt, key, &key_scalar);
    release_dict_receiver(context, builder, rt, receiver);
    containers::owned_word_element(context, builder, rt, &value_ty, value_word)
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
pub(crate) fn emit_set_add_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::SetAdd { set, value, ops } = expr else {
        unreachable!()
    };
    let set_ptr = emit_set_name_read(context, builder, module, rt, user_functions, locals, set);
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
    let Ty::Set(elem) = locals[set].ty.clone() else {
        panic!("pycc_codegen: internal error: `add` on a non-set")
    };
    let value_scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let value_scalar = coerce_scalar_to_type(context, builder, value_scalar, (*elem).clone());
    let word = list::element_word(context, builder, rt, &elem, value, value_scalar);
    containers::build_set_add(builder, rt, set_ptr, word);
    Scalar::Bool(context.i8_type().const_int(0, false))
}

// #1165: `ndarray(n)` / `NDArray(n)`. One call and no arithmetic,
// exactly as the two arms above -- `pycc_rt` owns the zero-fill, the
// negative-length refusal and the allocator pairing.
//
// `Ty::Int` (or `bool`, per D-086) arrives D-141 encoded, so the
// length is decoded before the allocator sees it -- but *not* the
// way a buffer index is: this arm decodes through
// `build_buffer_alloc_untag_len`, whose runtime function raises
// `OverflowError` (D-173) for a bigint or a negative length and
// returns the type-valid sentinel `0`. The
// D-173 raise a negative length leaves pending is declared by
// `expression_can_set_exception`, which emits the guard that reads
// it; the null view the refusal returns is safe for the epilogue,
// whose free is a documented no-op on null. A second guard is emitted
// *before* the allocator call, for an exception that was already
// pending when this arm was reached -- see it in place below.
pub(crate) fn emit_frozen_set_from_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::FrozenSetFrom { source } = expr else {
        unreachable!()
    };
    let Ty::FrozenSet(elem) = expr.ty() else {
        panic!("pycc_codegen: internal error: frozenset() without a frozenset type")
    };
    frozenset::emit_frozenset_from(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        source.as_deref(),
        &elem,
    )
}
