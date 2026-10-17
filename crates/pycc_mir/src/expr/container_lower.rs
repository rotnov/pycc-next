//! Lowering of container literals, subscripts, slices, methods and comprehensions.

use super::*;

pub(crate) fn lower_list_literal_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::ListLiteral(elements) = expr else {
        unreachable!()
    };
    // An empty display no evidence typed (`pycc_types`: a `list[int]`).
    if elements.is_empty() {
        return MirExpr::EmptyList(Ty::Int);
    }
    let elements: Vec<MirExpr> = elements
        .iter()
        .map(|e| lower_expr(e, scopes, classes, current_class))
        .collect();
    // Non-strict mode (#3): an object element makes it a CPython list.
    if super::super::obj_arith::is_object_display(pycc_hir::DisplayKind::List, &elements) {
        return MirExpr::ObjList {
            elements: elements
                .into_iter()
                .map(super::super::obj_arith::display_element)
                .collect(),
        };
    }
    MirExpr::ListLiteral(num_wrapped(elements))
}

/// `[1, 2.5]`: when the elements' types join to `int | float`, every
/// element is wrapped as an `int | float` word; otherwise they are returned
/// unchanged.
fn num_wrapped(elements: Vec<MirExpr>) -> Vec<MirExpr> {
    let mixed = elements
        .iter()
        .skip(1)
        .try_fold(elements.first().map(MirExpr::ty), |joined, element| {
            let joined = joined?;
            if joined == element.ty() {
                Some(Some(joined))
            } else {
                pycc_hir::num_join(&joined, &element.ty()).map(Some)
            }
        })
        .flatten();
    if mixed != Some(Ty::Num) {
        return elements;
    }
    elements
        .into_iter()
        .map(|element| {
            if element.ty() == Ty::Num {
                element
            } else {
                MirExpr::NumWrap(Box::new(element))
            }
        })
        .collect()
}

// `HirExpr::Subscript` is reused unconditionally by `pycc_hir`'s own
// lowering for both a list read and a dict read (it has no type
// information to pick a different node) -- `pycc_types::infer_expr_in`
// accepts either base equally (see its own `Subscript` arm), so this
// is the point where the real type is resolved and where a
// dict-typed base is routed into `MirExpr::DictGet` instead of
// `MirExpr::Subscript`, mirroring `lower_stmt`'s own `HirStmt::ForList`
// arm doing the same list/dict routing for iteration.
pub(crate) fn lower_subscript_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Subscript { base, index } = expr else {
        unreachable!()
    };
    // PEP 560 (#610): `C[x]` on a bare class name is
    // `C.__class_getitem__(x)`. `pycc_types`' own `Subscript` arm
    // has already resolved the hook through the MRO and rejected a
    // class that does not define it, so reaching this point with a
    // class-name base means the call is valid. The guard is the same
    // one that arm applies -- a name bound as a value shadows the
    // class, and `C[0]` then reads that value's element.
    //
    // Lowering is delegated to this function's own `MethodCall` arm
    // by way of a synthetic node rather than duplicated here: that
    // arm already handles both the `@staticmethod` and the
    // `@classmethod` spelling of the hook (the latter needs a
    // `NullInstance` receiver prepended), and re-deriving either
    // shape here would be a second copy of logic that must not
    // drift. `lower_expr` on a bare class name would panic (class
    // names are not in scope), which is why the interception happens
    // before the base is lowered.
    if let HirExpr::Name(class_name) = base.as_ref()
        && !scopes.iter().any(|scope| scope.contains_key(class_name))
        && classes.contains_key(class_name.as_str())
    {
        return lower_expr(
            &HirExpr::MethodCall {
                base: base.clone(),
                method: "__class_getitem__".to_string(),
                args: vec![(**index).clone()],
            },
            scopes,
            classes,
            current_class,
        );
    }
    let base = lower_expr(base, scopes, classes, current_class);
    let index = {
        let hir = index;
        let index = lower_expr(hir, scopes, classes, current_class);
        // A sequence index reads an `IntEnum` member as its `int` value.
        if matches!(base.ty(), Ty::Dict(_)) {
            index
        } else {
            super::int_enum::as_int(hir, index, scopes, classes, current_class)
        }
    };
    match base.ty() {
        Ty::Str => {
            builtin_call::lower_str_getitem(base, crate::index_operand(index, crate::IndexUse::Str))
        }
        // Part 3 of #1026 (PR 3b of #1082): a foreign CPython
        // object base routes to its own node, in exactly the
        // position `Ty::Dict` occupies and for exactly the same
        // reason -- `MirExpr::Subscript`'s own `ty()` arm panics on
        // any base that is neither list nor tuple, so the split has
        // to happen here, where the lowered base's type is known.
        // The dispatch is on `base.ty()` and nothing else: a
        // foreign name is always in scope, so the PEP 560
        // class-name interception above cannot fire for one, and an
        // extra guard would only desynchronise this lowering from
        // the checker (the lesson #1098 records for `len`).
        Ty::Object => MirExpr::ObjSubscript {
            base: Box::new(base),
            index: Box::new(index),
        },
        Ty::Dict(_) => MirExpr::DictGet {
            dict: Box::new(base),
            key: Box::new(index),
        },
        // Part 2 of #1027, in that same position and for that same
        // reason: without an arm of its own a `memoryview` base
        // reaches the catch-all below and `MirExpr::Subscript`'s
        // `ty()` panics on it. The dispatch is on `base.ty()` alone
        // here too -- the type checker has already refused every
        // `memoryview` subscript this arm must not see.
        Ty::MemoryView => MirExpr::BufferGet {
            base: Box::new(base),
            index: Box::new(index),
        },
        _ => {
            let use_ = if matches!(base.ty(), Ty::Tuple(_)) {
                crate::IndexUse::Tuple
            } else {
                crate::IndexUse::List
            };
            MirExpr::Subscript {
                base: Box::new(base),
                index: Box::new(crate::index_operand(index, use_)),
            }
        }
    }
}

