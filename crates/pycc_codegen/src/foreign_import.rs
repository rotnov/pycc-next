//! Emission for `MirItem::ForeignImport` (Part 1 of #1026, PR 1c of #1080)
//! and for `MirStmt::ForeignImport`, a foreign import (#1291) or
//! from-import (#1383) nested in a module-level `if`/`try` block.
//!
//! Cohesion-driven carve out of `lib.rs` under AGENTS.md's decomposability
//! rule: everything that knows how a foreign `import numpy` becomes machine
//! code lives here -- the module global its local name gets, the extern
//! declaration of the shim's import helpers, and the call emitted at the
//! import's own source position. Since #1278 that includes `from numpy
//! import pi`, which binds the module's attribute through
//! `pycc_ext_obj_import_from` instead of the module through
//! `pycc_ext_obj_import`, and the `MirItem::ForeignImport` arm of
//! `compile_to_object`'s item loop, which is only a call to [`emit_item`].
//!
//! **Ordering.** The call is emitted from `compile_to_object`'s single
//! source-order `for item in &mir.items` loop, at the position
//! `pycc_mir::build` spliced the item into. It is deliberately *not* a
//! hoisted prologue: CPython runs a module body statement by statement, so a
//! `print(...)` written above an `import` runs first, and D-244 rule 3 binds
//! the artifact to CPython's observable behaviour. `MirItem::ForeignImport`
//! exists precisely so that ordering is structural rather than a convention
//! this file would have to re-derive. A nested import is a statement, so
//! `emit_stmt` emits it where its block runs. Its failure edge first asks
//! the shim to bridge the failure (#1293): an `ImportError` becomes a
//! pending pycc exception and control branches to the innermost exception
//! target, so an enclosing `except`/`finally` runs as in CPython. Any other
//! exception takes the edge every module-body foreign failure takes (Part 1
//! of #1096, `foreign_fail.rs`): bridged by the object bridge when a
//! module-level `try` encloses the import, and otherwise the direct return
//! from the entry point. A top-level import has nothing to enclose it and
//! keeps the direct return.
//!
//! **Ownership** (`docs/RUNTIME.md`). The import yields a new reference,
//! stored during `pycc_ext_module_exec` into a module-level global that
//! owns it (Part 1 of #1499, `object_slot.rs`): importing the same name a
//! second time, or rebinding it, releases the previous value. The last
//! value bound is never released: a CPython module object lives in
//! `sys.modules` for the interpreter's lifetime anyway, and the generated
//! artifact has no teardown hook to release it from. Contrast the `Ty::Str` exit-time decref
//! loop in `compile_to_object`, which is `!options.ext`-guarded for the
//! same reason -- there is no "program exit" in a hosted extension module.
//!
//! Part 2 of #1026 kept that rule but removed its old justification: a
//! `Py_DECREF` path used to be *unreachable* because no operation on the
//! bound name was implemented at all, and now `foreign_attr.rs` implements
//! one. The leak here is still once per process; the one an attribute load
//! introduces is trip-count-linear, and `docs/RUNTIME.md` carries the
//! consequence for D-244 rule 6 benchmarking.
//!
//! **Native mode.** A foreign import reaches codegen only with `ext` set:
//! either `--ext`, or an embedded build (D-248), which compiles its module
//! with `ext` set. Every other build has no interpreter to import into, and
//! the driver refuses it with `I0403` after typed HIR. This file
//! therefore *ignores* a `ForeignImport` item when `options.ext` is false
//! rather than asserting -- an assertion here would be an unreachable line
//! under the 100%-changed-line coverage gate, and the driver gate is where
//! the refusal is actually tested.

use super::*;
use crate::ext::EXT_IMPORT_ERROR_BRIDGE_SYMBOL;
use crate::ext::EXT_OBJ_IMPORT_DOTTED_SYMBOL;
use crate::ext::EXT_OBJ_IMPORT_FROM_SYMBOL;
use inkwell::builder::Builder;
use pycc_mir::FromImport;

