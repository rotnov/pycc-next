//! Type checking for builtin exceptions (#382).

use std::collections::HashMap;

use super::env::BindingState;
use super::{
    Environment, HirExpr, HirStmt, Ty, block_always_returns, infer_expr_in, join_if_branches,
    join_loop_body,
};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    EXCEPTION_INIT_MANGLED_NAME, HirClassDef, HirExceptHandler, except_handler_binding_type_name,
};

mod try_join;
use try_join::{TryPaths, join_try_outcome};

pub(super) fn check_try_stmt(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    handlers: &[HirExceptHandler],
    orelse: &[HirStmt],
    finalbody: &[HirStmt],
    return_ty: Option<&Ty>,
) -> Result<(), Diagnostic> {
    let mut body_env = env.clone();
    check_stmt_sequence_shared(&mut body_env, local_names, body, return_ty)?;

    let mut handler_envs = Vec::with_capacity(handlers.len());
    for handler in handlers {
        let mut handler_env = env.clone();
        // Issue #769 follow-up (D-068 re-review of #780, third round): a
        // handler runs only after *some* prefix of `body` already
        // executed -- unlike `else_env` (which starts from `body_env`'s
        // full post-execution state because it only runs after `body`
        // completes normally), a handler can be entered after any
        // partial execution, so no single point-in-time snapshot of
        // `body`'s narrowing state is safe to hand it. Conservatively
        // prescan-drop every name `body` kills *anywhere* from
        // `handler_env`'s narrowing overlay before checking the handler,
        // rather than starting from the pre-try `env` clone unmodified
        // (which is what let a read of a name `body` reassigned to
        // `None` right before raising still see it narrowed). See
        // `narrow::apply_kill_prescan`'s doc comment in
        // `crates/pycc_types/src/narrow.rs` for the full rationale.
        super::narrow::apply_kill_prescan(&mut handler_env, body);
        // #1244: likewise, a name `body` deletes anywhere may be unbound.
        super::narrow::apply_delete_prescan(&mut handler_env, body, None);
        handler_env.in_except_handler = true;
        if let Some(exc_types) = &handler.exc_type {
            // PEP 758 (#740): a handler may name more than one exception
            // type (`except A, B:` / `except (A, B):`). Every named type is
            // validated independently, in source order, failing on the
            // *first* invalid name -- no diagnostic in this codebase
            // batches multiple errors together.
            for exc_type in exc_types {
                let builtin = is_unshadowed_builtin_exception(&body_env, local_names, exc_type);
                if !builtin {
                    // Part 2 of #541 (D-189): a user-declared class whose MRO
                    // reaches a builtin exception is catchable too.
                    let Some(def) = user_exception_class(&body_env, local_names, exc_type) else {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!(
                                "`{exc_type}` is not a recognized exception class — only builtin exception classes and classes derived from them are supported in `except` handlers"
                            ),
                            Span::new(0, 0),
                        ));
                    };
                    reject_own_constructor(&body_env, def)?;
                    if handler.name.is_some() {
                        // Deliberately *not* supported in Part 2, and not merely
                        // for want of the feature: binding the caught value would
                        // give it a `Ty::Instance(exc_type)`, and every consumer
                        // of that type reads an instance as a `PyInstanceObj`.
                        // The value the runtime actually holds is a
                        // `PyExceptionObj` — a different layout entirely — so the
                        // binding would be a type confusion, not a missing
                        // capability. Part 3 of #541 (#703) materializes a real
                        // instance; until then this must stay rejected. This
                        // applies per-name: any user-defined class among a
                        // multi-type handler's names with an `as` binding is
                        // rejected, naming the specific offending class.
                        return Err(Diagnostic::error(
                            "C0001",
                            format!(
                                "binding a caught `{exc_type}` with `as` is not supported \
                                 yet; pycc does not materialize an exception instance \
                                 for a user-defined exception class (Part 3 of #541)"
                            ),
                            Span::new(0, 0),
                        ));
                    }
                }
            }
            if let Some(name) = &handler.name {
                let binding_type = except_handler_binding_type_name(exc_types);
                handler_env.bind(name.clone(), Ty::Instance(Box::new(binding_type)));
                // D-068 re-review of #780 (fourth round): `bind` only
                // overwrites `bindings`, never `narrowed` (see `bind`'s own
                // doc comment in `env.rs`), so a name previously narrowed by
                // an enclosing `if <name> is not None:` stayed narrowed
                // here even though it is now bound to the caught exception
                // instance -- a later read inside the handler body would be
                // type-checked as the narrowed `int`/whatever, not
                // `Instance(binding_type)`, via `HirExpr::Name`'s
                // `narrowed_ty` preference (`expr.rs`). Kill it explicitly,
                // mirroring `check_assignment`'s own unconditional
                // `env.narrowed.remove(target)` for every other reassignment
                // kind -- this handler binding is not routed through
                // `check_assignment` itself (it must skip that function's
                // previous-type compatibility check, which would wrongly
                // reject `except X as name:` reusing a name whose earlier
                // type is incompatible with the exception instance type).
                handler_env.narrowed.remove(name);
            }
        }
        // #1485: the `except ImportError` fallback of a foreign import.
        super::foreign::seed_fallback_rebindings(&mut handler_env, body, &handler.body);
        check_stmt_sequence_shared(&mut handler_env, local_names, &handler.body, return_ty)?;
        handler_envs.push(handler_env);
    }

    // `else` runs only after the try body completes successfully, so it sees
    // bindings established by that successful path. Starting from the
    // pre-try environment silently rejected valid reads such as
    // `try: x = 1; ...; else: print(x)`.
    let mut else_env = body_env.clone();
    check_stmt_sequence_shared(&mut else_env, local_names, orelse, return_ty)?;

    join_try_outcome(
        env,
        local_names,
        return_ty,
        TryPaths {
            body,
            handlers,
            orelse,
            finalbody,
            body_env: &body_env,
            handler_envs: &handler_envs,
            else_env: &else_env,
        },
    )
}

