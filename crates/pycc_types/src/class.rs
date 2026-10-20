//! Class-body type-checking (D-154, Part 1 of #375): resolving instance
//! instantiation, attribute access, and method calls against
//! `Environment`'s class table.
//!
//! Unlike a hand-rolled per-class checking pass, every method (including
//! `__init__`) is already an ordinary `HirItem::Function` under its mangled
//! `<ClassName>.<method_name>` name (see `pycc_hir::class`'s own doc
//! comment) -- so `check_function_in`/`check_generic_function_in` and the
//! constraint solver already check a method body exactly like any other
//! function, with `self` bound to `Ty::Instance(class_name)` by the same
//! ordinary parameter-binding logic every other parameter goes through. The
//! functions in this module are only the *additional* pieces that ordinary
//! function-checking has no shape for: resolving `ClassName(...)`
//! (instantiation), `base.attr` (an instance attribute read or write), and
//! `base.method(...)` (an instance method call) against the class's
//! declared shape.

mod attr_set;
mod binding;
pub(crate) mod foreign_static;
mod method_call;
mod method_defaults;
mod receiver_class_call;
mod static_call;
mod super_call;
mod cast;
mod protocol;
pub(crate) use cast::*;
pub(crate) use protocol::*;

use crate::{Environment, infer_expr_in, is_assignable};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    HirExpr, ProtocolMember, Ty, declares_name_outside_class_attrs, extract_class_names,
    is_builtin_type_name,
};

// A private import, not a re-export: it keeps this module's own unqualified
// `expect_class` call sites compiling now that the function itself lives in
// `class/binding.rs`.
use binding::expect_class;
// A re-export, so every existing `class::bind_classes` /
// `class::resolve_instantiation` call site (and the doc comments in `lib.rs`
// naming those paths) keeps working across the `class/binding.rs` extraction.
pub(crate) use binding::{bind_classes, resolve_instantiation};
// A re-export, so every existing `class::check_attr_set` call site keeps
// working across the `class/attr_set.rs` extraction (#1219).
pub(crate) use attr_set::check_attr_set;
// A re-export, so every existing `class::resolve_method_call` call site
// (`expr.rs`'s `HirExpr::MethodCall` arm) keeps working across the
// `class/method_call.rs` extraction (#815, Part 1 of #737).
pub(crate) use method_call::resolve_method_call;
// #1411: `expr.rs`'s `HirExpr::ReceiverClassCall` arm.
pub(crate) use receiver_class_call::infer_receiver_class_call;
// Re-exports for the same reason, across the `class/static_call.rs` and
// `class/super_call.rs` extractions (#549): `expr.rs` and
// `crates/pycc_types/src/tests/init_rank.rs` both name these through
// `class::`.
pub(crate) use static_call::{has_static_or_class_method, resolve_static_or_class_method_call};
pub(crate) use super_call::{resolve_super_attr_get, resolve_super_method_call};

/// Looks up a method through the MRO, returning its parameter types
/// (including `self`) and return type. Walks the MRO most-derived-first
/// and returns the first match.
fn lookup_method_through_mro(
    env: &Environment,
    mro: &[String],
    method_name: &str,
) -> Option<(Vec<(Ty, String)>, Ty)> {
    for mro_class in mro {
        // Every class in the MRO was defined earlier in the module and
        // is present in the `Environment`'s class table — the C3 MRO is
        // built from already-lowered class definitions. Using `.expect`
        // (whose panic path lives in libcore, outside this crate's
        // instrumented regions) avoids a permanently-uncovered `?`-None
        // region under D-014's 100 %-coverage gate.
        let mro_def = env
            .lookup_class(mro_class)
            .expect("MRO classes are always in the environment");
        if let Some(mangled) = mro_def
            .methods
            .iter()
            .find(|(name, _)| name == method_name)
            .map(|(_, mangled)| mangled.as_str())
            && let Some((param_tys, return_ty)) = env.lookup_function(mangled)
        {
            // `param_tys` is `Vec<Ty>` (just types, no names). We
            // pair each with an empty name since conformance
            // checking only needs the types. The `self` parameter
            // is identified by position (first), not by name.
            let named: Vec<(Ty, String)> = param_tys
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    (
                        ty.clone(),
                        if i == 0 {
                            "self".to_string()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect();
            return Some((named, return_ty.clone()));
        }
    }
    None
}

/// Looks up an attribute type through the MRO. Walks the MRO
/// most-derived-first and returns the first match.
fn lookup_attr_through_mro(env: &Environment, mro: &[String], attr_name: &str) -> Option<Ty> {
    for mro_class in mro {
        // Same invariant as `lookup_method_through_mro` above: every
        // class in the MRO is present in the `Environment`'s class table.
        let mro_def = env
            .lookup_class(mro_class)
            .expect("MRO classes are always in the environment");
        if let Some((_, ty)) = mro_def.attrs.iter().find(|(name, _)| name == attr_name) {
            return Some(ty.clone());
        }
        // A @property satisfies an attribute requirement just as a direct
        // attribute does — `eval_isinstance_protocol` checks both, and the
        // conformance check must be consistent with it (#380 W2). Since #914
        // a class-level attribute is a third such source; it is matched by
        // `check_protocol_conformance`'s own fallback to
        // `lookup_class_attr_through_mro` rather than here, because this
        // walk is also the instance-attribute path.
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr_name) {
            let (_, return_ty) = env.lookup_function(&prop.getter).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: property getter `{}` is in class `{mro_class}`'s \
                     own property table but was not registered as an ordinary function",
                    prop.getter
                )
            });
            return Some(return_ty.clone());
        }
    }
    None
}

