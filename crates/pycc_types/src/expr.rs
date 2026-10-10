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

// Large `infer_expr_in` arms (mechanical extraction: each helper
// re-destructures its own node and runs the arm body verbatim).
mod attr_infer;
mod call_infer;
mod container_infer;
mod name_infer;

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
use pycc_hir::{BinOpKind, ContainerReceiver, FStringPart, HirExpr};

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
    infer_expr_here(env, local_names, expr)
        .map_err(|diagnostic| crate::locate::place_expr(diagnostic, expr))
}

/// [`infer_expr_in`] before its diagnostic is placed at `expr`.
fn infer_expr_here(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    match expr {
        HirExpr::IntLiteral(_) => Ok(Ty::Int),
        HirExpr::FloatLiteral(_) => Ok(Ty::Float),
        HirExpr::BoolLiteral(_) | HirExpr::OpaqueBool => Ok(Ty::Bool),
        HirExpr::StringLiteral(_) => Ok(Ty::Str),
        HirExpr::NoneLiteral => Ok(Ty::None),
        // #1418: CPython's `NotImplemented` singleton is a CPython object.
        HirExpr::NotImplemented => Ok(Ty::Object),
        HirExpr::FString(..) => name_infer::infer_f_string_expr(env, local_names, expr),
        HirExpr::Name(..) => name_infer::infer_name_expr(env, local_names, expr),
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
        HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right } => {
            let left_ty = infer_expr_in(env, local_names, left)?;
            let right_ty = infer_expr_in(env, local_names, right)?;
            reject_negative_literal_exponent(*op, &left_ty, right)?;
            if let Some(result) =
                crate::foreign::arith::object_binop_ty(Some(env), &left_ty, &right_ty)
            {
                return result;
            }
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
        HirExpr::Call { .. } => call_infer::infer_call_expr(env, local_names, expr),
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
        HirExpr::EmptySet(element) => {
            let set_ty = Ty::Set(Box::new(element.clone()));
            pycc_hir::check_container_ty(&set_ty, Span::new(0, 0))?;
            Ok(set_ty)
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
        HirExpr::ListLiteral(..) => container_infer::infer_list_literal_expr(env, local_names, expr),
        HirExpr::DictLiteral(..) => container_infer::infer_dict_literal_expr(env, local_names, expr),
        HirExpr::SetLiteral(..) => container_infer::infer_set_literal_expr(env, local_names, expr),
        HirExpr::TupleLiteral(..) => container_infer::infer_tuple_literal_expr(env, local_names, expr),
        HirExpr::Subscript { .. } => container_infer::infer_subscript_expr(env, local_names, expr),
        HirExpr::Slice { .. } => container_infer::infer_slice_expr(env, local_names, expr),
        HirExpr::ListAppend { .. } => container_infer::infer_list_append_expr(env, local_names, expr),
        HirExpr::ListPop { .. } => container_infer::infer_list_pop_expr(env, local_names, expr),
        HirExpr::DictGetOrDefault { .. } => container_infer::infer_dict_get_or_default_expr(env, local_names, expr),
        HirExpr::SetAdd { .. } => container_infer::infer_set_add_expr(env, local_names, expr),
        HirExpr::AttrGet { .. } => attr_infer::infer_attr_get_expr(env, local_names, expr),
        // Issue #1188: one of the four container method names in a module
        // that can see a user class defining it; the receiver picks the
        // reading.
        HirExpr::ReceiverDispatchedCall { call, container } => {
            receiver_dispatch::infer_receiver_dispatched_call(env, local_names, call, container)
        }
        HirExpr::MethodCall { .. } => attr_infer::infer_method_call_expr(env, local_names, expr),
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
        HirExpr::GenericClassInstantiate { .. } => attr_infer::infer_generic_class_instantiate_expr(env, expr),
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
        HirExpr::NamedExpr { .. } => name_infer::infer_named_expr_expr(env, local_names, expr),
        HirExpr::Comprehension(comp) => {
            crate::comprehension::infer_comprehension(env, local_names, comp)
        }
    }
}

/// `int ** <negative int literal>` is a `float` in CPython, but `**` on two
/// `int`s is typed `int` (`docs/DEVIATIONS.md`), so the compiled program
/// would raise `RuntimeError` at run time. A literal exponent is known now:
/// refuse it at compile time instead, where non-strict mode can fall back.
fn reject_negative_literal_exponent(
    op: BinOpKind,
    left_ty: &Ty,
    right: &HirExpr,
) -> Result<(), Diagnostic> {
    let negative_literal = matches!(right, HirExpr::IntLiteral(exponent) if *exponent < 0);
    if op == BinOpKind::Pow && negative_literal && matches!(left_ty, Ty::Int | Ty::Bool) {
        let diagnostic = Diagnostic::error(
            "C0001",
            "`int ** <negative int>` is a `float` in CPython, which is not compiled yet",
            Span::new(0, 0),
        )
        .with_help("write the base as a `float` literal, e.g. `10.0 ** -10`");
        return Err(crate::locate::place_labeled(
            diagnostic,
            crate::locate::expr_span(right),
            "negative exponent",
        ));
    }
    Ok(())
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
