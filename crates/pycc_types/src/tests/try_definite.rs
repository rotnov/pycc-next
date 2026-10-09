//! #1289: definite assignment across a `try` statement.
//!
//! A name bound by the body and by every handler that can complete the
//! statement is definitely bound after it. The conformance fixture
//! `tests/fixtures/try_except_definite_assign.py` pins the programs that now
//! run; these tests pin the arms that fixture cannot reach, or reaches only
//! indirectly: the zero-fall-through fallback, which paths count, the `as`
//! name's unbinding, the `finally` entry state, the type walk's direction,
//! and the constraint solver's mirror of each (`promote_try_fallthrough`).

use super::*;

fn code_of(src: &str) -> &'static str {
    parse_check(src).unwrap_err().code
}

// -- The check phase: which paths fall through --

/// No path falls through, so the conservative join is the post-`try` state:
/// a name bound only inside the statement stays possibly unbound, exactly as
/// before #1289, while a name `finally` binds is still definite after it.
#[test]
fn with_no_fall_through_path_the_conservative_state_is_kept() {
    let base = "\
try:
    x = 1
    raise ValueError(\"v\")
except ValueError:
    x = 2
    raise
finally:
    z = 10
";
    assert_eq!(code_of(&format!("{base}print(x)\n")), "T0041");
    assert!(parse_check(&format!("{base}print(z)\n")).is_ok());
}

/// A handler that always terminates contributes no fall-through path, so
/// the body's binding alone makes the name definite.
#[test]
fn a_terminating_handler_does_not_count_as_a_fall_through_path() {
    let src = "\
def f(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        raise
    return x
";
    assert!(parse_check(src).is_ok());
}

/// A body that always terminates excludes the `else` path, so the handlers
/// alone decide: here the only handler binds the name the body never does.
#[test]
fn a_terminating_body_leaves_the_handlers_to_decide() {
    let src = "\
def f() -> int:
    try:
        raise ValueError(\"v\")
    except ValueError:
        x = 1
    return x
";
    assert!(parse_check(src).is_ok());
}

/// An `else` that always terminates is excluded the same way: the handler
/// binds the name and the `else` path never reaches the read.
#[test]
fn a_terminating_else_is_not_a_fall_through_path() {
    let src = "\
def f(d: int) -> int:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = 1
    else:
        return y
    return x
";
    assert!(parse_check(src).is_ok());
}

/// `finally` also runs on the paths that leave early, so it sees the
/// conservative state (a read of a try-bound name inside it is `T0041`),
/// while the statement's own fall-through join still defines the name for
/// a read after it.
#[test]
fn finally_sees_the_conservative_state_and_the_read_after_it_the_join() {
    let read_inside = "\
def f(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = -1
    finally:
        print(x)
    return x
";
    assert_eq!(code_of(read_inside), "T0041");
    let read_after = "\
def f(d: int) -> int:
    count = 0
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = -1
    finally:
        count = count + 1
    return x + count
";
    assert!(parse_check(read_after).is_ok());
}

/// An `as` name is unbound when its handler exits (CPython's implicit
/// `del`), so a read after the statement is `T0041` even though the only
/// fall-through path is that handler's.
#[test]
fn an_as_name_is_unbound_after_its_handler() {
    let src = "\
def f() -> None:
    try:
        raise ValueError(\"v\")
    except ValueError as e:
        x = 1
    print(x)
    print(e)
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "T0041");
    assert!(err.message.contains("`e`"), "{}", err.message);
}

/// A name bound before the `try` is not re-typed by an `as` clause that
/// reuses it: the pre-existing `int` binding refuses the exception.
#[test]
fn a_pre_existing_name_reused_as_an_as_name_stays_t0023() {
    let src = "\
def f(d: int) -> None:
    e = 1
    try:
        x = 10 // d
    except ZeroDivisionError as e:
        x = 0
    print(x)
";
    assert_eq!(code_of(src), "T0023");
}

/// A handler's own `as` name is not compared with the body's binding of the
/// same spelling: the body's `e` is an `int` and the handler's is the
/// exception instance, and the statement is accepted as CPython accepts it.
/// The handler's exit unbinds `e`, so it stays possibly unbound and a read
/// after the statement is `T0041`, which is what makes the skip safe.
#[test]
fn an_as_name_is_exempt_from_the_type_walk_and_stays_maybe() {
    let base = "\
def f(d: int) -> None:
    try:
        e = 10 // d
    except ZeroDivisionError as e:
        print(\"zero\")
";
    assert!(parse_check(&format!("{base}    print(1)\n")).is_ok());
    let err = parse_check(&format!("{base}    print(e)\n")).unwrap_err();
    assert_eq!(err.code, "T0041");
    assert!(err.message.contains("`e`"), "{}", err.message);
}

/// Module scope, with no `def` wrapper: every handler binds `x`, so the
/// read after the statement is accepted.
#[test]
fn at_module_scope_a_name_bound_on_every_path_is_definite() {
    let src = "\
d = 0
try:
    x = 10 // d
except ZeroDivisionError:
    x = -1
print(x)
";
    assert!(parse_check(src).is_ok());
}

/// Module scope: a handler that falls through without binding `x` leaves it
/// possibly unbound, so the read after the statement is `T0041`.
#[test]
fn at_module_scope_a_handler_that_does_not_bind_is_t0041() {
    let src = "\
d = 0
try:
    x = 10 // d
except ZeroDivisionError:
    y = 1
print(x)
";
    assert_eq!(code_of(src), "T0041");
}

/// A name any handler binds with `as` is never promoted by the try join,
/// even when the handler always terminates and every fall-through path binds
/// the name: its slot holds the exception instance on that handler's path,
/// and codegen cannot share it with a later read of an `int`. pycc refuses
/// what CPython accepts, rather than miscompiling it.
#[test]
fn an_as_name_is_never_promoted_even_when_its_handler_terminates() {
    let function = "\
def f(d: int) -> int:
    try:
        e = 10 // d
    except ZeroDivisionError as e:
        raise
    return e
print(f(5))
";
    assert_eq!(code_of(function), "T0041");
    let module = "\
d = 5
try:
    e = 10 // d
except ZeroDivisionError as e:
    raise
print(e)
";
    assert_eq!(code_of(module), "T0041");
    let other_handler_binds = "\
def f(d: int) -> int:
    try:
        e = 10 // d
    except ZeroDivisionError as e:
        raise
    except ValueError:
        e = 0
    return e
";
    assert_eq!(code_of(other_handler_binds), "T0041");
}

/// The type walk leaves a handler's `as` name out on every path, not only
/// its own handler's: a body binding of the same spelling must not replace
/// the exception type the pre-#1289 join gives it, so a later `e = 5` is
/// still `T0023` against that type, as on main. The other-handler variant
/// puts the plain binding first so the walk would otherwise establish `int`.
#[test]
fn an_as_name_keeps_the_conservative_type_when_another_path_binds_it() {
    let function = "\
def f(c: bool) -> None:
    try:
        if c:
            e = 1
    except ZeroDivisionError as e:
        pass
    e = 5
    print(e)
f(True)
";
    assert_eq!(code_of(function), "T0023");
    let module = "\
c = True
try:
    if c:
        e = 1
except ZeroDivisionError as e:
    pass
e = 5
print(e)
";
    assert_eq!(code_of(module), "T0023");
    let other_handler = "\
def f(d: int) -> None:
    try:
        x = 10 // d
    except ValueError:
        e = 1
    except ZeroDivisionError as e:
        x = 0
    e = 5
    print(e)
";
    assert_eq!(code_of(other_handler), "T0023");
    let star = "\
def f(c: bool) -> None:
    try:
        if c:
            e = 1
    except* ZeroDivisionError as e:
        pass
    e = 5
    print(e)
f(True)
";
    assert_eq!(code_of(star), "T0023");
}

// -- The check phase: the type walk --

/// Two `as` handlers binding the same spelling to different exception
/// types are accepted: each handler's own `as` name is skipped by the walk.
/// A non-`as` name bound with incompatible types by two handlers is not.
#[test]
fn the_type_walk_skips_only_a_handlers_own_as_name() {
    let two_as = "\
def f(n: int) -> int:
    try:
        x = 10 // n
    except ZeroDivisionError as e:
        print(e)
        x = 1
    except ValueError as e:
        print(e)
        x = 2
    return x
";
    assert!(parse_check(two_as).is_ok());
    let clash = "\
def f(n: int) -> None:
    try:
        x = 10 // n
    except ZeroDivisionError:
        y = 1
    except ValueError:
        y = \"s\"
";
    assert_eq!(code_of(clash), "T0023");
}

/// b1: a handler binds `bool` and `else` binds `int`. The walk checks in
/// `check_assignment`'s direction (the later path must be assignable to the
/// first), and `int` is not assignable to `bool`.
#[test]
fn a_bool_handler_then_an_int_else_is_t0023() {
    let src = "\
def f(d: int) -> None:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = False
    else:
        x = 1
    print(x)
";
    assert_eq!(code_of(src), "T0023");
}

/// P1: a handler binds `int` and `else` binds `bool`, which is assignable to
/// `int`; the post-`try` type is the first-established `int`.
#[test]
fn an_int_handler_then_a_bool_else_is_accepted() {
    let src = "\
def f(d: int) -> None:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = 1
    else:
        x = y > 100
    print(x)
";
    assert!(parse_check(src).is_ok());
}

/// P2 (no read) and P7 (a read): an `int` body with a `bool` handler.
#[test]
fn an_int_body_with_a_bool_handler_is_accepted_with_or_without_a_read() {
    let no_read = "\
def f(d: int) -> None:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = False
";
    assert!(parse_check(no_read).is_ok());
    let read = format!("{no_read}    print(x)\n");
    assert!(parse_check(&read).is_ok());
}

/// The post-`try` type is the walk's first-established one, not the
/// conservative join's. That join takes a `Definitely` side's type over a
/// `Maybe` one, which for P1 is the `else` path's `bool` over the handler's
/// `int` -- the reversed choice behind the `if`/`else` miscompile described
/// in `join_try_outcome`'s doc comment. After the statement `x` is an `int`,
/// so it refuses a `bool` annotation and accepts a later `int` rebinding.
#[test]
fn the_post_try_type_is_the_first_established_one() {
    let base = "\
def f(d: int) -> None:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = 1
    else:
        x = y > 100
";
    assert_eq!(
        code_of(&format!("{base}    b: bool = x\n    print(b)\n")),
        "T0025"
    );
    assert!(parse_check(&format!("{base}    x = 5\n    print(x)\n")).is_ok());
}

/// P4: a body binding under an `if` leaves the name `Maybe`, and a `Maybe`
/// binding still claims the slot's type, so a `str` handler is `T0023`.
#[test]
fn a_maybe_body_binding_still_fixes_the_type() {
    let src = "\
def f(d: int) -> None:
    try:
        if d > 0:
            x = 10 // d
    except ZeroDivisionError:
        x = \"s\"
";
    assert_eq!(code_of(src), "T0023");
}

// -- The check phase: `except*` shares the same tail --

#[test]
fn try_star_binding_on_every_path_is_accepted() {
    let src = "\
def f(d: int) -> int:
    try:
        x = 10 // d
    except* ZeroDivisionError:
        x = -1
    return x
";
    assert!(parse_check(src).is_ok());
}

#[test]
fn try_star_handler_that_does_not_bind_is_t0041() {
    let src = "\
def f(d: int) -> int:
    try:
        x = 10 // d
    except* ZeroDivisionError:
        y = 1
    return x
";
    assert_eq!(code_of(src), "T0041");
}

// -- The constraint solver's mirror --

/// An unannotated private helper whose `try` binds the returned name on
/// every fall-through path now has an inferable return type, for both
/// statement forms, including a `bool` handler over an `int` body.
#[test]
fn the_solver_promotes_a_name_bound_on_every_fall_through_path() {
    for handler in ["except ZeroDivisionError:", "except* ZeroDivisionError:"] {
        for value in ["-1", "False"] {
            let src = format!(
                "\
def _h(d):
    try:
        r = 10 // d
    {handler}
        r = {value}
    return r


print(_h(0))
"
            );
            assert!(parse_check(&src).is_ok(), "{handler} {value}");
        }
    }
}

/// No fall-through path: every path returns, and nothing is promoted.
/// This pins `promote_try_fallthrough`'s empty-path early return only; the
/// promotion logic itself is pinned by the tests around it.
#[test]
fn the_solver_promotes_nothing_without_a_fall_through_path() {
    let src = "\
def _h(d):
    try:
        return 10 // d
    except ZeroDivisionError:
        return -1


print(_h(0))
";
    assert!(parse_check(src).is_ok());
}

/// A handler that falls through without binding the name keeps it maybe,
/// so the helper's return type is still not inferable.
#[test]
fn the_solver_keeps_a_name_maybe_when_one_path_misses_it() {
    let src = "\
def _h(d):
    try:
        r = 10 // d
    except ZeroDivisionError:
        q = 1
    return r


print(_h(0))
";
    assert_eq!(code_of(src), "T0021");
}

/// A terminating handler is excluded, so the body alone promotes the name.
#[test]
fn the_solver_excludes_a_terminating_handler() {
    let src = "\
def _h(d):
    try:
        r = 10 // d
    except* ZeroDivisionError:
        raise
    return r


print(_h(1))
";
    assert!(parse_check(src).is_ok());
}

/// A body that always terminates excludes the `else` path in the solver
/// too, so the handler alone promotes the name.
#[test]
fn the_solver_excludes_the_else_path_after_a_terminating_body() {
    let src = "\
def _h(d):
    try:
        raise ValueError(\"v\")
    except ValueError:
        r = d
    return r


print(_h(0))
";
    assert!(parse_check(src).is_ok());
}

/// Every path must bind the name, not only the first one collected: the
/// handler binds `r` but the `else` path does not.
#[test]
fn the_solver_requires_every_fall_through_path() {
    let src = "\
def _h(d):
    try:
        q = 10 // d
    except ZeroDivisionError:
        r = -1
    return r


print(_h(0))
";
    assert_eq!(code_of(src), "T0021");
}

/// A name bound before the `try` is never re-promoted by it: a pre-existing
/// name keeps the state the statements before the `try` gave it.
#[test]
fn the_solver_leaves_a_pre_existing_name_alone() {
    let src = "\
def _h(d):
    if d > 0:
        r = 1
    try:
        r = 10 // d
    except ZeroDivisionError:
        r = -1
    return r


print(_h(0))
";
    assert_eq!(code_of(src), "T0021");
}

/// The `as` name is unbound after its handler, so returning it is still not
/// inferable even though it is that handler's only fall-through path.
#[test]
fn the_solver_keeps_an_as_name_maybe_after_its_handler() {
    let src = "\
def _h(d):
    try:
        raise ValueError(\"v\")
    except ValueError as e:
        r = 1
    return e


print(_h(0))
";
    assert_eq!(code_of(src), "T0021");
}

/// The solver mirrors the check phase: a name any handler binds with `as`
/// is not promoted even when its handler terminates, so the helper's return
/// type is not inferable from it.
#[test]
fn the_solver_never_promotes_an_as_name() {
    let src = "\
def _h(d):
    try:
        e = 10 // d
    except ZeroDivisionError as e:
        raise
    return e


print(_h(5))
";
    assert_eq!(code_of(src), "T0021");
}
