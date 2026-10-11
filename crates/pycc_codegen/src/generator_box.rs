//! A native generator boxed into an `object` slot: the CPython iterator
//! `pycc_ext_gen_wrap` builds around it (`src/ext/module/object_ops.c`).
//!
//! The iterator shares the generator -- it holds one reference -- and
//! produces each value through a per-element-type function emitted here,
//! `pycc_gen_box_next.<T>`: it resumes the generator, packs the value the
//! generator left in its value area into a new CPython reference
//! (`foreign_pack::emit_pack`, which borrows) and releases the native value
//! it took ownership of. It returns `NULL` when the generator is exhausted
//! or raised; the C side tells the two apart by the runtime's pending
//! exception, which it raises in CPython. Releasing the iterator releases
//! the generator, closing it if it is unfinished.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::foreign_pack::{emit_pack, shim_fn};
use pycc_mir::Ty;

/// The C helper building the iterator: `(generator, next function) ->
/// PyObject *`, stealing the generator reference.
const GEN_WRAP_SYMBOL: &str = "pycc_ext_gen_wrap";

/// `value` (a generator) evaluated and boxed into a new CPython iterator.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_boxed_generator<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExpr,
    element: &Ty,
) -> PointerValue<'ctx> {
    let Scalar::Generator(object) =
        emit_expr(context, builder, module, rt, user_functions, locals, value)
    else {
        panic!("pycc_codegen: internal error: a boxed generator is not a generator")
    };
    // The iterator steals one reference: a fresh generator's, or a new one
    // taken on a generator some slot keeps owning.
    if heap_rc::is_borrowed_read(value) {
        heap_rc::emit_incref(context, builder, rt, heap_rc::HeapKind::Generator, object);
    }
    let next = next_function(context, module, rt, element);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let wrap = shim_fn(
        module,
        GEN_WRAP_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into()], false),
    );
    let boxed = builder
        .build_call(
            wrap,
            &[
                object.into(),
                next.as_global_value().as_pointer_value().into(),
            ],
            "gen_box",
        )
        .expect("build_call should not fail for pycc_ext_gen_wrap")
        .try_as_basic_value()
        .expect_basic("pycc_ext_gen_wrap returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        ForeignFailEdge::for_current(builder),
        boxed,
        "gen_box",
    );
    boxed
}

/// `pycc_gen_box_next.<element>`, emitted once per module.
fn next_function<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    element: &Ty,
) -> FunctionValue<'ctx> {
    let name = format!("pycc_gen_box_next.{}", element.name());
    if let Some(existing) = module.get_function(&name) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let function = module.add_function(
        &name,
        ptr.fn_type(&[ptr.into()], false),
        Some(inkwell::module::Linkage::Private),
    );
    let builder = context.create_builder();
    let entry = context.append_basic_block(function, "entry");
    let got = context.append_basic_block(function, "got");
    let done = context.append_basic_block(function, "done");
    builder.position_at_end(entry);
    let object = function
        .get_nth_param(0)
        .expect("the next function takes the generator")
        .into_pointer_value();
    let answer = builder
        .build_call(rt.gens.resume, &[object.into()], "gen_resume")
        .expect("build_call should not fail for pycc_rt_gen_resume")
        .try_as_basic_value()
        .expect_basic("pycc_rt_gen_resume returns an i8")
        .into_int_value();
    let has_value = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            answer,
            context.i8_type().const_zero(),
            "gen_has_value",
        )
        .expect("build_int_compare should not fail for an i8");
    builder
        .build_conditional_branch(has_value, got, done)
        .expect("build_conditional_branch should not fail for an i1");
    builder.position_at_end(got);
    let value = crate::generator::load_value(context, &builder, object, element);
    let packed = emit_pack(context, &builder, module, value, "gen_box_value");
    crate::generator::release_owned(context, &builder, rt, element, &value);
    builder
        .build_return(Some(&packed))
        .expect("build_return should not fail for the packed value");
    builder.position_at_end(done);
    builder
        .build_return(Some(&ptr.const_null()))
        .expect("build_return should not fail for NULL");
    function
}
