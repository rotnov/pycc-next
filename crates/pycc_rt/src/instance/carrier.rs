//! The `--ext` host's view of an instance (#1435): which class it is, and
//! which host-side carrier object currently stands for it.
//!
//! An instance passed to a call on a CPython object crosses into the host
//! as a `PyccExtInstance` carrier (`src/ext/module/`). The shim
//! needs two facts that only this crate holds:
//!
//! * **The class.** Field 0 of the layout descriptor (#1388) is the class
//!   the instance was constructed as. pycc refuses a subclass instance
//!   where a base class is declared, but an inherited method body compiled
//!   once for its base still runs with a derived `self`. The descriptor is
//!   the run-time class there, which a static type is not.
//! * **The carrier.** Python identity says that every crossing of one
//!   instance yields the same object while that object lives: `x is y` for
//!   two crossings, and `self` handed out by a method of a host-constructed
//!   object is that object. The shim therefore records the live carrier
//!   here and reuses it. The pointer is *weak*: the carrier's own
//!   `tp_dealloc` clears it, and the instance never owns a reference. An
//!   instance is never freed (D-107, D-154), so the shim can always reach
//!   this field from a live carrier.
//!
//! The pointer is opaque here. `pycc_rt` keeps no CPython dependency
//! (D-244 rule 2), and nothing in this crate dereferences it.

use std::ffi::c_void;

use super::PyInstanceObj;

/// The class name the instance was constructed as: the bytes before the
/// first NUL of its layout descriptor. Writes the length to `*len` and
/// answers a pointer into the descriptor, which is static. An empty name
/// (length `0`) means the instance has no descriptor; only an enum member
/// is allocated that way, and the shim refuses it.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj` and `len` at writable
/// storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_class(
    instance: *mut PyInstanceObj,
    len: *mut usize,
) -> *const u8 {
    let layout = unsafe { &*instance }.layout();
    let name = class_name(layout);
    unsafe { *len = name.len() };
    name.as_ptr()
}

/// Whether `instance` was constructed as the class named by the `len` bytes
/// at `name` (v0.9): the run-time class test of a dispatched method call
/// (`pycc_mir::dispatch`), compared by name because an instance a host
/// constructed carries the shim's copy of the descriptor, not codegen's.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj` and `name` at `len`
/// readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_instance_class_is(
    instance: *const PyInstanceObj,
    name: *const u8,
    len: usize,
) -> bool {
    let wanted = unsafe { std::slice::from_raw_parts(name, len) };
    class_name(unsafe { &*instance }.layout()) == wanted
}

/// Field 0 of a layout descriptor.
fn class_name(layout: &'static [u8]) -> &'static [u8] {
    layout
        .split(|byte| *byte == 0)
        .next()
        .expect("split yields at least one field")
}

/// The low bit of the carrier word: the carrier is an instance of a Python
/// subclass of the compiled class (v0.9), and the link is *strong* -- the
/// object's type and `__dict__` are part of the instance's state, so
/// compiled code holding the instance must keep that very object alive
/// rather than re-box the instance as a plain compiled-class carrier. A
/// carrier is a `PyObject *`, so its low bit is free.
const STRONG: usize = 1;

/// The carrier currently recorded for `instance`, or null.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_carrier(instance: *mut PyInstanceObj) -> *mut c_void {
    let word = unsafe { &*instance }.carrier.get() as usize;
    (word & !STRONG) as *mut c_void
}

/// Records `carrier` (or null, to clear it) as the host object standing
/// for `instance`, as a weak link.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_set_carrier(
    instance: *mut PyInstanceObj,
    carrier: *mut c_void,
) {
    unsafe { &*instance }.carrier.set(carrier);
}

/// Records `carrier`, an instance of a Python subclass, as the host object
/// standing for `instance`, flagged [`STRONG`]: the shim took a reference
/// to it, which it drops when it clears the link.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj` and `carrier` be
/// pointer-aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_set_carrier_strong(
    instance: *mut PyInstanceObj,
    carrier: *mut c_void,
) {
    unsafe { &*instance }
        .carrier
        .set((carrier as usize | STRONG) as *mut c_void);
}

/// Whether `instance`'s carrier is an instance of a Python subclass, linked
/// strongly ([`pycc_rt_ext_instance_set_carrier_strong`]).
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_carrier_is_strong(
    instance: *mut PyInstanceObj,
) -> bool {
    unsafe { &*instance }.carrier.get() as usize & STRONG != 0
}

