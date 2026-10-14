//! Class instance runtime object (D-154; see the class-instance-layout ADR):
//! `PyInstanceObj` and its `extern "C"` accessor functions.
//!
//! **Layout (a fixed ABI shared with `pycc_codegen`).** An instance is one
//! allocation: a `#[repr(C)]` header of five words followed inline by
//! `slot_count` [`InstanceSlot`]s of 16 bytes each -- the slot's `i64` word,
//! then an `i64` "assigned" flag (`0` until the first store, #1388).
//! Compiled code reads and writes a slot directly, at
//! [`INSTANCE_SLOTS_OFFSET`] `+` [`INSTANCE_SLOT_SIZE`] `* slot` (the word)
//! and `+` [`INSTANCE_SLOT_ASSIGNED_OFFSET`] (the flag), and calls
//! [`pycc_rt_instance_get_slot_checked`] only on the cold path where the flag
//! is clear, to raise `AttributeError`. `pycc_codegen`'s `attr_slot` module
//! mirrors the three constants; this module's tests pin them.
//!
//! **Slot representation.** An `int`/`bool` attribute stores its encoded
//! word, a `float` attribute its `f64::to_bits()` pattern, and a
//! heap-object-typed attribute (`str`, another instance, a container, a
//! foreign object) its pointer as an `i64`.
//!
//! **Reference counting.** The header's fifth word is the instance's
//! reference count (`crate::rc`, at `rc::RC_OFFSET`). Freeing an instance
//! releases each slot by the kind its layout descriptor records (see
//! [`pycc_rt_instance_new`]).

use std::cell::Cell;
use std::ffi::c_void;

use crate::exception::{EXCEPTION_TYPE_ATTRIBUTE_ERROR, raise_builtin};

/// Byte offset of slot 0 from the start of a `PyInstanceObj`.
pub const INSTANCE_SLOTS_OFFSET: usize = 40;
/// Size in bytes of one [`InstanceSlot`].
pub const INSTANCE_SLOT_SIZE: usize = 16;
/// Byte offset of a slot's "assigned" flag within the slot.
pub const INSTANCE_SLOT_ASSIGNED_OFFSET: usize = 8;

/// One attribute slot: its word, and whether it has been assigned. No
/// sentinel word could mark "unassigned", because `0` is a valid `int`,
/// `bool` and `float` word. A slot can be read before it is assigned --
/// `self.a = self.b` in `__init__`, or a derived `__init__` that never
/// assigns a base slot (#1148) -- and CPython raises `AttributeError` there.
#[repr(C)]
pub struct InstanceSlot {
    word: Cell<i64>,
    assigned: Cell<i64>,
}

impl InstanceSlot {
    fn get(&self) -> Option<i64> {
        (self.assigned.get() != 0).then(|| self.word.get())
    }

    fn set(&self, value: Option<i64>) {
        self.word.set(value.unwrap_or(0));
        self.assigned.set(i64::from(value.is_some()));
    }
}

/// See this module's own doc comment for the layout.
///
/// `layout_ptr`/`layout_len` is the static descriptor the constructor call
/// site passes (see [`pycc_rt_instance_new`]); it is read only to word the
/// unassigned-slot error and to name the instance's class to an `--ext` host
/// ([`pycc_rt_ext_instance_class`]).
///
/// `carrier` is the `--ext` host-side object currently standing for this
/// instance, or null (#1435). It is opaque here: `pycc_rt` keeps no CPython
/// dependency (D-244 rule 2), and only the C shim reads or writes it, through
/// [`pycc_rt_ext_instance_carrier`] and [`pycc_rt_ext_instance_set_carrier`].
#[repr(C)]
pub struct PyInstanceObj {
    layout_ptr: *const u8,
    layout_len: usize,
    carrier: Cell<*mut c_void>,
    slot_count: usize,
    /// The reference count (`crate::rc`).
    rc: i64,
    slots: [InstanceSlot; 0],
}

// The ABI `pycc_codegen` compiles against, checked at build time.
const _: () = {
    assert!(std::mem::offset_of!(PyInstanceObj, slots) == INSTANCE_SLOTS_OFFSET);
    assert!(std::mem::offset_of!(PyInstanceObj, rc) == crate::rc::RC_OFFSET);
    assert!(std::mem::size_of::<InstanceSlot>() == INSTANCE_SLOT_SIZE);
    assert!(std::mem::offset_of!(InstanceSlot, word) == 0);
    assert!(std::mem::offset_of!(InstanceSlot, assigned) == INSTANCE_SLOT_ASSIGNED_OFFSET);
};

