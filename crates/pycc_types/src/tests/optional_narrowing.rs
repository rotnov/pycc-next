//! `Optional[T]` narrowing unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! tests that exercise the #769 flow-sensitive `Optional[T]` narrowing (part 2
//! of #747, D-205), so the split keeps one cohesive feature in one place. As a
//! child module this still sees the parent's private items directly through
//! `use super::*`, so nothing needed widened visibility; only the tests'
//! location changed.

use super::*;

// -- #769 (Part 2 of #747, D-205): flow-sensitive `Optional[T]` narrowing --

#[test]
fn is_not_none_narrows_the_body_to_plain_int() {
    let result =
        check_source("x: int | None = 5\nif x is not None:\n    y: int = x\n    print(y)\n");
    assert!(
        result.is_ok(),
        "`is not None` must narrow `x` to plain `int` inside the body: {result:?}"
    );
}

#[test]
fn is_none_narrows_the_orelse_to_plain_int() {
    let result = check_source(
        "x: int | None = 5\nif x is None:\n    print(0)\nelse:\n    y: int = x\n    print(y)\n",
    );
    assert!(
        result.is_ok(),
        "`is None` must narrow `x` to plain `int` inside the `orelse` branch: {result:?}"
    );
}

#[test]
fn is_none_does_not_narrow_the_body() {
    // The body of `if x is None:` is exactly the case where `x` is absent
    // -- it must stay `Optional[int]` there, never narrow to `int`.
    let result = check_source("x: int | None = 5\nif x is None:\n    y: int = x\n    print(y)\n");
    let err = result.expect_err("`x` inside `if x is None:`'s own body must stay Optional");
    assert_eq!(err.code, "T0025");
}

#[test]
fn is_not_none_does_not_narrow_the_orelse() {
    // The `orelse` of `if x is not None:` is exactly the case where `x` is
    // absent -- it must stay `Optional[int]` there.
    let result = check_source(
        "x: int | None = 5\nif x is not None:\n    print(0)\nelse:\n    y: int = x\n    print(y)\n",
    );
    let err =
        result.expect_err("`x` inside the `orelse` of `if x is not None:` must stay Optional");
    assert_eq!(err.code, "T0025");
}

#[test]
fn narrowing_does_not_leak_past_the_if_from_the_body_side() {
    // A plain use of `x` *after* the whole `if` statement must still see
    // `x` as `Optional[int]`, not the narrowed `int` -- the overlay must
    // not survive `join_if_branches`.
    let result = check_source(
        "x: int | None = 5\nif x is not None:\n    y: int = x\n    print(y)\ny2: int = x\nprint(y2)\n",
    );
    let err = result.expect_err("narrowing inside the body must not leak past the `if`");
    assert_eq!(err.code, "T0025");
}

#[test]
fn narrowing_does_not_leak_past_the_if_from_the_orelse_side() {
    let result = check_source(
        "x: int | None = 5\nif x is None:\n    print(0)\nelse:\n    y: int = x\n    print(y)\ny2: int = x\nprint(y2)\n",
    );
    let err = result.expect_err("narrowing inside the `orelse` must not leak past the `if`");
    assert_eq!(err.code, "T0025");
}

#[test]
fn narrowing_does_not_apply_to_a_non_optional_name() {
    // `is`/`is not None` against a plain (non-`Optional`) name is already
    // rejected at the `Compare` level (T0021), long before narrowing would
    // ever apply -- this just pins that the narrowing machinery introduces
    // no new false positive/negative for that existing rejection.
    let result = check_source("x: int = 5\nif x is not None:\n    print(x)\n");
    let err = result.expect_err("`is not None` against a plain `int` must still be rejected");
    assert_eq!(err.code, "T0021");
}

