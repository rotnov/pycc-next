//! Function-definition lowering: parameters, return annotations, and the
//! annotation-to-`Ty` conversion every annotated binding in the crate goes
//! through.
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #547, Part 2). This is not a low-fan-in leaf the way `stmt`/`expr` were:
//! `lower_function` and `lower_params` are each called once (from
//! `module::lower_top_level_item` and from `lower_function` respectively), but the three
//! `pub(crate)` helpers are reached from across the crate --
//! `lower_return_annotation` from 3 call sites in `class.rs`,
//! `lower_arg_list` from 8, `type_param_name` from 1, and `annotation_to_ty`
//! from `class.rs`, `stmt.rs`, `tests.rs`, and the sibling `import` module's
//! type-alias lowering. `lib.rs` therefore re-exports all five items
//! `pub(crate)`, so every existing `crate::`-qualified call site keeps
//! resolving unchanged.

mod annotation;
mod bare_container;
mod buffer_spelling;
mod container_annotation;
pub(crate) mod generator;
pub(crate) mod params;
#[cfg(test)]
mod params_tests;
mod string_annotation;
mod union;
#[cfg(test)]
mod string_annotation_tests;

use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExpr, HirItem, HirStmt, ImportBinding, Ty, stmt, unsupported};
pub(crate) use bare_container::{with_bare_container_advice, with_bare_list_or_dict_advice};
use params::DefaultPolicy;
use pycc_ast::Expr;
use pycc_diag::{Diagnostic, Span};
pub(crate) use annotation::{
    EXT_MODULE_MARKER, NON_STRICT_MARKER, annotation_to_ty, is_ext_module, is_non_strict,
    objects_spellable,
};
#[cfg(test)]
pub(crate) use annotation::subscripted_base_description;

