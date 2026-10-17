//! Issue #977 (D-237): the string-conversion gate for `print` arguments and
//! f-string interpolations.
//!
//! `pycc_codegen`'s `to_str` renders scalars (`int`, `float`, `bool`, `str`,
//! `None`) directly and panics on a `Scalar::Instance`. The only way an
//! instance-typed value avoids that panic is a MIR rewrite: either
//! `rewrite_instance_to_repr` (a `@dataclass` instance becomes a call to its
//! synthesized `__repr__`) or `rewrite_exception_to_message` (a caught
//! builtin exception becomes `MirExpr::ExceptionMessage`). Until #977 the
//! type checker placed no restriction on either surface, so
//! `class C: ...; print(C())` passed `pycc check` and panicked in `pycc
//! build`, and several shadowing shapes passed `check` and aborted at
//! runtime. This module is the fail-closed predicate that keeps every such
//! program out of the backend: it is a conservative *under-approximation* of
//! what MIR renders, decided by **name provenance first, shape second**, and
//! it emits `C0001` (a capability gap, not a language rule) for everything it
//! cannot prove renderable.
//!
//! In its own module rather than `expr.rs` (already past AGENTS.md's ~1,000
//! line decomposition threshold), following `narrow.rs`/`std_receiver.rs`/
//! `redeclaration.rs`.

use crate::env::Environment;
use crate::expr::infer_expr_in;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

/// The two surfaces that hand a value to `pycc_codegen`'s `to_str`
/// (`crates/pycc_codegen/src/lib.rs`, the f-string and `print` lowering).
///
/// The `str()` builtin is not a third surface. It is `C0001` via
/// `KNOWN_CALLABLE_BUILTINS` for every argument type **except**
/// [`Ty::Object`], which PR 4b of #1083 (Part 4 of #1026) admits as an
/// explicit conversion out of the opaque type. That admission needs nothing
/// from this module: `str(o)` infers to [`Ty::Str`], which reaches
/// [`reject_unrenderable`]'s catch-all arm and is renderable, and codegen's
/// `to_str` returns a `Scalar::Str` unchanged -- so the heap `PyStrObj` the
/// shim copies out of CPython is handled identically to a literal-derived
/// one. `print(o)` and `f"{o}"` on the object itself are admitted as well
/// since #1340 ([`reject_unrenderable`] has no [`Ty::Object`] arm): codegen
/// renders the object through the shim, `print` with `str()` and an
/// f-string with `format(o, '')`, exactly as CPython does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringConversionSite {
    /// An argument of a `print(...)` call.
    PrintArgument,
    /// An `{expr}` interpolation inside an f-string.
    FStringInterpolation,
    /// The argument of a `str(...)` or `repr(...)` call.
    ConversionCall,
}

impl StringConversionSite {
    fn describe(self) -> &'static str {
        match self {
            StringConversionSite::PrintArgument => "a `print()` argument",
            StringConversionSite::FStringInterpolation => "an f-string interpolation",
            StringConversionSite::ConversionCall => "a `str()`/`repr()` argument",
        }
    }
}

