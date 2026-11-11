//! Exceptions in non-strict generic bodies, built `--ext --non-strict
//! --select all`: each module prints what CPython prints for its source,
//! with the fast paths on and off (`PYCC_NO_FAST_OPS=1`).
//!
//! - a value an exception edge releases may run compiled code (a `__del__`),
//!   which neither sees the exception nor loses it
//!   (`tests/fixtures/unwinding_finalizers.py`);
//! - a comprehension over an object releases its exhausted iterator before
//!   its last item, and, when its element or condition raises, its iterator,
//!   its result and then the item (`tests/fixtures/comprehension_unwinding.py`);
//! - an exception passing a `try` statement adds a traceback entry where
//!   it is raised, and none for an unmatched handler, a bare `raise` or a
//!   `finally` clause (`tests/fixtures/traceback_entries.py`);
//! - `"..." % e` of a caught exception is CPython's to format: the function
//!   is not compiled (`tests/fixtures/exception_operands.py`);
//! - an `except` clause publishes the exception it handles: `sys.exception()`
//!   sees it, what the clause raises gets it as its `__context__`, and a
//!   dict miss that went straight to its handler makes its `KeyError` when
//!   something could see it (`tests/fixtures/handled_exception.py`).

#[path = "harness_support/ext_oracle.rs"]
mod ext_oracle;
use ext_oracle::check;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_finalizer_run_while_an_exception_unwinds_leaves_it_pending() {
    check(
        "unwinding_finalizers",
        &[
            "Fin.__del__",
            "Busy.__del__",
            "sorted_mixed",
            "sorted_finalized",
            "sorted_finalized_escapes",
            "nested_handler_raises",
            "nested_handler_reraises",
            "handler_raises_with_key",
            "local_released",
            "temporary_released",
            "busy_finalizer",
            "finally_released",
            "main",
        ],
        None,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_comprehension_releases_its_iterator_and_item_as_cpython_does() {
    check(
        "comprehension_unwinding",
        &[
            "Fin.__del__",
            "Source.__del__",
            "names_of",
            "name_set",
            "name_dict",
            "pairs_of",
            "filtered",
            "element_raises",
            "element_raises_escapes",
            "condition_raises",
            "key_raises",
            "tuple_element_raises",
            "over_list",
            "miss_in_comprehension",
            "twice",
            "items_kept",
            "items_kept_in_set",
            "items_kept_from_generator",
        ],
        None,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_exception_passing_a_try_statement_adds_no_traceback_entry() {
    check(
        "traceback_entries",
        &[
            "fails",
            "unmatched",
            "unmatched_call",
            "bare",
            "bare_after_work",
            "named",
            "replaced",
            "through_finally",
            "unmatched_then_finally",
            "nested",
            "in_else",
            "in_loop",
            "star_unmatched",
            "star_unmatched_then_finally",
            "main",
        ],
        None,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_exception_is_formatted_by_cpython_not_read_as_another_object() {
    check("exception_operands", &["native"], None, None);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_handler_publishes_the_exception_it_handles() {
    check(
        "handled_exception",
        &[
            "seen",
            "info",
            "bound",
            "restored",
            "nested",
            "in_loop",
            "returns_from_handler",
            "after_return",
            "raises_new",
            "raises_object",
            "raises_from_none",
            "raises_from_cause",
            "raises_from_bound_cause",
            "raises_from_handled",
            "reraises",
            "reraises_named",
            "callee_raises",
            "operation_raises",
            "caught_inside",
            "three_deep",
            "raises_saved",
            "raises_with_key",
            "handler_in_callee",
            "config",
            "Probe.__eq__",
            "Watcher.__del__",
            "default_seen",
            "temporary_in_handler",
            "exception_type",
            "bare_handler",
            "second_handler",
        ],
        // The lazy misses still happen: `default_seen` and `config` make no
        // exception on their way to a default.
        Some(("pycc_ext_lazy_misses", true)),
        None,
    );
}
