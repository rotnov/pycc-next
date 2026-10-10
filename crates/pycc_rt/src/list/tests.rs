use super::*;
use crate::tests::pending_tag_and_message;

fn int_list(values: &[i64]) -> *mut PyListObj {
    let list = pycc_rt_list_new(LIST_KIND_INT, 0);
    for value in values {
        unsafe { pycc_rt_list_append(list, tag_smallint(*value)) };
    }
    list
}

fn ints(list: *mut PyListObj) -> Vec<i64> {
    unsafe { &*list }
        .items()
        .iter()
        .map(|w| untag_smallint(*w))
        .collect()
}

fn repr(list: *mut PyListObj) -> String {
    let s = unsafe { pycc_rt_list_repr(list) };
    String::from_utf8(unsafe { &*s }.bytes().to_vec()).unwrap()
}

fn str_word(text: &str) -> i64 {
    new_pystr(text.as_bytes()) as i64
}

#[test]
fn list_layout_matches_the_codegen_contract() {
    assert_eq!(std::mem::offset_of!(PyListObj, len), 0);
    assert_eq!(std::mem::offset_of!(PyListObj, data), 8);
}

#[test]
fn get_resolves_negative_indices_and_raises_outside() {
    pycc_rt_exception_clear();
    let list = int_list(&[10, 20, 30]);
    unsafe {
        assert_eq!(untag_smallint(pycc_rt_list_get(list, -1)), 30);
        assert_eq!(untag_smallint(pycc_rt_list_get(list, -3)), 10);
        assert_eq!(pycc_rt_list_get(list, 3), tag_smallint(0));
    }
    let (_, message) = pending_tag_and_message();
    assert_eq!(message, "list index out of range");
    pycc_rt_exception_clear();
    unsafe { pycc_rt_list_get(list, -4) };
    assert_ne!(pycc_rt_exception_active(), 0);
    pycc_rt_exception_clear();
}

#[test]
fn set_replaces_and_raises_its_own_message() {
    pycc_rt_exception_clear();
    let list = int_list(&[1, 2]);
    unsafe { pycc_rt_list_set(list, -1, tag_smallint(9)) };
    assert_eq!(ints(list), vec![1, 9]);
    unsafe { pycc_rt_list_set(list, 2, tag_smallint(9)) };
    let (_, message) = pending_tag_and_message();
    assert_eq!(message, "list assignment index out of range");
    pycc_rt_exception_clear();
}

#[test]
fn pop_and_insert_follow_cpython() {
    pycc_rt_exception_clear();
    let list = int_list(&[1, 2, 3, 4]);
    unsafe {
        assert_eq!(untag_smallint(pycc_rt_list_pop(list, 0, 0)), 4);
        assert_eq!(untag_smallint(pycc_rt_list_pop(list, 1, 0)), 1);
        assert_eq!(ints(list), vec![2, 3]);
        pycc_rt_list_insert(list, -100, tag_smallint(0));
        pycc_rt_list_insert(list, 100, tag_smallint(9));
        pycc_rt_list_insert(list, -1, tag_smallint(5));
        assert_eq!(ints(list), vec![0, 2, 3, 5, 9]);
        pycc_rt_list_pop(list, 1, 7);
    }
    assert_eq!(pending_tag_and_message().1, "pop index out of range");
    pycc_rt_exception_clear();
    let empty = int_list(&[]);
    unsafe { pycc_rt_list_pop(empty, 0, 0) };
    assert_eq!(pending_tag_and_message().1, "pop from empty list");
    pycc_rt_exception_clear();
}

#[test]
fn slice_matches_cpython_for_every_bound_shape() {
    pycc_rt_exception_clear();
    let list = int_list(&[0, 1, 2, 3, 4, 5]);
    let all = SLICE_HAS_START | SLICE_HAS_STOP | SLICE_HAS_STEP;
    let cases: [(i64, i64, i64, i64, Vec<i64>); 6] = [
        (1, 4, 1, all, vec![1, 2, 3]),
        (-2, 0, 0, SLICE_HAS_START, vec![4, 5]),
        (0, 0, -1, SLICE_HAS_STEP, vec![5, 4, 3, 2, 1, 0]),
        (4, 1, -2, all, vec![4, 2]),
        (0, -10, 0, SLICE_HAS_STOP, vec![]),
        (-100, 100, 2, all, vec![0, 2, 4]),
    ];
    for (start, stop, step, present, expected) in cases {
        let sliced = unsafe { pycc_rt_list_slice(list, start, stop, step, present) };
        assert_eq!(ints(sliced), expected, "{start}:{stop}:{step} ({present})");
    }
    unsafe { pycc_rt_list_slice(list, 0, 0, 0, SLICE_HAS_STEP) };
    assert_eq!(pending_tag_and_message().1, "slice step cannot be zero");
    pycc_rt_exception_clear();
}