pub(crate) fn lower_dict_literal_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::DictLiteral(pairs) = expr else {
        unreachable!()
    };
    let pairs: Vec<(MirExpr, MirExpr)> = pairs
        .iter()
        .map(|(k, v)| {
            (
                lower_expr(k, scopes, classes, current_class),
                lower_expr(v, scopes, classes, current_class),
            )
        })
        .collect();
    // Non-strict mode (#3): an object key or value makes it a CPython dict.
    let tys: Vec<Ty> = pairs.iter().flat_map(|(k, v)| [k.ty(), v.ty()]).collect();
    if pycc_hir::display_is_object(pycc_hir::DisplayKind::Dict, &tys) {
        return super::super::obj_arith::object_display(
            crate::ObjDisplayKind::Dict,
            pairs.into_iter().flat_map(|(k, v)| [k, v]).collect(),
        );
    }
    // `{1: 1.5, 2: 3}`: the values of an `int | float`-valued dict.
    let (keys, values): (Vec<MirExpr>, Vec<MirExpr>) = pairs.into_iter().unzip();
    MirExpr::DictLiteral(keys.into_iter().zip(num_wrapped(values)).collect())
}

pub(crate) fn lower_set_literal_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::SetLiteral(elements) = expr else {
        unreachable!()
    };
    let elements: Vec<MirExpr> = elements
        .iter()
        .map(|e| lower_expr(e, scopes, classes, current_class))
        .collect();
    if super::super::obj_arith::is_object_display(pycc_hir::DisplayKind::Set, &elements) {
        return super::super::obj_arith::object_display(crate::ObjDisplayKind::Set, elements);
    }
    // #1343: the first element's type is the literal's, as
    // `MirExpr::ty` derives it.
    let ops = elements
        .first()
        .and_then(|first| set_ops::lower_set_element_ops(&first.ty(), scopes, classes))
        .map(Box::new);
    MirExpr::SetLiteral { elements, ops }
}

// PR-12 Task 8 (D-118): purely structural -- recurse into `base` and
// every present bound, same as every other MIR-lowering site in this
// function. `pycc_types` already validated the base is
// `Ty::List(Int)` and every present bound is `int`-assignable
// (Task 7); this lowering does not re-validate or apply the runtime
// clamping/panic behavior D-118 requires -- that is
// `pycc_codegen`'s job, operating on this already-lowered shape.
//
// Part 2b of #1371: a CPython object base is split into its own
// `MirExpr::ObjSlice`, because `MirExpr::Slice::ty()` answers its
// base's type and codegen's `Slice` arm is list/str/tuple-only.
pub(crate) fn lower_slice_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Slice {
        base,
        start,
        stop,
        step,
    } = expr
    else {
        unreachable!()
    };
    let base = Box::new(lower_expr(base, scopes, classes, current_class));
    let lower_bound = |bound: &Option<Box<HirExpr>>| {
        bound
            .as_deref()
            .map(|e| Box::new(lower_expr(e, scopes, classes, current_class)))
    };
    let (mut start, mut stop, mut step) =
        (lower_bound(start), lower_bound(stop), lower_bound(step));
    // Non-strict mode: a CPython object bound of a native slice is read
    // through `__index__` (`MirExpr::ObjectIndex`).
    if base.ty() != Ty::Object {
        for bound in [&mut start, &mut stop, &mut step] {
            if let Some(b) = bound.take() {
                *bound = Some(Box::new(crate::index_operand(*b, crate::IndexUse::Slice)));
            }
        }
    }
    if base.ty() == Ty::Str {
        return builtin_call::lower_str_slice(*base, [start, stop, step]);
    }
    if base.ty() == Ty::Object {
        MirExpr::ObjSlice {
            base,
            start,
            stop,
            step,
        }
    } else {
        MirExpr::Slice {
            base,
            start,
            stop,
            step,
        }
    }
}

