//! The target-and-body half of a module-level `for` loop over a CPython
//! object, shared by `check_stmt`'s two loop arms: `HirStmt::ForObject`
//! (`for x in o.attr:` / `for x in o.method(...):`, PR 3c of #1082) and
//! `HirStmt::ForList` over a bare name bound to an object (`for t in x:`,
//! #1325).

use crate::{BindingState, Environment, join_loop_body, narrow};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirStmt, Ty};

/// Checks a module-level `for var in <object>:` loop once its iterable is
/// known to be [`Ty::Object`]: guards and binds the target, then checks
/// `body` and joins it back as a zero-or-more-times loop body.
pub(crate) fn check_module_object_loop(
    env: &mut Environment,
    var: &str,
    body: &[HirStmt],
) -> Result<(), Diagnostic> {
    // The loop variable holds each item as another opaque
    // `PyObject *`. It is bound directly rather than through
    // `check_assignment`: that function's K1 guard now refuses an
    // object binding only inside a function body (#1325), and the
    // target guards below are this construct's own -- they name the
    // `for` target in their help text, which `check_assignment`'s
    // generic reassignment diagnostics would not.
    // A loop target that already names a binding of some *other*
    // type is refused rather than overwritten. `env.bind` overwrites,
    // so without this guard `x = 5` followed by `for x in <object>:`
    // would leave `x` as `Ty::Object` for the rest of the module
    // while the reads above it stay `Ty::Int` -- and `pycc_codegen`
    // allocates exactly one storage slot per name per function
    // (`collect_stmt_bindings`), asserting at every scalar read that
    // the slot's type still equals the expression's
    // (`local type drifted`). One name with two types is therefore
    // unrepresentable downstream: whichever type won the slot, the
    // other access site would either trip that assertion or -- since
    // it is a `debug_assert`, compiled out in release -- silently
    // store a `PyObject *` into an `i64` slot. Refusing the shape in
    // the checker is the only resolution that leaves no program
    // compiling to wrong code. `T0023` is reused rather than a new
    // code minted: a `for` target *is* an assignment in Python, and
    // the message ("cannot assign ... previously inferred as ...")
    // describes this rebinding exactly. The mirror case -- the loop
    // first, then `x = 5` -- is a plain rebinding of an `object` name,
    // which since #1475 boxes the `5` into that same `PyObject *` slot
    // (`check_assignment_boxing`), so it has a representable lowering.
    // A *declared but never assigned* target is the other half of
    // the same rule, and `lookup_any` does not see it: `x: int`
    // puts `x` in `declared`, not `bindings`. `check_assignment`
    // consults `declared_ty` for exactly this case and reports
    // `T0026`, and a `for` target is an assignment, so it reports
    // the same. The refusal is unconditional because no declared
    // type can accept an object item: `object` is not a writable
    // annotation (`pycc_hir` refuses it with `C0001`), so
    // `declared_ty` never yields `Ty::Object`. A value-less
    // `Final[int]` declaration lands here too rather than in
    // `T0045`, which only fires once the name has a runtime value.
    if let Some(declared) = env.declared_ty(var) {
        return Err(Diagnostic::error(
                "T0026",
                format!(
                    "cannot assign `object` to `{var}`, previously declared as `{var}: {}`",
                    declared.name()
                ),
                Span::new(0, 0),
            )
            .with_help(format!(
                "use a different name for the `for` target: `{var}` is declared as `{}`, and a name has one type for its whole scope",
                declared.name()
            )));
    }
    if let Some(previous) = env.lookup_any(var)
        && !matches!(previous, Ty::Object)
    {
        return Err(Diagnostic::error(
                    "T0023",
                    format!(
                        "cannot assign `object` to `{var}`, previously inferred as `{}`",
                        previous.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!(
                    "use a different name for the `for` target: `{var}` is already bound as `{}`, and a name has one type for its whole scope",
                    previous.name()
                )));
    }
    let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
    env.bind(var.to_string(), Ty::Object);
    let mut body_env = env.clone();
    narrow::apply_kill_prescan(&mut body_env, body);
    narrow::apply_delete_prescan(&mut body_env, body, Some(var));
    narrow::check_stmt_sequence(&mut body_env, body)?;
    join_loop_body(env, &body_env);
    // The loop may execute zero times, so a newly introduced loop
    // variable is only maybe-bound afterwards -- exactly as in
    // `check_stmt`'s `ForList` arm, and load-bearing here because reading a
    // `Ty::Object` name in a module body is itself admitted.
    if !was_definite {
        env.bind_maybe(var.to_string(), Ty::Object);
    }
    Ok(())
}
