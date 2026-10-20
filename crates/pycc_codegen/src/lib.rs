use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::FileType;
use inkwell::types::BasicType;
use inkwell::values::{FloatValue, FunctionValue, IntValue, PointerValue};
use pycc_mir::{EXCEPTION_GROUP_TYPE_TAG, MirExceptionValue, MirExpr, MirItem, MirModule, MirStmt};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod exception;
mod fallthrough;
mod frame_ip;
mod frame_needs;
mod frames;
mod global_sync;
mod loop_control;
use exception::{ExceptionCodegenState, expression_can_set_exception, guard_statement_effects};
mod exception_value;
use exception_value::{emit_exception_set_frame, emit_exception_value};
mod attr_slot;
use attr_slot::{release_int_attr_slot_before_store, scalar_to_slot_word, slot_word_to_scalar};
mod assigned_slots;
mod bigint_rc;
/// `<< >> & | ^` operand encoding and the `bool`-result arm (#1210).
mod binop;
/// `and`/`or` short circuit and join (#1211).
mod boolop;
mod call_result;
mod compare;
mod compare_chain;
mod comprehension;
mod copy_slots;
mod if_exp;
mod int_tree;
mod nothrow;
mod num;
use bigint_rc::{
    BigIntRefcount, emit_bigint_refcount_call, pop_pending_int_release,
    push_pending_int_release_if_scalar_temporary, push_pending_int_release_if_temporary,
    release_if_int_temporary, release_int_slot_before_store,
    release_optional_int_slot_before_store, release_scalar_if_int_temporary,
    retain_if_int_duplicate, retain_if_int_duplicate_and_track_for_exception_edge,
};
use comprehension::{CompCx, CompElts, emit_comprehension, emit_comprehension_expr};
mod containers;
mod float_fast;
mod for_range_fast;
mod int_builtins;
mod int_const;
mod int_fast;
mod int_range;
mod int_small;
mod const_container;
mod extend_scratch;
mod leaf_inline;
/// `list[T]` emission over the generic `PyListObj` (`pycc_rt::list`).
mod list;
mod ndarray;
mod tbaa;
mod tuple_index;
use int_const::{emit_int_constant, tag_smallint_const};
mod exception_isinstance;
mod exception_render;
use exception_render::emit_exception_message;
mod borrowed_locals;
mod heap_rc;
mod operand_pin;
mod optional_compare;
mod optional_ptr;
mod str_rc;
use heap_rc::release_scalar_if_temporary;
use str_rc::incref_if_str_duplicate;
mod rt_call;
mod rt_fns;
use rt_fns::{RtFns, declare_rt_functions};
mod closure;
#[cfg(test)]
mod closure_tests;
mod ext;
mod ext_publish;
mod member_symbols;
mod sibling_call;
mod sibling_class;
pub use member_symbols::{MODULE_UNIT_ENTRIES, member_prefix};
pub use object_coerce::EXT_OBJ_COERCE_INSTANCE_SYMBOL;
pub use sibling_call::SiblingTarget;
pub use sibling_class::SiblingClass;
mod ext_thunk;
mod foreign_arith;
mod foreign_attr;
mod foreign_call;
mod foreign_call_emit;
mod foreign_compare;
mod foreign_fail;
mod foreign_import;
mod foreign_len;
mod foreign_pack;
mod foreign_raise;
mod foreign_slice;
mod foreign_store;
mod foreign_type;
mod foreign_unpack;
/// `frozenset(...)` construction and set truthiness (Part 1 of #1319).
mod frozenset;
mod instance_order;
mod generator;
mod generator_box;
mod hash;
mod object_box;
mod object_coerce;
mod tuple_rebox;
mod object_comprehension;
mod native_operand;
mod object_rc;
mod object_release;
mod object_return;
mod object_slot;
mod object_unbox;
mod sequence;
/// Set insertion, length and iteration helpers, and the insert of a set of
/// user-class instances (#1343, Part 1 of #1336).
mod set_instance;
mod slot_epilogue;
use set_instance::{
    SetEmitter, build_int_set_check_not_resized, build_int_set_get, build_int_set_len,
    expect_set_pointer, set_element_scalar,
};

