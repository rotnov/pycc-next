//! Comprehensions assigned straight into a binding.

use super::*;

// `target = <comprehension>` (PR-12 Tasks 5a/5b, D-117): the loop
// itself lives in `comprehension::emit_comprehension` (see its module
// docs for the shape and for why `target` is bound only after the
// whole loop has run, never before).
pub(crate) fn emit_list_comp_assign_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ListCompAssign {
        target,
        var,
        var_ty,
        source,
        cond,
        elt,
    } = stmt
    else {
        unreachable!()
    };
    let container = emit_comprehension(
        &CompCx {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        var,
        var_ty,
        source,
        cond.as_deref(),
        CompElts::List(elt),
    );
    store_fresh_container(context, builder, rt, locals, target, container);
    Ok(())
}

pub(crate) fn emit_set_comp_assign_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::SetCompAssign {
        target,
        var,
        var_ty,
        source,
        cond,
        elt,
        ops,
    } = stmt
    else {
        unreachable!()
    };
    let container = emit_comprehension(
        &CompCx {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        var,
        var_ty,
        source,
        cond.as_deref(),
        CompElts::Set(elt, ops.as_deref()),
    );
    store_fresh_container(context, builder, rt, locals, target, container);
    Ok(())
}

pub(crate) fn emit_dict_comp_assign_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::DictCompAssign {
        target,
        var,
        var_ty,
        source,
        cond,
        key,
        value,
    } = stmt
    else {
        unreachable!()
    };
    let container = emit_comprehension(
        &CompCx {
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
        },
        var,
        var_ty,
        source,
        cond.as_deref(),
        CompElts::Dict(key, value),
    );
    store_fresh_container(context, builder, rt, locals, target, container);
    Ok(())
}
