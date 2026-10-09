//! Class instance runtime object (D-154, Part 1 of #375; see the
//! class-instance-layout ADR): `PyInstanceObj` and its `extern "C"`
//! accessor functions.
//!
//! Follows the exact opaque-heap-object-with-FFI-accessors shape every
//! other `pycc_rt` container object (`PyStrObj`, `PyIntListObj`,
//! `PyDictObj`, `PyIntSetObj`) already uses: `PyInstanceObj` is `pub` only
//! so it can appear in an `extern "C"` function signature (the
//! `private_interfaces` lint, a hard error under this workspace's `-D
//! warnings` clippy gate, refuses a private type there) -- its field stays
//! private, so nothing outside this module can construct one or read its
//! contents except through the functions below. It is deliberately **not**
//! `#[repr(C)]`: `pycc_codegen` never reads or writes a slot by computing a
//! field offset itself (no LLVM `GEP` against this struct anywhere), only
//! by calling [`pycc_rt_instance_get_slot_checked`] (or the unchecked
//! [`pycc_rt_instance_get_slot`]) and [`pycc_rt_instance_set_slot`] with a
//! slot index `pycc_mir` already resolved at compile time against the
//! class's declared attribute list -- exactly the ADR's own recommended
//! fork (opaque-with-accessors, not a cross-crate ABI layout commitment).
//!
//! **Slot representation.** Every assigned slot holds a plain `i64` word,
//! matching this crate's own existing `PyIntListObj` element
//! representation; since #1388 the slot is an `Option<i64>` whose `None`
//! marks "not yet assigned" (16 bytes per slot instead of 8, see
//! [`PyInstanceObj`]). `int`/`bool`
//! attributes store their value directly; a `float` attribute stores its
//! `f64::to_bits()` bit pattern (`pycc_codegen` bitcasts around the call,
//! there is no separate float-typed accessor pair to keep this module's own
//! FFI surface minimal); a heap-object-typed attribute (`str`, or another
//! class instance, or a leak-only `list[int]`/`dict[str, int]` container
//! since #1262) stores its pointer, reinterpreted as an `i64` (valid on
//! every target this workspace compiles for, where a pointer and `i64` are
//! both 8 bytes) -- `pycc_codegen` handles the `inttoptr`/`ptrtoint`
//! conversion around the call, mirroring the float bitcast.
//!
//! **No reference counting.** Per the class-instance-layout ADR and this
//! issue's own explicit scope ("no new garbage-collection/reference-
//! counting design beyond what `pycc_rt`'s existing heap objects already
//! do"): `PyIntListObj`/`PyDictObj`/`PyIntSetObj` all define an `incref`/
//! `decref` pair, but `pycc_codegen` never actually calls any of them (D-107,
//! confirmed directly in `pycc_codegen`'s own doc comments) -- every
//! container value already leaks in practice, refcounting exists only as
//! unused-but-present infrastructure. `PyInstanceObj` does not even define
//! that unused pair, and deliberately carries no `rc` field the way
//! `PyIntListObj`/`PyDictObj`/`PyIntSetObj` do: an `rc` field with no
//! `incref`/`decref` pair ever reading or writing it past construction is
//! dead code (`-D warnings` rejects it outright), not merely unused-but-
//! harmless infrastructure -- so matching the *effective* (not aspirational)
//! behavior of every other heap object here means omitting the field
//! entirely, not carrying an inert placeholder. A future PR that wires up
//! real project-wide refcounting adds `rc` back alongside its own
//! `incref`/`decref` pair, exactly as it must add the same to every other
//! heap object in this file.

use std::cell::Cell;
use std::ffi::c_void;

use crate::exception::{EXCEPTION_TYPE_ATTRIBUTE_ERROR, raise_builtin};

