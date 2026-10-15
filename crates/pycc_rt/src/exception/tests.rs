use super::*;

/// Allocates an exception carrying a class name, the shape every
/// production caller uses since Part 2 of #541 (D-189).
fn alloc_named(type_tag: u8, name: &'static str, msg: &str) -> *mut PyExceptionObj {
    pycc_rt_exception_alloc(
        type_tag,
        name.as_ptr(),
        name.len(),
        alloc_exception_message(msg),
    )
}

/// D-244's `ext` exception accessors, all four states in one test so no
/// arm survives on a thread whose exception state another test left set:
/// the state is thread-local and `cargo test` runs tests in parallel, so
/// each assertion here is made against state this test itself installed.
#[test]
fn the_ext_accessors_report_the_pending_exception_and_nothing_when_clear() {
    // Nothing pending.
    pycc_rt_exception_clear();
    let mut len: usize = 7;
    assert_eq!(pycc_rt_ext_pending_type(), -1);
    assert!(unsafe { pycc_rt_ext_pending_message(&raw mut len) }.is_null());
    assert_eq!(len, 0);

    // A raised exception reports its tag and its message bytes.
    pycc_rt_exception_raise(alloc_named(
        EXCEPTION_TYPE_ZERO_DIV_ERROR,
        "ZeroDivisionError",
        "division by zero",
    ));
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_ZERO_DIV_ERROR)
    );
    let bytes = unsafe { pycc_rt_ext_pending_message(&raw mut len) };
    assert!(!bytes.is_null());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(bytes, len) },
        b"division by zero"
    );

    // An exception with no message object at all: the shim substitutes
    // an empty string rather than reading a null pointer.
    pycc_rt_exception_clear();
    pycc_rt_exception_raise(pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        std::ptr::null(),
        0,
        std::ptr::null_mut(),
    ));
    len = 7;
    assert_eq!(
        pycc_rt_ext_pending_type(),
        i32::from(EXCEPTION_TYPE_VALUE_ERROR)
    );
    assert!(unsafe { pycc_rt_ext_pending_message(&raw mut len) }.is_null());
    assert_eq!(len, 0);

    // An `active` flag with a null value is not a pending exception.
    set_state(ExceptionState {
        active: 1,
        value: std::ptr::null_mut(),
    });
    len = 7;
    assert_eq!(pycc_rt_ext_pending_type(), -1);
    assert!(unsafe { pycc_rt_ext_pending_message(&raw mut len) }.is_null());
    assert_eq!(len, 0);
    pycc_rt_exception_clear();
}

#[test]
fn class_name_round_trips_through_the_exception_object() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "MyError", "boom");
    assert_eq!(exception_type_name(unsafe { &*obj }), "MyError");
}

#[test]
fn a_nameless_exception_object_reports_the_root_class() {
    let obj = pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        std::ptr::null(),
        0,
        alloc_exception_message("boom"),
    );
    assert_eq!(exception_type_name(unsafe { &*obj }), "Exception");
}

#[test]
fn a_non_utf8_class_name_reports_the_root_class() {
    // Codegen never emits one; the runtime still must not panic on it.
    const INVALID: [u8; 2] = [0xff, 0xfe];
    let obj = pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        INVALID.as_ptr(),
        INVALID.len(),
        alloc_exception_message("boom"),
    );
    assert_eq!(exception_type_name(unsafe { &*obj }), "Exception");
}

#[test]
fn exception_message_returns_the_borrowed_message_pointer_unretained() {
    // Part 3A of #541 (#736): `pycc_rt_exception_message` returns the
    // exact `message` field, borrowed and unretained, matching this
    // module's own `bytes()`-comparison convention used elsewhere in
    // this file (e.g. `class_name_round_trips_through_the_exception_object`
    // sibling tests).
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    let message = unsafe { pycc_rt_exception_message(obj) };
    assert_eq!(unsafe { (*message).bytes() }, b"boom");
    assert_eq!(message, unsafe { (*obj).message });
}

// -- #707: raise-site frame recording and traceback rendering.

#[test]
fn a_freshly_allocated_exception_carries_no_frame() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    assert!(unsafe { (*obj).frame_function }.is_null());
    assert_eq!(unsafe { (*obj).frame_function_len }, 0);
    assert_eq!(exception_frame_function(unsafe { &*obj }), None);
}