/// `try: ... except* T: ...` (PEP 654, Part 3 of #382, #542).
///
/// Structurally mirrors [`check_try_stmt`] -- same body/handler/orelse/
/// finalbody scoping and environment-joining shape -- but differs in what an
/// `as` binding resolves to. Plain `except E as e:` binds `e` to `E` itself;
/// `except* E as e:` always binds `e` to an `ExceptionGroup` containing the
/// matched subgroup, never to `E`, because `except*` always operates on
/// groups (PEP 654 section "Runtime Semantics"). pycc has no first-class
/// generic-group type distinct from the flat `ExceptionGroup` name, so the
/// binding is `Ty::Instance("ExceptionGroup")` unconditionally.
pub(super) fn check_try_star_stmt(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    handlers: &[HirExceptHandler],
    orelse: &[HirStmt],
    finalbody: &[HirStmt],
    return_ty: Option<&Ty>,
) -> Result<(), Diagnostic> {
    let mut body_env = env.clone();
    check_stmt_sequence_shared(&mut body_env, local_names, body, return_ty)?;

    let mut handler_envs = Vec::with_capacity(handlers.len());
    for (index, handler) in handlers.iter().enumerate() {
        let mut handler_env = env.clone();
        // D-068 re-review of #780 (rebase onto #542's except* landing):
        // mirrors `check_try_stmt`'s identical prescan above -- `except*`
        // dispatches subgroups the same way a plain `except` handler does,
        // so a handler here can equally be entered after only some prefix
        // of `body` executed. Without this, a name `body` reassigns to
        // `None` right before raising could still read as narrowed inside
        // a handler that runs after that reassignment. See
        // `narrow::apply_kill_prescan`'s doc comment in
        // `crates/pycc_types/src/narrow.rs` for the full rationale.
        super::narrow::apply_kill_prescan(&mut handler_env, body);
        // #1244: several `except*` handlers can run in sequence for one
        // group, each seeing what the earlier ones left, so handler `index`
        // sees the deletions of the body *and* of every earlier handler.
        super::narrow::apply_delete_prescan(&mut handler_env, body, None);
        for earlier in &handlers[..index] {
            super::narrow::apply_delete_prescan(&mut handler_env, &earlier.body, None);
        }
        handler_env.in_except_handler = true;
        // A bare `except*:` is rejected by ruff's own parser as a syntax
        // error (PEP 654 requires every `except*` clause to name a type),
        // so `handler.exc_type` is always `Some` by the time an
        // `HirStmt::TryStar` reaches type-checking -- see
        // `pycc_hir::stmt`'s own comment on the matching lowering site for
        // why no defensive `None` re-check belongs here either.
        if let Some(exc_types) = &handler.exc_type {
            for exc_type in exc_types {
                let builtin = is_unshadowed_builtin_exception(&body_env, local_names, exc_type);
                if builtin && matches!(exc_type.as_str(), "ExceptionGroup" | "BaseExceptionGroup") {
                    // #795 (PEP 654): CPython *accepts* `except*
                    // ExceptionGroup:` at compile time and raises
                    // `TypeError: catching ExceptionGroup with except* is not
                    // allowed. Use except instead.` at handler-match time
                    // instead. pycc cannot model that today -- D-173
                    // propagates a raised exception through global runtime
                    // state rather than an allocated group instance, so there
                    // is no materialized value to type-test at match time --
                    // so this is a deliberate, documented divergence:
                    // `C0001` ("valid Python this compiler does not implement
                    // yet"), which is the honest code precisely because the
                    // program *is* valid Python. #903 tracks delivering the
                    // real runtime `TypeError`; the narrowing ADR beside
                    // D-202 records the divergence.
                    //
                    // Gated on `builtin` so a module that shadows the name
                    // (`class ExceptionGroup: ...`) keeps reaching the
                    // existing user-class path below rather than being
                    // rejected for a name it does not actually mean.
                    return Err(Diagnostic::error(
                        "C0001",
                        format!(
                            "catching `{exc_type}` with `except*` is not supported yet — CPython rejects it at runtime with a `TypeError`, which this compiler cannot raise yet; use a plain `except` clause instead"
                        ),
                        Span::new(0, 0),
                    ));
                }
                if !builtin {
                    let Some(def) = user_exception_class(&body_env, local_names, exc_type) else {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!(
                                "`{exc_type}` is not a recognized exception class — only builtin exception classes and classes derived from them are supported in `except*` handlers"
                            ),
                            Span::new(0, 0),
                        ));
                    };
                    if let Some(group) = derived_group_ancestor(def) {
                        // #795 (PEP 654), second round: CPython's runtime
                        // `TypeError` fires for *any* class whose MRO reaches
                        // `BaseExceptionGroup`, not only for the two group
                        // names themselves, so a user-defined subclass must be
                        // refused here exactly like the exact names above are.
                        // Without this the compiler would silently lower
                        // `except* G:` (with `class G(ExceptionGroup)`) into
                        // ordinary tag matching and produce the wrong runtime
                        // answer -- the very outcome the divergence recorded
                        // beside D-202 exists to prevent. Placed before
                        // `reject_own_constructor` so a group subclass that
                        // also declares its own `__init__` reports the group
                        // divergence, which is the reason the program can
                        // never be compiled, rather than the constructor gap,
                        // which #703 will eventually close.
                        return Err(Diagnostic::error(
                            "C0001",
                            format!(
                                "catching `{exc_type}` with `except*` is not supported yet — it derives from `{group}`, and CPython rejects catching a group class with `except*` at runtime with a `TypeError` this compiler cannot raise yet; use a plain `except` clause instead"
                            ),
                            Span::new(0, 0),
                        ));
                    }
                    reject_own_constructor(&body_env, def)?;
                }
            }
        }
        if let Some(name) = &handler.name {
            // Bound to `ExceptionGroup` unconditionally, whatever the module
            // contains (the same literal in `monomorphize.rs` and
            // `constraints.rs`); when the module shadows that name, or
            // shadows any builtin exception name so the seeded table was
            // withheld, `pycc_types::string_conversion` (#977, D-237) is what
            // keeps `print(eg)`/`f"{eg}"` from reaching the backend.
            handler_env.bind(
                name.clone(),
                Ty::Instance(Box::new("ExceptionGroup".to_string())),
            );
            // D-068 re-review of #780 (rebase onto #542's except* landing):
            // mirrors `check_try_stmt`'s identical fix above -- `bind`
            // overwrites `bindings`, never `narrowed`, so a name previously
            // narrowed by an enclosing `if <name> is not None:` would stay
            // narrowed in `handler_env` even though `name` now names the
            // caught `ExceptionGroup` instance, not its old Optional value.
            // Kill it explicitly, the same way a plain reassignment does.
            handler_env.narrowed.remove(name);
        }
        check_stmt_sequence_shared(&mut handler_env, local_names, &handler.body, return_ty)?;
        handler_envs.push(handler_env);
    }

    let mut else_env = body_env.clone();
    check_stmt_sequence_shared(&mut else_env, local_names, orelse, return_ty)?;

    join_try_outcome(
        env,
        local_names,
        return_ty,
        TryPaths {
            body,
            handlers,
            orelse,
            finalbody,
            body_env: &body_env,
            handler_envs: &handler_envs,
            else_env: &else_env,
        },
    )
}