pub(crate) fn lower_function(
    def: &pycc_ast::StmtFunctionDef,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirItem, Diagnostic> {
    if def.is_async {
        return Err(unsupported(
            "async functions are not supported yet",
            def.range,
        ));
    }
    if !def.decorator_list.is_empty() {
        return Err(unsupported(
            "function decorators are not supported yet -- only `@functools.cache` and \
             `@functools.lru_cache` on a module-level function with annotated `int`/`float`/\
             `bool`/`str` parameters compile; `--non-strict` runs any other decorated function \
             in CPython",
            def.range,
        ));
    }
    let type_param: Option<Box<str>> = match def.type_params.as_deref() {
        None => None,
        Some(type_params) => match type_params.type_params.as_slice() {
            [single] => Some(type_param_name(single, def.range)?.into()),
            _ => {
                return Err(unsupported(
                    "generic functions with more than one type parameter are not supported yet",
                    def.range,
                ));
            }
        },
    };
    // A `def` the desugaring synthesized from a lambda or a `sys` stream
    // alias carries `pycc_ast::INFER_MARKER` as its return annotation: its
    // signature is inferred like a private helper's.
    let synthesized = matches!(
        def.returns.as_deref(),
        Some(pycc_ast::Expr::Name(name)) if name.id.as_str() == pycc_ast::INFER_MARKER
    );
    // Non-strict mode (#3) outside an `--ext` module: no Python caller can
    // reach an unannotated public function except through its own module, so
    // its signature is inferred from its uses like a private helper's (a
    // type the solver cannot pin is an object, `constraints::signatures`).
    // An `--ext` module's function is called from Python with anything, so
    // there it stays an object.
    let inferred_public = !is_ext_module(aliases);
    let is_public = crate::is_public_name(def.name.as_str()) && !synthesized && !inferred_public; // D-038
    let params = lower_params(
        &def.parameters,
        is_public,
        def.name.as_str(),
        type_param.as_deref(),
        aliases,
        class_defs,
    )?;
    let return_ty = lower_return_annotation(
        if synthesized {
            None
        } else {
            def.returns.as_deref()
        },
        is_public,
        def.name.as_str(),
        type_param.as_deref(),
        None,
        aliases,
        class_defs,
    )?;
    let body = stmt::lower_body(
        &def.body,
        aliases,
        false,
        true,
        false,
        // #795 (PEP 654): a nested function body always starts
        // `Outside` any enclosing `except*` clause -- CPython accepts a
        // `return` in a `def` nested inside an `except*` body. This is a
        // constant, not a conditional on the enclosing context, exactly like
        // the three `false`s above it.
        stmt::ExceptStarCtx::Outside,
        None,
        type_param.as_deref(),
        class_defs,
        imports,
        signatures,
    )?;
    let mut body = body;
    body.splice(0..0, crate::positions::def_header(def));
    if crate::body_yields(&body) || pycc_ast::body_yields(&def.body) {
        let return_ty = generator::generator_return_ty(return_ty, aliases, def)?;
        generator::generator_returns(&mut body);
        return Ok(HirItem::Function {
            name: def.name.to_string(),
            params,
            return_ty,
            body,
        });
    }
    // Non-strict mode (#3): a function returning an object that can run off
    // its end returns `None` there, as every Python function does.
    if return_ty == Ty::Object
        && is_non_strict(aliases)
        && !crate::return_coverage::block_always_returns(&body)
    {
        body.push(HirStmt::Return(Some(HirExpr::NoneLiteral)));
    }
    Ok(HirItem::Function {
        name: def.name.to_string(),
        params,
        return_ty,
        body,
    })
}

/// The `C0001` for a `Callable[...]` annotation not of the form
/// `Callable[[A, B], R]`.
pub(super) fn callable_shape(annotation: &Expr) -> Diagnostic {
    unsupported(
        "`Callable` is supported as `Callable[[A, B], R]` (a parameter list and a return type)",
        pycc_ast::expr_range(annotation),
    )
}

/// Extracts a PEP 695 `TypeVar`'s identifier -- e.g. the `T` in `def
/// f[T](...)`. `Ty::Param` (D-133) is resolved by call-site substitution
/// (D-134) into one concrete scalar type per call, which is only a coherent
/// model for a plain `TypeVar`: `TypeVarTuple` (`def f[*Ts](...)`) stands for
/// a variable-length sequence of types, and `ParamSpec` (`def f[**P](...)`)
/// stands for a parameter list shape, neither of which `Ty::Param` can
/// represent. `def_range` is the enclosing function's range, reused for the
/// diagnostic span since `TypeParam`'s own range would require reaching past
/// the `pycc_ast` facade for the `Ranged` trait for no benefit here (the
/// arity-gate rejection just above already reports the same function-level
/// span for the analogous "too many type parameters" case).
pub(crate) fn type_param_name<R>(
    type_param: &pycc_ast::TypeParam,
    def_range: R,
) -> Result<&str, Diagnostic>
where
    std::ops::Range<u32>: From<R>,
{
    match type_param {
        pycc_ast::TypeParam::TypeVar(tv) => Ok(tv.name.as_str()),
        pycc_ast::TypeParam::TypeVarTuple(_) => Err(unsupported(
            "a `TypeVarTuple` type parameter (`*Ts`) is not supported yet",
            def_range,
        )),
        pycc_ast::TypeParam::ParamSpec(_) => Err(unsupported(
            "a `ParamSpec` type parameter (`**P`) is not supported yet",
            def_range,
        )),
    }
}

pub(crate) fn lower_params(
    parameters: &pycc_ast::Parameters,
    is_public: bool,
    fn_name: &str,
    type_param: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    // Every parameter kind below is silently absent from
    // `parameters.args` -- an earlier version of this function only ever
    // iterated `.args` and never checked for any of these, so a function
    // using them got a wrong signature built from whatever plain positional
    // args happened to exist, instead of the explicit capability diagnostic
    // every other out-of-scope construct in this file produces (self-review
    // finding, pre-merge). The checks themselves now live in `params`,
    // shared with `class::lower_method`; a *default value* is no longer one
    // of them -- Part 2 of #884 (#1189) implements it for this path and
    // the method part of #1140 for a method, and `lower_arg_list`'s
    // `DefaultPolicy::Reject` now keeps only a `Protocol` member rejecting
    // it.
    //
    // PEP 570 (#383): positional-only parameters (`posonlyargs`, before the
    // `/` marker) are lowered via the same `lower_arg_list` path as ordinary
    // `args`, prepended before `args` in the parameter list — so index `i`
    // of the returned vector is index `i` of `HirExpr::Call::args`.
    //
    // Part 1 of #884 (#1125) made that ordering load-bearing. Keyword call
    // arguments are no longer globally unsupported: a call to a module-level
    // `def` may now name a parameter, so the two parameter kinds are no
    // longer interchangeable at the call site. `expr::keyword_bind`
    // reproduces this exact concatenation order when it collects a
    // signature, and excludes the leading `posonlyargs` entries from the set
    // a keyword may name — naming one is a `T0021`, as in CPython. A
    // parameter after the `/` marker stays bindable by name.
    params::reject_unsupported_parameter_shapes(parameters)?;
    // PEP 570 (#383): lower `posonlyargs` (before `/`) via the same
    // `lower_arg_list` path as ordinary `args`, prepending them. The full
    // parameter list is `posonlyargs ++ args`.
    // The lowered defaults are dropped here: a module-level `def`'s
    // defaults are filled at its call sites from the AST-derived signature
    // table (`expr::keyword_bind`), which runs before any item is lowered.
    let (mut params, _) = lower_arg_list(
        &parameters.posonlyargs,
        is_public,
        fn_name,
        type_param,
        None,
        aliases,
        class_defs,
        DefaultPolicy::Admit,
    )?;
    params.extend(
        lower_arg_list(
            &parameters.args,
            is_public,
            fn_name,
            type_param,
            None,
            aliases,
            class_defs,
            DefaultPolicy::Admit,
        )?
        .0,
    );
    Ok(params)
}

/// Lowers a plain positional-parameter list (no `/`/`*`/`**`/keyword-only
/// markers -- callers are responsible for rejecting those first, since their
/// diagnostics differ by caller: `lower_params` reports them against a
/// top-level function's own `parameters`, `class::lower_method` (D-154, Part
/// 1 of #375) reports the identical checks against a method's `parameters`,
/// which also includes the leading `self` parameter that helper strips
/// before delegating here). Factored out of `lower_params` (which still owns
/// every top-level function's own shape validation, unchanged) so both
/// callers share this one per-parameter annotation-resolution rule instead
/// of duplicating it.
///
/// `policy` decides what a default value means here (Part 2 of #884, #1189).
/// Under [`DefaultPolicy::Reject`] -- the protocol-member callers -- the
/// pre-existing capability check runs unchanged and *before* annotation
/// resolution, so a parameter whose annotation would itself be rejected
/// still reports the capability message first. Under
/// [`DefaultPolicy::Admit`] -- `lower_params`' two calls -- and
/// [`DefaultPolicy::AdmitMethod`] -- every `class::lower_method` arm, since
/// the method part of #1140 -- the annotation
/// is resolved first and `params::check_default` then applies every default
/// rule, returning the lowered default alongside the parameter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_arg_list(
    args: &[pycc_ast::ParameterWithDefault],
    is_public: bool,
    fn_name: &str,
    type_param: Option<&str>,
    class_name: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
    policy: DefaultPolicy,
) -> Result<LoweredArgs, Diagnostic> {
    let lowered: Vec<((String, Ty), Option<HirExpr>)> = args
        .iter()
        .map(|param| {
            if policy == DefaultPolicy::Reject && param.default.is_some() {
                return Err(unsupported(
                    "default parameter values are not supported yet",
                    param.range,
                ));
            }
            let name = param.parameter.name.as_str();
            let ty = match &param.parameter.annotation {
                // An `--ext --non-strict` module's `Iterable[T]` parameter
                // takes any iterable (a list, a set, a `str`, a generator),
                // which only the object holds; elsewhere it is a generator.
                Some(ann)
                    if is_ext_module(aliases)
                        && is_non_strict(aliases)
                        && matches!(&**ann, Expr::Subscript(sub)
                            if matches!(sub.value.as_ref(), Expr::Name(base)
                                if base.id.as_str() == "Iterable"
                                    && !class_defs.iter().any(|info| info.name == "Iterable"))) =>
                {
                    Ty::Object
                }
                Some(ann) => annotation_to_ty(ann, type_param, class_name, aliases, class_defs)
                    .map_err(|error| with_bare_container_advice(error, ann))?,
                // #1409: in an `--ext` module an unannotated public
                // parameter takes the type its literal default implies.
                // `Reject` never gets here with a default (refused above).
                None if is_public
                    && let Some(ty) = param
                        .default
                        .as_deref()
                        .filter(|_| is_ext_module(aliases) && !is_non_strict(aliases))
                        .and_then(|default| params::unannotated_default_ty(default, policy)) =>
                {
                    ty
                }
                // Non-strict mode (#3): what is not annotated is an object.
                None if is_public && is_non_strict(aliases) => Ty::Object,
                None if is_public => {
                    return Err(Diagnostic::error(
                        "T0001",
                        format!(
                            "parameter `{name}` of public function `{fn_name}` needs a type annotation"
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!("add a type annotation to parameter `{name}`")));
                }
                None => Ty::Infer,
            };
            let default = param
                .default
                .as_deref()
                .map(|default| {
                    params::check_default(
                        default,
                        name,
                        fn_name,
                        &ty,
                        policy,
                        is_non_strict(aliases),
                    )
                })
                .transpose()?;
            Ok(((name.to_string(), ty), default))
        })
        .collect::<Result<_, Diagnostic>>()?;
    Ok(lowered.into_iter().unzip())
}

/// What [`lower_arg_list`] produces: the `(name, type)` parameter list and,
/// parallel to it, each parameter's lowered default value (`None` for a
/// parameter without one). Under [`DefaultPolicy::Reject`] the second half
/// is all `None` by construction.
pub(crate) type LoweredArgs = (Vec<(String, Ty)>, Vec<Option<HirExpr>>);

pub(crate) fn lower_return_annotation(
    returns: Option<&Expr>,
    is_public: bool,
    fn_name: &str,
    type_param: Option<&str>,
    class_name: Option<&str>,
    aliases: &[(String, Ty)],
    class_defs: &[ClassAnnotationInfo],
) -> Result<Ty, Diagnostic> {
    match returns {
        Some(ann) => {
            // Return position lowers a parameterized container annotation
            // exactly like every other position (#925, Part 2 of #918).
            // D-228 (Part 1) deliberately excluded it while a container-typed
            // call result still reached an unhandled codegen case; #925 added
            // the codegen arms that closed that gap, so that exclusion is
            // gone. Element-type and arity gates still fire on this
            // annotation's own span, because they run inside
            // `annotation_to_ty` below.
            let ty = annotation_to_ty(ann, type_param, class_name, aliases, class_defs)
                .map_err(|error| with_bare_container_advice(error, ann))?;
            // #934: the one return-position check that remains. A protocol
            // is a compile-time-only interface with no runtime
            // representation (D-166); a protocol-typed *parameter* or
            // *variable* is bound to the concrete class of the value it
            // receives (monomorphization, `pycc_mir`'s binding of the
            // inferred type), but a call to a `-> P` function has no
            // concrete type to bind, so every shape of such a function
            // used to type-check and then abort inside `pycc_mir` or
            // `pycc_codegen`. Rejecting the annotation here closes all of
            // them at once: this function is the single seam for a
            // module-level function, a method, and a protocol member
            // declaration. The check runs *after* `annotation_to_ty` so a
            // `-> list[P]` still reports D-105's `T0034` first, exactly as
            // D-228 pins for containers; `-> P | None` is `T0049` for the
            // same reason, so only a top-level `Ty::Protocol` reaches here.
            if let Ty::Protocol(protocol) = &ty {
                return Err(unsupported(
                    format!(
                        "a protocol class (`{protocol}`) as a return type annotation is not \
                         supported yet -- a protocol type is currently supported in parameter \
                         and variable positions only"
                    ),
                    pycc_ast::expr_range(ann),
                ));
            }
            Ok(ty)
        }
        // Non-strict mode (#3): an unannotated public return is an object.
        None if is_public && is_non_strict(aliases) => Ok(Ty::Object),
        None if is_public => Err(Diagnostic::error(
            "T0001",
            format!("public function `{fn_name}` needs a return type annotation"),
            Span::new(0, 0),
        )
        .with_help(format!("add a return type annotation to `{fn_name}`"))),
        None => Ok(Ty::Infer),
    }
}

#[cfg(test)]
mod return_annotation_tests;
