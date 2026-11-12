//! The lowerings of docs/design/compile-little.md sections 2.6 and 2.7 in
//! generic bodies, built `--ext --non-strict --select all`:
//!
//! - a dict miss caught by a handler that neither binds nor re-raises it goes
//!   straight to the handler (`tests/fixtures/lazy_key_error.py`);
//! - `tuple(a)`, `list(a)` and `sorted(a)` skip the call through the builtin
//!   (`tests/fixtures/sequence_builtins.py`), unless it was rebound
//!   (`tests/fixtures/sequence_builtins_rebound.py`);
//! - a comprehension that unpacks its item over a dict's own `items()`
//!   releases each item before it advances
//!   (`tests/fixtures/comprehension_item_release.py`);
//! - a comprehension that unpacks its item into two names binds them
//!   straight from the source, an exact dict's entries with no pair made,
//!   `sorted()` of a generator expression sorts its own list, `tuple()` of
//!   a list only it holds moves the items, and a tuple display stores its
//!   items in place (`tests/fixtures/dict_pairs.py`).
//!
//! Each module prints what CPython prints for its source, with the
//! lowerings on (their counters move) and off (`PYCC_NO_FAST_OPS=1`: every
//! miss raises, every call goes through the builtin, every pair is read
//! through the items iterator, and the counters stay at zero).

#[path = "harness_support/ext_oracle.rs"]
mod ext_oracle;
use ext_oracle::check;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dict_miss_caught_unseen_matches_cpython_with_the_lowering_on_and_off() {
    check(
        "lazy_key_error",
        &[
            "plain",
            "chained",
            "bound",
            "reraised",
            "index_first",
            "with_else_finally",
            "nested_both",
            "temporaries",
            "handler_raises",
            "temporary_key",
            "dropped_key",
            "temporary_keys_in_loop",
            "temporary_key_return_in_loop",
            "temporary_key_recursive",
            "meddling_key",
            "main",
        ],
        Some(("pycc_ext_lazy_misses", true)),
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn tuple_list_and_sorted_match_cpython_with_the_lowering_on_and_off() {
    check(
        "sequence_builtins",
        &[
            "sorted_comprehension",
            "tuple_sorted_generator",
            "list_of_call",
            "sorted_kept",
            "list_kept",
            "sorted_key_reverse",
            "sorted_compare_fails",
            "identity",
            "stable",
            "sorted_fails_finalized",
            "sorted_mixed_finalized",
            "sorted_spied",
            "sorted_grabbed",
            "identities",
            "failing_sorted",
            "list_finalized",
        ],
        Some(("pycc_ext_direct_builtins", true)),
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn rebound_sorted_list_and_tuple_are_called() {
    check(
        "sequence_builtins_rebound",
        &["calls", "main"],
        Some(("pycc_ext_direct_builtins", false)),
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_comprehension_over_dict_items_matches_cpython_with_the_lowering_on_and_off() {
    check(
        "comprehension_item_release",
        &[
            "over_generator",
            "over_iterator",
            "items_pairs",
            "items_kept",
            "items_nested",
            "items_values_replaced",
            "items_whole_replaced",
            "items_whole_named_like_a_temporary",
            "items_filtered_replaced",
            "items_fails",
            "main",
        ],
        Some(("pycc_ext_dict_pair_steps", true)),
        Some("replaced"),
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn two_name_comprehensions_sorted_tuple_and_displays_match_cpython_with_the_lowering_on_and_off() {
    check(
        "dict_pairs",
        &[
            "pairs",
            "keys_only",
            "values_filtered",
            "swapped",
            "value_set",
            "used_twice",
            "nested",
            "grown",
            "shrunk",
            "keys_changed",
            "values_replaced",
            "body_fails",
            "subclass",
            "plain_subclass",
            "ordered",
            "mapping",
            "over_list",
            "over_iterator",
            "over_zip",
            "over_pairish",
            "sorted_pairs",
            "sorted_generator",
            "sorted_generator_fails",
            "sorted_key_reverse",
            "generator_raises",
            "tuple_generator_raises",
            "sorted_spied_generator",
            "sorted_spied_named_like_fresh",
            "tuple_of_listcomp",
            "tuple_of_sorted",
            "displays",
            "display_fails",
            "display_item_fails",
            "shadowed_sorted",
            "shadowed_tuple",
            "shadowed_isinstance",
            "hashable",
            "main",
        ],
        Some(("pycc_ext_dict_pair_steps", true)),
        Some("replaced"),
    );
}
