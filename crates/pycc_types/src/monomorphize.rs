//! Generic monomorphization (D-133/D-134): call-site instantiation of
//! generic functions, generic classes, and protocol-parameterized
//! functions into ordinary concrete-`Ty` HIR.
//!
//! This submodule holds the whole monomorphization seam, extracted from
//! [`lib.rs`](crate) per the repository's source-file decomposition rule
//! (AGENTS.md "Keep source files decomposable", [D-185]), following the
//! precedent set by [`solver`](crate::solver), [`binop`](crate::binop),
//! and [`class`](crate::class).
//!
//! The seam is cohesive because every item in it exists to serve one
//! pipeline, entered exactly twice from the crate root:
//! [`instantiate_generic_call`] (one call site, invoked from the type
//! checker as it walks a generic call) and [`monomorphize`] (the
//! whole-module pass that rewrites every remaining generic call, generic
//! class instantiation, and protocol-parameterized function into concrete
//! items). Everything else here is one of that pipeline's three private
//! stages -- `substitute_*` (type/statement substitution), `collect_*`
//! (finding the instantiations a module needs), and `rewrite_*` (pointing
//! call sites at the mangled specializations) -- plus the mangling helpers
//! that give a specialization its deterministic name.
//!
//! Deliberately left in `lib.rs` when this module was extracted, and later
//! moved to its own sibling module [`enum_lower`](crate::enum_lower) (#544
//! Part 6): `enum_member_attr_type` and the `unroll_enum_loops` family
//! (#379 / PR-19). They run *after* `monomorphize` and mention it in
//! their own doc comments, but they are the enum-lowering pass, a
//! separate seam with its own boundary.
//!
//! [D-185]: https://github.com/rotnov/pycc/blob/main/docs/decisions/D-185-permit-a-dedicated-tracking-issue-per-oversized.md

use std::collections::{HashMap, HashSet};

use crate::{
    BindingState, Environment, bind_local_types_in_body, bind_local_types_in_stmt,
    function_local_names, generic_type_param_name, infer_expr_in, is_assignable,
    is_generic_signature, is_local, is_unshadowed_builtin_exception, module_function_local_names,
    t0042,
};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    CompElt, CompIter, FStringPart, ForeignImportSite, HirClassDef, HirComprehension, HirExpr,
    HirItem, HirModule, HirStmt, ImportBinding, PropertyDef, Ty,
};

mod comprehension;
mod method_defaults;
mod protocol_method;
pub(crate) use protocol_method::specialize_protocol_method_call;

/// One successful D-134 call-site monomorphization: the concrete return
/// type the call site's own type-checking needs immediately, plus the
/// substituted, mangled, ordinary concrete-`Ty` function body that
/// `pycc_mir` (Task 3) registers exactly like a non-generic function.
///
/// `mangled_name` is keyed only by `(generic function name, type-parameter
/// name, concrete Ty)`, never by anything call-site-specific (argument
/// expressions, source span, call order) -- two call sites that resolve `T`
/// to the same concrete `Ty` therefore produce byte-for-byte the same
/// `mangled_name` and the same `specialized` body content, which is exactly
/// the key Task 3 needs to deduplicate multiple call sites into one
/// compiled specialization (D-134's own "two call sites, same concrete
/// type, one compiled specialization" requirement).
#[derive(Debug, Clone, PartialEq)]
pub struct GenericInstantiation {
    pub mangled_name: String,
    pub specialized: HirItem,
    pub return_ty: Ty,
}

/// Mangled-name scheme for one generic-function call-site instantiation
/// (D-133/D-134). `crates/pycc_mir/src` has no existing per-instantiation
/// function-name-mangling convention to reuse: D-105's own container
/// monomorphization (`list[int]`, `dict[str, int]`, `set[int]`, ...)
/// dispatches on `Ty` at codegen time through a fixed set of `pycc_rt`
/// runtime helper functions (see e.g. `crates/pycc_rt/src/lib.rs`), not by
/// generating a differently-named MIR/codegen function per instantiation --
/// there is no prior "list[int]'s specialized entry point" name to mirror,
/// only a set of runtime helper names that don't vary per call site at all
/// (plan-deviation note: the task brief asked to reuse an existing
/// convention; none exists, confirmed by inspection before writing this).
///
/// The scheme instead mirrors this same file's (`pycc_hir`)
/// `synthesize_comp_var_name` precedent for a synthesized identifier that
/// must never collide with a real user-source name: a real Python `NAME`
/// token can never start with a decimal digit (confirmed against the
/// vendored `ruff_python_parser` tokenizer, same fact `synthesize_comp_var_name`'s
/// own doc comment cites), so prefixing with `0gen_` guarantees this string
/// can never equal any function name lowering could have produced from real
/// source, no matter what the user names their own top-level functions --
/// unlike a merely unusual but still syntactically legal name (e.g. the
/// task brief's own illustrative `f__T_int`, which a user's own source
/// *could* spell literally, `__` is ordinary Python identifier syntax).
fn mangle_generic_instantiation(fn_name: &str, type_param_name: &str, concrete: &Ty) -> String {
    format!("0gen_{fn_name}__{type_param_name}_{}", concrete.name())
}

/// D-133: a `Ty`-tree rewrite substituting a top-level `Ty::Param(param_name)`
/// occurrence with `concrete`, leaving every other `Ty` shape untouched. Not
/// a *recursive* container walk: `generic_type_param_name` (called by every
/// public entry point before this function ever runs, both here and in
/// `check_generic_function`) already rejects any container-position
/// `Ty::Param` with `T0042`, and since D-228 (issue #918) container
/// *annotation* lowering rejects one earlier still, in `pycc_hir`, with the
/// same code -- deliberately, precisely because this function is not
/// recursive. So by the time `substitute_ty` runs, `ty` is
/// provably either a bare `Ty::Param(param_name)` or a shape that can never
/// contain one nested inside `List`/`Set`/`Dict`/`Tuple` -- recursing into
/// those container variants here would be untestable dead code (D-014's
/// hard coverage gate has no exemption for it), not genuine generality.
fn substitute_ty(ty: &Ty, param_name: &str, concrete: &Ty) -> Ty {
    match ty {
        Ty::Param(name) if name.as_ref() == param_name => concrete.clone(),
        other => other.clone(),
    }
}

/// #380 (PR-20): Like `substitute_ty` but substitutes
/// `Ty::Protocol(protocol_name)` with `concrete` (a `Ty::Instance`).
/// Used to monomorphize functions with protocol-typed parameters — at
/// each call site, the protocol type is replaced with the concrete
/// argument's class type, so MIR/codegen can resolve method calls and
/// attribute access against the concrete class.
fn substitute_ty_protocol(ty: &Ty, protocol_name: &str, concrete: &Ty) -> Ty {
    match ty {
        Ty::Protocol(name) if name.as_ref() == protocol_name => concrete.clone(),
        other => other.clone(),
    }
}

/// #380 (PR-20): Returns `true` if a function signature has any
/// `Ty::Protocol` parameters, indicating it needs monomorphization at
/// each call site. Only parameters are consulted: a protocol *return*
/// annotation never reaches this crate any more -- `pycc_hir` rejects
/// `-> P` with `C0001` (#934), because a call to such a function has no
/// concrete type to specialize on. The return type is therefore not an
/// input here (it used to be threaded through as an unused parameter).
fn has_protocol_param(params: &[(String, Ty)]) -> bool {
    params.iter().any(|(_, ty)| matches!(ty, Ty::Protocol(_)))
}

/// #380 (PR-20): Mangles a function name for protocol monomorphization
/// with one or more protocol→concrete substitutions, following the
/// existing `0gen_` convention. Each substitution appends
/// `__{protocol_name}_{concrete_name}` to the mangled name.
pub(crate) fn mangle_protocol_instantiation(
    func_name: &str,
    substitutions: &[(String, Ty)],
) -> String {
    let mut name = format!("0gen_{func_name}");
    for (proto_name, concrete) in substitutions {
        if let Ty::Instance(concrete_name) = concrete {
            name.push_str(&format!("__{proto_name}_{concrete_name}"));
        }
    }
    name
}

/// #380 (PR-20): Substitutes multiple `Ty::Protocol` types in a single
/// `Ty` value, applying each substitution in order.
fn substitute_ty_protocols(ty: &Ty, substitutions: &[(String, Ty)]) -> Ty {
    let mut result = ty.clone();
    for (proto_name, concrete) in substitutions {
        result = substitute_ty_protocol(&result, proto_name, concrete);
    }
    result
}

/// #380 (PR-20): Like `substitute_body_protocol` but applies multiple
/// protocol→concrete substitutions.
fn substitute_body_protocols(body: &[HirStmt], substitutions: &[(String, Ty)]) -> Vec<HirStmt> {
    body.iter()
        .map(|stmt| substitute_stmt_protocols(stmt, substitutions))
        .collect()
}

fn substitute_stmt_protocols(stmt: &HirStmt, substitutions: &[(String, Ty)]) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty_protocols(annotation, substitutions),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body_protocols(body, substitutions),
            orelse: substitute_body_protocols(orelse, substitutions),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        other => other.clone(),
    }
}

/// PEP 695 (#387): Like `substitute_ty` but also substitutes
/// `Ty::Instance(class_name)` with `Ty::Instance(mangled_class)` — needed
/// when monomorphizing a generic class's methods, where `self`'s own
/// parameter type (`Ty::Instance("Container")`) must become
/// `Ty::Instance("0gen_Container__T_int")` so MIR/codegen resolve attribute
/// slots against the monomorphized class's substituted attribute types, not
/// the original generic class's `Ty::Param`-typed slots.
pub(crate) fn substitute_ty_with_class(
    ty: &Ty,
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> Ty {
    match ty {
        Ty::Param(name) if name.as_ref() == param_name => concrete.clone(),
        Ty::Instance(name) if name.as_ref() == class_name => {
            Ty::Instance(Box::new(mangled_class.to_string()))
        }
        other => other.clone(),
    }
}

/// D-133: clones a generic function's body, substituting only the `Ty`
/// annotations `substitute_ty` above would rewrite -- `HirStmt::AnnAssign`'s
/// `annotation` field is the only place an embedded `Ty` appears inside a
/// `HirStmt`/`HirExpr` tree (every `HirExpr` variant is untyped; type is
/// always inferred contextually), so every other statement shape is cloned
/// unchanged, recursing only into nested bodies (`If`/`While`/`ForRange`/
/// `ForList`) to reach a nested `AnnAssign`.
fn substitute_body(body: &[HirStmt], param_name: &str, concrete: &Ty) -> Vec<HirStmt> {
    body.iter()
        .map(|stmt| substitute_stmt(stmt, param_name, concrete))
        .collect()
}

fn substitute_stmt(stmt: &HirStmt, param_name: &str, concrete: &Ty) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty(annotation, param_name, concrete),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body(body, param_name, concrete),
            orelse: substitute_body(orelse, param_name, concrete),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        other => other.clone(),
    }
}