/// #380 (PR-20): Checks assignability with protocol conformance support.
/// Delegates to `is_assignable` for non-protocol cases. When `to` is
/// `Ty::Protocol(name)` and `from` is `Ty::Instance(class_name)`, checks
/// structural conformance via `check_protocol_conformance`. Returns
/// `true` if assignable, `false` otherwise. For detailed error messages,
/// call `check_protocol_conformance` directly.
pub(crate) fn is_assignable_env(env: &Environment, from: &Ty, to: &Ty) -> bool {
    // Protocol conformance: Instance -> Protocol
    if let (Ty::Instance(class_name), Ty::Protocol(protocol_name)) = (from, to) {
        return check_protocol_conformance(env, class_name, protocol_name).is_ok();
    }
    // Protocol to Protocol: same protocol or inherited
    if let (Ty::Protocol(from_name), Ty::Protocol(to_name)) = (from, to) {
        if from_name == to_name {
            return true;
        }
        // Check if from_name's MRO includes to_name
        if let Some(from_def) = env.lookup_class(from_name)
            && from_def.mro.iter().any(|m| m.as_str() == to_name.as_str())
        {
            return true;
        }
        return false;
    }
    // For all other cases, use the plain `is_assignable`.
    is_assignable(from.clone(), to.clone())
}

/// #380 (PR-20): Returns a `T0046` diagnostic for a protocol conformance
/// failure, or a `T0021` for a general type mismatch. Called when
/// `is_assignable_env` returns `false` to produce a detailed error.
pub(crate) fn assignable_error(env: &Environment, from: &Ty, to: &Ty) -> Diagnostic {
    if let (Ty::Instance(class_name), Ty::Protocol(protocol_name)) = (from, to) {
        // `assignable_error` is only called when `is_assignable_env`
        // returned `false`, which means `check_protocol_conformance`
        // already returned `Err`.  Using `.expect()` (whose panic path
        // lives in libcore, outside this crate's instrumented regions)
        // avoids a permanently-uncovered `unwrap_or_else` closure under
        // D-014's 100 %-region coverage gate.
        return check_protocol_conformance(env, class_name, protocol_name)
            .expect_err("is_assignable_env already returned false, so check_protocol_conformance must return Err");
    }
    if let (Ty::Protocol(from_name), Ty::Protocol(to_name)) = (from, to)
        && from_name != to_name
    {
        return Diagnostic::error(
            "T0046",
            format!("protocol `{from_name}` does not conform to protocol `{to_name}`"),
            Span::new(0, 0),
        );
    }
    Diagnostic::error(
        "T0021",
        format!(
            "type mismatch: `{}` is not assignable to `{}`",
            from.name(),
            to.name()
        ),
        Span::new(0, 0),
    )
}

fn t0043_not_an_instance(action: &str, ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "T0043",
        format!(
            "cannot {action} on `{}`: it is not a class instance",
            ty.name()
        ),
        Span::new(0, 0),
    )
}

