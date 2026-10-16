//! A set display of constants, in CPython's iteration order.
//!
//! CPython compiles `{-9, 6, 2, 7}` (three or more constants) into
//! `BUILD_SET 0; LOAD_CONST frozenset(...); SET_UPDATE`. The frozenset
//! constant is built from the elements in source order, and built again
//! from its own iteration order when the compiler merges constants
//! (`merge_consts_recursive`); `SET_UPDATE` then merges it into the empty
//! set. Each step can move a colliding element, so the result can iterate
//! differently from the same elements added in source order: `{-9, 6, 2, 7}`
//! prints `{2, -9, 6, 7}`, `set([-9, 6, 2, 7])` prints `{2, 7, 6, -9}`.
//! Found by the differential fuzzer.

use super::*;

/// Reorders `s`, a set display of constants built in source order, as
/// CPython's constant folding orders it (see the module documentation).
///
/// # Safety
/// `s` must be a live set the caller owns alone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_set_constant_display(s: *mut PySetObj) {
    if s.is_null() || is_instance_set(s) {
        return;
    }
    let s = set(s);
    let mut merged = PySetObj::new(s.kind);
    for (key, hash) in s.active() {
        value::retain(s.kind, key);
        merged.add_entry(key, hash);
    }
    let mut result = PySetObj::new(s.kind);
    result.merge(&merged);
    swap_bodies(s, &mut result);
    result.clear();
    merged.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::int_encoding::tag_smallint;

    fn display(items: &[i64]) -> Vec<i64> {
        let s = pycc_rt_set_new(crate::list::LIST_KIND_INT);
        for &v in items {
            unsafe { pycc_rt_set_add(s, tag_smallint(v)) };
        }
        unsafe { pycc_rt_set_constant_display(s) };
        let order = set(s).active().iter().map(|&(k, _)| k >> 1).collect();
        free_set(s);
        order
    }

    #[test]
    fn a_constant_display_iterates_in_cpythons_order() {
        // `python3 -c 'print({-9, 6, 2, 7})'`; `set([-9, 6, 2, 7])` is
        // `{2, 7, 6, -9}`.
        assert_eq!(display(&[-9, 6, 2, 7]), vec![2, -9, 6, 7]);
        // `python3 -c 'print({1, 9, 17, 25, 33, 41})'`.
        assert_eq!(display(&[1, 9, 17, 25, 33, 41]), vec![1, 33, 17, 25, 41, 9]);
    }
}
