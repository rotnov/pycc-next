pub use pycc_hir::{
    EnumMemberValue, FromImport, HirClassDef, InheritedCopy, foreign_binds_root,
    inherited_copy_origin,
};
mod binop;
use binop::binop_result_ty;
mod boolop;
mod compare_chain;
mod if_exp;
pub use compare_chain::{MirCompareKind, MirCompareLink};
mod class;
#[cfg(test)]
use class::eval_isinstance_protocol;
use class::{class_def_of, mro_attrs};
mod exception;
mod exception_isinstance;
pub use exception::{MirExceptHandler, MirExceptionValue};
use exception::{handler_type_tags, lower_raise};
mod expr;
use expr::lower_expr;
mod matching;
#[cfg(test)]
use matching::nest_match_alternatives;
use matching::try_lower_enum_member_attr;
mod item_names;
#[cfg(test)]
use item_names::resolve_method_owner_class;
use item_names::{item_anchor_class, source_frame_name};
mod obj_compare;
mod object_box;
mod object_narrow;
pub use obj_compare::{ObjBuiltinClass, ObjIsInstanceClass};
mod receiver_exact;
mod set_ops;
pub use set_ops::{SetElementOps, SetEqOp, SetHashOp};
mod stmt;
#[cfg(debug_assertions)]
mod verify_receiver;
use pycc_hir::{CompIter, ForeignImportSite, HirItem, HirModule, HirStmt, ImportBinding};
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use stmt::lower_stmt;

// Re-exported (not just `use`d) because `pycc_codegen` doesn't depend on
// `pycc_hir` directly (see its Cargo.toml) -- `Ty`, `BinOpKind`, and
// `CmpOpKind` all reach this crate's public API through `MirExpr`'s fields
// (`Name`/`Call` carry a `Ty`; `BinOp` carries a `BinOpKind`; `Compare`
// carries a `CmpOpKind`), so each must be nameable as
// `pycc_mir::{Ty, BinOpKind, CmpOpKind}` from any downstream crate, exactly
// like `pycc_types` already re-exports `Ty` (`pycc_types::Ty`, its own line
// 4) for the same reason.
pub use pycc_hir::{BinOpKind, BoolOpKind, CmpOpKind, EXCEPTION_GROUP_TYPE_TAG, Ty};

/// Monotonic counter for synthesized match-subject temporaries. Each
/// `match` statement gets a unique `__match_subj_N` name, avoiding
/// collisions between multiple matches at the same scope level (which
/// would otherwise share the same `scopes.len()`-derived name and
/// cause a first-assignment-wins type drift in `bind_variable`).
static MATCH_SUBJECT_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The receiver of `MirExpr::ListAppend`/`ListPop`/`DictGetOrDefault`
/// (mirrors `pycc_hir::ContainerReceiver`, #1263).
#[derive(Debug, Clone, PartialEq)]
pub enum MirContainerReceiver {
    /// A bare variable name, read through the codegen's name-read helpers
    /// exactly as before #1263.
    Name(String),
    /// A lowered attribute read (`MirExpr::AttrGet`, or a property getter
    /// call), whose value is the container pointer. Codegen evaluates it
    /// before the node's own arguments, matching CPython's order.
    Attr(Box<MirExpr>),
}