fn t0044_unknown_member(kind: &str, class_name: &str, member: &str) -> Diagnostic {
    // The methods `pycc_ast::desugar::dataclasses` gives a dataclass for
    // `dataclasses.asdict()` and friends: missing when the class is not a
    // dataclass or its fields cannot be converted.
    let function = [
        ("_pycc_asdict_", "asdict"),
        ("_pycc_astuple_", "astuple"),
        ("_pycc_fields_", "fields"),
        ("_pycc_replace_", "replace"),
    ]
    .into_iter()
    .find(|(prefix, _)| member.starts_with(prefix));
    if let Some((_, function)) = function {
        let mut diagnostic = Diagnostic::error(
            "C0001",
            format!(
                "`dataclasses.{function}()` of a `{class_name}` is valid Python but not \
                 implemented yet"
            ),
            Span::new(0, 0),
        );
        diagnostic = diagnostic.with_help(match function {
            "asdict" => "compiled for a dataclass whose fields are scalars, enums, dataclasses \
                         defined before it, or lists, dicts, sets, tuples or optionals of those, \
                         and in strict mode only when every converted value has one type \
                         (`--non-strict` builds a dict of objects)",
            "astuple" => "compiled for a dataclass whose fields are scalars, enums, dataclasses \
                          defined before it, or lists, dicts, sets, tuples or optionals of those",
            "replace" => "compiled for a dataclass with its generated `__init__`, given keyword \
                          arguments naming `init` fields",
            _ => "compiled for a dataclass instance or class",
        });
        return diagnostic;
    }
    Diagnostic::error(
        "T0044",
        format!("class `{class_name}` has no {kind} named `{member}`"),
        Span::new(0, 0),
    )
}

/// #587: `super().<attr>` naming an *instance* attribute — one established
/// by `self.<attr> = ...` inside `__init__`, and therefore living in the
/// instance's own slot rather than on any class in the MRO. CPython's
/// `super` object proxies class-level attributes and descriptors only, so
/// this raises `AttributeError` there; pycc used to resolve it against
/// `self`'s slot and return a value instead.
fn t0047_super_instance_attr(attr: &str, declaring_class: &str) -> Diagnostic {
    Diagnostic::error(
        "T0047",
        format!(
            "`super().{attr}` is not readable: `{attr}` is an instance attribute of class \
             `{declaring_class}`, and `super()` proxies class-level attributes and \
             descriptors along the MRO, not the instance's own attributes -- CPython raises \
             `AttributeError: 'super' object has no attribute '{attr}'` here"
        ),
        Span::new(0, 0),
    )
    // #1181: the receiver's *source* spelling never crosses into this crate
    // -- `pycc_hir::class::receiver` lowers it under the canonical name
    // `self` precisely so nothing here has to know it -- so the help names
    // the receiver without asserting how it is spelled. Naming `self` would
    // tell a user whose receiver is spelled `this` to write the one thing
    // that crate's own occurrence guard forbids.
    .with_help(format!(
        "read it through the method's own receiver instead: `<receiver>.{attr}`"
    ))
}

/// Validates a call's arguments against a resolved `(param_tys, return_ty)`
/// signature, reusing this crate's own existing "call to undefined
/// function"-adjacent diagnostic shape (`T0021`, `infer_expr_in`'s own
/// `HirExpr::Call` arm) rather than inventing a class-specific arity/type
/// mismatch code -- an instantiation call and a method call are both, at
/// their core, "call this mangled function with these arguments," the same
/// shape an ordinary function call already validates.
///
/// #953: `structural` selects the assignability predicate applied to each
/// argument. `true` uses the environment-aware
/// [`is_assignable_env`], so a concrete class instance is accepted for a
/// protocol-typed parameter when it structurally conforms (PEP 544);
/// `false` keeps the plain nominal [`is_assignable`]. Only the
/// instance-method-call sites (`class/method_call.rs`,
/// `class/method_defaults.rs`) pass `true`: the
/// static-method, class-method, `super()`, constructor and exception-
/// constructor sites still pass `false`, because accepting a conforming
/// concrete argument there needs protocol monomorphization support that
/// does not exist yet for those lowering shapes, and flipping one of them
/// without that support produces a `pycc_mir`/`pycc_codegen` panic rather
/// than a working program (see the #953 follow-up issue). The diagnostics
/// themselves -- both the arity `T0021` and the per-argument `T0021` --
/// are identical either way: a non-conforming argument keeps today's
/// message, and `T0046` (`assignable_error`) stays reserved for the
/// assignment/conformance positions that already use it.
///
/// Independently of `structural`, every site admits a native value boxed
/// into an `object` parameter ([`crate::object_box::admits_value`], Part 2
/// of #1387): codegen's one argument-marshalling helper boxes it for every
/// call shape, so no site needs a lowering of its own. `args` are the
/// call's argument expressions, in `arg_tys` order, so a class method's
/// own `cls` is refused (`I0404`) at every call shape. It is shorter than
/// `arg_tys` when `method_defaults` filled trailing defaults in; a filled
/// default is appended unboxed, so it never boxes here and must be
/// assignable as it stands.
pub(crate) fn check_call_args(
    env: &Environment,
    callee: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
    param_tys: &[Ty],
    structural: bool,
) -> Result<(), Diagnostic> {
    if arg_tys.len() != param_tys.len() {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects {} argument(s), got {}",
                param_tys.len(),
                arg_tys.len()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("pass exactly {} argument(s)", param_tys.len())));
    }
    for (i, (arg_ty, param_ty)) in arg_tys.iter().zip(param_tys.iter()).enumerate() {
        let assignable = if structural {
            is_assignable_env(env, arg_ty, param_ty)
        } else {
            is_assignable(arg_ty.clone(), param_ty.clone())
        } || args.get(i).map_or(Ok(false), |arg| {
            crate::object_box::admits_value(env, arg, arg_ty, param_ty)
        })? || crate::non_strict::admits_coercion(arg_ty, param_ty)
            || crate::non_strict::admits_instance_coercion(env, arg_ty, param_ty);
        if !assignable {
            let diagnostic = Diagnostic::error(
                "T0021",
                format!(
                    "argument {} of `{callee}` expects `{}`, got `{}`",
                    i + 1,
                    param_ty.name(),
                    arg_ty.name()
                ),
                Span::new(0, 0),
            )
            .with_help(format!("pass a `{}` value", param_ty.name()));
            let at = args.get(i).and_then(crate::locate::expr_span);
            return Err(crate::locate::place_at(diagnostic, at));
        }
    }
    Ok(())
}