pub(super) fn check_raise_stmt(
    env: &Environment,
    local_names: &[&str],
    exc: &Option<HirExpr>,
    cause: &Option<HirExpr>,
) -> Result<(), Diagnostic> {
    let mut object_operand = false;
    if let Some(exc) = exc {
        object_operand |=
            check_raise_operand(env, local_names, exc, "can only raise exception instances")?
                == RaiseOperand::Object;
    } else if !env.in_except_handler {
        return Err(Diagnostic::error(
            "T0021",
            "bare `raise` is only valid inside an except handler",
            Span::new(0, 0),
        ));
    }
    if let Some(cause) = cause {
        object_operand |= check_raise_operand(
            env,
            local_names,
            cause,
            "cause must be an exception instance",
        )? == RaiseOperand::Object;
        if object_operand {
            // Part 9 of #1371: `raise o` is admitted, but chaining a cause
            // onto, or from, a CPython object is not built yet.
            return Err(Diagnostic::error(
                "C0001",
                "`raise ... from ...` with a CPython object as the exception or the cause is not supported yet",
                Span::new(0, 0),
            ));
        }
    }
    Ok(())
}

/// What a `raise` operand [`check_raise_operand`] admitted is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RaiseOperand {
    /// A pycc exception: a constructor call or an existing instance.
    Native,
    /// A CPython object (`Ty::Object`, Part 9 of #1371). Whether it is an
    /// exception at all is CPython's to decide at run time, exactly as for
    /// CPython's own `raise`: an instance is raised, a class is
    /// instantiated, and anything else raises `TypeError`.
    Object,
}

