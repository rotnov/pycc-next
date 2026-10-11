//! # Build order: this crate's staticlib is not a normal Cargo dependency
//!
//! This crate's real consumer is not Rust code but pycc-generated object
//! files, which reference its `pycc_rt_*` symbols by name and are
//! linked against this crate's `staticlib` output (`libpycc_rt.a` on
//! Unix-like targets, `pycc_rt.lib` on `-msvc` targets -- see D-028) via a
//! linker-driver invocation (`cc`, or on Windows the bundled `clang` --
//! see D-028) -- in `pycc_codegen`'s own tests (`link_object_with_runtime`)
//! and in `pycc`'s real `build`/`run` (`src/main.rs`). Nothing in Cargo's
//! normal dependency graph expresses that relationship: Cargo does not
//! uplift a `staticlib` reached through an ordinary `[dependencies]` edge
//! to a predictable path, and `cargo test` does not build the `staticlib`
//! crate type at all. So neither `cargo build --workspace` nor
//! `cargo test --workspace` can be relied on to leave the archive where
//! anything looks for it.
//!
//! **This is now handled automatically.** `pycc_codegen`'s build script
//! (`crates/pycc_codegen/build.rs`) runs `cargo build --locked -p pycc_rt`
//! for *both* host profiles into a private directory under its own
//! `OUT_DIR`, then installs the resulting archive at `<target-root>/debug/`
//! and `<target-root>/release/` -- the directories
//! `pycc_artifact_layout::find_pycc_rt_lib_dir_in` searches. Because every
//! path into the compiler goes through `pycc_codegen`, a clean checkout
//! builds and tests with no manual step: `cargo build -p pycc_rt` is no
//! longer a prerequisite for `cargo test -p pycc_codegen` or for
//! `cargo run --bin pycc -- build ...`. See
//! `docs/decisions/D-184-build-pycc-rt-from-pycc-codegen-s-build.md`.
//!
//! The deadlock that made an earlier attempt at this unworkable was real
//! and is avoided rather than wished away: a build script that invokes
//! `cargo` at the *same* build directory blocks forever on Cargo's own
//! build-directory lock, which the outer invocation holds for the whole
//! build-script execution. The nested build therefore gets its own
//! `--target-dir` under `OUT_DIR`. This crate declaring no dependencies at
//! all is what keeps that nested build cheap and keeps it from contending
//! for `$CARGO_HOME/.package-cache`; `tests/issue_630_pycc_rt_build_dependency.rs`
//! asserts that property so it cannot be lost silently.
//!
//! Cross-compilation is still explicit. When `pycc build --target <triple>`
//! names a triple this workspace was not itself built for, run
//! `rustup target add <triple>` and
//! `cargo build [--release] --target <triple> -p pycc_rt` first; the build
//! script only produces the host's archives, and the diagnostic says so.

use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

pub mod container_ops;
pub mod dict;
mod exception;
pub mod generator;
/// D-244 rule 2's `PyObject*` boundary, runtime half (#1025/#1028).
pub mod ext_bridge;
/// `hash()` of `int`, `bool` and int/bool tuples (#1331).
mod hash;
mod instance;
/// Arbitrary-precision `int` arithmetic, comparison and conversion.
mod int_arith;
/// `<< >> & | ^` over encoded ints (#1210).
mod int_bitwise;
mod int_encoding;
/// `set[int]`/`frozenset[int]`'s `PyIntSetObj` (D-121, Part 1 of #1319).
mod int_set;
/// Standard input and output, shared with CPython's `sys` streams when an
/// interpreter is present.
mod io;
/// `list[T]`'s one generic runtime object, `PyListObj`.
mod list;
/// The float functions of `math`.
mod math_float;
/// The integer functions of `math`.
mod math_ops;
/// `print`'s stdout flush (#1340); the other print primitives stay below.
mod print;
mod rc;
/// The `round()` builtin over `float`.
mod round;
pub mod set;
mod str_hash;
/// `str` builtins and methods, and `input()`.
mod str_ops;
pub mod tuple;
pub mod value;

// The crate root's former body, split by concern (mechanical move); every
// `#[no_mangle]` symbol keeps its name.
mod buffer;
mod float_arith;
mod range;
mod str_core;
pub use buffer::*;
pub use float_arith::*;
pub use range::*;
pub use print::{
    pycc_rt_print_newline, pycc_rt_print_none, pycc_rt_print_space, pycc_rt_print_write_str,
};
pub use str_core::*;