impl MirContainerReceiver {
    /// The attribute read of an [`MirContainerReceiver::Attr`] receiver;
    /// `None` for a bare name, which has no sub-expression to walk.
    #[must_use]
    pub fn attr_expr(&self) -> Option<&MirExpr> {
        match self {
            MirContainerReceiver::Name(_) => None,
            MirContainerReceiver::Attr(receiver) => Some(receiver),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirExpr {
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    /// A `bool` crossing a statically-`int` boundary without becoming the
    /// arithmetic integer `0` or `1`.
    IntBoundary(Box<MirExpr>),
    StringLiteral(String),
    /// The bare literal `None` (D-197, #763, Part 1 of #747). Statically
    /// `Ty::None` on its own (mirroring `HirExpr::NoneLiteral`) -- when it
    /// (or a bare `inner`-typed value) flows into an `Optional[inner]`-typed
    /// slot, `pycc_codegen`'s `coerce_scalar_to_type` builds that slot's
    /// `{ inner, i8 }` present/absent representation, driven by the target
    /// type it is asked to coerce to. `OptionalWrap` below is the one place
    /// that target type is *not* simply "the slot's already-established
    /// type" -- see its own doc comment.
    NoneLiteral,
    /// CPython's `NotImplemented` singleton, the value of an admitted
    /// `return NotImplemented` in a comparison method of an `ext` module
    /// (#1418, mirroring `HirExpr::NotImplemented`). Statically
    /// `Ty::Object`; codegen loads it through the
    /// `pycc_ext_obj_not_implemented` shim.
    NotImplemented,
    /// Wraps a bare `inner`-typed value or a `NoneLiteral` so `.ty()`
    /// reports `Ty::Optional(inner)` regardless of the wrapped value's own
    /// static type (D-197, #763, Part 1 of #747). Exactly mirroring
    /// `IntBoundary`'s own "first assignment fixes a binding's
    /// representation" reason: `pycc_codegen::collect_stmt_bindings`
    /// derives a `MirStmt::Assign`'s slot type from `value.ty()` alone, with
    /// no memory of the HIR annotation that produced it, so an
    /// `AnnAssign`'s initializer under an `Optional[inner]` annotation must
    /// itself statically report `Ty::Optional(inner)` for the slot to be
    /// declared with the right representation at all -- unlike `IntBoundary`,
    /// the actual `{ inner, i8 }` struct-building work still happens at
    /// codegen (`pycc_codegen::coerce_scalar_to_type`, called with
    /// `Ty::Optional(inner)` as the target), not here; this node exists
    /// purely to fix `.ty()`. Only `pycc_mir::stmt::lower_stmt`'s
    /// `AnnAssign` arm introduces it, for the identical reason `IntBoundary`
    /// is introduced only there and not by a bare `Assign`: a later plain
    /// reassignment to an already-`Optional`-typed name (`x = None` after
    /// `x: int | None = 5`) needs no wrapper at all, because
    /// `pycc_mir::bind_variable`'s `or_insert` never overwrites the slot
    /// type the `AnnAssign` already fixed (D-074), and
    /// `coerce_scalar_to_type` is driven by that already-correct slot type
    /// directly at the assignment site either way.
    OptionalWrap(Box<MirExpr>, Box<Ty>),
    /// A native `int`, `float`, `bool`, `str`, compiled-instance or `None`
    /// value boxed into an `object` slot (Part 2 of #1387, D-258's boxing
    /// amendment). Statically `Ty::Object`, so the slot
    /// `pycc_codegen::collect_stmt_bindings` derives from `value.ty()` is
    /// the `PyObject *` one, exactly as `OptionalWrap` fixes an `Optional`
    /// slot. Unlike `OptionalWrap` it is a *converting* node:
    /// `pycc_codegen`'s `object_box` module packs the value through the
    /// shim's `pycc_ext_obj_pack_*` helpers, and a packer `NULL` (the
    /// #1040 bigint `OverflowError`) takes the foreign failure edge.
    ///
    /// `stmt::lower_stmt` inserts it at the three statement seams whose
    /// slot type it knows -- an `AnnAssign` under an `object` annotation,
    /// a plain rebinding of an `object`-typed name and an attribute store
    /// into an `object`-typed slot. A call argument and a returned value
    /// are boxed by codegen itself, which knows the parameter and return
    /// types this pass does not. `pycc_types::object_box::admits` owns
    /// which values reach it.
    ObjectBox(Box<MirExpr>),
    /// Issue #769 (Part 2 of #747): the read-side counterpart of
    /// `OptionalWrap` above. `pycc_types::check` has already proven, via its
    /// own flow-sensitive `narrowed` overlay (`pycc_types::narrow`), that a
    /// particular *read* of an `Optional[inner]`-typed name is reachable
    /// only when the value is actually present -- e.g. any read of `x`
    /// inside `if x is not None: ...`. This node re-states that already-
    /// proven fact for codegen: unlike `OptionalWrap`, which fixes `.ty()`
    /// to *widen* a bare value's static type to `Optional(inner)` so a slot
    /// gets declared with the right representation, `OptionalUnwrap`
    /// *narrows* `.ty()` back down to the plain `inner` type for this one
    /// read, without touching the underlying slot's own declared
    /// `Optional(inner)` representation at all (the boxed `Ty` here is that
    /// same `inner` type, carried for codegen the same way `OptionalWrap`
    /// carries its own target type). Only `pycc_mir::expr::lower_expr`'s
    /// `HirExpr::Name` arm introduces it, driven by `pycc_mir::lib`'s own
    /// `$narrowed:{name}` scope sentinel (see `narrowed_ty` below) that
    /// `pycc_mir::stmt::lower_stmt`'s `HirStmt::If` arm pushes for a
    /// narrowing-eligible branch, mirroring `pycc_types::narrow`'s own
    /// `Environment::narrowed` overlay one layer down. `pycc_codegen` reads
    /// the runtime `Scalar::Optional { inner, present }` representation
    /// directly for this node (see its own doc comment in
    /// `pycc_codegen::bigint_rc` for the refcount reasoning); it is a
    /// pure read, never a slot's own storage type.
    OptionalUnwrap(Box<MirExpr>, Box<Ty>),
    /// #1476 (Part 3 of #1387): the read-side counterpart of `ObjectBox`. A
    /// read of an `object`-typed name inside a region an `isinstance(name,
    /// C)` guard narrows (`crate::object_narrow`), unboxed to the guard's
    /// native type -- the carried `Ty`, one of `int`, `float`, `bool`,
    /// `str` or a compiled class's `Instance`. The operand is always the
    /// `object`-typed `Name` itself.
    ///
    /// Unlike `OptionalUnwrap` this node can fail at run time: codegen calls
    /// a checked `pycc_ext_obj_unbox_*` helper per read, which re-tests the
    /// type and raises `TypeError` (or `OverflowError` for an `int` outside
    /// D-141's inline range) instead of trusting the guard, because the
    /// kill prescan is syntactic and cannot see a callee rebinding a
    /// global. A read that flows straight back into an object position --
    /// a packed CPython operand, an `ObjectBox` -- uses the operand instead
    /// (`pycc_codegen::object_box`'s identity peephole), so the original
    /// object, not an unbox-then-repack copy, reaches CPython.
    ObjectUnbox(Box<MirExpr>, Box<Ty>),
    Name {
        name: String,
        ty: Ty,
    },
    Call {
        callee: String,
        args: Vec<MirExpr>,
        ty: Ty,
    },
    BinOp {
        op: BinOpKind,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
        ty: Ty,
    },
    Compare {
        op: CmpOpKind,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
        ty: Ty,
    },
    /// A chained comparison `first op1 r1 op2 r2 ...` (#1212, Part 4 of
    /// #1018); `links.len() >= 2`. Every operand is evaluated at most once,
    /// left to right, and evaluation stops at the first false link. Always
    /// `Ty::Bool`. `pycc_codegen` builds the short circuit with basic blocks
    /// and a join, like [`MirExpr::BoolOp`].
    CompareChain {
        first: Box<MirExpr>,
        links: Vec<MirCompareLink>,
    },
    /// `not x` (#604, Part 3 of #573). Unlike `USub`/`UAdd`/`Invert`, `not`
    /// has no equivalent `BinOp` shape to rewrite into -- its truthiness
    /// rule spans operand types (`int`, `float`, `str`, `Optional`) that
    /// `BinOp`'s own typing has no notion of -- so it is MIR's one
    /// dedicated unary node. Always `Ty::Bool`; `pycc_codegen` computes it
    /// with the same `truthy` helper an `if`/`while` condition already
    /// calls, then inverts the result.
    Not(Box<MirExpr>),
    /// `left and right` / `left or right` (#1211, Part 3 of #1018), right-
    /// folded by HIR lowering so `a or b or c` is `Or(a, Or(b, c))`. MIR
    /// stays tree-shaped: `pycc_codegen` builds the short circuit with basic
    /// blocks and a join, testing each operand's truth at most once.
    ///
    /// `truth_only` is HIR's context flag (`pycc_hir::boolop`). A truth-only
    /// node is `Ty::Bool` and joins truth bits; any other node's `ty` is
    /// `pycc_hir::bool_op_result_ty` of its operands, and it joins the
    /// selected operand's value converted to `ty`.
    BoolOp {
        op: BoolOpKind,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
        ty: Ty,
        truth_only: bool,
    },
    /// `body if test else orelse` (#1395). `test` is evaluated once, for
    /// its truth, then exactly one branch. `ty` is
    /// `pycc_hir::if_exp_result_ty` of the two branches, and `pycc_codegen`
    /// joins the selected branch converted to `ty` with a phi.
    IfExp {
        test: Box<MirExpr>,
        body: Box<MirExpr>,
        orelse: Box<MirExpr>,
        ty: Ty,
    },
    FString(Vec<MirFStringPart>),
    /// `[e1, e2, ...]`. No `ty` field: `ty()` below derives
    /// `Ty::List(Box::new(elements[0].ty()))` from the first element,
    /// exactly like `pycc_types::infer_expr_in`'s own `HirExpr::ListLiteral`
    /// arm derives the list's type from its elements rather than assuming
    /// `Ty::Int`. Empirically only `Ty::List(Box::new(Ty::Int))` ever
    /// reaches this crate today (`pycc_types`' T0034 gate rejects every
    /// other element type at construction time -- see that gate's own
    /// comment and its `a_for_list_loop_binds_its_variable_as_str_for_a_list_of_str`
    /// genericity test), but deriving here -- rather than hardcoding what
    /// today's upstream gate happens to allow -- keeps this lowering
    /// correct on its own terms, and correct automatically if that gate is
    /// ever relaxed, without requiring a matching `pycc_mir` change.
    ListLiteral(Vec<MirExpr>),
    /// A resolved empty `[]` (mirrors [`pycc_hir::HirExpr::EmptyList`],
    /// #1021, D-245). The carried `Ty` is the *element* type, so `ty()`
    /// returns `Ty::List(element)` without deriving anything from the
    /// (absent) elements -- which is the whole point of the variant:
    /// `ListLiteral`'s own `ty()` arm has nothing to derive from and panics.
    /// Codegen emits the same `pycc_rt_int_list_new()` object
    /// `ListLiteral(vec![])` would, with no `append` calls after it.
    EmptyList(Ty),
    /// `base[index]`, read-only (mirrors `HirExpr::Subscript`, D-105).
    /// `ty()` below derives its result from `base.ty()`'s element type
    /// (mirroring `pycc_types::infer_expr_in`'s own `Subscript` arm), for
    /// the same reason `ListLiteral` above derives rather than hardcodes.
    Subscript {
        base: Box<MirExpr>,
        index: Box<MirExpr>,
    },
    /// `list.append(value)` (mirrors `HirExpr::ListAppend`, D-105). `list` is
    /// a [`MirContainerReceiver`]: the plain variable name, exactly like
    /// `HirExpr::ListAppend` itself, or (#1263) a lowered attribute read,
    /// evaluated before `value`.
    ListAppend {
        list: MirContainerReceiver,
        value: Box<MirExpr>,
    },
    /// `{k1: v1, k2: v2, ...}` (mirrors `HirExpr::DictLiteral`, PR-11 Task 4).
    /// No `ty` field: `ty()` below derives `Ty::Dict(Box::new((pairs[0].0.ty(),
    /// pairs[0].1.ty())))` from the first pair, exactly like `ListLiteral`
    /// above derives its element type from its first element rather than
    /// hardcoding one. Empirically only `Ty::Dict(Box::new((Ty::Str,
    /// Ty::Int)))` ever reaches this crate today (`pycc_types`' T0036 gate
    /// rejects every other key/value combination at construction time), but
    /// deriving here keeps this lowering correct on its own terms rather
    /// than baking in an assumption this crate has no way to verify
    /// independently.
    DictLiteral(Vec<(MirExpr, MirExpr)>),
    /// A resolved empty `{}` (mirrors [`pycc_hir::HirExpr::EmptyDict`],
    /// #1021, D-245). The dict counterpart of [`MirExpr::EmptyList`]: the
    /// carried pair is the key/value type, `ty()` returns `Ty::Dict(pair)`,
    /// and codegen emits a bare `pycc_rt_dict_new()` with no `set` calls.
    EmptyDict(Box<(Ty, Ty)>),
    /// `dict[key]`, read-only (mirrors `HirExpr::Subscript` on a
    /// dict-typed base -- see `lower_expr`'s own `HirExpr::Subscript` arm
    /// for why a dict-typed base is routed here instead of into
    /// `MirExpr::Subscript`). `ty()` below derives its result from
    /// `dict.ty()`'s value type, for the same reason `Subscript` derives
    /// from `base.ty()`'s element type rather than hardcoding one.
    DictGet {
        dict: Box<MirExpr>,
        key: Box<MirExpr>,
    },
    /// `{e1, e2, ...}` (mirrors `HirExpr::SetLiteral`, PR-11 Task 8). No
    /// `ty` field: `ty()` below derives `Ty::Set(Box::new(elements[0].ty()))`
    /// from the first element, exactly like `ListLiteral`/`DictLiteral`
    /// above derive their own element/key-value type from their first
    /// element/pair rather than hardcoding one. Only a `set[int]` or (since
    /// #1343) a set of a user-class instance reaches this crate
    /// (`pycc_types`' T0037/T0038 gates reject every other element type at
    /// construction time), but deriving here keeps this lowering correct on
    /// its own terms rather than baking in an assumption this crate has no
    /// way to verify independently. Unlike `ListLiteral`/`DictLiteral`, an
    /// *empty* `SetLiteral` is impossible from real Python source in an even
    /// stronger sense than "`pycc_types::check` would reject it": Python's
    /// own grammar has no empty-set-literal syntax at all (`{}` parses as an
    /// empty `dict`, not an empty `set` -- CPython's own grammar routes a
    /// bare `{}` to `ast.Dict`, never `ast.Set`), so `elements` is non-empty
    /// for every `SetLiteral` `pycc_hir::lower_expr`'s own `Expr::Set` arm
    /// could ever construct from a real parse.
    ///
    /// #1343: the element is an `int` or a user-class instance. `ops` is
    /// `None` for `int` and carries the instance class's hash and equality
    /// otherwise ([`SetElementOps`]).
    SetLiteral {
        elements: Vec<MirExpr>,
        ops: Option<Box<SetElementOps>>,
    },
    /// `(e1, e2, ...)` (mirrors `HirExpr::TupleLiteral`, PR-11b Task 4). No
    /// `ty` field: `ty()` below derives `Ty::Tuple(Box::new(elements.iter()
    /// .map(MirExpr::ty).collect()))` positionally from every element (not
    /// just the first, unlike `ListLiteral`/`DictLiteral`/`SetLiteral` --
    /// heterogeneity means every position can differ). Empirically only
    /// `int`/`bool`/`float` elements ever reach this crate today
    /// (`pycc_types`' T0039 gate rejects every other element type at
    /// construction time), but deriving here keeps this lowering correct on
    /// its own terms rather than baking in an assumption this crate has no
    /// way to verify independently.
    TupleLiteral(Vec<MirExpr>),
    /// `base[start:stop:step]` (mirrors `HirExpr::Slice`, PR-12 Task 8,
    /// D-118). Each bound is independently optional, exactly like
    /// `HirExpr::Slice` itself. No `ty` field: `ty()` below returns
    /// `base.ty()` unchanged -- slicing a `list[int]` always produces
    /// another `list[int]`, unlike `Subscript`'s own element-type-narrowing
    /// `ty()` derivation above. This lowering is purely structural (recurse
    /// into `base` and every present bound, same as every other MIR node in
    /// this enum); it does not implement the runtime clamping/panic
    /// behavior D-118 requires (non-negative check, clamp into `[0, len]`,
    /// positive-step check) -- that is `pycc_codegen`'s job (a later task),
    /// operating on this already-lowered shape.
    Slice {
        base: Box<MirExpr>,
        start: Option<Box<MirExpr>>,
        stop: Option<Box<MirExpr>>,
        step: Option<Box<MirExpr>>,
    },
    /// `list.pop()` (mirrors `HirExpr::ListPop`, PR-12, D-119). `ty()` below
    /// returns the list's own element type -- empirically always `Ty::Int`
    /// (T0034 rejects every other `list[T]` before codegen), derived rather
    /// than hardcoded for the identical reason `ListLiteral`'s own `ty()`
    /// derives (D-105's own precedent).
    ListPop {
        list: MirContainerReceiver,
        ty: Ty,
    },
    /// `dict.get(key, default)` (mirrors `HirExpr::DictGetOrDefault`, PR-12,
    /// D-119). `ty()` below returns the dict's own value type, derived from
    /// the `dict` name's binding rather than hardcoded, for the same reason
    /// `ListPop` above derives its element type.
    DictGetOrDefault {
        dict: MirContainerReceiver,
        key: Box<MirExpr>,
        default: Box<MirExpr>,
        ty: Ty,
    },
    /// `set.add(value)` (mirrors `HirExpr::SetAdd`, PR-12, D-119). `.add()`
    /// always returns `None`, exactly like `ListAppend` -- a true invariant,
    /// not narrowed by any gate, hardcoded on purpose (mirrors `ListAppend`'s
    /// own `ty()` arm).
    ///
    /// #1343: `ops` is the set literal's own field, for the set's element.
    SetAdd {
        set: String,
        value: Box<MirExpr>,
        ops: Option<Box<SetElementOps>>,
    },
    /// `ClassName(args)` (D-154, Part 1 of #375): allocates a new instance
    /// with `slot_names.len()` slots (`pycc_codegen`'s job, via the class
    /// instance layout ADR's `pycc_rt_instance_new`), then calls `ctor`
    /// (the mangled `<ClassName>.__init__`) with the fresh instance
    /// pointer as `self`, followed by `args`. `HirExpr::Call` has no shape
    /// for "allocate, then call" -- unlike `MethodCall` (see this crate's
    /// own `lower_expr`, which lowers a method call directly into an
    /// ordinary `MirExpr::Call`, since a method call only ever *calls*, it
    /// never allocates), instantiation genuinely needs its own node.
    Instantiate(Box<InstantiateExpr>),
    /// `base.attr` (D-154, Part 1 of #375), resolved to a compile-time slot
    /// index by `lower_expr` against the class's `HirClassDef` -- never a
    /// runtime string-keyed lookup, per the class-instance-layout ADR.
    AttrGet {
        base: Box<MirExpr>,
        slot: usize,
        ty: Ty,
    },
    /// `base.attr` where `base` is a foreign CPython object (D-244, Part 2
    /// of #1026) -- a *string-keyed* attribute load performed at runtime by
    /// the `pycc_ext_obj_getattr` shim, in contrast to [`MirExpr::AttrGet`]
    /// directly above, whose `slot: usize` is resolved at compile time
    /// against a `HirClassDef`. A foreign object has no `HirClassDef` and
    /// therefore no slot to resolve, so the two shapes cannot share a node:
    /// the attribute name must survive lowering as a `String` (D-105's own
    /// String-keyed-node precedent).
    ///
    /// `ty` is always [`Ty::Object`] in Part 2 -- `pycc_types` knows nothing
    /// about a foreign attribute's type, so the result is opaque exactly
    /// like the base. The field is kept rather than hardcoded so a later
    /// part that learns an attribute's type (a typed stub, say) needs no
    /// shape change here.
    ///
    /// The load can fail: a missing attribute makes `PyObject_GetAttrString`
    /// return `NULL` with a CPython exception set, which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for this node.
    ObjAttrGet {
        base: Box<MirExpr>,
        attr: String,
        ty: Ty,
    },
    /// `base.method(args)` where `base` is a foreign CPython object (D-244,
    /// Part 2 of #1026, PR 2b of #1081) -- the *call* counterpart of
    /// [`MirExpr::ObjAttrGet`] directly above, and String-keyed for exactly
    /// the same reason: a foreign object has no `HirClassDef`, so `method`
    /// resolves to no mangled symbol and must survive lowering as a
    /// `String`. It is deliberately *not* lowered as an `ObjAttrGet` feeding
    /// a separate call node: the shim performs the attribute load and the
    /// vectorcall in one helper (`pycc_ext_obj_call`), so the bound method
    /// object never becomes a pycc value and the whole operation has one
    /// failure edge instead of two.
    ///
    /// `args` are already-checked packable operands (`pycc_types`'
    /// `HirExpr::MethodCall` arm admits `int`/`float`/`bool`/`str` and, since
    /// Part 2a of #1371, another object, and refuses everything else with
    /// `I0404`); codegen marshals each one into a new `PyObject *` reference
    /// before the call.
    ///
    /// The call can fail -- a missing method, or a method that raises --
    /// which is why `pycc_codegen::exception::expression_can_set_exception`
    /// answers `true` for this node.
    ///
    /// Unlike [`MirExpr::ObjAttrGet`] directly above, this variant carries
    /// **no `ty` field**: [`MirExpr::ty`] answers [`Ty::Object`] for it
    /// unconditionally. The reason is size rather than taste. The two would
    /// otherwise be symmetric -- both results are opaque for the same reason,
    /// and `ObjAttrGet` keeps the field so a later part that learns an
    /// attribute's real type needs no shape change -- but this variant is
    /// one `Vec` wider, which makes it the widest in the enum, and `MirExpr`
    /// is embedded twice over in `MirStmt`, which `MirItem::TopLevelStmt`
    /// holds by value. Carrying a field whose value is a constant costs 16
    /// bytes here and 16 bytes in `MirStmt`, which is enough to push
    /// `MirItem` past `clippy::large_enum_variant`'s threshold and fail the
    /// workspace lint gate. A later part that learns a foreign call's return
    /// type adds the field back and boxes something else in the same change.
    ObjMethodCall {
        base: Box<MirExpr>,
        method: String,
        args: Vec<MirExpr>,
    },
    /// `callee(args)` where `callee` is any name typed `Ty::Object` (#1313)
    /// -- `product("ab", "cd")` after `from itertools import product`, or a
    /// call of a `for` loop target bound to an object -- the direct-call
    /// sibling of [`MirExpr::ObjMethodCall`] directly above -- or, since
    /// Part 2a of #1371, a call of an `object`-typed subscript result
    /// (`callbacks[k](tok)`, from `pycc_hir`'s `HirExpr::ExprCall`).
    ///
    /// The callee's shape decides who owns it
    /// (`pycc_codegen::foreign_call::callee_is_produced`): a shim producer
    /// ([`MirExpr::ObjSubscript`], [`MirExpr::ObjAttrGet`],
    /// [`MirExpr::ObjMethodCall`] or another `ObjCall`) yields a new
    /// reference the consuming `pycc_ext_obj_call` releases; every other
    /// callee -- a [`MirExpr::Name`] read, which codegen loads as a *borrow*
    /// of a reference the caller keeps (a retained module global, or a `for`
    /// loop target's slot) -- goes to `pycc_ext_obj_call_borrowed`, which
    /// takes its own reference first.
    ///
    /// `args` are already-checked call arguments (scalars, `object`, or
    /// since #1435 an instance of a regular class) under the method call's
    /// rule
    /// (`pycc_types`' `check_object_call_args`). The call can fail -- the
    /// object is not callable, or the call raises -- which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for this node. Like `ObjMethodCall` it carries **no `ty`
    /// field**, for the same size reason; [`MirExpr::ty`] answers
    /// [`Ty::Object`] unconditionally.
    ObjCall {
        callee: Box<MirExpr>,
        args: Vec<MirExpr>,
    },
    /// Part 8 of #1371: a call of a CPython object that passes keyword
    /// arguments -- `o.method(x, key=v)`, `f(a, b=c)` with `f` a foreign
    /// function, `Cls(arg, flag=True)` with `Cls` a foreign class -- lowered
    /// from `pycc_hir`'s `HirExpr::KeywordCall` once `pycc_types` admitted
    /// its callee as an object. Boxed so the enum does not grow (see
    /// [`ObjKeywordCall`]); [`MirExpr::ty`] answers [`Ty::Object`] for it,
    /// as for the positional call it wraps.
    ObjKeywordCall(Box<ObjKeywordCall>),
    /// `len(base)` where `base` is a foreign CPython object (D-244, Part 3
    /// of #1026, PR 3a of #1082). The result is always [`Ty::Int`], so the
    /// variant carries no `ty` field -- the same size argument
    /// [`MirExpr::ObjMethodCall`] records above, for a value that is a true
    /// invariant rather than a narrowing.
    ///
    /// A dedicated node rather than the ordinary `len` `MirExpr::Call`
    /// lowering because the two produce different things: `list`/`dict`/`set`
    /// `len` yields a *raw* `i64` that codegen re-tags, while this one is
    /// answered by the shim's `pycc_ext_obj_len`, which hands back an
    /// already-D-141-encoded word through an out-parameter and can fail. The
    /// failure edge is what really separates them -- it needs the
    /// module-exec entry-point assertion that `foreign_attr.rs` and
    /// `foreign_len.rs` share, and which the scalar `len` path has no notion
    /// of.
    ///
    /// The call can fail: an operand with no `__len__` makes
    /// `PyObject_Size` raise, which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for this node.
    ObjLen {
        base: Box<MirExpr>,
    },
    /// `base[index]` where `base` is a foreign CPython object (D-244, Part 3
    /// of #1026, PR 3b of #1082) -- the subscript sibling of
    /// [`MirExpr::ObjAttrGet`], and a dedicated node rather than
    /// [`MirExpr::Subscript`] for the same reason [`MirExpr::ObjLen`] is not
    /// an ordinary `len` call: the two produce different things.
    /// `MirExpr::Subscript` indexes a pycc-owned `list`/`tuple` whose
    /// element type is known statically, while this one is answered by the
    /// shim's `pycc_ext_obj_getitem` and can fail. `MirExpr::Subscript`'s
    /// own `ty()` arm would panic on a `Ty::Object` base, which is exactly
    /// why the split happens during lowering rather than at codegen.
    ///
    /// `index` is an already-checked scalar (`pycc_types`' `HirExpr::Subscript`
    /// arm admits `int`/`float`/`bool`/`str` over a `Ty::Object` base and
    /// refuses everything else with `I0404`); codegen marshals it into a
    /// `PyObject *` through the same `pycc_ext_obj_pack_*` helper a method
    /// call's arguments use.
    ///
    /// Like [`MirExpr::ObjMethodCall`], this variant carries **no `ty`
    /// field**: [`MirExpr::ty`] answers [`Ty::Object`] for it
    /// unconditionally, because pycc knows nothing about what `o[k]` really
    /// is. The size argument that variant records applies here too.
    ///
    /// The load can fail -- a missing key, an unindexable operand, or a
    /// `__getitem__` that raises -- which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for this node.
    ///
    /// Only the *load* is modelled. A store (`o[k] = v`) is a separate HIR
    /// shape that `pycc_hir` still refuses with `C0001`, so no node for it
    /// exists.
    ObjSubscript {
        base: Box<MirExpr>,
        index: Box<MirExpr>,
    },
    /// `left op right` where at least one operand is a CPython object
    /// (Part 1 of #1371; `pycc_types::foreign::compare` owns the admitted
    /// shapes).
    ///
    /// `is`/`is not` is pointer identity: it cannot fail and answers
    /// [`Ty::Bool`]. An operand of an identity test may be
    /// [`MirExpr::NoneLiteral`], which codegen materializes as CPython's
    /// `None` singleton. Every other operator is CPython's
    /// `PyObject_RichCompare`, whose result is an arbitrary object --
    /// [`MirExpr::ty`] answers [`Ty::Object`] -- and which can raise, so
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for it. A scalar operand is boxed through the
    /// `pycc_ext_obj_pack_*` helpers.
    ObjCompare {
        op: pycc_hir::CmpOpKind,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
    },
    /// `item in container` (`negate: false`) or `item not in container`
    /// (`negate: true`) where `container` is a CPython object (Part 2b of
    /// #1371; `pycc_types::foreign::compare::membership_ty` owns the
    /// admitted shapes). CPython's `PySequence_Contains` -- which tries
    /// `__contains__`, then iteration, then the old `__getitem__` protocol --
    /// answers a C truth value, so [`MirExpr::ty`] answers [`Ty::Bool`]. It
    /// can raise (a raising `__contains__`, or a container that supports
    /// none of the three protocols), which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for this node. `item` is boxed through the
    /// `pycc_ext_obj_pack_*` helpers; `item` is evaluated before
    /// `container`, CPython's own order.
    ObjContains {
        negate: bool,
        item: Box<MirExpr>,
        container: Box<MirExpr>,
    },
    /// `type(base)` where `base` is a CPython object (Part 11 of #1371):
    /// CPython's `PyObject_Type`, a new reference to the operand's class,
    /// answered by the shim's `pycc_ext_obj_type`. A node of its own rather
    /// than a `Call { callee: "type" }`, which has no `$fn:type` signature
    /// to lower against; [`MirExpr::ty`] answers [`Ty::Object`]. The helper
    /// answers `NULL` only for a `NULL` operand (its defence-in-depth
    /// guard), and `pycc_codegen::exception::expression_can_set_exception`
    /// answers `true` for this node so that edge is honoured.
    ObjType {
        base: Box<MirExpr>,
    },
    /// `base[start:stop:step]` where `base` is a CPython object (Part 2b of
    /// #1371): CPython's `PyObject_GetItem` with a `slice` built by
    /// `PySlice_New`, an absent bound passed as `None`. A node of its own
    /// rather than a [`MirExpr::Slice`], whose `ty()` answers its base's
    /// type and whose codegen arm handles only native sequences;
    /// [`MirExpr::ty`] answers [`Ty::Object`]. It can raise (an unsliceable
    /// object, or a raising `__getitem__`), so
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for it. The deletion `del o[a:b]` is the statement
    /// [`MirStmt::ObjDelSlice`]; the store `o[a:b] = v` is still refused
    /// before MIR.
    ObjSlice {
        base: Box<MirExpr>,
        start: Option<Box<MirExpr>>,
        stop: Option<Box<MirExpr>>,
        step: Option<Box<MirExpr>>,
    },
    /// A list display `[e1, e2, ...]` bound to a CPython object slot (Part
    /// 2d of #1371, D-258 rule 4): a fresh CPython `list` built by
    /// `PyList_New`, each element boxed through a `pycc_ext_obj_pack_*`
    /// helper and evaluated left to right. A node of its own rather than a
    /// [`MirExpr::ListLiteral`], whose `ty()` answers a native `list[T]`;
    /// [`MirExpr::ty`] answers [`Ty::Object`]. It can raise (a failed
    /// element box or allocation), so
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for it.
    ObjList {
        elements: Vec<MirExpr>,
    },
    /// `isinstance(value, class)` where `value` is a CPython object (Part 1
    /// of #1371): a run-time `PyObject_IsInstance`, which can raise (a
    /// class argument that is not a class, or a raising
    /// `__instancecheck__`). [`MirExpr::ty`] answers [`Ty::Bool`].
    ObjIsInstance {
        value: Box<MirExpr>,
        class: ObjIsInstanceClass,
    },
    /// The value of a tuple-unpacking assignment `t1, ..., tn = value`
    /// whose `value` is a CPython object (Part 1 of #891): CPython's own
    /// unpack protocol -- iterate `value`, take exactly `arity` items --
    /// collecting the items into a fresh `tuple`. [`MirExpr::ty`] answers
    /// [`Ty::Object`]; each target is then bound by an ordinary
    /// [`MirExpr::ObjSubscript`] of that tuple. It can raise (a
    /// non-iterable, a wrong item count, a raising `__iter__`/`__next__`),
    /// so `pycc_codegen::exception::expression_can_set_exception` answers
    /// `true` for it. A native tuple value never reaches this node: its
    /// arity is checked statically, and the value passes through unchanged.
    ObjUnpack {
        value: Box<MirExpr>,
        arity: usize,
    },
    /// `b[i]` where `b` is a `pycc build --ext` export's `memoryview`
    /// parameter (Part 2 of #1027): a bounds-checked native `float` element
    /// load out of the borrowed buffer.
    ///
    /// A node of its own rather than a [`MirExpr::Subscript`] with a
    /// `Ty::MemoryView` base, because `Subscript::ty()` reads the *base's*
    /// type to answer -- it destructures `Ty::List`/`Ty::Tuple` and panics on
    /// anything else -- and a buffer element's type is not recoverable that
    /// way: `Ty::MemoryView` carries no element type, the element type is
    /// pinned to `float` by the unpack shim's `"d"` format requirement.
    /// [`MirExpr::ty`] therefore answers [`Ty::Float`] here unconditionally,
    /// exactly as [`MirExpr::ObjLen`] answers `Ty::Int`.
    ///
    /// The load can fail -- an index outside `[0, len)` -- which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers `true`
    /// for this node. `pycc_rt`'s `pycc_rt_buffer_f64_get` owns the bounds
    /// check and the D-173 raise; codegen emits one call and no arithmetic.
    ///
    /// Only the *load* is modelled. A store (`b[i] = v`) is a separate HIR
    /// shape the type checker still refuses, so no node for it exists.
    BufferGet {
        base: Box<MirExpr>,
        index: Box<MirExpr>,
    },
    /// `len(b)` where `b` is a `pycc build --ext` export's `memoryview`
    /// parameter (#1116): the element count the wrapper copied out of the
    /// exporter's `shape[0]`, in elements and never in bytes.
    ///
    /// A node of its own rather than the ordinary scalar `len` lowering
    /// (which stays a `MirExpr::Call`) for [`MirExpr::BufferGet`]'s reason:
    /// the operand is not a pycc-owned `list`/`dict`/`set` pointer, so the
    /// runtime entry point and the pointer's meaning both differ. It carries
    /// no `ty` field because the result is always [`Ty::Int`], exactly as
    /// [`MirExpr::ObjLen`] does.
    ///
    /// Unlike both its siblings, this one *cannot* fail: there is no index
    /// to range-check and `pycc_rt_buffer_len` never raises, which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers
    /// `false` for this node where it answers `true` for `ObjLen` and
    /// `BufferGet`.
    BufferLen {
        base: Box<MirExpr>,
    },
    /// `ndarray(n)` / `NDArray(n)` (Part 2a of #1142, issue #1165): the
    /// allocation of `n` zero-filled contiguous `f64`s that the artifact
    /// itself owns, together with the [`PyccExtBufferView`-shaped] pair that
    /// addresses them.
    ///
    /// Deliberately **not** a [`MirExpr::Call`]. Routing the producer through
    /// the ordinary call path would make its result an ordinary call result,
    /// which is what takes a `Ty::MemoryView` into
    /// `pycc_codegen::call_result`'s `Ty::MemoryView` panic. A node of its
    /// own keeps that panic unreachable by construction, exactly as
    /// [`MirExpr::BufferGet`] and [`MirExpr::BufferLen`] keep the scalar
    /// `len` lowering away from `expect_list_pointer`.
    ///
    /// [`MirExpr::ty`] answers [`Ty::MemoryView`] here unconditionally: the
    /// producer's element type and dimensionality are fixed by the same
    /// one-dimensional `"d"` contract the `--ext` unpack shim requires, so
    /// there is nothing per-node to carry.
    ///
    /// The allocation can fail -- a negative `len` raises `ValueError`
    /// (D-173) and yields a null view -- which is why
    /// `pycc_codegen::exception::expression_can_set_exception` answers `true`
    /// for this node, on [`MirExpr::BufferGet`]'s precedent rather than
    /// [`MirExpr::BufferLen`]'s: it returns a value and lets the consumer
    /// guard.
    ///
    /// [`PyccExtBufferView`-shaped]: MirExpr::BufferGet
    BufferAlloc {
        len: Box<MirExpr>,
    },
    /// `frozenset()` / `frozenset(x)` (Part 1 of #1319): a fresh
    /// frozenset built by the `frozenset` builtin, split off
    /// `Call { callee: "frozenset" }` in `expr.rs` under the user-shadow
    /// guard, exactly as `ObjLen`/`BufferLen` split off `len`.
    ///
    /// `source` is `None` for the empty call, otherwise the argument, whose
    /// own type (`set[int]`, `frozenset[int]`, `list[int]`, or since #1343 a
    /// `set`/`frozenset` of a user-class instance -- the only shapes
    /// `pycc_types` admits) selects the runtime copy constructor in
    /// codegen. [`MirExpr::ty`] answers the source's own element type (a
    /// `set`/`frozenset` of a user-class instance since #1343), and
    /// `frozenset[int]` for the empty call and a `list[int]` source.
    ///
    /// A dedicated node rather than a [`MirExpr::Call`], because no user
    /// signature backs the call: the ordinary call path would look up a
    /// `$fn:frozenset` that does not exist.
    FrozenSetFrom {
        source: Option<Box<MirExpr>>,
    },
    /// `hash(x)` of a user-class instance (#1335, Part 1 of #1332), split
    /// off `Call { callee: "hash" }` in `expr.rs` under the same user-shadow
    /// guard as `FrozenSetFrom`. `pycc_hir::resolve_instance_hash` chose
    /// `via` from the class table, which codegen does not see.
    ///
    /// For [`InstanceHashVia::Identity`], `operand` is the instance
    /// expression; for [`InstanceHashVia::Method`], it is an ordinary
    /// [`MirExpr::Call`] of the mangled `__hash__` with the instance, the
    /// shape `r.__hash__()` already lowers to, so the call keeps its own
    /// exception guard and argument ownership. [`MirExpr::ty`] answers `int`.
    InstanceHash {
        operand: Box<MirExpr>,
        via: InstanceHashVia,
    },
    /// `x: tuple[float, ..., float] = <object>` at module scope (D-244, Part
    /// 4 of #1026, PR 4c of #1083): the unpack of a foreign CPython object
    /// into a fixed-arity all-`float` tuple.
    ///
    /// A dedicated node rather than a coercion bolted onto the `AnnAssign`
    /// lowering's existing widening chain, for [`MirExpr::ObjLen`]'s reason:
    /// this one *can fail* -- a non-tuple, a wrong arity, or an item
    /// `PyNumber_Float` refuses -- so it needs the module body's foreign
    /// failure edge
    /// that `IntBoundary` and `OptionalWrap` have no notion of.
    ///
    /// `arity` is carried rather than rediscovered, and is always at least
    /// one: `pycc_types`' module-level `AnnAssign` arm admits this shape
    /// only for a non-empty tuple annotation whose every element is
    /// `Ty::Float`, which is also why [`MirExpr::ty`] can rebuild the
    /// annotation from the arity alone and this variant needs no `ty`
    /// field.
    ///
    /// Only a *module-level* annotated assignment produces it. The same
    /// source inside a function body is `I0404` at the read of the foreign
    /// name itself, long before this lowering runs.
    ObjUnpackFloatTuple {
        base: Box<MirExpr>,
        arity: usize,
    },
    /// #436: A null instance pointer used as the `cls` argument when a
    /// `@classmethod` is called on a class name (`ClassName.method(args)`)
    /// rather than an instance. In this compiler's static-dispatch model,
    /// `cls` is typed as `Ty::Instance(class_name)` but is not meaningfully
    /// used in the method body -- the method was compiled for a specific
    /// class, so any `cls.method()` or `cls.attr` resolves at compile time
    /// to that class's methods/attributes. The null pointer is never
    /// dereferenced at runtime (the method body does not access `cls`'s
    /// slots in practice), so this is safe.
    NullInstance {
        ty: Ty,
    },
    /// Reads a caught exception's own message string (Part 3A of #541,
    /// #736): `str(e)` semantics -- the message alone, never
    /// `exception_print_and_exit`'s own uncaught-exception `"{type}:
    /// {message}"` format, which this node has nothing to do with. The
    /// boxed inner expression is the exception-typed value (typically a
    /// `MirExpr::Name` reference to an `except ... as e:` handler binding);
    /// `pycc_mir::class::rewrite_exception_to_message` is the sole
    /// constructor, applied at the same two lowering sites
    /// `rewrite_instance_to_repr` is (a `print` argument and an f-string
    /// interpolation) whenever the expression's static type names a
    /// registered exception class, so codegen's `to_str` always receives a
    /// `str` scalar for a caught exception rather than reaching its
    /// `Scalar::Instance` panic arm. `.ty()` below always reports
    /// `Ty::Str`, exactly like `rewrite_instance_to_repr`'s own
    /// `MirExpr::Call` rewrite.
    ExceptionMessage(Box<MirExpr>),
    /// `isinstance(obj, T)` decided at run time from a caught exception's type
    /// tag (#1337, WI-6a): `obj`'s static type is a seeded builtin exception
    /// class, so its runtime value is a `PyExceptionObj` whose dynamic class
    /// may be any subclass. The result is whether `pycc_rt_exception_type_matches`
    /// accepts the object for any tag in `tags` -- the target classes' own
    /// tags plus every raisable class whose MRO reaches one of them, sorted
    /// and non-empty. `class::lower_isinstance` is the sole constructor,
    /// using `pycc_mir::exception_isinstance`'s helpers. `.ty()` is
    /// `Ty::Bool`.
    ExceptionTypeTest {
        obj: Box<MirExpr>,
        tags: Vec<u8>,
    },
    /// PEP 572 (#774): `target := value`. Evaluates `value`, stores it into
    /// `name`'s already-predeclared storage slot (see
    /// `pycc_codegen::collect_expr_bindings`, this node's own slot-scanning
    /// counterpart to `collect_stmt_bindings`'s `MirStmt::Assign` handling),
    /// and yields that same value as the expression's own result -- the
    /// "evaluate, store, yield" shape the issue's own design calls for,
    /// distinct from every other `MirExpr` variant (all of which are pure
    /// reads/computations with no store side effect). `name` is restricted
    /// (at the `pycc_hir` boundary, before this node is ever constructed) to
    /// exactly three statement placements -- an `if`/`while` test condition
    /// or a bare expression statement, at any expression nesting depth
    /// within those -- so `pycc_types::collect_named_expr_bindings` and this
    /// crate's own `pycc_mir::stmt::collect_named_expr_bindings` only ever
    /// need to hoist-and-bind at those three statement sites. `ty` is
    /// `value`'s own type, exactly mirroring an assignment statement's RHS
    /// typing rule (see `pycc_types::expr::infer_expr_in`'s own `NamedExpr`
    /// arm).
    NamedExpr {
        name: String,
        value: Box<MirExpr>,
        ty: Ty,
    },
    /// A comprehension in expression position (#1254, D-250), mirroring
    /// `pycc_hir::HirExpr::Comprehension`. Boxed for the same
    /// `large_enum_variant` reason as [`InstantiateExpr`].
    Comprehension(Box<MirComprehension>),
    /// Evaluate `discard` for its effects, retire its value, then evaluate
    /// and yield `value` (#1346). This is CPython's receiver-then-call order
    /// for `recv.attr` / `recv.attr(args)` when the attribute does not
    /// consume its receiver: a `@staticmethod` or a
    /// `staticmethod(<foreign callable>)` class attribute reached through an
    /// instance. `pycc_mir::expr`'s `sequence_after` builds it only for a
    /// receiver that is not a plain `Name` read, whose evaluation has no
    /// effect to preserve.
    Sequence {
        discard: Box<MirExpr>,
        value: Box<MirExpr>,
    },
}

/// [`MirExpr::Comprehension`]'s payload (#1254, D-250). The fields mirror
/// `MirStmt::ListCompAssign` and its siblings minus `target`: `var` is the
/// D-117 synthesized loop-variable name, scoped to this node -- codegen
/// gives it a slot of its own, and no enclosing-scope binding exists for
/// it -- and `var_ty` its resolved type.
#[derive(Debug, Clone, PartialEq)]
pub struct MirComprehension {
    pub var: String,
    pub var_ty: Ty,
    pub source: CompSource,
    pub cond: Option<MirExpr>,
    pub elt: MirCompElt,
}

/// A lowered comprehension's element expressions, by kind (#1254).
#[derive(Debug, Clone, PartialEq)]
pub enum MirCompElt {
    List(MirExpr),
    /// The element and, as for [`MirExpr::SetLiteral`], its
    /// [`SetElementOps`]: `None` for an `int` element, the instance class's
    /// hash and equality for a `set[C]` (#1344, Part 2 of #1336).
    Set(MirExpr, Option<Box<SetElementOps>>),
    Dict {
        key: MirExpr,
        value: MirExpr,
    },
}

impl MirComprehension {
    /// The produced container type, derived from the element types exactly
    /// as the statement form's binding of `target` is. A comprehension over
    /// a CPython object ([`CompSource::Object`], Part 1 of #1255) produces a
    /// CPython `list` or `set`, which is the opaque [`Ty::Object`] whatever
    /// its elements are.
    pub fn ty(&self) -> Ty {
        if let CompSource::Object(_) = self.source {
            return Ty::Object;
        }
        match &self.elt {
            MirCompElt::List(elt) => Ty::List(Box::new(elt.ty())),
            MirCompElt::Set(elt, _) => Ty::Set(Box::new(elt.ty())),
            MirCompElt::Dict { key, value } => Ty::Dict(Box::new((key.ty(), value.ty()))),
        }
    }
}

/// How a [`MirExpr::InstanceHash`] hashes its instance (#1335).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceHashVia {
    /// `object.__hash__`: the operand is the instance, hashed by address.
    Identity,
    /// A user `__hash__`: the operand is the call to it, whose `int` or
    /// `bool` result becomes the hash as CPython's `slot_tp_hash` does.
    Method,
}

/// The payload of [`MirExpr::ObjKeywordCall`] (Part 8 of #1371).
///
/// `call` is the positional half of the call, exactly the node the same
/// call without its keywords lowers to: a [`MirExpr::ObjMethodCall`] or a
/// [`MirExpr::ObjCall`]. `names` and `values` are the keyword arguments in
/// source order, one name per value; `pycc_parser` already refuses a
/// repeated name (`L0001`), so the names are distinct. Codegen evaluates the
/// callee (and, for a method call, looks the method up), then the
/// positional arguments, then the keyword values, which is CPython's order,
/// and marshals the values after the positional arguments in one vectorcall
/// argument array with the names as its `kwnames` tuple. The values follow
/// the positional argument rule (`pycc_types`' `check_object_call_args`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjKeywordCall {
    pub call: MirExpr,
    pub names: Vec<String>,
    pub values: Vec<MirExpr>,
}

/// `MirExpr::Instantiate`'s payload, boxed (not inlined into that variant
/// directly) to keep `MirExpr`'s own size close to its other variants --
/// `ctor: String` + the slot count + `args: Vec<MirExpr>` + `ty: Ty`
/// inlined directly measured large enough to trip clippy's
/// `large_enum_variant` lint (`-D warnings`) on `MirItem` (whose
/// `TopLevelStmt(MirStmt)` variant embeds `MirExpr` several layers deep,
/// e.g. via `MirStmt::AttrSet`'s two raw `MirExpr` fields and
/// `CompSource::Range`'s three), the same reasoning `Ty::Tuple(Box<Vec<Ty>>)`
/// and `Ty::Dict(Box<(Ty, Ty)>)` already apply one crate over.
#[derive(Debug, Clone, PartialEq)]
pub struct InstantiateExpr {
    pub ctor: String,
    /// The class's source name -- a monomorphised generic class's original
    /// name, not its `0gen_` mangling -- which `pycc_rt` uses to word the
    /// `AttributeError` of a slot read before its assignment (#1388).
    pub class_name: String,
    /// The name of every slot of the class's flat MRO layout, in slot
    /// order (`class::mro_attrs`). Its length is the slot count the
    /// instance is allocated with (#432); the names word the same
    /// `AttributeError` (#1388).
    pub slot_names: Vec<String>,
    pub args: Vec<MirExpr>,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirFStringPart {
    Literal(String),
    Interpolation(Box<MirExpr>),
}

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
            MirExpr::Name { ty, .. }
            | MirExpr::Call { ty, .. }
            | MirExpr::BinOp { ty, .. }
            | MirExpr::Compare { ty, .. }
            | MirExpr::BoolOp { ty, .. }
            | MirExpr::IfExp { ty, .. } => ty.clone(),
            MirExpr::Not(_) | MirExpr::CompareChain { .. } => Ty::Bool,
            MirExpr::EmptyList(element) => Ty::List(Box::new(element.clone())),
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
                    let MirExpr::IntLiteral(literal_index) = index.as_ref() else {
                        panic!(
                            "pycc_mir: internal error: tuple subscript index is not a literal int -- pycc_types::check should have rejected this HIR before it reached pycc_mir (T0040)"
                        )
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
            MirExpr::ListAppend { .. } => Ty::None,
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
            // Part 2b of #1371: CPython's own slice result, opaque.
            // Part 11 of #1371: `type(o)` is CPython's own class object.
            MirExpr::ObjSlice { .. } | MirExpr::ObjList { .. } | MirExpr::ObjType { .. } => {
                Ty::Object
            }
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
                Some(Ty::Set(elem) | Ty::FrozenSet(elem)) => Ty::FrozenSet(elem),
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
            | MirExpr::EmptyDict(_)
            | MirExpr::NoneLiteral
            | MirExpr::NotImplemented
            | MirExpr::Name { .. }
            | MirExpr::NullInstance { .. } => {}
            MirExpr::ListPop { list, .. } => {
                if let Some(receiver) = list.attr_expr() {
                    receiver.collect_named_expr_bindings(out);
                }
            }
            MirExpr::IntBoundary(inner) => inner.collect_named_expr_bindings(out),
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
            MirExpr::OptionalUnwrap(inner, _) | MirExpr::ObjectUnbox(inner, _) => {
                inner.collect_named_expr_bindings(out)
            }
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
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirStmt {
    ExprStmt(MirExpr),
    Assign {
        target: String,
        value: MirExpr,
    },
    /// A statement with zero runtime effect -- currently only produced by a
    /// value-less PEP 526 annotation (`x: int`), which CPython itself does
    /// nothing observable for either (confirmed empirically during PR-9
    /// planning: no store, no allocation, nothing an oracle diff could see).
    NoOp,
    /// A continuation that the validated HIR proves cannot be reached. The
    /// match lowerer uses this for the no-case fallback of an exhaustive
    /// `match`: preserving the proof in MIR keeps later fallthrough analysis
    /// aligned with the type checker instead of inventing a synthetic `NoOp`
    /// path after all exhaustive alternatives failed.
    Unreachable,
    If {
        test: MirExpr,
        body: Vec<MirStmt>,
        orelse: Vec<MirStmt>,
    },
    While {
        test: MirExpr,
        body: Vec<MirStmt>,
    },
    ForRange {
        var: String,
        start: MirExpr,
        stop: MirExpr,
        step: MirExpr,
        body: Vec<MirStmt>,
    },
    /// `for var in list:` (mirrors `HirStmt::ForList`, D-105). `list` is
    /// carried as the plain variable name, exactly like `HirStmt::ForList`
    /// itself; there is no start/stop/step to lower here, only `body`.
    ForList {
        var: String,
        list: String,
        body: Vec<MirStmt>,
    },
    /// `for var in <object expression>:` (mirrors `HirStmt::ForObject`,
    /// PR 3c of #1082). Unlike every other `For*` node here the iterable is
    /// a lowered expression rather than a variable name, because the two
    /// admitted source shapes (`o.attr`, `o.method(...)`) are expressions.
    /// `var` holds each item as an opaque `PyObject *` (`Ty::Object`), and
    /// `pycc_codegen` drives the loop through `PyObject_GetIter` /
    /// `PyIter_Next` behind the D-244 shim.
    ForObject {
        var: String,
        iter: MirExpr,
        body: Vec<MirStmt>,
    },
    /// `d[k] = v` (mirrors `HirStmt::DictSet`, PR-11 Task 4/D-123). `dict` is
    /// carried as the plain variable name, exactly like `ForList`'s `list`
    /// field -- there is no sub-expression to
    /// recursively lower for it, only for `key`/`value`.
    DictSet {
        dict: String,
        key: MirExpr,
        value: MirExpr,
    },
    /// `b[i] = v` on a `memoryview`-typed base (Part 1 of #1142) -- the
    /// store counterpart of [`MirExpr::BufferGet`], and a **statement**
    /// rather than an expression because `pycc_hir` lowers every
    /// `<name>[k] = v` to `HirStmt::DictSet` and never to a `Subscript`
    /// store. Being a statement is load-bearing rather than incidental:
    /// `pycc_codegen`'s `expression_can_set_exception` is a `&MirExpr`
    /// predicate, so the `IndexError` this node can leave pending is
    /// observed by `guard_statement_effects` at the statement level and
    /// by nothing else.
    ///
    /// Unlike [`MirStmt::DictSet`] immediately above, `base` is a lowered
    /// `MirExpr` and not a bare name: codegen then reuses
    /// [`MirExpr::BufferGet`]'s own `Scalar::MemoryView` extraction rather
    /// than the dict-specific by-name read. `pycc_types` admits the store
    /// only on a bare `HirExpr::Name` base, so in practice this is always
    /// a [`MirExpr::Name`] -- carrying it as an expression is what keeps
    /// the load and the store on one codegen path.
    BufferSet {
        base: MirExpr,
        index: MirExpr,
        value: MirExpr,
    },
    /// `for var in dict:` (mirrors `HirStmt::ForList` on a dict-typed base --
    /// see `lower_stmt`'s own `HirStmt::ForList` arm for why a dict-typed
    /// base is lowered to this node instead of `MirStmt::ForList`). `dict` is
    /// carried as the plain variable name, exactly like `ForList`'s `list`
    /// field.
    ForDict {
        var: String,
        dict: String,
        body: Vec<MirStmt>,
    },
    /// `for var in set:` (mirrors `HirStmt::ForList` on a set-typed base --
    /// see `lower_stmt`'s own `HirStmt::ForList` arm for why a set-typed
    /// base is lowered to this node instead of `MirStmt::ForList`, PR-11
    /// Task 8, D-123). `set` is carried as the plain variable name, exactly
    /// like `ForList`'s `list` field and `ForDict`'s `dict` field.
    ///
    /// #1343: `var_ty` is the set's element type, `int` or a user-class
    /// instance, which codegen binds `var` as.
    ForSet {
        var: String,
        var_ty: Ty,
        set: String,
        body: Vec<MirStmt>,
    },
    /// `target = [elt for var in <source> [if cond]]`, already fully
    /// resolved (mirrors `HirStmt::ListCompAssign`, PR-12, D-117). `var_ty`
    /// is the loop variable's own resolved type (`Ty::Int` for a `Range`
    /// source; the iterated container's element/key type for a `List`/
    /// `Dict`/`Set` source) -- carried explicitly, rather than requiring
    /// `pycc_codegen` to re-derive it from `elt`, because `elt` need not
    /// contain any `Name` reference to `var` at all in general (though in
    /// practice it usually does) and re-deriving it by walking `elt` would
    /// be a second, independent computation of a fact `resolve_comp_source`
    /// (below) already has in hand once.
    ListCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        elt: Box<MirExpr>,
    },
    /// `target = {key: value for var in <source> [if cond]}`, already fully
    /// resolved (mirrors `HirStmt::DictCompAssign`, PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly except for the key/value split.
    DictCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        key: Box<MirExpr>,
        value: Box<MirExpr>,
    },
    /// `target = {elt for var in <source> [if cond]}`, already fully
    /// resolved (mirrors `HirStmt::SetCompAssign`, PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly -- a set comprehension's own shape is
    /// identical to a list comprehension's, differing only in which
    /// runtime constructor/insert pair `pycc_codegen` ends up calling.
    ///
    /// #1344 (Part 2 of #1336): `ops` is `None` for an `int` element and
    /// carries the instance class's hash and equality for a `set[C]`, as
    /// [`MirExpr::SetLiteral`]'s own `ops` does.
    SetCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        elt: Box<MirExpr>,
        ops: Option<Box<SetElementOps>>,
    },
    Return(Option<MirExpr>),
    /// `return b[start:stop]` where `b` is a `memoryview` **parameter**
    /// (Part 2 of #1175, #1179) -- the one buffer sub-range egress
    /// `pycc_types::buffer::admitted_buffer_return` admits.
    ///
    /// A node of its own rather than `Return(Some(MirExpr::Slice { .. }))`,
    /// and the reason is a miscompile rather than taste:
    /// `MirExpr::Slice::ty()` passes its base's type straight through, so a
    /// buffer base would answer `Ty::MemoryView` and reach
    /// `pycc_codegen`'s list-only `MirExpr::Slice` emit arm, which calls
    /// `pycc_rt_int_list_slice` -- a buffer compiled as a list. Keeping the
    /// admitted shape out of `MirExpr::Slice` entirely makes that
    /// unreachable by construction instead of by a comment, and leaves
    /// every grouped `MirExpr` catch-all that lists `MirExpr::Slice { .. }`
    /// wholesale (`bigint_rc.rs`, `exception.rs`) answering for the list
    /// slice only, which is the answer they were written for.
    ///
    /// `name` is the parameter, which the compiled body still returns *as a
    /// whole view*: the wrapper's `result == &a{index}` identity test is
    /// exact and stays exact. The bounds travel out of band through the
    /// export thunk's trailing out-pointers. An absent `start` means `0`
    /// and an absent `stop` means `i64::MAX`; both defaults are CPython's
    /// own clamping, measured through `PySlice_New`/`PyObject_GetItem`.
    /// A `step` never reaches here -- `buffer_slice_step_unsupported`
    /// refuses it in both type walkers.
    ReturnBufferSlice {
        name: String,
        start: Option<MirExpr>,
        stop: Option<MirExpr>,
    },
    /// `base.attr = value` (D-154, Part 1 of #375), resolved to a
    /// compile-time slot index by `lower_stmt` against the class's
    /// `HirClassDef` -- mirrors `MirExpr::AttrGet`'s own resolution
    /// exactly.
    AttrSet {
        base: MirExpr,
        slot: usize,
        value: MirExpr,
    },
    /// `del base[start:stop:step]` where `base` is a CPython object (Part 2c
    /// of #1371, from `HirStmt::DeleteSlice`): CPython's `PyObject_DelItem`
    /// with the same `slice` key [`MirExpr::ObjSlice`] builds, an absent
    /// bound passed as `None`. A statement because it yields nothing. Its
    /// failure (an object without `__delitem__`, or a raising one) is
    /// routed by codegen's foreign-failure edge inside the statement, as
    /// [`MirStmt::ForObject`]'s `iter()` is.
    ObjDelSlice {
        base: MirExpr,
        start: Option<MirExpr>,
        stop: Option<MirExpr>,
        step: Option<MirExpr>,
    },
    /// `base.attr = value` where `base` is a CPython object (Part 2 of
    /// #1443, #1457, from a `HirStmt::AttrSet` whose base types as
    /// `Ty::Object`): CPython's `PyObject_SetAttr`, String-keyed like
    /// [`MirExpr::ObjAttrGet`] because a foreign object has no slot table.
    /// Codegen evaluates `value` before `base`, CPython's own order for an
    /// attribute assignment, and packs it into a new reference the shim
    /// helper consumes. Its failure (a read-only or `__slots__`-refused
    /// attribute, a raising `__setattr__`) is routed by codegen's
    /// foreign-failure edge inside the statement, as
    /// [`MirStmt::ObjDelSlice`]'s is.
    ObjAttrSet {
        base: MirExpr,
        attr: String,
        value: MirExpr,
    },
    /// `del base.attr` where `base` is a CPython object (#1457, from
    /// `HirStmt::DeleteAttr`): CPython's `PyObject_DelAttr`. A failure (a
    /// missing attribute, a raising `__delattr__`) takes the
    /// foreign-failure edge, as [`MirStmt::ObjAttrSet`]'s does.
    ObjDelAttr {
        base: MirExpr,
        attr: String,
    },
    /// PEP 634-636 (#381, PR-21): A sequence of statements executed in
    /// order — used by `match` lowering to pair the subject-temporary
    /// assignment with the nested `if` chain.
    Seq(Vec<MirStmt>),
    /// `try`/`except`/`else`/`finally` (PEP 3110, #382, PR-22 Part 1).
    /// Each handler's `exc_type_tag` is the sorted set of runtime exception
    /// type tags it accepts — the named class's own tag (matching `pycc_rt`'s
    /// `EXCEPTION_TYPE_*` constants for the seven builtins, or the tag HIR
    /// lowering assigned the class for a user-defined one) plus every
    /// raisable subclass's tag — or `None` for a bare `except:`.
    Try {
        body: Vec<MirStmt>,
        handlers: Vec<MirExceptHandler>,
        orelse: Vec<MirStmt>,
        finalbody: Vec<MirStmt>,
    },
    /// `try: ... except* T: ...` (PEP 654, Part 3 of #382, #542). Shares
    /// `Try`'s shape exactly -- `MirExceptHandler`'s `exc_type_tag` still
    /// carries the matched-type tag set (always `Some`, since `except*`
    /// requires a named type; see `pycc_hir::stmt`'s comment on the
    /// matching lowering site) -- but codegen dispatches each handler with
    /// a partition (`pycc_rt_exception_group_partition`) against the raised
    /// group rather than `Try`'s mutually-exclusive `type_matches` checks,
    /// and every matched subgroup's handler can run (PEP 654 semantics: more
    /// than one `except*` clause may fire for the same raised group), unlike
    /// `Try`'s first-match-wins dispatch.
    TryStar {
        body: Vec<MirStmt>,
        handlers: Vec<MirExceptHandler>,
        orelse: Vec<MirStmt>,
        finalbody: Vec<MirStmt>,
    },
    /// `raise <exception>` (PEP 3110, #382).
    Raise {
        exception: MirExceptionValue,
        /// The enclosing function's plain Python source name (`"<module>"`
        /// at top level, e.g. `create` rather than `pycc_hir`'s mangled
        /// `C.create.classmethod` for a classmethod -- see
        /// [`source_frame_name`]), identifying where this `raise` executes
        /// for traceback rendering (#707), matching what CPython's own
        /// traceback shows for the frame. Populated by
        /// [`set_frame_function`] as a post-pass over the body
        /// [`lower_item`] just finished lowering, rather than threaded as a
        /// parameter through `lower_stmt`'s whole recursive descent: the
        /// value is constant for an entire function body, and every
        /// statement-lowering function between `lower_item` and
        /// `lower_raise` would otherwise need a new parameter purely to pass
        /// it through unchanged. `lower_raise` itself always leaves this
        /// empty; only the post-pass fills it in.
        frame_function: String,
    },
    /// `raise <exception> from <cause>` (PEP 409, #382).
    RaiseFrom {
        exception: MirExceptionValue,
        cause: MirExceptionValue,
        /// See [`MirStmt::Raise::frame_function`] -- identifies where this
        /// `raise ... from ...` executes. Only `exception` (the raised
        /// effect) carries a frame; `cause` keeps whatever frame it already
        /// had (empty, if freshly constructed, since this compiler does not
        /// track cause objects raised elsewhere), matching CPython's
        /// distinction between an exception's own traceback and an
        /// explicitly chained cause's.
        frame_function: String,
    },
    /// Bare `raise` (re-raise, #382). Only valid inside an except handler.
    Reraise,
    /// `raise value` where `value` is a CPython object (Part 9 of #1371,
    /// from a cause-less `HirStmt::Raise` whose operand is `Ty::Object`).
    /// CPython decides what is raised, as its own `raise` does: an
    /// exception instance is raised, an exception class is instantiated
    /// with no arguments, and anything else raises `TypeError`. Codegen
    /// hands the CPython exception to the foreign-operation bridge, so it
    /// propagates as a pending pycc exception whose original the host sees.
    /// Carries no frame name: the bridged original keeps CPython's own
    /// traceback, unlike [`MirStmt::Raise`]'s pycc-rendered one.
    ObjRaise {
        value: MirExpr,
    },
    /// A foreign (CPython-object) import nested in a module-level `if`/`try`
    /// block (#1291, #1383), the statement counterpart of
    /// [`MirItem::ForeignImport`]: each `(local_name, module_path, from)`
    /// entry, in order, stores into the module global `local_name`, where
    /// the statement runs, the module object
    /// `pycc_ext_obj_import(module_path)` returns when `from` is `None`, or
    /// the attribute a `pycc_ext_obj_import_from` call returns for a
    /// from-import's name (#1383; `from` as for [`MirItem::ForeignImport`]).
    /// A failed import whose exception is an `ImportError` -- a missing
    /// name of a from-import included -- becomes a pycc raise of
    /// `ImportError`/`ModuleNotFoundError` (#1293), so an enclosing handler
    /// runs; any other failure is bridged by the object bridge inside a
    /// module-level `try`, and returns `-1` from `Py_mod_exec` directly
    /// outside every one (Part 1 of #1096).
    ForeignImport {
        bindings: Vec<(String, String, Option<FromImport>)>,
    },
}