/// Part 3 of #382 (#542, PEP 654, D-202): the narrow, literal-list-only
/// relaxation for `ExceptionGroup(msg, [exc1, exc2, ...])` /
/// `BaseExceptionGroup(msg, [...])` construction inside a `raise` operand.
///
/// D-105/T0021 makes `list[T]` annotations unsupported in general, so this
/// deliberately does not accept an arbitrary `list[BaseException]`-typed
/// expression as the second argument -- only a literal `[...]` written
/// directly at the call site, each element of which is validated by
/// [`check_exception_group_member_operand`], a narrower check than
/// [`check_raise_operand`]: it requires an *existing* exception value and
/// rejects any fresh constructor-call member, including a nested inline
/// `ExceptionGroup(...)`/`BaseExceptionGroup(...)` call, with `T0021`. A
/// member that fails its own validation reports that specific element's
/// diagnostic rather than a generic group-level one.
fn check_exception_group_operand(
    env: &Environment,
    local_names: &[&str],
    callee: &str,
    args: &[HirExpr],
) -> Result<(), Diagnostic> {
    if args.len() != 2 {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects exactly 2 arguments (a message string and a literal list \
                 of member exceptions), got {}",
                args.len()
            ),
            Span::new(0, 0),
        ));
    }
    let message_ty = infer_expr_in(env, local_names, &args[0])?;
    if message_ty != Ty::Str {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects a `str` message argument, got `{}`",
                message_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    let HirExpr::ListLiteral(members) = &args[1] else {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}`'s second argument must be a literal list of member exceptions \
                 (`[e1, e2, ...]`); a computed or `list[T]`-typed expression is not supported"
            ),
            Span::new(0, 0),
        ));
    };
    if members.is_empty() {
        return Err(Diagnostic::error(
            "T0021",
            format!("`{callee}` requires at least one member exception"),
            Span::new(0, 0),
        ));
    }
    for member in members {
        check_exception_group_member_operand(env, local_names, member)?;
    }
    Ok(())
}

/// Part 3 of #382 (#542, PEP 654, D-202): validates one `ExceptionGroup`/
/// `BaseExceptionGroup` member expression.
///
/// Deliberately narrower than [`check_raise_operand`]: a member must be an
/// *existing* exception value (e.g. a `except ... as e:` binding, or another
/// name/expression that already evaluates to one), not a fresh
/// `SomeError("msg")` constructor call, even though such a call is a
/// perfectly valid top-level `raise` operand. `pycc_mir::exception::
/// lower_exception_value`'s `ConstructedGroup` arm lowers each member through
/// ordinary expression lowering (`lower_expr`), which has no way to
/// construct a *new* exception object -- only `lower_exception_value` itself
/// (used for the group's own top-level `raise` operand, and for a plain,
/// non-group `raise SomeError(...)`) knows how to do that. Accepting a fresh
/// constructor-call member here would type-check successfully under
/// `check_raise_operand`'s own rules and then panic in codegen (member
/// lowering would resolve `SomeError` as an ordinary user function, which it
/// is not), so this narrower rule is enforced structurally at the type-check
/// boundary rather than left for MIR/codegen to reject.
fn check_exception_group_member_operand(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    if let HirExpr::Call { callee, .. } = expr
        && (is_unshadowed_builtin_exception(env, local_names, callee)
            || user_exception_class(env, local_names, callee).is_some())
    {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "an `ExceptionGroup`/`BaseExceptionGroup` member must be an existing exception \
                 value (e.g. a caught `except ... as e:` binding), not a fresh `{callee}(...)` \
                 construction -- assign it to a name first, then list that name"
            ),
            Span::new(0, 0),
        ));
    }
    let ty = infer_expr_in(env, local_names, expr)?;
    if let Ty::Instance(class_name) = &ty
        && pycc_hir::is_builtin_exception_class(class_name)
        && !is_user_defined_class(env, class_name)
    {
        // D-202's sixth simplification: `pycc_rt_exception_group_partition`
        // matches each member by its own top-level `type_tag` only and never
        // recurses into a member's own `exceptions`/`exceptions_len` array
        // when that member is itself a group -- unlike CPython's `split()`,
        // which does recurse into nested groups. `ExceptionGroup`/
        // `BaseExceptionGroup` are themselves ordinary entries in
        // `BUILTIN_EXCEPTION_CLASSES`, so without this check a value already
        // bound to one type-checks as a member of a freshly constructed
        // *outer* group (e.g. `ExceptionGroup("outer", [eg])` where `eg`
        // came from a prior `except* ValueError as eg:`), silently building
        // a nested group the runtime cannot partition correctly. Rejecting
        // it here keeps the type checker's accepted surface matching what
        // the runtime actually implements, the same enforce-at-type-check-
        // boundary approach this function already takes for a fresh
        // constructor-call member above.
        if class_name.as_str() == "ExceptionGroup" || class_name.as_str() == "BaseExceptionGroup" {
            return Err(Diagnostic::error(
                "T0021",
                "an `ExceptionGroup`/`BaseExceptionGroup` member must not itself be an \
                 exception group -- nested groups are not supported"
                    .to_string(),
                Span::new(0, 0),
            ));
        }
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0021",
        format!(
            "an `ExceptionGroup`/`BaseExceptionGroup` member must be an exception instance, got `{}`",
            ty.name()
        ),
        Span::new(0, 0),
    ))
}