/// Rejects a value that neither MIR rewrite can render as a string.
///
/// Every non-instance, non-protocol type passes through unchanged: this gate
/// covers only the instance/protocol shapes of #977. The tuple/container/
/// `Optional` `to_str` panics remain the documented follow-ups in
/// `docs/ROADMAP.md`.
///
/// The `Ty::Instance` arm's verdict, in the order it is decided:
///
/// 1. A builtin exception name (`pycc_hir::is_builtin_exception_class`)
///    is decided by *provenance*, never by the class's shape:
///    - seeded by HIR lowering (`env.is_synthetic_class`): accept.
///      `rewrite_exception_to_message` renders it -- the flat seven by name
///      (`pycc_mir::exception::resolve_exception_tag`), every other builtin
///      through its seeded class-table tag.
///    - absent from the class table (the shadow gate withheld seeding because
///      the module binds some *other* builtin exception name): only the flat
///      seven still render, by name, so they are accepted; every other
///      builtin gets no rewrite and would panic in codegen, so they are rejected.
///    - present but user-authored (plain or `@dataclass`) under any builtin
///      exception name: rejected. MIR resolves the name before the shape
///      (`rewrite_exception_to_message` runs first at both call sites in
///      `crates/pycc_mir/src/expr.rs`) and rewrites the plain instance to an
///      exception message, or applies a dataclass `__repr__` to an exception
///      object bound by `except*`, and the program crashes at runtime. This
///      deliberately also rejects a user `@dataclass` under a builtin exception
///      name past the flat seven that MIR would render today when instantiated
///      directly -- the one loss D-237 records.
/// 2. Any other name renders only through its synthesized dataclass
///    `__repr__`. A D-189 `exception_type_tag` on a user exception class is
///    *not* enough: MIR's class-table fallback would rewrite the plain
///    `PyInstanceObj` to `ExceptionMessage` and abort. A missing class-table
///    entry is reachable only through `pycc_types::check_function`'s empty
///    class table and is an ordinary rejection, not an internal error.
///
/// `Ty::Protocol` is rejected with its own message: a protocol-parameterised
/// body is checked once with the protocol type and its specialisations are
/// emitted by `monomorphize` without a second `infer_expr_in` pass, so the
/// checker cannot know the concrete class at the conversion site.
pub(crate) fn reject_unrenderable(
    env: &Environment,
    ty: &Ty,
    site: StringConversionSite,
) -> Result<(), Diagnostic> {
    match ty {
        Ty::Instance(name) => {
            let shadows_builtin_exception = pycc_hir::is_builtin_exception_class(name);
            let renderable = if shadows_builtin_exception {
                env.is_synthetic_class(name)
                    || (pycc_hir::is_flat_builtin_exception_class(name)
                        && env.lookup_class(name).is_none())
            } else {
                env.lookup_class(name).is_some_and(|def| def.is_dataclass)
                    || instance_text_dunder(env, name, false).is_some()
            };
            if renderable {
                Ok(())
            } else {
                Err(unrenderable_instance(name, site, shadows_builtin_exception))
            }
        }
        Ty::Protocol(name) => Err(unrenderable_protocol(name, site)),
        // #1340: a CPython object renders, so it takes the catch-all.
        // Part 2 of #1026 refused it here because `pycc_codegen`'s `to_str`
        // had no object arm; codegen now converts an object operand before
        // `to_str` -- `print` through the shim's `PyObject_Str` helper in
        // its write phase, an f-string part through its `PyObject_Format`
        // helper (`crates/pycc_codegen/src/string_render.rs`) -- so no arm
        // is needed.
        // Part 1 of #1319: `pycc_codegen`'s `to_str` has no set arm, so
        // before this refusal `print({1})` passed `pycc check` and panicked
        // in the backend. `list`, `dict` and `tuple` are deliberately not
        // added here: #1220 owns rendering them.
        // A set of hashable elements renders in CPython's order
        // (`pycc_rt::set`); an instance set has no `repr` for its elements.
        Ty::Set(elem) | Ty::FrozenSet(elem) if pycc_hir::is_hashable_ty(elem) => Ok(()),
        Ty::Set(_) | Ty::FrozenSet(_) => Err(crate::frozenset::unrenderable_set(ty)),
        // A list of instances renders each element by its `__repr__`
        // (`pycc_mir::class::rewrite_instance_to_text`).
        Ty::List(_) | Ty::Tuple(_) if renders_by_repr(env, ty) => Ok(()),
        Ty::Tuple(_) if instances_render(env, ty) == Some(false) => Err(Diagnostic::error(
            "C0001",
            format!(
                "rendering `{}` (a tuple holding instances of a class without `__repr__`) as a \
                 string is not supported yet",
                ty.name()
            ),
            Span::new(0, 0),
        )),
        Ty::List(_) => crate::list_ops::check_list_renderable(ty),
        // The runtime renders a `tuple[T, ...]` itself (`pycc_rt_list_repr`),
        // with no `__repr__` call for an instance element.
        Ty::VarTuple(elem) if matches!(**elem, Ty::Instance(_)) => Err(Diagnostic::error(
            "C0001",
            format!(
                "rendering `{}` (a tuple holding class instances) as a string is not \
                 supported yet",
                ty.name()
            ),
            Span::new(0, 0),
        )),
        // The runtime renders a dict's words itself, with no `__repr__` call
        // for an instance key or value.
        Ty::Dict(pair) if matches!(pair.0, Ty::Instance(_)) || matches!(pair.1, Ty::Instance(_)) => {
            Err(Diagnostic::error(
                "C0001",
                format!(
                    "rendering `{}` (a dict holding class instances) as a string is not \
                     supported yet",
                    ty.name()
                ),
                Span::new(0, 0),
            ))
        }
        // An `Optional[int|float|bool]` renders `None` or its payload
        // (`pycc_rt_optional_*_to_str`); no other `Optional` exists.
        Ty::Optional(inner) if matches!(inner.as_ref(), Ty::Int | Ty::Float | Ty::Bool) => Ok(()),
        // A pointer-represented `T | None` renders `None` or `T`'s text,
        // the runtime's own (`pycc_rt_optional_word_to_str`), so a `T`
        // holding a class instance (rendered by `__repr__`) stays refused.
        Ty::Optional(inner)
            if pycc_hir::is_pointer_optional_inner(inner) && !mentions_instance(inner) =>
        {
            reject_unrenderable(env, inner, site)
        }
        Ty::Optional(_) => Err(Diagnostic::error(
            "C0001",
            format!(
                "rendering `{}` (a value that may be `None`) as a string is not supported yet",
                ty.name()
            ),
            Span::new(0, 0),
        )),
        // A generator or a function value renders as CPython's does,
        // `<generator object g at 0x...>` / `<function f at 0x...>`, with
        // the object's own address (`pycc_rt::closure`).
        // This catch-all also swallows every `Ty` variant added later: a new
        // variant `to_str` cannot render needs its own arm above.
        _ => Ok(()),
    }
}

