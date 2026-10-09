//! #1316: the reserved `EXCEPTION_TYPE_FOREIGN_BASE` tag, exercised through
//! the `rlib` exactly as generated code links the two matchers from the
//! `staticlib`.
//!
//! `crates/pycc_rt/src/exception/foreign_base_tests.rs` owns the semantics;
//! this file pins them from outside the crate, which is also where the
//! diff-coverage gate measures these exported functions (`docs/TESTING.md`,
//! "A runtime function an integration test links is measured from that
//! binary").

use pycc_rt::{
    EXCEPTION_TYPE_EXCEPTION, EXCEPTION_TYPE_FOREIGN_BASE, EXCEPTION_TYPE_VALUE_ERROR,
    PyExceptionObj, pycc_rt_exception_alloc, pycc_rt_exception_group_alloc,
    pycc_rt_exception_group_partition, pycc_rt_exception_type_matches, pycc_rt_str_from_literal,
};

/// `ExceptionGroup`'s builtin tag, as codegen passes it.
const GROUP_TAG: u8 = 24;
const GROUP_NAME: &str = "ExceptionGroup";

fn alloc_tagged(type_tag: u8, name: &'static str) -> *mut PyExceptionObj {
    let message = unsafe { pycc_rt_str_from_literal(name.as_ptr(), name.len() as i64) };
    pycc_rt_exception_alloc(type_tag, name.as_ptr(), name.len(), message)
}

/// Partitions a group of `members` by `except* Exception`, answering
/// whether each side came back non-null.
fn partition_by_exception(members: &[*mut PyExceptionObj]) -> (bool, bool) {
    let message = unsafe { pycc_rt_str_from_literal(GROUP_NAME.as_ptr(), GROUP_NAME.len() as i64) };
    let group = unsafe {
        pycc_rt_exception_group_alloc(
            GROUP_TAG,
            GROUP_NAME.as_ptr(),
            GROUP_NAME.len(),
            message,
            members.as_ptr(),
            members.len(),
        )
    };
    let mut matched = std::ptr::null_mut();
    let mut rest = std::ptr::null_mut();
    let tags = [EXCEPTION_TYPE_EXCEPTION];
    unsafe {
        pycc_rt_exception_group_partition(
            group,
            tags.as_ptr(),
            tags.len(),
            GROUP_TAG,
            GROUP_NAME.as_ptr(),
            GROUP_NAME.len(),
            &mut matched,
            &mut rest,
        );
    }
    (!matched.is_null(), !rest.is_null())
}

#[test]
fn except_exception_lets_a_foreign_base_exception_through() {
    let base = alloc_tagged(EXCEPTION_TYPE_FOREIGN_BASE, "BaseException");
    let value = alloc_tagged(EXCEPTION_TYPE_VALUE_ERROR, "ValueError");
    let matches = |obj, tag| unsafe { pycc_rt_exception_type_matches(obj, tag) };
    assert_eq!(matches(base, EXCEPTION_TYPE_EXCEPTION), 0);
    assert_eq!(matches(base, EXCEPTION_TYPE_FOREIGN_BASE), 1);
    assert_eq!(matches(value, EXCEPTION_TYPE_EXCEPTION), 1);
    assert_eq!(matches(value, EXCEPTION_TYPE_FOREIGN_BASE), 0);
}

#[test]
fn except_star_exception_leaves_only_a_foreign_base_member_in_the_rest() {
    let base = alloc_tagged(EXCEPTION_TYPE_FOREIGN_BASE, "BaseException");
    let value = alloc_tagged(EXCEPTION_TYPE_VALUE_ERROR, "ValueError");
    assert_eq!(partition_by_exception(&[base]), (false, true));
    assert_eq!(partition_by_exception(&[value]), (true, false));
    assert_eq!(partition_by_exception(&[base, value]), (true, true));
}
