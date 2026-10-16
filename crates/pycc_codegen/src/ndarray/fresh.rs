//! The freshness of typed array header snapshots ([`super::NdCache`]):
//! which calls may run CPython code, and the pass that marks every
//! snapshot stale after one.

use super::*;

/// Clears every [`NdCache::fresh`] of a function after each call in it that
/// may run CPython code: everything but an LLVM intrinsic other than a
/// coroutine suspension (a generator's consumer runs while it is
/// suspended) and the helpers of [`PYTHON_FREE_CALLS`]. Run once the module
/// is fully emitted.
pub(crate) fn clear_fresh_after_calls<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let mut by_function: HashMap<String, Vec<PointerValue<'ctx>>> = HashMap::new();
    for ((function, _), cache) in rt.nd_caches.0.borrow().iter() {
        by_function
            .entry(function.clone())
            .or_default()
            .push(cache.fresh);
    }
    let stale = context.bool_type().const_zero();
    for (name, flags) in by_function {
        let Some(function) = module.get_function(&name) else {
            continue;
        };
        let mut calls = Vec::new();
        for block in function.get_basic_blocks() {
            let mut next = block.get_first_instruction();
            while let Some(instruction) = next {
                next = instruction.get_next_instruction();
                if instruction.get_opcode() == inkwell::values::InstructionOpcode::Call
                    && may_run_python(instruction)
                    && let Some(after) = instruction.get_next_instruction()
                {
                    calls.push(after);
                }
            }
        }
        for after in calls {
            builder.position_before(&after);
            for flag in &flags {
                builder
                    .build_store(*flag, stale)
                    .expect("build_store should not fail");
            }
        }
    }
}

/// The callees that provably run no CPython code, so a call to one keeps
/// every [`NdCache`] fresh. An explicit allowlist: anything not named here
/// -- including every other `pycc_rt_*` function, some of which reach
/// CPython through the hooks the extension shim installs (an object
/// release that runs `__del__`, a write to a replaced `sys.stdout`) -- is
/// assumed to run CPython code.
///
/// - pycc_rt's `int` and `float` arithmetic: pure Rust on tagged words and
///   pycc's own heap bigints (`pycc_rt` imports no C function and these
///   reach no installed hook); errors set pycc_rt's exception flag.
/// - `pycc_ext_nd_index_error`, `pycc_ext_nd_readonly`: `PyErr_Format` /
///   `PyErr_SetString` with C integers and literals only.
/// - `pycc_ext_nd_index_word`: classifies the word (pycc_rt) and sets
///   `IndexError`; never calls `__index__` -- an index is a pycc `int` word.
/// - `pycc_ext_nd_int_word`: classifies the word; a bigint is rebuilt as an
///   exact `int` (`PyLong_FromString`) and read by `PyLong_AsLongLong`,
///   which calls `__index__` only for a non-`int`; `int` objects are not
///   GC-tracked, so neither allocating nor freeing one can start a
///   collection (and with it a finalizer).
///
/// - `pycc_ext_nd_check`, on success: it runs CPython code (imports numpy,
///   reads the Python attributes of an array that may be a subclass with
///   properties) only until it has validated numpy's layout once, and no
///   snapshot can be fresh before that -- a snapshot is taken only after a
///   check of this same module succeeded, which needs the validation, and
///   the validation is never undone. After it, a successful check is a type
///   test along the MRO, raw field reads and numpy's C dtype comparison.
///   On failure it may run CPython code (the message formats the dtype's
///   `repr`), but every failure leaves through the foreign failure edge,
///   whose `pycc_ext_obj_error_bridge` call is not listed here and so
///   marks every snapshot stale before anything else runs.
/// - `pycc_ext_ba_check`: `PyByteArray_CheckExact`, `PyByteArray_AsString`
///   and `PyByteArray_Size` on success, none of which runs Python code;
///   its failure formats the type's name and leaves through the failure
///   edge, as above.
/// - `pycc_ext_ba_index_error`, `pycc_ext_ba_index_word`,
///   `pycc_ext_ba_byte_error`: `PyErr_SetString` with a literal only.
pub(super) const PYTHON_FREE_CALLS: &[&str] = &[
    "pycc_rt_int_add",
    "pycc_rt_int_sub",
    "pycc_rt_int_mul",
    "pycc_rt_int_floordiv",
    "pycc_rt_int_floormod",
    "pycc_rt_int_mod_noraise",
    "pycc_rt_int_truediv",
    "pycc_rt_int_pow",
    "pycc_rt_int_powmod",
    "pycc_rt_int_abs",
    "pycc_rt_int_and",
    "pycc_rt_int_or",
    "pycc_rt_int_xor",
    "pycc_rt_int_lshift",
    "pycc_rt_int_rshift",
    "pycc_rt_int_cmp",
    "pycc_rt_int_float_cmp",
    "pycc_rt_int_min2",
    "pycc_rt_int_max2",
    "pycc_rt_int_truthy",
    "pycc_rt_int_untag_checked",
    "pycc_rt_int_from_i64",
    "pycc_rt_int_from_float",
    "pycc_rt_int_to_float",
    "pycc_rt_bigint_retain",
    "pycc_rt_bigint_release",
    "pycc_rt_float_div",
    "pycc_rt_float_floordiv",
    "pycc_rt_float_floormod",
    "pycc_rt_float_pow",
    "pycc_rt_float_round",
    "pycc_rt_float_round_to_int",
    "pycc_rt_float_min2",
    "pycc_rt_float_max2",
    ND_CHECK_SYMBOL,
    ND_INDEX_ERROR_SYMBOL,
    ND_INDEX_WORD_SYMBOL,
    ND_INT_WORD_SYMBOL,
    ND_READONLY_SYMBOL,
    BA_CHECK_SYMBOL,
    BA_INDEX_ERROR_SYMBOL,
    BA_INDEX_WORD_SYMBOL,
    BA_BYTE_ERROR_SYMBOL,
];