mod carrier;
pub use carrier::{
    pycc_rt_ext_instance_carrier, pycc_rt_ext_instance_class, pycc_rt_ext_instance_set_carrier,
};
mod copy;
pub use copy::pycc_rt_ext_instance_copy;
mod store;
pub use store::{pycc_rt_ext_instance_delete_slot, pycc_rt_ext_instance_store_slot};

impl PyInstanceObj {
    /// The class's static layout descriptor.
    fn layout(&self) -> &'static [u8] {
        if self.layout_ptr.is_null() {
            &[]
        } else {
            // SAFETY: `pycc_rt_instance_new`'s contract -- the descriptor
            // lives for the rest of the process.
            unsafe { std::slice::from_raw_parts(self.layout_ptr, self.layout_len) }
        }
    }

    /// The inline slot array.
    fn slots(&self) -> &[InstanceSlot] {
        // SAFETY: `allocate_instance` allocated `slot_count` initialized
        // slots directly after the header.
        unsafe { std::slice::from_raw_parts(self.slots.as_ptr(), self.slot_count) }
    }
}

/// The allocation layout of an instance with `slot_count` slots.
fn instance_alloc_layout(slot_count: usize) -> std::alloc::Layout {
    let size = INSTANCE_SLOTS_OFFSET + INSTANCE_SLOT_SIZE * slot_count;
    std::alloc::Layout::from_size_align(size, 8)
        .expect("an instance allocation size never overflows isize")
}

/// The slot kind bytes of `layout` (`crate::rc`), one per slot: the
/// descriptor field after the last slot name. `None` for a descriptor that
/// records none (an enum member's, or one written by hand in a test), whose
/// slots then release nothing.
fn slot_kinds(layout: &'static [u8], slot_count: usize) -> Option<&'static [u8]> {
    layout
        .split(|byte| *byte == 0)
        .nth(1 + slot_count)
        .filter(|kinds| kinds.len() == slot_count)
}

/// Frees `instance` (count zero), pushing every slot value whose count its
/// release brings to zero onto `dead`.
pub(crate) fn free_instance(instance: *mut PyInstanceObj, dead: &mut crate::rc::Worklist) {
    crate::rc::INSTANCE_LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: the count reached zero, so nothing else names the instance.
    let object = unsafe { &*instance };
    let slot_count = object.slot_count;
    if let Some(kinds) = slot_kinds(object.layout(), slot_count) {
        for (slot, kind) in object.slots().iter().zip(kinds) {
            if let Some(word) = slot.get() {
                crate::rc::release_word_into(*kind, word, dead);
            }
        }
    }
    // SAFETY: allocated by `allocate_instance` with this very layout.
    unsafe { std::alloc::dealloc(instance.cast::<u8>(), instance_alloc_layout(slot_count)) };
}

/// Allocates an instance with `slot_count` unassigned slots, `layout` as
/// its descriptor and a count of one.
fn allocate_instance(slot_count: usize, layout: &'static [u8]) -> *mut PyInstanceObj {
    let alloc_layout = instance_alloc_layout(slot_count);
    // SAFETY: the size is non-zero (the header alone is 40 bytes). A zeroed
    // allocation is a null carrier plus `slot_count` unassigned slots; the
    // remaining header fields are written before the pointer escapes.
    unsafe {
        let ptr = std::alloc::alloc_zeroed(alloc_layout).cast::<PyInstanceObj>();
        if ptr.is_null() {
            std::alloc::handle_alloc_error(alloc_layout);
        }
        std::ptr::addr_of_mut!((*ptr).layout_ptr).write(layout.as_ptr());
        std::ptr::addr_of_mut!((*ptr).layout_len).write(layout.len());
        std::ptr::addr_of_mut!((*ptr).slot_count).write(slot_count);
        std::ptr::addr_of_mut!((*ptr).rc).write(1);
        crate::rc::INSTANCE_LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ptr
    }
}

/// Validates a constructor's slot count. A negative count is an
/// internal-error panic (impossible from `pycc_codegen` output, which only
/// ever passes a class's own declared attribute count), held in this private
/// function so a test can observe it -- see [`slot_ref`]'s panic note.
fn checked_slot_count(slot_count: i64) -> usize {
    usize::try_from(slot_count).unwrap_or_else(|_| {
        panic!("pycc_rt: internal error: negative instance slot count {slot_count}")
    })
}

/// Allocates an instance of a class with `slot_count` slots, all
/// unassigned.
///
/// `layout_ptr`/`layout_len` is the class's static layout descriptor
/// (#1388): the class's source name followed by the name of each slot in
/// slot order, separated by NUL bytes (`C\0x\0y` for a class `C` whose
/// slots are `x` then `y`), then optionally one more field of one
/// `crate::rc` kind byte per slot (`C\0x\0y\0si`), which says what freeing
/// the instance releases. `pycc_codegen` interns it once per class as a
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
    allocate_instance(checked_slot_count(slot_count), layout)
}

