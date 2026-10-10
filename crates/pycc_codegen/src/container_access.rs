//! Container receivers and name reads: list/dict/set/instance pointer checks,
//! dict runtime calls, loop holds and receiver release.

use super::*;

/// Extracts a `PyListObj` pointer from an already-evaluated operand that
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
pub(crate) fn expect_list_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
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
pub(crate) fn expect_instance_pointer<'ctx>(
    scalar: Scalar<'ctx>,
    what: &str,
) -> PointerValue<'ctx> {
    let Scalar::Instance(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: {what} did not evaluate to a class instance -- \
             pycc_types::check (T0043) should have rejected this before codegen"
        )
    };
    ptr
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
pub(crate) fn expect_dict_pointer<'ctx>(scalar: Scalar<'ctx>, what: &str) -> PointerValue<'ctx> {
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
/// key). Returns nothing, exactly like `list::build_list_append` above:
/// `pycc_rt_dict_set` is declared `void`.
pub(crate) fn build_dict_set<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    dict_ptr: PointerValue<'ctx>,
    key_word: IntValue<'ctx>,
    value_word: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.dict_set,
            &[dict_ptr.into(), key_word.into(), value_word.into()],
            "dict_set",
        )
        .expect("build_call should not fail for a well-formed dict set");
}

/// Returns the encoded value stored for `key_ptr`, or `encoded_default` if absent
/// (`dict.get(key, default)`, PR-12 Task 11, D-119). Mirrors
/// `list::build_list_pop`'s own one-`build_call` shape exactly, with one
/// extra `i64` operand for the default. Both the default and returned value
/// are D-141 int-compatible encoded words and retain bool identity.
pub(crate) fn build_dict_get_or_default<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    dict_ptr: PointerValue<'ctx>,
    key_word: IntValue<'ctx>,
    default_word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(
            rt.dict_get_or_default,
            &[dict_ptr.into(), key_word.into(), default_word.into()],
            "dict_get_or_default",
        )
        .expect("build_call should not fail for a well-formed dict get-or-default")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_get_or_default returns a non-void i64")
        .into_int_value()
}

/// A `PyDictObj`'s current entry count, as a raw `i64` counter, shared by
/// the `len(d)` builtin's `Ty::Dict` branch and
/// `MirStmt::ForDict`'s own loop bound -- mirrors `list::build_list_len`
/// exactly, for the identical reason.
pub(crate) fn build_dict_len<'ctx>(
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

/// Binds `target` to a freshly built container (a comprehension's result),
/// releasing the counted value the slot held before.
pub(crate) fn store_fresh_container<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    container: Scalar<'ctx>,
) {
    if !rt.borrowed_locals.borrow().contains(target)
        && let Some(slot) = locals.get(target)
    {
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
    }
    emit_assign(context, builder, rt, locals, target, container);
}

/// Every `for x in <name>` statement over a counted container in `body`, at
/// any depth: the statement, the name it iterates, its loop body and the
/// container's kind.
pub(crate) fn container_loops(
    body: &[MirStmt],
) -> Vec<(&MirStmt, &str, &[MirStmt], heap_rc::HeapKind)> {
    let mut out = Vec::new();
    for stmt in body {
        match stmt {
            MirStmt::ForList { list, body, .. } => {
                out.push((
                    stmt,
                    list.as_str(),
                    body.as_slice(),
                    heap_rc::HeapKind::List,
                ));
            }
            MirStmt::ForDict { dict, body, .. } => {
                out.push((
                    stmt,
                    dict.as_str(),
                    body.as_slice(),
                    heap_rc::HeapKind::Dict,
                ));
            }
            MirStmt::ForSet { set, body, .. } => {
                out.push((stmt, set.as_str(), body.as_slice(), heap_rc::HeapKind::Set));
            }
            _ => {}
        }
        for inner in stmt.bodies() {
            out.extend(container_loops(inner));
        }
    }
    out
}

/// A `for` loop's own reference to the container it iterates, taken when
/// the body could drop the container's other owner ([`list_may_be_rebound`]).
/// In a function it lives in a hidden owned frame slot (`loop_list_slots`),
/// so a `return` or an exception out of the loop releases it through the
/// owned-slot epilogue; the loop's normal exit releases it and clears the
/// slot.
pub(crate) struct LoopHold<'ctx> {
    pub(crate) held: Option<(
        heap_rc::HeapKind,
        PointerValue<'ctx>,
        Option<PointerValue<'ctx>>,
    )>,
}