/// What a failed import's `NULL` edge does.
pub(super) enum FailureEdge<'a, 'ctx> {
    /// Return [`EXT_MODULE_EXEC_FAILED`] from the entry point with CPython's
    /// exception set: a top-level [`MirItem::ForeignImport`], which no
    /// handler can enclose.
    ReturnFailed,
    /// Bridge an `ImportError` into a pending pycc exception and branch to
    /// the innermost exception target, falling back to the module-exec
    /// foreign failure edge (Part 1 of #1096) for anything the import
    /// bridge does not map: a nested `MirStmt::ForeignImport` (#1293).
    Bridge { rt: &'a RtFns<'ctx> },
}

/// Declares the shim's `int pycc_ext_import_error_bridge(void)` once per
/// module, returning the existing declaration on every later call.
fn import_error_bridge_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_IMPORT_ERROR_BRIDGE_SYMBOL) {
        return existing;
    }
    module.add_function(
        EXT_IMPORT_ERROR_BRIDGE_SYMBOL,
        context.i32_type().fn_type(&[], false),
        None,
    )
}

/// Emits `ret i64 EXT_MODULE_EXEC_FAILED` at the builder's position.
fn return_failed(context: &Context, builder: &Builder<'_>) {
    builder
        .build_return(Some(
            &context
                .i64_type()
                .const_int(EXT_MODULE_EXEC_FAILED as u64, true),
        ))
        .expect("build_return should not fail");
}

/// Declares the shim's `PyObject *pycc_ext_obj_import(const char *)` once
/// per module, returning the existing declaration on every later call.
fn obj_import_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_IMPORT_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_IMPORT_SYMBOL,
        ptr.fn_type(&[ptr.into()], false),
        None,
    )
}

/// One foreign import binding as codegen emits it: the module global it
/// stores into (`local_name`), the module, and -- for one name of a
/// `from X import a, b` (#1278) -- that name's place in the statement's
/// fromlist.
#[derive(Clone, Copy)]
pub(super) struct ForeignBinding<'a> {
    pub(super) local_name: &'a str,
    pub(super) module_path: &'a str,
    pub(super) from: Option<&'a FromImport>,
}

/// Emits a top-level [`MirItem::ForeignImport`] at its own position in
/// `compile_to_object`'s source-order item loop, or nothing at all in a
/// build without `ext` (see the module doc's **Native mode**). `def_iter`
/// in that loop is deliberately not advanced: it pairs with
/// `MirItem::Function` items only.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_item<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    ext: bool,
    module_globals: &BTreeMap<String, StorageSlot<'ctx>>,
    binding: ForeignBinding<'_>,
) {
    if ext {
        emit(
            context,
            builder,
            module,
            rt,
            entry_fn,
            &module_globals[binding.local_name],
            binding,
            FailureEdge::ReturnFailed,
        );
    }
}

/// Declares the shim's `PyObject *pycc_ext_obj_import_dotted(const char *,
/// long long)` once per module (#1381), returning the existing declaration
/// on every later call.
fn obj_import_dotted_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_IMPORT_DOTTED_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_IMPORT_DOTTED_SYMBOL,
        ptr.fn_type(&[ptr.into(), context.i64_type().into()], false),
        None,
    )
}

/// Declares the shim's `PyObject *pycc_ext_obj_import_from(const char *,
/// const char *const *, long long, long long, long long)` once per module
/// (#1278; the fifth parameter, the relative `level`, is #1366's).
fn obj_import_from_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_IMPORT_FROM_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64t = context.i64_type();
    module.add_function(
        EXT_OBJ_IMPORT_FROM_SYMBOL,
        ptr.fn_type(
            &[
                ptr.into(),
                ptr.into(),
                i64t.into(),
                i64t.into(),
                i64t.into(),
            ],
            false,
        ),
        None,
    )
}

