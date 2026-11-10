//! Exceptions in non-strict generic bodies, built `--ext --non-strict
//! --select all`: each module prints what CPython prints for its source,
//! with the fast paths on and off (`PYCC_NO_FAST_OPS=1`).
//!
//! - a value an exception edge releases may run compiled code (a `__del__`),
//!   which neither sees the exception nor loses it
//!   (`tests/fixtures/unwinding_finalizers.py`).

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