impl<'ctx> LoopHold<'ctx> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enter(
        context: &'ctx Context,
        builder: &inkwell::builder::Builder<'ctx>,
        rt: &RtFns<'ctx>,
        locals: &HashMap<String, StorageSlot<'ctx>>,
        stmt: &MirStmt,
        body: &[MirStmt],
        name: &str,
        kind: heap_rc::HeapKind,
        ptr: PointerValue<'ctx>,
    ) -> Self {
        if !list_may_be_rebound(locals, body, name) {
            return Self { held: None };
        }
        heap_rc::emit_incref(context, builder, rt, kind, ptr);
        let slot = rt
            .loop_list_slots
            .borrow()
            .get(&(std::ptr::from_ref(stmt) as usize))
            .copied();
        if let Some(slot) = slot {
            // A hold left by an exception caught in this frame is released
            // before the new one replaces it.
            let old = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot,
                    "old_hold",
                )
                .expect("build_load should not fail for a loop hold slot")
                .into_pointer_value();
            heap_rc::emit_decref(context, builder, rt, kind, old);
            builder
                .build_store(slot, ptr)
                .expect("build_store should not fail for a loop hold slot");
        }
        Self {
            held: Some((kind, ptr, slot)),
        }
    }

    pub(crate) fn exit(
        self,
        context: &'ctx Context,
        builder: &inkwell::builder::Builder<'ctx>,
        rt: &RtFns<'ctx>,
    ) {
        let Some((kind, ptr, slot)) = self.held else {
            return;
        };
        heap_rc::emit_decref(context, builder, rt, kind, ptr);
        if let Some(slot) = slot {
            builder
                .build_store(
                    slot,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                )
                .expect("build_store should not fail for a loop hold slot");
        }
    }
}

/// Whether the list a `for` loop iterates, read from `name`, can lose its
/// last other reference while the loop runs: `name` is a module global
/// (which any call may rebind) or a local the loop body itself rebinds.
pub(crate) fn list_may_be_rebound(
    locals: &HashMap<String, StorageSlot<'_>>,
    body: &[MirStmt],
    name: &str,
) -> bool {
    let is_frame_slot = locals.get(name).is_some_and(|slot| !slot.ptr.is_const());
    if !is_frame_slot {
        return true;
    }
    let mut bound = BTreeMap::new();
    for stmt in body {
        collect_stmt_bindings(stmt, &mut bound);
    }
    bound.contains_key(name)
}

/// Resolves the receiver of `MirExpr::ListAppend`/`ListPop` to its list
/// pointer (#1263): a bare name through [`emit_list_name_read`], exactly as
/// before, and an attribute receiver (a slot read or a `@property` getter
/// call) through an ordinary `emit_expr`. Containers are leak-only (D-107,
/// D-124), so the pointer an attribute read yields needs no retain or
/// release around the mutation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_list_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    receiver: &pycc_mir::MirContainerReceiver,
    node: &MirExpr,
) -> (PointerValue<'ctx>, bool) {
    // The second value says whether the receiver holds a reference the
    // caller releases after the call: CPython keeps the bound method's
    // receiver alive while the arguments run, and an argument may rebind
    // the attribute or module global it was read from (`s.xs.append(
    // s.swap())`). A frame-local name is rebound by an argument only
    // through a walrus naming it inside `node`.
    match receiver {
        pycc_mir::MirContainerReceiver::Name(name) => {
            let ptr =
                emit_list_name_read(context, builder, module, rt, user_functions, locals, name);
            let global = locals.get(name).is_none_or(|slot| slot.ptr.is_const());
            let mut walrus_targets = Vec::new();
            node.collect_named_expr_bindings(&mut walrus_targets);
            let held = global || walrus_targets.iter().any(|(target, _)| target == name);
            if held {
                heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::List, ptr);
            }
            (ptr, held)
        }
        pycc_mir::MirContainerReceiver::Attr(read) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, read);
            let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, read, scalar);
            (
                expect_list_pointer(scalar, "an attribute container receiver"),
                true,
            )
        }
    }
}

/// Releases the reference [`emit_list_receiver`] took, if it took one.
pub(crate) fn release_list_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    (ptr, held): (PointerValue<'ctx>, bool),
) {
    if held {
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::List, ptr);
    }
}

/// The element type of the list a container receiver names.
pub(crate) fn value_list_element(
    receiver: &pycc_mir::MirContainerReceiver,
    locals: &HashMap<String, StorageSlot<'_>>,
) -> pycc_mir::Ty {
    let list_ty = match receiver {
        pycc_mir::MirContainerReceiver::Name(name) => locals[name].ty.clone(),
        pycc_mir::MirContainerReceiver::Attr(read) => read.ty(),
    };
    list::element_ty(&list_ty).clone()
}

/// The `dict` counterpart of [`emit_list_receiver`], for
/// `MirExpr::DictGetOrDefault`'s receiver (#1263).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_dict_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    receiver: &pycc_mir::MirContainerReceiver,
) -> (PointerValue<'ctx>, bool) {
    // As for a list receiver: an attribute (or module-global) receiver
    // holds its own reference while the arguments run, which the caller
    // releases with [`release_dict_receiver`].
    match receiver {
        pycc_mir::MirContainerReceiver::Name(name) => {
            let ptr =
                emit_dict_name_read(context, builder, module, rt, user_functions, locals, name);
            let global = locals.get(name).is_none_or(|slot| slot.ptr.is_const());
            if global {
                heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::Dict, ptr);
            }
            (ptr, global)
        }
        pycc_mir::MirContainerReceiver::Attr(read) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, read);
            let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, read, scalar);
            (
                expect_dict_pointer(scalar, "an attribute container receiver"),
                true,
            )
        }
    }
}

/// Releases the reference [`emit_dict_receiver`] took, if it took one.
pub(crate) fn release_dict_receiver<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    (ptr, held): (PointerValue<'ctx>, bool),
) {
    if held {
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::Dict, ptr);
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
pub(crate) fn emit_list_name_read<'ctx>(
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
pub(crate) fn emit_dict_name_read<'ctx>(
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
pub(crate) fn emit_set_name_read<'ctx>(
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