/// Slot `slot` of `instance`. A negative or out-of-range `slot` is an
/// internal-error panic: every `AttrGet`/`AttrSet` slot index is resolved at
/// compile time against the class's own declared attribute count.
///
/// The FFI wrappers below are plain `extern "C" fn`s, not `extern
/// "C-unwind"`, so a panic reaching their boundary aborts the process
/// (same rationale as `pycc_rt_int_add`'s own doc comment); this private
/// function holds the freely-panicking logic so tests can observe it.
fn slot_ref(instance: &PyInstanceObj, slot: i64) -> &InstanceSlot {
    let slots = instance.slots();
    match usize::try_from(slot).ok().and_then(|i| slots.get(i)) {
        Some(cell) => cell,
        None => panic!(
            "pycc_rt: internal error: instance slot {slot} out of range (instance has {} \
             slots) -- pycc_mir should have resolved a slot index within the class's own \
             declared attribute count",
            slots.len()
        ),
    }
}

/// Slot `slot`'s word, or `None` when it is not yet assigned.
fn read_slot(instance: &PyInstanceObj, slot: i64) -> Option<i64> {
    slot_ref(instance, slot).get()
}

/// The unchecked read: an unassigned slot reads as the word `0`.
fn instance_get_slot(instance: &PyInstanceObj, slot: i64) -> i64 {
    read_slot(instance, slot).unwrap_or(0)
}

/// Reads slot `slot`'s raw `i64` word, `0` when it is not yet assigned.
///
/// Not the read a compiled `base.attr` performs (that is an inline load,
/// with [`pycc_rt_instance_get_slot_checked`] as its cold path). Its callers
/// are the stores that release a slot's old value before overwriting it, for
/// which "nothing to release" and the `0` word mean the same thing.
///
/// # Safety
/// `instance` must be a live `PyInstanceObj` pointer.
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
/// exactly as `list_get` does for `IndexError`.
fn instance_get_slot_checked(instance: &PyInstanceObj, slot: i64) -> i64 {
    if let Some(word) = read_slot(instance, slot) {
        return word;
    }
    let class_name = layout_field(instance.layout(), 0);
    let class_name = crate::qualified::plain(&class_name);
    let attr = layout_field(instance.layout(), 1 + slot as usize);
    raise_builtin(
        EXCEPTION_TYPE_ATTRIBUTE_ERROR,
        "AttributeError",
        &format!("'{class_name}' object has no attribute '{attr}'"),
    );
    0
}

/// The cold path of every compiled `base.attr` read; see
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
/// marking the slot assigned.
fn instance_set_slot(instance: &PyInstanceObj, slot: i64, value: i64) {
    slot_ref(instance, slot).set(Some(value));
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

    /// A store is observable at the documented address `pycc_codegen` reads
    /// and writes directly, and so is the flag.
    #[test]
    fn a_slot_lives_at_its_documented_address() {
        let ptr = unsafe { pycc_rt_instance_new(3, std::ptr::null(), 0) };
        unsafe { pycc_rt_instance_set_slot(ptr, 2, 77) };
        let base = ptr.cast::<u8>();
        let slot = INSTANCE_SLOTS_OFFSET + 2 * INSTANCE_SLOT_SIZE;
        let word = unsafe { base.add(slot).cast::<i64>().read() };
        let flag = unsafe {
            base.add(slot + INSTANCE_SLOT_ASSIGNED_OFFSET)
                .cast::<i64>()
                .read()
        };
        assert_eq!((word, flag), (77, 1));
        let unset = INSTANCE_SLOTS_OFFSET + INSTANCE_SLOT_ASSIGNED_OFFSET;
        assert_eq!(unsafe { base.add(unset).cast::<i64>().read() }, 0);
    }

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
        let instance = unsafe { &*pycc_rt_instance_new(0, std::ptr::null(), 0) };
        assert!(instance.slots().is_empty());
        assert!(instance.layout().is_empty());
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
        // The private validator, not the `extern "C"` wrapper, whose panic
        // would abort the test process instead of unwinding.
        checked_slot_count(-1);
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