#[test]
fn compound_and_test_does_not_narrow() {
    // Scope cut (D-205): narrowing only recognizes a *top-level* `is`/`is
    // not None` test -- `pycc_hir::optional_none_test` requires the whole
    // `if` test to itself be an `HirExpr::Compare { op: Is | IsNot, .. }`
    // node, never one operand nested inside a compound `and`/`or` test.
    // `and`/`or` lower since #1211, so the compound test now reaches the
    // checker, and `x` inside the right operand is still `int | None`:
    // comparing it with `>` is refused (T0021), and so is binding it to an
    // `int` in the body (T0025).
    let err = check_source("x: int | None = 5\nif x is not None and x > 0:\n    print(1)\n")
        .expect_err("the right operand must not see `x` narrowed");
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot compare `int | None` and `int`"),
        "{}",
        err.message
    );
    let err = check_source(
        "x: int | None = 5\ny: int = 1\nif x is not None and y > 0:\n    z: int = x\n    print(z)\n",
    )
    .expect_err("the body must not see `x` narrowed");
    assert_eq!(err.code, "T0025", "{}", err.message);
}

#[test]
fn early_return_in_body_narrows_the_continuation() {
    // `if x is None: return` -- the code after the whole `if` is only
    // reachable via the implicit "`x` is not `None`" else path, so `x` is
    // known to be plain `int` there.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is None:\n        return 0\n    y: int = x\n    return y\n",
    );
    assert!(
        result.is_ok(),
        "an early-return `if x is None:` must narrow the continuation to plain `int`: {result:?}"
    );
}

#[test]
fn early_return_only_in_a_nested_inner_if_does_not_narrow() {
    // The unsound `contains_return`-based design's regression case: a
    // `return` occurs *inside* the outer `if`'s body, but only on one of
    // its own inner paths (`if flag: return 0` has no `else`), so the
    // outer body does not *definitely* terminate. `x` must stay
    // `Optional[int]` after the outer `if`.
    let result = check_source(
        "def f(x: int | None, flag: bool) -> int:\n    if x is None:\n        if flag:\n            return 0\n    y: int = x\n    return y\n",
    );
    let err = result.expect_err(
        "a return nested inside a non-exhaustive inner `if` must not narrow the continuation",
    );
    assert_eq!(err.code, "T0025");
}

#[test]
fn raise_in_body_does_not_narrow_the_continuation() {
    // Deliberate scope cut (D-205): `raise` is not a terminator for
    // `definitely_terminates`, unlike `return`.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is None:\n        raise ValueError(\"absent\")\n    y: int = x\n    return y\n",
    );
    let err = result.expect_err("`raise` in the body must not narrow the continuation");
    assert_eq!(err.code, "T0025");
}

#[test]
fn assignment_inside_a_narrowed_branch_kills_the_overlay() {
    // `x = None` inside the narrowed `is not None` body re-widens `x` at
    // runtime; the overlay must be cleared from that point forward so a
    // later read in the same body is checked against the real (still
    // `Optional[int]`) type, not the stale narrowed `int`.
    let result = check_source(
        "x: int | None = 5\nif x is not None:\n    x = None\n    y: int = x\n    print(y)\n",
    );
    let err = result.expect_err("an assignment inside the narrowed branch must kill the overlay");
    assert_eq!(err.code, "T0025");
}

#[test]
fn assignment_target_inside_a_narrowed_branch_checks_against_the_real_type() {
    // `x = None` itself (the assignment statement, not a later read) must
    // be checked against `x`'s *real* declared type (`Optional[int]`, which
    // accepts `None`), not the narrowed `int` overlay type (which would
    // reject `None` as incompatible). This is the "assignment TARGET
    // checking never consults the overlay" half of the design.
    let result = check_source("x: int | None = 5\nif x is not None:\n    x = None\n    print(0)\n");
    assert!(
        result.is_ok(),
        "assigning `None` back to a narrowed `Optional[int]` name must check against its real type: {result:?}"
    );
}

#[test]
fn narrowed_read_composes_with_ordinary_int_operators() {
    // An end-to-end narrowing-semantics smoke test: the narrowed value is
    // usable directly as a plain `int` operand, not merely assignable to an
    // `int`-annotated target.
    let result = check_source("x: int | None = 5\nif x is not None:\n    print(x + 1)\n");
    assert!(
        result.is_ok(),
        "a narrowed `Optional[int]` must be usable as a plain `int` operand: {result:?}"
    );
}