#[test]
fn set_frame_records_the_function_name() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    const FRAME: &str = "do_thing";
    unsafe { pycc_rt_exception_set_frame(obj, FRAME.as_ptr(), FRAME.len()) };
    assert_eq!(exception_frame_function(unsafe { &*obj }), Some(FRAME));
}

#[test]
fn set_frame_on_a_null_object_is_a_no_op() {
    // Mirrors `pycc_rt_exception_raise_with_cause`'s own tolerance for a
    // null primary object -- codegen never emits this, but the runtime
    // contract stays defined rather than undefined behavior.
    unsafe { pycc_rt_exception_set_frame(std::ptr::null_mut(), std::ptr::null(), 0) };
}

#[test]
fn a_non_utf8_frame_name_reports_unknown() {
    const INVALID: [u8; 2] = [0xff, 0xfe];
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    unsafe { pycc_rt_exception_set_frame(obj, INVALID.as_ptr(), INVALID.len()) };
    assert_eq!(
        exception_frame_function(unsafe { &*obj }),
        Some("<unknown>")
    );
}

#[test]
fn render_single_exception_with_no_frame_matches_the_pre_707_one_liner() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    assert_eq!(
        render_single_exception(unsafe { &*obj }),
        "ValueError: boom"
    );
}

#[test]
fn render_single_exception_with_a_frame_shows_a_traceback_block() {
    let obj = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "boom");
    const FRAME: &str = "do_thing";
    unsafe { pycc_rt_exception_set_frame(obj, FRAME.as_ptr(), FRAME.len()) };
    assert_eq!(
        render_single_exception(unsafe { &*obj }),
        "Traceback (most recent call last):\n  File \"<compiled>\", in do_thing\nRuntimeError: boom"
    );
}

#[test]
fn render_single_exception_with_no_message_omits_the_colon() {
    let obj = pycc_rt_exception_alloc(
        EXCEPTION_TYPE_VALUE_ERROR,
        "ValueError".as_ptr(),
        "ValueError".len(),
        std::ptr::null_mut(),
    );
    assert_eq!(render_single_exception(unsafe { &*obj }), "ValueError");
}

/// #1369: an empty message renders like no message at all, matching
/// CPython's bare `ValueError` for `raise ValueError("")`.
#[test]
fn render_single_exception_with_an_empty_message_omits_the_colon() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "");
    assert_eq!(render_single_exception(unsafe { &*obj }), "ValueError");
}

#[test]
fn render_exception_chain_with_no_cause_matches_render_single_exception() {
    let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "boom");
    assert_eq!(
        render_exception_chain(unsafe { &*obj }),
        render_single_exception(unsafe { &*obj })
    );
}

#[test]
fn render_exception_chain_shows_the_direct_cause_separator_oldest_first() {
    let cause = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "cause");
    const CAUSE_FRAME: &str = "inner";
    unsafe { pycc_rt_exception_set_frame(cause, CAUSE_FRAME.as_ptr(), CAUSE_FRAME.len()) };
    let effect = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "effect");
    const EFFECT_FRAME: &str = "outer";
    unsafe { pycc_rt_exception_set_frame(effect, EFFECT_FRAME.as_ptr(), EFFECT_FRAME.len()) };
    unsafe { (*effect).cause.set(cause) };
    let rendered = render_exception_chain(unsafe { &*effect });
    let expected = format!(
        "{}\n\nThe above exception was the direct cause of the following exception:\n\n{}",
        render_single_exception(unsafe { &*cause }),
        render_single_exception(unsafe { &*effect })
    );
    assert_eq!(rendered, expected);
    // The cause's own block -- printed first -- must appear before the
    // effect's, matching CPython's oldest-first chained rendering.
    assert!(
        rendered.find("ValueError: cause").unwrap()
            < rendered.find("RuntimeError: effect").unwrap()
    );
}