/// Whether `call` may run CPython code ([`clear_fresh_after_calls`]).
fn may_run_python(call: inkwell::values::InstructionValue<'_>) -> bool {
    let Ok(site) = inkwell::values::CallSiteValue::try_from(call) else {
        return true;
    };
    let Some(callee) = site.get_called_fn_value() else {
        return true;
    };
    callee_may_run_python(&callee.get_name().to_string_lossy())
}

/// Whether a call to the function named `name` may run CPython code.
fn callee_may_run_python(name: &str) -> bool {
    if name.starts_with("llvm.") {
        return name.starts_with("llvm.coro.suspend");
    }
    !PYTHON_FREE_CALLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::callee_may_run_python;

    #[test]
    fn number_arithmetic_and_the_raising_helpers_keep_snapshots_fresh() {
        for name in [
            "pycc_rt_int_add",
            "pycc_rt_bigint_release",
            "pycc_rt_float_pow",
            "pycc_ext_nd_check",
            "pycc_ext_nd_index_error",
            "pycc_ext_nd_index_word",
            "pycc_ext_nd_int_word",
            "pycc_ext_nd_readonly",
            "pycc_ext_ba_check",
            "pycc_ext_ba_index_error",
            "pycc_ext_ba_index_word",
            "pycc_ext_ba_byte_error",
            "llvm.sadd.with.overflow.i64",
        ] {
            assert!(!callee_may_run_python(name), "{name}");
        }
    }

    /// A helper merely named like a safe one is not assumed safe, nor is
    /// the error bridge every failure edge calls.
    #[test]
    fn the_check_and_unlisted_helpers_may_run_python() {
        for name in [
            "pycc_ext_obj_error_bridge",
            "pycc_ext_nd_future_helper",
            "pycc_rt_int_print",
            "pycc_rt_int_list_release",
            "pycc_rt_float_to_str",
            "pycc_rt_list_release",
            "llvm.coro.suspend",
            "user_function",
        ] {
            assert!(callee_may_run_python(name), "{name}");
        }
    }
}