/// Whether a value of `ty` holds a class instance inside a list or tuple
/// (`None` when it holds none) and, if so, whether every such instance's
/// class renders through `__repr__` (a dataclass, an enum or a class
/// defining it), which `pycc_mir::class::rewrite_instance_to_text` calls as
/// `list.__repr__`/`tuple.__repr__` do.
fn instances_render(env: &Environment, ty: &Ty) -> Option<bool> {
    match ty {
        Ty::Instance(name) => Some(
            !pycc_hir::is_builtin_exception_class(name)
                && instance_text_dunder(env, name, true).is_some(),
        ),
        Ty::List(elem) => instances_render(env, elem),
        Ty::Tuple(items) => items
            .iter()
            .filter_map(|item| instances_render(env, item))
            .reduce(|a, b| a && b),
        _ => None,
    }
}

/// Whether `ty` is a list or tuple holding instances that all render by
/// their `__repr__`.
pub(crate) fn renders_by_repr(env: &Environment, ty: &Ty) -> bool {
    matches!(ty, Ty::List(_) | Ty::Tuple(_)) && instances_render(env, ty) == Some(true)
}

/// The method a class instance's text conversion calls, when its class
/// renders through one: `__str__` (falling back to `__repr__`, as
/// `object.__str__` does) for `str()`/`print`/an f-string, `__repr__` alone
/// for `repr()`. The method must be found in the class's MRO and take only
/// `self`, returning `str`; `pycc_mir` resolves the same MRO walk.
pub(crate) fn instance_text_dunder(
    env: &Environment,
    class_name: &str,
    want_repr: bool,
) -> Option<&'static str> {
    if pycc_hir::is_builtin_exception_class(class_name) || env.lookup_class(class_name).is_none() {
        return None;
    }
    let candidates: &[&'static str] = if want_repr {
        &["__repr__"]
    } else {
        &["__str__", "__repr__"]
    };
    let receiver = Ty::Instance(Box::new(class_name.to_string()));
    // The first declared candidate is the one `pycc_mir` calls: a `__str__`
    // of the wrong shape refuses the conversion rather than falling back to
    // `__repr__`, so the checked and the lowered call never differ.
    let method = candidates
        .iter()
        .copied()
        .find(|method| declares_method(env, class_name, method))?;
    matches!(
        crate::class::resolve_method_call(env, &receiver, method, &[], &[]),
        Ok(Ty::Str)
    )
    .then_some(method)
}