/// A comprehension's already-resolved iterable source (PR-12, D-117) --
/// the MIR-level counterpart to `pycc_hir::CompIter`, but with a
/// bare-name iterable already split into its concrete container kind,
/// mirroring `HirStmt::ForList`'s own split into `MirStmt::ForList`/
/// `ForDict`/`ForSet` at this exact lowering stage. Kept as a field on
/// each `*CompAssign` variant (not exploded into a full cross-product of
/// top-level `MirStmt` variants per comprehension-kind x source-kind
/// combination) -- mirrors the precedent `MirExpr::Subscript` already
/// established (one node, internal branching on the resolved type in
/// `pycc_codegen`), avoiding a 3x4 combinatorial explosion for no
/// benefit.
#[derive(Debug, Clone, PartialEq)]
pub enum CompSource {
    Range {
        start: MirExpr,
        stop: MirExpr,
        step: MirExpr,
    },
    List(String),
    Dict(String),
    Set(String),
    /// A CPython object iterated through the iterator protocol (Part 1 of
    /// #1255): the iterable expression, evaluated once in the enclosing
    /// scope. Its loop variable is a CPython object, and the comprehension
    /// produces a CPython `list` or `set` ([`MirComprehension::ty`]). Only
    /// the expression form carries it: `pycc_hir` lowers the statement form
    /// of such a comprehension to a plain assignment.
    Object(MirExpr),
}

