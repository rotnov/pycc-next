//! HIR-to-MIR expression lowering (#546): `lower_expr`, the per-`HirExpr`
//! dispatch that the crate root's lowering walk and every sibling lowering
//! module drive.

use super::class::{
    class_def_of, class_source_name, lower_isinstance, lower_issubclass, mro_attrs,
    rewrite_exception_to_message, rewrite_instance_to_repr, self_expr,
};
use super::{
    HirClassDef, InstantiateExpr, MirCompElt, MirComprehension, MirContainerReceiver, MirExpr,
    MirFStringPart, binop_result_ty, lookup, mro_class_def, try_lower_enum_member_attr,
};
use crate::receiver_exact::{exact_callee, receiver_class};
use pycc_hir::{
    BinOpKind, ClassAttrValue, CmpOpKind, CompElt, ContainerReceiver, FStringPart, HirExpr, Ty,
    UnaryOpKind, declares_name_outside_class_attrs,
};
use std::collections::HashMap;

pub(crate) mod builtin_call;
mod foreign_static;
mod instance_hash;
mod receiver_dispatch;
mod regular_method;
mod sequence;

// Large `lower_expr` arms (mechanical extraction: each helper re-destructures
// its own node and runs the arm body verbatim).
mod attr_lower;
mod call_lower;
mod container_lower;
mod operator_lower;
pub(crate) mod set_ops;

use instance_hash::lower_instance_hash;
use sequence::sequence_after;

/// D-154 (Part 1 of #375): lowers a construction of the class `callee`
/// (`class_def`) from its already-lowered `args` -- the shared tail of
/// `ClassName(args)` and of `type(self)(args)` (#1411), whose class is the
/// one `self` is typed as in the body being lowered.
fn lower_instantiation(
    callee: &str,
    class_def: &HirClassDef,
    args: Vec<MirExpr>,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    // #432: resolve `__init__` via the MRO -- a derived class
    // without its own `__init__` inherits the base class's
    // constructor. The MRO is ordered most-derived-first.
    //
    // #966: but the *first* `__init__` is not always the right
    // one. D-225 puts an implicit zero-argument constructor in
    // the own method table of every class that declares none,
    // so for `class C(A, B)` with `A` init-less and `B`
    // declaring `__init__`, `A`'s stub would win and `B`'s
    // constructor would never run -- leaving `B`'s slots
    // uninitialized. CPython ranks the equivalent
    // (`object.__init__`) last, so skip flagged classes on a
    // first pass. The `or_else` pass covers the all-implicit
    // MRO (`class A: pass` / `class B: pass` / `class C(A, B)`),
    // where the implicit constructor really is the one to call;
    // only a genuinely `__init__`-less MRO reaches the panic.
    let ctor_in = |skip_implicit: bool| {
        class_def.mro.iter().find_map(|mro_class| {
            let mro_def = classes.get(mro_class.as_str())?;
            if skip_implicit && mro_def.implicit_object_init {
                return None;
            }
            if mro_def.methods.iter().any(|(mn, _)| mn == "__init__") {
                Some((mro_class, format!("{mro_class}.__init__")))
            } else {
                None
            }
        })
    };
    let (owner, ctor) = ctor_in(true).or_else(|| ctor_in(false)).unwrap_or_else(|| {
        panic!(
            "pycc_mir: internal error: no `__init__` found in class `{callee}`'s \
             MRO -- pycc_hir guarantees an `__init__` for every non-enum class it \
             lowers (D-225: by inheritance or by synthesis), and a call to an enum \
             class is C0001 before MIR lowering -- pycc_hir's per-item scan reports \
             it at the call (#944) with pycc_types' guard behind it (#921)"
        )
    });
    // #1337 (D-254): an inherited constructor runs its
    // receiver-exact copy when one exists.
    let ctor = exact_callee(callee, owner, ctor, scopes, classes);
    MirExpr::Instantiate(Box::new(InstantiateExpr {
        ctor,
        class_name: class_source_name(callee, classes),
        // #432: allocate slots for all unique attributes across the
        // MRO, not just this class's own declared attributes.
        slot_names: mro_attrs(class_def, classes)
            .into_iter()
            .map(|(name, _)| name)
            .collect(),
        args,
        ty: Ty::Instance(Box::new(callee.to_string())),
    }))
}