/// Resolves `base.attr` (an instance attribute read) against `base_ty`.
/// Shared by `infer_expr_in`'s `HirExpr::AttrGet` arm and `check_stmt`'s
/// `HirStmt::AttrSet` arm (which also needs `base`'s attribute type, to
/// check the assigned value against it).
///
/// #377: a `@property` getter is checked *before* the regular attribute
/// slot table -- `obj.x` where `x` is a property resolves to the getter
/// method's return type, not a slot type. This mirrors CPython's own
/// observable behavior, where a property descriptor intercepts attribute
/// access before the instance's `__dict__`/slot table is consulted.
///
/// #432: walks the class's MRO (C3 linearization) in order, checking each
/// class's property table and attribute slots. The first class in the MRO
/// that declares the attribute (or a property of that name) wins, matching
/// CPython's own MRO-based attribute resolution.
pub(crate) fn resolve_attr_get(
    env: &Environment,
    base_ty: &Ty,
    attr: &str,
) -> Result<Ty, Diagnostic> {
    // #380 (PR-20, PEP 544): protocol-typed variable attribute access.
    // When `base_ty` is `Ty::Protocol(P)`, look up the attribute in
    // `P`'s `protocol_members`.
    if let Ty::Protocol(protocol_name) = base_ty {
        let proto_def = expect_class(env, protocol_name);
        for member in &proto_def.protocol_members {
            if let ProtocolMember::Attribute { name, ty } = member
                && name == attr
            {
                return Ok(ty.clone());
            }
        }
        return Err(t0044_unknown_member("attribute", protocol_name, attr));
    }
    let Ty::Instance(class_name) = base_ty else {
        return Err(t0043_not_an_instance("read an attribute", base_ty));
    };
    let class_def = expect_class(env, class_name);
    // #432/#377: walk the MRO for property lookup first (matching CPython's
    // descriptor protocol precedence — a property descriptor intercepts
    // attribute access before `__dict__`), across ALL classes in the MRO,
    // then fall back to regular attribute slots. This matches the MIR
    // lowering's own properties-first-across-full-MRO logic exactly,
    // avoiding a type-checker/MIR disagreement when a derived class has a
    // regular attr with the same name as a base class property.
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr) {
            let (_, return_ty) = env.lookup_function(&prop.getter).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: property getter `{}` is in class `{mro_class}`'s \
                     own property table but was not registered as an ordinary function",
                    prop.getter
                )
            });
            return Ok(return_ty.clone());
        }
    }
    // No property matched in any class — now check regular attribute slots
    // by walking the MRO in order (most-derived first, so a re-declared
    // attr uses the most-derived type).
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty)) = mro_def.attrs.iter().find(|(name, _)| name == attr) {
            return Ok(ty.clone());
        }
    }
    // #911 (Part 1 of #885): a class-level attribute (`MIN_WIDTH: int =
    // -1024`) read through an instance. Checked *after* the instance slots
    // so a real slot always wins, which is CPython's own precedence -- an
    // instance `__dict__` entry shadows a non-data-descriptor class
    // attribute.
    //
    // #960: that ordering is load-bearing, not merely defensive. `pycc_hir`'s
    // `reject_class_attr_collisions` walks only a class's *own* `class_attrs`
    // against its own MRO, so two independent sibling bases -- one
    // contributing an instance slot, the other a same-named class attribute
    // -- reach this function without ever having been compared. `pycc_mir`'s
    // instance `AttrGet` arm mirrors this exact order; folding first is what
    // #960 was.
    if let Some(ty) = lookup_class_attr_through_mro(env, class_name, attr) {
        return Ok(ty);
    }
    Err(t0044_unknown_member("attribute", class_name, attr))
}