/// The reference count of `instance`, for the shim's garbage-collector
/// traversal of a strongly linked carrier.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_refcount(instance: *mut PyInstanceObj) -> i64 {
    unsafe { &*instance }.rc
}

/// The number of slots of `instance` (v0.10): the slot names its layout
/// descriptor lists after the class name, which the shim's `__getstate__`
/// reads.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_slot_count(instance: *mut PyInstanceObj) -> i64 {
    unsafe { &*instance }.slot_count as i64
}

/// The whole layout descriptor of `instance` (v0.10), its length through
/// `len`: the class name, then the slot names, NUL-separated.
///
/// # Safety
/// `instance` must point at a live `PyInstanceObj`; `len` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_ext_instance_layout(
    instance: *mut PyInstanceObj,
    len: *mut usize,
) -> *const u8 {
    let layout = unsafe { &*instance }.layout();
    unsafe { *len = layout.len() };
    layout.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::super::pycc_rt_instance_new;
    use super::*;

    fn class_of(instance: *mut PyInstanceObj) -> Vec<u8> {
        let mut len = usize::MAX;
        let ptr = unsafe { pycc_rt_ext_instance_class(instance, &mut len) };
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    }

    #[test]
    fn class_is_the_first_descriptor_field() {
        static LAYOUT: &[u8] = b"Point\0x\0y";
        let instance = unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) };
        assert_eq!(class_of(instance), b"Point");
    }

    #[test]
    fn class_of_a_slotless_descriptor_is_the_whole_descriptor() {
        static LAYOUT: &[u8] = b"Empty";
        let instance = unsafe { pycc_rt_instance_new(0, LAYOUT.as_ptr(), LAYOUT.len()) };
        assert_eq!(class_of(instance), b"Empty");
    }

    #[test]
    fn class_of_an_instance_without_a_descriptor_is_empty() {
        let instance = unsafe { pycc_rt_instance_new(2, std::ptr::null(), 0) };
        assert_eq!(class_of(instance), b"");
    }

    #[test]
    fn carrier_starts_null_and_round_trips() {
        let instance = unsafe { pycc_rt_instance_new(1, std::ptr::null(), 0) };
        assert!(unsafe { pycc_rt_ext_instance_carrier(instance) }.is_null());
        let mut host = 0u64;
        let carrier = (&mut host as *mut u64).cast::<c_void>();
        unsafe { pycc_rt_ext_instance_set_carrier(instance, carrier) };
        assert_eq!(unsafe { pycc_rt_ext_instance_carrier(instance) }, carrier);
        unsafe { pycc_rt_ext_instance_set_carrier(instance, std::ptr::null_mut()) };
        assert!(unsafe { pycc_rt_ext_instance_carrier(instance) }.is_null());
    }

    #[test]
    fn a_strong_carrier_reads_back_unflagged_and_reports_its_flag() {
        let instance = unsafe { pycc_rt_instance_new(1, std::ptr::null(), 0) };
        let mut host = 0u64;
        let carrier = (&mut host as *mut u64).cast::<c_void>();
        unsafe { pycc_rt_ext_instance_set_carrier_strong(instance, carrier) };
        assert_eq!(unsafe { pycc_rt_ext_instance_carrier(instance) }, carrier);
        assert!(unsafe { pycc_rt_ext_instance_carrier_is_strong(instance) });
        assert_eq!(unsafe { pycc_rt_ext_instance_refcount(instance) }, 1);
        unsafe { pycc_rt_ext_instance_set_carrier(instance, carrier) };
        assert!(!unsafe { pycc_rt_ext_instance_carrier_is_strong(instance) });
    }

    #[test]
    fn class_is_compares_the_descriptor_name_exactly() {
        static LAYOUT: &[u8] = b"Point\0x\0y";
        let instance = unsafe { pycc_rt_instance_new(2, LAYOUT.as_ptr(), LAYOUT.len()) };
        assert!(unsafe { pycc_rt_instance_class_is(instance, b"Point".as_ptr(), 5) });
        assert!(!unsafe { pycc_rt_instance_class_is(instance, b"Poin".as_ptr(), 4) });
        assert!(!unsafe { pycc_rt_instance_class_is(instance, b"Point3".as_ptr(), 6) });
    }
}
