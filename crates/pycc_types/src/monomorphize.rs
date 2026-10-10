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
mod class_methods;
mod collect_classes;
mod protocol_params;
mod rewrite_calls;
mod substitute;
pub(crate) use class_methods::*;
pub(crate) use collect_classes::*;
pub(crate) use protocol_params::*;
pub(crate) use rewrite_calls::*;
pub(crate) use substitute::*;
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