fn check_raise_operand(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
    error_prefix: &str,
) -> Result<RaiseOperand, Diagnostic> {
    if let HirExpr::Call { callee, args } = expr
        && (callee == "ExceptionGroup" || callee == "BaseExceptionGroup")
        && is_unshadowed_builtin_exception(env, local_names, callee)
    {
        return check_exception_group_operand(env, local_names, callee, args)
            .map(|()| RaiseOperand::Native);
    }
    if let HirExpr::Call { callee, args } = expr
        && is_unshadowed_builtin_exception(env, local_names, callee)
    {
        if args.len() != 1 {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`{callee}` expects exactly 1 argument (the message string), got {}",
                    args.len()
                ),
                Span::new(0, 0),
            ));
        }
        let argument_type = infer_expr_in(env, local_names, &args[0])?;
        if argument_type != Ty::Str {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`{callee}` expects a `str` message argument, got `{}`",
                    argument_type.name()
                ),
                Span::new(0, 0),
            ));
        }
        return Ok(RaiseOperand::Native);
    }

    // Part 2 of #541 (D-189): `raise MyError("boom")` for a user-declared class
    // whose MRO reaches a builtin exception.
    //
    // This acceptance is keyed *structurally*, on the `HirExpr::Call` shape
    // matched above, and never on the inferred type. `e = MyError("x"); raise e`
    // infers the identical `Ty::Instance("MyError")` as `raise MyError("boom")`,
    // so widening the `Ty::Instance` predicate below would silently admit the
    // bound-value form -- and MIR lowers that form to
    // `MirExceptionValue::Existing`, which codegen hands to
    // `pycc_rt_exception_raise` as a `*mut PyExceptionObj` while the value is
    // really a `*mut PyInstanceObj`. That is memory corruption, not a
    // diagnostic. `raise <bound value>` therefore stays `T0021`.
    if let HirExpr::Call { callee, args } = expr
        && let Some(def) = user_exception_class(env, local_names, callee)
    {
        // #714: this branch used to reach `class::resolve_instantiation`
        // (via the `infer_expr_in` call this replaced), which rejects an
        // abstract (#380, PEP 3119) class before ever resolving its
        // constructor. Bypassing that function for the argument-validation
        // shortcut below must not silently drop that pre-existing guard for
        // the one path that still needs it -- a `class MyError(Exception,
        // ABC):` with an unimplemented `@abstractmethod` and no `__init__`
        // override must stay rejected for `raise MyError("boom")` exactly
        // as it was before this diff.
        //
        // `resolve_instantiation`'s sibling `is_protocol` guard has no
        // equivalent here: `is_protocol` is true only for `class X(Protocol):`
        // with `Protocol` as the class's *sole* base (`pycc_hir::class`'s own
        // lowering requires `bases.len() == 1`), which structurally precludes
        // also inheriting `Exception` -- so `def.is_protocol` can never be
        // true for a `def` this branch reaches (`user_exception_class`
        // already filtered on `exception_type_tag.is_some()`). Duplicating
        // that dead branch here would need a synthetic-only test to satisfy
        // D-014's 100%-region gate for a shape no real compiler input can
        // produce, exactly the pattern this crate's own established
        // convention (see `redeclaration.rs`'s MRO-lookup `.expect()`
        // precedent) avoids.
        if def.is_abstract {
            return Err(Diagnostic::error(
                "C0001",
                format!(
                    "cannot instantiate abstract class `{callee}` -- \
                     it has unimplemented abstract methods; subclass it and \
                     override all `@abstractmethod`-decorated methods first"
                ),
                Span::new(0, 0),
            ));
        }
        reject_own_constructor(env, def)?;
        // #714: argument validation is done directly here, against the
        // synthetic `Exception.__init__` placeholder's own registered
        // signature, instead of routing through the generic
        // `infer_expr_in` -> `class::resolve_instantiation` path used
        // everywhere else a `Call` is type-checked. `resolve_instantiation`
        // rejects *any* instantiation whose resolved constructor is still
        // that placeholder, precisely to stop `e = MyError("boom")` (an
        // ordinary value binding) from compiling into a call through a
        // function-pointer slot that is bound too late in module-execution
        // order (see `class::binding::resolve_instantiation`'s own comment
        // on this) -- but a fresh `raise MyError("boom")` is the one shape
        // that placeholder exists to support, and MIR's
        // `lower_exception_value` constructs the raised `PyExceptionObj`
        // directly from the message argument without ever calling through
        // `Exception.__init__` at runtime. Reusing `resolve_instantiation`
        // here would reject this valid raise the same way it now rejects
        // the bound-value form.
        // `reject_own_constructor` just confirmed the resolved `__init__`
        // *is* `EXCEPTION_INIT_MANGLED_NAME`, so it is guaranteed
        // registered -- matching `resolve_instantiation`'s own
        // "internal error" panic precondition, never actually reachable
        // here.
        let (param_tys, _return_ty) = env
            .lookup_function(EXCEPTION_INIT_MANGLED_NAME)
            .unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: `{EXCEPTION_INIT_MANGLED_NAME}` was not \
                     registered as an ordinary function -- `pycc_hir::lower_checked` seeds \
                     it whenever it seeds the builtin exception classes"
                )
            });
        let arg_tys = args
            .iter()
            .map(|arg| infer_expr_in(env, local_names, arg))
            .collect::<Result<Vec<_>, _>>()?;
        // `param_tys[0]` is always `self`, never part of the caller-supplied
        // argument list -- mirrors `resolve_instantiation`'s identical slice.
        super::class::check_call_args(env, callee, args, &arg_tys, &param_tys[1..], false)?;
        return Ok(RaiseOperand::Native);
    }

    let ty = infer_expr_in(env, local_names, expr)?;
    if matches!(&ty, Ty::Instance(class_name) if pycc_hir::is_builtin_exception_class(class_name) && !is_user_defined_class(env, class_name))
    {
        return Ok(RaiseOperand::Native);
    }
    // Part 9 of #1371: a CPython object -- a foreign class, a call of one
    // (`raise UnexpectedToken(token, expected)`), or any other `object`
    // value. Reached only by a callee the arms above did not claim: a name
    // a foreign import binds is in `env`'s bindings, so neither
    // `is_unshadowed_builtin_exception` nor `user_exception_class` accepts
    // it, even when it spells a builtin exception's name.
    if ty == Ty::Object {
        return Ok(RaiseOperand::Object);
    }
    Err(Diagnostic::error(
        "T0021",
        format!("{error_prefix}, got `{}`", ty.name()),
        Span::new(0, 0),
    ))
}