// PR-12 Task 11 (D-119): `list`'s element type is resolved via the
// same `lookup` mechanism every other name reference in this crate
// uses, mirroring `HirExpr::Subscript`'s own base-type lookup above.
// #1263: an attribute receiver's type is its lowered read's own.
pub(crate) fn lower_list_pop_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::ListPop { list } = expr else {
        unreachable!()
    };
    let (list, list_ty) = lower_container_receiver(list, scopes, classes, current_class);
    // `s.pop()` on a set resolves through `pycc_hir::builtins`.
    if !matches!(list_ty, Ty::List(_)) {
        let receiver = match list {
            MirContainerReceiver::Name(name) => MirExpr::Name { name, ty: list_ty },
            MirContainerReceiver::Attr(read) => *read,
        };
        return builtin_call::lower_builtin_method(receiver, "pop", Vec::new()).unwrap_or_else(
            |_| panic!("pycc_mir: internal error: `.pop()` on a receiver pycc_types rejected"),
        );
    }
    let Ty::List(elem_ty) = list_ty else {
        panic!(
            "pycc_mir: internal error: `.pop()` receiver {list:?} is not list-typed -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
        )
    };
    MirExpr::ListPop {
        list,
        index: None,
        ty: *elem_ty,
    }
}

pub(crate) fn lower_dict_get_or_default_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::DictGetOrDefault { dict, key, default } = expr else {
        unreachable!()
    };
    let (dict, dict_ty) = lower_container_receiver(dict, scopes, classes, current_class);
    let Ty::Dict(kv) = dict_ty else {
        panic!(
            "pycc_mir: internal error: `.get()` receiver {dict:?} is not dict-typed -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
        )
    };
    MirExpr::DictGetOrDefault {
        dict,
        key: Box::new(lower_expr(key, scopes, classes, current_class)),
        default: Box::new(lower_expr(default, scopes, classes, current_class)),
        ty: kv.1,
    }
}

// #1254 (D-250): `cond` and the elements are lowered against a copy
// of `scopes` with one pushed frame holding the synthesized loop
// variable, so it never becomes a binding of the enclosing scope.
// `resolve_comp_source` lowers the range operands before it binds
// `var`, so they still read only the enclosing bindings.
// `narrowed_ty` searches every frame, so narrowing carries over.
pub(crate) fn lower_comprehension_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Comprehension(comp) = expr else {
        unreachable!()
    };
    let mut inner = scopes.to_vec();
    inner.push(HashMap::new());
    let (source, var_ty) = super::super::resolve_comp_source(
        &comp.iter,
        &comp.var,
        &mut inner,
        classes,
        current_class,
    );
    let cond = comp
        .cond
        .as_ref()
        .map(|c| lower_expr(c, &inner, classes, current_class));
    let elt = match &comp.elt {
        CompElt::List(e) => MirCompElt::List(lower_expr(e, &inner, classes, current_class)),
        CompElt::Set(e) => {
            let e = lower_expr(e, &inner, classes, current_class);
            // #1344: a `set[C]` element carries its class's hash and
            // equality, exactly as a set literal's does.
            let ops = set_ops::lower_set_element_ops(&e.ty(), &inner, classes).map(Box::new);
            MirCompElt::Set(e, ops)
        }
        CompElt::Dict { key, value } => MirCompElt::Dict {
            key: lower_expr(key, &inner, classes, current_class),
            value: lower_expr(value, &inner, classes, current_class),
        },
    };
    MirExpr::Comprehension(Box::new(MirComprehension {
        var: comp.var.clone(),
        var_ty,
        source,
        cond,
        elt,
    }))
}