#[test]
fn render_exception_chain_walks_a_multi_level_cause_chain() {
    // A `from` chain nested two levels deep (`raise C from B`, itself
    // caught from `raise B from A`) exercises `render_exception_chain`'s
    // iterative walk beyond a single hop, following the `.cause` chain
    // two levels deep.
    let root = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "root");
    let middle = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "middle");
    unsafe { (*middle).cause.set(root) };
    let leaf = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "leaf");
    unsafe { (*leaf).cause.set(middle) };
    let rendered = render_exception_chain(unsafe { &*leaf });
    let root_pos = rendered.find("ValueError: root").unwrap();
    let middle_pos = rendered.find("TypeError: middle").unwrap();
    let leaf_pos = rendered.find("RuntimeError: leaf").unwrap();
    assert!(root_pos < middle_pos);
    assert!(middle_pos < leaf_pos);
    assert_eq!(
        rendered
            .matches("The above exception was the direct cause of the following exception:")
            .count(),
        2
    );
}

#[test]
fn render_exception_chain_breaks_a_self_cycle_instead_of_overflowing() {
    // `except ValueError as e: raise e from e` is valid Python source
    // and sets `e.cause = e`. Rendering must detect the repeat and stop
    // instead of recursing forever.
    let exc = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "self cycle");
    unsafe { (*exc).cause.set(exc) };
    let rendered = render_exception_chain(unsafe { &*exc });
    assert_eq!(rendered, render_single_exception(unsafe { &*exc }));
    assert!(!rendered.contains("direct cause"));
}

#[test]
fn render_exception_chain_breaks_a_two_node_cycle() {
    // A longer cycle is reachable through two `raise ... from`
    // statements: `e1.cause = e2` and, separately, `e2.cause = e1`.
    let e1 = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "e1");
    let e2 = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "e2");
    unsafe { (*e1).cause.set(e2) };
    unsafe { (*e2).cause.set(e1) };
    let rendered = render_exception_chain(unsafe { &*e1 });
    let expected = format!(
        "{}\n\nThe above exception was the direct cause of the following exception:\n\n{}",
        render_single_exception(unsafe { &*e2 }),
        render_single_exception(unsafe { &*e1 })
    );
    assert_eq!(rendered, expected);
    assert_eq!(
        rendered
            .matches("The above exception was the direct cause of the following exception:")
            .count(),
        1
    );
}

#[test]
fn render_exception_chain_renders_a_non_cyclic_prefix_before_a_later_cycle() {
    // The cycle's entry point (`b`) is not the chain's head (`a`):
    // `a.cause = b; b.cause = c; c.cause = b`. Every distinct node
    // reachable before the walk re-visits `b` must still render exactly
    // once, oldest first, per `render_exception_chain`'s own doc
    // comment -- this is stricter than only covering a cycle rooted at
    // the traversal's start.
    let a = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "a");
    let b = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "b");
    let c = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "c");
    unsafe { (*a).cause.set(b) };
    unsafe { (*b).cause.set(c) };
    unsafe { (*c).cause.set(b) };
    let rendered = render_exception_chain(unsafe { &*a });
    let expected = format!(
        "{}\n\nThe above exception was the direct cause of the following exception:\n\n{}\n\nThe above exception was the direct cause of the following exception:\n\n{}",
        render_single_exception(unsafe { &*c }),
        render_single_exception(unsafe { &*b }),
        render_single_exception(unsafe { &*a })
    );
    assert_eq!(rendered, expected);
}

#[test]
fn state_starts_clear_and_clear_resets_value() {
    pycc_rt_exception_clear();
    assert_eq!(pycc_rt_exception_active(), 0);
    assert!(pycc_rt_exception_value().is_null());
    let obj = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "clear test");
    pycc_rt_exception_raise(obj);
    assert_eq!(pycc_rt_exception_active(), 1);
    assert_eq!(pycc_rt_exception_value(), obj);
    pycc_rt_exception_clear();
    assert_eq!(pycc_rt_exception_active(), 0);
    assert!(pycc_rt_exception_value().is_null());
}

#[test]
fn state_is_isolated_between_threads() {
    pycc_rt_exception_clear();
    let child = std::thread::spawn(|| {
        let obj = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "child");
        pycc_rt_exception_raise(obj);
        assert_eq!(pycc_rt_exception_value(), obj);
        assert_eq!(pycc_rt_exception_active(), 1);
    });
    child.join().unwrap();
    assert_eq!(pycc_rt_exception_active(), 0);
    assert!(pycc_rt_exception_value().is_null());
}