#[derive(Debug, PartialEq)]
pub enum MirItem {
    Function {
        name: String,
        params: Vec<(String, Ty)>,
        return_ty: Ty,
        body: Vec<MirStmt>,
    },
    TopLevelStmt(MirStmt),
    /// Part 1 of #1026: a foreign (CPython-object) import, spliced into
    /// `MirModule::items` at the position the corresponding module-body
    /// statement would have occupied, so codegen's single source-order
    /// item loop emits the `pycc_ext_obj_import` call exactly where the
    /// `import` statement stood.
    ///
    /// It is an *item*, not a side table, precisely so that ordering is
    /// structural rather than a convention codegen has to re-derive: PR 1b
    /// carried the same information as a separate `foreign_imports` vector,
    /// which could only be emitted as a hoisted prologue and would have run
    /// a module's imports ahead of a preceding statement's observable
    /// effects -- a divergence from CPython that D-244 rule 3 does not
    /// admit.
    ///
    /// `from` is `None` for `import X`, and for one name of a top-level
    /// `from X import a, b` (#1278) it is that binding's name, its index,
    /// the statement's whole fromlist and its relative `level`, which the
    /// `pycc_ext_obj_import_from` call passes to CPython's `__import__`.
    /// `level` is `0` except for the entry module's relative
    /// `from .x import a, b` under `--ext --foreign-relative-imports`
    /// (#1366).
    ForeignImport {
        local_name: String,
        module_path: String,
        from: Option<FromImport>,
    },
}

