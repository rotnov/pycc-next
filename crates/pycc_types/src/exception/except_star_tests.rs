//! Type-checking tests for `except*`/`ExceptionGroup`/`BaseExceptionGroup`
//! (Part 3 of #382, #542, PEP 654, D-202).
//!
//! In its own submodule rather than appended to the crate's `tests.rs`, per
//! AGENTS.md's decomposability rule -- mirroring `synthetic_class_tests` and
//! `user_class_tests` in this same directory.
//!
//! These tests exercise [`check_exception_group_operand`] and
//! [`check_exception_group_member_operand`] (and the `ExceptionGroup`/
//! `BaseExceptionGroup` dispatch branch of [`check_raise_operand`]) through
//! real parsed source and the crate's public [`crate::check`] entry point.
//! They exist specifically so this crate's *own* `cargo test -p pycc_types
//! --lib` compilation -- a separate rustc invocation, and therefore a
//! separate instrumented binary, from the one produced for the workspace's
//! other crates and integration tests -- actually calls these functions:
//! `cargo llvm-cov`'s file-level region/line summary is computed per
//! monomorphized-function instance, so a function reachable only through
//! `tests/issue_542_except_star.rs` (linked against the *other* build of
//! this crate) still reports as a missed region/line for this crate's own
//! instance, even though every per-line rendering view (`--text`, `--html`,
//! the raw JSON `segments` array) shows the source as fully covered once the
//! two instances are considered together.

use super::*;
use crate::check;

fn check_source(source: &str) -> Result<(), Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check(&hir)
}

fn expect_error(source: &str) -> Diagnostic {
    check_source(source).expect_err("source must be rejected")
}

const CAUGHT_PAIR: &str = "def main() -> None:\n\
                            \x20   try:\n\
                            \x20       raise ValueError(\"v\")\n\
                            \x20   except ValueError as e1:\n\
                            \x20       try:\n\
                            \x20           raise TypeError(\"t\")\n\
                            \x20       except TypeError as e2:\n";

#[test]
fn raising_an_exception_group_with_existing_members_is_accepted() {
    check_source(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"multi\", [e1, e2])\n"
    ))
    .expect("an ExceptionGroup built from existing exception bindings must be accepted");
}

#[test]
fn raising_a_base_exception_group_with_existing_members_is_accepted() {
    check_source(&format!(
        "{CAUGHT_PAIR}\x20           raise BaseExceptionGroup(\"multi\", [e1, e2])\n"
    ))
    .expect("BaseExceptionGroup must be accepted exactly like ExceptionGroup");
}

#[test]
fn an_exception_group_with_a_single_member_is_accepted() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"v\")\n\
         \x20   except ValueError as e:\n\
         \x20       raise ExceptionGroup(\"single\", [e])\n",
    )
    .expect("a single-member ExceptionGroup must be accepted");
}

#[test]
fn catching_an_exception_group_member_with_except_star_is_accepted() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"caught\")\n",
    )
    .expect("except* over a builtin exception type must be accepted");
}

#[test]
fn an_except_star_handler_may_bind_the_caught_group_with_as() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError as eg:\n\
         \x20       print(\"caught\")\n",
    )
    .expect("except* ... as must be accepted, binding an ExceptionGroup instance");
}

#[test]
fn except_star_with_finally_is_accepted() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"handled\")\n\
         \x20   finally:\n\
         \x20       print(\"cleanup\")\n",
    )
    .expect("except* with a finally block must be accepted");
}

#[test]
fn except_star_with_else_is_accepted() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       print(\"body\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"handled\")\n\
         \x20   else:\n\
         \x20       print(\"ran\")\n",
    )
    .expect("except* with an else block must be accepted");
}

#[test]
fn multiple_except_star_handlers_are_accepted() {
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* TypeError:\n\
         \x20       print(\"wrong type\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"dispatched\")\n",
    )
    .expect("multiple except* handlers on the same try must be accepted");
}

