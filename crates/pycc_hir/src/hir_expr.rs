//! HIR expression nodes: `HirExpr`, comprehensions, f-string parts and container receivers.

use super::*;

/// The receiver of a hand-recognized container method node
/// (`HirExpr::ListAppend`, `HirExpr::ListPop`, `HirExpr::DictGetOrDefault`).
///
/// D-105 point 3 keyed these nodes by a bare variable name. #1263 (Part 2
/// of #1218) adds an attribute read, so a `list[int]`/`dict[str, int]`
/// instance slot (#1262) accepts `self.xs.append(v)`, `self.xs.pop()` and
/// `self.d.get(k, default)`. The `Name` arm keeps the original shape, so
/// every consumer's bare-name path is unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerReceiver {
    /// `xs.append(v)`: the container is the variable `xs`.
    Name(String),
    /// `<base>.<attr>.append(v)`. Always holds an `HirExpr::AttrGet`;
    /// `pycc_types` types it like any other attribute read and rejects a
    /// non-container result with the node's own diagnostic.
    Attr(Box<HirExpr>),
}

impl ContainerReceiver {
    /// The attribute-read sub-expression of an `Attr` receiver; `None` for a
    /// bare name. The expression walkers recurse through this, because an
    /// attribute receiver is an ordinary sub-expression (it may itself hold
    /// a walrus, a generic call, ...), while a bare name is a leaf they
    /// treat exactly as before #1263.
    #[must_use]
    pub fn attr_expr(&self) -> Option<&HirExpr> {
        match self {
            ContainerReceiver::Name(_) => None,
            ContainerReceiver::Attr(receiver) => Some(receiver),
        }
    }