/// Part 2 of #543 (#739): whether `name` counts as an "unshadowed builtin
/// exception" -- catchable in `except <name>:` and raisable in
/// `raise <name>("...")` without a `HirClassDef` behind it.
///
/// This is `true` for the original flat seven purely by name-table
/// membership and ordinary shadow checks: `resolve_exception_tag`
/// (`pycc_mir::exception`) resolves them by name, independent of whether the
/// class table actually seeded a definition, so an unseeded module (see
/// `pycc_hir::exception::shadowed_builtin_exception_name`'s
/// all-or-nothing gate) still behaves correctly for them.
///
/// For every builtin past the flat seven (Part 2 of #543, #739 introduced
/// the first) this is *not* enough. Those names have no name-based fallback -- deliberately, so that
/// `pycc_mir::exception::handler_type_tags`'s MRO-containment scan never
/// needs special-casing -- so a name outside the flat seven must also be
/// *actually present* in `env.classes` to count as unshadowed. Without this
/// conjunct, a module shadowing one family member (e.g. `class OSError:
/// pass`) at top level withholds seeding for *every* builtin exception name (the shadow
/// gate is all-or-nothing), and a separate, unrelated `except
/// FileNotFoundError:` would then reach `handler_type_tags`'s `.expect()`
/// and panic instead of producing the `T0021` this function's `false` result
/// routes to. See the Part 2 of #543 implementation plan's "shadow-gate
/// crash risk" section for the full trigger and the two rejected
/// alternatives.
pub(super) fn is_unshadowed_builtin_exception(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> bool {
    pycc_hir::is_builtin_exception_class(name)
        & env.lookup_any(name).is_none()
        & !local_names.contains(&name)
        & !env.functions.contains_key(name)
        & !is_user_defined_class(env, name)
        & (pycc_hir::is_flat_builtin_exception_class(name) || env.classes.contains_key(name))
}

/// Whether `name` is registered as a *user-authored* class (Part 1 of
/// #541). Since HIR lowering now seeds a `HirClassDef` for each of the
/// seven builtin exception names, mere presence in `Environment::classes`
/// no longer means the user shadowed the name -- only a non-synthetic
/// entry does. Without this distinction every `except ValueError:` and
/// `raise ValueError("x")` in the language would start being rejected.
fn is_user_defined_class(env: &Environment, name: &str) -> bool {
    env.classes.contains_key(name) && !env.is_synthetic_class(name)
}

/// The [`HirClassDef`] of a user-declared, unshadowed exception class -- one
/// HIR lowering assigned a runtime type tag to, which it does exactly when the
/// class's MRO reaches one of the seeded builtin exception classes (Part 2 of
/// #541, D-189). `None` for anything else, including a class that never
/// touches the exception hierarchy and a name rebound by a local, a parameter,
/// or a function.
fn user_exception_class<'e>(
    env: &'e Environment,
    local_names: &[&str],
    name: &str,
) -> Option<&'e HirClassDef> {
    if env.lookup_any(name).is_some()
        || local_names.contains(&name)
        || env.functions.contains_key(name)
        || !is_user_defined_class(env, name)
    {
        return None;
    }
    env.classes
        .get(name)
        .filter(|def| def.exception_type_tag.is_some())
}