#[test]
fn an_exception_group_constructor_call_with_the_wrong_argument_count_is_rejected() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"only one arg\")\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("expects exactly 2 arguments"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_exception_group_with_a_non_str_message_is_rejected() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(1, [e1, e2])\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("expects a `str` message argument"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_exception_group_whose_second_argument_is_not_a_literal_list_is_rejected() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           members = e1\n\
         \x20           raise ExceptionGroup(\"multi\", members)\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("must be a literal list of member exceptions"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_exception_group_with_no_members_is_rejected() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"empty\", [])\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("requires at least one member exception"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_fresh_builtin_exception_constructor_call_is_not_a_valid_group_member() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"multi\", [ValueError(\"fresh\")])\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("not a fresh"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_fresh_user_exception_constructor_call_is_not_a_valid_group_member() {
    let diagnostic = expect_error(
        "class AppError(Exception):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"v\")\n\
         \x20   except ValueError as e:\n\
         \x20       raise ExceptionGroup(\"multi\", [e, AppError(\"fresh\")])\n",
    );
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("not a fresh"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_exception_group_valued_binding_is_not_a_valid_group_member() {
    // D-202's sixth simplification: `pycc_rt_exception_group_partition`
    // matches each member by its own top-level `type_tag` only and never
    // recurses into a member's own `exceptions`/`exceptions_len` when that
    // member is itself a group, so a value already bound to
    // `ExceptionGroup`/`BaseExceptionGroup` -- such as an
    // `except* ... as eg:` binding -- must be rejected as a group member
    // rather than silently accepted into an unpartitionable nested group.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"v\")\n\
         \x20   except* ValueError as eg:\n\
         \x20       raise ExceptionGroup(\"outer\", [eg])\n",
    );
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("must not itself be"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_non_exception_group_member_is_rejected() {
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"multi\", [e1, 3])\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("must be an exception instance"),
        "unexpected message: {}",
        diagnostic.message
    );
}

// -- `check_try_star_stmt`'s own `?` error-propagation branches --
//
// Every test below exercises a distinct `?` inside `check_try_star_stmt`
// that -- like the functions above -- is reachable through the workspace's
// end-to-end `tests/issue_542_except_star.rs` integration suite (a
// different compiled instance of this crate) but was never reached by this
// crate's own `cargo test -p pycc_types --lib` binary, and therefore still
// reported as a missed region/line for that instance under `cargo llvm-cov
// --workspace`'s D-014 gate even though the merged per-source-line view
// (`--text`/`--html`) shows every one of these lines as covered.

#[test]
fn a_type_error_inside_a_try_star_body_propagates_out() {
    // The `?` on `check_stmt_sequence_shared` for the try body.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       x = 1 + \"bad\"\n\
         \x20   except* ValueError:\n\
         \x20       print(\"handled\")\n",
    );
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_except_star_handler_naming_an_unrecognized_class_is_rejected() {
    // The `else` branch of `let Some(def) = user_exception_class(...) else`:
    // `Point` is neither a builtin exception nor a user exception class.
    let diagnostic = expect_error(
        "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* Point:\n\
         \x20       print(\"caught\")\n",
    );
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("is not a recognized exception class"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_except_star_handler_naming_a_class_with_its_own_constructor_is_rejected() {
    // `reject_own_constructor`'s `?` inside the handler-type-checking loop.
    let diagnostic = expect_error(
        "class AppError(Exception):\n\
         \x20   def __init__(self, code: int) -> None:\n\
         \x20       self.code = code\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* AppError:\n\
         \x20       print(\"caught\")\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("declares or inherits an `__init__` other than"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_type_error_inside_a_try_star_handler_body_propagates_out() {
    // The `?` on `check_stmt_sequence_shared` for a handler body.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError:\n\
         \x20       x = 1 + \"bad\"\n",
    );
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_type_error_inside_a_try_star_else_body_propagates_out() {
    // The `?` on `check_stmt_sequence_shared` for the else body.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       print(\"body\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"handled\")\n\
         \x20   else:\n\
         \x20       x = 1 + \"bad\"\n",
    );
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_type_error_inside_a_try_star_finally_body_propagates_out() {
    // The `?` on `check_stmt_sequence_shared` for the finally body.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError:\n\
         \x20       print(\"handled\")\n\
         \x20   finally:\n\
         \x20       x = 1 + \"bad\"\n",
    );
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_try_star_handler_with_incompatible_binding_is_t0023() {
    // A handler that rebinds a pre-existing variable to an incompatible
    // type is rejected -- mirroring
    // `try_handler_with_incompatible_binding_is_t0023` in `tests.rs` for
    // the plain `try`/`except` case. This specific reassignment
    // (`x = "bad"` inside the handler body) is a same-block rebinding, so
    // it is actually caught directly by `check_assignment` (via
    // `check_stmt_sequence_shared`) before `check_try_star_stmt` ever reaches its
    // own `join_if_branches` call -- see
    // `try_star_handler_as_binding_incompatible_type_is_t0023` in
    // `tests.rs` for a fixture that reaches `join_if_branches` itself,
    // via an `except* ... as` binding (which bypasses `check_assignment`).
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   x = 1\n\
         \x20   try:\n\
         \x20       raise ValueError(\"bad\")\n\
         \x20   except* ValueError:\n\
         \x20       x = \"bad\"\n",
    );
    assert_eq!(diagnostic.code, "T0023");
}

#[test]
fn an_exception_group_with_a_type_error_in_the_message_argument_propagates() {
    // The `?` on `infer_expr_in` for `check_exception_group_operand`'s
    // message argument -- the message expression itself fails to
    // type-check (as opposed to being well-typed but not a `str`, already
    // covered by `an_exception_group_with_a_non_str_message_is_rejected`).
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(1 + \"bad\", [e1, e2])\n"
    ));
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_exception_group_member_with_a_type_error_propagates() {
    // The `?` on `infer_expr_in` for `check_exception_group_member_operand`
    // -- the member expression itself fails to type-check (as opposed to
    // being well-typed but not an exception instance, already covered by
    // `a_non_exception_group_member_is_rejected`).
    let diagnostic = expect_error(&format!(
        "{CAUGHT_PAIR}\x20           raise ExceptionGroup(\"multi\", [e1, 1 + \"bad\"])\n"
    ));
    assert!(
        diagnostic.message.contains("operator"),
        "unexpected message: {}",
        diagnostic.message
    );
}

// -- #795 (PEP 654), gap 2: `except* ExceptionGroup:` / `except*
// BaseExceptionGroup:` --
//
// CPython accepts both at compile time and raises `TypeError: catching
// ExceptionGroup with except* is not allowed. Use except instead.` when the
// handler is matched. pycc has no materialized group value at match time
// (D-173 propagates a raised exception through global runtime state), so it
// rejects the program at compile time with `C0001` instead -- a deliberate,
// documented divergence. #903 tracks delivering the real runtime behavior.

#[test]
fn catching_an_exception_group_with_except_star_is_rejected() {
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* ExceptionGroup:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("catching `ExceptionGroup`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn catching_a_base_exception_group_with_except_star_is_rejected() {
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* BaseExceptionGroup:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("catching `BaseExceptionGroup`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_group_type_in_a_non_first_tuple_position_is_still_rejected() {
    // The check runs per element of a PEP 758 multi-type handler, not only
    // on the first name -- the loop's own iteration, not a special case.
    let diagnostic = expect_error(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* (ValueError, ExceptionGroup):\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("catching `ExceptionGroup`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn catching_a_plain_builtin_with_except_star_is_still_accepted() {
    // The `matches!` conjunct's false arm: an unshadowed builtin exception
    // that is *not* one of the two group names lowers and type-checks
    // exactly as before.
    check_source(
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* ValueError:\n\
         \x20       pass\n",
    )
    .expect("`except* ValueError:` must still be accepted");
}

#[test]
fn a_shadowed_exception_group_name_does_not_reach_the_group_rejection() {
    // The `builtin` conjunct's own false arm *for a group name*: a module
    // that rebinds `ExceptionGroup` no longer means the builtin, so the
    // handler falls through to the existing unrecognized-class path
    // (`T0021`) instead of this rejection. Without the `builtin` gate the
    // `C0001` above would fire on a name the program does not actually mean.
    let diagnostic = expect_error(
        "ExceptionGroup = 1\n\
         \n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* ExceptionGroup:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn a_user_subclass_of_exception_group_is_rejected() {
    // #795, second round: the exact-name check above never sees this name,
    // so the rejection has to come from the MRO scan. CPython raises the
    // same `TypeError` for a subclass as for the group class itself.
    let diagnostic = expect_error(
        "class G(ExceptionGroup):\n\
         \x20   pass\n\
         \n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* G:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("catching `G` with `except*`")
            && diagnostic
                .message
                .contains("it derives from `ExceptionGroup`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_user_subclass_of_base_exception_group_is_rejected() {
    // The MRO scan's second `matches!` alternative: `BaseExceptionGroup` is
    // the group root, so a subclass of it is refused for the same reason.
    let diagnostic = expect_error(
        "class G(BaseExceptionGroup):\n\
         \x20   pass\n\
         \n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* G:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("it derives from `BaseExceptionGroup`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn an_ordinary_user_exception_class_in_an_except_star_handler_is_still_accepted() {
    // The MRO scan's `None` arm: a class that never touches the group
    // hierarchy keeps compiling, so the new rejection cannot regress the
    // ordinary user-exception path.
    check_source(
        "class AppError(Exception):\n\
         \x20   pass\n\
         \n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* AppError:\n\
         \x20       pass\n",
    )
    .expect("an ordinary user exception class must still be accepted");
}

#[test]
fn a_subclass_of_a_shadowed_exception_group_class_reports_t0021() {
    // Why the MRO scan needs no shadow gate of its own: a module that
    // defines its own class under a builtin exception name withholds the
    // seeded builtin exception classes entirely, so nothing in it ever gets
    // an `exception_type_tag` and `except* G:` stops at the existing
    // unrecognized-class path (`T0021`) long before the scan runs. A group
    // name that does reach the scan is therefore always the builtin one.
    let diagnostic = expect_error(
        "class ExceptionGroup:\n\
         \x20   def __init__(self) -> None:\n\
         \x20       pass\n\
         \n\
         class G(ExceptionGroup):\n\
         \x20   pass\n\
         \n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       pass\n\
         \x20   except* G:\n\
         \x20       pass\n",
    );
    assert_eq!(diagnostic.code, "T0021");
}