#[derive(Default)]
pub struct MirModule {
    pub items: Vec<MirItem>,
    /// #379 (PR-19): the module's class definitions, carried through to
    /// codegen so it can emit per-enum-member singleton init sequences.
    /// Codegen needs the `enum_members` table (member names and value
    /// types) to allocate and initialize each member's singleton instance
    /// at module-init time. Non-enum class defs are also carried, though
    /// codegen only reads `enum_members` from them today.
    pub class_defs: Vec<(String, pycc_hir::HirClassDef)>,
}

pub fn build(hir: &HirModule) -> MirModule {
    let mut scopes: Vec<HashMap<String, Ty>> = vec![HashMap::new()];
    // D-154 (Part 1 of #375): a plain `HashMap` clone of `hir.class_defs`,
    // built once and threaded read-only through every lowering call below
    // (`lower_item`/`lower_stmt`/`lower_expr`/`resolve_comp_source`) --
    // mirrors `scopes`' own "compute once in `build`, thread through every
    // recursive call" shape, but never mutated the way `scopes` is (a
    // class's declared shape does not change while lowering a module).
    let classes: HashMap<String, HirClassDef> = hir.class_defs.iter().cloned().collect();
    // Part 2 of #1026 (#1081): a foreign import's local name is a *value*
    // binding of type `object` for the rest of the module, so `lower_expr`'s
    // `HirExpr::Name` arm can resolve it through `lookup` like any other.
    // Part 1 needed no such binding -- the check phase refused every
    // expression-position read of a foreign name, so lowering never saw one
    // and `lookup`'s "pycc_types::check should have rejected this" panic was
    // correct. Part 2 admits `numpy.pi`, which makes the read reachable, and
    // without this bind the first admitted program would hit that panic.
    //
    // Bound eagerly, before any statement lowers, rather than at the import's
    // own position. That is sound only because the check phase refuses every
    // read this bind could answer wrongly, and PR 2a of #1081 had to *restore*
    // both halves of that after its review found the eager bind admitting
    // programs Part 1 refused: `pycc_types::module` no longer pre-seeds the
    // foreign name ahead of its own position, so a module-body read above the
    // `import` is `T0021`. A function body may read the name since #1316,
    // and a helper called above the `import` is admitted, as D-041 cannot
    // prove call order: the read goes through the global's initialized
    // flag, and `pycc_codegen`'s unbound-global branch raises the
    // `NameError` CPython raises there instead of trapping (`llvm.trap`,
    // rc 133). Part 1's shadowing rule (`C0001`, `docs/TYPE_SYSTEM.md`)
    // then guarantees no other top-level statement ever rebinds the name,
    // except the one admitted rebinding, an `except ImportError` fallback
    // of the same `try` (#1485), which stores a boxed `None` or another
    // foreign object into the same `object` slot -- so the eager bind
    // answers every admitted read with the same type a positional one
    // would. A function body reads it through the same
    // outward `lookup` walk, matching the constraint solver's own
    // per-function `foreign_objects` copy.
    for import in &hir.imports {
        if let ImportBinding::Foreign { local_name, .. } = import {
            bind(&mut scopes, local_name.clone(), Ty::Object);
        }
    }
    // First pass: register every function's mangled `$fn:name` signature
    // before lowering any item body -- mirrors `pycc_types::check`'s own
    // two-pass fix (D-038/D-039) so a forward reference, a sibling call, or
    // a recursive self-call all resolve to the right return type regardless
    // of where the callee's `def` appears in the module.
    for item in &hir.items {
        if let HirItem::Function {
            name, return_ty, ..
        } = item
        {
            bind(&mut scopes, format!("$fn:{name}"), return_ty.clone());
        }
    }
    // Lower module statements first, in source order, so the module scope is
    // complete before any function body is lowered. This mirrors
    // `pycc_types::check_with_signatures_all`'s D-041 three-pass contract:
    // top-level forward reads stay invalid because these statements are still
    // visited sequentially, while a function may read a global assigned after
    // its `def` because function bodies are evaluated only when called.
    let mut lowered: Vec<Option<MirItem>> = hir.items.iter().map(|_| None).collect();
    for (index, item) in hir.items.iter().enumerate() {
        if let HirItem::TopLevelStmt(stmt) = item {
            lowered[index] = Some(lower_item(item, &mut scopes, &classes));
            // Issue #769 (Part 2 of #747): the early-return continuation
            // shape has no meaning at true module scope (`return` cannot
            // syntactically appear outside a function, so
            // `apply_post_if_narrowing` never actually narrows anything
            // here), but this call keeps every sequential-statement-list
            // walk in this crate routed through the same narrowing-aware
            // shape uniformly, mirroring
            // `pycc_types::check_with_environment_all`'s own identical no-op
            // call at its module top level.
            object_narrow::apply_post_if_narrowing(stmt, &mut scopes, &classes);
        }
    }
    for (index, item) in hir.items.iter().enumerate() {
        if matches!(item, HirItem::Function { .. }) {
            lowered[index] = Some(lower_item(item, &mut scopes, &classes));
        }
    }
    let mut items: Vec<MirItem> = lowered
        .into_iter()
        .map(|item| item.expect("every HIR item is either a function or a top-level statement"))
        .collect();
    splice_foreign_imports(&mut items, &hir.imports);
    let module = MirModule {
        items,
        class_defs: hir.class_defs.clone(),
    };
    // #1337 (D-254): every user-method call runs the body its static
    // receiver class resolves to; re-derived independently of the routing
    // sites and checked on every debug (so every test) build.
    #[cfg(debug_assertions)]
    verify_receiver::verify(&module, &classes);
    module
}

