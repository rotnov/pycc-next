//! Expression type inference: [`infer_expr`] and its `local_names`-aware
//! worker `infer_expr_in`.
//!
//! This is the crate's single largest cohesion boundary -- one recursive
//! walk over every [`HirExpr`] variant, producing the expression's [`Ty`]
//! or the diagnostic that rejects it. It was extracted from
//! [`lib.rs`](crate) per the repository's source-file decomposition rule
//! (AGENTS.md "Keep source files decomposable", tracked by issue #544),
//! as a pure relocation: no inference behavior changed with the move.
//!
//! Statement checking, assignability, and the class/exception/operator
//! helpers this walk calls into all stay where they were -- in
//! [`lib.rs`](crate) and its other submodules -- so this module is the
//! expression seam only.

mod receiver_dispatch;

use crate::binop::numeric_result_type;
use crate::class;
use crate::std_receiver::{shadowed_std_receiver, std_qualified_symbol, std_receiver_shadowed};
use crate::string_conversion::{StringConversionSite, reject_unrenderable_expr};
use crate::unop::unary_result_type;
use crate::{
    BindingState, Environment, annotation_marker_is_not_a_value, cast_marker_is_not_a_value,
    enum_marker_is_not_a_value, enum_member_attr_type, instantiate_generic_call, is_assignable,
    is_known_callable_builtin, is_local, is_marker_kind, lookup_bound_name, marker_is_not_a_value,
    non_callable_binding, possibly_unbound, std_constant_is_not_callable,
    std_function_used_as_a_value, std_scalar_to_ty, type_checking_marker_is_not_a_value,
    typing_form_is_not_a_value, unbound_local, unsupported_callable_builtin,
};

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;
use pycc_hir::{ContainerReceiver, FStringPart, HirExpr};

pub fn infer_expr(env: &Environment, expr: &HirExpr) -> Result<Ty, Diagnostic> {
    infer_expr_in(env, &[], expr)
}

/// Whether `receiver` is bound at this use site, for the validation pass's
/// stdlib-receiver shadow check (`shadowed_std_receiver`). Three sources,
/// each closing a distinct hole:
///
/// - `binding_state` rather than `lookup`: `lookup` deliberately returns
///   `None` for a `BindingState::Maybe` binding, so a conditionally bound
///   `m` (`if c: m = 1.0`) would otherwise slip past the check and reach
///   the libm path.
/// - `def_rebound` (the precedent is the `non_callable_binding` gate in
///   `infer_expr_in`'s call arm), never `lookup_function`, which is
///   position-blind: `import math` / `print(math.sqrt(4.0))` / `def
///   math(): ...` *below* the use is valid CPython and must stay accepted,
///   which only the source-order rebinding set encodes. Inside a function
///   body the set is the module's final one (`child_for_function` clones
///   it after pass 2), so there a `def m()` anywhere in the module
///   rejects `m.sqrt` -- a fail-closed divergence recorded in the #962
///   ADR.
/// - `is_local`: a parameter or body-assigned local of the enclosing
///   function.
fn is_std_receiver_bound(env: &Environment, local_names: &[&str], receiver: &str) -> bool {
    env.binding_state(receiver).is_some()
        || env.def_rebound.contains(receiver)
        || is_local(local_names, receiver)
}

