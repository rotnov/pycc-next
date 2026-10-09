//! #1316: the reserved [`EXCEPTION_TYPE_FOREIGN_BASE`] tag the `ext` shim
//! gives a bridged CPython exception that is not an `Exception`. `except
//! Exception:` must not match it, in either matcher, while an exact handler
//! for it still does.
//!
//! Its own file rather than appended to `exception.rs`'s inline test module,
//! which already runs past this repository's ~1,000-line threshold.

use super::*;

fn alloc_tagged(type_tag: u8, name: &'static str) -> *mut PyExceptionObj {
    pycc_rt_exception_alloc(
        type_tag,
        name.as_ptr(),
        name.len(),
        alloc_exception_message(name),
    )
}

#[test]
fn except_exception_does_not_match_a_foreign_base_exception() {
    let obj = alloc_tagged(EXCEPTION_TYPE_FOREIGN_BASE, "BaseException");
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_EXCEPTION) },
        0
    );
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_FOREIGN_BASE) },
        1
    );
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_VALUE_ERROR) },
        0
    );
}

#[test]
fn except_exception_still_matches_every_other_tag() {
    for tag in [0, 1, 7, 25, 27, 28, 254] {
        let obj = alloc_tagged(tag, "Exception");
        assert_eq!(
            unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_EXCEPTION) },
            1,
            "tag {tag}"
        );
    }
}

#[test]
fn except_star_exception_leaves_a_foreign_base_member_in_the_rest() {
    const GROUP_NAME: &str = "ExceptionGroup";
    let base = alloc_tagged(EXCEPTION_TYPE_FOREIGN_BASE, "BaseException");
    let value = alloc_tagged(EXCEPTION_TYPE_VALUE_ERROR, "ValueError");
    let members = [base, value];
    let group = unsafe {
        pycc_rt_exception_group_alloc(
            24,
            GROUP_NAME.as_ptr(),
            GROUP_NAME.len(),
            alloc_exception_message("group"),
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
            24,
            GROUP_NAME.as_ptr(),
            GROUP_NAME.len(),
            &mut matched,
            &mut rest,
        );
        assert_eq!(
            std::slice::from_raw_parts((*matched).exceptions, 1),
            [value]
        );
        assert_eq!(std::slice::from_raw_parts((*rest).exceptions, 1), [base]);
    }
}
