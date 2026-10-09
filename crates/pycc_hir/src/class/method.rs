//! Lowering of one method definition inside a class body.
//!
//! Split out of `class.rs` (the method part of #1140 touched it, and that
//! file is far past the ~1,000-line decomposition threshold): this holds
//! [`lower_method`], which turns one `def` in a class body into an
//! ordinary `HirItem::Function` under its mangled name, together with the
//! method's full parameter list and its default parameter values.

use pycc_diag::Diagnostic;

use super::{ClassAnnotationInfo, MethodKind, receiver};
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExpr, HirItem, ImportBinding, Ty, lower_arg_list, unsupported};

/// Lowers a single method definition into an ordinary `HirItem::Function`
/// under its mangled `<ClassName>.<method_name>` name, plus that method's
/// own full parameter list (including `self`) -- returned alongside so
/// `lower_class` can build the `__init__`-specific attribute-slot pre-scan's
/// parameter-name -> `Ty` lookup table without re-deriving it, and the
/// method's lowered default values for `HirClassDef::method_defaults`.
///
/// `self`'s type never goes through `annotation_to_ty` -- it is assigned
/// `Ty::Instance(Box::new(class_name))` directly (mirroring how the type
/// itself carries only the class's name, not its shape), bypassing the
/// class-typed-annotation restriction entirely. An explicit annotation on
/// `self` is rejected rather than silently ignored, so a user-written
/// (and unchecked) annotation there can never appear to be honored.
///
/// `__init__`'s own (non-`self`) parameters are *always* required to carry
/// an explicit type annotation, regardless of the ordinary "only a public
/// name requires one" rule (D-038) every other function/method follows --
/// a deliberate, narrower rule than D-038's, not an oversight: those
/// parameter types are the only source `init_slot::collect_init_attrs` has for
/// deriving an attribute slot's `Ty` structurally, at HIR-lowering time,
/// with no type-inference pass of its own (this crate never runs one --
/// see `Ty::Infer`'s own doc comment). An unannotated `__init__` parameter
/// referenced by a `self.<attr> = <param>` assignment would otherwise seed
/// the slot with `Ty::Infer`, which must never reach `pycc_mir` unresolved.
/// The one exception, in an `--ext` module, is an unannotated parameter
/// whose literal or `None` default implies a concrete type (#1409,
/// `func::params::unannotated_default_ty`): it seeds the slot with that
/// type, never `Ty::Infer`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_method(
    def: &pycc_ast::StmtFunctionDef,
    class_name: &str,
    type_param: Option<&str>,
    aliases: &[(String, Ty)],
    kind: &MethodKind,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<LoweredMethod, Diagnostic> {
    if def.is_async {
        return Err(unsupported(
            "an async method is not supported yet",
            def.range,
        ));
    }
    // #377: decorators are now classified by `classify_decorator` in
    // `lower_class` before this function is called -- the `kind` parameter
    // carries the result. No additional decorator check is needed here.
    if def.type_params.is_some() {
        return Err(unsupported(
            "a generic method is not supported yet",
            def.range,
        ));
    }
    let parameters = &def.parameters;
    // PEP 570 (#383): positional-only parameters (`posonlyargs`, before the
    // `/` marker) are now lowered. For `@staticmethod`, posonlyargs come
    // first (no implicit `self`/`cls`). For regular/classmethod methods,
    // `self`/`cls` is always the first parameter, so posonlyargs follow it.
    // A method's parameters stay effectively positional-only: Part 1 of
    // #884 (#1125) made keyword call arguments bindable for a module-level
    // `def` only, and a method call keeps the unchanged `C0001` — so
    // accepting posonlyargs still changes nothing about call-site checking
    // here. `reject_unsupported_parameter_shapes` holds the shape checks
    // this shares with `func::lower_params` (Part 2 of #884, #1189).
    crate::func::params::reject_unsupported_parameter_shapes(parameters)?;
    let method_name = def.name.as_str();
    let is_public = crate::is_public_name(method_name); // D-038
    let params_is_public = is_public || method_name == "__init__";
    // #436: a `@staticmethod` takes no implicit `self`/`cls` -- the
    // method's own parameter list is exactly what the user wrote. A
    // `@classmethod` takes an implicit `cls` (typed
    // `Ty::Instance(class_name)`, matching `self`'s own type in this
    // compiler's static-dispatch model) as its first parameter. A
    // regular/property method takes `self` as before.
    // #1181: set by the `_ =>` arm below to the receiver's *source*
    // spelling, so the alias statement can be prepended to the lowered body
    // once the body exists. `None` for `@staticmethod` (no receiver) and
    // `@classmethod` (its own `cls` rule, the `ClassMethod` arm below).
    let mut receiver_name: Option<String> = None;
    // The method part of #1140: every arm below admits a default value on a
    // parameter after the receiver, under exactly the rules a module-level
    // `def` uses (`func::params::check_default`). Each lowered default is
    // collected here, parallel to the non-receiver parameters, and handed
    // back to the class-body walk for `HirClassDef::method_defaults`.
    let mut defaults: Vec<Option<HirExpr>> = Vec::new();
    let mut lower_rest =
        |args: &[pycc_ast::ParameterWithDefault]| -> Result<Vec<(String, Ty)>, Diagnostic> {
            let (lowered, lowered_defaults) = lower_arg_list(
                args,
                params_is_public,
                method_name,
                type_param,
                Some(class_name),
                aliases,
                class_defs,
                crate::func::params::DefaultPolicy::AdmitMethod,
            )?;
            defaults.extend(lowered_defaults);
            Ok(lowered)
        };
    let params = match kind {
        MethodKind::StaticMethod => {
            // PEP 570 (#383): for `@staticmethod`, posonlyargs come first
            // (no implicit `self`/`cls`), then ordinary `args`.
            let mut p = lower_rest(&parameters.posonlyargs)?;
            p.extend(lower_rest(&parameters.args)?);
            p
        }
        MethodKind::ClassMethod => {
            // PEP 570 (#383): `cls` is the first parameter overall — it
            // may be in `posonlyargs` (if `/` follows it) or in `args`.
            // Extract it from the combined list, then lower the rest.
            if parameters.posonlyargs.is_empty() && parameters.args.is_empty() {
                return Err(unsupported(
                    "a `@classmethod` must take `cls` as its first parameter",
                    def.range,
                ));
            }
            let (cls_param, posonly_rest, args_rest) = if !parameters.posonlyargs.is_empty() {
                let (cls, rest_pos) = parameters.posonlyargs.split_first().unwrap();
                (cls, rest_pos, parameters.args.as_slice())
            } else {
                let (cls, rest_args) = parameters.args.split_first().unwrap();
                (cls, &[][..], rest_args)
            };
            if cls_param.parameter.name.as_str() != "cls" {
                return Err(unsupported(
                    "a `@classmethod`'s first parameter must be named `cls`",
                    parameters.range,
                ));
            }
            if cls_param.default.is_some() {
                return Err(unsupported(
                    "`cls` cannot have a default value",
                    parameters.range,
                ));
            }
            if cls_param.parameter.annotation.is_some() {
                return Err(unsupported(
                    "an explicit type annotation on `cls` is not supported yet",
                    parameters.range,
                ));
            }
            receiver::check_receiver_not_deleted(&def.body, "cls", parameters.range.into())?;
            let cls_ty = Ty::Instance(Box::new(class_name.to_string()));
            let mut p = vec![("cls".to_string(), cls_ty)];
            // PEP 570 (#383): remaining posonlyargs follow `cls`, before
            // ordinary `args`.
            p.extend(lower_rest(posonly_rest)?);
            p.extend(lower_rest(args_rest)?);
            p
        }
        _ => {
            // #1181: the receiver is the first positional parameter whatever
            // it is spelled; `class::receiver` owns every part of that
            // decision (see its own module doc comment for the rule and for
            // why the two guards below exist). Both guards run *here*,
            // before the `stmt::lower_body` call further down: `global` and
            // `nonlocal` each report their own `C0001` from that pass, so a
            // scan placed after it would never reach those shapes with the
            // receiver's own message. A `del` of the receiver gets its own
            // refusal (#1244), whatever the receiver's spelling.
            let split = receiver::split_receiver(parameters, def.range.into())?;
            receiver::check_receiver_param(&split, parameters.range.into())?;
            receiver::check_property_arity(kind, &split, parameters.range.into())?;
            receiver::check_receiver_not_deleted(&def.body, split.name(), parameters.range.into())?;
            receiver::check_renamed_receiver(def, method_name, &split)?;
            receiver_name = Some(split.name().to_string());
            let (posonly_rest, args_rest) = (split.posonly_rest, split.args_rest);
            let self_ty = Ty::Instance(Box::new(class_name.to_string()));
            let mut p = vec![(receiver::CANONICAL_RECEIVER.to_string(), self_ty)];
            // PEP 570 (#383): remaining posonlyargs follow the receiver,
            // before ordinary `args`.
            p.extend(lower_rest(posonly_rest)?);
            p.extend(lower_rest(args_rest)?);
            p
        }
    };
    // Align the defaults with the full parameter list: a receiver (`self`
    // or `cls`) never carries one -- both are refused above -- so it gets a
    // leading `None`.
    let mut defaults = {
        let mut aligned = vec![None; params.len() - defaults.len()];
        aligned.append(&mut defaults);
        aligned
    };
    if defaults.iter().all(Option::is_none) {
        defaults.clear();
    }
    let params = equality_operand_as_object(params, method_name, kind, &defaults, aliases);
    let return_ty = crate::lower_return_annotation(
        def.returns.as_deref(),
        is_public,
        method_name,
        type_param,
        Some(class_name),
        aliases,
        class_defs,
    )?;
    let body = if matches!(kind, MethodKind::AbstractMethod) {
        // #380 (PR-20): an `@abstractmethod` has a declaration-style
        // body (`...` or `pass`) that is already validated in
        // `lower_class`. Skip body lowering — the abstract method is
        // registered as a function (for dispatch/mangling purposes)
        // but its body is never called. Use a `Return(None)` so the
        // function has a terminator for codegen; the type checker
        // skips the return-value check for abstract methods (see
        // `check_stmt_in_function`'s `Return(None)` arm).
        vec![crate::HirStmt::Return(None)]
    } else {
        crate::stmt::lower_body(
            &def.body,
            aliases,
            false,
            true,
            false,
            // #795 (PEP 654): a method body always starts `Outside` any
            // enclosing `except*` clause -- see `func.rs`'s own constant.
            crate::stmt::ExceptStarCtx::Outside,
            Some(class_name),
            type_param,
            class_defs,
            imports,
            signatures,
        )?
    };
    // #1181: when the source spells the receiver anything other than the
    // canonical `self`, open the lowered body with `<name> = self` so the
    // user's spelling is an ordinary local bound to the canonical receiver
    // (see `class::receiver`'s module doc comment). Applied uniformly,
    // including to `MethodKind::AbstractMethod`'s synthesized
    // `Return(None)` body above: an abstract method is registered as a
    // function but never called, so an unused alias there is inert, and a
    // separate branch would only be a second thing to keep in step.
    let body = match receiver_name.as_deref() {
        Some(name) if name != receiver::CANONICAL_RECEIVER => {
            let mut aliased = vec![receiver::alias_stmt(name)];
            aliased.extend(body);
            aliased
        }
        _ => body,
    };
    // #1418 (D-258's #1418 amendment): a comparison method that returns
    // `NotImplemented` returns a CPython object whatever its annotation
    // says, as in CPython, so its return type is the object. The annotation
    // is still lowered above, so a malformed one keeps its own diagnostic.
    // Only an `ext` module's admitted `return NotImplemented` lowers to the
    // node this looks for (`crate::not_implemented`).
    let return_ty = if crate::not_implemented::is_comparison_dunder(method_name)
        && crate::not_implemented::body_returns_not_implemented(&body)
    {
        Ty::Object
    } else {
        return_ty
    };
    // #377/#436: compute the mangled name based on the method kind. A
    // regular method uses `<Class>.<name>`. A property getter uses the
    // same `<Class>.<name>`. A property setter uses
    // `<Class>.<name>.setter`. A static method uses
    // `<Class>.<name>.static`. A class method uses
    // `<Class>.<name>.classmethod`. The `.static`/`.classmethod` suffixes
    // prevent collision with a regular method of the same name, since a
    // real Python identifier can never contain a `.`.
    let mangled_name = match kind {
        MethodKind::Regular { .. }
        | MethodKind::PropertyGetter { .. }
        | MethodKind::AbstractMethod => {
            format!("{class_name}.{method_name}")
        }
        MethodKind::PropertySetter { prop_name } => {
            format!("{class_name}.{prop_name}.setter")
        }
        MethodKind::StaticMethod => format!("{class_name}.{method_name}.static"),
        MethodKind::ClassMethod => format!("{class_name}.{method_name}.classmethod"),
    };
    Ok((
        HirItem::Function {
            name: mangled_name.clone(),
            params: params.clone(),
            return_ty,
            body,
        },
        params,
        (mangled_name, defaults),
    ))
}

