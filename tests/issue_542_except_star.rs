//! `except*`/`ExceptionGroup`/`BaseExceptionGroup` (Part 3 of #382, #542,
//! PEP 654, D-202) end-to-end coverage, following the same per-issue test
//! file convention as its sibling parts of the exception-chain epic
//! (`tests/issue_739_oserror_hierarchy.rs` for Part 2 of #543,
//! `tests/issue_740_multi_type_except.rs` for Part 3 of #543).

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

fn build_and_run(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, Vec<u8>, String) {
    let src = write_fixture(dir, src_name, source);
    let out = dir.join(src_name.replace(".py", ""));
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        );
    }
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        run.stdout.clone(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

fn check_only(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, String) {
    let src = write_fixture(dir, src_name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn except_star_single_clause_catches_a_plain_exception() {
    let dir = ScratchDir::new("542_single").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "single.py",
        "try:\n    raise ValueError(\"bad\")\nexcept* ValueError:\n    print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn except_star_dispatches_a_plain_exception_to_the_matching_clause() {
    let dir = ScratchDir::new("542_dispatch").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "dispatch.py",
        "try:\n    raise ValueError(\"bad\")\nexcept* TypeError:\n    print(\"type\")\nexcept* ValueError:\n    print(\"value\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"value\n");
}

#[test]
fn except_star_group_dispatches_each_member_to_its_own_clause() {
    let dir = ScratchDir::new("542_group").expect("failed to create scratch dir");
    // D-202: an `ExceptionGroup` member must be an *existing* exception
    // value, not a fresh `SomeError("msg")` construction (see
    // `check_exception_group_member_operand`'s own doc comment) -- so `e1`/
    // `e2` are bound by nested `except ... as` handlers before the group
    // that carries them both is constructed and raised.
    let (ok, out, err) = build_and_run(
        &dir,
        "group.py",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as e1:\n    try:\n        raise TypeError(\"t\")\n    except TypeError as e2:\n        try:\n            raise ExceptionGroup(\"multi\", [e1, e2])\n        except* ValueError:\n            print(\"caught value\")\n        except* TypeError:\n            print(\"caught type\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught value\ncaught type\n");
}

/// Part 3 of #382 (#542, PEP 654, D-202): a partial match. `except*
/// ValueError:` claims only `e1` from the raised two-member group; `e2`
/// (a `TypeError`) is left in the still-unmatched remainder threaded through
/// `emit_try_star`'s `pycc_rt_exception_group_partition` chain, and since no
/// further clause exists to claim it, the remainder is re-raised at
/// `reraise_remainder_bb` after the matching clause's body has already run.
/// This is the only test in this file that takes the non-null branch of
/// `current_group_slot` at that block -- every other `except*` test either
/// matches every member (so the remainder is empty and that branch's load is
/// never reached) or raises no group at all.
#[test]
fn except_star_partial_match_reraises_the_unmatched_remainder() {
    let dir = ScratchDir::new("542_partial").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "partial.py",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as e1:\n    try:\n        raise TypeError(\"t\")\n    except TypeError as e2:\n        try:\n            raise ExceptionGroup(\"multi\", [e1, e2])\n        except* ValueError:\n            print(\"caught value\")\n",
    );
    assert!(
        !ok,
        "expected non-zero exit: unmatched TypeError remainder should re-raise"
    );
    assert_eq!(
        out, b"caught value\n",
        "the matched clause should still run before the remainder re-raises"
    );
    assert!(
        err.contains("ExceptionGroup"),
        "stderr should mention the re-raised remainder's ExceptionGroup: {err}"
    );
}

/// Regression test for a P1 null-pointer dereference found in an external
/// review of PR #794: `except* Exception:` uses the universal
/// `EXCEPTION_TYPE_EXCEPTION` tag, so a first clause naming `Exception` can
/// claim every member of the raised group, leaving `current_group_slot`
/// null (`pycc_rt_exception_group_partition`'s `rest_out` is null for an
/// empty remainder). Without a null guard, the *next* clause's dispatch
/// block would reload that null pointer and pass it straight back into
/// `pycc_rt_exception_group_partition`, which unconditionally dereferences
/// its `group` argument -- undefined behavior. `emit_try_star` now skips a
/// clause's dispatch entirely once the threaded group pointer is null,
/// falling through to the next clause (or `reraise_remainder_bb`) instead.
#[test]
fn except_star_broad_first_clause_consuming_the_whole_group_does_not_crash_the_next_clause() {
    let dir = ScratchDir::new("542_broad_first").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "broad_first.py",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as e1:\n    try:\n        raise ExceptionGroup(\"multi\", [e1])\n    except* Exception:\n        print(\"caught broad\")\n    except* ValueError:\n        print(\"caught value\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(
        out, b"caught broad\n",
        "the broad first clause claims the only member; the second clause's dispatch \
         must be skipped (not crash) rather than reraise or run"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `BaseExceptionGroup` construction
/// and dispatch, exercised nowhere else in this file -- every other
/// `except*`/group test uses `ExceptionGroup`. D-202 records that
/// `BaseExceptionGroup`'s hierarchy parent is treated as `Exception` rather
/// than a separate `BaseException`-only branch; this confirms it still
/// raises, dispatches, and is caught end to end under that simplification.
#[test]
fn base_exception_group_construction_and_dispatch() {
    let dir = ScratchDir::new("542_baseeg").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "baseeg.py",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as e1:\n    try:\n        raise BaseExceptionGroup(\"multi\", [e1])\n    except* ValueError:\n        print(\"caught\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}

#[test]
fn except_star_as_binding_is_accessible() {
    let dir = ScratchDir::new("542_asbind").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "asbind.py",
        "try:\n    raise ValueError(\"bad\")\nexcept* ValueError as eg:\n    saved = eg\n    print(\"got it\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"got it\n");
}

/// `pycc_types::function_local_names` (used only for a function body, not a
/// top-level module body) has its own `collect_local_names` recursion with a
/// `TryStar` arm shared with `Try` -- `except_star_as_binding_is_accessible`
/// above exercises the same source shape, but at module level, which never
/// reaches `function_local_names` at all. Wrap the same shape in a function
/// to exercise that separate path.
#[test]
fn except_star_as_binding_inside_a_function_is_a_local() {
    let dir = ScratchDir::new("542_asbind_fn").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "asbind_fn.py",
        "def f() -> None:\n    try:\n        raise ValueError(\"bad\")\n    except* ValueError as eg:\n        saved = eg\n        print(\"got it in function\")\n\nf()\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"got it in function\n");
}

/// `reject_generic_calls_in_stmt` (D-133/D-134's PEP 695 self/mutual
/// generic-recursion check) walks every statement in a *generic* function's
/// own body looking for a disallowed nested generic call, and shares its
/// `Try`/`TryStar` arm with `pycc_types::lib`'s other structural walks --
/// but it only ever runs for a generic function specifically, so no
/// `except*`-inside-an-ordinary-function fixture reaches it. A generic
/// function whose `try`/`except*` bodies contain no generic call at all
/// still walks all four blocks looking for one.
#[test]
fn except_star_inside_a_generic_function_body_type_checks() {
    let dir = ScratchDir::new("542_generic").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "generic.py",
        "def identity[T](x: T) -> T:\n    try:\n        return x\n    except* ValueError:\n        pass\n    else:\n        return x\n    finally:\n        pass\n    return x\n",
    );
    assert!(ok, "check failed: {combined}");
}

#[test]
fn except_star_unmatched_member_propagates_uncaught() {
    let dir = ScratchDir::new("542_unmatched").expect("failed to create scratch dir");
    let (ok, _out, err) = build_and_run(
        &dir,
        "unmatched.py",
        "try:\n    raise ValueError(\"v\")\nexcept ValueError as e1:\n    try:\n        raise TypeError(\"t\")\n    except TypeError as e2:\n        raise ExceptionGroup(\"multi\", [e1, e2])\n",
    );
    assert!(
        !ok,
        "expected non-zero exit for an unmatched group remainder"
    );
    assert!(
        err.contains("ExceptionGroup"),
        "stderr should mention ExceptionGroup: {err}"
    );
}

#[test]
fn except_star_finally_always_runs() {
    let dir = ScratchDir::new("542_finally").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "finally.py",
        "try:\n    raise ValueError(\"bad\")\nexcept* ValueError:\n    print(\"caught\")\nfinally:\n    print(\"cleanup\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\ncleanup\n");
}

/// Unlike `except_star_finally_always_runs` above (a top-level, `None`-typed
/// `try`/`except*`/`finally`), a value-returning function whose `try`/
/// `except*` construct also carries a `finally` needs `emit_try_star` to
/// allocate a `ret_slot` to stash the return value across the `finally`
/// block's own codegen -- without a fixture like this one, that allocation
/// path is only ever exercised for plain `Try`, never `TryStar`.
#[test]
fn except_star_with_finally_in_a_value_returning_function_returns_correctly() {
    let dir = ScratchDir::new("542_finally_ret").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "finally_ret.py",
        "def f() -> int:\n    try:\n        return 2\n    except* ValueError:\n        print(\"handler\")\n    finally:\n        print(\"cleanup\")\n    return 9\n\nprint(f())\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\n2\n");
}

#[test]
fn except_star_else_runs_when_no_exception() {
    let dir = ScratchDir::new("542_else").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "else.py",
        "try:\n    print(\"body\")\nexcept* ValueError:\n    print(\"handler\")\nelse:\n    print(\"else\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"body\nelse\n");
}

/// `emit_try_star`'s `else` block emits `build_unconditional_branch(finally_bb)`
/// only when the `else` body itself falls through (`else_falls_through`,
/// computed from whether the `else` block's own generated code already ends
/// with an LLVM terminator). `except_star_else_runs_when_no_exception`
/// above ends its `else` body in a plain `print(...)` statement, so
/// `else_falls_through` is always `true` there -- the `false` case (an
/// `else` body whose last statement is itself a `return`, already
/// terminating the block before this check runs) has never been exercised.
#[test]
fn a_try_star_else_that_itself_returns_never_falls_through() {
    let dir = ScratchDir::new("542_else_returns").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "else_returns.py",
        "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        pass\n    else:\n        return 2\n    finally:\n        pass\n    return 1\n\nprint(f())\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"2\n");
}

#[test]
fn except_star_bare_clause_is_rejected_at_parse_time() {
    let dir = ScratchDir::new("542_bare").expect("failed to create scratch dir");
    let (ok, _combined) = check_only(&dir, "bare.py", "try:\n    pass\nexcept*:\n    pass\n");
    assert!(
        !ok,
        "a typeless except* should be rejected as a syntax error"
    );
}

#[test]
fn exception_group_construction_rejects_a_non_literal_member_list() {
    let dir = ScratchDir::new("542_nonlit").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "nonlit.py",
        "members = [1, 2]\ntry:\n    raise ExceptionGroup(\"multi\", members)\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "a non-literal ExceptionGroup member list should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_member_operand`'s
/// fresh-constructor-call rejection branch -- a group member must be an
/// *existing* exception value, never a `SomeError(...)` construction inline
/// in the member list, even though such a call is a perfectly valid
/// top-level `raise` operand.
#[test]
fn exception_group_construction_rejects_a_fresh_constructor_call_member() {
    let dir = ScratchDir::new("542_freshctor").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "freshctor.py",
        "try:\n    raise ExceptionGroup(\"multi\", [ValueError(\"v\")])\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "a fresh constructor-call ExceptionGroup member should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
    assert!(
        combined.contains("fresh"),
        "should explain the member must not be a fresh construction: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_operand`'s
/// argument-count branch -- `ExceptionGroup` requires exactly a message and a
/// member list, so a single-argument call is rejected structurally before
/// either argument's own type is even inspected.
#[test]
fn exception_group_construction_rejects_the_wrong_argument_count() {
    let dir = ScratchDir::new("542_argc").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "argc.py",
        "try:\n    raise ExceptionGroup(\"multi\")\nexcept* ValueError:\n    pass\n",
    );
    assert!(!ok, "a one-argument ExceptionGroup call should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_operand`'s
/// message-type branch -- the first argument must be a `str`, exactly like a
/// plain `raise SomeError(...)`'s own message argument.
#[test]
fn exception_group_construction_rejects_a_non_str_message() {
    let dir = ScratchDir::new("542_msgty").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "msgty.py",
        "try:\n    raise ExceptionGroup(1, [1, 2])\nexcept* ValueError:\n    pass\n",
    );
    assert!(!ok, "a non-str ExceptionGroup message should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_operand`'s
/// own `infer_expr_in(env, local_names, &args[0])?` call -- distinct from the
/// `message_ty != Ty::Str` check right below it. `exception_group_construction_rejects_a_non_str_message`
/// above supplies a message argument (`1`) that infers successfully as `int`
/// and is then rejected by the type comparison; it never makes `infer_expr_in`
/// itself return an `Err`. Naming an undefined variable as the message
/// argument makes inference itself fail (T0021, "not defined"), reaching the
/// `?` operator's own error-propagation branch instead.
#[test]
fn exception_group_construction_rejects_an_unresolved_message_name() {
    let dir = ScratchDir::new("542_msgname").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "msgname.py",
        "try:\n    raise ExceptionGroup(undefined_message, [ValueError(\"v\")])\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "an ExceptionGroup message naming an undefined variable should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Same rationale as `exception_group_construction_rejects_an_unresolved_message_name`
/// above, but for `check_exception_group_member_operand`'s own
/// `infer_expr_in(env, local_names, expr)?` call: every other member-rejection
/// fixture in this file (`exception_group_construction_rejects_a_non_exception_member`,
/// etc.) supplies a member expression that infers successfully and is then
/// rejected by the `matches!` check right after it, never by `infer_expr_in`
/// itself failing. An undefined variable as a member name makes inference
/// itself fail instead.
#[test]
fn exception_group_construction_rejects_an_unresolved_member_name() {
    let dir = ScratchDir::new("542_membername").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "membername.py",
        "try:\n    raise ExceptionGroup(\"multi\", [undefined_member])\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "an ExceptionGroup member naming an undefined variable should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_operand`'s
/// empty-member-list branch -- PEP 654 requires at least one member exception,
/// so a literal empty list is rejected at type-check time rather than
/// reaching codegen with nothing to partition.
#[test]
fn exception_group_construction_rejects_an_empty_member_list() {
    let dir = ScratchDir::new("542_empty").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "empty.py",
        "try:\n    raise ExceptionGroup(\"multi\", [])\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "an empty ExceptionGroup member list should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Part 3 of #382 (#542, PEP 654, D-202): `check_exception_group_member_operand`'s
/// final fallback branch -- a group member that isn't an exception instance
/// at all (e.g. a plain `int`) is rejected too.
#[test]
fn exception_group_construction_rejects_a_non_exception_member() {
    let dir = ScratchDir::new("542_nonexc").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "nonexc.py",
        "try:\n    raise ExceptionGroup(\"multi\", [1])\nexcept* ValueError:\n    pass\n",
    );
    assert!(
        !ok,
        "a non-exception ExceptionGroup member should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// `check_try_star_stmt`'s per-clause loop rejects an `except*` type that is
/// neither a builtin exception nor a user-defined exception class -- the
/// same T0021 check plain `except` already exercises for a made-up name, but
/// not yet exercised for `except*`.
#[test]
fn except_star_rejects_an_unrecognized_exception_type() {
    let dir = ScratchDir::new("542_unrecognized").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "unrecognized.py",
        "try:\n    raise ValueError(\"bad\")\nexcept* NotARealException:\n    pass\n",
    );
    assert!(!ok, "an unrecognized except* type should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Same rationale as `exception_group_construction_rejects_a_fresh_constructor_call_member`
/// above, but for a *user-defined* exception subclass's constructor call
/// rather than a builtin's: `check_exception_group_member_operand` rejects
/// both, but through two different disjuncts of the same `||` -- a builtin
/// callee short-circuits the check before the user-defined-class lookup
/// ever runs, so a builtin-only fixture never exercises the second disjunct.
#[test]
fn exception_group_construction_rejects_a_fresh_user_defined_constructor_call_member() {
    let dir = ScratchDir::new("542_freshuserctor").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "freshuserctor.py",
        "class MyError(ValueError):\n    pass\n\ntry:\n    raise ExceptionGroup(\"multi\", [MyError(\"v\")])\nexcept* MyError:\n    pass\n",
    );
    assert!(
        !ok,
        "a fresh user-defined-class constructor-call ExceptionGroup member should be rejected"
    );
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// Mirrors `a_value_returning_function_may_terminate_only_by_raising` and its
/// siblings in `tests/issue_382_exceptions.rs`, but for `TryStar` instead of
/// `Try`: a non-`None`-returning function whose entire body is a `try`/
/// `except*` where every path (the raising try body, and the re-raising
/// handler) terminates. `pycc_codegen::exception::block_always_terminates`
/// has a dedicated `MirStmt::TryStar` arm (shared textually with `Try`'s) --
/// without a fixture like this one, that arm's pattern binding is never
/// actually matched at runtime, since every other `except*` fixture's outer
/// function returns `None` and falls through normally instead of relying on
/// this fallthrough-proof machinery.
#[test]
fn a_value_returning_function_whose_entire_body_is_a_terminating_try_star_never_falls_through() {
    let dir = ScratchDir::new("542_terminates").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "terminates.py",
        "def fail() -> int:\n    try:\n        raise ValueError(\"from try star\")\n    except* ValueError:\n        raise ValueError(\"rethrown from handler\")\n\ntry:\n    x = fail()\nexcept* ValueError:\n    print(\"caught value callee via star\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught value callee via star\n");
}

/// `pycc_types::constraints::collect_block_constraints`'s `HirStmt::TryStar`
/// arm recurses into the try body, each handler body, the `else` body, and
/// the `finally` body in turn, propagating any error each recursive call
/// returns via `?`. Every other `except*` fixture in this file type-checks
/// successfully, so none of those four `?` sites has ever actually taken its
/// error branch -- each one is only reachable by making the *solver's own*
/// return-type unification fail (not the ordinary type checker, which is a
/// separate, later pass) inside exactly one of the four blocks. A
/// non-generic, non-`None`-returning function's declared return type is a
/// concrete solver term, so returning a mismatched literal type from within
/// the `try` body reliably reaches the body block's own recursive call
/// before any other block is even visited, giving an isolated repro for
/// this first `?` site.
#[test]
fn a_type_conflict_inside_a_try_star_body_is_rejected_by_the_solver() {
    let dir = ScratchDir::new("542_solver_body").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "solver_body.py",
        "def f() -> int:\n    try:\n        return \"wrong\"\n    except* ValueError:\n        pass\n    else:\n        return 1\n    finally:\n        pass\n    return 1\nf()\n",
    );
    assert!(
        !ok,
        "a body/declared-return-type conflict should be rejected"
    );
    assert!(
        combined.contains("T0022"),
        "should mention T0022: {combined}"
    );
}

/// Same rationale as `a_type_conflict_inside_a_try_star_body_is_rejected_by_the_solver`
/// above, but isolating the *handler* block's own recursive
/// `collect_block_constraints` call: the try body itself returns a
/// solver-compatible `int`, so the conflict is only discovered once the
/// `except*` handler body is visited.
///
/// #795 (PEP 654) forced the mechanism to change: a mismatched `return`
/// cannot be placed directly in an `except*` clause body any more (CPython
/// itself rejects it, and pycc now reports `L0001: 'return' in an 'except*'
/// block` before the solver ever runs, exactly as the `finally` sibling
/// below already had to work around for L0001's PEP 765 check). This drives
/// the conflict through the same *private helper* mechanism that sibling
/// uses (D-045): `_h`'s parameter type is inferred from its call sites, so
/// calling it once at module scope with `int` and once from inside the
/// `except*` handler with `str` makes the handler block's own recursive
/// call the one that discovers the conflict.
#[test]
fn a_type_conflict_inside_a_try_star_handler_is_rejected_by_the_solver() {
    let dir = ScratchDir::new("542_solver_handler").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "solver_handler.py",
        "def _h(x):\n    return x\n\n_h(1)\n\ndef f() -> int:\n    try:\n        return 1\n    except* ValueError:\n        _h(\"s\")\n    else:\n        return 1\n    finally:\n        pass\n    return 1\nf()\n",
    );
    assert!(
        !ok,
        "a handler private-helper argument-type conflict should be rejected"
    );
    assert!(
        combined.contains("T0021") || combined.contains("T0022"),
        "should mention a solver conflict code: {combined}"
    );
}

/// Same rationale again, isolating the `else` block's own recursive call:
/// both the try body and the handler return a solver-compatible `int`, so
/// the conflict only surfaces once the `else` body's mismatched return is
/// visited.
#[test]
fn a_type_conflict_inside_a_try_star_else_is_rejected_by_the_solver() {
    let dir = ScratchDir::new("542_solver_else").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "solver_else.py",
        "def f() -> int:\n    try:\n        return 1\n    except* ValueError:\n        pass\n    else:\n        return \"wrong\"\n    finally:\n        pass\n    return 1\nf()\n",
    );
    assert!(
        !ok,
        "an else/declared-return-type conflict should be rejected"
    );
    assert!(
        combined.contains("T0022"),
        "should mention T0022: {combined}"
    );
}

/// Isolates the `finally` block's own recursive `collect_block_constraints`
/// call -- the fourth and last `?` site in the `TryStar` arm. Unlike the
/// three siblings above, a mismatched `return` cannot be placed directly in
/// `finally` (rejected earlier, and unconditionally, by L0001's "'return' in
/// a 'finally' block" check, which would mask this arm's own error path
/// entirely). Instead this drives the conflict through a *private helper*'s
/// solver-inferred parameter type (D-045): `_h`'s parameter type is inferred
/// from its call sites rather than declared, so calling it once at module
/// scope with `int` and once from inside `f`'s `finally` block with `str`
/// makes the finally block's own recursive call the one that discovers the
/// conflict -- the try/handler/else bodies above it all type-check cleanly.
#[test]
fn a_type_conflict_inside_a_try_star_finally_is_rejected_by_the_solver() {
    let dir = ScratchDir::new("542_solver_finally").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "solver_finally.py",
        "def _h(x):\n    return x\n\n_h(1)\n\ndef f() -> int:\n    try:\n        return 1\n    except* ValueError:\n        pass\n    else:\n        return 1\n    finally:\n        _h(\"s\")\n    return 1\nf()\n",
    );
    assert!(
        !ok,
        "a finally-block private-helper argument-type conflict should be rejected"
    );
    assert!(
        combined.contains("T0021") || combined.contains("T0022"),
        "should mention a solver conflict code: {combined}"
    );
}

/// `check_try_star_stmt`'s per-clause loop calls `reject_own_constructor`
/// exactly like `check_try_stmt`'s does -- rejecting an `except*` handler
/// type that declares its own `__init__` (Part 3 of #541), not just a
/// `raise` of such a type. Every other `except*`/handler-type fixture in
/// this file names a type with no custom constructor, so this specific
/// call's error branch has never been reached for `TryStar`.
#[test]
fn except_star_rejects_a_handler_type_with_its_own_constructor() {
    let dir = ScratchDir::new("542_own_ctor").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "own_ctor.py",
        "class AppError(Exception):\n    def __init__(self, code: int) -> None:\n        self.code = code\n\n\ntry:\n    raise ValueError(\"v\")\nexcept* AppError:\n    pass\n",
    );
    assert!(
        !ok,
        "an except* handler type with its own constructor should be rejected"
    );
    assert!(
        combined.contains("C0001"),
        "should mention C0001: {combined}"
    );
}

/// `check_try_star_stmt` type-checks the handler body, `else` body, and
/// `finally` body through the same `check_stmt_shared` calls `check_try_stmt`
/// uses. Every other `except*` fixture's handler/else/finally bodies are
/// well-typed, so an ordinary type error placed in each of those three
/// positions has never propagated through `check_try_star_stmt`'s own call
/// sites specifically (as opposed to the analogous, already-covered `Try`
/// arm).
#[test]
fn a_type_error_inside_a_try_star_handler_body_is_rejected() {
    let dir = ScratchDir::new("542_handler_err").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "handler_err.py",
        "try:\n    raise ValueError(\"v\")\nexcept* ValueError:\n    y = 1 + \"s\"\n",
    );
    assert!(!ok, "a handler-body type error should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

#[test]
fn a_type_error_inside_a_try_star_else_body_is_rejected() {
    let dir = ScratchDir::new("542_else_err").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "else_err.py",
        "try:\n    print(\"body\")\nexcept* ValueError:\n    pass\nelse:\n    y = 1 + \"s\"\n",
    );
    assert!(!ok, "an else-body type error should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

#[test]
fn a_type_error_inside_a_try_star_finally_body_is_rejected() {
    let dir = ScratchDir::new("542_finally_err").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "finally_err.py",
        "try:\n    raise ValueError(\"v\")\nexcept* ValueError:\n    pass\nfinally:\n    y = 1 + \"s\"\n",
    );
    assert!(!ok, "a finally-body type error should be rejected");
    assert!(
        combined.contains("T0021"),
        "should mention T0021: {combined}"
    );
}

/// `check_try_star_stmt`'s handler-body loop type-checks each statement
/// through `check_stmt_shared`, which itself calls `check_assignment` for an
/// ordinary `Assign`. `check_assignment` already rejects an incompatible
/// reassignment of a pre-existing name *before* `check_try_star_stmt` ever
/// reaches its own `join_if_branches` call -- so despite the superficial
/// resemblance, this fixture (each `except*` clause reassigning a
/// pre-existing name to a mutually incompatible type) exercises the
/// handler-body statement loop's own `?` propagation (already covered by
/// `a_type_error_inside_a_try_star_handler_body_is_rejected`'s arithmetic
/// error), not the join. See
/// `a_handler_as_binding_conflicting_with_a_pre_existing_name_is_rejected_at_the_join`
/// below for a fixture that reaches `join_if_branches`'s own error branch.
#[test]
fn conflicting_types_across_two_try_star_handlers_are_rejected() {
    let dir = ScratchDir::new("542_join_conflict").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "join_conflict.py",
        "y = 1\ntry:\n    raise ValueError(\"v\")\nexcept* ValueError:\n    y = 2\nexcept* TypeError:\n    y = \"s\"\n",
    );
    assert!(
        !ok,
        "conflicting types joined across two except* handlers should be rejected"
    );
    assert!(
        combined.contains("T0023"),
        "should mention T0023: {combined}"
    );
}

/// `check_try_star_stmt`'s handler-join loop (`for handler_env in
/// &handler_envs { ... join_if_branches(&mut joined, &previous,
/// handler_env)?; }`) propagates `join_if_branches`'s own `Err` via `?`.
/// Reaching that specific error branch (as opposed to
/// `check_assignment`'s own, exercised by
/// `conflicting_types_across_two_try_star_handlers_are_rejected` above)
/// requires the *join itself* to be the first place two `Definitely`-typed,
/// mutually incompatible types for the same name meet -- which an ordinary
/// `Assign` inside a handler body can never do, since `check_assignment`
/// always intercepts an incompatible reassignment before the handler body
/// finishes. `except* ... as z:` is different: its binding
/// (`handler_env.bind(name, Ty::Instance("ExceptionGroup"))`) is a raw,
/// unconditional bind with no compatibility check against `z`'s
/// pre-existing type, so it slips a `Definitely(ExceptionGroup)` binding
/// straight into the first handler's own environment. When `z` was already
/// `Definitely(int)` before the `try` (from the module-level `z = 1`), the
/// join between that pre-`try` state and the first handler's freshly bound
/// `ExceptionGroup` type is where the mismatch is first detected.
#[test]
fn a_handler_as_binding_conflicting_with_a_pre_existing_name_is_rejected_at_the_join() {
    let dir = ScratchDir::new("542_join_as_conflict").expect("failed to create scratch dir");
    let (ok, combined) = check_only(
        &dir,
        "join_as_conflict.py",
        "z = 1\ntry:\n    raise ValueError(\"v\")\nexcept* ValueError as z:\n    pass\nexcept* TypeError:\n    pass\n",
    );
    assert!(
        !ok,
        "an except*-as binding conflicting with a pre-existing name's type should be rejected at the join"
    );
    assert!(
        combined.contains("T0023"),
        "should mention T0023: {combined}"
    );
}

/// `except_star_with_finally_in_a_value_returning_function_returns_correctly`
/// above exercises a `try`/`except*`/`finally` with a return value but no
/// *enclosing* finally, so the returned value is emitted directly (`ret_val`
/// with no outer `finally_stack` entry). Nesting that same construct inside
/// an outer `try`/`finally` instead reaches `emit_try_star`'s
/// `finally_stack.last_mut()` propagation branch: the inner `finally`'s
/// return is redirected into the outer's `ret_slot`/`is_returning` flag and
/// branched to the outer's `finally_bb`, rather than emitting `ret`
/// directly.
#[test]
fn a_try_star_finally_returning_a_value_propagates_through_an_enclosing_finally() {
    let dir = ScratchDir::new("542_nested_ret").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested_ret.py",
        "def f() -> int:\n    try:\n        try:\n            return 2\n        except* ValueError:\n            print(\"handler\")\n        finally:\n            print(\"inner\")\n    finally:\n        print(\"outer\")\n    return 9\n\nprint(f())\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"inner\nouter\n2\n");
}

/// The same enclosing-finally propagation as above, but for a bare `return`
/// (no value) inside a `None`-returning function's `try_star` `finally`:
/// `ret_slot` is `None`, so this reaches the sibling `finally_stack`
/// propagation branch that only forwards the `is_returning` flag, never the
/// `ret_val` store.
#[test]
fn a_try_star_finally_bare_return_propagates_through_an_enclosing_finally() {
    let dir = ScratchDir::new("542_nested_void").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested_void.py",
        "def g() -> None:\n    try:\n        try:\n            return\n        except* ValueError:\n            print(\"handler\")\n        finally:\n            print(\"inner\")\n    finally:\n        print(\"outer\")\n\ng()\nprint(\"done\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"inner\nouter\ndone\n");
}

/// A bare `return` inside a `None`-returning function's `try_star`
/// `finally`, with no enclosing finally at all (`finally_stack` empty),
/// reaches the plain `ret void` emission branch -- distinct from both
/// `a_try_star_finally_bare_return_propagates_through_an_enclosing_finally`
/// (which has an outer finally) and
/// `except_star_with_finally_in_a_value_returning_function_returns_correctly`
/// (which has a return value).
#[test]
fn a_try_star_finally_bare_return_with_no_enclosing_finally_emits_ret_void() {
    let dir = ScratchDir::new("542_bare_void").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "bare_void.py",
        "def h() -> None:\n    try:\n        return\n    except* ValueError:\n        print(\"handler\")\n    finally:\n        print(\"cleanup\")\n\nh()\nprint(\"done\")\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"cleanup\ndone\n");
}

/// All three fixtures above intercept a `return` from the *try* body
/// through an implicitly-falling-through `finally` block (one whose
/// own body never itself ends in a terminator), so `emit_try_star`'s
/// `finally_falls_through` flag -- computed from whether the `finally`
/// block's own generated code already ends with an LLVM terminator -- is
/// always `true` in each of them. A `finally` clause whose own last
/// statement is a `raise` gives the `finally` block its own terminator
/// before `emit_try_star` reaches this check, making `finally_falls_through`
/// `false` and skipping the whole return/pending-exception-restoration
/// block entirely -- the only way to exercise that flag's other value.
#[test]
fn a_try_star_finally_that_itself_raises_never_falls_through() {
    let dir = ScratchDir::new("542_finally_raises").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "finally_raises.py",
        "def f() -> int:\n    try:\n        raise ValueError(\"bad\")\n    except* ValueError:\n        pass\n    finally:\n        raise RuntimeError(\"boom\")\n    return 3\n\nprint(f())\n",
    );
    assert!(
        !ok,
        "the finally's own raise should propagate and abort the program"
    );
    assert!(out.is_empty(), "no output should be printed: {out:?}");
    assert!(
        err.contains("RuntimeError") && err.contains("boom"),
        "stderr should report the finally's own RuntimeError: {err}"
    );
}

/// `check_try_star_stmt`'s per-type validation loop (mirroring
/// `check_try_stmt`'s own PEP 758 loop) returns early via `?` on the first
/// invalid name, so every other `except*` fixture that names a *rejected*
/// type never reaches the loop's natural end-of-iteration fallthrough.
/// Naming a user-defined class that both is derived from a builtin
/// exception (so `user_exception_class` resolves it) and inherits only
/// `Exception`'s own constructor (so `reject_own_constructor` returns `Ok`)
/// -- with no `as` binding, matching `tests/issue_702_user_exceptions.rs`'s
/// analogous plain-`except` fixture -- lets the loop body finish normally
/// instead of diverging, exercising the loop's closing brace.
#[test]
fn except_star_catches_a_user_defined_exception_class_without_a_binding() {
    let dir = ScratchDir::new("542_user_class").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "user_exc_star.py",
        "class AppError(Exception):\n    pass\n\n\ndef f() -> None:\n    try:\n        raise AppError(\"bad\")\n    except* AppError:\n        print(\"caught\")\n\nf()\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"caught\n");
}
