//! Strict-mode refusals the differential fuzzer counted most often (v0.6),
//! now compiled natively. Each fixture is built in strict mode, with
//! `--non-strict` and as an `--ext` module, and every run is compared with
//! the live CPython oracle (`PYCC_PYTHON`, default `python3`).

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

fn matches_cpython(name: &str) {
    assert_native_matches(&format!("{name}.py"));
    assert_ext_matches(&format!("{name}.py"), &format!("import {name}"));
}

/// `break` and `continue` out of a `try` with a `finally:` clause run the
/// `finally` block first, through handlers, `else:`, nested `try`s, loop
/// `else:` clauses and `match` cases.
#[test]
fn jumps_run_the_finally_block_first() {
    matches_cpython("jump_through_finally");
}

/// The f-string `=` debug specifier, with and without a conversion or a
/// format spec.
#[test]
fn fstring_debug_specifier() {
    matches_cpython("fstring_debug");
}

/// `itertools.takewhile`/`dropwhile` with a lambda or a function predicate.
#[test]
fn takewhile_and_dropwhile() {
    matches_cpython("itertools_while");
}

/// A `match` on a tuple subject checks each sequence-pattern position
/// against its own element type, and a starred capture of a tuple of one
/// element type is a list.
#[test]
fn match_on_a_tuple_subject() {
    matches_cpython("match_tuple_subject");
}
