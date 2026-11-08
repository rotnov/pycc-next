//! Direct calls between compiled bodies in generic code.
//!
//! A lifted body (`__pycc_liftN_<name>__`, `src/fallback/lift.rs`) is
//! called from CPython through its Python forwarder, whose vectorcall is a
//! trampoline (`src/ext/module/forward.c`): the call goes through the
//! trampoline, the forwarder's slot, a recursion check and the body's
//! `METH_FASTCALL` wrapper, which takes a reference to each argument, calls
//! the body and turns a pending exception into CPython's. A compiled call
//! site calling the forwarder by name -- `name(...)` for a module global,
//! `obj.name(...)` for a method -- pays all of that on every call.
//!
//! So such a site, when the module has one lifted body of that name whose
//! parameters the call binds in order, checks inline that its callee is
//! that body's forwarder, still running the code it was patched with, and
//! calls the body itself: the arguments as the references the call already
//! made, and its exception, a pycc one already, taken on the site's own
//! edge. Anything else -- another callee, `__code__` replaced, a positional
//! count the parameters do not take, the recursion limit -- calls through
//! the forwarder as before.
//!
//! What the check reads is the body's *cell* (`fnptr_<name>__pycc_dcell`,
//! three words, zero at first, defined here beside `fnptr_<name>`): the
//! forwarder, its code and the positional counts a call may pass, which
//! `forward.c` fills when it patches the forwarder. The generated companion
//! names each cell by its body's wrapper (`direct_cells_c` in
//! `src/ext_build/exports_inc.rs`). A forwarder with `*args`, `**kwargs`
//! or a `__class__` cell leaves its cell zero, so no callee matches it.
//!
//! The keyword names a call passes are checked against the body's
//! parameter names here, at compile time: the forwarder's parameters are
//! the body's, in the same order (`Lift::forwarder`).

use super::*;
use inkwell::builder::Builder;
use inkwell::module::{Linkage, Module};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};

/// The suffix of a lifted body's cell after its `fnptr_` name. It starts
/// with the `fnptr_` slot's name, so a package member's header prefixes it
/// as it prefixes the slot (`pycc_ext_member.h`).
pub const DIRECT_CELL_SUFFIX: &str = "__pycc_dcell";

/// `int pycc_ext_direct_enter(void)` and `void pycc_ext_direct_leave(void)`
/// (`src/ext/module/fast_ops.c`): the recursion check around a direct call.
const EXT_DIRECT_ENTER_SYMBOL: &str = "pycc_ext_direct_enter";
const EXT_DIRECT_LEAVE_SYMBOL: &str = "pycc_ext_direct_leave";

/// The byte offset of `func_code` in a `PyFunctionObject` on CPython 3.13
/// and 3.15 (`PYCC_FUNC_CODE` in `forward.c`, checked by its probe before
/// any cell is filled).
const FUNC_CODE_OFFSET: u64 = 48;

/// The name of the source function a lifted body was lifted from, or None
/// for every other function (a nested body included).
fn lifted_name(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("__pycc_lift")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let source = rest[digits..].strip_prefix('_')?.strip_suffix("__")?;
    (digits > 0 && !source.is_empty() && !name.contains('.')).then_some(source)
}

/// Defines the cell of the lifted body `name`, zero, where the companion
/// finds it. Does nothing for any other function.
pub(crate) fn define_cell<'ctx>(context: &'ctx Context, module: &Module<'ctx>, name: &str) {
    if !name.starts_with("__pycc_lift") || name.contains('.') {
        return;
    }
    let ty = context.i64_type().array_type(3);
    let cell = module.add_global(ty, None, &format!("fnptr_{name}{DIRECT_CELL_SUFFIX}"));
    cell.set_initializer(&ty.const_zero());
}