/// See this module's own doc comment for the opacity/layout and
/// no-refcounting rationale.
///
/// **Unassigned slots (#1388).** A slot is `None` until its first
/// [`pycc_rt_instance_set_slot`]. No sentinel word could mark that state,
/// because `0` is a valid `int`, `bool` and `float` word. A slot can be read
/// before it is assigned -- `self.a = self.b` in `__init__`, a helper method
/// `__init__` calls before its assignments, or a derived `__init__` that
/// never assigns a base slot (#1148) -- and CPython raises `AttributeError`
/// there, so every compiled `base.attr` read goes through
/// [`pycc_rt_instance_get_slot_checked`], which raises it too.
///
/// `layout` is the static descriptor the constructor call site passes (see
/// [`pycc_rt_instance_new`]); it is read only to word that error and, since
/// #1435, to name the instance's class to an `--ext` host
/// ([`pycc_rt_ext_instance_class`]).
///
/// `carrier` is the `--ext` host-side object currently standing for this
/// instance, or null (#1435). It is opaque here: `pycc_rt` keeps no CPython
/// dependency (D-244 rule 2), and only the C shim reads or writes it, through
/// [`pycc_rt_ext_instance_carrier`] and [`pycc_rt_ext_instance_set_carrier`].
pub struct PyInstanceObj {
    slots: Cell<Vec<Option<i64>>>,
    layout: &'static [u8],
    carrier: Cell<*mut c_void>,
}

mod carrier;
pub use carrier::{
    pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_class, pycc_rt_ext_instance_set_carrier,
};
mod copy;
pub use copy::pycc_rt_ext_instance_copy;
mod store;
pub use store::{pycc_rt_ext_instance_delete_slot, pycc_rt_ext_instance_store_slot};

/// Allocates a fresh instance with `slot_count` unassigned slots. A negative
/// `slot_count` is an internal-error panic (impossible from real
/// `pycc_codegen` output, which only ever emits a class's own non-negative
/// declared attribute count as a compile-time constant), held in this
/// private function so the panic-across-FFI split matches every other
/// bounds check in this module (see `read_slot`'s own doc comment).
fn new_instance(slot_count: i64, layout: &'static [u8]) -> PyInstanceObj {
    let slot_count = usize::try_from(slot_count).unwrap_or_else(|_| {
        panic!("pycc_rt: internal error: negative instance slot count {slot_count}")
    });
    PyInstanceObj {
        slots: Cell::new(vec![None; slot_count]),
        layout,
        carrier: Cell::new(std::ptr::null_mut()),
    }
}

/// Allocates an instance of a class with `slot_count` slots, all
/// unassigned.
///
/// `layout_ptr`/`layout_len` is the class's static layout descriptor
/// (#1388): the class's source name followed by the name of each slot in
/// slot order, separated by NUL bytes (`C\0x\0y` for a class `C` whose
/// slots are `x` then `y`). `pycc_codegen` interns it once per class as a
/// private constant, so it outlives every instance. A null `layout_ptr` is
/// an empty descriptor.
///
/// # Safety
/// `slot_count` must be non-negative -- true for every `pycc_codegen` call
/// site, which always passes a compile-time-constant, non-negative class
/// attribute count. `layout_ptr` must be null or point at `layout_len`
/// bytes that live for the rest of the process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_new(
    slot_count: i64,
    layout_ptr: *const u8,
    layout_len: usize,
) -> *mut PyInstanceObj {
    let layout: &'static [u8] = if layout_ptr.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(layout_ptr, layout_len) }
    };
    Box::into_raw(Box::new(new_instance(slot_count, layout)))
}

