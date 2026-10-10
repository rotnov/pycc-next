use super::*;
use std::cmp::Ordering::{Equal, Greater, Less};

/// Part A of #1038 (#1063): the pending exception's tag and message, for
/// the converted `**` abort paths. The state is thread-local and
/// `cargo test` runs in parallel, so every caller clears before raising
/// and after asserting; a message left pending would otherwise be read by
/// whatever test the harness schedules next on this thread.
pub(crate) fn pending_tag_and_message() -> (u8, String) {
    assert_eq!(pycc_rt_exception_active(), 1);
    let obj = exception::pycc_rt_exception_value();
    let tag = unsafe { (*obj).type_tag };
    let mut len = 0usize;
    let bytes = unsafe { exception::pycc_rt_ext_pending_message(&mut len) };
    assert!(!bytes.is_null());
    let message = String::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec())
        .expect("the message is UTF-8");
    (tag, message)
}

// Topic submodules split out of this file.
mod ints;
mod buffers;
mod floats;
mod strs;