/// The body a call may run directly, and its cell.
pub(crate) struct DirectTarget<'ctx> {
    function: FunctionValue<'ctx>,
    /// The body's name among the module's functions.
    name: String,
    cell: PointerValue<'ctx>,
    /// How many arguments the call passes positionally, the receiver of a
    /// method call included: what the cell's counts are tested against.
    positional: u32,
    /// The body's multi-value twin the direct path calls instead, with the
    /// out-pointers it passes (`crate::multi_out`).
    multi: Option<(FunctionValue<'ctx>, Vec<PointerValue<'ctx>>)>,
}

/// An unpacking call's request for its callee's multi-value twin
/// (`pycc_mir::multi_return_object`): the twin of arity `outs.len() + 1`
/// of `body` (of whichever body the call runs directly, when `None`), and
/// the out-pointers it stores the results after the first through.
pub(crate) struct MultiCall<'ctx> {
    pub(crate) body: Option<String>,
    pub(crate) outs: Vec<PointerValue<'ctx>>,
}

impl<'ctx> DirectTarget<'ctx> {
    /// This target, calling its body's twin on the direct path when
    /// `multi` asks for it and the module has that twin.
    pub(crate) fn with_multi(
        mut self,
        user_functions: &HashMap<&str, UserFunction<'ctx>>,
        multi: Option<&MultiCall<'ctx>>,
    ) -> Self {
        let Some(multi) = multi else {
            return self;
        };
        if multi.body.as_ref().is_some_and(|body| *body != self.name) {
            return self;
        }
        let twin = pycc_mir::multi_return_object::twin_name(&self.name, multi.outs.len() + 1);
        if let Some(twin) = user_functions
            .get(twin.as_str())
            .and_then(|f| f.direct_value)
        {
            self.multi = Some((twin, multi.outs.clone()));
        }
        self
    }
}

/// The direct target of a call of the source function `source` with
/// `positional` arguments passed positionally (a method call's receiver
/// counted) and then `names` by keyword: the one lifted body of that name
/// whose parameters, all objects, the call binds in order.
fn target<'ctx>(
    module: &Module<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    source: &str,
    positional: usize,
    names: &[String],
) -> Option<DirectTarget<'ctx>> {
    let mut found = None;
    for (name, function) in user_functions {
        if lifted_name(name) != Some(source) {
            continue;
        }
        let binds = function.param_names.len() == positional + names.len()
            && function.param_names[positional..] == *names
            // A body taking the class first is called with the forwarder's
            // `__class__` cell, which the call site does not have.
            && function.param_names.first().map(String::as_str) != Some("__pycc_class__")
            // A private name is mangled in a class body, by CPython for the
            // forwarder's parameters and the call's keywords alike.
            && !names.iter().any(|keyword| keyword.starts_with("__"))
            && function.param_tys.iter().all(|ty| *ty == pycc_mir::Ty::Object)
            && function
                .fn_type
                .get_return_type()
                .is_some_and(|ret| ret.is_pointer_type());
        let (Some(body), true) = (function.single_def, binds) else {
            continue;
        };
        let cell = module.get_global(&format!("fnptr_{name}{DIRECT_CELL_SUFFIX}"))?;
        if found.is_some() {
            // Two bodies of the name: the site cannot tell them apart.
            return None;
        }
        found = Some(DirectTarget {
            function: body,
            name: (*name).to_string(),
            cell: cell.as_pointer_value(),
            positional: u32::try_from(positional).ok()?,
            multi: None,
        });
    }
    found.filter(|target| target.positional < 64)
}

/// The direct target of `obj.method(...)` passing `positional` arguments
/// after the receiver, then `names`.
pub(crate) fn method_target<'ctx>(
    module: &Module<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    method: &str,
    positional: usize,
    names: &[String],
) -> Option<DirectTarget<'ctx>> {
    target(module, user_functions, method, positional + 1, names)
}

