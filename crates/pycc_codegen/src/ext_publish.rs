//! When an `ext` export becomes visible on the module object (#1199).
//!
//! CPython puts an extension module in `sys.modules` before its
//! `Py_mod_exec` slot runs (PEP 489), so a foreign module the body imports
//! can import this one back and read its attributes while the body is still
//! running. In CPython a module-level name is bound when its `def` or
//! `class` statement executes, and a read before that raises
//! `AttributeError: partially initialized module ...`. Before #1199 the
//! shim published every export from `PyModuleDef.m_methods` and every
//! method type before the body, so such a read found a wrapper whose
//! `fnptr_` slot was still null and crashed the host.
//!
//! This file computes, from the MIR alone, the function ordinal (the
//! position among the module's `MirItem::Function` items, as
//! `function_defs_in_order` and `copy_slots` count it) after whose slot
//! stores each export name is published, and emits the shim call
//! [`EXT_PUBLISH_SYMBOL`] there. The shim resolves the name against its
//! generated tables and ignores one it does not know, so this side may
//! name a superset of what the driver exports.
//!
//! * A module-level function `f` is published at the ordinal of its
//!   *last* definition only. Every definition of `f` shares one `fnptr_f`
//!   slot and one generated wrapper, and the driver resolves that
//!   wrapper's per-definition metadata (`param_writable`, carried
//!   defaults) last-wins (`src/ext_build/exports.rs`). Publishing an
//!   earlier definition would let a host call run that definition's body
//!   through the last one's wrapper -- a `memoryview` the earlier body
//!   writes accepted as a read-only buffer -- and a function object the
//!   host captured then would follow the slot to the later body. A
//!   redefined name is therefore absent from the module until its last
//!   definition runs: a residual over-hiding where CPython would show
//!   the earlier definition (`docs/RUNTIME.md`), never a call through
//!   mismatched metadata. Only names [`is_ext_exportable_name`] admits, never a
//!   dotted name, an inherited-method copy, or a PEP 562 hook
//!   (`__getattr__`/`__dir__`), which the shim adds after the body.
//! * A class `C` is published at the first ordinal at which every slot its
//!   type object's thunks can call is bound: the largest ordinal of a
//!   non-copy item owned by any class in `C`'s MRO. That is the plan's
//!   maximum of `C`'s own items, the origins its copies are bound with
//!   (`copy_slots`), and each in-program base's own publication ordinal,
//!   because a copy's origin belongs to a base and a base's publication
//!   ordinal is the same maximum over the base's MRO. A class with an own
//!   item (every class that declares or synthesizes `__init__`, or
//!   declares a method) is therefore published right after its own class
//!   statement, whose items `pycc_hir` appends contiguously at the
//!   statement's position. A class with none (`class E(Base): pass`) is
//!   published once its bases' methods are bound, possibly before its own
//!   statement: a residual over-visibility that can never call a null slot
//!   (`docs/RUNTIME.md`). A class with no ordinal at all (the seeded
//!   built-in exception classes) is skipped.
//!
//! A native build has no host and gets an empty plan.

use super::*;
use crate::copy_slots::CopySlots;
use inkwell::builder::Builder;

/// Which export names are published after which function ordinal.
#[derive(Debug, Default)]
pub(crate) struct PublishPlan {
    at: HashMap<usize, Vec<String>>,
}

impl PublishPlan {
    /// The plan for `mir`, or an empty one when `ext` is false.
    pub(crate) fn new(mir: &MirModule, copy_slots: &CopySlots, ext: bool) -> Self {
        let mut plan = PublishPlan::default();
        if !ext {
            return plan;
        }
        let functions: Vec<(usize, &str)> = mir
            .items
            .iter()
            .filter_map(|item| match item {
                MirItem::Function { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .enumerate()
            .filter(|(ordinal, _)| !copy_slots.is_copy(*ordinal))
            .collect();
        // Only the last definition of a name is published: see the module
        // docs for why an earlier one must stay hidden.
        let mut last: HashMap<&str, usize> = HashMap::new();
        for &(ordinal, name) in &functions {
            if !name.contains('.')
                && is_ext_exportable_name(name)
                && name != "__getattr__"
                && name != "__dir__"
            {
                last.insert(name, ordinal);
            }
        }
        for &(ordinal, name) in &functions {
            if last.get(name) == Some(&ordinal) {
                plan.at.entry(ordinal).or_default().push(name.to_string());
            }
        }
        for (class, def) in &mir.class_defs {
            let bound = functions
                .iter()
                .filter(|(_, name)| {
                    name.split_once('.')
                        .is_some_and(|(owner, _)| def.mro.iter().any(|entry| entry == owner))
                })
                .map(|&(ordinal, _)| ordinal)
                .max();
            if let Some(ordinal) = bound {
                plan.at.entry(ordinal).or_default().push(class.clone());
            }
        }
        plan
    }

    /// The names published right after the function at `ordinal` binds its
    /// slots, in emission order.
    pub(crate) fn names_at(&self, ordinal: usize) -> &[String] {
        self.at.get(&ordinal).map_or(&[], Vec::as_slice)
    }
}

/// Declares the shim's `int pycc_ext_publish(const char *)` once per
/// module, returning the existing declaration on every later call.
fn publish_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_PUBLISH_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_PUBLISH_SYMBOL,
        context.i32_type().fn_type(&[ptr.into()], false),
        None,
    )
}

/// Emits one [`EXT_PUBLISH_SYMBOL`] call per name, at the builder's
/// position in the module entry point `entry_fn`. A negative status means
/// CPython raised (an allocation failure, or no executing module), so the
/// entry point returns [`EXT_MODULE_EXEC_FAILED`] with that exception set,
/// as a failed top-level import does; no handler can enclose a top-level
/// definition.
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    names: &[String],
) {
    for name in names {
        let text = builder
            .build_global_string_ptr(name, &format!("pycc_publish_{name}"))
            .expect("build_global_string_ptr should not fail")
            .as_pointer_value();
        let status = builder
            .build_call(publish_fn(context, module), &[text.into()], "publish")
            .expect("build_call should not fail for pycc_ext_publish")
            .try_as_basic_value()
            .expect_basic("pycc_ext_publish returns int")
            .into_int_value();
        let failed = builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                status,
                context.i32_type().const_zero(),
                "publish_failed",
            )
            .expect("build_int_compare should not fail");
        let fail_bb = context.append_basic_block(entry_fn, "publish_fail");
        let cont_bb = context.append_basic_block(entry_fn, "publish_cont");
        builder
            .build_conditional_branch(failed, fail_bb, cont_bb)
            .expect("build_conditional_branch should not fail");
        builder.position_at_end(fail_bb);
        builder
            .build_return(Some(
                &context
                    .i64_type()
                    .const_int(EXT_MODULE_EXEC_FAILED as u64, true),
            ))
            .expect("build_return should not fail");
        builder.position_at_end(cont_bb);
    }
}

#[cfg(test)]
#[path = "ext_publish_tests.rs"]
mod tests;