/// #911 (Part 1 of #885): looks a class-level attribute up through
/// `class_name`'s MRO, most-derived first, returning its declared type.
///
/// A class attribute occupies no instance slot -- `pycc_mir` folds a read of
/// it to the constant recorded in `HirClassDef::class_attrs` -- so this walk
/// is deliberately separate from the `attrs` walk above rather than merged
/// into it. Since #960 that fold is reached only when the instance-slot walk
/// missed, on both sides: a same-named slot contributed by a sibling MRO base
/// wins here and in `pycc_mir` alike.
pub(crate) fn lookup_class_attr_through_mro(
    env: &Environment,
    class_name: &str,
    attr: &str,
) -> Option<Ty> {
    let class_def = expect_class(env, class_name);
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty, _)) = mro_def.class_attrs.iter().find(|(name, _, _)| name == attr) {
            return Some(ty.clone());
        }
    }
    None
}

/// #974: looks a class-level attribute up through `class_name`'s MRO for a
/// **class-name-qualified** read (`Derived.LIMIT`), most-derived first,
/// returning its declared type.
///
/// This is deliberately *not* [`lookup_class_attr_through_mro`], and the two
/// must not be merged. That function serves the instance read path and its
/// four other callers, none of which want the rule below; it iterates
/// `class_attrs` alone, so on
///
/// ```python
/// class A:
///     x: int = 2
///
///
/// class B(A):
///     @staticmethod
///     def x() -> int:
///         return 1
/// ```
///
/// it would answer `int` for `B.x` and let `pycc_mir` fold the read to `2`,
/// turning a program that compiles correctly today into a wrong-value
/// mis-compile. CPython binds `B`'s own `x` and never reaches `A`'s.
///
/// The rule implemented here instead: walk `class_name`'s MRO most-derived
/// first and return the class attribute's type only when the **first** class
/// that declares `attr` at all declares it as a class attribute. When some
/// other class-level namespace claims the name first, the read stays
/// unresolved (`None`) and the caller reports `T0044` -- pycc does not model
/// a bare, uncalled class-name-qualified method read.
/// [`declares_name_outside_class_attrs`] is the shadowing predicate, shared
/// verbatim with `pycc_mir`'s fold so the two cannot drift; its own doc
/// comment records which namespaces are checked and why instance slots and
/// protocol members are excluded.
///
/// Excluding instance slots is what makes this path differ from
/// [`resolve_attr_get`]'s #960 precedence, on purpose: a class object has no
/// instance `__dict__`. The divergence is CPython's own -- see
/// `docs/TYPE_SYSTEM.md`'s "Class-level attributes" section for the measured
/// `c.x` / `C.x` pair.
pub(crate) fn lookup_class_attr_by_class_name(
    env: &Environment,
    class_name: &str,
    attr: &str,
) -> Option<Ty> {
    let class_def = expect_class(env, class_name);
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty, _)) = mro_def.class_attrs.iter().find(|(name, _, _)| name == attr) {
            return Some(ty.clone());
        }
        if declares_name_outside_class_attrs(mro_def, attr) {
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Issue #435: compile-time `isinstance`/`issubclass` type checking.
//
// pycc uses static dispatch (D-006) — every variable's runtime type is
// exactly its declared static type, so `isinstance`/`issubclass` can always
// be evaluated at compile time. These functions validate the call's
// arguments and return `Ty::Bool` (the result is computed at MIR lowering
// time, not here — the type checker only needs to confirm the call is
// well-formed and returns `bool`).
// ---------------------------------------------------------------------------

/// Validates a class name argument to `isinstance`/`issubclass`: it must be
/// either a registered user-defined class (including protocols) or one of
/// the builtin scalar type names (`int`, `str`, `float`, `bool`). Returns
/// `Ok(())` if valid, or a `T0001` diagnostic if the name is unknown.
fn validate_class_name(env: &Environment, name: &str) -> Result<(), Diagnostic> {
    if env.lookup_class(name).is_some() || is_builtin_type_name(name) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "T0001",
            format!("`{name}` is not a known class or builtin type"),
            Span::new(0, 0),
        ))
    }
}

#[cfg(test)]
mod tests;