#[test]
fn concat_repeat_and_contains() {
    let a = int_list(&[1, 2]);
    let b = int_list(&[3]);
    unsafe {
        assert_eq!(ints(pycc_rt_list_concat(a, b)), vec![1, 2, 3]);
        assert_eq!(ints(pycc_rt_list_repeat(a, 2)), vec![1, 2, 1, 2]);
        assert_eq!(ints(pycc_rt_list_repeat(a, -1)), Vec::<i64>::new());
        assert_eq!(pycc_rt_list_contains(a, tag_smallint(2)), 1);
        assert_eq!(pycc_rt_list_contains(a, tag_smallint(5)), 0);
        assert_eq!(pycc_rt_list_contains(a, BOOL_TRUE_MARKER), 1);
    }
}

#[test]
fn str_elements_are_owned_and_compared_by_value() {
    let list = pycc_rt_list_new(LIST_KIND_STR, 0);
    unsafe {
        pycc_rt_list_append(list, str_word("a'b"));
        pycc_rt_list_append(list, str_word("c\n"));
        let copy = pycc_rt_list_repeat(list, 2);
        assert_eq!(repr(copy), "[\"a'b\", 'c\\n', \"a'b\", 'c\\n']");
        let needle = str_word("c\n");
        assert_eq!(pycc_rt_list_contains(list, needle), 1);
        pycc_rt_list_set(copy, 0, str_word("z"));
        assert_eq!(repr(list), "[\"a'b\", 'c\\n']");
        let popped = pycc_rt_list_pop(list, 0, 0) as *mut PyStrObj;
        assert_eq!((*popped).bytes(), b"c\n");
    }
}

#[test]
fn repr_covers_every_scalar_kind_and_nesting() {
    pycc_rt_exception_clear();
    let floats = pycc_rt_list_new(LIST_KIND_FLOAT, 0);
    let bools = pycc_rt_list_new(LIST_KIND_BOOL, 0);
    let nested = pycc_rt_list_new(LIST_KIND_LIST, 0);
    unsafe {
        pycc_rt_list_append(floats, 1.5f64.to_bits() as i64);
        pycc_rt_list_append(floats, 2.0f64.to_bits() as i64);
        pycc_rt_list_append(bools, 1);
        pycc_rt_list_append(bools, 0);
        pycc_rt_list_append(nested, int_list(&[1]) as i64);
        pycc_rt_list_append(nested, int_list(&[]) as i64);
        assert_eq!(pycc_rt_list_contains(nested, int_list(&[1]) as i64), 1);
        assert_eq!(pycc_rt_list_contains(floats, 2.0f64.to_bits() as i64), 1);
    }
    assert_eq!(repr(floats), "[1.5, 2.0]");
    assert_eq!(repr(bools), "[True, False]");
    assert_eq!(repr(nested), "[[1], []]");
    let huge = pycc_rt_list_new(LIST_KIND_FLOAT, 0);
    unsafe { pycc_rt_list_append(huge, 1e300f64.to_bits() as i64) };
    assert_eq!(repr(huge), "");
    assert_ne!(pycc_rt_exception_active(), 0);
    pycc_rt_exception_clear();
}

#[test]
fn instance_repr_names_the_class() {
    let layout = b"Point\0x";
    let instance = unsafe { pycc_rt_instance_new(1, layout.as_ptr(), layout.len()) };
    let list = pycc_rt_list_new(LIST_KIND_INSTANCE, 0);
    unsafe { pycc_rt_list_append(list, instance as i64) };
    assert!(repr(list).starts_with("[<__main__.Point object at 0x"));
}

/// A `list[bool]` stores one byte per element; every operation reads and
/// writes the same `0`/`1` words through it.
#[test]
fn a_bool_list_stores_bytes_and_round_trips_every_operation() {
    pycc_rt_exception_clear();
    let one = pycc_rt_list_new(LIST_KIND_BOOL, 0);
    unsafe {
        pycc_rt_list_append(one, 1);
        let list = pycc_rt_list_repeat(one, 3);
        assert_eq!(&*(*list).items(), &[1, 1, 1]);
        assert_eq!((*list).typed::<u8>(), &[1, 1, 1]);
        pycc_rt_list_set(list, 1, 0);
        pycc_rt_list_append(list, 0);
        pycc_rt_list_insert(list, 0, 0);
        assert_eq!(repr(list), "[False, True, False, True, False]");
        assert_eq!(pycc_rt_list_get(list, -2), 1);
        assert_eq!(pycc_rt_list_pop(list, 1, 0), 0);
        assert_eq!(pycc_rt_list_pop(list, 0, 0), 0);
        assert_eq!(pycc_rt_list_contains(list, 0), 1);
        let both = pycc_rt_list_concat(list, one);
        assert_eq!(repr(both), "[True, False, True, True]");
        let picked = pycc_rt_list_slice(both, 0, 0, 2, SLICE_HAS_STEP);
        assert_eq!((*picked).typed::<u8>(), &[1, 1]);
        assert_eq!(pycc_rt_list_len(pycc_rt_list_repeat(one, -1)), 0);
    }
    assert_eq!(pycc_rt_exception_active(), 0);
}