#[test]
fn narrowing_applies_at_module_scope_and_function_scope_alike() {
    let module_scope =
        check_source("x: int | None = 5\nif x is not None:\n    y: int = x\n    print(y)\n");
    let function_scope = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        return x\n    return 0\n",
    );
    assert!(module_scope.is_ok(), "module scope: {module_scope:?}");
    assert!(function_scope.is_ok(), "function scope: {function_scope:?}");
}

/// Direct unit test on `narrow::narrowing_target` itself (bypassing
/// `infer_expr`'s own `?`-propagated `T0021` rejection of an unbound name,
/// which is what actually prevents this shape from ever reaching
/// `narrowing_target` through a real source program -- every real call site
/// runs `infer_expr(env, test)?` immediately beforehand, and that call would
/// already have failed for a wholly unbound name before `narrowing_target`
/// is ever invoked). This proves the fail-closed `_ => None` arm's other
/// sub-case -- `env.lookup_any(name)` returning `None` entirely, as opposed
/// to `narrowing_does_not_apply_to_a_non_optional_name`'s `Some(non-Optional)`
/// sub-case -- still degrades safely rather than panicking, matching
/// `pycc_mir`'s own identical defense-in-depth guard
/// (`a_plain_bool_test_reads_normally_with_no_unwrap`).
#[test]
fn narrowing_target_is_none_for_a_wholly_unbound_name() {
    let env = Environment::new();
    let test = HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(HirExpr::Name("never_bound".to_string())),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert!(narrow::narrowing_target(&env, &test).is_none());
}

// D-068 review of #780 (blocker finding 1): a reassignment inside a nested
// branch killed the narrowing overlay on that branch-local clone, but
// `join_if_branches` reconciled only `.bindings`, never `.narrowed` -- so
// the kill was silently discarded the moment the nested `if` closed, and a
// read immediately afterward (still inside the *outer* narrowed body) saw
// the stale narrowed (non-`Optional`) type instead of being correctly
// rejected. `join_loop_body`/`join_match_branches` had the identical
// defect for a kill nested inside a `while`/`for` body or a `match` case.
// See `narrow::join_narrowed`'s doc comment for the fix.