/// Part 1 of #1026: inserts a [`MirItem::ForeignImport`] into `items` for
/// every foreign binding, each at the position it occupied among the module
/// statements (its `ForeignImportSite::Item` index, rebased onto the linked
/// program by `pycc_hir::program::link`). A `ForeignImportSite::Block`
/// import is skipped: it runs as the `MirStmt::ForeignImport` statement its
/// block lowers to (#1291).
///
/// Insertion is by ascending index with a running offset, so two imports
/// recorded at the same or at increasing positions both land in source
/// order: the `n`-th insertion shifts every later recorded index by `n`.
/// `HirModule::imports` is already in source order -- `module::lower_module`
/// appends each statement's bindings as it walks the body, and `link`
/// concatenates modules in dependency order -- so the indices arrive sorted
/// and `sort` is unnecessary; the offset alone is what keeps them correct.
///
/// An index past the end of `items` cannot occur: it is the item count at
/// the moment the import lowered, and every stage between that moment and
/// this one either appends items or, when it changes the list the positions
/// refer to, recomputes them against the list it produces --
/// `pycc_types::monomorphize`, which drops each original generic and each
/// protocol-parameter function, and `pycc_types::enum_lower`'s
/// `unroll_enum_loops`, which expands one top-level enum loop into one item
/// per member. `insert` would panic rather than misplace
/// the call if that invariant were ever broken, which is the failure this
/// splice wants -- PR 1c of #1080 review finding 1 is exactly that panic,
/// observed before `monomorphize` did the recomputation.
fn splice_foreign_imports(items: &mut Vec<MirItem>, imports: &[ImportBinding]) {
    let mut inserted = 0usize;
    for binding in imports {
        let ImportBinding::Foreign {
            local_name,
            module_path,
            from,
            site: ForeignImportSite::Item(item_index),
            ..
        } = binding
        else {
            continue;
        };
        items.insert(
            item_index + inserted,
            MirItem::ForeignImport {
                local_name: local_name.clone(),
                module_path: module_path.clone(),
                from: from.clone(),
            },
        );
        inserted += 1;
    }
}