/// Emits the `pycc_ext_obj_import_from` call for one name of a
/// `from X import a, b` (#1278) and returns the new reference it yields.
///
/// The whole fromlist is passed on every name's call: CPython's
/// `IMPORT_NAME` receives all of it before its first `IMPORT_FROM`, so a
/// package submodule listed later in the statement is already imported
/// when an earlier name is fetched. Each name is a private string global
/// (`pycc_foreign_attr_{local}`), and the list is a private constant array
/// of pointers to them (`pycc_foreign_fromlist_{local}`).
///
/// The last argument is [`FromImport::level`] (#1366): `0` for an absolute
/// import, and the dot count of a relative one, which the shim resolves
/// against the executing module's own package, as CPython's `IMPORT_NAME`
/// does with the frame's globals.
fn emit_import_from_call<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    local_name: &str,
    module_name: PointerValue<'ctx>,
    from: &FromImport,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let names: Vec<PointerValue<'ctx>> = from
        .fromlist
        .iter()
        .map(|name| {
            builder
                .build_global_string_ptr(name, &format!("pycc_foreign_attr_{local_name}"))
                .expect("build_global_string_ptr should not fail")
                .as_pointer_value()
        })
        .collect();
    let array_ty = ptr.array_type(names.len() as u32);
    let fromlist = module.add_global(
        array_ty,
        None,
        &format!("pycc_foreign_fromlist_{local_name}"),
    );
    fromlist.set_initializer(&ptr.const_array(&names));
    fromlist.set_constant(true);
    fromlist.set_linkage(Linkage::Private);
    let i64t = context.i64_type();
    builder
        .build_call(
            obj_import_from_fn(context, module),
            &[
                module_name.into(),
                fromlist.as_pointer_value().into(),
                i64t.const_int(from.fromlist.len() as u64, false).into(),
                i64t.const_int(from.index as u64, false).into(),
                i64t.const_int(u64::from(from.level), false).into(),
            ],
            "foreign_import",
        )
        .expect("build_call should not fail for pycc_ext_obj_import_from")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_import_from returns PyObject *")
        .into_pointer_value()
}

