//! A set display of constants, in CPython's iteration order.
//!
//! CPython compiles `{-9, 6, 2, 7}` (three or more constants) into
//! `BUILD_SET 0; LOAD_CONST frozenset(...); SET_UPDATE`. The frozenset
//! constant is built from the elements in source order (CPython 3.13 built
//! it once more from its own iteration order when merging constants; 3.14
//! stopped), and `SET_UPDATE` merges it into the empty set, which can move
//! a colliding element: `{1, 9, 17, 25, 33, 41}` prints
//! `{1, 33, 17, 9, 25, 41}`. Found by the differential fuzzer.

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
    let mut result = PySetObj::new(s.kind);
    result.merge(s);
    swap_bodies(s, &mut result);
    result.clear();
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
        // `python3.15 -c 'print({-9, 6, 2, 7})'`, as `set([-9, 6, 2, 7])`.
        assert_eq!(display(&[-9, 6, 2, 7]), vec![2, 7, 6, -9]);
        // `python3.15 -c 'print({1, 9, 17, 25, 33, 41})'`.
        assert_eq!(display(&[1, 9, 17, 25, 33, 41]), vec![1, 33, 17, 9, 25, 41]);
    }
}
