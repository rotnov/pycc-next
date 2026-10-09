//! Class, MRO, and compile-time `isinstance`/`issubclass` lowering (#546):
//! the attribute-slot layout walk, `super()`/`__repr__` resolution helpers,
//! and the constant-folding of both class-predicate builtins.

use super::exception::exception_type_tag;
use super::exception_isinstance::{isinstance_tags, tests_by_tag};
use super::{HirClassDef, MirExpr, lookup, lower_expr, mro_class_def};
use pycc_hir::{
    HirExpr, Ty, eval_isinstance_single, eval_issubclass_single, extract_class_names,
    is_builtin_type_name,
};
use std::collections::HashMap;

/// Resolves `expr`'s own `Ty::Instance` payload to its declared
/// `HirClassDef`, shared by `HirExpr::AttrGet`/`MethodCall`'s lowering in
/// `expr.rs` and `HirStmt::AttrSet`'s in `stmt.rs`. Panics (never a `Result`,
/// matching this crate's own established "pycc_types::check should have
/// rejected this" convention -- see `lookup`'s own doc comment) when `expr`
/// isn't instance-typed at all, or names a class this module's own `classes`
/// table doesn't have: both are impossible from a program
/// `pycc_types::check` (or `check_and_resolve`) already accepted, so only a
/// hand-built `MirExpr`/`HirModule` bypassing that validation (e.g. this
/// crate's own internal-error tests under `tests/`) can reach either panic.
/// #378 (PR-18): If `expr` is a class-instance-typed expression whose
/// class has a `__repr__` method (found via the MRO), rewrites it to a
/// `MirExpr::Call` to that `__repr__` method, passing the original
/// expression as the `self` argument. The result type is `Ty::Str`. If
/// the expression is not an instance or the class has no `__repr__`,
/// returns the original expression unchanged.
///
/// This is used by `HirExpr::FString`'s interpolation lowering and
/// `HirExpr::Call`'s `print` argument lowering so the codegen's `to_str`
/// receives a `str` scalar (from the `__repr__` call's return value)
/// instead of an `Instance` scalar (which would panic in `to_str`).
pub(super) fn rewrite_instance_to_repr(
    expr: &MirExpr,
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    let Ty::Instance(class_name) = expr.ty() else {
        return expr.clone();
    };
    let Some(class_def) = classes.get(class_name.as_str()) else {
        return expr.clone();
    };
    // #378 (PR-18): only rewrite for dataclass classes, whose
    // compiler-synthesized `__repr__` has a known-correct signature
    // `(self) -> str`. A user-defined `__repr__` on a non-dataclass class
    // may have a different arity or return type, which would cause a
    // codegen panic if rewritten to a call here. Non-dataclass instances
    // pass through unchanged. The real gate is upstream (#977, D-237):
    // `pycc_types::string_conversion` rejects with `C0001` every `print`
    // argument and f-string interpolation that neither this rewrite nor
    // `rewrite_exception_to_message` renders -- a conservative
    // under-approximation, stricter than MIR's own resolution for a user
    // class (plain or `@dataclass`) declared under any builtin
    // exception name and for a tagged user exception class instantiated
    // as a value -- so a widening here must widen the checker first, or
    // the widened shape can never reach this function.
    if !class_def.is_dataclass {
        return expr.clone();
    }
    let repr_mangled = class_def.mro.iter().find_map(|mro_class| {
        // Every class in the MRO was registered when the class was lowered;
        // using `.expect` (whose panic path lives in libcore, outside this
        // crate's instrumented regions) avoids a `?` whose `None` branch is
        // structurally unreachable and would show up as a permanently
        // uncovered region under D-014's 100% coverage gate.
        let mro_def = classes
            .get(mro_class.as_str())
            .expect("MRO class must be registered");
        mro_def
            .methods
            .iter()
            .find(|(mn, _)| mn == "__repr__")
            .map(|(_, mangled)| mangled.clone())
    });
    // A dataclass always has a synthesized `__repr__` in its MRO (the
    // `is_dataclass` guard above ensures we only reach here for dataclass
    // classes). Using `.expect` (whose panic path lives in libcore,
    // outside this crate's instrumented regions) avoids a `match` whose
    // `None` arm is structurally unreachable for a dataclass and would
    // show up as a permanently uncovered region under D-014's 100%
    // coverage gate.
    let repr_mangled = repr_mangled.expect("dataclass must have __repr__");
    MirExpr::Call {
        callee: repr_mangled,
        args: vec![expr.clone()],
        ty: Ty::Str,
    }
}

