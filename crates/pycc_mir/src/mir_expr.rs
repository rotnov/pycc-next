//! MIR expression nodes (`MirExpr`) and their structural queries.

use super::*;

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
    /// Codegen emits the same `pycc_rt_list_new()` object
    /// `ListLiteral(vec![])` would, with no `append` calls after it.
    EmptyList(Ty),
    /// An empty set of a resolved element type (`HirExpr::EmptySet`).
    EmptySet(Ty),
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
    /// `list.pop()` (mirrors `HirExpr::ListPop`) and `list.pop(index)` (a
    /// `HirExpr::MethodCall` on a list receiver). `ty` is the list's element
    /// type.
    ListPop {
        list: MirContainerReceiver,
        index: Option<Box<MirExpr>>,
        ty: Ty,
    },
    /// `list.insert(index, value)` (a `HirExpr::MethodCall` on a list
    /// receiver); its value is `None`.
    ListInsert {
        list: MirContainerReceiver,
        index: Box<MirExpr>,
        value: Box<MirExpr>,
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
    /// `left op right` where at least one operand is a CPython object
    /// (non-strict mode, #3): CPython's `PyNumber_*` for `op`, so the result
    /// and every exception are CPython's own. A native operand is boxed;
    /// the result is a new reference ([`Ty::Object`]). `in_place` (an
    /// augmented assignment's operator) selects `PyNumber_InPlace*`.
    ObjBinOp {
        op: BinOpKind,
        in_place: bool,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
    },
    /// `-o`, `+o` or `~o` on a CPython object (non-strict mode, #3):
    /// `PyNumber_Negative`/`_Positive`/`_Invert`, a new reference.
    ObjUnaryOp {
        op: pycc_hir::UnaryOpKind,
        operand: Box<MirExpr>,
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
    /// A tuple, set or dict display with a CPython object element
    /// (non-strict mode, #3), built as the CPython container: each element
    /// boxed and evaluated left to right like [`MirExpr::ObjList`]'s. A dict
    /// display's `elements` alternate key and value. [`Ty::Object`].
    ObjDisplay {
        kind: ObjDisplayKind,
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
    /// `next(gen)` / `next(gen, default)` of a generator (`ty`, its element
    /// type): resumes it, raising `StopIteration` (or answering `default`)
    /// when it is exhausted.
    GenNext {
        generator: Box<MirExpr>,
        default: Option<Box<MirExpr>>,
        ty: Ty,
    },
    /// The list of a generator's remaining values, `list(gen)` (and the
    /// operand of a consumer `pycc_types` types as given that list: `sum`,
    /// `sorted`, ...). `ty` is the `list[T]`.
    GenToList {
        generator: Box<MirExpr>,
        ty: Ty,
    },
    /// `any(gen)` / `all(gen)`: resumes the generator only until the first
    /// deciding value.
    GenAnyAll {
        generator: Box<MirExpr>,
        all: bool,
    },
    /// A function value (`ty`, a `Ty::Callable`): module-level `function`
    /// with its leading parameters bound to `captures` -- a nested
    /// function's enclosing variables (`pycc_ast::desugar::closures`), or
    /// none for a module-level function used as a value.
    MakeClosure {
        function: String,
        captures: Vec<MirExpr>,
        ty: Ty,
    },
    /// A call of a function value: `callee` (a `Ty::Callable`) applied to
    /// `args`; `ty` is its return type.
    CallClosure {
        callee: Box<MirExpr>,
        args: Vec<MirExpr>,
        ty: Ty,
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
    /// A builtin function or builtin-type method lowered to a direct call
    /// into `pycc_rt` (`pycc_hir::builtins`, whose module doc states the ABI).
    RtCall(Box<RtCallExpr>),
    /// A typed numpy array operation (`pycc_hir::ndarray`): `base` is the
    /// array's object, `args` the indices (and, for a store, the value
    /// last). `pycc_codegen::ndarray` emits element access inline from the
    /// array's data pointer, shape and strides.
    Nd(Box<MirNdExpr>),
}

/// `MirExpr::Nd`'s payload.
#[derive(Debug, Clone, PartialEq)]
pub struct MirNdExpr {
    pub op: pycc_hir::ndarray::NdOp,
    pub base: MirExpr,
    pub args: Vec<MirExpr>,
    pub nd: pycc_hir::ndarray::NdTy,
    /// `pycc_hir::ndarray::NdExpr::cached`.
    pub cached: bool,
}

impl MirNdExpr {
    /// The type the operation evaluates to (`pycc_hir::ndarray::NdExpr::ty`).
    #[must_use]
    pub fn ty(&self) -> Ty {
        use pycc_hir::ndarray::NdOp;
        match self.op {
            NdOp::Get { .. } => self.nd.elem_ty(),
            NdOp::Set { .. } | NdOp::Check => Ty::None,
            NdOp::Shape => Ty::Tuple(Box::new(vec![Ty::Int; usize::from(self.nd.ndim)])),
            NdOp::Size | NdOp::Ndim | NdOp::Dim(_) => Ty::Int,
        }
    }
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