/// The builtin group class a user exception class derives from, if any
/// (#795, PEP 654).
///
/// `def.mro`'s first entry is the class's own name (both `compute_c3_mro`
/// and the synthetic builtin table build it that way), so the scan skips it:
/// this asks what the class *inherits*, not what it is named.
///
/// Unlike the exact-name check in `check_try_star_stmt`, this needs no
/// shadow gate. `def` only ever reaches here through `user_exception_class`,
/// which resolves nothing without an `exception_type_tag`, and HIR lowering
/// assigns that tag only when the class's MRO reaches a *seeded* builtin
/// exception class -- seeding a module withholds entirely once the module
/// defines a class of its own under a builtin exception name. So in a module
/// that shadows `ExceptionGroup` with an unrelated class of its own, no class
/// is tagged at all and `except* G:` reports `T0021` well before this scan
/// runs (see `a_subclass_of_a_shadowed_exception_group_class_reports_t0021`
/// in `except_star_tests`). A group name reaching this point is therefore
/// always the builtin one.
fn derived_group_ancestor(def: &HirClassDef) -> Option<&str> {
    def.mro
        .iter()
        .skip(1)
        .find(|ancestor| matches!(ancestor.as_str(), "ExceptionGroup" | "BaseExceptionGroup"))
        .map(String::as_str)
}