/// Part 3A of #541 (#736): if `expr` is typed as a registered exception
/// class (a builtin like `ValueError`, one of the `OSError` family, or a
/// user-defined exception class -- anything [`exception_type_tag`] resolves,
/// which is exactly the discriminator `pycc_mir::exception`'s own handler-
/// matching logic uses), rewrites it to a `MirExpr::ExceptionMessage`
/// wrapping the original expression. The result type is `Ty::Str`. If the
/// expression is not an instance, or names a class that isn't a registered
/// exception, returns the original expression unchanged -- exactly mirroring
/// [`rewrite_instance_to_repr`]'s own three early-return shapes (non-
/// instance, unregistered class name, and here, "registered but not an
/// exception").
///
/// This sibling of `rewrite_instance_to_repr` is applied at the identical
/// two call sites (`HirExpr::FString` interpolation and a `print` call
/// argument in `expr.rs`), and BEFORE that function is tried at each site,
/// so an exception-typed value is rendered as its message rather than
/// falling through to (and being rejected by, or mis-rewritten by) the
/// dataclass `__repr__` path -- a caught exception's `Ty::Instance` payload
/// is never itself a dataclass.
pub(super) fn rewrite_exception_to_message(
    expr: &MirExpr,
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    let Ty::Instance(class_name) = expr.ty() else {
        return expr.clone();
    };
    if exception_type_tag(class_name.as_str(), classes).is_none() {
        return expr.clone();
    }
    MirExpr::ExceptionMessage(Box::new(expr.clone()))
}

pub(super) fn class_def_of<'c>(
    expr: &MirExpr,
    classes: &'c HashMap<String, HirClassDef>,
) -> &'c HirClassDef {
    // #380 (PR-20): protocol-typed expressions resolve to the protocol's
    // own class def. This is used for method/attribute resolution on
    // protocol-typed variables that were not monomorphized (e.g. a
    // protocol-typed local variable inside a function body that doesn't
    // take a protocol parameter). A protocol's own class def declares no
    // methods, so a method call on a genuinely protocol-typed value would
    // panic in `expr.rs`; that used to be reachable through a `-> P`
    // function's result until `pycc_hir` started rejecting the annotation
    // with `C0001` (#934). A protocol-typed *variable* is bound to its
    // value's concrete type before it gets here (`stmt.rs`).
    let ty = expr.ty();
    let class_name = match &ty {
        Ty::Instance(name) => name.as_str(),
        Ty::Protocol(name) => name.as_str(),
        other => panic!(
            "pycc_mir: internal error: expected an instance- or protocol-typed expression, found `{}` -- pycc_types::check should have rejected this HIR before it reached pycc_mir",
            other.name()
        ),
    };
    classes.get(class_name).unwrap_or_else(|| {
        panic!(
            "pycc_mir: internal error: class `{class_name}` has no registered HirClassDef -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
        )
    })
}

/// #433: Builds a `MirExpr::Name` for the current method's `self` parameter,
/// looked up from the innermost scope. Used by `super().method()` and
/// `super().attr` lowering to pass the most-derived instance as the implicit
/// first argument / attribute base. Panics if `self` is not bound in the
/// current scope (impossible for a method body that reached MIR lowering —
/// `pycc_hir` always includes `self` as the first parameter of a method).
pub(super) fn self_expr(scopes: &[HashMap<String, Ty>]) -> MirExpr {
    let ty = lookup(scopes, "self");
    MirExpr::Name {
        name: "self".to_string(),
        ty,
    }
}

/// #432: Computes the flat attribute-slot layout for a class by walking its
/// MRO, delegating the walk itself to `pycc_hir::flat_attr_layout`.
///
/// The resulting flat `(name, ty)` list's indices are the slot indices used
/// at runtime -- the instance is allocated with exactly this many slots
/// (`InstantiateExpr::slot_names`), and every `AttrGet`/`AttrSet` resolves its slot index
/// against this flat layout, not the individual class's own `attrs` list.
/// `pycc_hir`'s #969 layout gate computes the same layout to reject a
/// multiple-inheritance shape whose bases' layouts cannot all be embedded
/// in the derived one, so the walk is defined once, in `pycc_hir`.
pub(super) fn mro_attrs(
    class_def: &HirClassDef,
    classes: &HashMap<String, HirClassDef>,
) -> Vec<(String, Ty)> {
    // Collect the MRO defs once (verifying all classes exist) so the
    // defensive panic is exercised here rather than inside the shared
    // layout walk, then delegate the walk itself to `pycc_hir` so the two
    // crates cannot drift on what counts as a slot (#969).
    let mro_defs: Vec<&HirClassDef> = class_def
        .mro
        .iter()
        .map(|mro_class| mro_class_def(mro_class, classes))
        .collect();
    pycc_hir::flat_attr_layout(&mro_defs)
}

