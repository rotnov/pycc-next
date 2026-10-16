//! Generator functions: checking a `yield` against the generator's element
//! type.
//!
//! `pycc_hir` lowers a `yield value` statement to a call of
//! [`pycc_hir::YIELD_MARKER`] and gives the function the return type
//! `Generator[T]` (annotated, or `Generator[<inferred>]` for the solver to
//! fill from the yielded values).

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, HirStmt, Ty};

/// Checks `yield args[0]` in a function returning `return_ty`.
pub(crate) fn check_yield(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
    return_ty: &Ty,
) -> Result<(), Diagnostic> {
    let Ty::Generator(sig) = return_ty else {
        return Err(Diagnostic::error(
            "C0001",
            "`yield` is supported only in a function whose return type is a generator",
            Span::new(0, 0),
        ));
    };
    let element = &sig.0;
    let actual = infer_expr_in(env, local_names, &args[0])?;
    if crate::class::is_assignable_env(env, &actual, element) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0022",
        format!(
            "expected a yielded value of type `{}`, got `{}`",
            element.name(),
            actual.name()
        ),
        Span::new(0, 0),
    )
    .with_help(format!("yield a `{}` value", element.name())))
}

/// The builtins that consume a generator argument entirely (or, `any` and
/// `all`, until the first deciding value) and accept a list of its values
/// in its place.
pub fn is_generator_consumer(callee: &str) -> bool {
    matches!(
        callee,
        "list"
            | "sum"
            | "min"
            | "max"
            | "sorted"
            | "set"
            | "frozenset"
            | "any"
            | "all"
            | "tuple"
            | "__pycc_counter_of"
    )
}

/// `arg_tys` with a generator argument of a consumer seen as the list of
/// its values, or `None` when nothing changes.
pub(crate) fn consumer_arg_tys(callee: &str, arg_tys: &[Ty]) -> Option<Vec<Ty>> {
    if !is_generator_consumer(callee) || !arg_tys.iter().any(|t| matches!(t, Ty::Generator(_))) {
        return None;
    }
    // `min`/`max` of a generator of CPython objects run CPython's own over
    // it (`pycc_hir::builtins::resolve_call`).
    if matches!(callee, "min" | "max")
        && matches!(arg_tys, [Ty::Generator(sig)] if sig.0 == Ty::Object)
    {
        return None;
    }
    Some(
        arg_tys
            .iter()
            .map(|t| match t {
                Ty::Generator(sig) => Ty::List(Box::new(sig.0.clone())),
                other => other.clone(),
            })
            .collect(),
    )
}

/// `next(g)` / `next(g, default)` of a generator: its element type.
pub(crate) fn next_call(
    callee: &str,
    arg_tys: &[Ty],
    env: &Environment,
) -> Result<Option<Ty>, Diagnostic> {
    if callee != "next" {
        return Ok(None);
    }
    let Some(Ty::Generator(sig)) = arg_tys.first() else {
        return Ok(None);
    };
    let element = &sig.0;
    match arg_tys {
        [_] => Ok(Some(element.clone())),
        [_, default] if crate::class::is_assignable_env(env, default, element) => {
            Ok(Some(element.clone()))
        }
        [_, default] => Err(Diagnostic::error(
            "T0021",
            format!(
                "the default of `next()` on a `{}` must be a `{}`, got `{}`",
                arg_tys[0].name(),
                element.name(),
                default.name()
            ),
            Span::new(0, 0),
        )),
        _ => Err(Diagnostic::error(
            "T0033",
            format!("`next` expects 1 or 2 arguments, got {}", arg_tys.len()),
            Span::new(0, 0),
        )),
    }
}

/// The enclosing generator function's `(yield, send, return)` types, or the
/// diagnostic for a `yield` (or `yield from`) outside one.
fn enclosing(env: &Environment) -> Result<&(Ty, Ty, Ty), Diagnostic> {
    env.generator_sig.as_deref().ok_or_else(|| {
        Diagnostic::error(
            "C0001",
            "`yield` is supported only in a function whose return type is a generator",
            Span::new(0, 0),
        )
    })
}

/// `yield args[0]` used as a value: checks the yielded value and answers
/// the generator's send type, the type of the value sent in.
pub(crate) fn yield_value(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    let sig = enclosing(env)?.clone();
    check_yield(
        env,
        local_names,
        args,
        &Ty::Generator(Box::new(sig.clone())),
    )?;
    Ok(sig.1)
}

/// `return args[0]` in a generator: the value must suit its return type
/// (it becomes `StopIteration.value`).
pub(crate) fn check_return(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
    return_ty: &Ty,
) -> Result<(), Diagnostic> {
    let Ty::Generator(sig) = return_ty else {
        return Err(Diagnostic::error(
            "C0001",
            "`return` with a value in a generator needs a generator return type",
            Span::new(0, 0),
        ));
    };
    let actual = infer_expr_in(env, local_names, &args[0])?;
    if crate::class::is_assignable_env(env, &actual, &sig.2) {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0022",
        format!(
            "expected a generator return value of type `{}`, got `{}`",
            sig.2.name(),
            actual.name()
        ),
        Span::new(0, 0),
    )
    .with_help(format!(
        "annotate the generator `Generator[{}, {}, {}]`",
        sig.0.name(),
        sig.1.name(),
        actual.name()
    )))
}

