//! Inference of container literals, subscripts, slices and container methods.

use super::*;

pub(crate) fn infer_list_literal_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::ListLiteral(elements) = expr else {
        unreachable!()
    };
    if let Some(result) = crate::foreign::arith::object_display_ty(
        env,
        local_names,
        pycc_hir::DisplayKind::List,
        elements,
    ) {
        return result;
    }
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
        return Err(crate::empty_container::unresolved_list());
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
                let diagnostic = Diagnostic::error(
                    "T0032",
                    format!(
                        "list element type mismatch: expected {} (from the first element), found {}",
                        expected.name(),
                        this_ty.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("use a `{}` value here", expected.name()));
                return Err(crate::locate::place_labeled(
                    diagnostic,
                    crate::locate::expr_span(element),
                    format!("expected `{}`, found `{}`", expected.name(), this_ty.name()),
                ));
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
pub(crate) fn infer_dict_literal_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::DictLiteral(pairs) = expr else {
        unreachable!()
    };
    if let Some(result) = crate::foreign::arith::object_display_ty(
        env,
        local_names,
        pycc_hir::DisplayKind::Dict,
        pairs.iter().flat_map(|(k, v)| [k, v]),
    ) {
        return result;
    }
    // #1021: same reasoning as `ListLiteral`'s own empty arm above --
    // the pre-pass already rewrote every resolvable `{}` into
    // `HirExpr::EmptyDict`, so reaching here means no evidence
    // exists.
    let Some((first_key, first_value)) = pairs.first() else {
        return Err(crate::empty_container::unresolved_dict());
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
            )
            .with_help(format!(
                "use a `{}` key and `{}` value here",
                key_ty.name(),
                val_ty.name()
            )));
        }
    }
    let dict_ty = Ty::Dict(Box::new((key_ty, val_ty)));
    // Shared with annotation lowering -- see the `ListLiteral` arm.
    pycc_hir::check_container_ty(&dict_ty, Span::new(0, 0))?;
    crate::dict_key::check_dict_key(env, &dict_ty)?;
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
pub(crate) fn infer_set_literal_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::SetLiteral(elements) = expr else {
        unreachable!()
    };
    if let Some(result) = crate::foreign::arith::object_display_ty(
        env,
        local_names,
        pycc_hir::DisplayKind::Set,
        elements,
    ) {
        return result;
    }
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
            let diagnostic = Diagnostic::error(
                "T0037",
                format!(
                    "set element type mismatch: expected {} (from the first element), found {}",
                    elem_ty.name(),
                    this_ty.name(),
                ),
                Span::new(0, 0),
            )
            .with_help(format!("use a `{}` value here", elem_ty.name()));
            return Err(crate::locate::place_labeled(
                diagnostic,
                crate::locate::expr_span(element),
                format!("expected `{}`, found `{}`", elem_ty.name(), this_ty.name()),
            ));
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
pub(crate) fn infer_tuple_literal_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::TupleLiteral(elements) = expr else {
        unreachable!()
    };
    if let Some(result) = crate::foreign::arith::object_display_ty(
        env,
        local_names,
        pycc_hir::DisplayKind::Tuple,
        elements,
    ) {
        return result;
    }
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

