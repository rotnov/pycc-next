//! Structural queries over `MirExpr` (purity, exception effects, sub-expression walks).

use super::*;

impl MirExpr {
    pub fn ty(&self) -> Ty {
        match self {
            MirExpr::IntLiteral(_) | MirExpr::IntBoundary(_) => Ty::Int,
            MirExpr::FloatLiteral(_) => Ty::Float,
            MirExpr::BoolLiteral(_) => Ty::Bool,
            MirExpr::StringLiteral(_) | MirExpr::FString(_) => Ty::Str,
            MirExpr::NoneLiteral => Ty::None,
            MirExpr::NotImplemented | MirExpr::ObjectBox(_) => Ty::Object,
            MirExpr::OptionalWrap(_, inner) => Ty::Optional(inner.clone()),
            MirExpr::OptionalUnwrap(_, inner) | MirExpr::ObjectUnbox(_, inner) => (**inner).clone(),
            MirExpr::ObjectIndex(..) => Ty::Int,
            MirExpr::Name { ty, .. }
            | MirExpr::Call { ty, .. }
            | MirExpr::BinOp { ty, .. }
            | MirExpr::Compare { ty, .. }
            | MirExpr::BoolOp { ty, .. }
            | MirExpr::IfExp { ty, .. } => ty.clone(),
            MirExpr::Not(_) | MirExpr::CompareChain { .. } => Ty::Bool,
            MirExpr::EmptyList(element) => Ty::List(Box::new(element.clone())),
            MirExpr::EmptySet(element) => Ty::Set(Box::new(element.clone())),
            MirExpr::EmptyDict(pair) => Ty::Dict(pair.clone()),
            MirExpr::ListLiteral(elements) => {
                let elem_ty = elements.first().map(|e| e.ty()).unwrap_or_else(|| {
                    panic!(
                        "pycc_mir: internal error: an empty list literal has no element type to derive -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                    )
                });
                Ty::List(Box::new(elem_ty))
            }
            // Unlike `ForList`'s own `Ty::List`/`Ty::Dict` dispatch just
            // below, this arm is *not* where a dict-typed base gets
            // resolved: `pycc_types::infer_expr_in`'s own `Subscript` arm
            // accepts a `Ty::Dict` base just as readily as `Ty::List` (it
            // returns the dict's value type directly), so a non-list base
            // reaching *this* HIR shape is not, on its own, something
            // `pycc_types::check` would have rejected. What actually keeps
            // a dict-typed base out of this arm is `lower_expr`'s own
            // `HirExpr::Subscript` arm: it inspects the lowered base's type
            // and constructs `MirExpr::DictGet` instead of
            // `MirExpr::Subscript` whenever that type is `Ty::Dict`, so no
            // `MirExpr::Subscript` node produced by `build()` ever carries a
            // dict base. A base that is neither list nor dict *is* still
            // impossible from a type-checked program (`pycc_types` rejects
            // every other subscript base with T0033 before HIR is even
            // constructed) -- this panic remains reachable only from a
            // hand-built `MirExpr` that bypasses both of those guarantees,
            // e.g. this file's own `a_subscript_over_a_non_list_bases_ty_panics_with_an_internal_error` test.
            MirExpr::Subscript { base, index } => match base.ty() {
                Ty::List(elem_ty) => *elem_ty,
                // PR-11b Task 4 (D-116): the literal-index extraction and
                // bounds check already happened in `pycc_types::infer_expr_in`
                // -- this re-derives the same positional element type from
                // the already-validated literal `index`, mirroring how
                // every other panic path in this file re-states "pycc_types
                // should have rejected this" rather than re-validating.
                Ty::Tuple(elems) => {
                    // A computed index: `pycc_types` admits it only on a
                    // tuple whose elements share one scalar type.
                    let MirExpr::IntLiteral(literal_index) = index.as_ref() else {
                        return elems.first().cloned().unwrap_or_else(|| {
                            panic!(
                                "pycc_mir: internal error: computed index into an empty tuple -- pycc_types::check should have rejected this HIR before it reached pycc_mir (T0040)"
                            )
                        });
                    };
                    let literal_index = usize::try_from(*literal_index).unwrap_or_else(|_| {
                        panic!(
                            "pycc_mir: internal error: tuple subscript index is negative -- pycc_types::check should have rejected this HIR before it reached pycc_mir (T0040)"
                        )
                    });
                    elems.get(literal_index).cloned().unwrap_or_else(|| {
                        panic!(
                            "pycc_mir: internal error: tuple subscript index out of range -- pycc_types::check should have rejected this HIR before it reached pycc_mir (T0040)"
                        )
                    })
                }
                other => panic!(
                    "pycc_mir: internal error: subscript base has non-list/tuple type `{}` -- pycc_types::check should have rejected this HIR before it reached pycc_mir, or lower_expr should have routed a dict base to MirExpr::DictGet instead",
                    other.name()
                ),
            },
            // `.append()` always returns `None` in Python, independent of
            // the list's element type -- a true invariant (like `Compare`'s
            // `Bool` and `ForRange`'s `Int` above), not narrowed by any
            // `pycc_types` gate, so this is hardcoded on purpose.
            MirExpr::ListAppend { .. } | MirExpr::ListInsert { .. } => Ty::None,
            MirExpr::DictLiteral(pairs) => {
                let (first_key, first_value) = pairs.first().unwrap_or_else(|| {
                    panic!(
                        "pycc_mir: internal error: an empty dict literal has no key/value type to derive -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                    )
                });
                Ty::Dict(Box::new((first_key.ty(), first_value.ty())))
            }
            MirExpr::DictGet { dict, .. } => match dict.ty() {
                Ty::Dict(kv) => kv.1,
                other => panic!(
                    "pycc_mir: internal error: dict subscript base has non-dict type `{}` -- pycc_types::check should have rejected this HIR before it reached pycc_mir",
                    other.name()
                ),
            },
            MirExpr::SetLiteral { elements, .. } => {
                let first = elements.first().unwrap_or_else(|| {
                    panic!(
                        "pycc_mir: internal error: an empty set literal has no element type to derive -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                    )
                });
                Ty::Set(Box::new(first.ty()))
            }
            MirExpr::TupleLiteral(elements) => {
                if elements.is_empty() {
                    panic!(
                        "pycc_mir: internal error: an empty tuple literal has no element types to derive -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                    )
                }
                Ty::Tuple(Box::new(elements.iter().map(MirExpr::ty).collect()))
            }
            // A slice of a `list[int]` is still a `list[int]` -- `ty()`
            // passes `base.ty()` through unchanged, matching
            // `pycc_types::infer_expr_in`'s own `HirExpr::Slice` arm, which
            // returns `base_ty.clone()` after validating the base is
            // `Ty::List(Int)` (that validation already happened in
            // `pycc_types`; this crate only re-derives the resulting type).
            MirExpr::Slice { base, .. } => base.ty(),
            MirExpr::ListPop { ty, .. } => ty.clone(),
            MirExpr::DictGetOrDefault { ty, .. } => ty.clone(),
            // `.add()` always returns `None`, exactly like `ListAppend`
            // above -- a true invariant, not narrowed by any `pycc_types`
            // gate, so this is hardcoded on purpose.
            MirExpr::SetAdd { .. } => Ty::None,
            MirExpr::Instantiate(inst) => inst.ty.clone(),
            MirExpr::AttrGet { ty, .. } | MirExpr::ObjAttrGet { ty, .. } => ty.clone(),
            // No `ty` field to read: a foreign method call's result is
            // opaque by construction (see the variant's own documentation,
            // which also records why it carries no field where `ObjAttrGet`
            // does).
            MirExpr::ObjMethodCall { .. }
            | MirExpr::ObjCall { .. }
            | MirExpr::ObjKeywordCall(_) => Ty::Object,
            // Likewise hardcoded: `len` is an `int` for every operand the
            // shim can answer for. See the variant's own documentation.
            MirExpr::ObjLen { .. } => Ty::Int,
            // No `ty` field to read either: a foreign subscript's result is
            // opaque by construction, exactly as `ObjMethodCall`'s is. See
            // the variant's own documentation.
            MirExpr::ObjSubscript { .. } => Ty::Object,
            // Part 1 of #1371: identity is a pointer compare; a rich
            // comparison answers CPython's own result object. See the
            // variant's own documentation.
            MirExpr::ObjCompare { op, .. } => {
                if matches!(op, pycc_hir::CmpOpKind::Is | pycc_hir::CmpOpKind::IsNot) {
                    Ty::Bool
                } else {
                    Ty::Object
                }
            }
            MirExpr::ObjIsInstance { .. } | MirExpr::ObjContains { .. } => Ty::Bool,
            MirExpr::ObjBinOp { .. } | MirExpr::ObjUnaryOp { .. } => Ty::Object,
            // Part 2b of #1371: CPython's own slice result, opaque.
            // Part 11 of #1371: `type(o)` is CPython's own class object.
            MirExpr::ObjSlice { .. }
            | MirExpr::ObjList { .. }
            | MirExpr::ObjDisplay { .. }
            | MirExpr::ObjType { .. } => Ty::Object,
            // Hardcoded for `ObjLen`'s reason, not `ObjSubscript`'s: the
            // element type is known, it is just not recoverable from the
            // base. A `memoryview` parameter is one-dimensional and `"d"`-
            // formatted by the unpack shim's own contract, so every element
            // is a `float`. See the variant's own documentation.
            MirExpr::BufferGet { .. } => Ty::Float,
            // Hardcoded for `ObjLen`'s reason as well: a buffer's element
            // count is an `int` unconditionally. See the variant's own
            // documentation.
            MirExpr::BufferLen { .. } => Ty::Int,
            MirExpr::GenNext { ty, .. } | MirExpr::GenToList { ty, .. } => ty.clone(),
            MirExpr::GenAnyAll { .. } => Ty::Bool,
            MirExpr::GenYield { ty, .. }
            | MirExpr::GenYieldFrom { ty, .. }
            | MirExpr::GenSend { ty, .. }
            | MirExpr::GenThrow { ty, .. }
            | MirExpr::StopIterationValue { ty, .. } => ty.clone(),
            MirExpr::GenClose { .. } => Ty::None,
            MirExpr::MakeClosure { ty, .. } | MirExpr::CallClosure { ty, .. } => ty.clone(),
            // Hardcoded for the same reason once more, and it is this fact
            // that the rest of Part 2a rests on: the producer's result is
            // the very same `Ty::MemoryView` a `memoryview` parameter
            // carries, so `MirExpr::BufferGet`, `MirExpr::BufferLen` and
            // `MirStmt::BufferSet` all dispatch onto an owned buffer with no
            // change at all. See the variant's own documentation.
            MirExpr::BufferAlloc { .. } => Ty::MemoryView,
            // Part 1 of #1319: the only compiled frozenset type. See the
            // variant's own documentation.
            // #1343: the source's element type; `frozenset()` is `frozenset[int]`.
            MirExpr::FrozenSetFrom { source } => match source.as_deref().map(MirExpr::ty) {
                Some(Ty::Set(elem) | Ty::FrozenSet(elem) | Ty::List(elem)) => Ty::FrozenSet(elem),
                _ => Ty::FrozenSet(Box::new(Ty::Int)),
            },
            // #1335: `hash()` always yields an `int`.
            MirExpr::InstanceHash { .. } => Ty::Int,
            // Rebuilt from `arity` rather than read from a field: the
            // annotation this node exists for is a fixed-arity tuple whose
            // every element is `float`, so the arity is the whole type.
            // See the variant's own documentation.
            MirExpr::ObjUnpackFloatTuple { arity, .. } => {
                Ty::Tuple(Box::new(vec![Ty::Float; *arity]))
            }
            MirExpr::ObjUnpack { .. } => Ty::Object,
            MirExpr::NullInstance { ty } => ty.clone(),
            MirExpr::ExceptionMessage(_) => Ty::Str,
            MirExpr::ExceptionTypeTest { .. } => Ty::Bool,
            MirExpr::NamedExpr { ty, .. } => ty.clone(),
            MirExpr::Comprehension(comp) => comp.ty(),
            MirExpr::Sequence { value, .. } => value.ty(),
            MirExpr::RtCall(call) => call.sig.ret.clone(),
            MirExpr::Nd(nd) => nd.ty(),
        }
    }

    /// PEP 572 (#774): walks every `MirExpr::NamedExpr` reachable from
    /// `self`, at any nesting depth, appending `(name, ty)` for each one
    /// found (evaluation order, left to right/outer to inner as the fields
    /// are declared -- a walrus target is rebindable, so a later entry for
    /// the same `name` naturally overrides an earlier one when both are
    /// later applied via `bind_variable`'s "last write wins" semantics).
    /// `pycc_hir::stmt::lower_stmt`'s own `contains_named_expr` restriction
    /// guarantees a `NamedExpr` node only ever exists nested inside exactly
    /// three statement placements (an `if`/`while` test or a bare
    /// expression statement) -- this walker itself makes no such
    /// assumption and is exhaustive over every `MirExpr` variant so that
    /// adding a new variant without touching this function is a compile
    /// error, not a silently-skipped binding. Two independent call sites
    /// need this exact same walk: `pycc_mir::stmt::lower_stmt` (to
    /// `bind_variable` each name into `scopes` before lowering the
    /// statement's own body/next statement) and
    /// `pycc_codegen::collect_expr_bindings` (to predeclare each name's
    /// storage slot, mirroring `collect_stmt_bindings`'s own
    /// `MirStmt::Assign` handling) -- defined once, here, rather than
    /// duplicated in both crates.
    pub fn collect_named_expr_bindings(&self, out: &mut Vec<(String, Ty)>) {
        match self {
            MirExpr::IntLiteral(_)
            | MirExpr::FloatLiteral(_)
            | MirExpr::BoolLiteral(_)
            | MirExpr::StringLiteral(_)
            | MirExpr::EmptyList(_)
            | MirExpr::EmptySet(_)
            | MirExpr::EmptyDict(_)
            | MirExpr::NoneLiteral
            | MirExpr::NotImplemented
            | MirExpr::Name { .. }
            | MirExpr::NullInstance { .. } => {}
            MirExpr::ListPop { list, index, .. } => {
                if let Some(receiver) = list.attr_expr() {
                    receiver.collect_named_expr_bindings(out);
                }
                if let Some(index) = index {
                    index.collect_named_expr_bindings(out);
                }
            }
            MirExpr::ListInsert { list, index, value } => {
                if let Some(receiver) = list.attr_expr() {
                    receiver.collect_named_expr_bindings(out);
                }
                index.collect_named_expr_bindings(out);
                value.collect_named_expr_bindings(out);
            }
            MirExpr::IntBoundary(inner) => inner.collect_named_expr_bindings(out),
            MirExpr::GenToList { generator, .. }
            | MirExpr::GenAnyAll { generator, .. }
            | MirExpr::GenClose { generator }
            | MirExpr::GenYield {
                value: generator, ..
            }
            | MirExpr::GenYieldFrom { sub: generator, .. }
            | MirExpr::StopIterationValue {
                exception: generator,
                ..
            } => generator.collect_named_expr_bindings(out),
            MirExpr::GenSend {
                generator, value, ..
            } => {
                generator.collect_named_expr_bindings(out);
                value.collect_named_expr_bindings(out);
            }
            MirExpr::GenThrow {
                generator,
                exception,
                ..
            } => {
                generator.collect_named_expr_bindings(out);
                for expr in exception.exprs() {
                    expr.collect_named_expr_bindings(out);
                }
            }
            MirExpr::GenNext {
                generator, default, ..
            } => {
                generator.collect_named_expr_bindings(out);
                if let Some(default) = default {
                    default.collect_named_expr_bindings(out);
                }
            }
            MirExpr::MakeClosure { captures, .. } => {
                for capture in captures {
                    capture.collect_named_expr_bindings(out);
                }
            }
            MirExpr::CallClosure { callee, args, .. } => {
                callee.collect_named_expr_bindings(out);
                for arg in args {
                    arg.collect_named_expr_bindings(out);
                }
            }
            MirExpr::OptionalWrap(inner, _) | MirExpr::ObjectBox(inner) => {
                inner.collect_named_expr_bindings(out)
            }
            // Issue #769 (Part 2 of #747): `OptionalUnwrap` wraps a single
            // sub-expression (the narrowed name's own read), exactly like
            // `OptionalWrap`/`IntBoundary` immediately above -- a walrus
            // nested inside it (e.g. `(x if (n := x) is not None else
            // 0)`-shaped MIR is not producible by this compiler's own
            // narrowing lowering today, but the recursive walk still must
            // not silently skip whatever sub-expression this node wraps).
            MirExpr::OptionalUnwrap(inner, _)
            | MirExpr::ObjectUnbox(inner, _)
            | MirExpr::ObjectIndex(inner, _) => inner.collect_named_expr_bindings(out),
            MirExpr::Call { args, .. } => {
                for arg in args {
                    arg.collect_named_expr_bindings(out);
                }
            }
            MirExpr::CompareChain { first, links } => {
                first.collect_named_expr_bindings(out);
                for link in links {
                    link.right.collect_named_expr_bindings(out);
                }
            }
            MirExpr::BinOp { left, right, .. }
            | MirExpr::Compare { left, right, .. }
            | MirExpr::BoolOp { left, right, .. } => {
                left.collect_named_expr_bindings(out);
                right.collect_named_expr_bindings(out);
            }
            MirExpr::IfExp {
                test, body, orelse, ..
            } => {
                for part in [test, body, orelse] {
                    part.collect_named_expr_bindings(out);
                }
            }
            MirExpr::FString(parts) => {
                for part in parts {
                    if let MirFStringPart::Interpolation(inner) = part {
                        inner.collect_named_expr_bindings(out);
                    }
                }
            }
            MirExpr::ListLiteral(elements)
            | MirExpr::ObjList { elements }
            | MirExpr::ObjDisplay { elements, .. }
            | MirExpr::SetLiteral { elements, .. }
            | MirExpr::TupleLiteral(elements) => {
                for element in elements {
                    element.collect_named_expr_bindings(out);
                }
            }
            MirExpr::Subscript { base, index } => {
                base.collect_named_expr_bindings(out);
                index.collect_named_expr_bindings(out);
            }
            MirExpr::ListAppend { list, value } => {
                if let Some(receiver) = list.attr_expr() {
                    receiver.collect_named_expr_bindings(out);
                }
                value.collect_named_expr_bindings(out);
            }
            MirExpr::SetAdd { value, .. } => {
                value.collect_named_expr_bindings(out);
            }
            MirExpr::DictLiteral(pairs) => {
                for (key, value) in pairs {
                    key.collect_named_expr_bindings(out);
                    value.collect_named_expr_bindings(out);
                }
            }
            MirExpr::DictGet { dict, key } => {
                dict.collect_named_expr_bindings(out);
                key.collect_named_expr_bindings(out);
            }
            MirExpr::Slice {
                base,
                start,
                stop,
                step,
            } => {
                base.collect_named_expr_bindings(out);
                for bound in [start, stop, step].into_iter().flatten() {
                    bound.collect_named_expr_bindings(out);
                }
            }
            MirExpr::DictGetOrDefault {
                dict, key, default, ..
            } => {
                if let Some(receiver) = dict.attr_expr() {
                    receiver.collect_named_expr_bindings(out);
                }
                key.collect_named_expr_bindings(out);
                default.collect_named_expr_bindings(out);
            }
            MirExpr::Instantiate(inst) => {
                for arg in &inst.args {
                    arg.collect_named_expr_bindings(out);
                }
            }
            // Part 1 of #1319: the argument, when present, is the node's
            // only child (`if frozenset([(n := 3)]):`).
            MirExpr::FrozenSetFrom { source } => {
                if let Some(source) = source {
                    source.collect_named_expr_bindings(out);
                }
            }
            // #1335: the operand is the only child (`hash((r := R()))`).
            MirExpr::InstanceHash { operand, .. } => operand.collect_named_expr_bindings(out),
            MirExpr::ObjUnaryOp { operand, .. } => operand.collect_named_expr_bindings(out),
            MirExpr::AttrGet { base, .. }
            | MirExpr::ObjAttrGet { base, .. }
            | MirExpr::ObjLen { base }
            // Part 11 of #1371: `type(o)`'s operand is its only child.
            | MirExpr::ObjType { base }
            // The base is this node's only child too -- `len(b)` takes no
            // index -- so a walrus can hide only there.
            | MirExpr::BufferLen { base }
            // #1165: the length is this node's only child -- the producer
            // takes no base -- so a walrus can hide only there
            // (`ndarray((n := 4))`).
            | MirExpr::BufferAlloc { len: base }
            // PR 4c of #1083: the base is the node's only child -- `arity`
            // is a `usize`, not an expression -- so a walrus can hide only
            // there (`x: tuple[float, float] = (o := numpy).pair`).
            | MirExpr::ObjUnpackFloatTuple { base, .. }
            // Part 1 of #891: the unpacked value is the only child.
            | MirExpr::ObjUnpack { value: base, .. } => base.collect_named_expr_bindings(out),
            // Both sides, unlike `ObjAttrGet` directly above: a walrus can
            // hide in an argument (`numpy.seed((n := 1))`) just as easily as
            // in the base, and a binding missed here is a name codegen never
            // allocates storage for.
            // #1313: a direct call's arguments can hide a walrus exactly as
            // a method call's can, and since Part 2a of #1371 so can its
            // callee (`t[(k := 1)](x)`).
            MirExpr::ObjMethodCall { base, args, .. }
            | MirExpr::ObjCall { callee: base, args } => {
                base.collect_named_expr_bindings(out);
                for arg in args {
                    arg.collect_named_expr_bindings(out);
                }
            }
            // Part 8 of #1371: the positional half, then every keyword
            // value (`o.m(k=(n := 1))`).
            MirExpr::ObjKeywordCall(call) => {
                call.call.collect_named_expr_bindings(out);
                for value in &call.values {
                    value.collect_named_expr_bindings(out);
                }
            }
            // Both sides too, for the identical reason: a walrus can hide in
            // the key (`gc.garbage[(n := 0)]`) just as easily as in the base.
            // Both sides for that same reason. `b[(n := 0)]` is the shape
            // that makes the index half load-bearing here: a walrus binding
            // missed in the index is a local codegen never allocates
            // storage for.
            MirExpr::ObjSubscript { base, index } | MirExpr::BufferGet { base, index } => {
                base.collect_named_expr_bindings(out);
                index.collect_named_expr_bindings(out);
            }
            MirExpr::ObjCompare { left, right, .. }
            | MirExpr::ObjBinOp { left, right, .. }
            | MirExpr::ObjContains {
                item: left,
                container: right,
                ..
            } => {
                left.collect_named_expr_bindings(out);
                right.collect_named_expr_bindings(out);
            }
            MirExpr::ObjSlice {
                base,
                start,
                stop,
                step,
            } => {
                base.collect_named_expr_bindings(out);
                for bound in [start, stop, step].into_iter().flatten() {
                    bound.collect_named_expr_bindings(out);
                }
            }
            MirExpr::ObjIsInstance { value, class } => {
                value.collect_named_expr_bindings(out);
                if let ObjIsInstanceClass::Object(class) = class {
                    class.collect_named_expr_bindings(out);
                }
            }
            MirExpr::ExceptionMessage(inner)
            | MirExpr::ExceptionTypeTest { obj: inner, .. }
            | MirExpr::Not(inner) => {
                inner.collect_named_expr_bindings(out)
            }
            MirExpr::NamedExpr { name, value, ty } => {
                value.collect_named_expr_bindings(out);
                out.push((name.clone(), ty.clone()));
            }
            // #1254 (D-250): `pycc_hir` refuses a walrus inside a
            // comprehension, and the loop variable is node-scoped, so there
            // is no enclosing-scope binding to predeclare.
            MirExpr::Comprehension(_) => {}
            // #1346: a walrus inside a discarded receiver
            // (`mk((k := 4)).h(2)`) still binds its target.
            MirExpr::Sequence { discard, value } => {
                discard.collect_named_expr_bindings(out);
                value.collect_named_expr_bindings(out);
            }
            MirExpr::RtCall(call) => {
                for arg in &call.args {
                    arg.collect_named_expr_bindings(out);
                }
            }
            MirExpr::Nd(nd) => {
                nd.base.collect_named_expr_bindings(out);
                for arg in &nd.args {
                    arg.collect_named_expr_bindings(out);
                }
            }
        }
    }
}
