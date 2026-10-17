//! `ExceptionGroup`/`BaseExceptionGroup` construction and the `except*`
//! partition (Part 3 of #382, #542, PEP 654).
//!
//! A group owns one reference to its message and to each member, which
//! freeing it releases (`lifetime.rs`). Both entry points *borrow* the
//! members and the message they are handed and take references of their
//! own, so a member shared by several groups -- the original group and each
//! subgroup a partition derives from it -- is released once per holder.

use super::lifetime::{new_exception, pycc_rt_exception_incref};
use super::{PyExceptionObj, tag_matches};
use crate::PyStrObj;

/// Copies `members` into a freshly heap-allocated, owned slice, taking one
/// reference to each, and returns its thin pointer and length (`(null, 0)`
/// for no members).
fn own_members(members: &[*mut PyExceptionObj]) -> (*mut *mut PyExceptionObj, usize) {
    if members.is_empty() {
        return (std::ptr::null_mut(), 0);
    }
    for member in members {
        // SAFETY: the members of a group being built are live exceptions.
        unsafe { pycc_rt_exception_incref(*member) };
    }
    let boxed: Box<[*mut PyExceptionObj]> = members.to_vec().into_boxed_slice();
    (Box::leak(boxed).as_mut_ptr(), members.len())
}

/// A fresh group of `members` (each retained) whose message is a new
/// reference to `message`, or null when `members` is empty -- "this handler
/// caught nothing" / "nothing is left to reraise", which codegen tests with
/// a null check.
fn build_group_or_null(
    members: &[*mut PyExceptionObj],
    group_type_tag: u8,
    group_name: *const u8,
    group_name_len: usize,
    message: *mut PyStrObj,
) -> *mut PyExceptionObj {
    if members.is_empty() {
        return std::ptr::null_mut();
    }
    // SAFETY: a group's message is a live `str` the group owns.
    unsafe { crate::pycc_rt_str_incref(message) };
    let (exceptions, exceptions_len) = own_members(members);
    new_exception(
        group_type_tag,
        group_name,
        group_name_len,
        message,
        exceptions,
        exceptions_len,
    )
}

/// Constructs a group carrying `exceptions_len` member exceptions copied
/// out of `exceptions` (borrowed: the group takes a reference to each) and
/// the message `message` (owned: the reference passes to the group). The one
/// production caller is codegen's lowering of a literal-list
/// `ExceptionGroup(msg, [e1, e2])` (`pycc_types`/`pycc_mir` restrict the
/// second argument to a literal list -- D-202).
///
/// # Safety
/// `exceptions` must be null (iff `exceptions_len == 0`) or point to
/// `exceptions_len` live exception objects, readable for the duration of
/// this call; the array is copied, not retained by pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_group_alloc(
    type_tag: u8,
    name: *const u8,
    name_len: usize,
    message: *mut PyStrObj,
    exceptions: *const *mut PyExceptionObj,
    exceptions_len: usize,
) -> *mut PyExceptionObj {
    let members: &[*mut PyExceptionObj] = if exceptions_len == 0 {
        &[]
    } else {
        // SAFETY (caller): `exceptions_len` readable pointers.
        unsafe { std::slice::from_raw_parts(exceptions, exceptions_len) }
    };
    let (exceptions, exceptions_len) = own_members(members);
    new_exception(
        type_tag,
        name,
        name_len,
        message,
        exceptions,
        exceptions_len,
    )
}

/// Partitions `group`'s (borrowed) members by whether they match any tag in
/// `tags` (PEP 654 `except*` dispatch): an empty `tags` matches every
/// member, mirroring a bare `except*:`; a nonempty `tags` matches a member
/// whose own tag passes `tag_matches` against any entry (so `Exception`'s
/// catch-all matches everything but a bridged non-`Exception`).
///
/// `matched_out` receives a new reference to a fresh group of the matched
/// members, or null when none matched; `rest_out` the same for the
/// unmatched members. Both are tagged `group_type_tag`/`group_name`, which
/// codegen supplies, and share `group`'s message (each holding its own
/// reference).
///
/// A `group` with no members is treated as a one-member group of itself:
/// an ordinary exception raised inside a `try*` body, which a matching
/// `except*` clause still catches wrapped in a fresh one-member group, as
/// CPython does.
///
/// # Safety
/// `group` must point to a live exception object. `tags` must be null (iff
/// `tags_len == 0`) or point to `tags_len` readable bytes.
/// `matched_out`/`rest_out` must be valid, writable out-pointers.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pycc_rt_exception_group_partition(
    group: *mut PyExceptionObj,
    tags: *const u8,
    tags_len: usize,
    group_type_tag: u8,
    group_name: *const u8,
    group_name_len: usize,
    matched_out: *mut *mut PyExceptionObj,
    rest_out: *mut *mut PyExceptionObj,
) {
    // SAFETY (caller): live.
    let obj = unsafe { &*group };
    let members: Vec<*mut PyExceptionObj> = if obj.exceptions_len == 0 {
        vec![group]
    } else {
        // SAFETY: a group's member array holds `exceptions_len` pointers.
        unsafe { std::slice::from_raw_parts(obj.exceptions, obj.exceptions_len) }.to_vec()
    };
    let tag_slice: &[u8] = if tags_len == 0 {
        &[]
    } else {
        // SAFETY (caller): `tags_len` readable bytes.
        unsafe { std::slice::from_raw_parts(tags, tags_len) }
    };
    let (matched, rest): (Vec<_>, Vec<_>) = members.into_iter().partition(|member| {
        // SAFETY: every member is a live exception object.
        let member_tag = unsafe { (**member).type_tag };
        tag_slice.is_empty() || tag_slice.iter().any(|tag| tag_matches(member_tag, *tag))
    });
    let message = obj.message;
    // SAFETY (caller): both out-pointers are writable.
    unsafe {
        *matched_out = build_group_or_null(
            &matched,
            group_type_tag,
            group_name,
            group_name_len,
            message,
        );
        *rest_out = build_group_or_null(&rest, group_type_tag, group_name, group_name_len, message);
    }
}