pub(crate) fn infer_subscript_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::Subscript { base, index } = expr else {
        unreachable!()
    };
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
    // A sequence index reads an `IntEnum` member as its `int` value
    // (`pycc_hir::int_enum`); a dict key is the member itself.
    let index_ty = if matches!(base_ty, Ty::Dict(_)) {
        index_ty
    } else {
        pycc_hir::int_enum::as_int(&index_ty)
    };
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
            if !is_assignable(index_ty.clone(), Ty::Int)
                && !crate::non_strict::admits_index(&index_ty)
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!("list index must be `int`, found `{}`", index_ty.name()),
                    Span::new(0, 0),
                )
                .with_help("use an `int` value"));
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
                // Non-strict mode: as for a store (`stmt_check`), a key of
                // another type makes the dict CPython's own.
                if let HirExpr::Name(dict) = base.as_ref() {
                    crate::non_strict::request_widen(
                        dict,
                        &Ty::Dict(Box::new((key_ty.clone(), val_ty.clone()))),
                        &Ty::Object,
                        |_| true,
                    );
                }
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "dict key type mismatch: expected `{}`, found `{}`",
                        key_ty.name(),
                        index_ty.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!("use a `{}` value here", key_ty.name())));
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
            // A computed `int` index into a tuple whose elements share one
            // `int`/`bool`/`float` type (`t[k]` over `(True, False, True)`):
            // every position has that type.
            if !matches!(index.as_ref(), HirExpr::IntLiteral(_))
                && matches!(index_ty, Ty::Int | Ty::Bool)
                && let Some(first) = elems.first()
                && matches!(first, Ty::Int | Ty::Bool | Ty::Float)
                && elems.iter().all(|elem| elem == first)
            {
                return Ok(first.clone());
            }
            let HirExpr::IntLiteral(literal_index) = index.as_ref() else {
                return Err(Diagnostic::error(
                    "T0040",
                    "tuple index must be a non-negative literal integer within range".to_string(),
                    Span::new(0, 0),
                )
                .with_help("use a literal, non-negative integer index within range"));
            };
            let Ok(literal_index) = usize::try_from(*literal_index) else {
                return Err(Diagnostic::error(
                    "T0040",
                    "tuple index must be a non-negative literal integer within range".to_string(),
                    Span::new(0, 0),
                )
                .with_help("use a literal, non-negative integer index within range"));
            };
            let Some(elem_ty) = elems.get(literal_index) else {
                return Err(Diagnostic::error(
                    "T0040",
                    "tuple index must be a non-negative literal integer within range".to_string(),
                    Span::new(0, 0),
                )
                .with_help("use a literal, non-negative integer index within range"));
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
        Ty::Str => {
            if !is_assignable(index_ty.clone(), Ty::Int)
                && !crate::non_strict::admits_index(&index_ty)
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!("string index must be `int`, found `{}`", index_ty.name()),
                    Span::new(0, 0),
                ));
            }
            Ok(Ty::Str)
        }
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
pub(crate) fn infer_slice_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::Slice {
        base,
        start,
        stop,
        step,
    } = expr
    else {
        unreachable!()
    };
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
    let (Ty::List(_) | Ty::Str) = &base_ty else {
        return Err(Diagnostic::error(
            "T0033",
            format!(
                "`{}` does not support slicing (only a list or a str does)",
                base_ty.name()
            ),
            Span::new(0, 0),
        ));
    };
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
            if !is_assignable(bound_ty.clone(), Ty::Int)
                && !crate::non_strict::admits_index(&bound_ty)
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!("slice {label} must be `int`, got `{}`", bound_ty.name()),
                    Span::new(0, 0),
                )
                .with_help("use an `int` value"));
            }
        }
    }
    Ok(base_ty.clone())
}

pub(crate) fn infer_list_append_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::ListAppend { list, value } = expr else {
        unreachable!()
    };
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
        // Non-strict mode (#3): a list holding values of two types is
        // CPython's own list, as every Python list is.
        if let pycc_hir::ContainerReceiver::Name(name) = list {
            crate::non_strict::request_widen(name, &list_ty, &Ty::Object, |_| true);
        }
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
pub(crate) fn infer_list_pop_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::ListPop { list } = expr else {
        unreachable!()
    };
    let list_ty = infer_container_receiver(env, local_names, list, ".pop()")?;
    // `s.pop()` on a set resolves through `pycc_hir::builtins`.
    if let Some(Ok(sig)) = pycc_hir::builtins::resolve_method(&list_ty, "pop", &[]) {
        return Ok(sig.ret);
    }
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
pub(crate) fn infer_dict_get_or_default_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::DictGetOrDefault { dict, key, default } = expr else {
        unreachable!()
    };
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
pub(crate) fn infer_set_add_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::SetAdd { set, value } = expr else {
        unreachable!()
    };
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