/// Whether a class of `class_name`'s MRO defines a method `method`: the
/// same walk `pycc_mir::class::rewrite_instance_to_text` makes, over every
/// MRO class including the builtin exception classes lowering synthesizes
/// (which declare only `__init__`), so the two never pick differently.
fn declares_method(env: &Environment, class_name: &str, method: &str) -> bool {
    env.lookup_class(class_name).is_some_and(|def| {
        def.mro.iter().any(|mro_class| {
            env.lookup_class(mro_class)
                .is_some_and(|mro_def| mro_def.methods.iter().any(|(name, _)| name == method))
        })
    })
}

/// `str(x)` / `repr(x)` of a class instance that renders through its own
/// `__str__`/`__repr__` ([`instance_text_dunder`]): `Some(Ok(str))`; `None`
/// for any other call.
pub(crate) fn instance_text_call(
    env: &Environment,
    callee: &str,
    arg_tys: &[Ty],
) -> Option<Result<Ty, Diagnostic>> {
    let want_repr = match callee {
        "str" => false,
        "repr" => true,
        _ => return None,
    };
    let [arg] = arg_tys else {
        return None;
    };
    // A list of instances renders each by its `__repr__`.
    if renders_by_repr(env, arg) {
        return Some(Ok(Ty::Str));
    }
    // A dict holding instances: the runtime cannot render them.
    if let Ty::Dict(_) = arg
        && let Err(err) = reject_unrenderable(env, arg, StringConversionSite::ConversionCall)
    {
        return Some(Err(err));
    }
    let Ty::Instance(class_name) = arg else {
        // `repr()` of a tuple, set, `None`, ...: its `str`, which `print`
        // renders (`pycc_mir` lowers it to an f-string of the value).
        // So is `str()` of an `Optional[int|float|bool]`, a tuple or a set.
        if (want_repr || matches!(arg, Ty::Optional(_) | Ty::Tuple(_) | Ty::Set(_) | Ty::FrozenSet(_)))
            && pycc_hir::builtins::resolve_call(callee, arg_tys).is_none()
            && repr_is_str(arg)
            && reject_unrenderable(env, arg, StringConversionSite::ConversionCall).is_ok()
        {
            return Some(Ok(Ty::Str));
        }
        return None;
    };
    // `str(e)` of an exception without a `__str__` of its own is its
    // message, as `print(e)` renders it (`pycc_mir`'s
    // `rewrite_exception_to_message`).
    let is_exception = pycc_hir::is_builtin_exception_class(class_name)
        || env.lookup_class(class_name).is_some_and(|class| {
            class.exception_type_tag.is_some()
                || class.mro.iter().any(|entry| pycc_hir::is_builtin_exception_class(entry))
        });
    if is_exception {
        let has_str = !pycc_hir::is_builtin_exception_class(class_name)
            && instance_text_dunder(env, class_name, want_repr).is_some();
        return (!want_repr && !has_str).then_some(Ok(Ty::Str));
    }
    Some(match instance_text_dunder(env, class_name, want_repr) {
        Some(_) => Ok(Ty::Str),
        None => Err(unrenderable_instance(
            class_name,
            StringConversionSite::ConversionCall,
            false,
        )),
    })
}

/// Whether `repr(x)` equals `str(x)` for every value of `ty` (everything
/// but `str` itself and a value that may hold one at the top level).
pub(crate) fn repr_is_str(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::None => true,
        Ty::Optional(inner) => !matches!(**inner, Ty::Str) && repr_is_str(inner),
        Ty::Int | Ty::Float | Ty::Bool => true,
        _ => false,
    }
}