/// Emits the import call for one foreign import binding (a
/// [`MirItem::ForeignImport`], or one entry of a `MirStmt::ForeignImport`)
/// and stores the resulting object into `slot`, the binding's module
/// global. `pycc_ext_obj_import(module_path)` binds the module for `import
/// X`; `pycc_ext_obj_import_dotted` binds the root of `import X.Y` or the
/// leaf of `import X.Y as Z` (#1381); `pycc_ext_obj_import_from` binds the
/// named attribute for `from X import n` (#1278). The local name also names the module-path string
/// global; two imports binding the same name get two strings, which LLVM
/// names apart by emission order.
///
/// A `NULL` return means CPython raised (`ModuleNotFoundError`, or
/// `ImportError` for a missing name of a from-import), and `edge` decides
/// what happens next. Under
/// [`FailureEdge::ReturnFailed`] the exception is already set by the shim,
/// so the entry point returns [`EXT_MODULE_EXEC_FAILED`] immediately -- the
/// `Py_mod_exec` slot's failure convention -- and the module body's
/// remaining statements never run. Under [`FailureEdge::Bridge`] the shim's
/// `pycc_ext_import_error_bridge` is asked first: a non-zero answer means an
/// `ImportError` is now a pending pycc exception, and control branches to
/// the innermost exception target exactly as an explicit `raise` does
/// (`emit_body`); a zero answer reaches a block named
/// `foreign_import_unbridged`, which takes `foreign_fail::emit_failure`'s
/// module-exec edge: the object bridge and the same branch when a
/// module-level `try` encloses the import, and the direct return when none
/// does (Part 1 of #1096). Both edges serve both forms: a
/// top-level from-import (#1278) takes [`FailureEdge::ReturnFailed`], and
/// one nested in a module-level block (#1383) takes
/// [`FailureEdge::Bridge`], whose shim maps CPython's "cannot import name"
/// `ImportError` like any other.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    slot: &StorageSlot<'ctx>,
    binding: ForeignBinding<'_>,
    edge: FailureEdge<'_, 'ctx>,
) {
    let ForeignBinding {
        local_name,
        module_path,
        from,
    } = binding;
    let name = builder
        .build_global_string_ptr(module_path, &format!("pycc_foreign_module_{local_name}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let imported = match from {
        None if module_path.contains('.') => {
            // #1381: `import a.b` binds the root, `import a.b as c` the leaf.
            let bind_root = pycc_mir::foreign_binds_root(local_name, module_path, None);
            builder
                .build_call(
                    obj_import_dotted_fn(context, module),
                    &[
                        name.into(),
                        context
                            .i64_type()
                            .const_int(u64::from(bind_root), false)
                            .into(),
                    ],
                    "foreign_import",
                )
                .expect("build_call should not fail for pycc_ext_obj_import_dotted")
                .try_as_basic_value()
                .expect_basic("pycc_ext_obj_import_dotted returns PyObject *")
                .into_pointer_value()
        }
        None => builder
            .build_call(
                obj_import_fn(context, module),
                &[name.into()],
                "foreign_import",
            )
            .expect("build_call should not fail for pycc_ext_obj_import")
            .try_as_basic_value()
            .expect_basic("pycc_ext_obj_import returns PyObject *")
            .into_pointer_value(),
        Some(from) => emit_import_from_call(context, builder, module, local_name, name, from),
    };
    let failed = builder
        .build_is_null(imported, "foreign_import_failed")
        .expect("build_is_null should not fail");
    let fail_bb = context.append_basic_block(entry_fn, "foreign_import_fail");
    let cont_bb = context.append_basic_block(entry_fn, "foreign_import_cont");
    builder
        .build_conditional_branch(failed, fail_bb, cont_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(fail_bb);
    match edge {
        FailureEdge::ReturnFailed => return_failed(context, builder),
        FailureEdge::Bridge { rt } => {
            let bridged = builder
                .build_call(
                    import_error_bridge_fn(context, module),
                    &[],
                    "import_bridged",
                )
                .expect("build_call should not fail for pycc_ext_import_error_bridge")
                .try_as_basic_value()
                .expect_basic("pycc_ext_import_error_bridge returns int")
                .into_int_value();
            let raised = builder
                .build_int_compare(
                    IntPredicate::NE,
                    bridged,
                    context.i32_type().const_zero(),
                    "import_error_raised",
                )
                .expect("build_int_compare should not fail");
            // A bare branch, with no `guard_statement_effects` unwind: a
            // `ForeignImport` is a statement, emitted at a statement
            // boundary, where `rt.exceptions.pending_int_releases` is empty
            // by that field's own invariant, so nothing can be orphaned.
            let target = *rt
                .exceptions
                .targets
                .borrow()
                .last()
                .expect("a module body statement is always inside an exception target");
            let unbridged_bb = context.append_basic_block(entry_fn, "foreign_import_unbridged");
            builder
                .build_conditional_branch(raised, target, unbridged_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(unbridged_bb);
            // Anything the import bridge does not map -- the imported
            // module's own `ValueError`, say -- takes the edge every other
            // module-body foreign failure takes (Part 1 of #1096): bridged
            // by the total object bridge to an enclosing module-level
            // `try`, or the direct return when none encloses the import.
            crate::foreign_fail::emit_failure(
                context,
                builder,
                module,
                rt,
                crate::foreign_fail::ForeignFailEdge::ModuleExec(entry_fn),
            );
        }
    }
    builder.position_at_end(cont_bb);
    // `imported` is a new reference (`PyImport_ImportModule`,
    // `PyImport_GetModule`, `PyObject_GetOptionalAttr`). Part 1 of #1499:
    // the module global owns it, so importing the same name twice -- or
    // rebinding it later -- releases the previous value, and the store
    // raises the definite-assignment flag exactly as an ordinary
    // top-level `Assign` does (`object_slot::store_new_reference`).
    crate::object_slot::store_new_reference(context, builder, module, rt, slot, imported);
}

/// Emits a [`pycc_mir::MirStmt::ForeignImport`]: one import per binding,
/// in order, into the module-exec entry point `builder` is emitting into.
/// No `options.ext` guard is needed: a foreign binding reaches codegen only
/// in an `ext` build, because the driver refuses every other build with
/// `I0403`, and `expect_module_exec_entry` pins the entry point. Each
/// binding's failure edge is [`FailureEdge::Bridge`] (#1293), so a failed
/// binding stops the remaining ones exactly as a raise would, leaving the
/// earlier ones bound as in CPython. A binding with a `from` is one name
/// of a nested `from X import a, b` (#1383).
pub(super) fn emit_stmt<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    bindings: &[(String, String, Option<FromImport>)],
) {
    let entry_fn = crate::foreign_attr::expect_module_exec_entry(builder);
    for (local_name, module_path, from) in bindings {
        emit(
            context,
            builder,
            module,
            rt,
            entry_fn,
            &locals[local_name],
            ForeignBinding {
                local_name,
                module_path,
                from: from.as_ref(),
            },
            FailureEdge::Bridge { rt },
        );
    }
}

#[cfg(test)]
mod tests;