    /// [`ContainerReceiver::attr_expr`], mutably, for the rewriting passes.
    pub fn attr_expr_mut(&mut self) -> Option<&mut HirExpr> {
        match self {
            ContainerReceiver::Name(_) => None,
            ContainerReceiver::Attr(receiver) => Some(receiver),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    /// A `bool` whose value no pass may know or fold: the test of
    /// [`loop_else_as_if`]'s `if`, so the suite it guards always joins
    /// with the path that skipped it. Never produced from source.
    OpaqueBool,
    StringLiteral(String),
    /// The bare literal `None` used as a value expression (D-197, #763,
    /// Part 1 of #747), e.g. `x = None`, `return None`, `x is None`. Before
    /// this PR, `Expr::NoneLiteral` had no `lower_expr` arm at all and fell
    /// through to the generic `C0001` "expression kind not supported yet"
    /// rejection -- distinct from `ret.value.as_deref().is_none()`'s
    /// existing `HirStmt::Return(None)` encoding for a bare `return`
    /// (no expression at all), which this variant does not replace.
    NoneLiteral,
    /// CPython's `NotImplemented` singleton, as the value of `return
    /// NotImplemented` in a comparison method of an `ext` module (#1418).
    /// Typed `Ty::Object`. `crate::not_implemented` owns where it may
    /// appear: nowhere else, and never in a `native` build, where the name
    /// stays an undefined `HirExpr::Name`.
    NotImplemented,
    Name(String),
    Call {
        callee: String,
        args: Vec<HirExpr>,
    },
    BinOp {
        op: BinOpKind,
        left: Box<HirExpr>,
        right: Box<HirExpr>,
    },
    /// The binary operation of an augmented assignment `target op= value`,
    /// which `stmt::aug_assign` lowers to `target = target op value` with
    /// this node as the value. Typed and lowered exactly as
    /// [`HirExpr::BinOp`], except that a CPython object operand runs the
    /// in-place protocol (`PyNumber_InPlaceAdd`, ...), so a mutable object
    /// is updated in place as CPython's `__iadd__` does (non-strict mode, #3).
    AugBinOp {
        op: BinOpKind,
        left: Box<HirExpr>,
        right: Box<HirExpr>,
    },
    Compare {
        op: CmpOpKind,
        left: Box<HirExpr>,
        right: Box<HirExpr>,
    },
    /// A chained comparison `first op1 r1 op2 r2 ...` (#1212, Part 4 of
    /// #1018).
    ///
    /// Invariant: `links.len() >= 2`. A single comparison is always
    /// [`HirExpr::Compare`]; only `crate::expr::lower_expr` builds this node,
    /// from an AST comparison with two or more operators.
    ///
    /// Evaluation contract (CPython's): `a op1 b op2 c` means `(a op1 b) and
    /// (b op2 c)`, except that each operand is evaluated at most once,
    /// strictly left to right, and evaluation stops at the first false link
    /// without evaluating any later operand. The result is always `bool`.
    CompareChain {
        first: Box<HirExpr>,
        links: Vec<CompareLink>,
    },
    /// `-operand` / `+operand` where `operand` is *not* a numeric literal
    /// (#603, Part 2 of #573).
    ///
    /// A literal operand never reaches this variant: `expr.rs`'s
    /// `Expr::UnaryOp` arm folds `-1` straight into `HirExpr::IntLiteral(-1)`
    /// first (#602, Part 1), so this node exists exactly for the cases
    /// folding cannot reach -- `-x`, `-f(y)`, `-(a + b)`.
    UnaryOp {
        op: UnaryOpKind,
        operand: Box<HirExpr>,
    },
    /// `left and right` / `left or right` (#1211, Part 3 of #1018).
    ///
    /// The AST node is n-ary; `crate::expr::lower_expr` right-folds it, so
    /// `a or b or c` is `Or(a, Or(b, c))` and every operand's truth is
    /// tested at most once (see `crate::boolop`).
    ///
    /// `truth_only` is `true` exactly when the value is consumed only for
    /// its truth -- an `if`/`elif`/`while` test, a comprehension filter, or
    /// the operand of `not` -- and is set by `crate::boolop::mark_truth_context`
    /// after lowering. A truth-only node is typed `bool` and its operands
    /// need not share a type; every other node yields the selected operand's
    /// *value*, typed by [`bool_op_result_ty`].
    BoolOp {
        op: BoolOpKind,
        left: Box<HirExpr>,
        right: Box<HirExpr>,
        truth_only: bool,
    },
    /// `body if test else orelse` (#1395).
    ///
    /// `test` is evaluated exactly once, first, and only for its truth:
    /// lowering marks it a truth position (`crate::boolop::mark_truth_context`),
    /// so an `and`/`or` directly inside it is typed `bool`. Then exactly one
    /// of `body` (truthy) or `orelse` (falsy) is evaluated, and the node
    /// yields that branch's value, typed by [`if_exp_result_ty`].
    ///
    /// A walrus is admitted in `test`, which always runs, and refused in
    /// either branch, which may not: every binding walker binds a walrus
    /// target unconditionally (see `HirExpr::BoolOp`'s lowering).
    IfExp {
        test: Box<HirExpr>,
        body: Box<HirExpr>,
        orelse: Box<HirExpr>,
    },
    /// The value of a tuple-unpacking assignment `t1, ..., tn = value`
    /// (Part 1 of #891), checked to hold exactly `arity` items.
    ///
    /// Never written by a user: `crate::stmt::unpack` binds it to a
    /// synthesized temporary and assigns each target from that temporary's
    /// literal-index subscript, so every binding pass sees ordinary
    /// `HirStmt::Assign`s. `pycc_types` admits a `tuple[...]` of exactly
    /// `arity` elements (the node is that tuple itself) and a CPython
    /// `object`, which CPython's unpack protocol turns into a fresh `tuple`
    /// of exactly `arity` items at run time or raises `ValueError` /
    /// `TypeError`. `docs/TYPE_SYSTEM.md`'s "Tuple-unpacking assignment"
    /// section owns the rule.
    Unpack {
        value: Box<HirExpr>,
        arity: usize,
    },
    FString(Vec<FStringPart>),
    /// `[e1, e2, ...]`. Element homogeneity is `pycc_types`' job, not this
    /// lowering step's -- HIR only records the syntactic shape (D-105).
    ListLiteral(Vec<HirExpr>),
    /// An empty `[]` whose element type was resolved by `pycc_types`'
    /// empty-container pre-pass (#1021, D-245).
    ///
    /// **Construction invariant:** `lower_expr` never builds this variant --
    /// it always produces `ListLiteral(vec![])` for an empty list literal.
    /// The only *inference* construction site in the workspace is
    /// `pycc_types::empty_container::resolve_empty_containers`, which
    /// rewrites the module before either `check_all_keyed` or
    /// `check_and_resolve_all_keyed` walks it. The one *syntactic* site is
    /// `pycc_hir`'s `stmt::ann_assign` (#1264): an annotated attribute
    /// target `self.xs: list[int] = []` builds it from the written
    /// annotation, so both checkers still consume the same node (D-245's
    /// 2026-09-24 amendment). The carried `Ty` is always
    /// fully concrete -- never `Ty::Infer` -- and is the *element* type, so
    /// the expression's own type is `Ty::List(element)`.
    ///
    /// It exists because `pycc_mir` derives a container's type from the
    /// literal's first element and has nothing to derive from when the
    /// literal is empty (`MirExpr::ty()`); carrying the resolved type on the
    /// node itself is what lets a resolved `[]` survive into MIR and codegen.
    EmptyList(Ty),
    /// An empty `set()` (or `{}`-free empty set) whose element type the
    /// empty-container pre-pass resolved; its type is `Ty::Set(element)`.
    EmptySet(Ty),
    /// A list display `[e1, e2, ...]`, empty or not, assigned to a name
    /// whose type is the opaque CPython object (Part 2d of #1371, D-258
    /// rule 4): it builds a fresh CPython `list`, each element packed into a
    /// `PyObject *`, and its own type is `Ty::Object`.
    ///
    /// **Construction invariant:** like [`HirExpr::EmptyList`], `lower_expr`
    /// never builds this variant. Its only construction site is
    /// `pycc_types::empty_container::resolve_empty_containers`, which
    /// rewrites a `ListLiteral` into it before either checker walks the
    /// module, so `pycc check` and `pycc build` consume the same node. Each
    /// element must have a packable type (`int`, `float`, `bool`, `str` or
    /// `object`); `pycc_types` refuses anything else with `I0404`.
    ObjectList(Vec<HirExpr>),
    /// `base[index]`, a read (Load position). `Stmt::Assign`'s own target
    /// handling below special-cases an `Expr::Subscript` target on a bare
    /// name into a dedicated `HirStmt::DictSet` node instead of ever
    /// constructing this variant for it (PR-11 Task 3, D-123 -- `list[int]`
    /// itself is still read-only-indexed, D-105, but that is now
    /// `pycc_types`' judgment on `HirStmt::DictSet`'s base type, not a
    /// structural HIR-shape restriction), and every other assignment/for
    /// target (`Stmt::AnnAssign`, `Stmt::For`) still rejects a subscript
    /// target before ever calling `lower_expr` on it (`Stmt::AnnAssign`
    /// admits an attribute target since #1264, lowering only its base, never
    /// a subscript). So a `Subscript` node
    /// still reaches this arm only in a value (Load) position; no separate
    /// `ExprContext` check is needed to enforce that here.
    Subscript {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    /// `base[start:stop:step]` (PR-12, D-118). Each bound is independently
    /// optional, matching Python's own slice grammar (`xs[:3]`, `xs[2:]`,
    /// `xs[:]`, `xs[::2]` all parse). Unlike `Subscript`'s `index` (a plain
    /// `HirExpr`), each bound here is an `Option<Box<HirExpr>>` -- an
    /// omitted bound has no source expression to lower at all, and
    /// defaulting it to a literal `0`/some sentinel here would be
    /// incorrect: `stop`'s default is `len(base)`, which needs `base`'s own
    /// already-lowered value to compute, not a value knowable at this
    /// lowering step in isolation. `pycc_types`/`pycc_mir`/`pycc_codegen`
    /// each apply the actual default at the point they have enough context
    /// to do so correctly.
    ///
    /// Like `Subscript` above, this variant is only ever constructed in
    /// Load (value) position. A slice **assignment target**
    /// (`xs[1:3] = value`) never reaches it: `Stmt::Assign`'s own
    /// `Expr::Subscript` target arm (see that arm's own handling below)
    /// calls `lower_expr(&sub.slice)` directly on the bare `Expr::Slice`
    /// node, and `lower_expr`'s outer match has no top-level arm for
    /// `Expr::Slice` (only the nested one inside this file's own
    /// `Expr::Subscript` value-position arm) -- so it falls through to the
    /// generic "expression kind not supported yet" `C0001` catch-all,
    /// unchanged by this task. This is intentional, not an oversight: D-118
    /// scopes this PR's slicing support to reads only.
    Slice {
        base: Box<HirExpr>,
        start: Option<Box<HirExpr>>,
        stop: Option<Box<HirExpr>>,
        step: Option<Box<HirExpr>>,
    },
    /// `list.append(value)`, recognized as a single dedicated node rather
    /// than through any general method-call mechanism (D-105). Unlike
    /// `Subscript` above, this arm is *not* structurally restricted to any
    /// particular position -- because `ListAppend` is an `HirExpr` (not a
    /// statement-only form), it currently lowers successfully anywhere an
    /// expression is accepted, e.g. `y = x.append(2)` or
    /// `print(x.append(1))`, even though real Python's `list.append()`
    /// always returns `None` there and a value-producing use is
    /// meaningless. This lowering step deliberately does not judge that
    /// (see `list_append_used_as_a_value_lowers_successfully_today` below,
    /// which locks in today's actual behavior). `pycc_types` doesn't reject
    /// it either -- it type-checks `y = x.append(2)` as binding `y: None`,
    /// same as any other `None`-typed value. `print(x.append(1))` runs and
    /// prints `None`, matching CPython. D-131 also gives
    /// `y = x.append(2)` ordinary canonical `None` assignment storage, so
    /// both the side effect and the stored unit value are preserved. D-072
    /// remains narrower: it rejects using `print()` itself as a nested
    /// expression, not materializable `None` results such as `.append()`.
    ///
    /// `list` is a [`ContainerReceiver`]: a bare name (D-105 point 3) or,
    /// since #1263 (Part 2 of #1218), an attribute read such as
    /// `self.xs.append(v)`.
    ListAppend {
        list: ContainerReceiver,
        value: Box<HirExpr>,
    },
    /// `{k1: v1, k2: v2, ...}`. Key/value homogeneity and the `dict[str,
    /// int]`-only codegen gate are `pycc_types`' job, not this lowering
    /// step's -- HIR only records the syntactic shape (mirrors
    /// `ListLiteral`, PR-11 Task 3). A dict-unpacking entry (`{**other}`,
    /// `DictItem.key == None` in the upstream grammar) is rejected as
    /// unsupported at lowering time (see `lower_expr`'s `Expr::Dict` arm)
    /// rather than represented here, since this variant has no shape for
    /// it.
    DictLiteral(Vec<(HirExpr, HirExpr)>),
    /// An empty `{}` whose key/value types were resolved by `pycc_types`'
    /// empty-container pre-pass (#1021, D-245). The dict counterpart of
    /// [`HirExpr::EmptyList`], and it carries that variant's construction
    /// invariant verbatim: never built by `lower_expr`, inferred only by the
    /// pre-pass, built syntactically only by `stmt::ann_assign` for
    /// `self.d: dict[str, int] = {}` (#1264), and always a fully concrete
    /// key/value pair, so the expression's own type is
    /// `Ty::Dict(pair)`.
    EmptyDict(Box<(Ty, Ty)>),
    /// `{e1, e2, ...}`. Element homogeneity and the `set[int]`-only codegen
    /// gate are `pycc_types`' job, not this lowering step's -- HIR only
    /// records the syntactic shape (mirrors `ListLiteral`/`DictLiteral`
    /// exactly, PR-11 Task 7). Unlike `Expr::Dict`'s `DictItem`, upstream's
    /// `ExprSet` has no unpacking hole in its `elts: Vec<Expr>` shape, so
    /// there is no analogous rejection arm to write. Python also has no
    /// empty-set literal syntax at all (`{}` always parses as `Expr::Dict`,
    /// the grammar's own ambiguity resolution) -- so an empty
    /// `SetLiteral` can only ever be constructed by hand-built HIR (e.g. a
    /// `pycc_types` unit test), never by `lower_expr` itself.
    SetLiteral(Vec<HirExpr>),
    /// `(e1, e2, ...)`. Element *heterogeneity* is deliberately allowed at
    /// this HIR layer -- unlike `ListLiteral`/`SetLiteral`, a tuple's whole
    /// point is mixing element types (D-116). `pycc_types` still gates which
    /// element *types* are accepted (int/bool/float only, T0039) and which
    /// index forms are readable (literal in-range only, T0040); this
    /// variant only records the syntactic shape.
    ///
    /// Only a parenthesized/bare tuple literal (`(1, 2)`, `1, 2`) lowers to
    /// this form. Tuple-unpacking assignment (`a, b = t`) is a distinct
    /// shape: its value lowers to [`HirExpr::Unpack`] (Part 1 of #891).
    TupleLiteral(Vec<HirExpr>),
    /// `list.pop()` (PR-12, D-119): a hand-recognized special form, mirroring
    /// `ListAppend`'s own shape exactly (no general method-call dispatch).
    /// No-argument form only -- removes and returns the list's last element,
    /// panicking at runtime if the list is empty (honest-panic convention,
    /// D-119). Unlike `ListAppend`, whose value is always `None`, this
    /// variant's own value is the list's element type -- `y = xs.pop()` is
    /// its primary intended use, not merely a today's-actual-behavior
    /// curiosity. A bare `xs.pop()` `ExprStmt` discarding the popped value is
    /// also fine and matches CPython. `pycc_types` gates the base value's
    /// type (`T0033`); MIR lowering and real runtime behavior are a later
    /// task's job (D-119 point 2 of the delivery split).
    ///
    /// One pre-existing, tracked caveat this variant newly reaches (not
    /// introduced or fixed by this task): `pycc_types`' constraint solver
    /// gives this expression no unification term (`Ok(None)`, mirroring
    /// every container-literal expression, per D-116's own correction), so
    /// once any function in a module is unannotated, assigning `y =
    /// xs.pop()` inside that solver-checked function's body never registers
    /// a real type term for `y` in that solver pass. This is not a novel
    /// gap: `HirExpr::Subscript`'s own `collect_expr_constraints` arm
    /// (predates D-116/D-119, see commit `0930903`) already returns
    /// `Ok(None)` the same way, so a scalar-typed expression (`xs[0]`)
    /// already reached this exact gap long before this task. `.pop()` is
    /// simply another instance of the same pre-existing class, not a new or
    /// different failure mode. A later read of `y` in the same function no
    /// longer fails with a misleading "not bound before this use" (issue
    /// #771, D-199): the solver now tracks `y` as definitely-but-opaquely
    /// bound, so a genuine downstream diagnostic surfaces instead. The
    /// underlying gap -- no type term for `y` in this solver pass -- is
    /// unchanged; only its misleading consequence is fixed.
    ///
    /// `list` is a [`ContainerReceiver`] (#1263), as for `ListAppend`.
    ListPop {
        list: ContainerReceiver,
    },
    /// `dict.get(key, default)` (PR-12, D-119): exactly two arguments --
    /// returns `default` if `key` is absent from the dict, else the stored
    /// value. Mirrors `ListAppend`'s hand-recognized shape structurally, but
    /// -- like `ListPop` above -- is value-producing (the dict's value
    /// type), not `None`-producing. The zero/one-argument form CPython also
    /// supports (returning `None` on a missing key) is deliberately not
    /// shipped: this compiler has no `Optional[int]`/`None`-union
    /// representation for a `dict[str, int]`'s value type, so requiring the
    /// caller to always supply a same-typed default sidesteps that gap
    /// entirely rather than half-solving it. Shares `ListPop`'s own
    /// pre-existing D-116 solver-binding caveat above verbatim (also
    /// `Ok(None)` in the solver, also scalar-valued, also no longer
    /// misleading downstream per issue #771/D-199).
    ///
    /// `dict` is a [`ContainerReceiver`] (#1263), as for `ListAppend`.
    DictGetOrDefault {
        dict: ContainerReceiver,
        key: Box<HirExpr>,
        default: Box<HirExpr>,
    },
    /// `set.add(value)` (PR-12, D-119): mirrors `ListAppend`'s shape and
    /// `None`-producing behavior exactly. D-131 gives `y = s.add(1)` the
    /// same canonical `None` assignment storage as `.append()`; D-072's
    /// remaining nested-expression rejection is specific to `print()`'s
    /// own result. Insertion deduplicates exactly like set-literal
    /// construction already does (`pycc_rt_int_set_add`, already shipped by
    /// PR-11a), from a second, user-facing call site added by a later task.
    SetAdd {
        set: String,
        value: Box<HirExpr>,
    },
    /// `base.attr` (D-154, Part 1 of #375): a read of an instance attribute
    /// (or, structurally, any attribute-shaped expression -- `base` is
    /// lowered generically, since `lower_expr` has no type information to
    /// narrow it at this stage). Distinct from the `Expr::Attribute` arm's
    /// other lowering target, `HirExpr::Name("module.symbol")`: that form
    /// only fires when the receiver is a bare name that resolves against
    /// `pycc_std`'s stdlib module registry (a compile-time textual match,
    /// D-136/D-137); every other attribute-access shape -- including `self`
    /// and any other class-instance-typed receiver -- falls through to this
    /// variant instead. `pycc_types` rejects it when `base`'s inferred type
    /// is not `Ty::Instance` or `attr` does not name one of that class's
    /// declared attribute slots; `pycc_mir`/`pycc_codegen` resolve `attr` to
    /// a compile-time slot index via the class's `HirClassDef` (never a
    /// runtime string-keyed lookup, per the class-instance-layout ADR).
    AttrGet {
        base: Box<HirExpr>,
        attr: String,
    },
    /// `base.method(args)` (D-154, Part 1 of #375): a call shaped like an
    /// attribute access followed by a call, structurally recognized for any
    /// receiver that isn't one of the hand-recognized container methods
    /// (`.append()`/`.pop()`/`.get()`/`.add()`, tried first, see
    /// `lower_expr`'s `Expr::Call` arm) or a resolved stdlib module symbol
    /// call (`math.sqrt(x)`, also tried first). `self.foo()` and
    /// `some_instance.foo()` both lower to this same shape -- there is
    /// nothing at HIR-lowering time to distinguish them, since both are
    /// just "call a method named `foo` on whatever `base` evaluates to".
    /// `pycc_types` resolves `base`'s inferred type to a `Ty::Instance`,
    /// looks `method` up in that class's method table, and checks the call
    /// against the method's real signature; `pycc_mir`/`pycc_codegen`
    /// resolve the call to the method's compile-time-known mangled function
    /// symbol (`<ClassName>.<method_name>`, D-006's static-dispatch framing
    /// for a non-inherited class -- there is no vtable or runtime dispatch
    /// in this PR).
    MethodCall {
        base: Box<HirExpr>,
        method: String,
        args: Vec<HirExpr>,
    },
    /// Issue #1188: `recv.<append|pop|get|add>(args)` in a module from which
    /// a user class defining a method of that name is reachable (see
    /// `SignatureTable`'s container-method gate). Both readings of the call
    /// are carried because only the receiver's static type, known from
    /// `pycc_types` on, decides between them ([`receiver_takes_method_path`]).
    /// `call` is always a [`HirExpr::MethodCall`]; `container` records what
    /// the container reading of the same call is. Never built in a module
    /// whose gate is off, so every such module lowers exactly as before.
    ReceiverDispatchedCall {
        call: Box<HirExpr>,
        container: ContainerFallback,
    },
    /// `C[type_arg](args)` (PEP 695, #387): instantiation of a generic class
    /// with an explicit type argument. `class` is the generic class's name,
    /// `type_arg` is the resolved concrete type (a scalar `Ty` — int/float/
    /// bool/str — resolved at HIR-lowering time from the subscript's slice
    /// expression), and `args` are the constructor arguments. Reuses PR-13's
    /// `Ty::Param` call-site-substitution mechanism (D-133/D-134): at
    /// monomorphization time, the class's methods are specialized with `T`
    /// substituted by `type_arg`, and this expression is rewritten into an
    /// ordinary `HirExpr::Call` to the specialized class's mangled name.
    GenericClassInstantiate {
        class: String,
        type_arg: Ty,
        args: Vec<HirExpr>,
    },
    /// Part 2a of #1371: a positional call whose callee is a subscript
    /// result, `table[k](args)`, when the subscript is not a generic class
    /// instantiation (`expr::subscript_call` owns the rule). `callee` is
    /// always a lowered [`HirExpr::Subscript`]. `pycc_types` admits it only
    /// when the callee is a CPython object (`Ty::Object`), and MIR lowers it
    /// to `MirExpr::ObjCall` with the subscript as the callee.
    ExprCall {
        callee: Box<HirExpr>,
        args: Vec<HirExpr>,
    },
    /// Part 8 of #1371: a call that passes keyword arguments and that
    /// the keyword binder (`expr::keyword_bind`) cannot bind at lowering time, kept whole
    /// because only the callee's static type decides whether pycc can
    /// compile it (`expr::object_keyword_call` owns the lowering rule).
    /// `call` is the positional part of the same call: a
    /// [`HirExpr::Call`] of a bare name, a [`HirExpr::MethodCall`] or a
    /// [`HirExpr::ExprCall`]. `keywords` holds each `name=value` pair in
    /// source order, after every positional argument, which is CPython's
    /// evaluation order. `pycc_types` admits the node only when the callee
    /// is a CPython object (`Ty::Object`) and otherwise reports the
    /// pre-Part-8 `C0001` "keyword call arguments are not supported yet" at
    /// `span`, the whole call's source range.
    KeywordCall {
        call: Box<HirExpr>,
        keywords: Vec<(String, HirExpr)>,
        span: Span,
    },
    /// #1411: `type(self)(args)` inside an instance method -- a construction
    /// of the receiver's own class. The class is not carried here: it is the
    /// static type of the canonical receiver `self` in the body being
    /// compiled, recovered by `pycc_types` and `pycc_mir` exactly as
    /// [`HirExpr::Super`] recovers its class. Because an inherited body that
    /// constructs through its receiver is compiled once more for every
    /// subclass with `self` retyped (D-254), each compilation builds its own
    /// receiver's class, as CPython's run-time `type(self)` does
    /// (`expr::receiver_class_call` owns the lowering rule).
    ReceiverClassCall {
        args: Vec<HirExpr>,
    },
    /// Zero-arg `super()` (PEP 3135, #433): represents the implicit
    /// `super(__class__, self)` reference available inside a method body.
    /// Only ever appears as the `base` of a `HirExpr::MethodCall` or
    /// `HirExpr::AttrGet` — a bare `super()` used as a standalone value is
    /// rejected at HIR-lowering time (it has no useful static-dispatch
    /// lowering on its own). The enclosing class name is NOT carried here:
    /// it is recovered downstream from the method's own mangled
    /// `<ClassName>.<method>` name (type checker) or from the MIR lowering
    /// context, the same way `self`'s own `Ty::Instance(class_name)` is
    /// already recovered. `super().method(args)` resolves at compile time to
    /// the next class after the current one in the MRO (D-006's static-
    /// dispatch framing — no vtable, no runtime dispatch, per the #433 ADR).
    Super,
    /// `target := value` (PEP 572, #774): the walrus assignment expression.
    /// CPython's own grammar restricts `target` to a single bare identifier
    /// (`Expr::Named`'s target is never a tuple/attribute/subscript, unlike
    /// `Stmt::Assign`'s multi-shape target) -- `lower_expr`'s own
    /// `Expr::Named` arm enforces that restriction defensively even though
    /// no real parse should ever violate it. Evaluating this expression
    /// binds `name` in the *enclosing function or module scope* (PEP 572's
    /// headline scoping rule -- a walrus never creates its own scope, and
    /// even skips a comprehension's scope, unlike a `for`-target) to
    /// `value`'s own value, and the expression's own result is that same
    /// value -- mirroring an assignment statement's binding side effect
    /// while still yielding a value the way every other `HirExpr` does.
    /// `pycc_types::infer_expr_in`'s own `NamedExpr` arm reports `value`'s
    /// inferred type as this expression's type; the actual binding into the
    /// enclosing scope's `Environment` happens one level up, at the
    /// statement boundary (`pycc_types::collect_named_expr_bindings`),
    /// because `infer_expr_in` itself only ever holds a shared `&Environment`
    /// reference. Comprehension-embedded walrus scoping (binding into the
    /// scope *outside* the comprehension) is out of scope for #774 -- see
    /// that issue's own scope-cut note and the corresponding manifest entry.
    NamedExpr {
        name: String,
        value: Box<HirExpr>,
    },
    /// A list, set or dict comprehension in expression position (#1254,
    /// D-250): a call argument, a `return` value, an operand, an `if`/
    /// `while` test, an f-string interpolation. Its loop variable is scoped
    /// to this node -- `var` is the D-117 synthesized name, bound only while
    /// `cond` and the element expressions are checked and evaluated, and
    /// never a binding of the enclosing scope. `name = <comp>` keeps lowering
    /// to the statement forms (`HirStmt::ListCompAssign` and its siblings).
    /// Boxed: the node is large and rare.
    Comprehension(Box<HirComprehension>),
    /// A typed numpy array operation (`crate::ndarray`): an element read
    /// or write, `.shape`, `.size`, `.ndim`, `len()`, or the run-time check
    /// that a value is an array of the static dtype and dimension count.
    /// Built only by `crate::ndarray::lower` once the program is linked;
    /// `base` is always an object-typed expression (the array).
    Nd(Box<crate::ndarray::NdExpr>),
}

/// The parts of a [`HirExpr::Comprehension`] (#1254, D-250). `var` is the
/// synthesized loop-variable name, already substituted for the source name
/// throughout `cond` and `elt` (and in a nested comprehension's own
/// iterable); `iter` is evaluated in the enclosing scope and is not renamed.
#[derive(Debug, Clone, PartialEq)]
pub struct HirComprehension {
    pub var: String,
    pub iter: CompIter,
    pub cond: Option<HirExpr>,
    pub elt: CompElt,
}

impl HirComprehension {
    /// Every sub-expression, in evaluation order: the range operands or an
    /// iterable expression (in the enclosing scope), then `cond`, then the element expressions (a
    /// dict's key before its value). For walkers that only need to visit
    /// each sub-expression once, whatever scope it is evaluated in.
    pub fn sub_exprs(&self) -> Vec<&HirExpr> {
        let mut out = Vec::new();
        match &self.iter {
            CompIter::Range { start, stop, step } => out.extend([start, stop, step]),
            CompIter::Iterable(iterable) => out.push(iterable),
            CompIter::Name(_) => {}
        }
        out.extend(self.body_exprs());
        out
    }

    /// The per-iteration sub-expressions, in evaluation order: `cond`, then
    /// the element expressions. These are the ones evaluated with `var`
    /// bound.
    pub fn body_exprs(&self) -> Vec<&HirExpr> {
        let mut out: Vec<&HirExpr> = self.cond.iter().collect();
        match &self.elt {
            CompElt::List(e) | CompElt::Set(e) => out.push(e),
            CompElt::Dict { key, value } => out.extend([key, value]),
        }
        out
    }

    /// [`HirComprehension::body_exprs`], mutably, for rewriting passes.
    pub fn body_exprs_mut(&mut self) -> Vec<&mut HirExpr> {
        let mut out: Vec<&mut HirExpr> = self.cond.iter_mut().collect();
        match &mut self.elt {
            CompElt::List(e) | CompElt::Set(e) => out.push(e),
            CompElt::Dict { key, value } => out.extend([key, value]),
        }
        out
    }
}

/// A comprehension's element expressions, by kind (#1254).
#[derive(Debug, Clone, PartialEq)]
pub enum CompElt {
    /// `[elt for ...]`, producing `list[int]`.
    List(HirExpr),
    /// `{elt for ...}`, producing `set[int]`.
    Set(HirExpr),
    /// `{key: value for ...}`, producing `dict[str, int]`.
    Dict { key: HirExpr, value: HirExpr },
}

/// What the container reading of a [`HirExpr::ReceiverDispatchedCall`] is
/// (issue #1188).
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerFallback {
    /// The container lowering succeeded. Its node is derived from `call` on
    /// demand by [`HirExpr::container_form`] and never stored, so no HIR
    /// rewriting pass can leave a stale copy behind.
    Admitted,
    /// The container lowering failed with exactly this diagnostic (a
    /// non-bare-name receiver, a wrong arity, or a boundary literal).
    Refused(Box<Diagnostic>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Literal(String),
    Interpolation(Box<HirExpr>),
}

/// A comprehension's iterable source (PR-12, D-117): reuses
/// `HirStmt::ForList`'s own iterable polymorphism verbatim (a bare name is
/// resolved to `Ty::List`/`Ty::Dict`/`Ty::Set` downstream by
/// `pycc_types`/`pycc_mir`, exactly like a plain `for` loop) rather than
/// inventing a narrower, comprehension-specific iterable gate. Shared by
/// the `*CompAssign` statements and [`HirComprehension`] (#1254).
#[derive(Debug, Clone, PartialEq)]
pub enum CompIter {
    Range {
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,
    },
    Name(String),
    /// Any other iterable expression (Part 1 of #1255), such as
    /// `o.keys()` or `d[k].values()`, lowered in the enclosing scope. Only
    /// a CPython object (D-258) is iterable this way: `pycc_types` refuses
    /// every other type, and the comprehension then produces a CPython
    /// `list` or `set` object. A comprehension's statement form with this
    /// source lowers to a plain `HirStmt::Assign` of a
    /// [`HirExpr::Comprehension`], not to a `*CompAssign` statement.
    Iterable(Box<HirExpr>),
}