/// #1387: in an `--ext` module, the unannotated operand of `__eq__` or
/// `__ne__` is the opaque CPython `object` (D-258) instead of an inference
/// variable.
///
/// Python's data model fixes that operand's type: `x == y` hands the method
/// whatever `y` is, which is why typeshed spells `object.__eq__`'s operand
/// `object`, and why an unannotated operand -- lark's `ParserState.__eq__`
/// writes `def __eq__(self, other) -> bool` -- has no call site the solver
/// could infer it from. Left as `Ty::Infer` it is `T0021` ("cannot infer
/// type of parameter").
///
/// The rule is deliberately narrow: a regular method (not a static method,
/// class method or property, none of which the data model calls with an
/// operand), named exactly `__eq__` or `__ne__`, with exactly one parameter
/// after the receiver, which carries neither an annotation nor a default.
/// An annotated operand keeps its annotation, a defaulted one keeps #1409's
/// default-derived type, and a `native` module keeps `T0021`, since `object`
/// is not spellable in a `native` annotation (`docs/TYPE_SYSTEM.md`).
fn equality_operand_as_object(
    mut params: Vec<(String, Ty)>,
    method_name: &str,
    kind: &MethodKind,
    defaults: &[Option<HirExpr>],
    aliases: &[(String, Ty)],
) -> Vec<(String, Ty)> {
    let applies = matches!(kind, MethodKind::Regular { .. })
        && matches!(method_name, "__eq__" | "__ne__")
        && crate::func::is_ext_module(aliases)
        && params.len() == 2
        && params[1].1 == Ty::Infer
        && defaults.get(1).is_none_or(Option::is_none);
    if applies {
        params[1].1 = Ty::Object;
    }
    params
}

/// What [`lower_method`] produces: the lowered item, the method's full
/// parameter list (receiver included), and the item's mangled name paired
/// with -- parallel to that list -- each parameter's lowered default value,
/// or an empty vector when no parameter has one (the method part of #1140).
pub(super) type LoweredMethod = (HirItem, Vec<(String, Ty)>, (String, Vec<Option<HirExpr>>));

#[cfg(test)]
#[path = "method_tests.rs"]
mod tests;