/// Gates a string-conversion *expression*: [`reject_unrenderable`] on its
/// inferred type, then again on the value under every erased `cast`.
///
/// `pycc_mir` erases `cast(T, v)` to `v` and its `__repr__` rewrite keys on
/// the erased value's own class, so a representation-preserving upcast from
/// a non-dataclass subclass to a `@dataclass` base (`print(cast(Base, d))`
/// with `class Derived(Base): pass`) would pass a type-only gate as `Base`
/// and still reach codegen's `to_str` panic as a `Derived`. Only a call that
/// `infer_expr_in` itself routes to `check_cast` is looked through -- a user
/// `def cast` shadows the builtin there too -- and the descent repeats for a
/// nested `cast(A, cast(B, v))`. The value's own type is inferred exactly as
/// `check_cast` infers it, so no new diagnostic can surface from the descent.
pub(crate) fn reject_unrenderable_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
    site: StringConversionSite,
) -> Result<(), Diagnostic> {
    let ty = infer_expr_in(env, local_names, expr)?;
    reject_unrenderable(env, &ty, site)?;
    let mut current = expr;
    while let HirExpr::Call { callee, args } = current
        && callee == "cast"
        && env.lookup_function(callee).is_none()
    {
        // `infer_expr_in` above already ran `check_cast`, which rejects any
        // arity other than two and re-infers this very value before this
        // point, so the value's own inference cannot fail here.
        let value = &args[1];
        let value_ty = infer_expr_in(env, local_names, value)
            .expect("check_cast already inferred the cast value successfully");
        reject_unrenderable(env, &value_ty, site)?;
        current = value;
    }
    Ok(())
}

/// The `C0001` for a class instance neither MIR rewrite renders.
///
/// Reported at this crate's conventional `(0, 0)` span (rendered `1:1` by the
/// CLI): `pycc_types` carries no expression spans, and D-233's HIR-level
/// syntactic scan cannot apply because the value's *type* is unknown at HIR.
///
/// The help depends on why the instance is unrenderable. A class under any
/// other name becomes renderable by adding `@dataclass`. A user class
/// declared under one of the builtin exception names is rejected by name
/// before its shape is consulted, so `@dataclass` cannot help it (the
/// predicate never reaches the dataclass check for such a name); the only
/// remedy that works is renaming the class so it no longer shadows the
/// builtin.
fn unrenderable_instance(
    class_name: &str,
    site: StringConversionSite,
    shadows_builtin_exception: bool,
) -> Diagnostic {
    let help = if shadows_builtin_exception {
        format!(
            "print the instance's attributes individually, or rename `{class_name}` so it \
             no longer shadows the builtin exception `{class_name}`; adding `@dataclass` \
             does not make a class under a builtin exception name renderable"
        )
    } else {
        format!(
            "print the instance's attributes individually, or declare `{class_name}` with \
             `@dataclass` (or give it a `__repr__(self) -> str`)"
        )
    };
    Diagnostic::error(
        "C0001",
        format!(
            "string conversion of a `{class_name}` instance as {} is not supported yet; \
             `print()` and f-string interpolation can render only an instance whose class defines \
             `__str__`/`__repr__` returning `str` \
             or a caught builtin exception",
            site.describe()
        ),
        Span::new(0, 0),
    )
    .with_help(help)
}

/// The `C0001` for a protocol-typed value; see [`reject_unrenderable`].
fn unrenderable_protocol(protocol_name: &str, site: StringConversionSite) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "string conversion of a value typed as protocol `{protocol_name}` as {} is not \
             supported yet; the concrete class is not known at the conversion site",
            site.describe()
        ),
        Span::new(0, 0),
    )
}

/// Whether `ty` is or holds a class instance (or a CPython object).
fn mentions_instance(ty: &Ty) -> bool {
    match ty {
        Ty::Instance(_) | Ty::Protocol(_) | Ty::Object => true,
        Ty::List(e) | Ty::Set(e) | Ty::FrozenSet(e) | Ty::VarTuple(e) | Ty::Optional(e) => {
            mentions_instance(e)
        }
        Ty::Dict(kv) => mentions_instance(&kv.0) || mentions_instance(&kv.1),
        Ty::Tuple(items) => items.iter().any(mentions_instance),
        _ => false,
    }
}

#[cfg(test)]
#[path = "string_conversion_tests.rs"]
mod tests;