/// #1388: the source name of class `name`, as CPython's `type(x).__name__`
/// spells it. A class `pycc_types` monomorphised from a generic one is named
/// `0gen_<G>__<T>_<concrete>`; its source name is the generic class `G`
/// whose own type parameter `T` that mangling starts with. When several
/// generic classes match (`G` and a `G__T_x` that happens to share the
/// prefix), the longest name wins -- two matches of equal length are
/// prefixes of the same string and so the same name -- and the answer does
/// not depend on map order. Every other class is its own source name.
pub(super) fn class_source_name(name: &str, classes: &HashMap<String, HirClassDef>) -> String {
    let Some(mangled) = name.strip_prefix("0gen_") else {
        return name.to_string();
    };
    classes
        .iter()
        .filter(|(generic, def)| {
            def.type_param
                .as_deref()
                .is_some_and(|param| mangled.starts_with(&format!("{generic}__{param}_")))
        })
        .map(|(generic, _)| generic)
        .max_by_key(|generic| generic.len())
        .map_or_else(|| mangled.to_string(), Clone::clone)
}

// ---------------------------------------------------------------------------
// Issue #435: compile-time `isinstance`/`issubclass` MIR lowering.
//
// Both builtins are evaluated at compile time (pycc's static dispatch model
// means every variable's runtime type is exactly its declared type), emitting
// `MirExpr::BoolLiteral(result)` constants, with one exception (#1337): a
// value statically typed as a seeded builtin exception class is a runtime
// exception object whose class may be any subclass, so `isinstance` against a
// target outside its static MRO reads the object's type tag
// (`MirExpr::ExceptionTypeTest`, see `exception_isinstance`).
// ---------------------------------------------------------------------------

/// #435: Lowers `isinstance(obj, class_arg)` to a compile-time boolean
/// constant. The object expression is lowered to MIR (to extract its type
/// via `.ty()`), but the class argument is NOT lowered — it is a class
/// reference, not a value. The result is computed using
/// `eval_isinstance_single` with the object's class MRO.
pub(super) fn lower_isinstance(
    args: &[HirExpr],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    // The type checker already validated arg count and class names. If we
    // reach here, args has exactly 2 elements and args[1] is a valid class
    // name or tuple of class names.
    // #1476: a narrowed `object` read is tested as the object it is, so a
    // nested guard (`isinstance(o, bool)` under `isinstance(o, int)`) still
    // asks CPython rather than folding over the narrowed static type.
    let obj =
        super::object_narrow::object_operand(lower_expr(&args[0], scopes, classes, current_class));
    let obj_ty = obj.ty();
    // Part 1 of #1371: a CPython object is tested at run time; the fold
    // below reads only the static type and would answer `False`.
    if obj_ty == Ty::Object {
        return super::obj_compare::lower_object_isinstance(
            obj,
            &args[1],
            scopes,
            classes,
            current_class,
        );
    }
    // Extract class names from the second argument.
    let class_names = extract_class_names(&args[1]).expect(
        "pycc_mir: internal error: isinstance's second argument was not validated by pycc_types",
    );
    // Compute the result: true if any target class matches.
    let obj_mro = match &obj_ty {
        Ty::Instance(class_name) => classes
            .get(class_name.as_str())
            .map(|cd| cd.mro.as_slice())
            .unwrap_or(&[]),
        _ => &[],
    };
    // #1337 (WI-6a): a caught builtin exception's dynamic class may be any
    // subclass, so a target outside its static MRO is tested by tag.
    let by_tag = matches!(&obj_ty, Ty::Instance(name) if tests_by_tag(name, classes));
    let mut tags: Vec<u8> = Vec::new();
    let result = class_names.iter().any(|target| {
        // #380 (PR-20, PEP 544): if the target is a protocol class, use
        // structural conformance checking instead of nominal MRO
        // membership. The type checker already validated that the
        // protocol is `@runtime_checkable`.
        if let Some(target_def) = classes.get(target.as_str())
            && target_def.is_protocol
        {
            return eval_isinstance_protocol(&obj_ty, target_def, classes);
        }
        let statically = eval_isinstance_single(&obj_ty, target, obj_mro);
        if !statically && by_tag {
            tags.extend(isinstance_tags(target, classes));
        }
        statically
    });
    if result || tags.is_empty() {
        return MirExpr::BoolLiteral(result);
    }
    tags.sort_unstable();
    tags.dedup();
    MirExpr::ExceptionTypeTest {
        obj: Box::new(obj),
        tags,
    }
}