/// # Safety (panic-across-FFI note, same rationale as `pycc_rt_int_add`'s
/// own doc comment)
/// The getters below are plain `extern "C" fn`s, not `extern "C-unwind"`.
/// A panic that would otherwise unwind past their boundary is instead
/// turned into a process abort -- correct for pycc-generated LLVM code
/// calling them (an out-of-range `slot` is impossible from a
/// `pycc_types`-checked program, since every `AttrGet`/`AttrSet` slot index
/// is resolved at compile time against the class's own declared attribute
/// count), but unsuitable for a `#[should_panic]` test directly against a
/// public wrapper (confirmed empirically the same way
/// `pycc_rt_float_to_str`'s own split was: see this module's own tests
/// below). This private function holds the real, freely-panicking logic.
/// Takes `slot` as the raw `i64` the FFI boundary receives, exactly like
/// `int_list_get`'s own `index: i64` -- a negative `slot` and an
/// out-of-range `slot` are both "index out of range" here, matching that
/// function's own combined bounds check, not two separate failure modes.
/// Returns `None` for a slot not yet assigned.
fn read_slot(instance: &PyInstanceObj, slot: i64) -> Option<i64> {
    let slots = instance.slots.take();
    if slot < 0 || slot as usize >= slots.len() {
        let len = slots.len();
        instance.slots.set(slots);
        panic!(
            "pycc_rt: internal error: instance slot {slot} out of range (instance has {len} \
             slots) -- pycc_mir should have resolved a slot index within the class's own \
             declared attribute count"
        );
    }
    let value = slots[slot as usize];
    instance.slots.set(slots);
    value
}

/// The unchecked read: an unassigned slot reads as the word `0`.
fn instance_get_slot(instance: &PyInstanceObj, slot: i64) -> i64 {
    read_slot(instance, slot).unwrap_or(0)
}

/// Reads slot `slot`'s raw `i64` word, `0` when it is not yet assigned --
/// see this module's own doc comment for what that word means for each
/// attribute type.
///
/// This is not the read a compiled `base.attr` performs (that is
/// [`pycc_rt_instance_get_slot_checked`]). Its callers are the stores that
/// release a slot's old value before overwriting it, for which "nothing to
/// release" and the `0` word mean the same thing.
///
/// # Safety
/// `instance` must be a live `PyInstanceObj` pointer -- every `pycc_codegen`
/// call site only ever passes a value it just evaluated from a
/// well-typed `Ty::Instance` expression.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_get_slot(instance: *mut PyInstanceObj, slot: i64) -> i64 {
    instance_get_slot(unsafe { &*instance }, slot)
}

/// The NUL-separated field `index` of `layout` (`0` is the class name,
/// `1 + slot` a slot name), or `"?"` when the descriptor has no such
/// field -- never the case for a descriptor `pycc_codegen` emitted.
fn layout_field(layout: &[u8], index: usize) -> String {
    layout
        .split(|byte| *byte == 0)
        .nth(index)
        .filter(|field| !field.is_empty())
        .map_or_else(
            || "?".to_string(),
            |field| String::from_utf8_lossy(field).into_owned(),
        )
}

/// The checked read (#1388): an assigned slot reads as its word; an
/// unassigned one raises CPython's ``AttributeError: '<C>' object has no
/// attribute '<x>'`` -- `<C>` the instance's own class, which for an
/// instance of a subclass is the subclass, as in CPython -- and returns `0`,
/// which every slot type tolerates on the path that unwinds to the handler,
/// exactly as `int_list_get` does for `IndexError`.
fn instance_get_slot_checked(instance: &PyInstanceObj, slot: i64) -> i64 {
    if let Some(word) = read_slot(instance, slot) {
        return word;
    }
    let class_name = layout_field(instance.layout, 0);
    let attr = layout_field(instance.layout, 1 + slot as usize);
    raise_builtin(
        EXCEPTION_TYPE_ATTRIBUTE_ERROR,
        "AttributeError",
        &format!("'{class_name}' object has no attribute '{attr}'"),
    );
    0
}