/// `yield from operand`: the operand must be a generator yielding the
/// enclosing generator's element type (whose send type, when it takes
/// values, is the enclosing one's) -- or, as a statement (`statement`), a
/// list of that element type. Answers the operand's return type.
pub(crate) fn check_yield_from(
    env: &Environment,
    local_names: &[&str],
    operand: &HirExpr,
    statement: bool,
) -> Result<Ty, Diagnostic> {
    let sig = enclosing(env)?.clone();
    let actual = infer_expr_in(env, local_names, operand)?;
    match &actual {
        Ty::Generator(sub) => {
            if sub.0 != sig.0 {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "`yield from` a generator yielding `{}` in a generator yielding `{}`",
                        sub.0.name(),
                        sig.0.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            if !matches!(sub.1, Ty::None) && sub.1 != sig.1 {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "`yield from` a generator taking `{}` values in a generator taking \
                         `{}` values",
                        sub.1.name(),
                        sig.1.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(sub.2.clone())
        }
        Ty::List(element) if statement && **element == sig.0 => Ok(Ty::None),
        other => Err(Diagnostic::error(
            "C0001",
            format!(
                "`yield from` a `{}` is not supported yet -- only a generator yielding `{}` \
                 (or, as a statement, a `list[{}]`)",
                other.name(),
                sig.0.name(),
                sig.0.name()
            ),
            Span::new(0, 0),
        )),
    }
}

/// `gen.send(v)`, `gen.throw(e)` or `gen.close()` on a generator of type
/// `Generator[sig]`.
pub(crate) fn method_call(
    env: &Environment,
    local_names: &[&str],
    sig: &(Ty, Ty, Ty),
    method: &str,
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    let arity = |expected: usize| {
        if args.len() == expected {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "T0021",
                format!(
                    "generator `{method}()` takes {expected} argument(s), got {}",
                    args.len()
                ),
                Span::new(0, 0),
            ))
        }
    };
    match method {
        "send" => {
            arity(1)?;
            let actual = infer_expr_in(env, local_names, &args[0])?;
            // `send(None)` is checked like any other value: a `yield` in a
            // generator taking `int` values is read as an `int`, and CPython
            // hands it the `None` sent in. Priming a just-started generator
            // whose send type excludes `None` takes `next(gen)`.
            if crate::class::is_assignable_env(env, &actual, &sig.1) {
                return Ok(sig.0.clone());
            }
            Err(Diagnostic::error(
                "T0021",
                format!(
                    "`send()` to a generator taking `{}` values, got `{}`",
                    sig.1.name(),
                    actual.name()
                ),
                Span::new(0, 0),
            ))
        }
        "throw" => {
            arity(1)?;
            crate::exception::check_thrown_operand(env, local_names, &args[0])?;
            Ok(sig.0.clone())
        }
        "close" => {
            arity(0)?;
            Ok(Ty::None)
        }
        other => Err(Diagnostic::error(
            "C0001",
            format!("generator method `{other}()` is not supported yet"),
            Span::new(0, 0),
        )),
    }
}

/// Records the type of `e.value` for the handler binding `name` of the
/// exception class `class` caught around `body` (checked in `body_env`):
/// for `StopIteration`, the return type of the generators `body` resumes
/// (`pycc_hir::stop_iteration_value_ty`), kept as an error message when it
/// cannot be told, reported only if `e.value` is read.
pub(crate) fn bind_stop_value(
    handler_env: &mut Environment,
    body_env: &Environment,
    local_names: &[&str],
    name: &str,
    class: &str,
    body: &[HirStmt],
) {
    handler_env.stop_values.remove(name);
    handler_env.stop_value_errors.remove(name);
    if class != "StopIteration" {
        return;
    }
    let tys: Vec<Ty> = pycc_hir::stop_iteration_sources(body)
        .into_iter()
        .filter_map(|source| infer_expr_in(body_env, local_names, source).ok())
        .collect();
    match pycc_hir::stop_iteration_value_ty(&tys) {
        Ok(ty) => {
            handler_env.stop_values.insert(name.to_string(), ty);
        }
        Err(message) => {
            handler_env
                .stop_value_errors
                .insert(name.to_string(), message);
        }
    }
}

/// `base.value` when `base` is a caught `StopIteration` binding: its type,
/// or the diagnostic for one whose generators cannot be told.
pub(crate) fn stop_value_ty(env: &Environment, base: &HirExpr) -> Option<Result<Ty, Diagnostic>> {
    let HirExpr::Name(name) = base else {
        return None;
    };
    if let Some(ty) = env.stop_values.get(name) {
        return Some(Ok(ty.clone()));
    }
    env.stop_value_errors
        .get(name)
        .map(|message| Err(Diagnostic::error("C0001", message.clone(), Span::new(0, 0))))
}
