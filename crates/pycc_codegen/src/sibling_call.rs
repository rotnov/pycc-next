//! Native calls between the modules of a package built whole (v0.8).
//!
//! A module of the package that imports a compiled function of another
//! (`from .util import escape`) calls it through a function of its own the
//! driver adds, `_pycc_sib_escape(a, b)`, whose body is the call of the
//! imported object as CPython makes it (`__pycc_fallback_ns['escape'](a,
//! b)`: the import runs in CPython and binds the name in the module dict)
//! and whose signature is the other module's compiled one
//! (`crate::package_build`'s interface in the driver). Each call of it
//! first reads the name from the module dict, as CPython reads the callee
//! before the arguments, and compares it with the object the other module
//! published for the function (its `pycc_ext_interface_table` entry): while
//! they are the same object, the call goes to the other module's compiled
//! function directly, with the arguments as they are; otherwise -- the name
//! was rebound from Python, or the other module's body has not finished --
//! to the body that calls the object. Both have the same signature, so the
//! arguments are marshalled once, for whichever runs.

use super::*;
use crate::user_call::Callee;

/// One function a module calls natively in another module of its package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingTarget {
    /// The calling module's own function (`_pycc_sib_<global>`).
    pub function: String,
    /// The module global the call reads first (the imported name).
    pub global: String,
    /// The other module's compiled function (`<prefix>pyfn_<name>`).
    pub producer_symbol: String,
    /// The other module's table of published objects
    /// (`<prefix>pycc_ext_interface_table`).
    pub table_symbol: String,
    /// The function's entry in that table.
    pub index: u32,
}

/// What a call of a [`SiblingTarget::function`] needs at codegen.
#[derive(Debug, Clone)]
pub(crate) struct SiblingCallee<'ctx> {
    producer: FunctionValue<'ctx>,
    table: inkwell::values::GlobalValue<'ctx>,
    index: u32,
    global: String,
}

/// Declares each target's producer function (with its own function's
/// signature) and table, and attaches them to its own function.
pub(crate) fn attach<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    targets: &[SiblingTarget],
    user_functions: &mut HashMap<&str, UserFunction<'ctx>>,
) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    for target in targets {
        let Some(own) = user_functions.get_mut(target.function.as_str()) else {
            continue;
        };
        let producer = module
            .get_function(&target.producer_symbol)
            .unwrap_or_else(|| {
                module.add_function(
                    &target.producer_symbol,
                    own.fn_type,
                    Some(inkwell::module::Linkage::External),
                )
            });
        let table = module.get_global(&target.table_symbol).unwrap_or_else(|| {
            let table = module.add_global(ptr.array_type(0), None, &target.table_symbol);
            table.set_linkage(inkwell::module::Linkage::External);
            table
        });
        own.sibling = Some(SiblingCallee {
            producer,
            table,
            index: target.index,
            global: target.global.clone(),
        });
    }
}

/// The callee of one call of a sibling function: the producer's compiled
/// function while the module global still binds the object the producer
/// published, `own` (the body calling the object) otherwise.
pub(crate) fn resolve<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    sibling: &SiblingCallee<'ctx>,
    own: FunctionValue<'ctx>,
) -> Callee<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let bound = namespace_read(context, builder, module, rt, &sibling.global);
    let entry = unsafe {
        builder.build_in_bounds_gep(
            ptr.array_type(0),
            sibling.table.as_pointer_value(),
            &[
                context.i64_type().const_zero(),
                context
                    .i64_type()
                    .const_int(u64::from(sibling.index), false),
            ],
            "sibling_entry",
        )
    }
    .expect("build_in_bounds_gep should not fail for the interface table");
    let published = builder
        .build_load(ptr, entry, "sibling_published")
        .expect("build_load should not fail for an interface table entry")
        .into_pointer_value();
    let same = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            bound,
            published,
            "sibling_is_published",
        )
        .expect("build_int_compare should not fail for two pointers");
    let target = builder
        .build_select(
            same,
            sibling.producer.as_global_value().as_pointer_value(),
            own.as_global_value().as_pointer_value(),
            "sibling_callee",
        )
        .expect("build_select should not fail for two function pointers")
        .into_pointer_value();
    Callee::Indirect(target)
}

/// The object the module dict binds to `name`, for a name the module keeps
/// only there (an import the fallback loop runs in CPython): a slot of this
/// module's own, brought up to date from the dict whenever its version tag
/// changed (`global_sync`), so a read is a few loads while nothing rebinds
/// a name. Null while the dict has not bound the name, or on a CPython
/// whose dict layout the shim does not know -- either way not the published
/// object, so the call takes the body that calls the object.
fn namespace_read<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let slot_name = format!("pycc_sib_ns.{name}");
    let slot = module.get_global(&slot_name).unwrap_or_else(|| {
        let slot = module.add_global(ptr, None, &slot_name);
        slot.set_initializer(&ptr.const_null());
        slot.set_linkage(inkwell::module::Linkage::Private);
        slot
    });
    let flag_name = format!("pycc_sib_ns_bound.{name}");
    let flag = module.get_global(&flag_name).unwrap_or_else(|| {
        let flag = module.add_global(context.i8_type(), None, &flag_name);
        flag.set_initializer(&context.i8_type().const_zero());
        flag.set_linkage(inkwell::module::Linkage::Private);
        flag
    });
    crate::global_sync::emit_sync(
        context,
        builder,
        module,
        rt,
        name,
        slot.as_pointer_value(),
        flag.as_pointer_value(),
        &pycc_mir::Ty::Object,
    );
    builder
        .build_load(ptr, slot.as_pointer_value(), "sibling_bound")
        .expect("build_load should not fail for a namespace slot")
        .into_pointer_value()
}

/// The name a sibling function's own diagnostics read: the imported name.
pub(crate) fn display_name(name: &str) -> &str {
    name.strip_prefix(pycc_hir::SIBLING_CALL_PREFIX)
        .unwrap_or(name)
}
