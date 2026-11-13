//! `declare_rt_functions`: declares every `pycc_rt` function `RtFns`
//! holds in the module being emitted and builds the handle, with the
//! per-module and per-function codegen state it carries empty.

use super::*;

pub(crate) fn declare_rt_functions<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> RtFns<'ctx> {
    let i64_type = context.i64_type();
    let i32_type = context.i32_type();
    let void_type = context.void_type();
    let f64_type = context.f64_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let declare = |name: &str, fn_type: inkwell::types::FunctionType<'ctx>| {
        module.add_function(name, fn_type, Some(Linkage::External))
    };
    let overflow_intrinsic = |name: &str| {
        inkwell::intrinsics::Intrinsic::find(name)
            .and_then(|intrinsic| intrinsic.get_declaration(module, &[i64_type.into()]))
            .expect("LLVM provides the integer overflow intrinsics")
    };
    let f64_intrinsic = |name: &str| {
        inkwell::intrinsics::Intrinsic::find(name)
            .and_then(|intrinsic| intrinsic.get_declaration(module, &[f64_type.into()]))
            .expect("LLVM provides the f64 math intrinsics")
    };
    RtFns {
        containers: crate::containers::ContainerFns::declare(context, module),
        gens: crate::generator::GenFns::declare(context, module),
        num: crate::num::NumFns::declare(context, module),
        closures: crate::closure::ClosureFns::declare(context, module),
        gen_frame: std::cell::RefCell::new(None),
        context,
        sqrt_f64: f64_intrinsic("llvm.sqrt"),
        int_truediv: declare(
            "pycc_rt_int_truediv",
            f64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_float_cmp: declare(
            "pycc_rt_int_float_cmp",
            i32_type.fn_type(&[i64_type.into(), f64_type.into()], false),
        ),
        int_from_float: declare(
            "pycc_rt_int_from_float",
            i64_type.fn_type(&[f64_type.into()], false),
        ),
        int_abs: declare(
            "pycc_rt_int_abs",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        int_mod_noraise: declare(
            "pycc_rt_int_mod_noraise",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        sadd_overflow: overflow_intrinsic("llvm.sadd.with.overflow"),
        ssub_overflow: overflow_intrinsic("llvm.ssub.with.overflow"),
        smul_overflow: overflow_intrinsic("llvm.smul.with.overflow"),
        int_add: declare(
            "pycc_rt_int_add",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_sub: declare(
            "pycc_rt_int_sub",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_mul: declare(
            "pycc_rt_int_mul",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_floordiv: declare(
            "pycc_rt_int_floordiv",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_floormod: declare(
            "pycc_rt_int_floormod",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_pow: declare(
            "pycc_rt_int_pow",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_lshift: declare(
            "pycc_rt_int_lshift",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_rshift: declare(
            "pycc_rt_int_rshift",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_and: declare(
            "pycc_rt_int_and",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_or: declare(
            "pycc_rt_int_or",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_xor: declare(
            "pycc_rt_int_xor",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_cmp: declare(
            "pycc_rt_int_cmp",
            i32_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_truthy: declare(
            "pycc_rt_int_truthy",
            context.i8_type().fn_type(&[i64_type.into()], false),
        ),
        range_continue: declare(
            "pycc_rt_range_continue",
            context
                .i8_type()
                .fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        range_sized_stop: declare(
            "pycc_rt_range_sized_stop",
            i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        range_normalize_operand: declare(
            "pycc_rt_range_normalize_operand",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        int_to_float: declare(
            "pycc_rt_int_to_float",
            f64_type.fn_type(&[i64_type.into()], false),
        ),
        float_div: declare(
            "pycc_rt_float_div",
            f64_type.fn_type(&[f64_type.into(), f64_type.into()], false),
        ),
        float_floordiv: declare(
            "pycc_rt_float_floordiv",
            f64_type.fn_type(&[f64_type.into(), f64_type.into()], false),
        ),
        float_floormod: declare(
            "pycc_rt_float_floormod",
            f64_type.fn_type(&[f64_type.into(), f64_type.into()], false),
        ),
        float_pow: declare(
            "pycc_rt_float_pow",
            f64_type.fn_type(&[f64_type.into(), f64_type.into()], false),
        ),
        libm_pow: declare(
            "pow",
            f64_type.fn_type(&[f64_type.into(), f64_type.into()], false),
        ),
        str_from_literal: declare(
            "pycc_rt_str_from_literal_cached",
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into()], false),
        ),
        str_concat: declare(
            "pycc_rt_str_concat",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
        ),
        str_repeat: declare(
            "pycc_rt_str_repeat",
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        str_cmp: declare(
            "pycc_rt_str_cmp",
            i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
        ),
        str_truthy: declare(
            "pycc_rt_str_truthy",
            context.i8_type().fn_type(&[ptr_type.into()], false),
        ),
        str_incref: declare(
            "pycc_rt_str_incref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        str_decref: declare(
            "pycc_rt_str_decref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        obj_retain: declare(
            crate::ext::EXT_OBJ_RETAIN_SYMBOL,
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        obj_release: declare(
            crate::ext::EXT_OBJ_RELEASE_SYMBOL,
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        bigint_retain: declare(
            "pycc_rt_bigint_retain",
            void_type.fn_type(&[i64_type.into()], false),
        ),
        bigint_release: declare(
            "pycc_rt_bigint_release",
            void_type.fn_type(&[i64_type.into()], false),
        ),
        int_to_str: declare(
            "pycc_rt_int_to_str",
            ptr_type.fn_type(&[i64_type.into()], false),
        ),
        float_to_str: declare(
            "pycc_rt_float_to_str",
            ptr_type.fn_type(&[f64_type.into()], false),
        ),
        bool_to_str: declare(
            "pycc_rt_bool_to_str",
            ptr_type.fn_type(&[context.i8_type().into()], false),
        ),
        optional_to_str: [
            declare(
                "pycc_rt_optional_int_to_str",
                ptr_type.fn_type(&[i64_type.into(), context.i8_type().into()], false),
            ),
            declare(
                "pycc_rt_optional_float_to_str",
                ptr_type.fn_type(&[f64_type.into(), context.i8_type().into()], false),
            ),
            declare(
                "pycc_rt_optional_bool_to_str",
                ptr_type.fn_type(&[context.i8_type().into(), context.i8_type().into()], false),
            ),
        ],
        print_write_str: declare(
            "pycc_rt_print_write_str",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        print_space: declare("pycc_rt_print_space", void_type.fn_type(&[], false)),
        print_newline: declare("pycc_rt_print_newline", void_type.fn_type(&[], false)),
        print_none: declare("pycc_rt_print_none", void_type.fn_type(&[], false)),
        print_flush: declare("pycc_rt_print_flush", void_type.fn_type(&[], false)),
        int_from_i64: declare(
            "pycc_rt_int_from_i64",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        hash_int: declare(
            "pycc_rt_hash_int",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        hash_tuple: declare(
            "pycc_rt_hash_tuple",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        hash_str: declare(
            "pycc_rt_hash_str",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        hash_float: declare(
            "pycc_rt_hash_float",
            i64_type.fn_type(&[f64_type.into()], false),
        ),
        hash_tuple_object: declare(
            "pycc_rt_hash_tuple_object",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        hash_pointer: declare(
            "pycc_rt_hash_pointer",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        hash_slot_int: declare(
            "pycc_rt_hash_slot_int",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        int_untag_checked: declare(
            "pycc_rt_int_untag_checked",
            i64_type.fn_type(&[i64_type.into(), i32_type.into()], false),
        ),
        float_round: declare(
            "pycc_rt_float_round",
            f64_type.fn_type(&[f64_type.into(), i64_type.into()], false),
        ),
        float_round_to_int: declare(
            "pycc_rt_float_round_to_int",
            i64_type.fn_type(&[f64_type.into()], false),
        ),
        list_new: declare(
            "pycc_rt_list_new",
            ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        list_append: declare(
            "pycc_rt_list_append",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        list_reserve: declare(
            "pycc_rt_list_reserve",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        list_reset: declare("pycc_rt_list_reset", void_type.fn_type(&[ptr_type.into()], false)),
        comprehension_into: std::cell::Cell::new(None),
        list_index_error: declare(
            "pycc_rt_list_index_error_of",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        tuple_index_error: declare("pycc_rt_tuple_index_error", void_type.fn_type(&[], false)),
        list_set: declare(
            "pycc_rt_list_set",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        list_pop: declare(
            "pycc_rt_list_pop",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        list_insert: declare(
            "pycc_rt_list_insert",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        list_slice: declare(
            "pycc_rt_list_slice",
            ptr_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    i64_type.into(),
                    i64_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
        ),
        list_concat: declare(
            "pycc_rt_list_concat",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
        ),
        list_repeat: declare(
            "pycc_rt_list_repeat",
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        list_contains: declare(
            "pycc_rt_list_contains",
            context
                .i8_type()
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        list_repr: declare(
            "pycc_rt_list_repr",
            ptr_type.fn_type(&[ptr_type.into()], false),
        ),
        dict_repr: declare(
            "pycc_rt_dict_repr",
            ptr_type.fn_type(&[ptr_type.into()], false),
        ),
        buffer_f64_get: declare(
            "pycc_rt_buffer_f64_get",
            f64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        buffer_f64_set: declare(
            "pycc_rt_buffer_f64_set",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), f64_type.into()], false),
        ),
        buffer_len: declare(
            "pycc_rt_buffer_len",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        buffer_alloc_untag_len: declare(
            "pycc_rt_buffer_alloc_untag_len",
            i64_type.fn_type(&[i64_type.into()], false),
        ),
        buffer_f64_alloc: declare(
            "pycc_rt_buffer_f64_alloc",
            ptr_type.fn_type(&[i64_type.into()], false),
        ),
        // Returns nothing, exactly like `buffer_f64_set` above, so its call
        // site must not go through `try_as_basic_value()`.
        buffer_f64_free: declare(
            "pycc_rt_buffer_f64_free",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        dict_new: declare(
            "pycc_rt_dict_new",
            ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        // Returns nothing, exactly like `list_append` above: this
        // signature must match `pycc_rt_dict_set`'s real Rust one
        // (`fn(*mut PyDictObj, *mut PyStrObj, i64) -> ()`) -- key is a
        // second, distinct pointer parameter (a dict's key and its
        // container are two different heap objects), not folded into one
        // like `list_append`'s single `i64` value parameter.
        dict_set: declare(
            "pycc_rt_dict_set",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        dict_get: declare(
            "pycc_rt_dict_get",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        dict_get_or_default: declare(
            "pycc_rt_dict_get_or_default",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        dict_len: declare(
            "pycc_rt_dict_len",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        dict_key_at: declare(
            "pycc_rt_dict_key_at",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        dict_check_not_resized: declare(
            "pycc_rt_dict_check_not_resized",
            void_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        int_set_new: declare("pycc_rt_int_set_new", ptr_type.fn_type(&[], false)),
        int_set_len: declare(
            "pycc_rt_set_len",
            i64_type.fn_type(&[ptr_type.into()], false),
        ),
        int_set_get: declare(
            "pycc_rt_set_get",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        int_set_copy: declare(
            "pycc_rt_int_set_copy",
            ptr_type.fn_type(&[ptr_type.into()], false),
        ),
        int_set_check_not_resized: declare(
            "pycc_rt_set_check_not_resized",
            void_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        ),
        obj_set_candidate: declare(
            "pycc_rt_obj_set_candidate",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        obj_set_push: declare(
            "pycc_rt_obj_set_push",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        obj_set_add_identity: declare(
            "pycc_rt_obj_set_add_identity",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        instance_new: declare(
            "pycc_rt_instance_new",
            ptr_type.fn_type(&[i64_type.into(), ptr_type.into(), i64_type.into()], false),
        ),
        instance_get_slot_checked: declare(
            "pycc_rt_instance_get_slot_checked",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        instance_class_is: declare(
            "pycc_rt_instance_class_is",
            context
                .i8_type()
                .fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
        ),
        instance_set_slot: declare(
            "pycc_rt_instance_set_slot",
            void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        ),
        raise_name_error: declare(
            "pycc_rt_raise_name_error",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        exc_pending: {
            let global = module.add_global(context.i8_type(), None, "pycc_rt_exc_pending");
            global.set_linkage(Linkage::External);
            global
        },
        exc_pending_threads: {
            let global =
                module.add_global(context.i64_type(), None, "pycc_rt_exc_pending_threads");
            global.set_linkage(Linkage::External);
            global
        },
        exception_active: declare(
            "pycc_rt_exception_active",
            context.i8_type().fn_type(&[], false),
        ),
        thread_safe_exception_check: std::cell::Cell::new(false),
        publish_handled: std::cell::Cell::new(false),
        glibc_libm: std::cell::Cell::new(false),
        assigned_slots: std::cell::RefCell::new(Default::default()),
        emitting_ctor: std::cell::Cell::new(false),
        nothrow: std::cell::RefCell::new(Default::default()),
        exception_value: declare("pycc_rt_exception_value", ptr_type.fn_type(&[], false)),
        exception_take: declare("pycc_rt_exception_take", ptr_type.fn_type(&[], false)),
        exc_incref: declare(
            "pycc_rt_exception_incref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exc_decref: declare(
            "pycc_rt_exception_decref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_alloc: declare(
            "pycc_rt_exception_alloc",
            ptr_type.fn_type(
                &[
                    context.i8_type().into(),
                    ptr_type.into(),
                    context.i64_type().into(),
                    ptr_type.into(),
                ],
                false,
            ),
        ),
        exception_raise: declare(
            "pycc_rt_exception_raise",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_suppress_context: declare(
            "pycc_rt_exception_suppress_context",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_restore: declare(
            "pycc_rt_exception_restore",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_raise_with_cause: declare(
            "pycc_rt_exception_raise_with_cause",
            void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
        ),
        exception_set_frame: declare(
            "pycc_rt_exception_set_frame",
            void_type.fn_type(
                &[ptr_type.into(), ptr_type.into(), context.i64_type().into()],
                false,
            ),
        ),
        exception_type_matches: declare(
            "pycc_rt_exception_type_matches",
            context
                .i8_type()
                .fn_type(&[ptr_type.into(), context.i8_type().into()], false),
        ),
        exception_message: declare(
            "pycc_rt_exception_message",
            ptr_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_print_and_exit: declare(
            "pycc_rt_exception_print_and_exit",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        exception_group_alloc: declare(
            "pycc_rt_exception_group_alloc",
            ptr_type.fn_type(
                &[
                    context.i8_type().into(),
                    ptr_type.into(),
                    context.i64_type().into(),
                    ptr_type.into(),
                    ptr_type.into(),
                    context.i64_type().into(),
                ],
                false,
            ),
        ),
        exception_group_partition: declare(
            "pycc_rt_exception_group_partition",
            void_type.fn_type(
                &[
                    ptr_type.into(),
                    ptr_type.into(),
                    context.i64_type().into(),
                    context.i8_type().into(),
                    ptr_type.into(),
                    context.i64_type().into(),
                    ptr_type.into(),
                    ptr_type.into(),
                ],
                false,
            ),
        ),
        exceptions: ExceptionCodegenState::new(),
        frames: crate::frames::FrameState::declare(context, module),
        nd_caches: crate::ndarray::NdCaches::default(),
        loops: crate::loop_control::LoopControlState::default(),
        char_classes: crate::char_class::LoopClasses::default(),
        list_free: declare(
            "pycc_rt_list_free",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        instance_free: declare(
            "pycc_rt_instance_free",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        dict_incref: declare(
            "pycc_rt_dict_incref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        dict_decref: declare(
            "pycc_rt_dict_decref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        set_incref: declare(
            "pycc_rt_set_incref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        set_decref: declare(
            "pycc_rt_set_decref",
            void_type.fn_type(&[ptr_type.into()], false),
        ),
        obj_set_new: declare("pycc_rt_obj_set_new", ptr_type.fn_type(&[], false)),
        obj_set_discard_candidate: declare(
            "pycc_rt_obj_set_discard_candidate",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
        ),
        rc_class_kinds: std::cell::RefCell::new(HashMap::new()),
        exception_classes: std::cell::RefCell::new(Default::default()),
        class_records: std::cell::RefCell::new(HashMap::new()),
        borrowed_locals: std::cell::RefCell::new(std::collections::HashSet::new()),
        tagged_slots: std::cell::RefCell::new(std::collections::HashSet::new()),
        direct_entered: std::cell::RefCell::new(HashMap::new()),
        owned_int_names: std::cell::RefCell::new(std::collections::HashSet::new()),
        int_ranges: std::cell::RefCell::new(Default::default()),
        loop_list_slots: std::cell::RefCell::new(HashMap::new()),
    }
}