fn lower_item(
    item: &HirItem,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
) -> MirItem {
    match item {
        HirItem::Function {
            name,
            params,
            return_ty,
            body,
        } => {
            // #433: extract the class name from a mangled
            // `<ClassName>.<method>` name so `lower_expr`'s `Super` arm
            // can resolve the next class in the MRO. A top-level function
            // name contains no `.`, so `current_class` is `None` for those.
            let current_class: Option<&str> = item_anchor_class(name, classes);
            scopes.push(params.iter().cloned().collect());
            let mut body = lower_stmt_sequence(body, scopes, classes, current_class);
            scopes.pop();
            let frame_name = source_frame_name(name);
            set_frame_function(&mut body, &frame_name);
            MirItem::Function {
                name: name.clone(),
                params: params.clone(),
                return_ty: return_ty.clone(),
                body,
            }
        }
        HirItem::TopLevelStmt(stmt) => {
            let mut stmt = lower_stmt(stmt, scopes, classes, None);
            set_frame_function(std::slice::from_mut(&mut stmt), "<module>");
            MirItem::TopLevelStmt(stmt)
        }
    }
}

/// Fills every [`MirStmt::Raise`]/[`MirStmt::RaiseFrom`] reachable from
/// `body` with `frame_name` -- the enclosing function's source name, or
/// `"<module>"` for a top-level statement (#707). Run once per item by
/// [`lower_item`] after lowering completes, rather than threading
/// `frame_name` as a parameter through every statement-lowering function's
/// own recursive descent (see [`MirStmt::Raise::frame_function`]'s doc
/// comment for why): `frame_name` never varies within one item, so a single
/// post-order walk over the already-built tree is strictly cheaper and
/// cannot desynchronize from `lower_stmt`'s own call graph the way a
/// parallel threaded parameter could.
///
/// Recurses into every `MirStmt` variant that nests further statements --
/// `If`, `While`, every `For*` loop, `Try`/`TryStar` (body, each handler's
/// body, `orelse`, `finalbody`), and `Seq` -- so a `raise` anywhere inside a
/// loop body, a handler, or a `match`'s lowered `if` chain (`Seq`/`If`) is
/// reached, not only a top-level statement in `body` itself.
fn set_frame_function(body: &mut [MirStmt], frame_name: &str) {
    for stmt in body {
        match stmt {
            MirStmt::Raise { frame_function, .. } => {
                frame_function.clear();
                frame_function.push_str(frame_name);
            }
            MirStmt::RaiseFrom { frame_function, .. } => {
                frame_function.clear();
                frame_function.push_str(frame_name);
            }
            MirStmt::If { body, orelse, .. } => {
                set_frame_function(body, frame_name);
                set_frame_function(orelse, frame_name);
            }
            MirStmt::While { body, .. }
            | MirStmt::ForRange { body, .. }
            | MirStmt::ForList { body, .. }
            | MirStmt::ForObject { body, .. }
            | MirStmt::ForDict { body, .. }
            | MirStmt::ForSet { body, .. } => set_frame_function(body, frame_name),
            MirStmt::Seq(stmts) => set_frame_function(stmts, frame_name),
            MirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | MirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                set_frame_function(body, frame_name);
                for handler in handlers {
                    set_frame_function(&mut handler.body, frame_name);
                }
                set_frame_function(orelse, frame_name);
                set_frame_function(finalbody, frame_name);
            }
            MirStmt::ExprStmt(_)
            | MirStmt::Assign { .. }
            | MirStmt::NoOp
            | MirStmt::Unreachable
            | MirStmt::DictSet { .. }
            | MirStmt::BufferSet { .. }
            | MirStmt::ListCompAssign { .. }
            | MirStmt::DictCompAssign { .. }
            | MirStmt::SetCompAssign { .. }
            | MirStmt::Return(_)
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::AttrSet { .. }
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjRaise { .. }
            | MirStmt::ForeignImport { .. }
            | MirStmt::Reraise => {}
        }
    }
}

fn bind(scopes: &mut [HashMap<String, Ty>], name: String, ty: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .insert(name, ty);
}

fn bind_variable(scopes: &mut [HashMap<String, Ty>], name: String, ty: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .entry(name)
        .or_insert(ty);
}

fn lookup(scopes: &[HashMap<String, Ty>], name: &str) -> Ty {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).cloned())
        .unwrap_or_else(|| panic!("pycc_mir: internal error: `{name}` has no recorded type -- pycc_types::check should have rejected this HIR before it reached pycc_mir"))
}

/// Issue #769 (Part 2 of #747): the sentinel key `narrowed_scope_key`
/// pushes into the top `scopes` frame to record "`name` is currently
/// narrowed to its `Optional`'s inner type" -- mirroring `lower_expr`'s
/// existing `format!("$fn:{callee}")` sentinel-key pattern (used to record
/// "this name is a user-defined function", also inside the same `scopes`
/// stack rather than through a separate threaded parameter). `$` never
/// appears in a real Python identifier, so a sentinel key can never
/// collide with an actual variable name.
fn narrowed_scope_key(name: &str) -> String {
    format!("$narrowed:{name}")
}

/// Issue #769 (Part 2 of #747): records that `name` is narrowed to `inner`
/// for the remainder of the current top `scopes` frame -- called by
/// `lower_stmt`'s `If` arm before lowering the narrowed branch, and popped
/// with [`kill_narrowing`] immediately after.
fn push_narrowing(scopes: &mut [HashMap<String, Ty>], name: &str, inner: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .insert(narrowed_scope_key(name), inner);
}

/// Issue #769 (Part 2 of #747): clears `name`'s narrowing sentinel from the
/// top `scopes` frame -- called both to pop a narrowed region
/// ([`push_narrowing`]'s counterpart) and, independently, whenever `name`
/// is reassigned inside a narrowed region (`lower_stmt`'s `Assign`/
/// `AnnAssign` arms) so a stale `MirExpr::OptionalUnwrap` is never emitted
/// for a read that follows the reassignment.
///
/// D-068 re-review of #780 (seventh round): only clears the sentinel in
/// `scopes.last()`, while [`narrowed_ty`] searches the whole stack
/// innermost-to-outward. This is sound today only because `scopes` ever
/// gains a second frame in exactly one place (`lower_item`'s `Function`
/// arm) and no narrowing sentinel is ever live across that push --
/// functions are lowered in a separate pass after all module-level
/// statements, and this HIR has no nested-function construct. If a future
/// change adds another `scopes.push` while an outer-frame sentinel is
/// still live, that sentinel would become unkillable through this
/// function (which only touches the top frame) while remaining visible to
/// [`narrowed_ty`]'s full-stack search -- silently reintroducing the same
/// stale-narrowing class every earlier round of this review fixed, through
/// a different mechanism. Any new `scopes.push` site must account for this
/// before it can carry a live narrowing sentinel across it.
fn kill_narrowing(scopes: &mut [HashMap<String, Ty>], name: &str) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .remove(&narrowed_scope_key(name));
}

/// Issue #769 follow-up (D-068 re-review of #780, third round): the MIR
/// counterpart of `pycc_types::narrow::apply_kill_prescan` -- drops every
/// name `body` reassigns anywhere within it (per `pycc_hir::killed_names`)
/// from `scopes`' top-frame narrowing sentinels, for `body`'s entire
/// extent rather than only from the kill's own source position onward.
///
/// Required for MIR to stay consistent with the checker's identical fix:
/// without this, `pycc_types::check` would reject a re-entrant read (a
/// loop body, or an `except` handler reached partway through the `try`
/// body it guards) that the checker's own `narrow::apply_kill_prescan`
/// drops the narrowing for, but MIR lowering -- reached only when
/// `pycc_types::check` already accepted the same HIR for some *other*,
/// still-narrowable read in the same body -- would still emit an
/// unconditional `MirExpr::OptionalUnwrap` for a read this checker fix
/// now no longer allows to be narrowed. Called from the same two site
/// classes as the checker's: `stmt::lower_loop_body` (every
/// `While`/`ForRange`/`ForList` lowering path, all funneled through that
/// one shared helper) and `stmt::try_stmt::lower_try` (each handler body,
/// prescanned against the *try body's* kill set before lowering, and the
/// `finally` body against every path's).
fn apply_kill_prescan(scopes: &mut [HashMap<String, Ty>], body: &[HirStmt]) {
    for name in pycc_hir::killed_names(body) {
        kill_narrowing(scopes, &name);
    }
}

/// Issue #769 (Part 2 of #747): `Some(inner)` when `name` is currently
/// narrowed to `inner` (an `Optional`'s inner type), `None` otherwise.
/// Consulted only by `lower_expr`'s `HirExpr::Name` arm, exactly mirroring
/// `pycc_types::env::Environment::narrowed_ty`'s own "reads consult the
/// overlay, assignment targets never do" split -- `lower_stmt`'s own
/// `Assign`/`AnnAssign` arms bind `target`'s *real* type via `bind`/
/// `bind_variable` directly and never call this.
fn narrowed_ty(scopes: &[HashMap<String, Ty>], name: &str) -> Option<Ty> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(&narrowed_scope_key(name)).cloned())
}