/// One Part 4 conversion emitter in `foreign_len.rs` (PR 4b of #1083).
///
/// Named so `emit_expr`'s `int`/`str` arm can pick between
/// [`foreign_len::emit_to_int`] and [`foreign_len::emit_to_str`] by value
/// instead of duplicating the arity and operand backstops around each. The
/// `for<'a>` binder is what lets one alias serve both, since the emitters
/// are generic over the LLVM context's lifetime rather than over a type.
type ObjectConversionEmitter = for<'a> fn(
    &'a Context,
    &inkwell::builder::Builder<'a>,
    &inkwell::module::Module<'a>,
    &RtFns<'a>,
    Scalar<'a>,
) -> Scalar<'a>;
mod slot_bindings;
mod string_render;
#[cfg(test)]
use slot_bindings::collect_expr_bindings;
use slot_bindings::{collect_module_bindings, collect_stmt_bindings};
mod store_width;
mod target_machine;
pub use ext::{
    CompileOptions, EXT_MODULE_EXEC_FAILED, EXT_MODULE_EXEC_SYMBOL, EXT_OBJ_IMPORT_FROM_SYMBOL,
    EXT_PUBLISH_SYMBOL, EXT_THUNK_PREFIX, body_returns_buffer_slice, buffer_slice_out_names,
    ext_boundary_slots, ext_thunk_out_tys, ext_thunk_param_tys, ext_thunk_required,
    ext_thunk_symbol, is_ext_exportable_name, mangle_ext_name,
};
use ext::{
    EXT_NAME_ERROR_SYMBOL, EXT_OBJ_BUILD_LIST_SYMBOL, EXT_OBJ_CALL_BORROWED_SYMBOL,
    EXT_OBJ_CALL_KW_BORROWED_SYMBOL, EXT_OBJ_CALL_KW_SYMBOL, EXT_OBJ_CALL_SYMBOL,
    EXT_OBJ_COLLECT_PAIR_SYMBOL, EXT_OBJ_COLLECT_SYMBOL, EXT_OBJ_CONTAINS_SYMBOL, EXT_OBJ_DELATTR_SYMBOL,
    EXT_OBJ_DELSLICE_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL, EXT_OBJ_FORMAT_SYMBOL,
    EXT_OBJ_GET_ITER_SYMBOL, EXT_OBJ_GETATTR_SYMBOL, EXT_OBJ_GETITEM_SYMBOL,
    EXT_OBJ_GETSLICE_SYMBOL, EXT_OBJ_IMPORT_SYMBOL, EXT_OBJ_ISINSTANCE_COMPILED_SYMBOL,
    EXT_OBJ_ISINSTANCE_SYMBOL, EXT_OBJ_ITER_NEXT_SYMBOL, EXT_OBJ_LEN_SYMBOL,
    EXT_OBJ_ELLIPSIS_SYMBOL, EXT_OBJ_NEW_COLLECTION_SYMBOL, EXT_OBJ_NONE_SYMBOL,
    EXT_OBJ_NOT_IMPLEMENTED_SYMBOL,
    EXT_OBJ_PACK_BOOL_SYMBOL, EXT_OBJ_PACK_CLOSURE_SYMBOL, EXT_OBJ_PACK_FUNCTION_SYMBOL, EXT_OBJ_PACK_FLOAT_SYMBOL, EXT_OBJ_PACK_FROZENSET_SYMBOL,
    EXT_OBJ_PACK_INSTANCE_SYMBOL,
    EXT_OBJ_PACK_INT_SYMBOL, EXT_OBJ_PACK_LIST_SYMBOL, EXT_OBJ_PACK_OBJECT_SYMBOL,
    EXT_OBJ_PACK_STR_SYMBOL, EXT_OBJ_RAISE_FROM_SYMBOL, EXT_OBJ_RAISE_SYMBOL, EXT_OBJ_REPR_SYMBOL,
    EXT_OBJ_RICHCOMPARE_SYMBOL,
    EXT_OBJ_SETATTR_SYMBOL, EXT_OBJ_SETSLICE_SYMBOL, EXT_OBJ_TO_FLOAT_SYMBOL,
    EXT_OBJ_TO_INT_SYMBOL, EXT_OBJ_TO_STR_SYMBOL, EXT_OBJ_TRUTHY_SYMBOL, EXT_OBJ_TYPE_SYMBOL,
    EXT_OBJ_UNBOX_BOOL_SYMBOL, EXT_OBJ_UNBOX_FLOAT_SYMBOL, EXT_OBJ_UNBOX_INSTANCE_SYMBOL,
    EXT_OBJ_UNBOX_INT_SYMBOL, EXT_OBJ_UNBOX_STR_SYMBOL, EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL,
    ObjCollectionKind, entry_fn_name, is_module_entry_symbol,
};
#[cfg(test)]
mod tests;
pub use pycc_artifact_layout as artifact_layout;

const RELEASE_PASS_PIPELINE: &str = "default<O3>";
type CodegenObserver<'observer> =
    dyn for<'ctx> FnMut(&inkwell::module::Module<'ctx>, Option<&'static str>) + 'observer;

// The crate root's former body, split by concern (mechanical move): value
// representations, coercions, container access, user calls, entry-block
// slots, module globals, the compile driver, and expression/statement
// lowering. Each module re-exports into the root so existing `crate::x` and
// `super::x` paths keep resolving.
mod coerce;
mod compile;
mod emit;
mod split_emit;
mod container_access;
mod entry_slots;
mod expr;
mod globals;
mod scalar;
mod stmt;
mod str_truthy;
mod user_call;
mod multi_out;
use coerce::*;
use compile::*;
pub use compile::{
    compile_to_object, compile_to_object_with_options, compile_to_objects_with_options,
};
use container_access::*;
use entry_slots::*;
use expr::*;
use globals::*;
use scalar::*;
use stmt::*;
use str_truthy::*;
use user_call::*;