/// The read every compiled `base.attr` performs; see
/// [`instance_get_slot_checked`]. `pycc_codegen` follows the call with the
/// pending-exception check it emits after any operation that can raise.
///
/// # Safety
/// Same requirement as [`pycc_rt_instance_get_slot`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_get_slot_checked(
    instance: *mut PyInstanceObj,
    slot: i64,
) -> i64 {
    instance_get_slot_checked(unsafe { &*instance }, slot)
}

/// Writes `value` into slot `slot`, overwriting whatever was there and
/// marking the slot assigned -- mirrors [`read_slot`]'s own
/// panic-across-FFI split and out-of-range reasoning.
fn instance_set_slot(instance: &PyInstanceObj, slot: i64, value: i64) {
    let mut slots = instance.slots.take();
    if slot < 0 || slot as usize >= slots.len() {
        let len = slots.len();
        instance.slots.set(slots);
        panic!(
            "pycc_rt: internal error: instance slot {slot} out of range (instance has {len} \
             slots) -- pycc_mir should have resolved a slot index within the class's own \
             declared attribute count"
        );
    }
    slots[slot as usize] = Some(value);
    instance.slots.set(slots);
}

/// # Safety
/// `instance` must be a live `PyInstanceObj` pointer, same requirement as
/// [`pycc_rt_instance_get_slot`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_set_slot(
    instance: *mut PyInstanceObj,
    slot: i64,
    value: i64,
) {
    instance_set_slot(unsafe { &*instance }, slot, value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::pending_tag_and_message;

    #[test]
    fn a_fresh_instance_reads_every_slot_as_zero_unchecked() {
        let instance = unsafe { &*pycc_rt_instance_new(3, std::ptr::null(), 0) };
        assert_eq!(instance_get_slot(instance, 0), 0);
        assert_eq!(instance_get_slot(instance, 1), 0);
        assert_eq!(instance_get_slot(instance, 2), 0);
    }

    #[test]
    fn setting_a_slot_is_observable_by_a_later_get() {
        let instance = unsafe { &*pycc_rt_instance_new(2, std::ptr::null(), 0) };
        instance_set_slot(instance, 0, 42);
        instance_set_slot(instance, 1, -7);
        assert_eq!(instance_get_slot(instance, 0), 42);
        assert_eq!(instance_get_slot(instance, 1), -7);
    }

    #[test]
    fn setting_one_slot_does_not_disturb_another() {
        let instance = unsafe { &*pycc_rt_instance_new(2, std::ptr::null(), 0) };
        instance_set_slot(instance, 0, 1);
        instance_set_slot(instance, 1, 2);
        instance_set_slot(instance, 0, 99);
        assert_eq!(instance_get_slot(instance, 0), 99);
        assert_eq!(instance_get_slot(instance, 1), 2);
    }

    #[test]
    fn a_zero_slot_instance_allocates_without_panicking() {
        // A class could in principle declare no attributes at all (not
        // reachable from this PR's own HIR lowering, which requires
        // `__init__`, but not that it assign any attribute) -- confirm the
        // degenerate zero-slot case is not an internal-error trap.
        let instance = unsafe { &*pycc_rt_instance_new(0, std::ptr::null(), 0) };
        let slots = instance.slots.take();
        assert!(slots.is_empty());
        assert!(instance.layout.is_empty());
        instance.slots.set(slots);
    }

    #[test]
    #[should_panic(expected = "instance slot 5 out of range")]
    fn reading_an_out_of_range_slot_panics_with_an_internal_error() {
        let instance = unsafe { &*pycc_rt_instance_new(2, std::ptr::null(), 0) };
        instance_get_slot(instance, 5);
    }

    #[test]
    #[should_panic(expected = "instance slot 5 out of range")]
    fn writing_an_out_of_range_slot_panics_with_an_internal_error() {
        let instance = unsafe { &*pycc_rt_instance_new(2, std::ptr::null(), 0) };
        instance_set_slot(instance, 5, 1);
    }

    #[test]
    fn the_public_get_wrapper_reads_a_slot_through_the_ffi_boundary() {
        let ptr = unsafe { pycc_rt_instance_new(1, std::ptr::null(), 0) };
        unsafe { pycc_rt_instance_set_slot(ptr, 0, 123) };
        assert_eq!(unsafe { pycc_rt_instance_get_slot(ptr, 0) }, 123);
    }

    #[test]
    #[should_panic(expected = "negative instance slot count")]
    fn a_negative_slot_count_panics_with_an_internal_error() {
        // Calls the private `new_instance` directly, not the public
        // `pycc_rt_instance_new` FFI wrapper -- that wrapper is a plain
        // `extern "C" fn`, not `extern "C-unwind"`, so a panic propagating
        // through it aborts the process instead of unwinding, the same
        // panic-across-FFI reasoning `instance_get_slot`'s own doc comment
        // documents (confirmed empirically: an earlier version of this test
        // called the public wrapper and aborted the whole test process
        // instead of being caught by `#[should_panic]`).
        new_instance(-1, &[]);
    }

    #[test]
    #[should_panic(expected = "instance slot -1 out of range")]
    fn a_negative_get_slot_index_panics_with_an_internal_error() {
        let instance = unsafe { &*pycc_rt_instance_new(1, std::ptr::null(), 0) };
        instance_get_slot(instance, -1);
    }

    #[test]
    #[should_panic(expected = "instance slot -1 out of range")]
    fn a_negative_set_slot_index_panics_with_an_internal_error() {
        let instance = unsafe { &*pycc_rt_instance_new(1, std::ptr::null(), 0) };
        instance_set_slot(instance, -1, 0);
    }

    /// #1388: the checked read of an unassigned slot raises CPython's
    /// `AttributeError`, naming the class and the slot from the layout
    /// descriptor, and returns the word `0`.
    #[test]
    fn a_checked_read_of_an_unassigned_slot_raises_attribute_error() {
        const LAYOUT: &[u8] = b"Point\0x\0y";
        crate::pycc_rt_exception_clear();
        let ptr = unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) };
        unsafe { pycc_rt_instance_set_slot(ptr, 0, 5) };
        assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(ptr, 0) }, 5);
        assert_eq!(crate::pycc_rt_exception_active(), 0);
        assert_eq!(unsafe { pycc_rt_instance_get_slot_checked(ptr, 1) }, 0);
        let (tag, message) = pending_tag_and_message();
        assert_eq!(tag, EXCEPTION_TYPE_ATTRIBUTE_ERROR);
        assert_eq!(message, "'Point' object has no attribute 'y'");
        crate::pycc_rt_exception_clear();
    }

    /// #1388: an assigned `0` word -- `0`, `False`, `0.0`'s bits -- is a
    /// value, not the unassigned state.
    #[test]
    fn an_assigned_zero_word_is_not_unassigned() {
        crate::pycc_rt_exception_clear();
        let instance = unsafe { &*pycc_rt_instance_new(1, std::ptr::null(), 0) };
        instance_set_slot(instance, 0, 0);
        assert_eq!(instance_get_slot_checked(instance, 0), 0);
        assert_eq!(crate::pycc_rt_exception_active(), 0);
    }

    /// #1388: a descriptor too short for the slot (never one `pycc_codegen`
    /// emits) still words the error, with `?` for each missing field.
    #[test]
    fn a_missing_descriptor_field_reads_as_a_question_mark() {
        crate::pycc_rt_exception_clear();
        let instance = unsafe { &*pycc_rt_instance_new(1, std::ptr::null(), 0) };
        instance_get_slot_checked(instance, 0);
        let (_, message) = pending_tag_and_message();
        assert_eq!(message, "'?' object has no attribute '?'");
        crate::pycc_rt_exception_clear();
        assert_eq!(layout_field(b"C\0\0z", 1), "?");
        assert_eq!(layout_field(b"C\0\0z", 2), "z");
    }
}