pub(super) fn lower_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    match expr {
        HirExpr::IntLiteral(n) => MirExpr::IntLiteral(*n),
        HirExpr::FloatLiteral(f) => MirExpr::FloatLiteral(*f),
        HirExpr::BoolLiteral(b) => MirExpr::BoolLiteral(*b),
        // Only the test of `pycc_hir::loop_else_as_if`, whose `if` is
        // lowered for its body alone (`HirStmt::LoopElse` discards the
        // test); no emitted code ever evaluates this value.
        HirExpr::OpaqueBool => MirExpr::BoolLiteral(true),
        HirExpr::StringLiteral(s) => MirExpr::StringLiteral(s.clone()),
        HirExpr::NoneLiteral => MirExpr::NoneLiteral,
        HirExpr::NotImplemented => MirExpr::NotImplemented,
        // #1021: a `[]`/`{}` whose element type `pycc_types`'
        // empty-container pre-pass resolved. The carried `Ty` crosses
        // straight into MIR, because MIR has nothing to derive it from --
        // see `MirExpr::EmptyList`'s own doc comment.
        HirExpr::EmptyList(element) => MirExpr::EmptyList(element.clone()),
        HirExpr::EmptySet(element) => MirExpr::EmptySet(element.clone()),
        HirExpr::EmptyDict(pair) => MirExpr::EmptyDict(pair.clone()),
        // D-136: `math.pi` (a `pycc_hir`-qualified stdlib constant name --
        // real Python identifiers never contain `.`, see
        // `pycc_types::std_qualified_symbol`'s own doc comment for the
        // same invariant) is never bound in `scopes` the way an ordinary
        // assigned variable is, so it needs its own arm here rather than
        // falling into the ordinary `lookup` below, which would panic.
        HirExpr::Name(name)
            if matches!(
                name.as_str(),
                "math.pi" | "math.e" | "math.tau" | "math.inf" | "math.nan"
            ) =>
        {
            MirExpr::Name {
                name: name.clone(),
                ty: Ty::Float,
            }
        }
        // Issue #769 (Part 2 of #747): a name read inside a narrowing-
        // eligible branch (`super::narrowed_ty`'s `$narrowed:{name}`
        // sentinel, pushed by `stmt::lower_stmt`'s `HirStmt::If` arm) is
        // wrapped in `OptionalUnwrap` so `.ty()` reports the Optional's
        // inner type for this read alone, without touching the slot's own
        // still-`Optional` declared representation looked up via `lookup`
        // just below. Checked before the plain `Name` arm so a narrowed
        // read never falls through to it.
        //
        // #1476: over an `object` slot narrowed by an `isinstance` guard the
        // read is an `ObjectUnbox` instead (`crate::object_narrow`).
        HirExpr::Name(name) if super::narrowed_ty(scopes, name).is_some() => {
            let inner = super::narrowed_ty(scopes, name).expect("just matched Some above");
            super::object_narrow::narrowed_read(name, lookup(scopes, name), inner)
        }
        // A module-level function used as a value: its function value.
        HirExpr::Name(name)
            if !scopes.iter().any(|scope| scope.contains_key(name))
                && scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$callable:{name}"))) =>
        {
            MirExpr::MakeClosure {
                function: name.clone(),
                captures: Vec::new(),
                ty: lookup(scopes, &format!("$callable:{name}")),
            }
        }
        HirExpr::Name(name) => MirExpr::Name {
            name: name.clone(),
            ty: lookup(scopes, name),
        },
        HirExpr::ReceiverClassCall { .. } => {
            call_lower::lower_receiver_class_call_expr(expr, scopes, classes, current_class)
        }
        HirExpr::Call { .. } => call_lower::lower_call_expr(expr, scopes, classes, current_class),
        HirExpr::UnaryOp { .. } => {
            operator_lower::lower_unary_op_expr(expr, scopes, classes, current_class)
        }
        HirExpr::BoolOp {
            op,
            left,
            right,
            truth_only,
        } => super::boolop::lower_bool_op(
            *op,
            lower_expr(left, scopes, classes, current_class),
            lower_expr(right, scopes, classes, current_class),
            *truth_only,
        ),
        HirExpr::Unpack { .. } => {
            operator_lower::lower_unpack_expr(expr, scopes, classes, current_class)
        }
        HirExpr::IfExp { test, body, orelse } => super::if_exp::lower_if_exp(
            lower_expr(test, scopes, classes, current_class),
            lower_expr(body, scopes, classes, current_class),
            lower_expr(orelse, scopes, classes, current_class),
        ),
        HirExpr::BinOp { .. } | HirExpr::AugBinOp { .. } => {
            operator_lower::lower_bin_op_expr(expr, scopes, classes, current_class)
        }
        HirExpr::Compare { .. } => {
            operator_lower::lower_compare_expr(expr, scopes, classes, current_class)
        }
        HirExpr::CompareChain { .. } => {
            operator_lower::lower_compare_chain_expr(expr, scopes, classes, current_class)
        }
        HirExpr::FString(..) => {
            operator_lower::lower_f_string_expr(expr, scopes, classes, current_class)
        }
        // Part 2d of #1371: a list display the empty-container pre-pass
        // resolved to an object slot.
        HirExpr::ObjectList(elements) => MirExpr::ObjList {
            elements: elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect(),
        },
        HirExpr::ListLiteral(..) => {
            container_lower::lower_list_literal_expr(expr, scopes, classes, current_class)
        }
        HirExpr::Subscript { .. } => {
            container_lower::lower_subscript_expr(expr, scopes, classes, current_class)
        }
        HirExpr::ListAppend { list, value } => MirExpr::ListAppend {
            list: lower_container_receiver(list, scopes, classes, current_class).0,
            value: Box::new(lower_expr(value, scopes, classes, current_class)),
        },
        HirExpr::DictLiteral(..) => {
            container_lower::lower_dict_literal_expr(expr, scopes, classes, current_class)
        }
        HirExpr::SetLiteral(..) => {
            container_lower::lower_set_literal_expr(expr, scopes, classes, current_class)
        }
        HirExpr::TupleLiteral(elements) => {
            let elements: Vec<MirExpr> = elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect();
            if super::obj_arith::is_object_display(pycc_hir::DisplayKind::Tuple, &elements) {
                return super::obj_arith::object_display(crate::ObjDisplayKind::Tuple, elements);
            }
            MirExpr::TupleLiteral(elements)
        }
        HirExpr::Slice { .. } => {
            container_lower::lower_slice_expr(expr, scopes, classes, current_class)
        }
        HirExpr::ListPop { .. } => {
            container_lower::lower_list_pop_expr(expr, scopes, classes, current_class)
        }
        HirExpr::DictGetOrDefault { .. } => {
            container_lower::lower_dict_get_or_default_expr(expr, scopes, classes, current_class)
        }
        HirExpr::SetAdd { set, value } => {
            let Ty::Set(elem_ty) = lookup(scopes, set) else {
                panic!("pycc_types admits `.add()` only on a set")
            };
            MirExpr::SetAdd {
                set: set.clone(),
                value: Box::new(lower_expr(value, scopes, classes, current_class)),
                ops: set_ops::lower_set_element_ops(&elem_ty, scopes, classes).map(Box::new),
            }
        }
        HirExpr::AttrGet { .. } => {
            attr_lower::lower_attr_get_expr(expr, scopes, classes, current_class)
        }
        // D-154 (Part 1 of #375): `base.method(args)` resolves to the
        // method's mangled, compile-time-known function symbol (D-006's
        // static-dispatch framing for a non-inherited class -- no vtable,
        // no runtime dispatch), then lowers into an ordinary `MirExpr::Call`
        // with `base` prepended as `self` -- unlike instantiation, a method
        // call never allocates, so it needs no dedicated MIR node of its
        // own (see `MirExpr::Instantiate`'s own doc comment for the
        // contrasting case).
        // #432: method resolution walks the MRO, so a method declared in a
        // base class is found when called on a derived class instance. A
        // subclass method shadows a base class method of the same name (the
        // subclass appears first in the MRO).
        // Issue #1188: one of the four container method names in a module
        // that can see a user class defining it; lowered under the reading
        // `pycc_types` chose.
        HirExpr::ReceiverDispatchedCall { call, container } => {
            receiver_dispatch::lower_receiver_dispatched_call(
                call,
                container,
                scopes,
                classes,
                current_class,
            )
        }
        HirExpr::MethodCall { .. } => {
            attr_lower::lower_method_call_expr(expr, scopes, classes, current_class)
        }
        HirExpr::ExprCall { .. } => {
            call_lower::lower_expr_call_expr(expr, scopes, classes, current_class)
        }
        HirExpr::KeywordCall { .. } => {
            call_lower::lower_keyword_call_expr(expr, scopes, classes, current_class)
        }
        // PEP 695 (#387): `GenericClassInstantiate` should never reach MIR
        // — `pycc_types::monomorphize` rewrites every
        // `GenericClassInstantiate` expression to an ordinary
        // `HirExpr::Call` to the mangled class name before MIR lowering
        // runs. If this arm is reached, it indicates a bug in the
        // monomorphization pass.
        HirExpr::GenericClassInstantiate { class, .. } => {
            panic!(
                "pycc_mir: internal error: `GenericClassInstantiate` for class `{class}` \
                 reached MIR lowering -- pycc_types::monomorphize should have rewritten it \
                 to an ordinary `HirExpr::Call` before this point"
            )
        }
        // PEP 572 (#774): `target := value`. Lowers `value` first (bottom-up,
        // like every other composite arm here), then wraps it in
        // `MirExpr::NamedExpr` carrying the lowered value's own type -- this
        // node's own "evaluate, store, yield" codegen (see
        // `pycc_codegen::emit_expr_unchecked`'s `MirExpr::NamedExpr` arm)
        // does the actual store. This function only ever holds a read-only
        // `scopes: &[HashMap<String, Ty>]` slice, so it cannot itself
        // register `name`'s binding for a later statement to `lookup` --
        // that happens one level up, in `pycc_mir::stmt::lower_stmt`'s
        // `ExprStmt`/`If`/`While` arms, via
        // `pycc_mir::stmt::collect_named_expr_bindings`, which walks the
        // already-lowered `MirExpr` tree (so it can read `ty()` directly
        // rather than re-inferring it) and calls `bind_variable` for every
        // `NamedExpr` node found, before that statement's body/next
        // statement is lowered. `pycc_hir::stmt::lower_stmt`'s own
        // `contains_named_expr` restriction guarantees a `NamedExpr` node
        // only ever appears nested inside exactly those three statement
        // placements, so no other `lower_stmt` arm needs this treatment.
        // PEP 572 (#774): by the time this arm runs, `pre_bind_named_expr_targets`
        // (called by `pycc_mir::stmt::lower_stmt` before it lowers the whole
        // statement) has already bound `name` into `scopes` -- see that
        // function's own doc comment for why a pre-pass, not this ordinary
        // bottom-up lowering, has to be the one doing the actual `scopes`
        // mutation. This arm's own job is unchanged from a first glance:
        // lower `value` (again -- cheap, `value` has no side effects beyond
        // further nested walrus bindings the pre-pass already applied) and
        // wrap it in `MirExpr::NamedExpr` carrying its type.
        HirExpr::NamedExpr { name, value } => {
            let value = lower_expr(value, scopes, classes, current_class);
            let ty = value.ty();
            MirExpr::NamedExpr {
                name: name.clone(),
                value: Box::new(value),
                ty,
            }
        }
        HirExpr::Comprehension(..) => {
            container_lower::lower_comprehension_expr(expr, scopes, classes, current_class)
        }
        HirExpr::Nd(nd) => MirExpr::Nd(Box::new(super::MirNdExpr {
            op: nd.op,
            base: lower_expr(&nd.base, scopes, classes, current_class),
            args: nd
                .args
                .iter()
                .map(|arg| lower_expr(arg, scopes, classes, current_class))
                .collect(),
            nd: nd.nd,
            cached: nd.cached,
        })),
        // #433: a bare `HirExpr::Super` should never reach MIR lowering —
        // HIR lowering rejects a standalone `super()` with C0001, and
        // `super().method()`/`super().attr` are handled by the special-case
        // blocks above before recursing into `lower_expr` for the base.
        HirExpr::Super => {
            panic!(
                "pycc_mir: internal error: a bare `HirExpr::Super` reached MIR lowering -- \
                 pycc_hir::lower_expr should have rejected this with C0001, or the \
                 `MethodCall`/`AttrGet` arms should have intercepted it before recursing"
            )
        }
    }
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, value }`
/// node, in the expression's own left-to-right evaluation order, and binds
/// each `name` into `scopes` *before* `lower_stmt` calls `lower_expr` on the
/// whole enclosing statement.
///
/// This pre-pass exists because `lower_expr` itself only ever holds a
/// read-only `scopes: &[HashMap<String, Ty>]` slice -- by design, since it is
/// a pure bottom-up lowering with no other reason to mutate scope state. That
/// is fine for every ordinary case, where a `NamedExpr`'s binding only needs
/// to be visible to a *later statement* (handled by
/// `pycc_mir::stmt::collect_named_expr_bindings`, which walks the
/// already-lowered `MirExpr` tree after the fact). It breaks down for a
/// walrus value that itself references an *earlier walrus in the same
/// expression* -- `(a := 1) + (b := a + 1)` -- where `b`'s value needs `a`
/// already bound in `scopes` while `lower_expr` is still partway through
/// lowering the very same top-level expression that defines `a`. Without
/// this pre-pass, `lower_expr`'s `HirExpr::Name("a")` arm would call
/// `lookup(scopes, "a")`, which panics (`pycc_types::check` already proved
/// the program well-typed, including this exact ordering, by running its own
/// mirror-image pre-pass -- see `pycc_types::collect_named_expr_bindings`'s
/// doc comment for the type-checker side of this same fix).
///
/// Each `name`'s type is computed by actually lowering its `value` here
/// (recursing into `value` first, so a nested `NamedExpr` inside it is bound
/// before `value` itself is lowered) -- cheap and side-effect-free (beyond
/// the further nested bindings this same pre-pass already applies), and the
/// simplest way to get a `Ty` without re-deriving one structurally from the
/// unlowered `HirExpr`. `lower_stmt`'s own subsequent full-statement
/// `lower_expr` call re-lowers this same subtree, which is redundant but
/// harmless: `bind_variable`'s `.entry(..).or_insert(..)` semantics make a
/// repeated bind of the same name a no-op.
pub(super) fn pre_bind_named_expr_targets(
    expr: &HirExpr,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
            let lowered_value = lower_expr(value, scopes, classes, current_class);
            let ty = lowered_value.ty();
            // D-068 review of #780/#774's interaction (blocker finding 1): a
            // walrus target is a reassignment exactly like `Assign`'s own arm
            // in `stmt.rs` (see its paired `kill_narrowing`/`bind_variable`
            // calls), so it must clear any stale narrowing sentinel for
            // `name` the same way -- otherwise a subsequent read still
            // lowers to an unconditional `MirExpr::OptionalUnwrap` for a
            // value the walrus may have just overwritten with `None`.
            super::kill_narrowing(scopes, name);
            super::bind_variable(scopes, name.clone(), ty);
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
        }
        // #1254 (D-250): `pycc_hir` refuses a walrus inside a comprehension.
        HirExpr::Comprehension(_) => {}
        HirExpr::Nd(_) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                pre_bind_named_expr_targets(sub, scopes, classes, current_class);
            }
        }
        HirExpr::Call { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            pre_bind_named_expr_targets(left, scopes, classes, current_class);
            pre_bind_named_expr_targets(right, scopes, classes, current_class);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                pre_bind_named_expr_targets(part, scopes, classes, current_class);
            }
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                pre_bind_named_expr_targets(operand, scopes, classes, current_class);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            pre_bind_named_expr_targets(operand, scopes, classes, current_class)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    pre_bind_named_expr_targets(inner, scopes, classes, current_class);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                pre_bind_named_expr_targets(e, scopes, classes, current_class);
            }
        }
        HirExpr::Subscript { base, index } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            pre_bind_named_expr_targets(index, scopes, classes, current_class);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            for bound in [start, stop, step].into_iter().flatten() {
                pre_bind_named_expr_targets(bound, scopes, classes, current_class);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
        }
        HirExpr::SetAdd { value, .. } => {
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                pre_bind_named_expr_targets(k, scopes, classes, current_class);
                pre_bind_named_expr_targets(v, scopes, classes, current_class);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
            pre_bind_named_expr_targets(key, scopes, classes, current_class);
            pre_bind_named_expr_targets(default, scopes, classes, current_class);
        }
        HirExpr::AttrGet { base, .. } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class)
        }
        HirExpr::MethodCall { base, args, .. } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            pre_bind_named_expr_targets(call, scopes, classes, current_class)
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            pre_bind_named_expr_targets(callee, scopes, classes, current_class);
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            pre_bind_named_expr_targets(call, scopes, classes, current_class);
            for (_, value) in keywords {
                pre_bind_named_expr_targets(value, scopes, classes, current_class);
            }
        }
    }
}

/// #911 (Part 1 of #885): folds a class-level attribute read into the
/// literal recorded in `HirClassDef::class_attrs`, or `None` when
/// `class_def` declares no attribute of that name.
///
/// A class attribute has no runtime storage at all -- no instance slot, no
/// module global (unlike an enum member's singleton), and no `pycc_codegen`
/// footprint -- so every read is resolved here, at MIR-lowering time.
fn fold_class_attr(class_def: &HirClassDef, attr: &str) -> Option<MirExpr> {
    let (_, _, value) = class_def
        .class_attrs
        .iter()
        .find(|(name, _, _)| name == attr)?;
    Some(match value {
        ClassAttrValue::Int(i) => MirExpr::IntLiteral(*i),
        ClassAttrValue::Float(f) => MirExpr::FloatLiteral(*f),
        ClassAttrValue::Bool(b) => MirExpr::BoolLiteral(*b),
        ClassAttrValue::Str(s) => MirExpr::StringLiteral(s.clone()),
        // Part 1 of #1284: no constant to fold. Every class-name and
        // instance read reaches `foreign_static`'s rewrite before this
        // fold, so the only caller that gets here is the `super()` walk,
        // which `pycc_types` refuses; returning `None` lets that walk fall
        // through to its own internal-error panic.
        ClassAttrValue::ForeignStatic(_) => return None,
    })
}

/// Lowers the receiver of a `ListAppend`/`ListPop`/`DictGetOrDefault` node
/// together with its static type (#1263): a bare name keeps the plain name
/// and its scope type, exactly as before; an attribute receiver is lowered
/// like any other expression (a slot read, or a `@property` getter call)
/// and typed by that lowered read.
fn lower_container_receiver(
    receiver: &ContainerReceiver,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (MirContainerReceiver, Ty) {
    match receiver {
        ContainerReceiver::Name(name) => (
            MirContainerReceiver::Name(name.clone()),
            lookup(scopes, name),
        ),
        ContainerReceiver::Attr(read) => {
            let lowered = lower_expr(read, scopes, classes, current_class);
            let ty = lowered.ty();
            (MirContainerReceiver::Attr(Box::new(lowered)), ty)
        }
    }
}