#[test]
fn reassignment_inside_a_nested_if_kills_narrowing_at_module_scope() {
    let result = check_source(
        "x: int | None = 5\nflag: bool = True\nif x is not None:\n    if flag:\n        x = None\n    print(x + 1)\n",
    );
    let err = result.expect_err(
        "a reassignment inside a nested `if`'s body must kill the outer narrowing once the nested `if` closes",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn reassignment_inside_a_nested_if_kills_narrowing_at_function_scope() {
    let result = check_source(
        "def f(x: int | None, flag: bool) -> int:\n    if x is not None:\n        if flag:\n            x = None\n        return x + 1\n    return 0\n",
    );
    let err = result.expect_err(
        "a reassignment inside a nested `if`'s body must kill the outer narrowing once the nested `if` closes (function scope)",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn reassignment_inside_a_nested_while_kills_narrowing() {
    // Structural analog of the nested-`if` repro, via `join_loop_body`
    // instead of `join_if_branches`: the loop may execute the kill on some
    // iteration, so the narrowing must not survive the loop closing.
    let result = check_source(
        "x: int | None = 5\nflag: bool = True\nif x is not None:\n    while flag:\n        x = None\n    print(x + 1)\n",
    );
    let err =
        result.expect_err("a reassignment inside a nested `while` body must kill the narrowing");
    assert_eq!(err.code, "T0021");
}

#[test]
fn reassignment_inside_one_match_case_kills_narrowing() {
    // Structural analog via `join_match_branches`: only one case reassigns
    // `x`, so the narrowing must not survive the `match` closing.
    let result = check_source(
        "x: int | None = 5\nflag: int = 0\nif x is not None:\n    match flag:\n        case 0:\n            x = None\n        case _:\n            pass\n    print(x + 1)\n",
    );
    let err = result.expect_err("a reassignment inside one `match` case must kill the narrowing");
    assert_eq!(err.code, "T0021");
}

#[test]
fn narrowing_still_survives_a_join_when_both_arms_agree() {
    // Companion negative case: the fix must not become so conservative that
    // it drops narrowing nothing actually killed -- neither arm of the
    // nested `if` reassigns `x`, so it stays narrowed after the nested
    // `if` closes, exactly as before this fix.
    let result = check_source(
        "x: int | None = 5\nflag: bool = True\nif x is not None:\n    if flag:\n        print(0)\n    print(x + 1)\n",
    );
    assert!(
        result.is_ok(),
        "narrowing must still survive a nested `if` neither arm of which reassigns the name: {result:?}"
    );
}

// D-068 review of #780 (warning finding 2): the checker's fast-path
// helpers (`check_if_branches_in_place`/`check_while_body_in_place` and
// their function-scope counterparts) bypassed `narrow::check_stmt_sequence`
// in favor of a raw per-statement loop, so `apply_post_if_narrowing` never
// ran on the fast path -- a nested early-return guard's narrowing never
// propagated to a later statement in the same body whenever the outer
// construct itself introduced no bindings (the fast-path's own gate).

#[test]
fn nested_early_return_guard_narrows_inside_an_unrelated_outer_if_in_function_scope() {
    let result = check_source(
        "def f(cond: bool, x: int | None) -> int:\n    if cond:\n        if x is None:\n            return 0\n        return x\n    return 0\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of its enclosing body even on the fast (no-new-bindings) `if` path: {result:?}"
    );
}

#[test]
fn nested_early_return_guard_narrows_inside_an_unrelated_outer_while_in_function_scope() {
    // The `while` counterpart of the same fast-path bypass
    // (`check_while_body_in_place_in_function`).
    let result = check_source(
        "def f(flag: bool, x: int | None) -> int:\n    while flag:\n        if x is None:\n            return 0\n        return x\n    return 0\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of its enclosing body even on the fast `while` path: {result:?}"
    );
}

// Module scope has no legal `return` statement (`check_stmt`'s own
// `HirStmt::Return` arm rejects it outright before `apply_post_if_narrowing`
// ever runs), so the early-return continuation shape -- and therefore
// finding 2's specific defect -- cannot occur at module scope at all; there
// is deliberately no module-scope counterpart to the two tests above.

// D-068 re-review of #780 (third round): the single left-to-right
// source-order narrowing pass reconciled overlays only at control-flow
// *joins*, so it silently assumed a body's execution order always matches
// its source order. That assumption fails whenever a body can be
// re-entered such that a statement earlier in *execution* order runs
// after a kill that appears later in *source* order -- a `while` loop
// re-running its own body, or an `except` handler that only ever runs
// after some partial prefix of the `try` body already executed. Both are
// fixed the same way: `narrow::apply_kill_prescan` scans the re-enterable
// body for every name it kills anywhere within it, and drops those names'
// narrowing overlay entries for the *entire* body before the normal
// left-to-right pass ever starts, so no read inside the body can observe
// a narrowing that some other execution path through the same body would
// already have invalidated. See `docs/decisions/` for the entry
// superseding D-199's now-inaccurate "narrowing never survives a
// reassignment of the narrowed name" consequence.

#[test]
fn a_narrowed_read_inside_a_while_loop_body_the_same_body_later_kills_is_rejected() {
    // Blocker 1 (loop re-entry): `x` is narrowed on `while` entry and the
    // *first* iteration's `print(x + 1)` would be sound in isolation, but
    // the loop can run again -- and the second iteration's `print(x + 1)`
    // actually executes after the first iteration's own `x = None`, so it
    // must be rejected exactly like any other post-kill read.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        i: int = 0\n        while i < 2:\n            print(x + 1)\n            x = None\n            i = i + 1\n        return 0\n    return -1\n",
    );
    let err = result.expect_err(
        "a `while` body that reads a narrowed name before killing it must still be rejected, because a later iteration runs the read after the kill",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn an_except_handler_reached_after_a_try_body_kill_is_rejected() {
    // Blocker 2 (except-from-mid-try): the handler can only ever be
    // entered after the `try` body's own `x = None` already ran (the
    // `raise` that transfers control to the handler is the very next
    // statement), so the handler's read must be checked against the
    // post-kill state, not the narrowed pre-try state.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        try:\n            x = None\n            raise ValueError(\"boom\")\n        except ValueError:\n            return x + 1\n    return -1\n",
    );
    let err = result.expect_err(
        "an `except` handler reachable only after the `try` body's own kill must not see the pre-try narrowing",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_while_loop_body_that_reads_but_never_kills_the_narrowed_name_stays_narrowed() {
    // Completeness guard, matching `pycc_mir`'s own equivalent
    // (`a_while_body_that_reads_but_never_kills_the_narrowed_name_still_unwraps_the_read`
    // in `crates/pycc_mir/src/tests/narrow.rs`): the kill-prescan must
    // scope its pruning to names the body *actually* kills, not
    // unconditionally drop narrowing for every loop-containing body --
    // otherwise this fix would trade the soundness bug for a spurious
    // rejection of `nested_early_return_guard_narrows_inside_an_unrelated_outer_while_in_function_scope`'s
    // own defect class.
    let result = check_source(
        "def f(x: int | None, flag: bool) -> int:\n    if x is not None:\n        while flag:\n            print(x + 1)\n        return 0\n    return -1\n",
    );
    assert!(
        result.is_ok(),
        "a `while` body that reads a narrowed name but never kills it anywhere must stay narrowed: {result:?}"
    );
}

// D-068 re-review of #780 (third round, warning finding): `check_match`'s
// case-body loop and `check_try_stmt`'s four body loops (`body`, each
// handler's `body`, `orelse`, `finalbody`) used a raw per-statement loop
// instead of `narrow::check_stmt_sequence[_in_function]`, so a nested
// early-return guard's narrowing never propagated to a later statement in
// the same case/try/handler/else/finally body -- the identical fast-path
// bypass finding 2 had already fixed for `if`/`while`, just never routed
// through those two constructs in the first place.

#[test]
fn nested_early_return_guard_narrows_the_rest_of_the_same_match_case_body() {
    let result = check_source(
        "def f(flag: int, x: int | None) -> int:\n    match flag:\n        case 0:\n            if x is None:\n                return 0\n            return x\n        case _:\n            return 0\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of its enclosing `match` case body: {result:?}"
    );
}

#[test]
fn nested_early_return_guard_narrows_the_rest_of_the_try_body() {
    let result = check_source(
        "def f(x: int | None) -> int:\n    try:\n        if x is None:\n            return 0\n        return x\n    except ValueError:\n        return -1\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of the `try` body: {result:?}"
    );
}

#[test]
fn nested_early_return_guard_narrows_the_rest_of_an_except_handler_body() {
    let result = check_source(
        "def f(x: int | None) -> int:\n    try:\n        raise ValueError(\"boom\")\n    except ValueError:\n        if x is None:\n            return 0\n        return x\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of the `except` handler body: {result:?}"
    );
}

#[test]
fn nested_early_return_guard_narrows_the_rest_of_the_else_body() {
    let result = check_source(
        "def f(x: int | None) -> int:\n    try:\n        pass\n    except ValueError:\n        return -1\n    else:\n        if x is None:\n            return 0\n        return x\n",
    );
    assert!(
        result.is_ok(),
        "a nested early-return guard must narrow the rest of the `else` body: {result:?}"
    );
}

#[test]
fn a_plain_narrowed_read_inside_the_finally_body_still_type_checks() {
    // `finalbody` also moved from a raw per-statement loop to
    // `narrow::check_stmt_sequence_in_function`, but `finally` cannot
    // itself contain a `return` (L0001 rejects that outright) --
    // `apply_post_if_narrowing`'s own condition (`definitely_terminates`,
    // which only recognizes a trailing `return`) can therefore never fire
    // inside a `finally` body, so unlike the four sibling tests above this
    // is a plain sanity check that the routing change left ordinary
    // `finally`-body narrowing intact, not a regression test for a
    // behavior the fix newly enables there.
    let result = check_source(
        "def f(x: int | None) -> int:\n    try:\n        pass\n    except ValueError:\n        pass\n    finally:\n        if x is not None:\n            print(x + 1)\n    return 0\n",
    );
    assert!(
        result.is_ok(),
        "a narrowed read inside the `finally` body must still type-check after the sequencing fix: {result:?}"
    );
}

#[test]
fn a_narrowed_read_inside_a_while_loop_body_killed_only_by_a_walrus_is_rejected() {
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        i: int = 0\n        while i < 2:\n            print(x + 1)\n            (x := None)\n            i = i + 1\n        return 0\n    return -1\n",
    );
    let err = result.expect_err(
        "a `while` body that reads a narrowed name before a bare-walrus kill must still be rejected, because a later iteration runs the read after the kill",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn an_except_as_binding_that_reuses_a_narrowed_name_kills_the_narrowing() {
    // D-068 re-review of #780 (fourth round, blocker finding 1): `except
    // ValueError as x:` rebinds `x` to the caught exception instance, but
    // `handler_env.bind` alone never cleared `x`'s narrowing overlay entry
    // -- a read of `x` inside the handler body was wrongly type-checked as
    // the narrowed `int` (from the enclosing `if x is not None:`) instead
    // of the real `Instance(ValueError)` the runtime actually holds. Before
    // the fix this incorrectly type-checked; after it, `x + 1` is a binop
    // between an exception instance and an `int`, which must be rejected.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        try:\n            raise ValueError(\"boom\")\n        except ValueError as x:\n            return x + 1\n    return -1\n",
    );
    let err = result.expect_err(
        "an `except ... as x:` binding must kill `x`'s narrowing overlay entry, since `x` now holds the caught exception instance rather than the narrowed `Optional`'s inner value",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_match_capture_pattern_that_reuses_a_narrowed_name_inside_a_while_loop_is_rejected() {
    // D-068 re-review of #780 (fourth round, blocker finding 2): a `match`
    // case's capture pattern (`case x:`) binds a bare name exactly like an
    // `Assign` does (routed through `check_assignment` by `check_match`),
    // but `collect_killed_names`'s `Match` arm only recursed into each
    // case's body, never inspecting the pattern itself -- so
    // `apply_kill_prescan` under-reported this kill for a re-enterable
    // `while` body, reproducing D-206's own loop-reentry counterexample
    // through an unrecognized kill vector.
    let result = check_source(
        "def f(x: int | None, y: int | None) -> int:\n    if x is not None:\n        i: int = 0\n        while i < 2:\n            print(x + 1)\n            match y:\n                case x:\n                    pass\n            i = i + 1\n        return 0\n    return -1\n",
    );
    let err = result.expect_err(
        "a `while` body that reads a narrowed name before a later iteration's `match` capture-pattern kill must still be rejected",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_try_star_except_as_binding_that_reuses_a_narrowed_name_kills_the_narrowing() {
    // D-068 re-review of #780 (rebase onto #542's except* landing):
    // mirrors `an_except_as_binding_that_reuses_a_narrowed_name_kills_the_narrowing`
    // above for `except* ValueError as x:` -- `check_try_star_stmt` bound
    // the caught `ExceptionGroup` into `handler_env` but never cleared
    // `x`'s narrowing overlay entry, so a read of `x` inside the handler
    // body was wrongly type-checked as the narrowed `int` instead of the
    // real `Instance(ExceptionGroup)` the runtime actually holds.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        try:\n            raise ValueError(\"boom\")\n        except* ValueError as x:\n            print(x + 1)\n    return -1\n",
    );
    let err = result.expect_err(
        "an `except* ... as x:` binding must kill `x`'s narrowing overlay entry, since `x` now holds the caught `ExceptionGroup` rather than the narrowed `Optional`'s inner value",
    );
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_try_star_except_handler_reached_after_a_try_body_kill_is_rejected() {
    // D-068 re-review of #780 (rebase onto #542's except* landing):
    // mirrors `an_except_handler_reached_after_a_try_body_kill_is_rejected`
    // above for `except*` -- `check_try_star_stmt` never called
    // `apply_kill_prescan` on its handler bodies, so a handler reachable
    // only after the `try` body's own kill would have wrongly kept seeing
    // the pre-try narrowing.
    let result = check_source(
        "def f(x: int | None) -> int:\n    if x is not None:\n        try:\n            x = None\n            raise ValueError(\"boom\")\n        except* ValueError:\n            print(x + 1)\n    return -1\n",
    );
    let err = result.expect_err(
        "an `except*` handler reachable only after the `try` body's own kill must not see the pre-try narrowing",
    );
    assert_eq!(err.code, "T0021");
}