#[test]
fn type_matching_is_exact_except_for_exception_root() {
    let obj = alloc_named(EXCEPTION_TYPE_KEY_ERROR, "KeyError", "key");
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_KEY_ERROR) },
        1
    );
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_VALUE_ERROR) },
        0
    );
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(obj, EXCEPTION_TYPE_EXCEPTION) },
        1
    );
    assert_eq!(
        unsafe { pycc_rt_exception_type_matches(std::ptr::null_mut(), 0) },
        0
    );
}

#[test]
fn explicit_cause_and_default_fields_are_preserved() {
    pycc_rt_exception_clear();
    let cause = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "cause");
    let exc = alloc_named(EXCEPTION_TYPE_RUNTIME_ERROR, "RuntimeError", "effect");
    assert_eq!(unsafe { (*(*exc).message).bytes() }, b"effect");
    assert!(unsafe { (*exc).cause.get() }.is_null());
    unsafe { pycc_rt_exception_raise_with_cause(exc, cause) };
    assert_eq!(unsafe { (*exc).cause.get() }, cause);
    assert_eq!(pycc_rt_exception_value(), exc);
    pycc_rt_exception_clear();
}

#[test]
fn raise_with_cause_accepts_a_null_primary_object() {
    unsafe { pycc_rt_exception_raise_with_cause(std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(pycc_rt_exception_active(), 1);
    assert!(pycc_rt_exception_value().is_null());
    pycc_rt_exception_clear();
}

#[test]
fn message_allocation_covers_inline_and_heap_payloads() {
    let inline = alloc_exception_message("short");
    assert_eq!(unsafe { (*inline).bytes() }, b"short");
    let long = "this is a long exception message that exceeds 22 bytes";
    let heap = alloc_exception_message(long);
    assert_eq!(unsafe { (*heap).bytes() }, long.as_bytes());
}

#[test]
fn builtin_raise_sets_the_requested_tag() {
    pycc_rt_exception_clear();
    raise_builtin(EXCEPTION_TYPE_ZERO_DIV_ERROR, "ZeroDivisionError", "zero");
    let obj = pycc_rt_exception_value();
    assert_eq!(unsafe { (*obj).type_tag }, EXCEPTION_TYPE_ZERO_DIV_ERROR);
    pycc_rt_exception_clear();
}

// -- Part 3 of #382 (#542, PEP 654): exception-group construction and
// -- partition tests, written before `emit_try_star` (TDD, per this
// -- issue's implementation plan).

const GROUP_TAG: u8 = 24; // `ExceptionGroup`'s fixed builtin tag.
const GROUP_NAME: &str = "ExceptionGroup";

fn group_of(members: &[*mut PyExceptionObj]) -> *mut PyExceptionObj {
    unsafe {
        pycc_rt_exception_group_alloc(
            GROUP_TAG,
            GROUP_NAME.as_ptr(),
            GROUP_NAME.len(),
            alloc_exception_message("group"),
            members.as_ptr(),
            members.len(),
        )
    }
}

fn partition(
    group: *mut PyExceptionObj,
    tags: &[u8],
) -> (*mut PyExceptionObj, *mut PyExceptionObj) {
    let mut matched = std::ptr::null_mut();
    let mut rest = std::ptr::null_mut();
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
    (matched, rest)
}

#[test]
fn group_alloc_copies_the_member_array_and_reports_its_length() {
    let a = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "a");
    let b = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "b");
    let group = group_of(&[a, b]);
    unsafe {
        assert_eq!((*group).type_tag, GROUP_TAG);
        assert_eq!((*group).exceptions_len, 2);
        let members = std::slice::from_raw_parts((*group).exceptions, 2);
        assert_eq!(members, [a, b]);
    }
}

#[test]
fn group_alloc_with_no_members_leaves_a_null_exceptions_pointer() {
    let group = group_of(&[]);
    unsafe {
        assert_eq!((*group).exceptions_len, 0);
        assert!((*group).exceptions.is_null());
    }
}

#[test]
fn partition_splits_a_group_into_matched_and_unmatched_subgroups() {
    let value_err = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "v");
    let type_err = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "t");
    let key_err = alloc_named(EXCEPTION_TYPE_KEY_ERROR, "KeyError", "k");
    let group = group_of(&[value_err, type_err, key_err]);
    let (matched, rest) = partition(
        group,
        &[EXCEPTION_TYPE_VALUE_ERROR, EXCEPTION_TYPE_KEY_ERROR],
    );
    unsafe {
        assert!(!matched.is_null());
        assert_eq!((*matched).type_tag, GROUP_TAG);
        assert_eq!((*matched).exceptions_len, 2);
        assert_eq!(
            std::slice::from_raw_parts((*matched).exceptions, 2),
            [value_err, key_err]
        );
        assert!(!rest.is_null());
        assert_eq!((*rest).exceptions_len, 1);
        assert_eq!(
            std::slice::from_raw_parts((*rest).exceptions, 1),
            [type_err]
        );
    }
}