/// Issue #769 (Part 2 of #747): lowers a sequential list of statements --
/// a function body, or any `if`/`while`/`for` body -- applying the
/// early-return continuation narrowing ([`object_narrow::apply_post_if_narrowing`]) after
/// each one. The narrowing-aware replacement for the raw `stmts.iter()
/// .map(|s| lower_stmt(s, scopes, classes, current_class)).collect()`
/// pattern every sequential body lowering in this crate used before this
/// issue (mirroring `pycc_types::narrow::check_stmt_sequence`'s identical
/// role at the checker layer).
///
/// Callers processing the *top-level* sequence of a function or module body
/// call this directly, since narrowing established there is meant to persist
/// for the rest of that same sequence. A *nested* body (an `if`/`while`/`for`
/// arm, a `try`/`except` handler, a `match` arm, ...) must instead go through
/// [`lower_scoped_body`], which isolates whatever this function accumulates
/// in `scopes`' top frame so it cannot leak past the nested body's own close
/// -- see that function's doc comment for the soundness rationale.
fn lower_stmt_sequence(
    stmts: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> Vec<MirStmt> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        out.push(lower_stmt(stmt, scopes, classes, current_class));
        object_narrow::apply_post_if_narrowing(stmt, scopes, classes);
    }
    out
}

/// Issue #769 (Part 2 of #747): captures the narrowing-sentinel subset
/// (every `$narrowed:{name}` key, see [`narrowed_scope_key`]) of `scopes`'
/// top frame, for later restoration via [`restore_narrowing`].
fn narrowing_snapshot(scopes: &[HashMap<String, Ty>]) -> HashMap<String, Ty> {
    scopes
        .last()
        .expect("at least one scope is always present")
        .iter()
        .filter(|(key, _)| key.starts_with("$narrowed:"))
        .map(|(key, ty)| (key.clone(), ty.clone()))
        .collect()
}

/// Issue #769 (Part 2 of #747): replaces `scopes`' top frame's entire
/// narrowing-sentinel subset with `snapshot` -- the counterpart of
/// [`narrowing_snapshot`], undoing every `push_narrowing`/`kill_narrowing`
/// mutation made since the snapshot was taken, whether from a direct
/// `push_narrowing` call or indirectly from a nested
/// [`object_narrow::apply_post_if_narrowing`] hit deeper in the body being restored from.
fn restore_narrowing(scopes: &mut [HashMap<String, Ty>], snapshot: HashMap<String, Ty>) {
    let top = scopes
        .last_mut()
        .expect("at least one scope is always present");
    top.retain(|key, _| !key.starts_with("$narrowed:"));
    top.extend(snapshot);
}

/// #1476: the end state of the branch an `if` guard narrows `name` in, with
/// the guard's own narrowing replaced by `name`'s pre-`if` entry in
/// `before` (or none) -- unless `branch` rebinds `name`, whose kill already
/// decided its end state. The MIR-side twin of
/// `pycc_types::narrow::end_branch_narrowing`.
fn end_branch_narrowing(
    before: &HashMap<String, Ty>,
    end: &mut HashMap<String, Ty>,
    name: &str,
    branch: &[HirStmt],
) {
    if pycc_hir::killed_names(branch)
        .iter()
        .any(|killed| killed == name)
    {
        return;
    }
    let key = narrowed_scope_key(name);
    match before.get(&key) {
        Some(outer) => end.insert(key, outer.clone()),
        None => end.remove(&key),
    };
}

/// Blocker fix (D-068 review of #780): the MIR-side counterpart of
/// `pycc_types::narrow::join_narrowed`. A name stays narrowed to `ty` after
/// two or more possible-path narrowing snapshots are reconciled only if
/// *every* supplied snapshot narrows it to that exact `ty`; a snapshot that
/// killed it (or never had it) drops it from the intersection entirely.
/// Operates on [`narrowing_snapshot`]-shaped maps (`$narrowed:{name}` ->
/// inner type), so the result can be handed directly to
/// [`restore_narrowing`]. See that Rust doc's checker-side twin for the
/// full soundness rationale -- this is the identical algorithm, kept as an
/// independent copy rather than a shared crate dependency because
/// `pycc_mir` does not (and per the existing `pycc_hir::optional_none_test`
/// precedent, should not) depend on `pycc_types`.
///
/// Every call site has at least one snapshot by construction (an `if`'s
/// body/orelse end-state, a loop's pre-body/post-body end-state, or a
/// `try`'s non-empty list of fall-through end states), so
/// `first` is taken separately from `rest` rather than a possibly-empty
/// slice -- that keeps the join total without an unreachable empty-input
/// branch.
fn join_narrowed(
    first: &HashMap<String, Ty>,
    rest: &[&HashMap<String, Ty>],
) -> HashMap<String, Ty> {
    let mut joined: HashMap<String, Ty> = first.clone();
    for snapshot in rest {
        joined.retain(|key, ty| snapshot.get(key) == Some(ty));
    }
    joined
}

/// Issue #769 (Part 2 of #747): lowers a *nested* statement body (an
/// `if`/`while`/`for` arm, a `try`/`except`/`finally` handler, a `match`
/// arm, ...) with its own narrowing state isolated from the enclosing
/// sequence, mirroring `pycc_types::narrow`'s own per-branch `Environment`
/// clone-and-discard design (see `narrow.rs`'s module doc comment) one layer
/// down.
///
/// `scopes`' single function-level frame (pushed once at function entry,
/// never per-block -- see this module's existing scope-stack convention) is
/// shared, mutable state threaded through every nested body, unlike the
/// checker's own per-branch `Environment` *clones*. Without this wrapper, a
/// narrowing fact established by [`object_narrow::apply_post_if_narrowing`] deep inside one
/// branch (e.g. a nested early-return guard) would persist in that shared
/// frame past the branch's own close and incorrectly narrow reads in a
/// sibling branch or in code after the enclosing construct entirely.
/// [`narrowing_snapshot`]/[`restore_narrowing`] recreate the clone-and-discard
/// effect cheaply, without cloning the rest of `scopes`.
///
/// `narrow` optionally pushes one additional in-branch narrowing fact (the
/// enclosing `if`'s own recognized `is`/`is not None` test, exactly the
/// role `push_narrowing`/`kill_narrowing` played before this wrapper existed)
/// before lowering `stmts` -- `restore_narrowing` undoes this push too, since
/// the snapshot is taken before it is applied.
///
/// Blocker fix (D-068 review of #780): also returns the narrowing-sentinel
/// subset as it stood *immediately after* lowering `stmts`, before the
/// isolating restore -- this branch's "ending" narrowed state. Isolation
/// during lowering (so a nested body's own narrowing never leaks into a
/// sibling while both are being lowered) and reporting that branch's own
/// outcome to the caller are two different concerns: unconditionally
/// discarding the ending state (the pre-fix behavior) silently reverted a
/// same-branch `kill_narrowing` the moment this nested body closed, even
/// when nothing else contradicted it. A caller that needs the
/// join-reconciled effect of one or more sibling branches (`HirStmt::If`'s
/// body+orelse pair, a loop's single body joined against "the loop ran zero
/// times") combines the returned end-states with [`join_narrowed`] and
/// applies the result back onto `scopes` via a second, deliberate
/// `restore_narrowing` call of its own -- see `stmt.rs`'s `If`/`While`/
/// `ForRange`/`ForList` arms, and `stmt::try_stmt`, which since #1476
/// continues `else` from the body's end state and joins every
/// fall-through path's end state after the `try`. A caller that does not
/// need that (a `match` case) simply ignores the second return value,
/// preserving the prior "restore to entry state" behavior exactly.
fn lower_scoped_body(
    stmts: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
    narrow: Option<(&str, Ty)>,
) -> (Vec<MirStmt>, HashMap<String, Ty>) {
    let snapshot = narrowing_snapshot(scopes);
    if let Some((name, inner)) = narrow {
        push_narrowing(scopes, name, inner);
    }
    let out = lower_stmt_sequence(stmts, scopes, classes, current_class);
    let end_state = narrowing_snapshot(scopes);
    restore_narrowing(scopes, snapshot);
    (out, end_state)
}

/// #436/#432: Looks up a class in the `classes` table by its MRO entry
/// name, panicking with a consistent internal-error message if the class
/// is not registered. Centralizing this lookup ensures the defensive
/// panic is covered by a single test rather than duplicated across every
/// MRO walk (where later walks are unreachable after the first walk
/// panics on a ghost class).
fn mro_class_def<'a>(
    mro_class: &str,
    classes: &'a HashMap<String, HirClassDef>,
) -> &'a HirClassDef {
    classes.get(mro_class).unwrap_or_else(|| {
        panic!(
            "pycc_mir: internal error: class `{mro_class}` in MRO has no registered \
             HirClassDef -- pycc_types::check should have rejected this HIR before \
             it reached pycc_mir"
        )
    })
}

/// Resolves a `pycc_hir::CompIter` into a fully-typed `CompSource`,
/// lowering any range sub-expressions and binding `var`'s type into
/// `scopes` -- mirrors `HirStmt::ForList`'s own resolution exactly
/// (`lower_stmt`'s existing `ForList` arm), reused via this shared helper
/// rather than duplicated three times (once per comprehension kind, PR-12,
/// D-117). Takes `scopes` as `&mut [HashMap<String, Ty>]` (a slice), not
/// `&mut Vec<..>` like `lower_stmt`/`lower_item` -- unlike those two, this
/// helper never pushes/pops a scope itself, so `clippy::ptr_arg` genuinely
/// flags an owned `Vec` parameter here as unnecessary. `lower_item`'s
/// `&mut Vec` is a real need: its `Function` arm calls `scopes.push(..)`
/// before lowering the function body and `scopes.pop()` after, and
/// `&mut [_]` has no such methods. `lower_stmt` itself never calls a
/// `Vec`-only method on `scopes` either (every other callee it passes
/// `scopes` to -- `lower_expr`, `bind_variable`, `lookup`,
/// `resolve_comp_source` -- already takes a slice); it is self-recursive
/// (it calls itself for nested statement bodies, e.g. `If`/`While`), not
/// mutually recursive with `lower_item` (`lower_item` calls `lower_stmt`,
/// but `lower_stmt` never calls back into `lower_item`). `clippy::ptr_arg`
/// does not fire on a self-recursive function, so nothing forces or even
/// flags `lower_stmt`'s own `&mut Vec` as unnecessary; it keeps the owned
/// type by convention, matching its only caller, `lower_item`, rather than
/// out of its own requirement.
pub(crate) fn resolve_comp_source(
    iter: &CompIter,
    var: &str,
    scopes: &mut [HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (CompSource, Ty) {
    match iter {
        CompIter::Range { start, stop, step } => {
            let start = lower_expr(start, scopes, classes, current_class);
            let stop = lower_expr(stop, scopes, classes, current_class);
            let step = lower_expr(step, scopes, classes, current_class);
            bind_variable(scopes, var.to_string(), Ty::Int);
            // D-068 re-review of #780 (sixth round): a comprehension's own
            // loop variable is a rebinding exactly like `ForRange`'s
            // induction variable (see that arm's identical comment in
            // `stmt.rs`) -- must kill a stale narrowing sentinel too,
            // mirroring the checker's `check_assignment(env, var, Ty::Int)`
            // for the equivalent `ListCompAssign`/`SetCompAssign`/
            // `DictCompAssign` arms.
            kill_narrowing(scopes, var);
            (CompSource::Range { start, stop, step }, Ty::Int)
        }
        CompIter::Name(name) => match lookup(scopes, name) {
            Ty::List(elem_ty) => {
                bind_variable(scopes, var.to_string(), (*elem_ty).clone());
                kill_narrowing(scopes, var);
                (CompSource::List(name.clone()), *elem_ty)
            }
            Ty::Dict(kv) => {
                bind_variable(scopes, var.to_string(), kv.0.clone());
                kill_narrowing(scopes, var);
                (CompSource::Dict(name.clone()), kv.0)
            }
            Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => {
                bind_variable(scopes, var.to_string(), (*elem_ty).clone());
                kill_narrowing(scopes, var);
                (CompSource::Set(name.clone()), *elem_ty)
            }
            other => panic!(
                "pycc_mir: internal error: `{name}` is neither a list, dict, set, nor frozenset (found `{}`) -- pycc_types::check should have rejected this HIR before it reached pycc_mir",
                other.name()
            ),
        },
        // Part 1 of #1255: `pycc_types` proved the iterable a CPython
        // object, so the loop variable is one too. The iterable is lowered
        // before `var` is bound, so it reads only the enclosing bindings.
        CompIter::Iterable(iterable) => {
            let iterable = lower_expr(iterable, scopes, classes, current_class);
            bind_variable(scopes, var.to_string(), Ty::Object);
            kill_narrowing(scopes, var);
            (CompSource::Object(iterable), Ty::Object)
        }
    }
}

#[cfg(test)]
mod tests;