#[cfg(not(test))]
pub use exception::pycc_rt_exception_print_and_exit;
use exception::raise_builtin;
pub use exception::{
    EXCEPTION_TYPE_ATTRIBUTE_ERROR, EXCEPTION_TYPE_EXCEPTION, EXCEPTION_TYPE_FOREIGN_BASE,
    EXCEPTION_TYPE_INDEX_ERROR, EXCEPTION_TYPE_KEY_ERROR, EXCEPTION_TYPE_OVERFLOW_ERROR,
    EXCEPTION_TYPE_RUNTIME_ERROR, EXCEPTION_TYPE_TYPE_ERROR, EXCEPTION_TYPE_VALUE_ERROR,
    EXCEPTION_TYPE_ZERO_DIV_ERROR, PyExceptionObj, pycc_rt_exception_active,
    pycc_rt_exception_alloc, pycc_rt_exception_clear, pycc_rt_exception_group_alloc,
    pycc_rt_exception_group_partition, pycc_rt_exception_message, pycc_rt_exception_raise,
    pycc_rt_exception_raise_with_cause, pycc_rt_exception_type_matches,
    pycc_rt_ext_pending_message, pycc_rt_ext_pending_type,
};
pub use hash::{pycc_rt_hash_int, pycc_rt_hash_pointer, pycc_rt_hash_slot_int, pycc_rt_hash_tuple};
pub use instance::{
    PyInstanceObj, pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_class,
    pycc_rt_ext_instance_copy, pycc_rt_ext_instance_delete_slot, pycc_rt_ext_instance_set_carrier,
    pycc_rt_ext_instance_store_slot, pycc_rt_instance_get_slot, pycc_rt_instance_get_slot_checked,
    pycc_rt_instance_new, pycc_rt_instance_set_slot,
};
pub use int_bitwise::{
    pycc_rt_int_and, pycc_rt_int_lshift, pycc_rt_int_or, pycc_rt_int_rshift, pycc_rt_int_xor,
};
// D-061/D-141's one-word `int` encoding and its heap bigint representation.
// Glob-imported so the operations below -- and their `#[cfg(test)]` tests,
// which reach them through `use super::*` -- keep referring to these names
// unqualified, exactly as when they lived in this file.
pub use dict::{PyDictObj, pycc_rt_dict_decref, pycc_rt_dict_incref};
pub use int_arith::*;
use int_encoding::*;
pub use int_set::{
    PyIntSetObj, pycc_rt_int_set_copy, pycc_rt_int_set_new, pycc_rt_obj_set_add_identity,
    pycc_rt_obj_set_candidate, pycc_rt_obj_set_discard_candidate, pycc_rt_obj_set_new,
    pycc_rt_obj_set_push,
};
pub use list::{
    LIST_KIND_BOOL, LIST_KIND_DICT, LIST_KIND_FLOAT, LIST_KIND_INSTANCE, LIST_KIND_INT,
    LIST_KIND_LIST, LIST_KIND_SET, LIST_KIND_STR, LIST_KIND_TUPLE, LIST_KIND_UNCOUNTED_INSTANCE,
    PyListObj, SLICE_HAS_START, SLICE_HAS_STEP, SLICE_HAS_STOP, pycc_rt_list_append,
    pycc_rt_list_concat, pycc_rt_list_contains, pycc_rt_list_get, pycc_rt_list_index_error,
    pycc_rt_list_insert, pycc_rt_list_len, pycc_rt_list_new, pycc_rt_list_pop, pycc_rt_list_repeat,
    pycc_rt_list_repr, pycc_rt_list_set, pycc_rt_list_slice,
};
pub use print::pycc_rt_print_flush;
pub use rc::{
    RC_OFFSET, pycc_rt_instance_decref, pycc_rt_instance_free, pycc_rt_instance_incref,
    pycc_rt_instance_live_objects, pycc_rt_list_decref, pycc_rt_list_free, pycc_rt_list_incref,
    pycc_rt_list_live_objects,
};
pub use round::{pycc_rt_float_round, pycc_rt_float_round_to_int};
pub use set::{PySetObj, pycc_rt_set_decref, pycc_rt_set_incref};

/// #146 Part 1's refcounting for heap bigints: adds one live reference to
/// the D-141 encoded int word `word`.
///
/// A thin `extern "C"` wrapper over `int_encoding::bigint_retain`, split
/// exactly like `pycc_rt_range_continue`/`range_continue`: an `extern "C"`
/// function may not unwind, so the classification panic on an invalid word
/// lives in the private function, which unit tests can call directly.
///
/// Word `0` and every inline kind (smallints, the two bool-identity
/// markers) are no-ops. `pycc_codegen` additionally emits an inline
/// `(word & 3) == 0 && word != 0` test before every call, so an ordinary
/// smallint loop never reaches this symbol at all -- that guard is what
/// keeps D-084/D-140's nbody throughput floor untouched by this change.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bigint_retain(word: i64) {
    bigint_retain(word);
}

/// #146 Part 1's refcounting for heap bigints: drops one live reference
/// from `word`, freeing the `BigIntObj` when the last one goes away.
/// Same wrapper split, same no-op cases, and the same inline codegen guard
/// as `pycc_rt_bigint_retain` above.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_bigint_release(word: i64) {
    bigint_release(word);
}

/// Runtime NameError for call-before-`def` (issue #22). pycc's type checker
/// rejects a call to a not-yet-`def`ined function in top-level code statically,
/// but a function body may call a sibling whose `def` has not executed yet at
/// the time the caller is invoked -- CPython raises `NameError` at that point,
/// and so does this. Until v0.3's exception machinery exists, a runtime panic
/// becomes an explicit process failure at the plain-C ABI boundary, matching
/// every other runtime error in this crate.
///
/// Takes a null-terminated C string (the function name) so the codegen can
/// pass a global string constant directly. The panic-across-FFI note on
/// `pycc_rt_int_add` applies: a panic that would unwind past this `extern "C"`
/// boundary is caught and turned into a process abort by Rust's default
/// panic-over-FFI behavior, which is exactly the desired outcome for a
/// runtime NameError in a compiled program.
///
/// Split into a private freely-panicking function (testable with
/// `#[should_panic]`) and a thin `extern "C"` wrapper, matching the same
/// split every other panicking runtime function in this file uses.
fn name_error(name: *const std::os::raw::c_char) -> ! {
    let name_str = if name.is_null() {
        "<unknown>"
    } else {
        unsafe { std::ffi::CStr::from_ptr(name) }
            .to_str()
            .unwrap_or("<invalid utf-8>")
    };
    panic!("pycc_rt: NameError: name '{name_str}' is not defined");
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_name_error(name: *const std::os::raw::c_char) -> ! {
    name_error(name)
}

#[cfg(test)]
mod tests;