/// Whether a bare `name` standing in a class-dispatch position -- `C[i]`,
/// `C.attr`, `C.m()` -- refers to the class itself rather than to a value
/// that happens to carry the same name.
///
/// Three answers, not two:
///
/// - `Ok(true)`: `name` is a registered class and nothing shadows it, so the
///   caller dispatches on the class.
/// - `Ok(false)`: `name` is not a class at all, or a *parameter or
///   function-local* of that name shadows it. `def f(B: D) -> int: return
///   B.X` reads the parameter, exactly as CPython does, so the caller falls
///   through to the ordinary value path.
/// - `Err(C0001)`: `name` is a class whose own name the **module's top-level
///   code** also binds to a value, and the read is inside a function body.
///
/// That last case is the one #974's round-5 review found, and it is rejected
/// rather than resolved because no resolution would be correct. A function
/// body is checked (D-041 late binding) against the environment as it stands
/// after *all* top-level code has run, so `binding_state` there cannot say
/// whether the rebinding executes before or after the call: `A = D()` above
/// the call means the read is the instance, the same statement below it means
/// the read is the class, and one ordering-blind snapshot has to answer both.
/// Resolving either way silently mis-compiles the other. Answering the same
/// question at *module* scope is sound and stays supported -- pass 2 walks
/// top-level statements sequentially, so `in_function_body` is `false` there
/// and a rebinding simply shadows the class from that statement on, which is
/// what CPython does.
///
/// Ordering-aware name resolution would resolve the rejected case properly;
/// this compiler does not have it, and building it is a separate analysis.
pub(crate) fn class_name_dispatch(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> Result<bool, Diagnostic> {
    if env.lookup_class(name).is_none() || is_local(local_names, name) {
        return Ok(false);
    }
    if env.binding_state(name).is_none() {
        return Ok(true);
    }
    if !env.in_function_body {
        return Ok(false);
    }
    Err(Diagnostic::error(
        "C0001",
        format!(
            "class `{name}` is also bound to a value at module scope, so `{name}` inside a \
             function body is ambiguous -- pycc resolves a bare class name against the \
             module environment as it stands after all top-level code has run, which cannot \
             tell whether the rebinding happens before or after this call"
        ),
        Span::new(0, 0),
    )
    .with_help("give the value binding a name of its own"))
}

/// The static type of a container method node's receiver (#1263, Part 2 of
/// #1218). A bare name keeps `lookup_bound_name`, so its diagnostics are
/// exactly D-105's; an attribute read is inferred like any other
/// expression, and the node's own arm then applies the same `T0033`/`T0021`
/// checks to the result.
///
/// An attribute of a CPython object infers as `Ty::Object`. The node's
/// `T0033` ("`object` does not support `.append()`") would misstate that as
/// a type error, when CPython runs the call and pycc merely does not
/// implement it, so this refuses it with the #1026 `I0404` family instead.
/// Since #1095 a module with its own foreign import (or an `ext` module)
/// lowers such a call to a receiver-dispatched call whose object receiver
/// takes the foreign method call, and since #1425 so does a module that
/// imports an object from a sibling project module, for its whole body.
/// Since #1482 an own foreign import admits for the whole body too, so no
/// source program reaches this arm: natively a `Ty::Object` comes only from
/// an own foreign import, a sibling module's object or an `ext` module, all
/// of which admit, and the one remaining candidate, `NotImplemented.x`, is
/// `T0021` ("name `NotImplemented` is not defined") outside an `ext` module.
/// The arm stays as a defensive refusal, so a lowering regression reports
/// `I0404` rather than a misleading `T0033`; its test builds the container
/// node directly (`foreign/container_names_tests.rs`).
fn infer_container_receiver(
    env: &Environment,
    local_names: &[&str],
    receiver: &ContainerReceiver,
    method: &str,
) -> Result<Ty, Diagnostic> {
    match receiver {
        ContainerReceiver::Name(name) => lookup_bound_name(env, local_names, name),
        ContainerReceiver::Attr(receiver) => {
            let receiver_ty = infer_expr_in(env, local_names, receiver)?;
            if receiver_ty == Ty::Object {
                return Err(crate::foreign::object_operation_unsupported(&format!(
                    "calling `{method}` on a CPython object's attribute"
                )));
            }
            Ok(receiver_ty)
        }
    }
}

pub(crate) fn infer_expr_in(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    match expr {
        HirExpr::IntLiteral(_) => Ok(Ty::Int),
        HirExpr::FloatLiteral(_) => Ok(Ty::Float),
        HirExpr::BoolLiteral(_) => Ok(Ty::Bool),
        HirExpr::StringLiteral(_) => Ok(Ty::Str),
        HirExpr::NoneLiteral => Ok(Ty::None),
        // #1418: CPython's `NotImplemented` singleton is a CPython object.
        HirExpr::NotImplemented => Ok(Ty::Object),
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(expr) = part {
                    // #977 (D-237): the backend renders scalars, `@dataclass`
                    // instances and caught builtin exceptions; every other
                    // instance and every protocol-typed value is rejected
                    // here with `C0001` before it can reach the codegen
                    // `to_str` panic. See `string_conversion.rs`.
                    reject_unrenderable_expr(
                        env,
                        local_names,
                        expr,
                        StringConversionSite::FStringInterpolation,
                    )?;
                }
            }
            Ok(Ty::Str)
        }
        HirExpr::Name(name) => {
            if let Some(symbol) = std_qualified_symbol(name) {
                // See `shadowed_std_receiver`'s own doc comment -- a real
                // local/parameter named `math` (or an alias of it, #962)
                // shadows the stdlib module.
                if let Some(shadowed) = shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                    is_std_receiver_bound(env, local_names, receiver)
                }) {
                    return Err(std_receiver_shadowed(shadowed, symbol.module));
                }
                return match symbol.kind {
                    pycc_std::StdSymbolKind::Constant { ty } => Ok(std_scalar_to_ty(ty)),
                    pycc_std::StdSymbolKind::Function { .. } => {
                        Err(std_function_used_as_a_value(name))
                    }
                    pycc_std::StdSymbolKind::EnumMarker => {
                        Err(enum_marker_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::AnnotationMarker => {
                        Err(annotation_marker_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::CastMarker => Err(cast_marker_is_not_a_value(name)),
                    pycc_std::StdSymbolKind::TypeCheckingMarker => {
                        Err(type_checking_marker_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::TypingFormMarker => {
                        Err(typing_form_is_not_a_value(name))
                    }
                    pycc_std::StdSymbolKind::ProtocolMarker
                    | pycc_std::StdSymbolKind::AbcMarker
                    | pycc_std::StdSymbolKind::DecoratorMarker
                    | pycc_std::StdSymbolKind::EnumAutoMarker => {
                        Err(marker_is_not_a_value(name))
                    }
                };
            }
            // Issue #118 Part 1: three-way distinction -- definitely bound ->
            // ok, maybe bound -> T0041, unbound -> T0021 (local) or "not
            // defined" (global). The stdlib-qualified check above already
            // handled `math.sqrt`-style names.
            match env.binding_state(name) {
                // Issue #769 (Part 2 of #747): a *read* of a definitely-bound
                // name inside a narrowed region resolves to the narrowed
                // (Optional's inner) type instead of the name's real,
                // still-Optional binding -- consulted here only, never by
                // `check_assignment`'s target checking (see `env.narrowed`'s
                // own doc comment).
                Some(BindingState::Definitely(ty)) => {
                    // Part 2 of #1026 (#1081) removed Part 1's
                    // unconditional `reject_object_read` call from this
                    // arm. A read of a foreign binding at *module* scope
                    // now yields `Ty::Object` like any other read: this
                    // function is context-free, so it cannot tell a
                    // `numpy.pi` base apart from a `print(numpy)` operand,
                    // and Part 2 must admit the first. Every *consumer*
                    // refuses on its own instead -- `crate::foreign`'s
                    // module doc carries the full rule.
                    let ty = env.narrowed_ty(name).unwrap_or_else(|| ty.clone());
                    // The read is admitted in both scopes, for any
                    // definitely-bound `object` name -- a foreign import, a
                    // module-level `x = <object>` global, a function local
                    // or a solver-inferred `object` parameter (#1316, Part 1
                    // of #1333). A read that runs before a module-level
                    // binding at run time raises `NameError` from codegen's
                    // unbound-global branch, since D-041 cannot prove call
                    // order here, and a failed operation bridges CPython's
                    // exception into pycc's pending state and branches to
                    // the innermost exception target (`pycc_codegen`'s
                    // `foreign_fail.rs`).
                    reject_memoryview_read(name, &ty, env.owned_buffers.contains(name))?;
                    Ok(ty)
                }
                Some(BindingState::Maybe(_)) => Err(possibly_unbound(name)),
                None => {
                    if is_local(local_names, name) {
                        Err(unbound_local(name))
                    } else {
                        Err(Diagnostic::error(
                            "T0021",
                            format!("name `{name}` is not defined"),
                            Span::new(0, 0), // real span threading through HIR is out of scope for this task -- see Task 15's follow-up note
                        ))
                    }
                }
            }
        }
        HirExpr::UnaryOp { op, operand } => {
            let operand_ty = infer_expr_in(env, local_names, operand)?;
            unary_result_type(*op, operand_ty)
        }
        HirExpr::BoolOp {
            op,
            left,
            right,
            truth_only,
        } => crate::boolop::infer_bool_op(env, local_names, *op, left, right, *truth_only),
        HirExpr::IfExp { test, body, orelse } => {
            crate::if_exp::infer_if_exp(env, local_names, test, body, orelse)
        }
        HirExpr::Unpack { value, arity } => {
            crate::unpack::infer_unpack(env, local_names, value, *arity)
        }
        HirExpr::BinOp { op, left, right } => {
            let left_ty = infer_expr_in(env, local_names, left)?;
            let right_ty = infer_expr_in(env, local_names, right)?;
            numeric_result_type(*op, left_ty, right_ty)
        }
        HirExpr::Compare { op, left, right } => {
            let left_ty = infer_expr_in(env, local_names, left)?;
            let right_ty = infer_expr_in(env, local_names, right)?;
            crate::compare_chain::compare_link_ty(env, *op, left, right, &left_ty, &right_ty)
        }
        HirExpr::CompareChain { first, links } => {
            crate::compare_chain::infer_compare_chain(env, local_names, first, links)
        }
        HirExpr::Call { callee, args } => {
            // D-110 (#133): a call target resolves through the active value
            // binding before any builtin or function-registry fallback -- a
            // module `helper = 1` shadows both a same-named `def` and a
            // builtin at every later call site, and every value binding in
            // the current subset is a primitive, so a shadowed target is
            // non-callable -- with one exception: a module-body `object`
            // binding (a foreign import or a `for` loop target) is callable
            // since #1313, admitted by the first branch inside the gate. The
            // gate is deliberately callee-first (before argument inference),
            // uniform with how the local gate below always behaved. Local diagnostics are preserved exactly:
            // a value-bound local reported `non_callable_binding` before this
            // reordering too, and a local without a binding still falls
            // through to `unbound_local`. In pass 3 the environment is the
            // final module environment (D-041), so a body call is rejected
            // when the callee is value-bound anywhere at top level -- D-110
            // records that consequence (it rejects some later-rebind programs
            // CPython's dynamic order would run) as deliberate; source-order
            // visibility questions stay #22's scope.
            if let Some(ty) = env.lookup(callee)
                && !env.def_rebound.contains(callee)
            {
                // #1313: a direct call of an `object`-typed name (a foreign
                // binding or a `for` loop target) in a module body is an
                // `object` producer under the shared object-call argument
                // rule (`crate::foreign::check_object_call_args`). `lookup` answers `None` for a maybe-bound name, so a
                // one-arm-`if` import falls through to the `T0041` below.
                // #1316 and Part 1 of #1333 admit the same call in a
                // function body for any `object` callee, exactly as the
                // `Name` arm admits its read.
                if matches!(ty, Ty::Object) {
                    let arg_tys = args
                        .iter()
                        .map(|arg| infer_expr_in(env, local_names, arg))
                        .collect::<Result<Vec<_>, _>>()?;
                    crate::foreign::check_object_call_args(env, args, &arg_tys, "call")?;
                    return Ok(Ty::Object);
                }
                return Err(non_callable_binding(callee));
            }
            // Issue #118 Part 1: a maybe-bound callee is not callable -- it
            // may not be bound on every path reaching this call. Reject with
            // T0041 before the `is_local` unbound check below, since a
            // maybe-bound local is "possibly unbound," not "never bound."
            if matches!(env.binding_state(callee), Some(BindingState::Maybe(_))) {
                return Err(possibly_unbound(callee));
            }
            if is_local(local_names, callee) {
                return Err(unbound_local(callee));
            }
            // #435: `isinstance`/`issubclass` are compile-time-evaluated
            // builtins. They must be intercepted BEFORE the generic arg
            // inference loop below, because the class argument (args[1] for
            // isinstance, both args for issubclass) is a class name or tuple
            // of class names — not a value expression. Inferring a bare class
            // name as a regular expression would fail with "name not defined"
            // (class names are registered in `env.classes`, not
            // `env.bindings`). The object argument (isinstance's args[0]) IS
            // inferred normally.
            // A user-defined function named `isinstance`/`issubclass` takes
            // priority over the builtin (same pattern as `float` — see the
            // `a_user_defined_float_function_takes_priority_over_the_builtin`
            // test and its identical guard in the constraint solver).
            if callee == "isinstance" && !env.lookup_function(callee).is_some() {
                return class::check_isinstance(env, local_names, args);
            }
            if callee == "issubclass" && !env.lookup_function(callee).is_some() {
                return class::check_issubclass(env, args);
            }
            // #767: `typing.cast(T, value)` is a compile-time-only construct
            // — a runtime no-op in CPython whose sole effect is to declare
            // `value`'s static type as `T`. Like `isinstance`/`issubclass`
            // above it must be intercepted BEFORE the generic argument
            // inference loop, because `args[0]` is a *type* expression (a
            // bare class or builtin-scalar name, not a value binding) that
            // ordinary inference would reject as an undefined name. A
            // user-defined function named `cast` takes priority over the
            // special case, exactly as for `isinstance`/`issubclass`.
            //
            // The interception does not verify that the module actually wrote
            // `from typing import cast`: `pycc_types` has no access to
            // `HirModule::imports` at all, so no bare-name stdlib symbol is
            // import-gated today (the `Final`/`Annotated`/`Enum` markers behave
            // the same way). Closing that gap uniformly is tracked by #768;
            // `cast_without_its_import_is_currently_accepted` in
            // `tests/typing_cast.rs` pins the present behavior so that fix has
            // to invert it deliberately.
            if callee == "cast" && !env.lookup_function(callee).is_some() {
                return class::check_cast(env, local_names, args);
            }
            // For a callee that survives D-110's binding gate above, preserve
            // the established diagnostic order by inferring every
            // argument before validating arity or compatibility. Most Python
            // calls are small, so keep up to four inferred types on the stack
            // and reserve a heap vector only for wider calls.
            // #1116: `len(b)` on a name bound to a `memoryview` is the
            // buffer's `Ty::Int` element count -- the second read of such a
            // name this compiler admits, after Part 2's `b[i]`. Like the
            // `Subscript` interception further below it must run *before*
            // the argument inference that follows, because `infer_expr_in`'s
            // own `Name` arm calls `reject_memoryview_read`, so inferring
            // the argument first would report the `C0001` capability gap for
            // the very expression that closes it.
            //
            // Only the one-argument shape is claimed, so `len(b, x)` still
            // reaches the `callee == "len"` block's own `T0033` arity
            // refusal below. There is deliberately no `$fn:len` shadow check
            // here, for the reason that block records: D-105 point 3 makes
            // `len` a hand-recognized builtin, not a user-declarable
            // signature.
            if args.len() == 1
                && callee == "len"
                && let HirExpr::Name(buffer_name) = &args[0]
                && matches!(
                    env.binding_state(buffer_name),
                    Some(BindingState::Definitely(Ty::MemoryView))
                )
            {
                return Ok(Ty::Int);
            }
            const INLINE_ARG_TYPES: usize = 4;
            let mut inline_arg_tys = [const { Ty::Infer }; INLINE_ARG_TYPES];
            let heap_arg_tys;
            let arg_tys: &[Ty] = if args.len() <= INLINE_ARG_TYPES {
                for (slot, arg) in inline_arg_tys.iter_mut().zip(args) {
                    *slot = infer_expr_in(env, local_names, arg)?;
                }
                &inline_arg_tys[..args.len()]
            } else {
                heap_arg_tys = args
                    .iter()
                    .map(|arg| infer_expr_in(env, local_names, arg))
                    .collect::<Result<Vec<_>, _>>()?;
                &heap_arg_tys
            };
            if callee == "print" {
                // print's own signature isn't user-declarable in v0.1.
                // #977 (D-237): both arg-inference branches above funnel
                // into `arg_tys`, so this is the single place the
                // string-conversion gate sees every argument -- see the
                // `FString` arm and `string_conversion.rs`. The expression
                // form re-infers each argument (already inferred into
                // `arg_tys` above) so it can look under an erased `cast`.
                for arg in args {
                    reject_unrenderable_expr(
                        env,
                        local_names,
                        arg,
                        StringConversionSite::PrintArgument,
                    )?;
                }
                return Ok(Ty::None);
            }
            if callee == "len" {
                // D-105 point 3: `len(lst)` is a hand-recognized builtin
                // call, same as `print` above -- not a user-declarable
                // signature. Generic over any scalar element type (`T0034`
                // already gates non-`int` lists further upstream, at the
                // point a list literal is constructed), reusing T0033 for
                // both failure shapes (wrong arity, non-list argument) --
                // the same "value does not support list operations" shape
                // already established for `ForList`/`Subscript`/`ListAppend`.
                // PR-11 Task 3 (D-123): also accepts `Ty::Dict` (gated the
                // same way by `T0036`), so `len(d)` type-checks too. PR-11
                // Task 7 (D-123): also accepts `Ty::Set` (gated the same way
                // by `T0038`), so `len(s)` type-checks too.
                if arg_tys.len() != 1 {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!("`len` expects exactly 1 argument, got {}", arg_tys.len()),
                        Span::new(0, 0),
                    ).with_help("pass exactly 1 argument"));
                }
                // Part 3 of #1026 (PR 3a of #1082): `Ty::Object` joins them.
                // A CPython object's length is whatever `PyObject_Size`
                // answers at run time, so the refusal moves from compile
                // time to the host -- `len(o)` on an operand with no
                // `__len__` raises `TypeError` there. The diagnostic text
                // deliberately stays as it is: `object` is not spellable in
                // an annotation (only a foreign-imported class resolves to
                // it, Part 1 of #1367), so naming it in the message a user
                // sees for `len(5)` would point at a type they cannot write.
                if !matches!(
                    arg_tys[0],
                    Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Object
                ) {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!(
                            "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `{}`",
                            arg_tys[0].name()
                        ),
                        Span::new(0, 0),
                    ).with_help("pass a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` value"));
                }
                return Ok(Ty::Int);
            }
            if let Some(symbol) = std_qualified_symbol(callee) {
                // See `shadowed_std_receiver`'s own doc comment -- a real
                // local/parameter named `math` (or an alias of it, #962)
                // shadows the stdlib module.
                if let Some(shadowed) = shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                    is_std_receiver_bound(env, local_names, receiver)
                }) {
                    return Err(std_receiver_shadowed(shadowed, symbol.module));
                }
                let pycc_std::StdSymbolKind::Function {
                    arg_tys: expected_arg_tys,
                    ret_ty,
                } = symbol.kind
                else {
                    return Err(if is_marker_kind(symbol.kind) {
                        if matches!(symbol.kind, pycc_std::StdSymbolKind::EnumMarker) {
                            enum_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::AnnotationMarker)
                        {
                            annotation_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::CastMarker) {
                            cast_marker_is_not_a_value(callee)
                        } else if matches!(
                            symbol.kind,
                            pycc_std::StdSymbolKind::TypeCheckingMarker
                        ) {
                            type_checking_marker_is_not_a_value(callee)
                        } else if matches!(symbol.kind, pycc_std::StdSymbolKind::TypingFormMarker) {
                            typing_form_is_not_a_value(callee)
                        } else {
                            marker_is_not_a_value(callee)
                        }
                    } else {
                        std_constant_is_not_callable(callee)
                    });
                };
                if arg_tys.len() != expected_arg_tys.len() {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`{callee}` expects {} argument(s), got {}",
                            expected_arg_tys.len(),
                            arg_tys.len()
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("pass exactly {} argument(s)", expected_arg_tys.len())));
                }
                for (arg_ty, expected) in arg_tys.iter().zip(expected_arg_tys) {
                    if *arg_ty != std_scalar_to_ty(*expected) {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!(
                                "`{callee}` expects `{}`, got `{}`",
                                std_scalar_to_ty(*expected).name(),
                                arg_ty.name()
                            ),
                            Span::new(0, 0),
                        ).with_help(format!("pass a `{}` value", std_scalar_to_ty(*expected).name())));
                    }
                }
                return Ok(std_scalar_to_ty(ret_ty));
            }
            if callee == "float" && env.lookup_function(callee).is_none() {
                // D-086's own remedy for the int-to-float boundary: a hand-recognized
                // builtin, same as `len` above, not a user-declarable signature --
                // except, unlike `print`/`len`, a program can predate this builtin's
                // introduction with its own `def float(...)`. Reviewer finding
                // (post-merge review): unlike `print`/`len`, which have been
                // hand-recognized since before this compiler could compile
                // user-declared functions at all, `float` was undefined until
                // this issue, so a user-defined `float` was a valid, working
                // program on `main` immediately before this change landed --
                // silently reinterpreting it as this builtin would be a real
                // regression, not an inherited, already-accepted precedent.
                // `env.lookup_function` takes priority; only fall through to the
                // builtin when no such definition exists.
                // Always returns `Ty::Float` regardless of the argument's own type,
                // once that argument is numeric-like -- unlike `len`, there is no
                // homogeneity/element-type question to defer.
                if arg_tys.len() != 1 {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!("`float` expects exactly 1 argument, got {}", arg_tys.len()),
                        Span::new(0, 0),
                    ).with_help("pass exactly 1 argument"));
                }
                // Part 4 of #1026 (PR 4a of #1083) admits `Ty::Object`: an
                // explicit `float(o)` runs CPython's own `PyNumber_Float`
                // protocol through the shim. That is not the implicit
                // thunk-seam crossing D-244 rule 7 closes -- the author named
                // the destination type -- see `docs/TYPE_SYSTEM.md`'s `object`
                // row.
                //
                // The message below deliberately does *not* enumerate
                // `object`: outside an `--ext` module it is unspellable in
                // an annotation (`docs/TYPE_SYSTEM.md`'s `object` row) and
                // exists only because a foreign `import` bound it -- or,
                // since Part 1 of #1367, a class such an import binds
                // annotated it -- so in a native build "pass an `object`" is
                // advice nobody can act on by writing a type. (Under D-258 an
                // ext module can spell it as `Any`/`object`; the message is
                // shared by both modes and keeps the native wording.) `tests::float_of_a_str_is_rejected_as_t0021` pins the
                // exact text, so the omission reads as deliberate rather than
                // as an oversight.
                //
                // Reviewer finding (codex, PR 4a of #1083): the `Ty::Object`
                // admission must not preempt a user-defined `class float`.
                // MIR resolves a call whose callee names a class as an
                // instantiation, so without this guard a module defining
                // `class float` and calling `float(o)` passes `pycc check`
                // with `Ty::Float` and then panics in codegen. The guard is
                // deliberately confined to the `Ty::Object` arm this change
                // introduces: `class float` with an `int` argument already
                // diverges the same way on `main` (`pycc check` accepts,
                // `pycc build` panics) and is tracked separately, so widening
                // the guard to the whole arm would change behavior this change
                // does not own.
                let object_admitted =
                    matches!(arg_tys[0], Ty::Object) && env.lookup_class(callee).is_none();
                if !(matches!(arg_tys[0], Ty::Int | Ty::Float | Ty::Bool) || object_admitted) {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`float` expects an `int`, `float`, or `bool` argument, got `{}`",
                            arg_tys[0].name()
                        ),
                        Span::new(0, 0),
                    ).with_help("pass an `int`, `float`, or `bool` value"));
                }
                return Ok(Ty::Float);
            }
            if callee == "bool"
                && env.lookup_function(callee).is_none()
                && env.lookup_class(callee).is_none()
            {
                // Part 4 of #1026 (PR 4a of #1083): `bool` is admitted for a
                // `Ty::Object` argument *only*. Every other argument type falls
                // through to `unsupported_callable_builtin` below and keeps its
                // C0001 verbatim, so `bool(1)` is refused exactly as it was --
                // the residual incoherence (`bool(o)` compiles, `bool(1)` does
                // not) is stated openly in `docs/TYPE_SYSTEM.md` and tracked by
                // #1017/#1018, which own the general builtin-conversion story.
                //
                // The user-defined-function guard is `float`'s, for `float`'s
                // reason: a `def bool(...)` is a valid, working program on
                // `main` today, so `env.lookup_function` takes priority. The
                // class guard is codex's PR 4a finding, also `float`'s: MIR
                // resolves a call naming a class as an instantiation, so a
                // module defining `class bool` would otherwise pass
                // `pycc check` here and panic in codegen. It sits on the whole
                // arm rather than on the `Ty::Object` test alone because this
                // arm admits nothing else -- every other argument already falls
                // through to `C0001` -- so the two placements are equivalent
                // and the outer one reads plainly.
                if arg_tys.len() == 1 && matches!(arg_tys[0], Ty::Object) {
                    return Ok(Ty::Bool);
                }
            }
            if (callee == "int" || callee == "str")
                && env.lookup_function(callee).is_none()
                && env.lookup_class(callee).is_none()
            {
                // Part 4 of #1026 (PR 4b of #1083): `int` and `str` are
                // admitted for a `Ty::Object` argument *only*, exactly as
                // `bool` above is. Every other argument type falls through to
                // `unsupported_callable_builtin` and keeps its `C0001`
                // verbatim, so `int(1)` and `str(1)` are refused precisely as
                // they were -- the residual asymmetry (`str(o)` compiles,
                // `str(1)` does not) is stated openly in `docs/TYPE_SYSTEM.md`
                // and left to #1017/#1018, which own the general
                // builtin-conversion story.
                //
                // Both guards are `bool`'s, for `bool`'s reasons: a
                // `def int(...)`/`def str(...)` is a valid working program on
                // `main` today and must keep winning, and MIR resolves a call
                // naming a class as an instantiation, so a module defining
                // `class int` would otherwise type-check here and panic in
                // codegen.
                //
                // **Placement of the class guard.** It sits on the whole arm,
                // like `bool`'s and unlike `float`'s. `float`'s is confined to
                // its `Ty::Object` admission because that arm already admitted
                // `int`/`float`/`bool` on `main`, and widening the guard would
                // have changed behavior PR 4a did not own. These two arms are
                // new and admit nothing else, so every other argument already
                // falls through to `C0001` and the two placements are
                // equivalent -- the outer one simply reads plainly. (The
                // general precedence divergence between a class-shadowed
                // builtin in `pycc check` and in `pycc build` is #1107's, not
                // this change's.)
                if arg_tys.len() == 1 && matches!(arg_tys[0], Ty::Object) {
                    return Ok(if callee == "int" { Ty::Int } else { Ty::Str });
                }
            }
            // D-154 (Part 1 of #375): `ClassName(args)` (instantiation)
            // reuses this same generic `HirExpr::Call` node -- there is no
            // dedicated HIR shape for it (`pycc_hir::class`'s own doc
            // comment) -- so it is resolved here, checked before the
            // ordinary ("ClassName" as a plain function) lookup below, the
            // same precedence a generic-function call already gets just
            // above. `pycc_hir::lower_all` (`lower_checked` wraps it) enforces
            // that a class name can never collide with a top-level function,
            // type-alias, or import name in this compiler's flat,
            // single-namespace model -- both directions are rejected with
            // `C0001` at HIR-lowering time, before `env` is ever built (D-068
            // review finding on #385; `crates/pycc_hir/src/module.rs`'s
            // `lower_top_level_item`) -- so trying the class table first here
            // is unambiguous, not merely an assumption.
            if env.lookup_class(callee).is_some() {
                return class::resolve_instantiation(env, callee, args, arg_tys);
            }
            // D-133/D-134: a call to a PEP 695 generic function is resolved
            // through call-site substitution, not through the ordinary
            // `functions` signature -- `env.lookup_function(callee)` would
            // otherwise see a signature still carrying `Ty::Param` and
            // reject every real (concrete) argument as an assignability
            // mismatch. Checked before the ordinary lookup below so this
            // takes precedence for every generic function, including one
            // reached recursively while inferring a nested call's own
            // arguments (e.g. `print(identity(1))`).
            // #1316: a type parameter admits any argument, so a CPython
            // object passed to a generic is refused before substitution
            // (an ordinary user function refuses it in its argument loop
            // below; a constructor or method parameter is always declared
            // or refused as uninferable, so `class::check_call_args`'s own
            // mismatch already refuses an `object` there).
            if let Some(generic_func) = env.lookup_generic(callee) {
                crate::foreign::reject_object_arguments(arg_tys)?;
                return Ok(instantiate_generic_call(generic_func, arg_tys)?.return_ty);
            }
            // Part 2b of #1142 (#1164): a buffer-returning function is
            // callable from the CPython host through its generated wrapper
            // and from nowhere else. `crate::buffer`'s own diagnostic
            // carries the reason; refusing here is what keeps
            // `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView`
            // panic unreachable from source.
            if let Some((_, Ty::MemoryView)) = env.lookup_function(callee) {
                return Err(crate::buffer::buffer_returning_call_unsupported(callee));
            }
            let Some((param_tys, return_ty)) = env.lookup_function(callee) else {
                // Issue #142: before falling back to T0021 ("call to undefined
                // function"), check whether `callee` is a known Python 3.14
                // callable builtin that this compiler version does not implement
                // (e.g. `ValueError`, `Exception`, `int`, `range`). Such a call
                // is valid Python -- the builtin genuinely exists -- so it is a
                // capability gap (`C0001`), not a name-resolution failure
                // (`T0021`). This check is deliberately *after* the user-defined
                // function lookup, the `print`/`len`/`float` special cases, the
                // stdlib-qualified symbol lookup, the class-instantiation lookup,
                // and the generic-function lookup, so a user `def
                // ValueError(...)` always takes priority over this classification.
                // Part 2a of #1142 (#1165): the buffer producer. Placed
                // here, and not early like `len`'s own interception, so
                // D-244's #1129 statement (h) falls out of the ordering
                // rather than needing a second precedence rule: the class
                // table, the generic table and the user-function table have
                // all been consulted above, so a program's own `class
                // ndarray` or `def NDArray` keeps its meaning and never
                // reaches this line.
                //
                // A refusal rather than a type: `ndarray(n)` is admitted
                // only as an assignment's whole right-hand side, which
                // `crate::buffer_producer_assignment` handles before the
                // value ever reaches this walk. Every other position --
                // including the bare `HirStmt::ExprStmt` whose arm discards
                // the inferred type, and which would otherwise leak one
                // allocation per call -- arrives here.
                //
                // The alias gate is statement (h)'s fifth arm, in the
                // position `is_local` holds further above: `import math as
                // ndarray` binds the spelling to the `math` module, and a
                // module alias lives in no table the lookups above consult,
                // so without it this line refused the program's *own* call
                // with the producer's position message. Declining leaves the
                // `T0021` below, which is what an aliased spelling that is
                // not a producer (`import math as m`, then `m(4)`) already
                // reports. `crate::buffer::producer_assignment_ty` owns the
                // full reason; `docs/TYPE_SYSTEM.md`'s `memoryview` row is
                // the canonical enumeration.
                if crate::buffer::is_producer_spelling(callee)
                    && !env
                        .std_module_aliases
                        .iter()
                        .any(|(alias, _)| alias == callee)
                {
                    return Err(if env.in_function_body {
                        crate::buffer::producer_position_unsupported(callee)
                    } else {
                        crate::buffer::producer_at_module_scope(callee)
                    });
                }
                // Part 1 of #1319: the `frozenset(...)` builtin, placed here
                // for the same statement-(h) reason as the buffer producer
                // above -- the class, generic and user-function tables have
                // all been consulted, so a program's own `class frozenset`
                // or `def frozenset` keeps its meaning.
                if callee == crate::frozenset::FROZENSET {
                    return crate::frozenset::check_call(arg_tys);
                }
                // #1331: `hash(...)`, placed here for `frozenset`'s reason
                // directly above. It also yields to a stdlib module alias
                // spelled `hash` (`import math as hash`), which lives in no
                // table consulted above; that call keeps the known-builtin
                // `C0001` below instead of being typed as the builtin.
                if callee == crate::hash::HASH
                    && !env
                        .std_module_aliases
                        .iter()
                        .any(|(alias, _)| alias == callee)
                {
                    return crate::hash::check_call(arg_tys, env);
                }
                // Part 11 of #1371: `type(o)` on a CPython object, placed
                // here for `frozenset`'s reason above (`foreign::type_call`).
                // A bare narrowed `object` read is the object itself
                // (#1476), as `pycc_mir`'s split reads it.
                let narrowed_object = matches!(&args[..], [arg]
                    if crate::narrow::is_bare_narrowed_object_read(env, arg));
                if crate::foreign::type_call::is_object_type_call(
                    &env.std_module_aliases,
                    callee,
                    if narrowed_object {
                        &[Ty::Object]
                    } else {
                        arg_tys
                    },
                ) {
                    return Ok(Ty::Object);
                }
                if is_known_callable_builtin(callee) {
                    return Err(unsupported_callable_builtin(callee));
                }
                return Err(Diagnostic::error(
                    "T0021",
                    format!("call to undefined function `{callee}`"),
                    Span::new(0, 0),
                ));
            };
            // Issue #22: in top-level code, a call to a function whose
            // `def` has not been encountered yet in source order is a
            // static error -- CPython raises `NameError` at runtime for
            // the same case. Function bodies are exempt (all functions
            // are marked defined in `child_for_function`) because Python
            // evaluates a function body at call time, by which point all
            // module-level `def`s have typically executed.
            if !env.defined_functions.contains(callee.as_str()) {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot call function `{callee}` before its definition \
                         (NameError in CPython: name '{callee}' is not defined)"
                    ),
                    Span::new(0, 0),
                ));
            }
            if arg_tys.len() != param_tys.len() {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "`{callee}` expects {} argument(s), got {}",
                        param_tys.len(),
                        arg_tys.len()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("pass exactly {} argument(s)", param_tys.len())));
            }
            for (i, (arg_ty, param_ty)) in arg_tys.iter().zip(param_tys.iter()).enumerate() {
                if !class::is_assignable_env(env, arg_ty, param_ty)
                    && !crate::object_box::admits_value(env, &args[i], arg_ty, param_ty)?
                {
                    // #380 (PR-20): if the mismatch involves a protocol,
                    // produce a detailed T0046 conformance error.
                    let diag = if matches!(param_ty, Ty::Protocol(_)) || matches!(arg_ty, Ty::Protocol(_)) {
                        class::assignable_error(env, arg_ty, param_ty)
                    } else {
                        Diagnostic::error(
                            "T0021",
                            format!(
                                "argument {} of `{callee}` expects `{}`, got `{}`",
                                i + 1,
                                param_ty.name(),
                                arg_ty.name()
                            ),
                            Span::new(0, 0),
                        ).with_help(format!("pass a `{}` value", param_ty.name()))
                    };
                    return Err(diag);
                }
                // Part 1 of #1333: an `object` argument that got this far
                // met a parameter it is assignable to -- an unannotated
                // private helper's solver-inferred `object` parameter -- and
                // is passed through as a borrowed pointer.
            }
            Ok(return_ty.clone())
        }
        // #1021: an empty `[]`/`{}` whose element type the empty-container
        // pre-pass resolved before this walk started. The resolved type is
        // *not* trusted blindly: it goes through the same
        // `pycc_hir::check_container_ty` gate a written annotation does
        // (D-228), so an inferred `list[str]` is `T0034` and an inferred
        // `dict[int, int]` is `T0036`, exactly as the written forms are.
        HirExpr::EmptyList(element) => {
            let list_ty = Ty::List(Box::new(element.clone()));
            pycc_hir::check_container_ty(&list_ty, Span::new(0, 0))?;
            Ok(list_ty)
        }
        HirExpr::EmptyDict(pair) => {
            let dict_ty = Ty::Dict(pair.clone());
            pycc_hir::check_container_ty(&dict_ty, Span::new(0, 0))?;
            Ok(dict_ty)
        }
        // Part 2d of #1371: a list display the empty-container pre-pass
        // resolved to an object slot, built as a CPython `list`.
        HirExpr::ObjectList(elements) => {
            crate::foreign::list_display::object_list_ty(env, local_names, elements)
        }
        HirExpr::ListLiteral(elements) => {
            // #1021: reaching this arm with no elements means the
            // empty-container pre-pass (`crate::empty_container`) found no
            // evidence for an element type -- no annotation, no existing
            // binding, no `.append()` producer -- or the literal is in a
            // position that has no binding to resolve against at all
            // (`f([])`, `return []`, `[[]]`). Every resolvable case was
            // already rewritten into `HirExpr::EmptyList` above. `T0003`
            // ("untyped empty container needs annotation") is the code
            // registered for exactly this, so this no longer borrows
            // `T0021`, and it no longer points at #927: the annotated form
            // is handled now.
            if elements.is_empty() {
                return Err(crate::empty_container::unresolved_list(env.in_function_body));
            }
            let mut elem_ty: Option<Ty> = None;
            for element in elements {
                let this_ty = infer_expr_in(env, local_names, element)?;
                match &elem_ty {
                    None => elem_ty = Some(this_ty),
                    Some(expected) if *expected == this_ty => {}
                    Some(expected) => {
                        // Exact `Ty` equality, not `is_assignable`'s
                        // bool-is-an-int-subtype rule used elsewhere in this
                        // file -- D-105 requires every element to share the
                        // *exact same* `Ty`, so `[1, True]` is T0032 even
                        // though a bare `bool` is assignable to `int`.
                        return Err(Diagnostic::error(
                            "T0032",
                            format!(
                                "list element type mismatch: expected {} (from the first element), found {}",
                                expected.name(),
                                this_ty.name()
                            ),
                            Span::new(0, 0),
                        ).with_help(format!("use a `{}` value here", expected.name())));
                    }
                }
            }
            let elem_ty = elem_ty.expect("checked non-empty above");
            // This is the one place a list literal's element type becomes
            // known with a real source construct behind it (D-105's
            // Consequences) -- deliberately placed here rather than as a
            // separate pre-codegen pass. Everything above this gate (the
            // homogeneity check) is fully generic over any scalar `Ty`; a
            // future PR widening codegen to e.g. `list[str]` only has to
            // relax this one check.
            let list_ty = Ty::List(Box::new(elem_ty));
            // The gate itself lives in `pycc_hir` (D-228): lowering a written
            // `list[T]` annotation builds the same `Ty` one crate lower down
            // and has to reject exactly the same shapes. `Span::new(0, 0)` is
            // what this call site passed before the gate moved, kept so the
            // rendered diagnostic is unchanged.
            pycc_hir::check_container_ty(&list_ty, Span::new(0, 0))?;
            Ok(list_ty)
        }
        // PR-11 Task 3 (D-123): mirrors `ListLiteral`'s own homogeneity
        // check above, extended to a key/value pair, plus a `dict[str,
        // int]`-only gate mirroring `ListLiteral`'s own `T0034` gate
        // (D-122: "exactly one combination gets real codegen").
        HirExpr::DictLiteral(pairs) => {
            // #1021: same reasoning as `ListLiteral`'s own empty arm above --
            // the pre-pass already rewrote every resolvable `{}` into
            // `HirExpr::EmptyDict`, so reaching here means no evidence
            // exists.
            let Some((first_key, first_value)) = pairs.first() else {
                return Err(crate::empty_container::unresolved_dict(env.in_function_body));
            };
            let key_ty = infer_expr_in(env, local_names, first_key)?;
            let val_ty = infer_expr_in(env, local_names, first_value)?;
            for (key, value) in &pairs[1..] {
                let this_key_ty = infer_expr_in(env, local_names, key)?;
                let this_val_ty = infer_expr_in(env, local_names, value)?;
                // Exact `Ty` equality on both key and value, not
                // `is_assignable`'s bool-is-an-int-subtype rule -- same
                // reasoning as `ListLiteral`'s own homogeneity check.
                if this_key_ty != key_ty || this_val_ty != val_ty {
                    return Err(Diagnostic::error(
                        "T0035",
                        format!(
                            "dict entry type mismatch: expected {}: {} (from the first pair), found {}: {}",
                            key_ty.name(),
                            val_ty.name(),
                            this_key_ty.name(),
                            this_val_ty.name(),
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("use a `{}` key and `{}` value here", key_ty.name(), val_ty.name())));
                }
            }
            let dict_ty = Ty::Dict(Box::new((key_ty, val_ty)));
            // Shared with annotation lowering -- see the `ListLiteral` arm.
            pycc_hir::check_container_ty(&dict_ty, Span::new(0, 0))?;
            Ok(dict_ty)
        }
        // PR-11 Task 7 (D-123): mirrors `ListLiteral`'s own homogeneity
        // check above, for a single-element-type container (no key/value
        // pair), plus a `set[int]`-only gate mirroring `ListLiteral`'s own
        // `T0034` gate (D-122: "exactly one combination gets real codegen").
        // Unlike `DictLiteral` above, the empty-literal branch below is
        // unreachable from any real Python source: `{}` always parses as an
        // empty *dict* (Python has no empty-set literal spelling at all --
        // `set()` is a call, not a literal), so this only fires for a
        // hand-built `HirExpr::SetLiteral(vec![])` (e.g. the crate root's
        // own unit tests, in `tests.rs`).
        HirExpr::SetLiteral(elements) => {
            let Some(first) = elements.first() else {
                return Err(Diagnostic::error(
                    "T0021",
                    "an empty set literal's element type cannot be inferred from the literal alone -- inferring it from a `set[T]` annotation is not supported yet (issue #927)".to_string(),
                    Span::new(0, 0),
                ));
            };
            let elem_ty = infer_expr_in(env, local_names, first)?;
            for element in &elements[1..] {
                let this_ty = infer_expr_in(env, local_names, element)?;
                // Exact `Ty` equality, not `is_assignable`'s
                // bool-is-an-int-subtype rule -- same reasoning as
                // `ListLiteral`'s own homogeneity check.
                if this_ty != elem_ty {
                    return Err(Diagnostic::error(
                        "T0037",
                        format!(
                            "set element type mismatch: expected {} (from the first element), found {}",
                            elem_ty.name(),
                            this_ty.name(),
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("use a `{}` value here", elem_ty.name())));
                }
            }
            let set_ty = Ty::Set(Box::new(elem_ty.clone()));
            // Shared with annotation lowering -- see the `ListLiteral` arm.
            pycc_hir::check_container_ty(&set_ty, Span::new(0, 0))?;
            // #1343: an instance element's class verdict (T0054/C0001).
            crate::set_element::check_set_element(&elem_ty, env)?;
            Ok(set_ty)
        }
        // PR-11b Task 3 (D-116): unlike `ListLiteral`/`DictLiteral`/
        // `SetLiteral`'s homogeneity checks, this arm allows *any* mix of
        // accepted element types -- heterogeneity is tuple's own defining
        // feature. The gate is per-element type membership (int/bool/float
        // only), not agreement with a first element's type.
        HirExpr::TupleLiteral(elements) => {
            if elements.is_empty() {
                return Err(Diagnostic::error(
                    "T0021",
                    "an empty tuple literal's element types cannot be inferred from the literal alone -- inferring them from a `tuple[...]` annotation is not supported yet (issue #927)".to_string(),
                    Span::new(0, 0),
                ));
            }
            let mut elem_tys = Vec::with_capacity(elements.len());
            for element in elements {
                let this_ty = infer_expr_in(env, local_names, element)?;
                // Per-element, *inside* the loop, not a whole-`Ty` postcheck
                // like the three arms above: the elements are gated in source
                // order, so an earlier element's type gate is reported ahead
                // of a later element's own inference failure (an undefined
                // name, say). `pycc_hir` exposes the element-shaped entry
                // point for exactly this reason (D-228).
                pycc_hir::check_tuple_element_ty(&this_ty, Span::new(0, 0))?;
                elem_tys.push(this_ty);
            }
            Ok(Ty::Tuple(Box::new(elem_tys)))
        }
        HirExpr::Subscript { base, index } => {
            // PEP 560 (#610): `C[x]` where `C` is a bare class name is
            // `C.__class_getitem__(x)`, not a container index. The base is
            // `HirExpr::Name` referring to a registered class, exactly as in
            // the `MethodCall` arm's own `ClassName.static_method(args)`
            // interception -- and, like that arm, it must run before the
            // ordinary base inference, which has no type for a bare class
            // name and would reject it as an undefined name.
            //
            // Unlike that arm, this one also requires the name to be unbound
            // as a value: `class C: ...` followed by `C = [1, 2, 3]` is
            // accepted (only *type aliases* collide with a class name, see
            // `pycc_hir`'s own collision check), and `C[0]` must then read
            // the list element, not dispatch a class hook. `pycc_mir`'s own
            // `Subscript` arm applies the identical guard, so both crates
            // agree on which of the two `C[0]` means.
            //
            // A class that defines no `__class_getitem__` anywhere in its
            // MRO is rejected by `resolve_static_or_class_method_call`'s own
            // unknown-member diagnostic, which names the class and the
            // missing hook -- CPython raises `TypeError: type 'C' is not
            // subscriptable` for the same program.
            if let HirExpr::Name(class_name) = base.as_ref()
                && class_name_dispatch(env, local_names, class_name)?
            {
                let index_ty = infer_expr_in(env, local_names, index)?;
                return class::resolve_static_or_class_method_call(
                    env,
                    class_name,
                    "__class_getitem__",
                    std::slice::from_ref(index.as_ref()),
                    &[index_ty],
                );
            }
            // Part 2 of #1027: `b[i]` on a name bound to a `memoryview` is a
            // native `float` element load, the one read of such a name this
            // compiler admits. Like the PEP 560 interception above, it must
            // run *before* the ordinary base inference -- `infer_expr_in`'s
            // own `Name` arm calls `reject_memoryview_read`, so inferring the
            // base first would report the `C0001` capability gap for the very
            // expression that closes it.
            //
            // The binding is therefore read raw, through `binding_state`,
            // rather than through `lookup_bound_name`, which calls that same
            // refusal. `Definitely` and not `BindingState::ty()`: a
            // possibly-unbound name is left to the ordinary path, which
            // reports the unbound-local diagnostic it already has.
            //
            // Only a bare `HirExpr::Name` base is intercepted, which is the
            // whole of the admitted surface -- a `memoryview` cannot be
            // aliased, stored or returned, so no other expression can have
            // the type. Part 2a of #1142 (#1165) added the one producing
            // expression and does not widen this: `ndarray(n)` is admitted
            // only as an assignment's whole right-hand side, so
            // `ndarray(4)[0]` is refused by the `Call` arm's own named
            // position diagnostic before this seam ever sees it. `b[i][j]`
            // therefore still falls through to the `Ty::Float` catch-all
            // below and is refused with `T0033`.
            if let HirExpr::Name(buffer_name) = base.as_ref()
                && matches!(
                    env.binding_state(buffer_name),
                    Some(BindingState::Definitely(Ty::MemoryView))
                )
            {
                let index_ty = infer_expr_in(env, local_names, index)?;
                // Reuses T0021 and `is_assignable` for exactly the reasons
                // the `Ty::List` arm below records: a non-int-compatible
                // index is that same operand mismatch, and D-086 admits
                // `bool` wherever `int` is expected.
                if !is_assignable(index_ty.clone(), Ty::Int) {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "`memoryview` index must be `int`, found `{}`",
                            index_ty.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help("use an `int` value"));
                }
                return Ok(Ty::Float);
            }
            let base_ty = infer_expr_in(env, local_names, base)?;
            let index_ty = infer_expr_in(env, local_names, index)?;
            match base_ty {
                Ty::List(elem_ty) => {
                    // Reuses T0021 (an unconstrained/conflicting-constraint
                    // shape), not a new code -- a non-int-compatible index is
                    // that same "operand type mismatch" failure, not a
                    // distinct one.
                    //
                    // Uses `is_assignable`, not exact `Ty` equality: D-086
                    // already established that `bool` is accepted wherever
                    // `int` is expected at an operand boundary (mirroring
                    // `is_assignable`'s own existing param/assignment rule),
                    // and indexing is exactly that kind of boundary --
                    // `xs[True]` is ordinary, CPython-valid Python (`bool` is
                    // an `int` subtype, PEP 285), not a type error.
                    // `pycc_codegen`'s `to_numeric_encoded_int` already has a
                    // `Scalar::Bool` arm reached unconditionally by every
                    // subscript index, so this was a pure over-rejection in
                    // the type checker, not a missing codegen capability.
                    if !is_assignable(index_ty.clone(), Ty::Int) {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!("list index must be `int`, found `{}`", index_ty.name()),
                            Span::new(0, 0),
                        ).with_help("use an `int` value"));
                    }
                    Ok(*elem_ty)
                }
                // PR-11 Task 3 (D-123): `d[k]` read. Uses exact `Ty`
                // equality on the key, not `is_assignable` -- every
                // `Ty::Dict` value that survives `DictLiteral`'s own
                // `T0036` gate above has key type exactly `Ty::Str` (no
                // other combination reaches codegen, and no other source
                // construct can produce a `Ty::Dict` value at all), so
                // there is no bool/int-style widening question here, unlike
                // the list-index case above.
                Ty::Dict(kv) => {
                    let (key_ty, val_ty) = *kv;
                    if index_ty != key_ty {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!(
                                "dict key type mismatch: expected `{}`, found `{}`",
                                key_ty.name(),
                                index_ty.name()
                            ),
                            Span::new(0, 0),
                        ).with_help(format!("use a `{}` value here", key_ty.name())));
                    }
                    Ok(val_ty)
                }
                // PR-11b Task 3 (D-116): `t[k]` requires `k` to be a
                // literal, non-negative, in-bounds integer -- not merely
                // `int`-typed, unlike `List`'s case above. A heterogeneous
                // tuple's element type at position `k` is only knowable
                // when `k` is known at compile time, so every failure shape
                // (non-literal, negative, out-of-range) shares one code.
                Ty::Tuple(elems) => {
                    let HirExpr::IntLiteral(literal_index) = index.as_ref() else {
                        return Err(Diagnostic::error(
                            "T0040",
                            "tuple index must be a non-negative literal integer within range"
                                .to_string(),
                            Span::new(0, 0),
                        ).with_help("use a literal, non-negative integer index within range"));
                    };
                    let Ok(literal_index) = usize::try_from(*literal_index) else {
                        return Err(Diagnostic::error(
                            "T0040",
                            "tuple index must be a non-negative literal integer within range"
                                .to_string(),
                            Span::new(0, 0),
                        ).with_help("use a literal, non-negative integer index within range"));
                    };
                    let Some(elem_ty) = elems.get(literal_index) else {
                        return Err(Diagnostic::error(
                            "T0040",
                            "tuple index must be a non-negative literal integer within range"
                                .to_string(),
                            Span::new(0, 0),
                        ).with_help("use a literal, non-negative integer index within range"));
                    };
                    Ok(elem_ty.clone())
                }
                // Part 3 of #1026 (PR 3b of #1082): `o[k]` on a foreign
                // CPython object. Placed before the `other` catch-all,
                // which reported the `T0033` ("`object` does not support
                // indexing") this branch replaces, and after both operands
                // have had their ordinary inference -- an index that is
                // itself an unsupported operation must report its own
                // diagnostic rather than this one, exactly as the
                // `MethodCall` arm below orders the same two concerns.
                //
                // Only an operand the shim can box is a key
                // (`foreign::is_object_operand`: the four scalars, since
                // Part 2a of #1371 a second `object`, and since #1470 an
                // instance of a regular user class, which crosses as its
                // carrier and hashes through the class's `__hash__`): each
                // has a `pycc_ext_obj_pack_*` helper in the shim, and the
                // packer contract is the same one a call's arguments use.
                // Anything else -- a container, an enum member, an
                // exception instance or `None` -- has no boundary
                // representation yet and is refused here rather than
                // reaching codegen.
                //
                // The *result* is `Ty::Object`: pycc knows nothing about
                // what `o[k]` really is, exactly as it knows nothing about
                // `o.attr` or `o.m()`. `o[k] = v` is deliberately not part
                // of this -- a store target is a separate HIR shape with
                // its own pre-existing refusal (`C0001`), and
                // `docs/TYPE_SYSTEM.md`'s `object` row records the
                // asymmetry.
                Ty::Object => {
                    crate::foreign::refuse_classmethod_cls(
                        env,
                        index,
                        "indexing a CPython object with a class method's `cls`",
                    )?;
                    if !crate::foreign::is_object_operand(env, &index_ty) {
                        return Err(crate::foreign::object_operation_unsupported(&format!(
                            "indexing a CPython object with a `{}` key",
                            index_ty.name()
                        )));
                    }
                    Ok(Ty::Object)
                }
                // PR-11 Task 7 (D-123): `Ty::Set` deliberately has no
                // explicit arm here and falls through to this rejection --
                // real Python sets are not subscriptable either (`s[0]`
                // raises `TypeError` in CPython too), so this is not a v0.2
                // scope cut needing its own arm, it is the semantically
                // correct behavior for every other base type as well.
                other => Err(Diagnostic::error(
                    "T0033",
                    format!("`{}` does not support indexing", other.name()),
                    Span::new(0, 0),
                )),
            }
        }
        // PR-12 Task 7 (D-118): `base[start:stop:step]` read. Only
        // `list[int]` ships slicing in v0.2 -- every other base (including
        // `Ty::Dict`/`Ty::Set`, which real CPython also rejects for `[i:j]`,
        // and `Ty::Tuple`, an explicit deferral rather than a "never
        // supported" case) reuses the same `T0033` code `Subscript`'s own
        // `other` fallthrough above already established for non-indexable
        // bases, not a new one. A `list[T]` with `T != Ty::Int` reuses
        // `ListLiteral`'s own `T0034` ("only list[int] is compiled") gate.
        // Diagnostic order is deliberately base-type (`T0033`) before
        // element-type (`T0034`) before any bound's own type (`T0021`),
        // mirroring this file's existing "callee/base-type errors before
        // argument errors" convention (D-110's callee-first precedent,
        // applied here to base-type-before-bound-type) -- pinned by
        // `slicing_reports_the_base_type_error_before_any_bound_error` and
        // `slicing_reports_the_element_type_error_before_any_bound_error`
        // in `tests.rs`.
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            let base_ty = infer_expr_in(env, local_names, base)?;
            // Part 2b of #1371: a CPython object base is CPython's own
            // `PyObject_GetItem` with a `slice` key (`foreign::slice`).
            if base_ty == Ty::Object {
                return crate::foreign::slice::object_slice_ty(
                    env,
                    local_names,
                    [start.as_deref(), stop.as_deref(), step.as_deref()],
                );
            }
            let Ty::List(elem_ty) = &base_ty else {
                return Err(Diagnostic::error(
                    "T0033",
                    format!(
                        "`{}` does not support slicing (only list[int] does)",
                        base_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            };
            if **elem_ty != Ty::Int {
                return Err(Diagnostic::error(
                    "T0034",
                    format!(
                        "list codegen only supports `list[int]` in v0.2, cannot slice `list[{}]`",
                        elem_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            // Each bound is independently optional (`xs[:]`, `xs[1:]`,
            // `xs[:3]`, `xs[::2]` all parse) and, when present, an
            // arbitrary runtime `int`-typed expression -- not a literal-only
            // check -- so `is_assignable` (not exact `Ty` equality) applies
            // here, same as `Subscript`'s own index check above: `xs[True:]`
            // is ordinary, CPython-valid Python (`bool` is an `int`
            // subtype, PEP 285), not a type error. `step`'s runtime
            // positivity is deliberately not validated here -- it can't be,
            // for a non-literal runtime expression, at compile time; that
            // check is a later task's job (D-118).
            for (label, bound) in [("start", start), ("stop", stop), ("step", step)] {
                if let Some(bound) = bound {
                    let bound_ty = infer_expr_in(env, local_names, bound)?;
                    if !is_assignable(bound_ty.clone(), Ty::Int) {
                        return Err(Diagnostic::error(
                            "T0021",
                            format!("slice {label} must be `int`, got `{}`", bound_ty.name()),
                            Span::new(0, 0),
                        ).with_help("use an `int` value"));
                    }
                }
            }
            Ok(base_ty.clone())
        }
        HirExpr::ListAppend { list, value } => {
            let list_ty = infer_container_receiver(env, local_names, list, ".append()")?;
            let Ty::List(elem_ty) = &list_ty else {
                return Err(Diagnostic::error(
                    "T0033",
                    format!("`{}` does not support `.append()`", list_ty.name()),
                    Span::new(0, 0),
                ));
            };
            let value_ty = infer_expr_in(env, local_names, value)?;
            // Uses `is_assignable`, not exact `Ty` equality -- matching the
            // `Subscript` index check above (D-086): `x = [1]; x.append(True)`
            // is ordinary, CPython-valid Python (`bool` is an `int` subtype),
            // and `pycc_codegen`'s `MirExpr::ListAppend` arm already routes
            // the appended value through `to_encoded_int`, preserving a
            // `Scalar::Bool` marker while validating the int-compatible
            // payload unconditionally, so there is no missing codegen
            // capability here either. This is NOT the same question as `ListLiteral`'s
            // own homogeneity check above (`[1, True]`'s element type is
            // genuinely ambiguous to infer -- there is no already-known
            // `elem_ty` to check against); `.append()` on an *already-typed*
            // `list[int]` has no such ambiguity, so the looser
            // `is_assignable` rule applies here, not there. Reuses T0021 (a
            // call-site/assignment constraint mismatch), not a new code.
            if !is_assignable(value_ty.clone(), (**elem_ty).clone()) {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot append `{}` to a list of `{}`",
                        value_ty.name(),
                        elem_ty.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", elem_ty.name(), value_ty.name())));
            }
            Ok(Ty::None)
        }
        // PR-12 Task 10 (D-119): `list.pop()`. Unlike `ListAppend`, this
        // arm's result is the list's own element type, not `Ty::None` --
        // `.pop()` is meant to be used for its value.
        HirExpr::ListPop { list } => {
            let list_ty = infer_container_receiver(env, local_names, list, ".pop()")?;
            let Ty::List(elem_ty) = &list_ty else {
                return Err(Diagnostic::error(
                    "T0033",
                    format!("`{}` does not support `.pop()`", list_ty.name()),
                    Span::new(0, 0),
                ));
            };
            Ok((**elem_ty).clone())
        }
        // PR-12 Task 10 (D-119): `dict.get(key, default)`. The key check
        // uses exact `Ty` equality, not `is_assignable` -- same reasoning as
        // `Subscript`'s own `Ty::Dict` read arm and `check_dict_set` above
        // (every `Ty::Dict` value that survives `DictLiteral`'s own `T0036`
        // gate has key type exactly `Ty::Str`, so there is no bool/int-style
        // widening question for a dict *key*, unlike a list index or a
        // dict *value*). The `default` check does use `is_assignable`,
        // mirroring `ListAppend`'s/`check_dict_set`'s own value-position
        // leniency (D-086 `bool`-subtypes-`int`). The overall result is the
        // dict's *value* type, never `Ty::None`, since a missing key still
        // yields the (same-typed) default rather than `None`.
        HirExpr::DictGetOrDefault { dict, key, default } => {
            let dict_ty = infer_container_receiver(env, local_names, dict, ".get()")?;
            let Ty::Dict(kv) = &dict_ty else {
                return Err(Diagnostic::error(
                    "T0033",
                    format!("`{}` does not support `.get()`", dict_ty.name()),
                    Span::new(0, 0),
                ));
            };
            let key_ty = infer_expr_in(env, local_names, key)?;
            if key_ty != kv.0 {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot look up a `{}` key in a dict of `{}` keys",
                        key_ty.name(),
                        kv.0.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", kv.0.name(), key_ty.name())));
            }
            let default_ty = infer_expr_in(env, local_names, default)?;
            if !is_assignable(default_ty.clone(), kv.1.clone()) {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot use a `{}` default for a dict of `{}` values",
                        default_ty.name(),
                        kv.1.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", kv.1.name(), default_ty.name())));
            }
            Ok(kv.1.clone())
        }
        // PR-12 Task 10 (D-119): `set.add(value)`. Mirrors `ListAppend`
        // exactly -- always `Ty::None`, with D-131's ordinary assignment
        // storage available when that result is bound to a name.
        HirExpr::SetAdd { set, value } => {
            let set_ty = lookup_bound_name(env, local_names, set)?;
            let Ty::Set(elem_ty) = &set_ty else {
                return Err(Diagnostic::error(
                    "T0033",
                    format!("`{}` does not support `.add()`", set_ty.name()),
                    Span::new(0, 0),
                ));
            };
            let value_ty = infer_expr_in(env, local_names, value)?;
            if !is_assignable(value_ty.clone(), (**elem_ty).clone()) {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot add `{}` to a set of `{}`",
                        value_ty.name(),
                        elem_ty.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", elem_ty.name(), value_ty.name())));
            }
            // #1343: needed for a `set[R]` no literal checked (a parameter).
            crate::set_element::check_set_element(elem_ty, env)?;
            Ok(Ty::None)
        }
        // D-154 (Part 1 of #375): an instance attribute read/method call --
        // see `class::resolve_attr_get`/`class::resolve_method_call` for the
        // actual resolution (both shared with `check_stmt`'s own
        // `HirStmt::AttrSet` arm, which needs the identical attribute-type
        // lookup for its own assigned-value check).
        HirExpr::AttrGet { base, attr } => {
            // #433: `super().attr` — resolve the attribute starting from
            // the next class in the current class's MRO, not from the
            // current class itself. The `self` instance retains its actual
            // (most-derived) type, so the slot index is still computed from
            // the full MRO's flat layout downstream.
            if matches!(base.as_ref(), HirExpr::Super) {
                return class::resolve_super_attr_get(env, attr);
            }
            // #436: `ClassName.attr` — accessing an attribute on a class
            // name (not an instance) is not supported. A static or class
            // method accessed without calling it (e.g. `C.create` instead
            // of `C.create()`) has no value representation in this
            // compiler's static-dispatch model. Reject with a clear error
            // rather than letting `infer_expr_in` on the class name
            // produce a confusing "name not defined" diagnostic.
            //
            // An active value binding of that same name wins over the class:
            // in `def f(B: D) -> int: return B.X`, `B` is the parameter, not
            // the class `B`, exactly as CPython resolves it. Guard with the
            // same binding/local pair the `Subscript` class-name path above
            // already uses, so a shadowed name falls through to the ordinary
            // instance path below.
            if let HirExpr::Name(class_name) = base.as_ref()
                && class_name_dispatch(env, local_names, class_name)?
                && let Some(class_def) = env.lookup_class(class_name)
            {
                // #379 (PR-19): `Color.RED` — accessing an enum member by
                // name on the enum class.
                if let Some(ty) = enum_member_attr_type(class_def, class_name, attr) {
                    return Ok(ty);
                }
                // Part 1 of #1284: a winning `staticmethod(<foreign
                // callable>)` class attribute reads the foreign callable.
                if let Some(ty) =
                    class::foreign_static::class_name_read(env, local_names, class_name, attr)?
                {
                    return Ok(ty);
                }
                // #911 (Part 1 of #885): `W.MIN_WIDTH` -- reading a
                // class-level attribute through the class name itself. This
                // is the *only* class-name attribute read pycc supports;
                // everything else still falls through to the `T0044` below.
                //
                // #974: the lookup walks `class_name`'s MRO rather than its
                // own `class_attrs` alone, so an *inherited* class attribute
                // (`Derived.LIMIT` where a base declares `LIMIT`) resolves,
                // as it does in CPython. It deliberately does **not** reuse
                // `lookup_class_attr_through_mro`: that walk would also
                // resolve a name a derived class re-declares as a method,
                // static method, class method or property, which CPython
                // shadows -- see `lookup_class_attr_by_class_name`. It is
                // equally deliberately *not* `resolve_attr_get`'s #960
                // instance precedence: a class object has no instance
                // `__dict__`, so an instance slot does not shadow here.
                // `pycc_mir`'s fold runs the same walk over the same shared
                // predicate; the two must never drift.
                if let Some(ty) = class::lookup_class_attr_by_class_name(env, class_name, attr) {
                    return Ok(ty);
                }
                return Err(Diagnostic::error(
                    "T0044",
                    format!(
                        "class `{class_name}` has no attribute named `{attr}` -- \
                         accessing a class attribute or method without an instance is \
                         not supported (use `instance.{attr}` or `{class_name}.{attr}()` \
                         for a static/class method)"
                    ),
                    Span::new(0, 0),
                ));
            }
            let base_ty = infer_expr_in(env, local_names, base)?;
            // Part 1 of #1284: an instance read reaching a
            // `staticmethod(<foreign callable>)` class attribute. Must run
            // before the #911 literal-fold gate below: that gate's
            // `lookup_class_attr_through_mro` also matches a `ForeignStatic`
            // entry, so reaching it first would wrongly refuse a non-name
            // receiver (`FS().exists`, #1346) as a folded constant. Also runs
            // before `resolve_attr_get`'s property walk, matching
            // `pycc_mir`'s guard placement.
            if let Ty::Instance(class_name) = &base_ty
                && let Some(ty) = class::foreign_static::instance_read(
                    env,
                    local_names,
                    class_name,
                    attr,
                )?
            {
                return Ok(ty);
            }
            // #911 (Part 1 of #885): a class-attribute read is folded to its
            // constant by `pycc_mir`, which *discards* the base expression.
            // That is only sound when evaluating the base has no observable
            // effect, so the base is restricted to a bare name (`w.LIMIT`,
            // `self.LIMIT`). `make_window().LIMIT` would otherwise drop the
            // call, so it is rejected here rather than silently mis-compiled.
            //
            // #960 left this gate deliberately coarse, exactly like
            // `check_attr_set`'s own class-attribute rejection: it fires when
            // a class attribute of that name exists anywhere in the MRO,
            // without asking whether a sibling base's instance slot shadows
            // it. For that one shape the read would no longer fold, so the
            // message's stated reason does not apply -- but the result is a
            // conservative rejection of a program CPython accepts, never a
            // mis-compile, and relaxing it belongs with the write-side
            // follow-up rather than with #960's read fix.
            if !matches!(base.as_ref(), HirExpr::Name(_))
                && let Ty::Instance(class_name) = &base_ty
                && class::lookup_class_attr_through_mro(env, class_name, attr).is_some()
            {
                return Err(Diagnostic::error(
                    "T0044",
                    format!(
                        "the class-level attribute `{class_name}.{attr}` can only be read \
                         through a plain name (`obj.{attr}`, `self.{attr}`) or through the \
                         class itself (`{class_name}.{attr}`) -- it is folded to a constant \
                         at compile time, which would discard the base expression"
                    ),
                    Span::new(0, 0),
                ));
            }
            // Part 2 of #1026 (#1081): `numpy.pi`. A CPython object has no
            // `HirClassDef`, so `class::resolve_attr_get`'s
            // `let Ty::Instance(..) else` just below would report `T0043`
            // ("not a class instance") -- a message that describes pycc's
            // own representation rather than the user's program, and a
            // user-visible regression the moment the `Name` arm above stops
            // refusing the read. The load itself is what Part 2 implements,
            // so it is admitted here, opaquely: the result is another
            // `Ty::Object`, because pycc knows nothing about the attribute's
            // real type either.
            //
            // Placed after the `Super` (#433) and `ClassName.attr` (#436)
            // guards above so neither changes meaning: both key on the
            // *syntactic* base, which a foreign read never matches.
            if matches!(base_ty, Ty::Object) {
                return Ok(Ty::Object);
            }
            class::resolve_attr_get(env, &base_ty, attr)
        }
        // Issue #1188: one of the four container method names in a module
        // that can see a user class defining it; the receiver picks the
        // reading.
        HirExpr::ReceiverDispatchedCall { call, container } => {
            receiver_dispatch::infer_receiver_dispatched_call(env, local_names, call, container)
        }
        HirExpr::MethodCall { base, method, args } => {
            // #433: `super().method(args)` — resolve the method starting
            // from the next class in the current class's MRO, with `self`
            // (the most-derived instance) as the implicit first argument.
            if matches!(base.as_ref(), HirExpr::Super) {
                let arg_tys = args
                    .iter()
                    .map(|arg| infer_expr_in(env, local_names, arg))
                    .collect::<Result<Vec<_>, _>>()?;
                return class::resolve_super_method_call(env, method, args, &arg_tys);
            }
            // Part 1 of #1284: `C.name(args)` reaching a
            // `staticmethod(<foreign callable>)` class attribute. Runs
            // before the static/class-method table walk, which is not
            // positional against class attributes (MRO case D3).
            if let HirExpr::Name(class_name) = base.as_ref()
                && class_name_dispatch(env, local_names, class_name)?
                && let Some(ty) = class::foreign_static::class_name_call(
                    env,
                    local_names,
                    class_name,
                    method,
                    args,
                )?
            {
                return Ok(ty);
            }
            // #436: `ClassName.static_method(args)` or
            // `ClassName.class_method(args)` — a method call on a class
            // name (not an instance). The base is `HirExpr::Name` referring
            // to a registered class. Check the static_methods and
            // class_methods tables before the regular instance-method
            // resolution (which requires a `Ty::Instance` base and would
            // reject a bare class name).
            // An active value binding of that name shadows the class
            // here exactly as it does for the class-name `AttrGet` arm
            // above: in `def f(B: D) -> int: return B.m()`, `B` is the
            // parameter, so the call must reach `D.m`, not `B`'s static or
            // class method. The guard runs before the method-table walk so
            // a shadowed name short-circuits straight to the ordinary
            // instance path below.
            if let Some(class_name) =
                receiver_dispatch::static_or_class_method_receiver(env, local_names, base, method)?
            {
                let arg_tys = args
                    .iter()
                    .map(|arg| infer_expr_in(env, local_names, arg))
                    .collect::<Result<Vec<_>, _>>()?;
                return class::resolve_static_or_class_method_call(
                    env, class_name, method, args, &arg_tys,
                );
            }
            let base_ty = infer_expr_in(env, local_names, base)?;
            let arg_tys = args
                .iter()
                .map(|arg| infer_expr_in(env, local_names, arg))
                .collect::<Result<Vec<_>, _>>()?;
            // Part 2 of #1026 (PR 2b of #1081): a method call on a foreign
            // CPython object. Placed after `base_ty`/`arg_tys` are computed
            // -- both still need their ordinary inference, and an argument
            // that is itself an unsupported operation must report its own
            // diagnostic rather than this one -- and before
            // `resolve_method_call`, whose `Ty::Object` base reports the
            // `T0043` ("not a class instance") that this branch replaces.
            //
            // Only the packable operands can be marshalled: the four
            // scalars, another `Ty::Object` (Part 2a of #1371) and an
            // instance of a regular user class (#1435), each with a
            // `pycc_ext_obj_pack_*` helper in the shim, and `None` (Part 8
            // of #1371), passed as CPython's own `Py_None`. Anything else
            // -- a container, an enum member -- has no boundary
            // representation yet and is refused here rather than reaching
            // codegen.
            if matches!(base_ty, Ty::Object) {
                crate::foreign::check_object_call_args(env, args, &arg_tys, "method")?;
                return Ok(Ty::Object);
            }
            // Part 1 of #1284: the instance form of the call above, placed
            // before the static/class-method tables for the same reason.
            if let Ty::Instance(ref class_name) = base_ty
                && let Some(ty) = class::foreign_static::instance_call(
                    env,
                    local_names,
                    class_name,
                    method,
                    args,
                )?
            {
                return Ok(ty);
            }
            // #436: static and class methods can also be called on an
            // instance. Check the static/class method tables before the
            // regular instance-method resolution.
            if let Ty::Instance(ref class_name) = base_ty
                && class::has_static_or_class_method(env, class_name, method)
            {
                return class::resolve_static_or_class_method_call(
                    env, class_name, method, args, &arg_tys,
                );
            }
            class::resolve_method_call(env, &base_ty, method, args, &arg_tys)
        }
        // PEP 695 (#387): `C[type_arg](args)` — a generic class
        // instantiation. The result type is `Ty::Instance(class)` — the
        // type argument is a compile-time scalar substitution, not a
        // runtime value, so it does not affect the result's nominal type
        // (the class instance). The actual monomorphization (substituting
        // `T` with `type_arg` in the class's methods) happens later in
        // `pycc_types`' `instantiate_generic_call` / `monomorphize`
        // pipeline, reusing PR-13's generic-function infrastructure.
        // Part 2a of #1371: `table[k](args)`, a call of a subscript result.
        HirExpr::ExprCall { callee, args } => {
            crate::foreign::subscript_call::infer_expr_call(env, local_names, callee, args)
        }
        // Part 8 of #1371: a keyword call the binder could not bind.
        HirExpr::KeywordCall {
            call,
            keywords,
            span,
        } => crate::foreign::keyword_call::infer_keyword_call(
            env,
            local_names,
            call,
            keywords,
            *span,
        ),
        HirExpr::ReceiverClassCall { args } => {
            class::infer_receiver_class_call(env, local_names, args)
        }
        HirExpr::GenericClassInstantiate { class, .. } => {
            // Verify the class exists. Genericity (the class has a type
            // parameter) is checked later during monomorphization's rewrite
            // pass (`rewrite_generic_calls_in_expr`), which rejects a
            // non-generic class used with `C[int](args)` with T0042.
            if !env.classes.contains_key(class) {
                return Err(Diagnostic::error(
                    "T0001",
                    format!("class `{class}` is not defined"),
                    Span::new(0, 0),
                ));
            }
            Ok(Ty::Instance(Box::new(class.to_string())))
        }
        // #433: a bare `HirExpr::Super` should never reach `infer_expr_in`
        // — HIR lowering rejects a standalone `super()` with C0001, and
        // `super().method()`/`super().attr` are handled by the `MethodCall`/
        // `AttrGet` arms below (which special-case a `Super` base before
        // recursing into `infer_expr_in` for it). This arm is a defense-in-
        // depth guard for a hand-built HIR that bypasses `lower_expr`'s own
        // rejection.
        HirExpr::Super => Err(Diagnostic::error(
            "C0001",
            "a bare `super()` expression is not supported — use `super().method()` or `super().attr`".to_string(),
            Span::new(0, 0),
        )),
        // PEP 572 (#774): `target := value`'s own type is simply `value`'s
        // type -- mirroring an assignment statement's own RHS typing rule.
        // The persistent binding of `name` into the *enclosing* scope's
        // `Environment` cannot happen here: this function only ever holds a
        // shared `&Environment` reference, never a mutable one. That
        // mutation instead happens one level up, at the statement boundary
        // that calls into this expression (`crate::collect_named_expr_bindings`,
        // invoked from `check_stmt`/`check_stmt_in_function` before the
        // statement's own body/orelse/next-statement checking, so a name
        // bound by a walrus in an `if`/`while` test or a bare expression
        // statement is visible to every statement that follows it in the
        // same scope) -- this arm only ever reports the type, never binds.
        // #774's own scope note: codegen's `MirExpr::NamedExpr` yields the
        // stored value by re-reading the slot it was just stored into (the
        // "assignment followed by a name-load" shape the issue calls for),
        // exactly mirroring a plain `Name` read's own refcount contract. That
        // contract is only worked out (`pycc_codegen::bigint_rc`'s
        // `int_value_is_a_duplicate_reference`) for `Ty::Int`; a
        // reference-counted `Ty::Str`/`Ty::List`/`Ty::Dict`/`Ty::Set`/
        // `Ty::Tuple`/`Ty::Instance`/`Ty::Protocol` value would need the same
        // "yielded value borrows the slot's reference" treatment
        // `str_value_is_a_duplicate_reference` and its container/instance
        // counterparts don't yet carry for this new node, and getting that
        // wrong silently reproduces the exact D-154 Part 1 use-after-free
        // class already fixed once for plain reads. Rather than widen every
        // one of those classifiers for a PR whose own fixture never exercises
        // them, T0050 restricts a walrus's value to the non-reference-counted
        // scalar types (`int`, `float`, `bool`, `None`, and `Optional` of
        // those) up front -- a `core` gap recorded in the conformance-breadth
        // manifest, not a silent one, and revisited only alongside a real
        // audit of every refcounted `MirExpr` classifier this crate over.
        // (T0050, not T0048/T0049: those two codes are already registered
        // for the unrelated PEP 604 general-union-annotation gap -- see
        // `pycc_diag::explain`'s own registry -- so this diagnostic takes
        // the next free code instead of colliding with them.)
        HirExpr::NamedExpr { name: _, value } => {
            let ty = infer_expr_in(env, local_names, value)?;
            if !is_walrus_value_ty_supported(&ty) {
                return Err(Diagnostic::error(
                    "T0050",
                    format!(
                        "a walrus assignment (`:=`) value of type `{}` is not yet supported — \
                         only `int`, `float`, `bool`, and `None` (including `Optional` of those) \
                         are currently allowed as a walrus value (#774)",
                        ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(ty)
        }
        HirExpr::Comprehension(comp) => {
            crate::comprehension::infer_comprehension(env, local_names, comp)
        }
    }
}

/// PEP 572 (#774): the non-reference-counted scalar types a walrus
/// assignment's value is currently permitted to have (T0050). See
/// `infer_expr_in`'s own `HirExpr::NamedExpr` arm above for why this is
/// restricted rather than handled generally for every `Ty`.
fn is_walrus_value_ty_supported(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::None => true,
        Ty::Optional(inner) => is_walrus_value_ty_supported(inner),
        Ty::Str
        | Ty::Infer
        | Ty::Param(_)
        | Ty::List(_)
        | Ty::Dict(_)
        | Ty::Set(_)
        | Ty::FrozenSet(_)
        | Ty::Tuple(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView => false,
    }
}

/// `Err(C0001)` when `name` is bound to a `memoryview`.
///
/// #1027 admits `memoryview` at exactly one position: a parameter of a
/// function exported across a `pycc build --ext` boundary, where the
/// generated wrapper acquires the buffer, proves its shape and hands the
/// compiled body a `{ ptr, len }` pair. The plan's section 3.5 states the
/// other half of that admission -- "no aliasing into a local, no
/// reassignment, no storing into a container, no passing to another
/// function" -- and this is where it is enforced.
///
/// Refusing the *read* is what makes that list closed rather than a list.
/// `memoryview` has no literal, and until Part 2a of #1142 (#1165) it had no
/// producing expression either, so the only way a value of the type could
/// reach any of those positions was through a read of its own parameter
/// name; refusing the read therefore refuses every one of them at once.
/// #1165 adds a producing expression (`crate::buffer`), and closes the
/// positions it opens on its own terms rather than through this function: a
/// producer is admitted in exactly one syntactic position -- an assignment's
/// whole right-hand side -- so the value still cannot reach a container, a
/// call argument or a return without passing through a read of the name it
/// was bound to, which arrives here. Without it `pycc_codegen` reaches a local load it has no
/// lowering for and panics (`reading a `memoryview`-typed local is not
/// supported yet`) -- an ICE where the contract calls for a diagnostic.
///
/// Part 2 of #1027 opened the first hole in that blanket refusal, and #1116
/// the second. Both do it by *interception* rather than by weakening this
/// function: `b[i]` on a `memoryview`-bound name is answered with `Ty::Float`
/// in [`infer_expr_in`]'s own `Subscript` arm (and in the solver's), and
/// `len(b)` with `Ty::Int` in its `Call` arm (and in the solver's), each
/// before the base or argument is ever inferred, so this call is never
/// reached for those two shapes. Every other read still arrives here. That is
/// why the four call sites are untouched: the set of refusals is unchanged,
/// and only the set of expressions that reach them narrowed.
///
/// `C0001` rather than a new code: this is the crate's established "valid
/// Python this compiler version does not implement yet" spelling.
///
/// "Every read" is two seams, not one. `HirStmt::ForList` and
/// `HirExpr::Comprehension` hold their iterable as a plain `String` rather than a
/// `HirExpr::Name` (D-105's HIR shape), so `for x in v` never reaches
/// `infer_expr_in`'s `Name` arm; `lib.rs`'s `lookup_bound_name` is the other
/// caller, and it calls this for the same reason it calls
/// [`crate::foreign::reject_object_read`]. Without that second call the
/// iteration is still refused, but as `T0033` -- "`memoryview` cannot be
/// iterated" -- which is false about Python and mislabels a capability gap
/// as a type error.
///
/// Part 2a of #1142 (#1165) gives the buffer type a second source, so this
/// refusal stops meaning "this type" and starts meaning "this *binding*".
/// `owned` is the caller's answer to "did this artifact allocate the storage
/// `name` is bound to?", read from the walker's own `owned_buffers` set; the
/// two walkers each answer it from their own environment because the
/// constraint solver runs first and would otherwise report the parameter
/// message for every owned read before the check phase ever looked.
///
/// The parameter arm's *prefix* is unchanged verbatim -- eight assertions pin
/// it -- while its tail was corrected: it used to assert that #1027, #1129
/// and #1142 admit a buffer only as such a parameter, which Part 2a's second
/// provenance falsified. The owned arm is [`crate::buffer::owned_buffer_use_unsupported`],
/// which names the owned case and the reason it is still refused (there is
/// nowhere for the value to go until egress lands in Part 2b).
pub(crate) fn reject_memoryview_read(name: &str, ty: &Ty, owned: bool) -> Result<(), Diagnostic> {
    if matches!(ty, Ty::MemoryView) {
        if owned {
            return Err(crate::buffer::owned_buffer_use_unsupported(name));
        }
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "using `{name}`, which is bound to a buffer parameter of a \
                 `pycc build --ext` export, is valid Python but not implemented yet; \
                 the name cannot be used as a whole value here -- read \
                 one element at a time with `{name}[i]` over `range(len({name}))`, and \
                 store one with `{name}[i] = 1.0`"
            ),
            Span::new(0, 0),
        ));
    }
    Ok(())
}

/// `Err(C0001)` when a declaration's annotation is `memoryview`.
///
/// The companion to [`reject_memoryview_read`], at the one position that
/// read cannot cover. `pycc_hir`'s `annotation_to_ty` is both the parser of
/// a signature's types *and* the parser of a bare `x: T` declaration, so
/// admitting `Ty::MemoryView` there widened every annotation position at
/// once -- including a value-less `AnnAssign`, which `env.declare` then
/// records with no scalar-type restriction and `pycc_mir` lowers to a
/// `MirStmt::NoOp`. The program compiled silently, where `x: object` -- any
/// other annotation this compiler does not implement -- is still refused.
///
/// This restores that refusal, so `src/memoryview_mode.rs`'s native-mode
/// gate keeps its narrow job: the *signature* positions, which are the only
/// ones `pycc build --ext` admits at all. The declaration is refused in both
/// modes, because neither has anything to bind to the name.
pub(crate) fn reject_memoryview_declaration(
    target: &str,
    annotation: &Ty,
) -> Result<(), Diagnostic> {
    if matches!(annotation, Ty::MemoryView) {
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "declaring `{target}` as a buffer is valid Python but not implemented \
                 yet; this declaration binds no buffer storage to the name, and a \
                 `pycc build --ext` export admits the annotation in its signature or on \
                 a declaration whose initializer allocates the storage"
            ),
            Span::new(0, 0),
        ));
    }
    Ok(())
}