/// #380 (PR-20, PEP 544): Evaluates `isinstance(obj, Protocol)` at
/// compile time using structural conformance. Returns `true` if the
/// object's class has all the protocol's required members with
/// compatible types.
pub(super) fn eval_isinstance_protocol(
    obj_ty: &Ty,
    proto_def: &HirClassDef,
    classes: &HashMap<String, HirClassDef>,
) -> bool {
    let Ty::Instance(class_name) = obj_ty else {
        return false;
    };
    let Some(class_def) = classes.get(class_name.as_str()) else {
        return false;
    };
    // Check each protocol member against the class's members (through
    // its MRO).
    use pycc_hir::ProtocolMember;
    for member in &proto_def.protocol_members {
        match member {
            ProtocolMember::Method {
                name: method_name,
                param_tys: proto_param_tys,
                return_ty: proto_return_ty,
            } => {
                // Look up the method through the MRO.
                let found = class_def.mro.iter().find_map(|mro_class| {
                    // An MRO entry may refer to a class not present in the
                    // `classes` map (e.g. a ghost base in unit tests).
                    // `filter_map` skips such entries, matching the
                    // `None`-is-not-found semantics of the attribute arm.
                    let mro_def = classes.get(mro_class.as_str())?;
                    let mangled = mro_def
                        .methods
                        .iter()
                        .find(|(n, _)| n == method_name)
                        .map(|(_, m)| m.as_str())?;
                    // The function signature is not available in MIR's
                    // `classes` table (it only has `HirClassDef`, not
                    // the `Environment`). For `@runtime_checkable`
                    // protocols, PEP 544 specifies that only the
                    // *presence* of attributes/methods is checked, not
                    // their types. This matches CPython's own
                    // `@runtime_checkable` behavior (it only checks
                    // for the presence of attributes, not their types).
                    Some(mangled)
                });
                if found.is_none() {
                    return false;
                }
                // For runtime_checkable, we only check presence, not
                // type compatibility (matching CPython's behavior).
                let _ = (proto_param_tys, proto_return_ty);
            }
            ProtocolMember::Attribute {
                name: attr_name, ..
            } => {
                // Look up the attribute through the MRO.
                let found = class_def.mro.iter().any(|mro_class| {
                    // An MRO entry may refer to a class not present in the
                    // `classes` map (e.g. a ghost base in unit tests).
                    // `is_some_and(…)` treats missing entries as not
                    // having the attribute, matching the method arm's
                    // `find_map` skip semantics.
                    classes.get(mro_class.as_str()).is_some_and(|mro_def| {
                        mro_def.attrs.iter().any(|(n, _)| n == attr_name)
                            || mro_def.properties.iter().any(|p| &p.name == attr_name)
                            // #914: a class-level attribute satisfies the
                            // member too. `check_protocol_conformance` now
                            // accepts one, and this arm must stay consistent
                            // with it (#380 W2) -- otherwise `isinstance`
                            // folds to `False` for a class the checker just
                            // certified as conforming.
                            || mro_def.class_attrs.iter().any(|(n, _, _)| n == attr_name)
                    })
                });
                if !found {
                    return false;
                }
            }
        }
    }
    true
}

/// #435: Lowers `issubclass(cls_arg, class_arg)` to a compile-time boolean
/// constant. Neither argument is lowered as a MIR expression — both are
/// class references. The result is computed using `eval_issubclass_single`
/// with the source class's MRO.
pub(super) fn lower_issubclass(
    args: &[HirExpr],
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    // The type checker already validated arg count and class names. If we
    // reach here, args has exactly 2 elements, args[0] is a bare class name,
    // and args[1] is a valid class name or tuple of class names.
    let cls_name = match &args[0] {
        HirExpr::Name(name) => name.as_str(),
        _ => unreachable!(
            "pycc_mir: internal error: issubclass's first argument was not validated by pycc_types"
        ),
    };
    let target_names = extract_class_names(&args[1]).expect(
        "pycc_mir: internal error: issubclass's second argument was not validated by pycc_types",
    );
    // Get the source class's MRO (empty for builtin types).
    let cls_mro = if is_builtin_type_name(cls_name) {
        &[][..]
    } else {
        classes
            .get(cls_name)
            .map(|cd| cd.mro.as_slice())
            .unwrap_or(&[])
    };
    let result = target_names
        .iter()
        .any(|target| eval_issubclass_single(cls_name, target, cls_mro));
    MirExpr::BoolLiteral(result)
}