/// The direct target of `callee(...)` when `callee` reads a module global
/// (`__pycc_fallback_ns["name"]`).
pub(crate) fn global_target<'ctx>(
    module: &Module<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    callee: &MirExpr,
    positional: usize,
    names: &[String],
) -> Option<DirectTarget<'ctx>> {
    let MirExpr::ObjSubscript { base, index } = callee else {
        return None;
    };
    let (MirExpr::Name { name: ns, .. }, MirExpr::StringLiteral(global)) =
        (base.as_ref(), index.as_ref())
    else {
        return None;
    };
    if ns != crate::ext::FALLBACK_NS_NAME {
        return None;
    }
    target(module, user_functions, global, positional, names)
}

fn shim_void_fn<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    symbol: &str,
    returns_int: bool,
) -> FunctionValue<'ctx> {
    module.get_function(symbol).unwrap_or_else(|| {
        let ty = if returns_int {
            context.i32_type().fn_type(&[], false)
        } else {
            context.void_type().fn_type(&[], false)
        };
        module.add_function(symbol, ty, Some(Linkage::External))
    })
}

/// Emits the call of `callee` with the packed arguments `args` (new
/// references, a failed packer's NULL among them possibly, the receiver of
/// a method call first) as `target`'s body when the cell says the callee
/// is its forwarder, and as `slow` otherwise. `slow` emits the call through
/// the forwarder at the builder's position, consuming the arguments and,
/// when `consumes_callee`, the callee, and leaves the builder on its
/// success continuation with its result; the direct path consumes the same.
/// A body that raised leaves its exception pending, so the direct path
/// branches to the innermost exception target as a statement's guard does
/// (`exception::jump_to_exception_target`). Inside the module-exec entry,
/// whose foreign failures may return CPython's exception to the import,
/// only `slow` is emitted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    rt: &RtFns<'ctx>,
    target: Option<&DirectTarget<'ctx>>,
    callee: PointerValue<'ctx>,
    consumes_callee: bool,
    args: &[PointerValue<'ctx>],
    slow: impl FnOnce() -> PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    let edge = crate::foreign_fail::ForeignFailEdge::for_current(builder);
    let (Some(target), crate::foreign_fail::ForeignFailEdge::Function(function)) = (target, edge)
    else {
        return slow();
    };
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let check_bb = context.append_basic_block(function, "direct_check");
    let enter_bb = context.append_basic_block(function, "direct_enter");
    let call_bb = context.append_basic_block(function, "direct_call");
    let unwind_bb = context.append_basic_block(function, "direct_unwind");
    let done_bb = context.append_basic_block(function, "direct_done");
    let slow_bb = context.append_basic_block(function, "direct_slow");
    let merge_bb = context.append_basic_block(function, "direct_merge");

    let word = |index: u64, name: &str| -> BasicValueEnum<'ctx> {
        let at = unsafe {
            builder
                .build_in_bounds_gep(
                    i64_type,
                    target.cell,
                    &[i64_type.const_int(index, false)],
                    &format!("{name}_at"),
                )
                .expect("build_in_bounds_gep should not fail")
        };
        builder
            .build_load(
                if index == 2 {
                    inkwell::types::BasicTypeEnum::from(i64_type)
                } else {
                    ptr.into()
                },
                at,
                name,
            )
            .expect("build_load should not fail")
    };
    let branch = |condition, then_bb, else_bb| {
        let instruction = builder
            .build_conditional_branch(condition, then_bb, else_bb)
            .expect("build_conditional_branch should not fail");
        crate::int_fast::weight_branch(context, instruction, true);
    };

    // The callee is the body's forwarder.
    let forwarder = word(0, "direct_func").into_pointer_value();
    let same = builder
        .build_int_compare(IntPredicate::EQ, callee, forwarder, "direct_same")
        .expect("icmp");
    branch(same, check_bb, slow_bb);

    // Still running the code it was patched with, called with a positional
    // count its parameters take, every argument packed.
    builder.position_at_end(check_bb);
    let code_at = unsafe {
        builder
            .build_in_bounds_gep(
                context.i8_type(),
                callee,
                &[i64_type.const_int(FUNC_CODE_OFFSET, false)],
                "direct_code_at",
            )
            .expect("build_in_bounds_gep should not fail")
    };
    let code = builder
        .build_load(ptr, code_at, "direct_code")
        .expect("build_load should not fail")
        .into_pointer_value();
    let patched = word(1, "direct_patched").into_pointer_value();
    let mut ok = builder
        .build_int_compare(IntPredicate::EQ, code, patched, "direct_code_same")
        .expect("icmp");
    let counts = word(2, "direct_counts").into_int_value();
    let bit = builder
        .build_and(
            counts,
            i64_type.const_int(1u64 << target.positional, false),
            "direct_count_bit",
        )
        .expect("and");
    let takes = builder
        .build_int_compare(IntPredicate::NE, bit, i64_type.const_zero(), "direct_takes")
        .expect("icmp");
    ok = builder.build_and(ok, takes, "direct_ok").expect("and");
    for (index, arg) in args.iter().enumerate() {
        let present = builder
            .build_is_not_null(*arg, &format!("direct_arg{index}"))
            .expect("build_is_not_null should not fail");
        ok = builder.build_and(ok, present, "direct_ok").expect("and");
    }
    branch(ok, enter_bb, slow_bb);

    // The recursion check the forwarder's call makes; at the limit the
    // forwarder raises `RecursionError` itself.
    builder.position_at_end(enter_bb);
    let entered = builder
        .build_call(
            shim_void_fn(context, module, EXT_DIRECT_ENTER_SYMBOL, true),
            &[],
            "direct_entered",
        )
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("pycc_ext_direct_enter returns int")
        .into_int_value();
    let entered = builder
        .build_int_compare(
            IntPredicate::NE,
            entered,
            context.i32_type().const_zero(),
            "direct_entered_ok",
        )
        .expect("icmp");
    branch(entered, call_bb, slow_bb);

    builder.position_at_end(call_bb);
    let mut operands: Vec<BasicMetadataValueEnum<'ctx>> =
        args.iter().map(|arg| (*arg).into()).collect();
    let function = match &target.multi {
        Some((twin, outs)) => {
            operands.extend(outs.iter().map(|out| BasicMetadataValueEnum::from(*out)));
            *twin
        }
        None => target.function,
    };
    let result = builder
        .build_call(function, &operands, "direct_result")
        .expect("build_call should not fail for a direct call")
        .try_as_basic_value()
        .expect_basic("a lifted body returns PyObject *")
        .into_pointer_value();
    builder
        .build_call(
            shim_void_fn(context, module, EXT_DIRECT_LEAVE_SYMBOL, false),
            &[],
            "",
        )
        .expect("build_call should not fail");
    if consumes_callee {
        crate::object_release::release_now(context, module, builder, callee);
    }
    let raised = crate::int_fast::emit_exception_pending_test(context, builder, rt, "direct_raised");
    let instruction = builder
        .build_conditional_branch(raised, unwind_bb, done_bb)
        .expect("build_conditional_branch should not fail");
    crate::int_fast::weight_branch(context, instruction, false);
    builder.position_at_end(unwind_bb);
    crate::exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(done_bb);
    builder
        .build_unconditional_branch(merge_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(slow_bb);
    let slow_result = slow();
    let slow_end = builder.get_insert_block().expect("slow path ends in a block");
    builder
        .build_unconditional_branch(merge_bb)
        .expect("build_unconditional_branch should not fail");

    builder.position_at_end(merge_bb);
    let phi = builder.build_phi(ptr, "direct_or_slow").expect("build_phi should not fail");
    phi.add_incoming(&[(&result, done_bb), (&slow_result, slow_end)]);
    phi.as_basic_value().into_pointer_value()
}