/// Rejects a raisable user exception class that declares (or inherits from a
/// non-synthetic ancestor) its own `__init__`.
///
/// Part 2 of #541 raises and catches by type tag alone: the message string is
/// the only payload `PyExceptionObj` carries, and it is filled from the single
/// argument the synthetic `Exception.__init__` accepts. A class with its own
/// constructor has state that never reaches the raised object, so accepting it
/// would silently drop the user's fields. Part 3 of #541 (#703) materializes a
/// real instance; until then this is a capability gap, reported as `C0001`.
fn reject_own_constructor(env: &Environment, def: &HirClassDef) -> Result<(), Diagnostic> {
    // #966: rank the MRO the way constructor resolution does -- a D-225
    // implicit zero-argument constructor ranks last, so skip it rather
    // than letting it decide raisability. Before this, `class Base: pass` /
    // `class MyError(Base, Exception): pass` was rejected purely because
    // `Base` carried an implicit stub; CPython raises that program fine.
    //
    // There is deliberately no all-implicit fallback pass here, unlike the
    // instantiation and `super()` sites. This function only ever runs on a
    // tagged class, whose MRO therefore reaches a seeded builtin, and
    // `pycc_hir::exception` gives the root `Exception` -- and only it -- a
    // real `__init__` entry that is never flagged. So the walk always
    // terminates on an unflagged candidate, and a fallback arm would be
    // unreachable code (D-014 scores every region).
    let init = def.mro.iter().find_map(|ancestor| {
        env.classes.get(ancestor.as_str()).and_then(|ancestor_def| {
            if ancestor_def.implicit_object_init {
                return None;
            }
            ancestor_def
                .methods
                .iter()
                .find(|(method, _)| method == "__init__")
                .map(|(_, mangled)| mangled.as_str())
        })
    });
    if init == Some(EXCEPTION_INIT_MANGLED_NAME) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "C0001",
        format!(
            "exception class `{}` declares or inherits an `__init__` other than \
             `Exception`'s; pycc supports only exception classes that inherit \
             `Exception`'s single-message constructor (Part 3 of #541)",
            def.name
        ),
        Span::new(0, 0),
    ))
}

/// D-068 re-review of #780 (third round, warning finding): a
/// return-type-generic sequence-checking helper, routing through
/// `narrow::check_stmt_sequence[_in_function]` instead of
/// `check_stmt`/`check_stmt_in_function` directly so a nested early-return
/// guard inside a `try`/`except`/`else`/`finally` body narrows the rest of
/// that same body -- `check_try_stmt`'s four body loops (`body`, each
/// handler's `body`, `orelse`, `finalbody`) previously called a
/// per-statement `check_stmt`/`check_stmt_in_function` dispatcher in a raw
/// loop, which never ran `apply_post_if_narrowing` and so silently dropped
/// this narrowing, exactly like the `if`/`while` fast-path helpers and
/// `check_match`'s own case-body loop had before their own fixes for the
/// same finding.
fn check_stmt_sequence_shared(
    env: &mut Environment,
    local_names: &[&str],
    stmts: &[HirStmt],
    return_ty: Option<&Ty>,
) -> Result<(), Diagnostic> {
    if let Some(return_ty) = return_ty {
        super::narrow::check_stmt_sequence_in_function(env, local_names, stmts, return_ty.clone())
    } else {
        super::narrow::check_stmt_sequence(env, stmts)
    }
}

#[cfg(test)]
mod synthetic_class_tests;

#[cfg(test)]
mod user_class_tests;

#[cfg(test)]
mod except_star_tests;