#[test]
fn partition_where_every_member_matches_leaves_rest_null() {
    let value_err = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "v");
    let group = group_of(&[value_err]);
    let (matched, rest) = partition(group, &[EXCEPTION_TYPE_VALUE_ERROR]);
    unsafe {
        assert!(!matched.is_null());
        assert_eq!((*matched).exceptions_len, 1);
    }
    assert!(rest.is_null());
}

#[test]
fn partition_where_no_member_matches_leaves_matched_null() {
    let value_err = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "v");
    let group = group_of(&[value_err]);
    let (matched, rest) = partition(group, &[EXCEPTION_TYPE_KEY_ERROR]);
    assert!(matched.is_null());
    unsafe {
        assert!(!rest.is_null());
        assert_eq!((*rest).exceptions_len, 1);
    }
}

#[test]
fn partition_with_an_empty_tag_set_matches_every_member_like_bare_except_star() {
    let value_err = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "v");
    let type_err = alloc_named(EXCEPTION_TYPE_TYPE_ERROR, "TypeError", "t");
    let group = group_of(&[value_err, type_err]);
    let (matched, rest) = partition(group, &[]);
    unsafe {
        assert!(!matched.is_null());
        assert_eq!((*matched).exceptions_len, 2);
    }
    assert!(rest.is_null());
}

#[test]
fn partition_with_the_exception_root_tag_matches_every_member() {
    // `EXCEPTION_TYPE_EXCEPTION` is the universal catch-all, consistent
    // with `pycc_rt_exception_type_matches`: a handler for it catches
    // every member regardless of that member's own tag.
    let value_err = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "v");
    let key_err = alloc_named(EXCEPTION_TYPE_KEY_ERROR, "KeyError", "k");
    let group = group_of(&[value_err, key_err]);
    let (matched, rest) = partition(group, &[EXCEPTION_TYPE_EXCEPTION]);
    unsafe {
        assert!(!matched.is_null());
        assert_eq!((*matched).exceptions_len, 2);
    }
    assert!(rest.is_null());
}

#[test]
fn partition_treats_a_non_group_exception_as_an_implicit_single_member() {
    // An ordinary exception (`exceptions_len == 0`) raised inside a
    // `try*` body -- not already an `ExceptionGroup` -- is still caught
    // by a matching `except*` clause, wrapped as a single-member group.
    let plain = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "plain");
    assert_eq!(unsafe { (*plain).exceptions_len }, 0);
    let (matched, rest) = partition(plain, &[EXCEPTION_TYPE_VALUE_ERROR]);
    unsafe {
        assert!(!matched.is_null());
        assert_eq!((*matched).exceptions_len, 1);
        assert_eq!(
            std::slice::from_raw_parts((*matched).exceptions, 1),
            [plain]
        );
    }
    assert!(rest.is_null());
}

#[test]
fn partition_of_a_non_group_exception_that_does_not_match_leaves_it_in_rest() {
    let plain = alloc_named(EXCEPTION_TYPE_VALUE_ERROR, "ValueError", "plain");
    let (matched, rest) = partition(plain, &[EXCEPTION_TYPE_KEY_ERROR]);
    assert!(matched.is_null());
    unsafe {
        assert!(!rest.is_null());
        assert_eq!(std::slice::from_raw_parts((*rest).exceptions, 1), [plain]);
    }
}

/// A `NameError` names a dependency module's global by its plain name.
#[test]
fn a_qualified_global_is_named_by_its_plain_name() {
    assert_eq!(plain_global_name("0m6config_READY"), "READY");
    assert_eq!(plain_global_name("0m3pkg3sub_X"), "X");
    assert_eq!(plain_global_name("0m1a__pycc_global_N"), "_pycc_global_N");
    assert_eq!(plain_global_name("count"), "count");
    assert_eq!(plain_global_name("0m9short_X"), "0m9short_X");
}