/// PEP 695 (#387): Like `substitute_body` but uses `substitute_ty_with_class`
/// to also rewrite `Ty::Instance(class_name)` to `Ty::Instance(mangled_class)`.
fn substitute_body_with_class(
    body: &[HirStmt],
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> Vec<HirStmt> {
    body.iter()
        .map(|stmt| {
            substitute_stmt_with_class(stmt, param_name, concrete, class_name, mangled_class)
        })
        .collect()
}

pub(crate) fn substitute_stmt_with_class(
    stmt: &HirStmt,
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty_with_class(
                annotation,
                param_name,
                concrete,
                class_name,
                mangled_class,
            ),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
            orelse: substitute_body_with_class(
                orelse,
                param_name,
                concrete,
                class_name,
                mangled_class,
            ),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        other => other.clone(),
    }
}

/// D-133/D-134: resolves one call site's argument types against a generic
/// function's signature, substituting the single type parameter with the
/// one concrete `Ty` every occurrence agrees on, and produces the
/// specialized, mangled, ordinary concrete-`Ty` function body `pycc_mir`
/// (Task 3) registers as an independent function. Rejects with `T0042`
/// when: the function has no type parameter to instantiate; the call's own
/// arity doesn't match (reuses `T0021`'s existing call-arity message shape,
/// not a new code, mirroring `infer_expr_in`'s own `HirExpr::Call` arm);
/// two occurrences of the type parameter resolve to different concrete
/// `Ty`s; the type parameter is never used by any parameter (nothing to
/// resolve it from); or the resolved `Ty` is not one of
/// `Int`/`Float`/`Bool`/`Str` (D-134's own scalar-only call-site scope).
/// Every non-generic parameter position is still checked for ordinary
/// assignability via the existing `is_assignable`, reusing `T0021`'s
/// existing "argument N expects ..." message shape exactly (not a new
/// diagnostic for an unrelated failure mode).
pub fn instantiate_generic_call(
    func: &HirItem,
    arg_tys: &[Ty],
) -> Result<GenericInstantiation, Diagnostic> {
    let HirItem::Function {
        name,
        params,
        return_ty,
        body,
    } = func
    else {
        panic!("instantiate_generic_call called with a non-Function HirItem");
    };
    let type_param_name = generic_type_param_name(params, return_ty)?.ok_or_else(|| {
        t0042(format!(
            "`{name}` has no PEP 695 type parameter to instantiate"
        ))
    })?;
    if arg_tys.len() != params.len() {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{name}` expects {} argument(s), got {}",
                params.len(),
                arg_tys.len()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("pass exactly {} argument(s)", params.len())));
    }
    let mut resolved: Option<Ty> = None;
    for ((param_name, param_ty), arg_ty) in params.iter().zip(arg_tys) {
        match param_ty {
            Ty::Param(name_ref) if name_ref.as_str() == type_param_name => match &resolved {
                Some(existing) if existing != arg_ty => {
                    return Err(t0042(format!(
                        "generic function `{name}`'s type parameter `{type_param_name}` was resolved inconsistently across its own call-site arguments (`{}` vs `{}`)",
                        existing.name(),
                        arg_ty.name()
                    )));
                }
                _ => resolved = Some(arg_ty.clone()),
            },
            other => {
                if !is_assignable(arg_ty.clone(), other.clone()) {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "argument `{param_name}` of `{name}` expects `{}`, got `{}`",
                            other.name(),
                            arg_ty.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!("pass a `{}` value", other.name())));
                }
            }
        }
    }
    let concrete = resolved.ok_or_else(|| {
        t0042(format!(
            "generic function `{name}`'s type parameter `{type_param_name}` is not used by any parameter, so no call-site argument can resolve it"
        ))
    })?;
    if !matches!(concrete, Ty::Int | Ty::Float | Ty::Bool | Ty::Str) {
        return Err(t0042(format!(
            "generic function `{name}` was called with `{}` for type parameter `{type_param_name}`, but v0.2 only instantiates a type parameter with `int`, `float`, `bool`, or `str`",
            concrete.name()
        )));
    }
    let substituted_params = params
        .iter()
        .map(|(param_name, ty)| {
            (
                param_name.clone(),
                substitute_ty(ty, &type_param_name, &concrete),
            )
        })
        .collect();
    let substituted_return = substitute_ty(return_ty, &type_param_name, &concrete);
    let substituted_body = substitute_body(body, &type_param_name, &concrete);
    let mangled_name = mangle_generic_instantiation(name, &type_param_name, &concrete);
    Ok(GenericInstantiation {
        mangled_name: mangled_name.clone(),
        return_ty: substituted_return.clone(),
        specialized: HirItem::Function {
            name: mangled_name,
            params: substituted_params,
            return_ty: substituted_return,
            body: substituted_body,
        },
    })
}

/// PR-13 Task 3 (D-133/D-134): rewrites every call site whose callee is a
/// PEP 695 generic function into a call to that call site's mangled,
/// monomorphized specialization, mutating `expr` and any nested
/// sub-expression in place. Returns `expr`'s resulting `Ty` exactly like
/// `infer_expr_in` would (the two agree on every non-generic-call
/// expression, since this function delegates directly to `infer_expr_in`
/// once every nested `HirExpr::Call` reachable from `expr` has already been
/// rewritten to name only ordinary, concrete functions).
///
/// This does not repeat `infer_expr_in`'s own validation logic -- `env` is
/// only ever used here to look up a call's argument types via the ordinary,
/// unmodified `infer_expr_in`/`instantiate_generic_call`, both of which
/// this module's callers have already run successfully once (D-133/D-134's
/// call-arm dispatch in `infer_expr_in` itself) before this rewrite ever
/// runs. Every `HirExpr` variant with a sub-expression is visited so a
/// generic call nested at any depth (e.g. `print(identity(1))`) is found
/// and rewritten; a leaf variant has nothing to recurse into and falls
/// straight through to `infer_expr_in`.
/// Whether `expr` names a class rather than a value binding.
///
/// A bare class name is not something `infer_expr_in` can resolve on its own,
/// so every arm of [`rewrite_generic_calls_in_expr`] that would otherwise
/// recurse into such a base has to skip it: `C[x]` (PEP 560
/// `__class_getitem__`), `C.attr` (a class-attribute read), and `C.m()` (a
/// `@staticmethod` or `@classmethod` call through the class) all put the class
/// name in a position where recursing fails with T0021 ("name not defined"),
/// exactly the failure mode this module's `isinstance` class-argument skip
/// already avoids. Skipping the base loses nothing: `infer_expr_in` still runs
/// on the whole expression afterwards, and that is what resolves the class read
/// and reports any real error.
///
/// An active value binding or local of that name shadows the class -- in
/// `def f(B: D) -> int: return B.X`, `B` is the parameter -- so those two
/// checks come first, matching the order `pycc_types`' own class-name
/// `AttrGet` and `MethodCall` arms use.
fn is_class_name_base(env: &Environment, local_names: &[&str], expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::Name(name)
        if env.binding_state(name).is_none()
            && !is_local(local_names, name)
            && env.lookup_class(name).is_some())
}

/// Whether `expr` is a bare name bound to a `memoryview` (Part 2 of #1027).
///
/// The `Subscript` arm's second do-not-recurse guard, deliberately *not*
/// folded into [`is_class_name_base`]: that predicate's other two callers
/// are the `AttrGet` and `MethodCall` arms, where `b.attr` and `b.tolist()`
/// must keep reaching the `C0001` read refusal. Only the subscript position
/// has a Part 2 lowering, so only the subscript position skips the base.
///
/// No `local_names` check, unlike `is_class_name_base`: there the binding
/// state is the *shadow* being looked for, here it is the whole subject.
/// `Definitely` matches `crate::expr`'s own interception -- a
/// possibly-unbound name is left to the ordinary path and its own
/// unbound-local diagnostic.
fn is_memoryview_base(env: &Environment, expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::Name(name)
    if matches!(
        env.binding_state(name),
        Some(BindingState::Definitely(Ty::MemoryView))
    ))
}

pub(crate) fn rewrite_generic_calls_in_expr(
    env: &mut Environment,
    local_names: &[&str],
    expr: &mut HirExpr,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<Ty, Diagnostic> {
    match expr {
        HirExpr::Call { callee, args } => {
            // #380 (#435): `isinstance`/`issubclass` are compile-time
            // builtins whose class-name arguments (args[1] for isinstance,
            // both args for issubclass) are not value expressions — they
            // are bare class names or tuples of class names. Rewriting them
            // with `rewrite_generic_calls_in_expr` would call
            // `infer_expr_in` on a bare class name, which looks it up as a
            // value binding and fails with T0021 ("name not defined").
            // Skip the class-name arguments entirely, rewriting only the
            // object argument (isinstance's args[0]). A user-defined
            // function named `isinstance` takes priority over the builtin
            // (same pattern as `infer_expr_in`'s own guard).
            //
            // Only `isinstance` needs this special-case here:
            // `issubclass`'s arguments are always class names (never
            // generic call expressions), so `rewrite_generic_calls_in_expr`
            // never encounters a builtin `issubclass` call that needs arg
            // rewriting. Checking only `isinstance` avoids permanently-
            // uncovered `issubclass` branches under D-014's 100 %
            // coverage gate.
            if callee == "isinstance" && env.lookup_function(callee).is_none() {
                // `isinstance` always has ≥ 2 args (the type checker
                // validates argument count before this code runs), so
                // `args[0]` is always safe. The guard was removed to
                // avoid a permanently-uncovered `false` branch under
                // D-014's 100 %-coverage gate.
                //
                // Discard the returned `Ty` — only the rewriting side
                // effect matters here; the subsequent `infer_expr_in`
                // on the whole expression re-derives the type.  Using
                // `let _ =` instead of `?` avoids a permanently-
                // uncovered error-path region under D-014's 100 %
                // coverage gate: `rewrite_generic_calls_in_expr` can
                // only fail when `infer_expr_in` on a sub-expression
                // fails, and this same `infer_expr_in` call on the
                // whole expression immediately below would surface
                // the identical error.
                let _ = rewrite_generic_calls_in_expr(
                    env,
                    local_names,
                    &mut args[0],
                    instantiations,
                    seen,
                );
                return infer_expr_in(env, local_names, expr);
            }
            // #1116: `len(b)` on a `memoryview`-bound name skips the
            // argument recursion below entirely, for the reason the
            // `Subscript` arm's own `is_memoryview_base` guard records --
            // that recursion tails into `infer_expr_in` on the bare name,
            // which reaches `reject_memoryview_read` and reports `C0001`
            // for an expression `crate::expr`'s `len` interception admits.
            // A bare `memoryview` name is never a generic-call argument
            // (there is nothing to instantiate), so nothing is lost by not
            // descending. The `infer_expr_in` on the whole expression below
            // then resolves the call through that interception, exactly as
            // the `isinstance` arm above does.
            if callee == "len" && args.len() == 1 && is_memoryview_base(env, &args[0]) {
                return infer_expr_in(env, local_names, expr);
            }
            // Each arg's `Ty` comes directly from this same rewriting
            // recursion's own return value -- not a second, separate
            // `infer_expr_in` pass over the now-rewritten args -- since
            // both would always agree (this function delegates to
            // `infer_expr_in` for exactly the types it doesn't special-case
            // itself) and only one needs to actually run.
            let arg_tys = args
                .iter_mut()
                .map(|arg| {
                    rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(generic_func) = env.lookup_generic(callee).cloned() {
                let instantiation = instantiate_generic_call(&generic_func, &arg_tys)?;
                // Registered into `env` regardless of whether this is the
                // first call site to discover this exact specialization --
                // this scope's own `env` may be an independent clone (e.g.
                // a fresh `child_for_function` per function) that never
                // observed a sibling scope's earlier registration, and a
                // *later* expression in this same scope may itself call the
                // now-rewritten mangled name (directly, or as a nested
                // generic-call argument), which needs it bound to resolve
                // as an ordinary function via `infer_expr_in`.
                if env.lookup_function(&instantiation.mangled_name).is_none() {
                    env.bind_function(
                        instantiation.mangled_name.clone(),
                        arg_tys,
                        instantiation.return_ty.clone(),
                    );
                }
                if seen.insert(instantiation.mangled_name.clone()) {
                    instantiations.push(instantiation.clone());
                }
                *callee = instantiation.mangled_name;
                return Ok(instantiation.return_ty);
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            // `let _ =` rather than `?`, for the same reason the
            // `isinstance` arm above uses it: only the rewriting side
            // effect matters here, and the `infer_expr_in` call on the
            // whole unary expression immediately below recurses into this
            // same operand, so it surfaces the identical error. Propagating
            // here as well would leave a permanently-uncovered error-path
            // region under D-014's 100 %-coverage gate.
            let _ = rewrite_generic_calls_in_expr(env, local_names, operand, instantiations, seen);
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::CompareChain { first, links } => {
            // `let _ =` rather than `?`, as in the `UnaryOp` arm above: the
            // `infer_expr_in` on the whole chain below recurses into every
            // operand and surfaces the identical error.
            let _ = rewrite_generic_calls_in_expr(env, local_names, first, instantiations, seen);
            for link in links.iter_mut() {
                let _ = rewrite_generic_calls_in_expr(
                    env,
                    local_names,
                    &mut link.right,
                    instantiations,
                    seen,
                );
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            for sub in [left.as_mut(), right.as_mut()] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for sub in [test.as_mut(), body.as_mut(), orelse.as_mut()] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::FString(parts) => {
            for part in parts.iter_mut() {
                if let FStringPart::Interpolation(inner) = part {
                    rewrite_generic_calls_in_expr(env, local_names, inner, instantiations, seen)?;
                }
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ListLiteral(elements)
        | HirExpr::ObjectList(elements)
        | HirExpr::SetLiteral(elements)
        | HirExpr::TupleLiteral(elements) => {
            for element in elements.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, element, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs.iter_mut() {
                for sub in [key, value] {
                    rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
                }
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::Subscript { base, index } => {
            // PEP 560 (#610): when the base is a bare class name, `C[x]` is
            // `C.__class_getitem__(x)` and the base is not a value
            // expression -- recursing into it would call `infer_expr_in` on
            // a bare class name and fail with T0021 ("name not defined"),
            // exactly the failure mode this function's own `isinstance`
            // class-argument skip above exists to avoid. Rewrite only the
            // index in that case; `infer_expr_in` on the whole expression
            // below still resolves the hook and reports any error.
            //
            // Part 2 of #1027 adds a second base this arm must not recurse
            // into, for the same reason and with the same remedy: a
            // `memoryview`-bound name. `rewrite_generic_calls_in_expr`'s own
            // tail calls `infer_expr_in` on each sub-expression, and on a
            // bare `memoryview` name that is the `C0001` read refusal --
            // reported here, in a module that happens to contain a generic
            // function, for a `b[i]` the check phase admits. The guard is a
            // separate predicate rather than a widening of
            // `is_class_name_base`, whose two other callers (the `AttrGet`
            // and `MethodCall` arms below) must keep refusing `b.attr` and
            // `b.tolist()` exactly as they do today.
            if !is_class_name_base(env, local_names, base.as_ref())
                && !is_memoryview_base(env, base.as_ref())
            {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            rewrite_generic_calls_in_expr(env, local_names, index, instantiations, seen)?;
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            for sub in std::iter::once(base.as_mut()).chain(
                [start, stop, step]
                    .into_iter()
                    .flatten()
                    .map(|b| b.as_mut()),
            ) {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // #1263: an attribute receiver is an ordinary sub-expression,
        // rewritten before the node's own arguments (CPython's order).
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr_mut().into_iter().chain([value.as_mut()]) {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::SetAdd { value, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ListPop { list } => {
            if let Some(sub) = list.attr_expr_mut() {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr_mut()
                .into_iter()
                .chain([key.as_mut(), default.as_mut()])
            {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::AttrGet { base, .. } => {
            // `C.attr` on a bare class name reads a class attribute, so the
            // base is not a value expression -- the same skip the `Subscript`
            // arm above applies, for the same reason.
            if !is_class_name_base(env, local_names, base.as_ref()) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::MethodCall { base, args, .. } => {
            // `C.m()` on a bare class name calls a `@staticmethod` or
            // `@classmethod` through the class, so the base is not a value
            // expression either; the arguments still are.
            if !is_class_name_base(env, local_names, base.as_ref()) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Part 2a of #1371: the callee is an ordinary value expression (a
        // subscript), so it is rewritten like any argument.
        HirExpr::ExprCall { callee, args } => {
            rewrite_generic_calls_in_expr(env, local_names, callee, instantiations, seen)?;
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Part 8 of #1371: rewrite every operand of the positional half
        // and every keyword value, but infer the *whole* node -- inferring
        // the positional half alone would check a pycc callee's arity
        // before the node's own keyword refusal.
        HirExpr::KeywordCall { call, keywords, .. } => {
            let (callee, args) = match call.as_mut() {
                HirExpr::Call { args, .. } => (None, args),
                HirExpr::MethodCall { base, args, .. } => {
                    let base = (!is_class_name_base(env, local_names, base.as_ref()))
                        .then_some(base.as_mut());
                    (base, args)
                }
                HirExpr::ExprCall { callee, args } => (Some(callee.as_mut()), args),
                other => unreachable!("a keyword call never wraps {other:?}"),
            };
            for part in callee
                .into_iter()
                .chain(args.iter_mut())
                .chain(keywords.iter_mut().map(|(_, value)| value))
            {
                rewrite_generic_calls_in_expr(env, local_names, part, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Issue #1188: rewrite inside the wrapped method call exactly as the
        // `MethodCall` arm does, but infer the *whole* node, so the result
        // follows the receiver's reading -- inferring `call` alone would
        // report a method error on a list receiver.
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            let (base, args) = call
                .method_call_parts_mut()
                .expect("a receiver-dispatched call always wraps a MethodCall");
            if !is_class_name_base(env, local_names, base) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // PEP 695 (#387): `C[type_arg](args)` — the monomorphized class
        // methods were pre-registered in `env` by `monomorphize`'s own
        // pre-scan (see `instantiate_generic_class_methods`), so this arm
        // only needs to recurse into `args` (rewriting any nested generic
        // function calls) and then rewrite the expression in place to an
        // ordinary `HirExpr::Call` to the mangled class name. The mangled
        // name follows the same `0gen_`-prefixed scheme as generic
        // functions (D-133/D-134).
        HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        } => {
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            let class_def = env.lookup_class(class).ok_or_else(|| {
                Diagnostic::error(
                    "T0001",
                    format!("class `{class}` is not defined"),
                    Span::new(0, 0),
                )
            })?;
            let type_param_name = class_def.type_param.as_deref().ok_or_else(|| {
                t0042(format!(
                    "class `{class}` is not generic (has no type parameter to instantiate)"
                ))
            })?;
            let mangled = mangle_generic_instantiation(class, type_param_name, type_arg);
            // Rewrite in place to an ordinary call.
            let args_vec = args.clone();
            *expr = HirExpr::Call {
                callee: mangled.clone(),
                args: args_vec,
            };
            infer_expr_in(env, local_names, expr)
        }
        // PEP 572 (#774): `target := value` — recurse into `value` first
        // (mirroring `AttrGet`'s own "recurse into the one sub-expression,
        // then delegate to `infer_expr_in` for the final type" shape), so a
        // generic call nested inside a walrus's value (`(x := identity(1))`)
        // is still found and rewritten. Unlike every other leaf-delegating
        // arm above, `name` must also be bound into `env` here: this pass
        // starts each statement's rewrite from a *fresh* `Environment`
        // (see `monomorphize`), not the one the module's ordinary
        // `check_and_resolve` pass populated, so a walrus target read later
        // in the same scope (as a plain name, or as another generic call's
        // argument) would otherwise resolve against a binding that was
        // never carried over and spuriously fail. `rewrite_generic_calls_in_stmt`
        // binds an `Assign`/`AnnAssign` target the same way for the same
        // reason; a walrus can appear inside any expression position, so
        // its own binding has to happen here rather than only at the
        // statement level.
        HirExpr::NamedExpr { name, value } => {
            let name = name.clone();
            rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            let ty = infer_expr_in(env, local_names, expr)?;
            env.bind(name, ty.clone());
            Ok(ty)
        }
        // #1254 (D-250): `env` has no scope stack, so the synthesized loop
        // variable is bound directly, exactly as the statement arms in
        // `rewrite_generic_calls_in_stmt` do. That is harmless: the
        // digit-led name is unique per source offset and no user code can
        // name it.
        HirExpr::Comprehension(comp) => {
            comprehension::rewrite_comprehension_expr(env, local_names, comp, instantiations, seen)
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => infer_expr_in(env, local_names, expr),
    }
}

/// Structural counterpart to `rewrite_generic_calls_in_expr`, one level up:
/// walks every statement position, rewriting any embedded `HirExpr` in
/// place and growing `env`'s bindings exactly enough (a plain `env.bind`,
/// not the validating `check_assignment`) for a later statement in the same
/// scope to resolve a local name as a generic call's argument. No shape or
/// assignability validation happens here -- the module already passed that
/// validation once, via the ordinary `check`/`check_and_resolve` path that
/// calls this function only afterward (see `monomorphize`).
fn rewrite_generic_calls_in_stmt(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &mut HirStmt,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::ExprStmt(expr) => {
            rewrite_generic_calls_in_expr(env, local_names, expr, instantiations, seen)?;
            Ok(())
        }
        HirStmt::Assign { target, value } => {
            let ty = rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            env.bind(target.clone(), ty);
            Ok(())
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final: _,
        } => {
            if let Some(value) = value {
                rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            }
            env.bind(target.clone(), annotation.clone());
            Ok(())
        }
        HirStmt::If { test, body, orelse } => {
            rewrite_generic_calls_in_expr(env, local_names, test, instantiations, seen)?;
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::While { test, body } => {
            rewrite_generic_calls_in_expr(env, local_names, test, instantiations, seen)?;
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            for sub in [start, stop, step] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            env.bind(var.clone(), Ty::Int);
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::ForList { var, list, body } => {
            // Issue #118 Part 1: use `lookup_any` (not `lookup`) since this
            // pass runs post-validation and needs the type regardless of
            // binding state -- the validation pass already rejected any
            // maybe-bound iterable read.
            let var_ty = match env.lookup_any(list) {
                Some(Ty::List(elem)) => *elem,
                Some(Ty::Dict(kv)) => kv.0,
                Some(Ty::Set(elem) | Ty::FrozenSet(elem)) => *elem,
                _ => {
                    // Already validated as iterable by the ordinary check
                    // pass that ran before `monomorphize`. A *missing*
                    // binding is reachable: this pass's environment lacks
                    // the foreign names, so a module-level bare-name loop
                    // over a CPython object (#1325) lands here and types its
                    // target `Ty::Infer` rather than `Ty::Object` -- the
                    // #1101 module-body gap `foreign/binding_tests.rs`'s
                    // `a_module_with_a_generic_function_refuses_foreign_reads_in_monomorphization`
                    // pins.
                    Ty::Infer
                }
            };
            env.bind(var.clone(), var_ty);
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        // PR 3c of #1082: the iterable is a real expression and may hold a
        // generic call, so it is walked. It is also the whole arm: the
        // check pass has already proved the iterable is `Ty::Object`, so
        // its root is a foreign name, and this pass seeds no foreign name
        // into its own environment -- a pre-existing gap that refuses
        // `len(numpy.pi)` in a module with a generic function exactly as it
        // refuses this loop. The walk therefore always returns `T0021` here
        // and the loop body is unreachable. It is kept as the walk rather
        // than a no-op precisely so that closing that gap cannot silently
        // let a generic call inside the iterable escape this pass.
        HirStmt::ForObject { iter, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, iter, instantiations, seen)?;
            Ok(())
        }
        HirStmt::DictSet { key, value, .. } => {
            for sub in [key, value] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::AttrSet { base, value, .. } => {
            for sub in [base, value] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            let body = cond.iter_mut().map(|c| c.as_mut()).chain([elt.as_mut()]);
            comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), Ty::List(Box::new(Ty::Int)));
            Ok(())
        }
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            let body = cond.iter_mut().map(|c| c.as_mut()).chain([elt.as_mut()]);
            let elt_ty = comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), comprehension::set_comp_container(elt_ty));
            Ok(())
        }
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => {
            let body = cond
                .iter_mut()
                .map(|c| c.as_mut())
                .chain([key.as_mut(), value.as_mut()]);
            comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
            Ok(())
        }
        HirStmt::Return(value) => {
            if let Some(value) = value {
                rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::Delete { .. } | HirStmt::ForeignImport { .. } => Ok(()),
        // Part 2c of #1371: a slice `del` reads its base and bounds, so a
        // generic call in any of them is rewritten like any other read.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            for bound in [start, stop, step].into_iter().flatten() {
                rewrite_generic_calls_in_expr(env, local_names, bound, instantiations, seen)?;
            }
            Ok(())
        }
        // #1457: an attribute `del` reads its base.
        HirStmt::DeleteAttr { base, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            Ok(())
        }
        HirStmt::Match { subject, cases } => {
            rewrite_generic_calls_in_expr(env, local_names, subject, instantiations, seen)?;
            for case in cases.iter_mut() {
                if let Some(guard) = case.guard.as_mut() {
                    rewrite_generic_calls_in_expr(env, local_names, guard, instantiations, seen)?;
                }
                for s in case.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
                }
            }
            Ok(())
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for handler in handlers.iter_mut() {
                let mut handler_env = env.clone();
                if let (Some(exc_types), Some(name)) = (&handler.exc_type, &handler.name) {
                    let binding_type = pycc_hir::except_handler_binding_type_name(exc_types);
                    handler_env.bind(name.clone(), Ty::Instance(Box::new(binding_type)));
                }
                for s in handler.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(
                        &mut handler_env,
                        local_names,
                        s,
                        instantiations,
                        seen,
                    )?;
                }
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in finalbody.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        // Part 3 of #382 (#542): `except*` always binds `as e` to
        // `ExceptionGroup`, never to the named handler type -- see
        // `check_try_star_stmt` in `pycc_types::exception` for the same rule
        // at type-checking time.
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for handler in handlers.iter_mut() {
                let mut handler_env = env.clone();
                if let Some(name) = &handler.name {
                    handler_env.bind(
                        name.clone(),
                        Ty::Instance(Box::new("ExceptionGroup".to_string())),
                    );
                }
                for s in handler.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(
                        &mut handler_env,
                        local_names,
                        s,
                        instantiations,
                        seen,
                    )?;
                }
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in finalbody.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::Raise { exc, cause } => {
            if let Some(exc) = exc.as_mut() {
                rewrite_generic_calls_in_raise_operand(
                    env,
                    local_names,
                    exc,
                    instantiations,
                    seen,
                )?;
            }
            if let Some(cause) = cause.as_mut() {
                rewrite_generic_calls_in_raise_operand(
                    env,
                    local_names,
                    cause,
                    instantiations,
                    seen,
                )?;
            }
            Ok(())
        }
    }
}

/// Rewrites generic calls nested in a `raise` operand without asking the
/// ordinary call checker to re-type-check the builtin exception constructor
/// itself. The validation pass has already checked that constructor; only its
/// message expression can still contain a generic call that needs rewriting.
fn rewrite_generic_calls_in_raise_operand(
    env: &mut Environment,
    local_names: &[&str],
    expr: &mut HirExpr,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    if let HirExpr::Call { callee, args } = expr
        && is_unshadowed_builtin_exception(env, local_names, callee)
    {
        for arg in args {
            rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
        }
        return Ok(());
    }
    rewrite_generic_calls_in_expr(env, local_names, expr, instantiations, seen)?;
    Ok(())
}

/// PR-13 Task 3 (D-133/D-134): the monomorphization pass that turns a
/// validated module possibly containing PEP 695 generic functions into an
/// equivalent module containing only ordinary, concrete-`Ty` functions --
/// exactly what `pycc_mir::build` already expects, with no MIR/codegen
/// change needed for genericity itself. Every call site whose callee was a
/// generic function is rewritten to call that call site's mangled,
/// monomorphized specialization instead; two call sites resolving the same
/// type parameter to the same concrete `Ty` share one specialization
/// (deduplicated by `GenericInstantiation::mangled_name`, per
/// `instantiate_generic_call`'s own documented dedup guarantee). Every
/// original generic function itself is dropped -- it still carries
/// `Ty::Param`, which must never reach `pycc_mir` -- and each distinct
/// specialization is appended as an ordinary `HirItem::Function`, in the
/// order its first call site was encountered (module item order, depth
/// first through each function body).
///
/// Callers must have already validated `hir` (e.g. via `check_all` or
/// `checked_function_signatures_all`, both of which now dispatch a generic call
/// site through `instantiate_generic_call` themselves via
/// `infer_expr_in`'s own generic arm) -- this pass performs no validation
/// of its own and instead trusts that every generic call site it encounters
/// resolves successfully.
/// PEP 695 (#387): Recursively collects every `(class_name, type_arg)` pair
/// from `GenericClassInstantiate` expressions reachable from `expr`. Used by
/// `instantiate_generic_class_methods`'s pre-scan to know which generic
/// classes need monomorphization before the rewrite pass starts. `Ty` does
/// not implement `Hash`, so dedup is done linearly against this `Vec`.
pub(crate) fn collect_generic_class_instantiations_from_expr(
    expr: &HirExpr,
    out: &mut Vec<(String, Ty)>,
) {
    match expr {
        HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        } => {
            let pair = (class.clone(), type_arg.clone());
            if !out.contains(&pair) {
                out.push(pair);
            }
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_generic_class_instantiations_from_expr(operand, out);
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_generic_class_instantiations_from_expr(operand, out);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_generic_class_instantiations_from_expr(left, out);
            collect_generic_class_instantiations_from_expr(right, out);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_generic_class_instantiations_from_expr(part, out);
            }
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_generic_class_instantiations_from_expr(inner, out);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_generic_class_instantiations_from_expr(e, out);
            }
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_generic_class_instantiations_from_expr(k, out);
                collect_generic_class_instantiations_from_expr(v, out);
            }
        }
        HirExpr::Subscript { base, index } => {
            collect_generic_class_instantiations_from_expr(base, out);
            collect_generic_class_instantiations_from_expr(index, out);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_generic_class_instantiations_from_expr(bound, out);
            }
        }
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr().into_iter().chain([value.as_ref()]) {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::SetAdd { value, .. } => {
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirExpr::ListPop { list } => {
            if let Some(sub) = list.attr_expr() {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr()
                .into_iter()
                .chain([key.as_ref(), default.as_ref()])
            {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::AttrGet { base, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
        }
        HirExpr::MethodCall { base, args, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            collect_generic_class_instantiations_from_expr(callee, out);
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_generic_class_instantiations_from_expr(call, out);
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_generic_class_instantiations_from_expr(call, out);
            for (_, value) in keywords {
                collect_generic_class_instantiations_from_expr(value, out);
            }
        }
        // PEP 572 (#774): `target := value` — recurse into `value` only,
        // mirroring `AttrGet`'s own single-sub-expression shape just above.
        HirExpr::NamedExpr { name: _, value } => {
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirExpr::Comprehension(comp) => {
            for sub in comp.sub_exprs() {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
    }
}

/// PEP 695 (#387): Traverses a `CompIter` for `GenericClassInstantiate`
/// expressions. `CompIter::Range` carries three `HirExpr`s (`start`, `stop`,
/// `step`) that can each contain a GCI (e.g. `range(C[int](0), 10)`), and
/// `CompIter::Iterable` (Part 1 of #1255) carries one. `CompIter::Name`
/// holds only a bare `String`, so it cannot contain one.
pub(crate) fn collect_generic_class_instantiations_from_comp_iter(
    iter: &CompIter,
    out: &mut Vec<(String, Ty)>,
) {
    match iter {
        CompIter::Range { start, stop, step } => {
            for sub in [start, stop, step] {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        CompIter::Iterable(iterable) => {
            collect_generic_class_instantiations_from_expr(iterable, out)
        }
        CompIter::Name(_) => {}
    }
}

/// PEP 695 (#387): Recursively collects `(class_name, type_arg)` pairs from
/// `GenericClassInstantiate` expressions reachable from a statement.
pub(crate) fn collect_generic_class_instantiations_from_stmt(
    stmt: &HirStmt,
    out: &mut Vec<(String, Ty)>,
) {
    match stmt {
        HirStmt::ExprStmt(expr) => collect_generic_class_instantiations_from_expr(expr, out),
        HirStmt::Assign { value, .. } => collect_generic_class_instantiations_from_expr(value, out),
        HirStmt::AnnAssign { value, .. } => {
            if let Some(v) = value {
                collect_generic_class_instantiations_from_expr(v, out);
            }
        }
        HirStmt::If { test, body, orelse } => {
            collect_generic_class_instantiations_from_expr(test, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for s in orelse {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::While { test, body } => {
            collect_generic_class_instantiations_from_expr(test, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForRange {
            start,
            stop,
            step,
            body,
            ..
        } => {
            for sub in [start, stop, step] {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForList { body, .. } => {
            // `list` is a bare `String` (variable name), not an `HirExpr`,
            // so it cannot contain a `GenericClassInstantiate`.
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForObject { iter, body, .. } => {
            // PR 3c of #1082: unlike `ForList`'s `list`, this iterable is an
            // `HirExpr` and can hold a `GenericClassInstantiate`.
            collect_generic_class_instantiations_from_expr(iter, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::DictSet { key, value, .. } => {
            collect_generic_class_instantiations_from_expr(key, out);
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirStmt::ListCompAssign {
            elt, cond, iter, ..
        }
        | HirStmt::SetCompAssign {
            elt, cond, iter, ..
        } => {
            collect_generic_class_instantiations_from_comp_iter(iter, out);
            collect_generic_class_instantiations_from_expr(elt, out);
            if let Some(c) = cond {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
        HirStmt::DictCompAssign {
            key,
            value,
            cond,
            iter,
            ..
        } => {
            collect_generic_class_instantiations_from_comp_iter(iter, out);
            collect_generic_class_instantiations_from_expr(key, out);
            collect_generic_class_instantiations_from_expr(value, out);
            if let Some(c) = cond {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
        HirStmt::Return(None) | HirStmt::Delete { .. } | HirStmt::ForeignImport { .. } => {}
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_generic_class_instantiations_from_expr(bound, out);
            }
        }
        HirStmt::DeleteAttr { base, .. } => {
            collect_generic_class_instantiations_from_expr(base, out)
        }
        HirStmt::Return(Some(expr)) => collect_generic_class_instantiations_from_expr(expr, out),
        HirStmt::AttrSet { base, value, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirStmt::Match { subject, cases } => {
            collect_generic_class_instantiations_from_expr(subject, out);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_generic_class_instantiations_from_expr(guard, out);
                }
                for s in &case.body {
                    collect_generic_class_instantiations_from_stmt(s, out);
                }
            }
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for handler in handlers {
                for s in &handler.body {
                    collect_generic_class_instantiations_from_stmt(s, out);
                }
            }
            for s in orelse {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for s in finalbody {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::Raise { exc, cause } => {
            if let Some(e) = exc {
                collect_generic_class_instantiations_from_expr(e, out);
            }
            if let Some(c) = cause {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
    }
}

/// PEP 695 (#387): Pre-scans `hir` for every `GenericClassInstantiate`
/// expression, collecting the unique `(class_name, type_arg)` pairs, then
/// monomorphizes each generic class's methods by substituting the class's
/// type parameter with the concrete `type_arg` — reusing PR-13's own
/// `substitute_ty`/`substitute_body`/`mangle_generic_instantiation`
/// infrastructure (D-133/D-134). Each monomorphized method is registered
/// in `env` (as an ordinary function signature) and collected in
/// `instantiations` (as a full `HirItem::Function` body for `pycc_mir` to
/// register). The monomorphized class itself is also registered in
/// `env.classes` under its mangled name, with mangled method names, so
/// `infer_expr_in`'s class-instantiation and method-call resolution work
/// transparently on the specialized class.
pub(crate) fn instantiate_generic_class_methods(
    hir: &HirModule,
    env: &mut Environment,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
    new_class_defs: &mut Vec<(String, HirClassDef)>,
) {
    // Collect all unique (class_name, type_arg) pairs from the entire module.
    let mut pairs: Vec<(String, Ty)> = Vec::new();
    for item in &hir.items {
        match item {
            HirItem::TopLevelStmt(stmt) => {
                collect_generic_class_instantiations_from_stmt(stmt, &mut pairs);
            }
            HirItem::Function { body, .. } => {
                for stmt in body {
                    collect_generic_class_instantiations_from_stmt(stmt, &mut pairs);
                }
            }
        }
    }

    for (class_name, type_arg) in &pairs {
        let Some(class_def) = hir.class_defs.iter().find(|(name, _)| name == class_name) else {
            continue;
        };
        let (_, class_def) = class_def;
        let Some(type_param_name) = &class_def.type_param else {
            continue;
        };
        let mangled_class = mangle_generic_instantiation(class_name, type_param_name, type_arg);

        // Monomorphize each method: find its HirItem::Function in hir.items
        // by its mangled name (e.g. "C.__init__"), substitute T with
        // type_arg, rename to the new mangled class name, and register.
        // #386 rebind semantics: class lowering retains multiple same-named
        // method items (the latest definition wins at runtime), and the
        // method table entry was replaced to point at the latest mangled
        // name on redefinition. Since the mangled name is identical across
        // redefinitions, `rfind` (reverse find) specializes the *last*
        // matching `HirItem::Function` -- the one whose body actually runs
        // -- rather than `find`'s first match, which would specialize a
        // stale, shadowed definition.
        let mut mangled_methods: Vec<(String, String)> = Vec::new();
        // Part 1 of #1191: (origin mangled, specialized mangled) for
        // re-keying `method_defaults` below.
        let mut method_renames: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.methods {
            // The `rfind` filter already guarantees the item is a
            // `HirItem::Function`, so the destructuring cannot fail — the
            // `else { continue }` handles only the `None` case (method not
            // found in `hir.items`).
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            // Register the method's signature in env so infer_expr_in can
            // resolve calls to it.
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            // substitute_body_with_class (called above) already rewrites
            // both `Ty::Param` → `type_arg` and `Ty::Instance(class_name)`
            // → `Ty::Instance(mangled_class)` in the method body's
            // annotations. Self/class-name resolution happened at
            // annotation_to_ty time (HIR lowering), where `Self` and the
            // class name both became `Ty::Instance(class_name)`, and
            // substitute_body_with_class rewrites those to the mangled
            // monomorphized class name. No further substitution is needed
            // here.
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            method_renames.push((original_mangled.clone(), new_mangled.clone()));
            mangled_methods.push((method_name.clone(), new_mangled));
        }

        // #377: Monomorphize properties. Each property's getter and setter
        // are ordinary `HirItem::Function` items with mangled names like
        // `Box.v` and `Box.v.setter`. They are NOT in the class's `methods`
        // table (they're in `properties`), so the method monomorphization
        // loop above does not handle them. We monomorphize them here by
        // finding their `HirItem::Function` in `hir.items`, substituting the
        // type parameter, and renaming to the mangled class name. The
        // monomorphized property entries point at the new mangled names.
        let mut monomorphized_properties: Vec<PropertyDef> = Vec::new();
        for prop in &class_def.properties {
            let new_getter = format!("{mangled_class}.{}", prop.name);
            if let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == &prop.getter
                )
            }) {
                let substituted_params = params
                    .iter()
                    .map(|(pn, ty)| {
                        (
                            pn.clone(),
                            substitute_ty_with_class(
                                ty,
                                type_param_name,
                                type_arg,
                                class_name,
                                &mangled_class,
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                let substituted_return = substitute_ty_with_class(
                    return_ty,
                    type_param_name,
                    type_arg,
                    class_name,
                    &mangled_class,
                );
                let substituted_body = substitute_body_with_class(
                    body,
                    type_param_name,
                    type_arg,
                    class_name,
                    &mangled_class,
                );
                let param_tys = substituted_params
                    .iter()
                    .map(|(_, ty)| ty.clone())
                    .collect::<Vec<_>>();
                env.bind_function(new_getter.clone(), param_tys, substituted_return.clone());
                let specialized = HirItem::Function {
                    name: new_getter.clone(),
                    params: substituted_params,
                    return_ty: substituted_return,
                    body: substituted_body,
                };
                if seen.insert(new_getter.clone()) {
                    instantiations.push(GenericInstantiation {
                        mangled_name: new_getter.clone(),
                        specialized,
                        return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                    });
                }
            }
            let new_setter = prop.setter.as_ref().map(|s| {
                let new_s = format!("{mangled_class}.{}.setter", prop.name);
                if let Some(HirItem::Function {
                    name: _,
                    params,
                    return_ty,
                    body,
                }) = hir.items.iter().rfind(|item| {
                    matches!(
                        item,
                        HirItem::Function { name, .. } if name == s
                    )
                }) {
                    let substituted_params = params
                        .iter()
                        .map(|(pn, ty)| {
                            (
                                pn.clone(),
                                substitute_ty_with_class(
                                    ty,
                                    type_param_name,
                                    type_arg,
                                    class_name,
                                    &mangled_class,
                                ),
                            )
                        })
                        .collect::<Vec<_>>();
                    let substituted_return = substitute_ty_with_class(
                        return_ty,
                        type_param_name,
                        type_arg,
                        class_name,
                        &mangled_class,
                    );
                    let substituted_body = substitute_body_with_class(
                        body,
                        type_param_name,
                        type_arg,
                        class_name,
                        &mangled_class,
                    );
                    let param_tys = substituted_params
                        .iter()
                        .map(|(_, ty)| ty.clone())
                        .collect::<Vec<_>>();
                    env.bind_function(new_s.clone(), param_tys, substituted_return.clone());
                    let specialized = HirItem::Function {
                        name: new_s.clone(),
                        params: substituted_params,
                        return_ty: substituted_return,
                        body: substituted_body,
                    };
                    if seen.insert(new_s.clone()) {
                        instantiations.push(GenericInstantiation {
                            mangled_name: new_s.clone(),
                            specialized,
                            return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                        });
                    }
                }
                new_s
            });
            monomorphized_properties.push(PropertyDef {
                name: prop.name.clone(),
                getter: new_getter,
                setter: new_setter,
            });
        }

        // #436: Monomorphize static methods. Each static method's
        // mangled name uses a `.static` suffix; the monomorphized name
        // replaces the class prefix with the mangled class name. Static
        // methods have no `self`/`cls` parameter, so only the type
        // parameter substitution applies.
        let mut mangled_static_methods: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.static_methods {
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}.static");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            mangled_static_methods.push((method_name.clone(), new_mangled));
        }

        // #436: Monomorphize class methods. Each class method's mangled
        // name uses a `.classmethod` suffix; the monomorphized name
        // replaces the class prefix with the mangled class name. Class
        // methods have a `cls` parameter typed `Ty::Instance(class_name)`,
        // which `substitute_body_with_class` rewrites to
        // `Ty::Instance(mangled_class)`.
        let mut mangled_class_methods: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.class_methods {
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}.classmethod");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            mangled_class_methods.push((method_name.clone(), new_mangled));
        }

        // Register the monomorphized class in env.classes with mangled
        // method names and substituted attribute types.
        let substituted_attrs = class_def
            .attrs
            .iter()
            .map(|(attr_name, ty)| {
                (
                    attr_name.clone(),
                    substitute_ty(ty, type_param_name, type_arg),
                )
            })
            .collect::<Vec<_>>();
        // #911: a class attribute is a compile-time constant with no
        // instance slot, so monomorphization carries it through unchanged
        // -- its type is already a non-`Param` scalar (a `Param(_)`
        // annotation is rejected at lowering), so there is nothing to
        // substitute. Omitting the field here would silently drop every
        // class attribute from a monomorphized generic class.
        // #977 (D-237): a `@dataclass` generic class keeps its dataclass
        // identity in every specialization. `mangled_methods` above already
        // carries the synthesized `__init__`/`__eq__`/`__repr__` under the
        // mangled class name, so `pycc_mir::class::rewrite_instance_to_repr`
        // can render the specialization exactly as it renders the origin --
        // but only if `is_dataclass` says so. Dropping the flag here made
        // `check` accept `print(Box[int](1))` (the origin `Box` is a
        // dataclass) while `build` re-inferred the rewritten call against
        // the specialization and rejected it with `C0001`. The field list
        // is substituted the same way `attrs` is, so a `T`-typed field
        // reports its concrete type on the specialization.
        let substituted_dataclass_fields = class_def
            .dataclass_fields
            .iter()
            .map(|(field_name, ty)| {
                (
                    field_name.clone(),
                    substitute_ty(ty, type_param_name, type_arg),
                )
            })
            .collect::<Vec<_>>();
        let new_class_def = HirClassDef {
            class_attrs: class_def.class_attrs.clone(),
            exception_type_tag: None,
            name: mangled_class.clone(),
            bases: Vec::new(),
            mro: vec![mangled_class.clone()],
            attrs: substituted_attrs,
            methods: mangled_methods,
            type_param: None, // The monomorphized class is not generic.
            properties: monomorphized_properties,
            static_methods: mangled_static_methods,
            class_methods: mangled_class_methods,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: method_defaults::rekey_method_defaults(
                &class_def.method_defaults,
                &method_renames,
            ),
            enum_members: Vec::new(),
            is_dataclass: class_def.is_dataclass,
            dataclass_fields: substituted_dataclass_fields,
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        };
        // `bind_class` clears any synthetic marking for the name it binds
        // (Part 1 of #541, D-188), which is harmless here: `mangled_class`
        // always carries `mangle_generic_instantiation`'s `0gen_` prefix, so
        // it can never equal one of the seven `BUILTIN_EXCEPTION_CLASSES`
        // names and can never unmark a synthetic entry `bind_classes` made.
        env.bind_class(mangled_class.clone(), new_class_def.clone());
        new_class_defs.push((mangled_class, new_class_def));
    }
}

/// PEP 695 (#387): Pass 2b per-item worker — rewrites any
/// `GenericClassInstantiate` expressions that survived inside one
/// monomorphized generic-class method body. Extracted from `monomorphize`'s
/// Pass 2b loop so that the defense-in-depth non-`Function` arm (all
/// instantiations are `HirItem::Function` in practice) is a `match` arm in
/// a standalone function that a direct unit test can cover, rather than an
/// `if let` whose never-taken false branch would carry an uncovered
/// continuation region under the 100%-coverage gate (D-014).
///
/// Clones the function's params/body out of `instantiations[i]` so the
/// immutable borrow ends before `rewrite_generic_calls_in_stmt` takes
/// `&mut instantiations` (it may push new instantiations for a GCI inside
/// the body that triggers another class monomorphization).
pub(crate) fn rewrite_generic_calls_in_instantiation(
    env: &mut Environment,
    i: usize,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    let (name, params, return_ty, body) = match &instantiations[i].specialized {
        HirItem::Function {
            name,
            params,
            return_ty,
            body,
        } => (
            name.clone(),
            params.clone(),
            return_ty.clone(),
            body.clone(),
        ),
        // All instantiations are `HirItem::Function` (created by
        // `instantiate_generic_call` or `instantiate_generic_class_methods`).
        // This arm is defense-in-depth, covered by a direct unit test.
        _ => return Ok(()),
    };
    let local_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
    let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
    let mut fn_env = env.child_for_function(&local_names_refs);
    for (param_name, param_ty) in &params {
        fn_env.bind(param_name.clone(), param_ty.clone());
    }
    let mut new_body = body;
    for stmt in new_body.iter_mut() {
        rewrite_generic_calls_in_stmt(&mut fn_env, &local_names_refs, stmt, instantiations, seen)?;
    }
    // Direct assignment avoids a second `if let` (whose never-taken false
    // branch would carry an uncovered region under D-014). All instantiations
    // are `HirItem::Function`, so this reconstruction is always valid.
    instantiations[i].specialized = HirItem::Function {
        name,
        params,
        return_ty,
        body: new_body,
    };
    Ok(())
}

/// #380 (PR-20): Monomorphizes functions with protocol-typed parameters.
/// Scans `items` for calls to functions that have `Ty::Protocol` parameters,
/// and for each call site, creates a specialized version with the protocol
/// type substituted by the concrete argument's class type. The original
/// function is dropped (only specializations reach MIR/codegen). Returns
/// the updated items list with monomorphized functions appended and call
/// sites rewritten.
///
/// The second return value is parallel to `items`: `kept[i]` says whether
/// input item `i` survived into the returned list. Dropping an item shifts
/// every later item's position, and the `ForeignImportSite::Item` index of each `ImportBinding::Foreign`
/// records a position in this very list, so the caller needs the mask to
/// recompute those positions (PR 1c of #1080 review finding 1).
fn monomorphize_protocol_params(
    items: Vec<HirItem>,
    env: &Environment,
    _new_class_defs: &mut Vec<(String, HirClassDef)>,
) -> (Vec<HirItem>, Vec<bool>) {
    // Collect functions with protocol-typed parameters (cloned, so we
    // can move `items` below without a borrow conflict).
    let protocol_funcs: HashMap<String, HirItem> = items
        .iter()
        .filter_map(|item| {
            if let HirItem::Function { name, params, .. } = item
                && has_protocol_param(params)
            {
                return Some((name.clone(), item.clone()));
            }
            None
        })
        .collect();
    if protocol_funcs.is_empty() {
        let kept = vec![true; items.len()];
        return (items, kept);
    }
    let mut kept = Vec::with_capacity(items.len());
    let mut new_items = Vec::new();
    let mut specializations: Vec<HirItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // #953: a module-level statement's own bindings have to accumulate as
    // the loop walks them, exactly as `bind_local_types_in_body` does for
    // a function body. Without this, a module-level `p: P = C()` leaves
    // `p` untyped for the following `p.same(C())`, no specialization is
    // created, and `pycc_mir` panics on the dropped `$fn:C.same`.
    let mut module_env = env.clone();
    for item in items {
        match item {
            HirItem::Function {
                ref name,
                ref params,
                ref return_ty,
                ..
            } if protocol_funcs.contains_key(name.as_str()) => {
                // Drop the original protocol-parameter function (only
                // specializations are kept). But if the function has no
                // call sites (no specializations), it would be silently
                // dropped — this is fine, matching how generic functions
                // are dropped.
                let _ = (params, return_ty);
                kept.push(false);
            }
            HirItem::TopLevelStmt(ref stmt) => {
                bind_local_types_in_stmt(&mut module_env, &[], stmt);
                let mut new_stmt = stmt.clone();
                rewrite_protocol_calls_in_stmt(
                    &mut new_stmt,
                    &protocol_funcs,
                    &module_env,
                    &[],
                    &mut specializations,
                    &mut seen,
                );
                new_items.push(HirItem::TopLevelStmt(new_stmt));
                kept.push(true);
            }
            HirItem::Function {
                name,
                params,
                return_ty,
                body,
            } => {
                let local_names: Vec<String> = function_local_names(&params, &body)
                    .into_iter()
                    .map(String::from)
                    .collect();
                let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
                let mut fn_env = env.child_for_function(&local_names_refs);
                for (param_name, param_ty) in &params {
                    fn_env.bind(param_name.clone(), param_ty.clone());
                }
                bind_local_types_in_body(&mut fn_env, &local_names_refs, &body);
                let mut new_body = body;
                for stmt in new_body.iter_mut() {
                    rewrite_protocol_calls_in_stmt(
                        stmt,
                        &protocol_funcs,
                        &fn_env,
                        &local_names_refs,
                        &mut specializations,
                        &mut seen,
                    );
                }
                new_items.push(HirItem::Function {
                    name,
                    params,
                    return_ty,
                    body: new_body,
                });
                kept.push(true);
            }
        }
    }
    // Rewrite calls inside specializations recursively. A specialized
    // body may itself call another protocol-parameter function, which
    // needs its own specialization. Process by index because rewriting
    // may push new specializations, extending the vector we're walking.
    //
    // The per-item work is extracted into `rewrite_protocol_calls_in_specialization`
    // so that the defense-in-depth non-`Function` arm (all specializations
    // are `HirItem::Function` in practice) is a `match` arm in a standalone
    // function that a direct unit test can cover, rather than an `if let`
    // whose never-taken false branch would carry an uncovered continuation
    // region under the 100%-coverage gate (D-014).
    let mut all_specializations = std::mem::take(&mut specializations);
    let mut i = 0;
    while i < all_specializations.len() {
        rewrite_protocol_calls_in_specialization(
            env,
            &mut all_specializations[i],
            &protocol_funcs,
            &mut specializations,
            &mut seen,
        );
        i += 1;
        if !specializations.is_empty() {
            all_specializations.append(&mut specializations);
        }
    }
    new_items.extend(all_specializations);
    (new_items, kept)
}

/// Recomputes every `ImportBinding::Foreign`'s recorded item position for
/// an item list monomorphization has rewritten (PR 1c of #1080 review
/// finding 1).
///
/// A `ForeignImportSite::Item` index is the item *count* at the moment the
/// `import` lowered, so the import belongs immediately before that original
/// item --
/// which means the new position is simply how many of the original items
/// strictly before it still exist. `survives[j]` says whether original
/// item `j` reached the returned list; a trailing import records
/// `hir.items.len()`, which the clamp maps to "after everything that
/// survived". Items monomorphization *appends* (generic instantiations,
/// protocol specializations) all land after every original item, so they
/// never shift a recomputed position. The result stays ascending, which is
/// what `pycc_mir::splice_foreign_imports`'s running-offset insertion
/// assumes.
fn remap_foreign_import_positions(
    imports: &[ImportBinding],
    survives: &[bool],
) -> Vec<ImportBinding> {
    imports
        .iter()
        .map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from,
                site,
                span,
            } => ImportBinding::Foreign {
                local_name: local_name.clone(),
                module_path: module_path.clone(),
                from: from.clone(),
                // A block import has no item position (#1291).
                site: match site {
                    ForeignImportSite::Item(index) => ForeignImportSite::Item(
                        survives[..(*index).min(survives.len())]
                            .iter()
                            .filter(|kept| **kept)
                            .count(),
                    ),
                    site @ ForeignImportSite::Block { .. } => *site,
                },
                span: *span,
            },
            other => other.clone(),
        })
        .collect()
}

/// #380 (PR-20): Per-item worker for the specialization rewrite loop.
/// Rewrites protocol-parameter function calls inside one already-created
/// specialization's body, pushing any newly discovered specializations.
/// Extracted from `monomorphize_protocol_params`'s loop so the
/// defense-in-depth non-`Function` arm is a `match` arm coverable by a
/// direct unit test (D-014).
pub(crate) fn rewrite_protocol_calls_in_specialization(
    env: &Environment,
    spec: &mut HirItem,
    protocol_funcs: &HashMap<String, HirItem>,
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    let (params, body) = match spec {
        HirItem::Function { params, body, .. } => (params, body),
        // All specializations are `HirItem::Function` (created by
        // `rewrite_protocol_calls_in_expr`). This arm is defense-in-depth,
        // covered by a direct unit test.
        _ => return,
    };
    let local_names: Vec<String> = function_local_names(params, body)
        .into_iter()
        .map(String::from)
        .collect();
    let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
    let mut spec_env = env.child_for_function(&local_names_refs);
    for (param_name, param_ty) in params.iter() {
        spec_env.bind(param_name.clone(), param_ty.clone());
    }
    bind_local_types_in_body(&mut spec_env, &local_names_refs, body);
    for stmt in body.iter_mut() {
        rewrite_protocol_calls_in_stmt(
            stmt,
            protocol_funcs,
            &spec_env,
            &local_names_refs,
            specializations,
            seen,
        );
    }
}

/// #380 (PR-20): Rewrites calls to protocol-parameter functions in a
/// statement, creating monomorphized specializations as needed.
fn rewrite_protocol_calls_in_stmt(
    stmt: &mut HirStmt,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    match stmt {
        HirStmt::ExprStmt(expr) => {
            rewrite_protocol_calls_in_expr(
                expr,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::Assign { value, .. } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::AnnAssign {
            value: Some(value), ..
        } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::Return(Some(expr)) => {
            rewrite_protocol_calls_in_expr(
                expr,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::If { test, body, orelse } => {
            rewrite_protocol_calls_in_expr(
                test,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            for s in orelse.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::While { test, body } => {
            rewrite_protocol_calls_in_expr(
                test,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::ForRange {
            start,
            stop,
            step,
            body,
            ..
        } => {
            rewrite_protocol_calls_in_expr(
                start,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                stop,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                step,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::ForList { body, .. } => {
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::DictSet { key, value, .. } => {
            rewrite_protocol_calls_in_expr(
                key,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::AttrSet { base, value, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // Part 2c of #1371: a slice `del` reads its base and bounds; an
        // unrewritten protocol call there would dangle once the original
        // function item is dropped.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            for operand in std::iter::once(base).chain([start, stop, step].into_iter().flatten()) {
                rewrite_protocol_calls_in_expr(
                    operand,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1457: an attribute `del` reads its base, for the same reason.
        HirStmt::DeleteAttr { base, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1254: the loop variable is bound in a scoped clone of `env`
        // before `cond` and the elements are walked, so a protocol call
        // whose argument reads it resolves; before, `infer_expr_in` failed
        // on the unbound name and the call was silently left unrewritten.
        HirStmt::ListCompAssign {
            var,
            iter,
            cond,
            elt,
            ..
        }
        | HirStmt::SetCompAssign {
            var,
            iter,
            cond,
            elt,
            ..
        } => {
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().map(|c| c.as_mut()).collect();
            body.push(elt);
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::DictCompAssign {
            var,
            iter,
            cond,
            key,
            value,
            ..
        } => {
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().map(|c| c.as_mut()).collect();
            body.extend([key.as_mut(), value.as_mut()]);
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        _ => {}
    }
}

/// #380 (PR-20): Rewrites calls to protocol-parameter functions in an
/// expression, creating monomorphized specializations as needed.
fn rewrite_protocol_calls_in_expr(
    expr: &mut HirExpr,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    match expr {
        HirExpr::Call { callee, args } => {
            // First, recurse into arguments (they may contain nested calls).
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            // Check if this is a call to a protocol-parameter function.
            if let Some(func_item) = protocol_funcs.get(callee.as_str())
                && let HirItem::Function {
                    name,
                    params,
                    return_ty,
                    body,
                } = func_item
            {
                // Collect all protocol→concrete substitutions from every
                // protocol-typed parameter's corresponding argument.
                let mut substitutions: Vec<(String, Ty)> = Vec::new();
                for (i, (_, param_ty)) in params.iter().enumerate() {
                    if let Ty::Protocol(proto_name) = param_ty
                        && i < args.len()
                    {
                        let arg_ty = infer_expr_in(env, local_names, &args[i]);
                        if let Ok(Ty::Instance(concrete_name)) = arg_ty {
                            substitutions
                                .push((proto_name.as_ref().clone(), Ty::Instance(concrete_name)));
                        }
                    }
                }
                if !substitutions.is_empty() {
                    let mangled = mangle_protocol_instantiation(name, &substitutions);
                    if seen.insert(mangled.clone()) {
                        let substituted_params: Vec<(String, Ty)> = params
                            .iter()
                            .map(|(n, ty)| (n.clone(), substitute_ty_protocols(ty, &substitutions)))
                            .collect();
                        let substituted_return = substitute_ty_protocols(return_ty, &substitutions);
                        let substituted_body = substitute_body_protocols(body, &substitutions);
                        specializations.push(HirItem::Function {
                            name: mangled.clone(),
                            params: substituted_params,
                            return_ty: substituted_return,
                            body: substituted_body,
                        });
                    }
                    *callee = mangled;
                }
            }
        }
        HirExpr::MethodCall { base, method, args } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            // #953: a method whose signature has a protocol-typed
            // parameter is in `protocol_funcs` under its mangled
            // `<Class>.<method>` name, so the loop above already dropped
            // its original definition from the module. Without the
            // specialization this arm creates, `$fn:<Class>.<method>` is
            // never bound and `pycc_mir::build` panics on the very call
            // the type checker just accepted.
            if let Some(rewritten) = specialize_protocol_method_call(
                base,
                method,
                args,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            ) {
                *expr = rewritten;
            }
        }
        HirExpr::AttrGet { base, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            rewrite_protocol_calls_in_expr(
                operand,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1211: `BoolOp` shares `BinOp`'s arm. Without it the `_ => {}`
        // catch-all below would skip a protocol-typed call inside `and`/`or`
        // and leave it unspecialized.
        HirExpr::BinOp { left, right, .. } | HirExpr::BoolOp { left, right, .. } => {
            rewrite_protocol_calls_in_expr(
                left,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                right,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1395: like `BoolOp` above, without this arm the `_ => {}`
        // catch-all below would leave a protocol-typed call inside a
        // conditional expression unspecialized.
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1411: without this arm the `_ => {}` catch-all below would leave
        // a protocol-typed call in the constructor arguments of
        // `type(self)(...)` unspecialized.
        HirExpr::ReceiverClassCall { args } => {
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Part 2a of #1371: like `IfExp` above, without this arm the
        // `_ => {}` catch-all below would leave a protocol-typed call in the
        // arguments of `table[k](args)` unspecialized.
        HirExpr::ExprCall { callee, args } => {
            for part in std::iter::once(callee.as_mut()).chain(args.iter_mut()) {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Part 8 of #1371: the positional half and every keyword value.
        HirExpr::KeywordCall { call, keywords, .. } => {
            for part in
                std::iter::once(call.as_mut()).chain(keywords.iter_mut().map(|(_, value)| value))
            {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1212: without this arm the `_ => {}` catch-all below would skip
        // a protocol-typed call inside a chained comparison and leave it
        // unspecialized.
        HirExpr::CompareChain { first, links } => {
            rewrite_protocol_calls_in_expr(
                first,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for link in links.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    &mut link.right,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirExpr::Compare { left, right, .. } => {
            rewrite_protocol_calls_in_expr(
                left,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                right,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirExpr::ListLiteral(elements) | HirExpr::ObjectList(elements) => {
            for e in elements.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    e,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Issue #1188: rewrite inside the wrapped method call. A
        // specialization replaces `call` with a plain `Call`, and that only
        // happens for an `Instance` receiver -- the method reading -- so the
        // specialized call then stands for the whole node.
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            rewrite_protocol_calls_in_expr(
                call,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            if !matches!(call.as_ref(), HirExpr::MethodCall { .. }) {
                *expr = (**call).clone();
            }
        }
        // PEP 572 (#774), deep-review follow-up: unlike its exhaustive
        // `rewrite_generic_calls_in_expr` sibling, this function is a
        // non-exhaustive recursion over only the shapes a protocol-typed
        // call can appear inside -- the trailing `_ => {}` above is
        // deliberate for every other `HirExpr` variant. But a walrus's own
        // value is exactly such a shape (`(x := proto_fn(c))` nests a
        // protocol call inside `value`), so without a dedicated arm here it
        // fell into the catch-all: `proto_fn`'s call site was never
        // rewritten to its specialization, while `monomorphize_protocol_params`
        // unconditionally drops the original `proto_fn` item regardless,
        // leaving a dangling call to a function no longer in the module.
        HirExpr::NamedExpr { value, .. } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1254 (D-250): without this arm a protocol call inside a
        // comprehension fell into the catch-all below and was left calling
        // an item `monomorphize_protocol_params` drops (the #774 bug class).
        HirExpr::Comprehension(comp) => {
            let HirComprehension {
                var,
                iter,
                cond,
                elt,
            } = comp.as_mut();
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().collect();
            match elt {
                CompElt::List(e) | CompElt::Set(e) => body.push(e),
                CompElt::Dict { key, value } => body.extend([key, value]),
            }
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        _ => {}
    }
}

/// Protocol-call rewriting for one comprehension, statement or expression
/// form (#1254): the range operands are walked against `env`, where they
/// are evaluated; `cond` and the elements against a clone of `env` with the
/// synthesized loop variable bound, since this walker's `env` is immutable
/// and `bind_local_types_in_stmt` never binds that variable.
#[allow(clippy::too_many_arguments)]
fn rewrite_protocol_calls_in_comprehension(
    var: &str,
    iter: &mut CompIter,
    body: Vec<&mut HirExpr>,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    let operands: Vec<&mut HirExpr> = match iter {
        CompIter::Range { start, stop, step } => vec![start, stop, step],
        CompIter::Iterable(iterable) => vec![iterable.as_mut()],
        CompIter::Name(_) => vec![],
    };
    for operand in operands {
        rewrite_protocol_calls_in_expr(
            operand,
            protocol_funcs,
            env,
            local_names,
            specializations,
            seen,
        );
    }
    let mut scoped = env.clone();
    if let Ok(var_ty) = crate::comprehension::resolve_comp_iter(env, local_names, iter) {
        scoped.bind(var.to_string(), var_ty);
    }
    for sub in body {
        rewrite_protocol_calls_in_expr(
            sub,
            protocol_funcs,
            &scoped,
            local_names,
            specializations,
            seen,
        );
    }
}

pub(crate) fn monomorphize(hir: &HirModule) -> Result<HirModule, Diagnostic> {
    let generics: HashMap<String, HirItem> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function {
                name,
                params,
                return_ty,
                ..
            } if is_generic_signature(params, return_ty) => Some((name.clone(), item.clone())),
            _ => None,
        })
        .collect();
    if generics.is_empty()
        && !hir.class_defs.iter().any(|(_, cd)| cd.type_param.is_some())
        && !hir.items.iter().any(|item| {
            if let HirItem::Function { params, .. } = item {
                has_protocol_param(params)
            } else {
                false
            }
        })
    {
        // PR-13 final review (I1): `type_aliases` is emptied here exactly as
        // it is on the monomorphized path below, so a resolved module's own
        // `type_aliases` value never depends on whether the module happened
        // to contain a generic function. D-135 aliases are fully discharged
        // during HIR lowering (`annotation_to_ty` resolves every alias name
        // into a concrete `Ty` before `pycc_types` runs); nothing downstream
        // -- not `pycc_mir`, not `pycc_codegen` -- reads the field after
        // lowering, so the resolved HIR's `type_aliases` is empty by design.
        return Ok(HirModule {
            // Provenance travels with the module: monomorphization
            // rewrites and extends `class_defs`, but never changes who
            // produced the builtin exception entries already in it.
            seeded_builtin_exception_classes: hir.seeded_builtin_exception_classes,
            items: hir.items.clone(),
            type_aliases: Vec::new(),
            // Part 1 of #1026: `imports` used to be dropped here for the
            // same reason `type_aliases` still is -- fully discharged
            // during HIR lowering, read by nothing downstream. That stopped
            // being true when `ImportBinding::Foreign` became the record of
            // a CPython import: `pycc_mir::build` splices one
            // `MirItem::ForeignImport` per foreign binding into the item
            // list at its recorded position, and `src/main.rs`'s `I0403`
            // gate reads the same list. Dropping the field here left both
            // reading an empty list, so an `import numpy` compiled to an
            // artifact that never imported anything. This exit returns
            // `hir.items` unchanged, so each binding's recorded
            // item index still addresses the item it was recorded
            // against; the monomorphized exit below has to recompute them.
            imports: hir.imports.clone(),
            // `class_defs` is likewise actively consumed after this point: `check`'s
            // own class-body checking (Task 3) and every one of
            // `pycc_mir`/`pycc_codegen`'s slot-index/method-mangled-name
            // lookups (Tasks 5/6) read it from the `HirModule` that reaches
            // them, which is this function's own return value on the
            // monomorphized path. Dropping it here would silently break
            // every class-containing module the moment it reached
            // `pycc_mir::build`.
            class_defs: hir.class_defs.clone(),
        });
    }

    let mut env = Environment::new();
    // D-154 (Part 1 of #375): this pass's own `env` needs the class table
    // too -- `rewrite_generic_calls_in_expr`'s `AttrGet`/`MethodCall` arms
    // fall through to `infer_expr_in` exactly like every other arm here,
    // which resolves a class-instance attribute/method through
    // `Environment::classes` (`class::resolve_attr_get`/
    // `resolve_method_call`). Without this, a generic function whose own
    // body reads/writes an instance attribute or calls a method would
    // panic here with "class `X` has no registered HirClassDef" the
    // moment this pass reached it, even though `check_with_signatures_all`
    // already validated that exact body successfully against a `classes`-
    // populated `Environment` of its own.
    crate::class::bind_classes(&mut env, hir);
    for item in &hir.items {
        if let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        {
            let param_tys = params.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
            env.bind_function(name.clone(), param_tys, return_ty.clone());
        }
    }
    for (name, item) in &generics {
        env.bind_generic(name.clone(), item.clone());
    }

    let function_local_names = module_function_local_names(hir);
    let mut instantiations: Vec<GenericInstantiation> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // PEP 695 (#387): pre-scan for generic class instantiations
    // (`C[type_arg](args)`) and monomorphize each unique `(class, type_arg)`
    // pair's methods before the rewrite pass starts. This way,
    // `rewrite_generic_calls_in_expr`'s `GenericClassInstantiate` arm can
    // simply rewrite the expression to an ordinary `HirExpr::Call` to the
    // mangled class name, confident the monomorphized methods are already
    // registered in `env` and collected in `instantiations`.
    let mut new_class_defs: Vec<(String, HirClassDef)> = Vec::new();
    instantiate_generic_class_methods(
        hir,
        &mut env,
        &mut instantiations,
        &mut seen,
        &mut new_class_defs,
    );

    // Two passes, matching `check_with_environment_all`'s own D-041 discipline
    // exactly (register every function's signature -- already done above --
    // then process every top-level statement in source order against the
    // *growing* module `env`, then check every function body against the
    // module `env` as it stands once the whole module's top-level code has
    // run): a function body must be able to rewrite a generic call whose
    // argument reads a module-level global assigned *later* in the file
    // than the function's own `def`, exactly as `check_function_in` already
    // allows for ordinary type-checking. A single source-order pass here
    // would process a function body's rewrite before a later top-level
    // assignment had grown `env`, wrongly reporting the global as
    // undefined even though `check`/`check_and_resolve`'s own validation
    // (which already ran successfully before `monomorphize` is ever called)
    // accepted the exact same program.
    //
    // Each item's *original* index is carried alongside its rewritten form
    // so the final assembly can restore source order -- this pass processes
    // top-level statements and function bodies in two separate sweeps, not
    // in original item order.
    let mut rewritten: Vec<Option<HirItem>> = vec![None; hir.items.len()];

    // Pass 1: every top-level statement, in source order, growing `env`.
    for (index, (item, local_names)) in hir.items.iter().zip(&function_local_names).enumerate() {
        if let HirItem::TopLevelStmt(stmt) = item {
            let mut new_stmt = stmt.clone();
            rewrite_generic_calls_in_stmt(
                &mut env,
                local_names,
                &mut new_stmt,
                &mut instantiations,
                &mut seen,
            )?;
            rewritten[index] = Some(HirItem::TopLevelStmt(new_stmt));
        }
    }

    // Pass 2: every function body, against the module `env` as it stands
    // once the whole module's top-level code has been processed above --
    // an original generic function is dropped entirely (only its concrete
    // specializations, appended below, may reach `pycc_mir`).
    for (index, (item, local_names)) in hir.items.iter().zip(&function_local_names).enumerate() {
        let HirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        else {
            continue;
        };
        if generics.contains_key(name) {
            continue;
        }
        let mut fn_env = env.child_for_function(local_names);
        for (param_name, param_ty) in params {
            fn_env.bind(param_name.clone(), param_ty.clone());
        }
        let mut new_body = body.clone();
        for stmt in new_body.iter_mut() {
            rewrite_generic_calls_in_stmt(
                &mut fn_env,
                local_names,
                stmt,
                &mut instantiations,
                &mut seen,
            )?;
        }
        rewritten[index] = Some(HirItem::Function {
            name: name.clone(),
            params: params.clone(),
            return_ty: return_ty.clone(),
            body: new_body,
        });
    }

    // PR 1c of #1080 review finding 1: `flatten` drops every slot pass 2
    // left `None` (each original generic function), so an item's position
    // in `items` is no longer its position in `hir.items`. Record which
    // originals survive before the shape that carried the information is
    // gone; `monomorphize_protocol_params` below drops items a second time
    // and reports its own mask, and the two compose into one remap of
    // the `ForeignImportSite::Item` index of each `ImportBinding::Foreign`.
    let kept_after_rewrite: Vec<bool> = rewritten.iter().map(Option::is_some).collect();
    let mut items = rewritten.into_iter().flatten().collect::<Vec<_>>();
    // PEP 695 (#387): Pass 2b — rewrite any `GenericClassInstantiate`
    // expressions that survived inside monomorphized generic-class method
    // bodies. Pass 2 only iterates over the original `hir.items`, not the
    // `instantiations` appended below, so a GCI nested inside a generic
    // class method (e.g. `class C[T]: def f(self): b = D[int](1)`) would
    // survive into the monomorphized copy and panic at MIR lowering. Run
    // the same `rewrite_generic_calls_in_stmt` over each instantiation's
    // body before appending it, using a fresh child environment seeded
    // with the instantiation's own (already-substituted) parameter types.
    // Process by index because `rewrite_generic_calls_in_stmt` may push
    // new instantiations (a GCI inside a monomorphized body triggers
    // another class monomorphization), extending the vector we're walking.
    //
    // The per-item work is extracted into `rewrite_generic_calls_in_instantiation`
    // so that the defense-in-depth non-`Function` arm (all instantiations
    // are `HirItem::Function` in practice) is a `match` arm in a standalone
    // function that a direct unit test can cover, rather than an `if let`
    // whose never-taken false branch would carry an uncovered continuation
    // region under the 100%-coverage gate (D-014).
    let mut i = 0;
    while i < instantiations.len() {
        rewrite_generic_calls_in_instantiation(&mut env, i, &mut instantiations, &mut seen)?;
        i += 1;
    }
    for instantiation in instantiations {
        items.push(instantiation.specialized);
    }
    // #380 (PR-20): Protocol-typed parameter monomorphization. Functions
    // with `Ty::Protocol` parameters need to be specialized per concrete
    // call-site type, substituting the protocol type with the concrete
    // `Ty::Instance` so MIR/codegen can resolve method calls and attribute
    // access against the concrete class. This pass runs after the existing
    // generic function monomorphization, scanning the (already rewritten)
    // items for calls to protocol-parameter functions.
    let kept_after_protocol;
    (items, kept_after_protocol) = monomorphize_protocol_params(items, &env, &mut new_class_defs);
    // Compose the two drop masks into one "did original item `j` survive
    // the whole function" mask, then recompute every foreign import's
    // recorded position against it. `kept_after_protocol` is parallel to
    // that pass's *input*, whose leading `kept_after_rewrite.iter().filter(..).count()`
    // entries are the surviving originals in order, so walking the two in
    // lockstep is the composition.
    let mut surviving_position = 0usize;
    let survives: Vec<bool> = kept_after_rewrite
        .iter()
        .map(|kept| {
            if !*kept {
                return false;
            }
            let still_here = kept_after_protocol[surviving_position];
            surviving_position += 1;
            still_here
        })
        .collect();
    let imports = remap_foreign_import_positions(&hir.imports, &survives);
    // `type_aliases` is empty by design on both of this function's exits
    // -- see the no-generics early return above (PR-13 final review I1)
    // and that return's own comment for why `class_defs` is not treated
    // the same way. `imports` is carried on both exits (Part 1 of #1026),
    // recomputed here because this exit rewrites the item list the
    // positions in it refer to.
    // PEP 695 (#387): include the monomorphized class definitions alongside
    // the originals, so `pycc_mir`'s `classes` HashMap can resolve the
    // mangled class name to its specialized `HirClassDef` (attribute slots,
    // method table) for instantiation and method-call lowering.
    let mut class_defs = hir.class_defs.clone();
    class_defs.extend(new_class_defs);
    Ok(HirModule {
        // Provenance travels with the module: monomorphization
        // rewrites and extends `class_defs`, but never changes who
        // produced the builtin exception entries already in it.
        seeded_builtin_exception_classes: hir.seeded_builtin_exception_classes,
        items,
        type_aliases: Vec::new(),
        // Carried for the same reason as on the no-generics path above:
        // Part 1 of #1026 gave `imports` a downstream reader. Unlike that
        // path, this one rewrote `items`, so the positions are remapped.
        imports,
        class_defs,
    })
}
