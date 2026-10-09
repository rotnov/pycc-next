use pycc_diag::{Diagnostic, Span};

mod boolop;
mod buffer_store;
mod class;
mod compare_chain;
mod container;
mod dunder_name;
mod exception;
mod expr;
mod func;
mod hir_module;
mod if_exp;
mod import;
mod int_boundary;
mod isinstance_narrow;
mod module;
mod not_implemented;
mod post_if;
mod program;
mod return_coverage;
mod stmt;
mod typecheck;

pub use boolop::{BoolOpKind, bool_op_result_ty};
pub use buffer_store::{body_returns_inside_finally, body_returns_slice_of, body_stores_into};
pub use class::enum_call::enum_class_call_message;
pub use class::{
    ClassAttrValue, ClassNamespaceWinner, CopiedMemberKind, EnumMemberValue, EqRefusal,
    ForeignCallableRef, HashRefusal, HirClassDef, InheritedCopy, InstanceEq, InstanceHash,
    InstanceHashLowering, PropertyDef, ProtocolMember, SUPER_TARGET_MARKER, binds_member,
    class_name_foreign_static, class_namespace_winner, dataclass_repr_body,
    declares_name_outside_class_attrs, first_definer, flat_attr_layout, inherited_copy_name,
    inherited_copy_origin, instance_foreign_static, method_shadows_foreign_static,
    mro_has_instance_slot, resolve_instance_eq, resolve_instance_hash,
    slot_behind_foreign_static_meets_property, subclass_divergence,
};
pub use compare_chain::{CompareLink, compare_chain_operands};
pub use container::{check_container_ty, check_tuple_element_ty, set_element_message};
pub use dunder_name::DUNDER_NAME;
pub use exception::{
    BUILTIN_EXCEPTION_CLASSES, EXCEPTION_GROUP_TYPE_TAG, EXCEPTION_INIT_MANGLED_NAME,
    FIRST_USER_EXCEPTION_TYPE_TAG, FOREIGN_BASE_EXCEPTION_TYPE_TAG, HirExceptHandler,
    MAX_USER_EXCEPTION_CLASSES, builtin_exception_class_defs, builtin_exception_init_item,
    builtin_exception_parent, except_handler_binding_type_name, is_builtin_exception_class,
    is_flat_builtin_exception_class,
};
pub use expr::object_keyword_call::KEYWORD_CALL_UNSUPPORTED;
pub use expr::receiver_takes_method_path;
pub(crate) use func::{
    annotation_to_ty, lower_arg_list, lower_function, lower_return_annotation, type_param_name,
    with_bare_container_advice,
};
pub use hir_module::{
    ForeignImportSite, FromImport, HirModule, ImportBinding, ProjectBindingKind,
    foreign_binds_root, foreign_bound_module, foreign_bound_object, foreign_import_statement,
    killed_names, opens_foreign_statement, top_level_bound_names,
};
pub use if_exp::if_exp_result_ty;
pub use import::{
    ProjectImportRequest, ResolvedImport, ResolvedImports, ResolvedModule, is_native_std_module,
    project_import_requests,
};
pub(crate) use import::{
    import_local_name, lower_import_stmt, lower_legacy_type_alias_ann_assign, lower_type_alias_stmt,
};
pub use isinstance_narrow::{IsInstancePolarity, isinstance_narrow_target, isinstance_test};
pub use module::{LoweredModule, lower_all, lower_checked, lower_module};
pub use not_implemented::{WIDENED_RETURN_HELP, body_returns_not_implemented};
pub use post_if::continuation_narrows;
pub use program::{LinkInput, finalize, link};
pub use return_coverage::block_always_returns;
pub use stmt::del::{deleted_names, mentioned_names};
pub use typecheck::{
    ExtractClassNamesError, eval_isinstance_single, eval_issubclass_single, extract_class_names,
    is_abc_base_name, is_builtin_type_name, is_enum_base_name, is_protocol_base_name,
};

/// D-038's visibility predicate: a name is public unless it begins with an
/// underscore.
///
/// One canonical home for a rule four call sites used to spell inline
/// (`func.rs`, `class.rs`, and twice in `class/protocol.rs`). It is `pub`
/// because the rule now has a consumer outside HIR lowering as well: D-244
/// rule 1 exports *every public module-level function* of an `--ext` build,
/// and a public `@staticmethod` or `@classmethod` of a public class, and
/// the driver derives that export set from the same predicate the
/// frontend's own `T0001` annotation requirement uses -- applied
/// unchanged to the class name and the method name alike, never forked --
/// so "public" can only ever mean one thing across the compiler.
///
/// The name is the whole input on purpose. This is a *visibility* predicate
/// over a source-level identifier, unrelated to `pycc_types`'
/// `is_private_solver_scalar`, which is a `Ty`-shape predicate that happens
/// to carry "private" in its name.
#[must_use]
pub fn is_public_name(name: &str) -> bool {
    !name.starts_with('_')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    None,
    Infer,
    /// A generic function's own type parameter (PEP 695, D-133), e.g. the `T`
    /// in `def f[T](x: T) -> T`. Distinct from `Ty::Infer`: this is resolved by
    /// call-site substitution (D-134), not by unification, and must never
    /// reach `pycc_mir` unsubstituted (same invariant `Ty::Infer` already
    /// holds, see the assertion near this enum's other pre-MIR checks).
    /// Boxed as `Box<String>`, not `Box<str>`: `str` is unsized, so
    /// `Box<str>` is itself a fat (16-byte, data-ptr + length) pointer,
    /// which measured `size_of::<Ty>() == 24` here -- it broke the
    /// niche-filling layout uniformity the other thin-pointer variants
    /// rely on (see `Tuple`'s doc comment above for the same phenomenon
    /// with `Box<[Ty]>` vs `Box<Vec<Ty>>`). `Box<String>` is a single
    /// (8-byte) thin pointer to a further heap-indirected `String` --
    /// confirmed by measurement to restore `size_of::<Ty>() == 16`,
    /// matching `Tuple`'s `Box<Vec<Ty>>` shape. (D-133's ADR text says
    /// `Box<str>`; this is a deliberate, measurement-driven deviation --
    /// `Box<str>` does not in fact keep `Ty` at the D-109 ceiling.)
    Param(Box<String>),
    /// `list[T]`. Type-checking is planned to accept any scalar `T`; only
    /// `T = Ty::Int` gets real codegen in v0.2 (D-105). Codegen rejecting
    /// every other element type before it becomes an unhandled codegen
    /// case is planned via a `pycc_types` diagnostic (`T0034`, per D-105 --
    /// not yet implemented as of this commit; this variant only defines
    /// the type representation).
    List(Box<Ty>),
    /// `dict[K, V]`. No v0.2 code path constructs this yet (PR-11's own
    /// scope per `docs/DELIVERY_PLAN.md`) -- the variant exists now only
    /// because D-089 decided `Ty`'s full recursive shape up front, so
    /// every later PR's match arms are additive, not migratory again.
    /// The key/value pair is boxed together as a single pointer (D-109):
    /// this variant was not itself the dominant contributor to `Ty`'s
    /// pre-fix 24-byte size (`Tuple(Vec<Ty>)` was), but once `Tuple`
    /// shrinks to a single 8-byte pointer too, `Dict`'s original
    /// two-separate-`Box<Ty>` shape (16 bytes) would become the new
    /// ceiling -- boxing it avoids that, not because `dict[K,V]` needs
    /// its own codegen yet.
    Dict(Box<(Ty, Ty)>),
    /// `set[T]`. Same status as `Dict` above -- PR-11's own scope.
    Set(Box<Ty>),
    /// `frozenset[T]` (Part 1 of #1319). The immutable sibling of `Set`:
    /// only `frozenset[int]` is compiled (D-122's one-combination gate,
    /// `T0038`), and it shares `set[int]`'s run-time representation
    /// (`PyIntSetObj`); immutability is enforced by `pycc_types` alone.
    /// `Box<Ty>` is a thin pointer, the same shape as `Set`, so this
    /// variant keeps `size_of::<Ty>()` at 16 bytes, inside the D-109
    /// ceiling (measured, and pinned by `ty_size_stays_within_d109_ceiling`).
    FrozenSet(Box<Ty>),
    /// `tuple[A, B, ...]`. Same status as `Dict` above -- PR-11's own
    /// scope. Boxed (D-109) as `Box<Vec<Ty>>` -- a second indirection: a
    /// thin (8-byte) pointer to a heap-allocated `Vec<Ty>` -- not as
    /// `Box<[Ty]>` (a 16-byte fat pointer: data ptr + length), which was
    /// tried first and measured `size_of::<Ty>() == 24`, no reduction at
    /// all from the pre-fix size (confirmed independently in-crate and
    /// via a standalone `rustc` reproduction). `Box<Vec<Ty>>` measured
    /// `size_of::<Ty>() == 16` instead (`align_of::<Ty>()` stayed `8` in
    /// every configuration measured). The most plausible explanation is
    /// that rustc's niche-filling enum-layout optimization (the trick
    /// behind `size_of::<Option<Box<T>>>() == size_of::<Box<T>>()`) needs
    /// every dataful variant to share a uniform pointer shape to collapse
    /// the discriminant for free -- `Box<[Ty]>`'s fat pointer broke that
    /// uniformity against `List`/`Dict`/`Set`'s thin ones, `Box<Vec<Ty>>`
    /// restores it -- but this project has not independently re-derived
    /// rustc's exact layout algorithm against every configuration (in
    /// particular, the pre-fix shape already had non-uniform dataful
    /// variant sizes yet still measured 24, not the 32 bytes a naive
    /// "tag plus largest payload" rule would predict). Treat the measured
    /// numbers above as the authoritative facts and this paragraph's
    /// mechanism as a plausible, not proven, explanation.
    Tuple(Box<Vec<Ty>>),
    /// A single-class instance (no inheritance in this PR -- see D-154,
    /// "class instance layout"). Carries only the originating class's name,
    /// not its shape: attribute-name -> slot-index mapping and method
    /// signatures live in a module-level side table
    /// (`HirModule::class_defs`, `pycc_hir::class::HirClassDef`), the same
    /// shape `type_aliases`/`imports` already use for compile-time-only
    /// side information that isn't part of a `HirItem`/`HirStmt` value
    /// itself. Boxed as `Box<String>`, not `Box<str>`, mirroring
    /// `Ty::Param`'s own documented reasoning exactly: `Box<str>` is a fat
    /// (16-byte) pointer because `str` is unsized, which would break the
    /// niche-filling layout uniformity every other dataful `Ty` variant
    /// relies on to stay at a single thin (8-byte) pointer; `Box<String>`
    /// is a single thin pointer to a further heap-indirected `String`, so
    /// this variant does not move `size_of::<Ty>()` past the D-109 16-byte
    /// ceiling (see `ty_size_stays_within_d109_ceiling` below).
    Instance(Box<String>),
    /// A protocol-typed value (PEP 544, #380, PR-20). Carries the protocol
    /// class name. Distinct from `Ty::Instance` so that `is_assignable` and
    /// every match arm can distinguish "this value is an instance of a
    /// concrete class" from "this value is typed as a protocol." A
    /// protocol-typed value's concrete representation is determined by the
    /// first assignment (D-040 sticky-representation rule) or by
    /// monomorphization at each call site (D-006/D-134). Boxed as
    /// `Box<String>`, mirroring `Ty::Instance`'s own documented reasoning
    /// exactly — maintains the D-109 16-byte `size_of::<Ty>()` ceiling.
    Protocol(Box<String>),
    /// `T | None` (PEP 604, D-197), scoped in this PR to `Optional[Ty::Int]`
    /// for codegen (see `crates/pycc_codegen`'s niche-optimized `int | None`
    /// representation) -- every other inner type type-checks but is
    /// rejected pre-lowering by a dedicated diagnostic. A general
    /// `Ty::Union(Vec<Ty>)` was considered and rejected for this PR: Python's
    /// `X | Y | None` general union has no agreed-on runtime representation
    /// here yet, while `Optional[T]` alone has a well-known niche-filling
    /// encoding (mirroring `Option<Box<T>>` in Rust itself). Boxed as
    /// `Box<Ty>`, the same thin-pointer shape as `List`/`Set`, so this
    /// variant does not move `size_of::<Ty>()` past the D-109 16-byte
    /// ceiling.
    Optional(Box<Ty>),
    /// An opaque CPython object (Part 1 of #1026): the value a foreign
    /// `import numpy` binds. Carries no shape at all -- pycc knows only
    /// that the value is a `PyObject *` owned by the CPython runtime.
    /// `pycc_types` admits only the operations lowered through the CPython C
    /// API (`docs/TYPE_SYSTEM.md`'s `object` row) and refuses every other one
    /// with `I0404`. In a `native` build `object` itself is unspellable in
    /// an annotation: `annotation_to_ty` rejects it with `C0001`, and `Any`
    /// with `T0002`. In every module of a `pycc build --ext` artifact (D-258,
    /// #1397) `typing.Any`, the builtin `object`, a bare
    /// `list`/`dict`/`tuple`/`set` and a container with an object argument
    /// all spell this variant, and a public signature carries it across the
    /// export boundary as the `PyObject *` itself. Since Part 1 of #1367 the name of a
    /// class a foreign import binds resolves to this variant in an
    /// annotation in either mode (`docs/TYPE_SYSTEM.md`'s `object` row). The
    /// variant is deliberately unit-shaped.
    /// A unit variant adds no payload, so `size_of::<Ty>()` stays at the
    /// D-109 16-byte ceiling.
    Object,
    /// A one-dimensional buffer over contiguous `float` (C `double`)
    /// elements, admitted only at a `pycc build --ext` boundary (Part 1 of
    /// #1027, D-244). Carries no shape: what the boundary guarantees is
    /// exactly what the four refusal arms in `src/ext_build.rs` check at
    /// run time -- exports a buffer, C-contiguous, `ndim == 1`, format
    /// `"d"` -- so no payload could add information the wrapper has not
    /// already proved. A unit variant adds no payload, so `size_of::<Ty>()`
    /// stays at the D-109 16-byte ceiling.
    ///
    /// Three source spellings lower to this one variant: `memoryview`,
    /// since #1129 `ndarray`, and since #1134 `NDArray`. They are one type
    /// because their run-time contract is identical -- the boundary admits
    /// any object exporting a conforming buffer, whichever way the parameter
    /// was spelled -- so diagnostics render the canonical `memoryview` for
    /// all three, exactly as they already do for an alias of it.
    MemoryView,
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Str => "str".to_string(),
            Ty::None => "None".to_string(),
            Ty::Infer => "<inferred>".to_string(),
            Ty::Param(name) => name.to_string(),
            Ty::List(elem) => format!("list[{}]", elem.name()),
            Ty::Dict(kv) => format!("dict[{}, {}]", kv.0.name(), kv.1.name()),
            Ty::Set(elem) => format!("set[{}]", elem.name()),
            Ty::FrozenSet(elem) => format!("frozenset[{}]", elem.name()),
            Ty::Tuple(elems) => format!(
                "tuple[{}]",
                elems.iter().map(Ty::name).collect::<Vec<_>>().join(", ")
            ),
            Ty::Instance(class_name) => class_name.to_string(),
            Ty::Protocol(name) => name.to_string(),
            Ty::Optional(inner) => format!("{} | None", inner.name()),
            Ty::Object => "object".to_string(),
            Ty::MemoryView => "memoryview".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOpKind {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    /// `is` (D-197, #763, Part 1 of #747). This compiler's first `is`/`is
    /// not` support of any kind -- deliberately scoped at HIR-lowering to
    /// exactly the case #763 needs: one operand is syntactically
    /// `Expr::NoneLiteral`, the other anything. Since Part 1 of #1371 a
    /// single comparison between two non-literal operands lowers too, and
    /// `pycc_types` admits it only between two CPython objects; every other
    /// pair keeps the `C0001` "comparison operator not supported yet"
    /// rejection (see `crate::compare_chain::lower_cmp_op`). Since Part 11
    /// of #1371 a chain link between two non-literal operands lowers the
    /// same way, and `pycc_types` refuses the chain: the object chain's
    /// `I0404` when an operand is an object, that `C0001` otherwise.
    Is,
    /// `is not`. Same scoping as `Is` above.
    IsNot,
    /// `item in container` (Part 2b of #1371). Lowered only for a single
    /// comparison whose right operand is not syntactically a literal or a
    /// display (see `crate::compare_chain::lower_cmp_op`); `pycc_types`
    /// admits it only with a CPython-object container and refuses every
    /// other pair with the same `C0001` "comparison operator not supported
    /// yet" message. A chain keeps that `C0001` here.
    In,
    /// `item not in container`. Same scoping as `In` above.
    NotIn,
}

/// The unary operators this compiler supports (#603 Part 2, #604 Part 3, of
/// #573).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    /// `-x`.
    USub,
    /// `+x`. Not a no-op: `+True` is the integer `1` in Python, so a
    /// `bool` operand crosses into `int` here (see
    /// `pycc_types::unop::unary_result_type`).
    UAdd,
    /// `not x`. Defined by truthiness: the result is always `bool`, for
    /// every operand type this compiler can evaluate a truth value for
    /// (#604, Part 3 of #573).
    Not,
    /// `~x`. Bitwise complement, defined only for `int` (`bool` included,
    /// since it is a numeric subtype); `~x == -x - 1` (#604, Part 3 of
    /// #573).
    Invert,
}

impl UnaryOpKind {
    /// The operator's Python source spelling, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOpKind::USub => "-",
            UnaryOpKind::UAdd => "+",
            UnaryOpKind::Not => "not",
            UnaryOpKind::Invert => "~",
        }
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    ExprStmt(HirExpr),
    Assign {
        target: String,
        value: HirExpr,
    },
    AnnAssign {
        target: String,
        annotation: Ty,
        value: Option<HirExpr>,
        /// PEP 591 (#383): `true` when the original annotation was
        /// `Final[X]`, meaning this binding may not be reassigned after its
        /// initial assignment. Set at HIR-lowering time in `stmt.rs`'s
        /// `Stmt::AnnAssign` arm by inspecting the raw AST annotation
        /// before `annotation_to_ty` unwraps `Final[X]` to `X`. The type
        /// checker's `Environment.finals` set is populated from this flag,
        /// and `check_assignment` rejects a reassignment of a `Final` name
        /// with `T0045`.
        is_final: bool,
    },
    If {
        test: HirExpr,
        body: Vec<HirStmt>,
        orelse: Vec<HirStmt>,
    },
    While {
        test: HirExpr,
        body: Vec<HirStmt>,
    },
    ForRange {
        var: String,
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,
        body: Vec<HirStmt>,
    },
    /// `for var in list:`, parallel to the existing `ForRange` -- desugars
    /// to an index-counted loop starting in a later PR-10 task (D-105),
    /// not here. Also reused, unconditionally, for `for k in <bare-name
    /// dict>:` (PR-11 Task 3, D-123: iterates keys in insertion order) --
    /// this lowering step has no type information to distinguish a
    /// `list`-typed iterable from a `dict`-typed one, so `pycc_types`'
    /// own `ForList` arms resolve `list`'s real type to `Ty::List`,
    /// `Ty::Dict` (binding `var` as the key type), or reject it, not here.
    ForList {
        var: String,
        list: String,
        body: Vec<HirStmt>,
    },
    /// `for var in <expression>:` where the expression is syntactically an
    /// attribute load (`o.attr`) or a method call (`o.method(...)`), the two
    /// shapes that can produce a foreign `object` value under D-244's `ext`
    /// artifact mode (Part 3 of #1026, PR 3c of #1082). HIR carries no
    /// types, so this node records only that syntactic shape: `pycc_types`
    /// resolves `iter` and refuses it with `I0404` unless it is `Ty::Object`
    /// -- `for x in xs.copy():` over a `list` lowers here too and is
    /// rejected there, not here. Unlike `ForList`, the iterable is a full
    /// `HirExpr` rather than a name (D-105's `String` field cannot carry
    /// `o.attr`), so every pass that walks statement sub-expressions must
    /// walk `iter` as well as `body`.
    ForObject {
        var: String,
        iter: Box<HirExpr>,
        body: Vec<HirStmt>,
    },
    /// `<bare name>[key] = value`, PR-11 Task 3 (D-123 supersedes D-105's
    /// "no subscript assignment target anywhere in this file" consequence
    /// for `list[int]`; see `Stmt::Assign`'s own lowering arm below). `dict`
    /// is carried as a plain variable name, exactly like `ForList`'s `list`
    /// field and `ListAppend`'s `list` field -- this lowering step has no
    /// type information, so a `list[int]` target also lowers to this node
    /// today (`pycc_types` rejects it with `T0033`, mirroring how
    /// `ForList`'s own `list` field is resolved to `Ty::List`, `Ty::Dict`,
    /// or rejected downstream, not here). Since Part 1 of #1142 a
    /// `memoryview` target lowers here too: `pycc_types` admits it as a
    /// buffer element store and `pycc_mir` lowers *that* one to
    /// `MirStmt::BufferSet` instead of `MirStmt::DictSet`, so this node
    /// stays the single type-blind shape for every `<name>[k] = v`.
    DictSet {
        dict: String,
        key: HirExpr,
        value: HirExpr,
    },
    /// `target = [elt for var in iter [if cond]]` (PR-12, D-117). Scoped to
    /// exactly one `for` clause and at most one `if` filter; only lowered
    /// when the comprehension is the direct RHS of a bare-name
    /// `Stmt::Assign` (see that arm's own handling below). Anywhere else a
    /// comprehension appears it lowers to [`HirExpr::Comprehension`] (#1254,
    /// D-250); this statement form is built from that same
    /// [`HirComprehension`] by `comp_assign_stmt`. `var` is already the
    /// D-117 synthesized internal name, not the source spelling -- every
    /// occurrence of the source name inside `cond`/`elt` has already been
    /// rewritten by `rename_name_in_expr` before this node is constructed,
    /// so downstream crates never see the user's own loop-variable
    /// spelling at all.
    ListCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        elt: Box<HirExpr>,
    },
    /// `target = {key: value for var in iter [if cond]}` (PR-12, D-117).
    /// Mirrors `ListCompAssign` exactly except for the key/value split
    /// (Python's dict-comprehension grammar has no direct list-comp analog
    /// of a single `elt`).
    DictCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        key: Box<HirExpr>,
        value: Box<HirExpr>,
    },
    /// `target = {elt for var in iter [if cond]}` (PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly -- a set comprehension's own shape is
    /// identical to a list comprehension's, differing only in which
    /// runtime constructor/insert pair `pycc_codegen` ends up calling.
    SetCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        elt: Box<HirExpr>,
    },
    Return(Option<HirExpr>),
    /// `base.attr = value` (D-154, Part 1 of #375). Structurally recognized
    /// for any assignment target shaped like an attribute access -- `base`
    /// is lowered generically, exactly like `HirExpr::AttrGet`'s own `base`,
    /// and for the same reason (no type information is available at this
    /// lowering step to narrow it to only `self` or only an instance-typed
    /// receiver). No slot `Ty` is carried here: `pycc_types` resolves
    /// `attr`'s declared type from the class's `HirModule::class_defs` entry
    /// (populated by `class::lower_class`'s own `__init__` pre-scan) and
    /// checks `value` against it; `pycc_mir`/`pycc_codegen` resolve `attr`
    /// to a compile-time slot index the same way. This single shape covers
    /// both an attribute's first (slot-establishing) assignment inside
    /// `__init__` and every later reassignment inside any method body --
    /// the two are distinguished only by `class::lower_class`'s own
    /// pre-scan, not by this node's own shape.
    AttrSet {
        base: HirExpr,
        attr: String,
        value: HirExpr,
    },
    /// PEP 634-636 (#381, PR-21): structural pattern matching. The subject is
    /// evaluated once and matched against each case in order; the first
    /// matching case's body executes. See `HirPattern`/`HirMatchCase` for
    /// the per-case shape.
    Match {
        subject: HirExpr,
        cases: Vec<HirMatchCase>,
    },
    /// `try`/`except`/`else`/`finally` (PEP 3110, #382, PR-22 Part 1).
    /// The body is executed; if an exception is raised, each handler is
    /// checked in order. The `orelse` runs only if no exception was raised.
    /// The `finalbody` always runs, whether the try completed normally, an
    /// exception was caught, or an exception is propagating.
    Try {
        body: Vec<HirStmt>,
        handlers: Vec<HirExceptHandler>,
        orelse: Vec<HirStmt>,
        finalbody: Vec<HirStmt>,
    },
    /// `try: ... except* T: ...` (PEP 654, Part 3 of #382, #542). Shares
    /// `HirExceptHandler`'s shape with plain `Try` -- the grammar node is
    /// identical, only the `is_star` flag on the AST differs -- but is a
    /// distinct variant rather than an `is_star: bool` field on `Try`
    /// because the two forms cannot mix (`except`/`except*` may not appear
    /// in the same `try`, enforced at parse time) and every downstream
    /// consumer (MIR lowering, `block_always_terminates`, `pycc_types`)
    /// needs its own dispatch anyway: `except*` always binds `as e` to an
    /// `ExceptionGroup`, never to the named type itself, and lowers to a
    /// partition-based dispatch rather than mutually exclusive `type_matches`
    /// branches. A bare `except*:` has no `HirExceptHandler` counterpart --
    /// PEP 654 requires every `except*` clause to name a type -- so
    /// `handlers[i].exc_type` is never `None` here: a bare `except*:` is
    /// rejected by ruff's own parser as a syntax error (`L0001`) before HIR
    /// lowering ever runs.
    TryStar {
        body: Vec<HirStmt>,
        handlers: Vec<HirExceptHandler>,
        orelse: Vec<HirStmt>,
        finalbody: Vec<HirStmt>,
    },
    /// `raise` (PEP 3110, #382). `exc` is the exception expression to raise;
    /// `None` means a bare re-raise (only valid inside an except handler).
    /// `cause` is the optional `raise ... from cause` expression (PEP 409).
    Raise {
        exc: Option<HirExpr>,
        cause: Option<HirExpr>,
    },
    /// `del name` (#1244, Part 1 of #1216). One statement per deleted name:
    /// `del a, (b, c)` is lowered left to right into three of these by
    /// `stmt::del::lower_delete`. The statement has no runtime effect -- the
    /// memory model is leak-only (D-124), so nothing is released -- and exists
    /// so the type checker can unbind `name`; every read that CPython would
    /// answer with `NameError`/`UnboundLocalError` is refused statically.
    /// `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the canonical
    /// statement of the rule.
    Delete {
        name: String,
    },
    /// `del base[start:stop:step]`, any bound omitted (Part 2c of #1371),
    /// one per slice target of a `del` statement, in the statement's
    /// left-to-right target order. Unlike [`HirStmt::Delete`] it unbinds no
    /// name: it *reads* `base` and every present bound, and at run time asks
    /// the base to delete the slice. `pycc_hir` cannot see types, so every
    /// slice target lowers here; `pycc_types` admits only a CPython-object
    /// base (`PyObject_DelItem` with a `slice` key, `foreign::slice`) and
    /// refuses every other base with this node's `span` -- the slice
    /// target's own range -- so the native refusal stays located.
    /// `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the canonical
    /// statement of the rule.
    DeleteSlice {
        base: Box<HirExpr>,
        start: Option<Box<HirExpr>>,
        stop: Option<Box<HirExpr>>,
        step: Option<Box<HirExpr>>,
        span: Span,
    },
    /// `del base.attr` (Part 2 of #1443, #1457), one per attribute target of
    /// a `del` statement, in the statement's left-to-right target order. Like
    /// [`HirStmt::DeleteSlice`] it unbinds no name: it *reads* `base` and at
    /// run time asks it to delete `attr`. `pycc_hir` cannot see types, so
    /// every attribute target lowers here; `pycc_types` admits only a
    /// CPython-object base (`PyObject_DelAttr`, `foreign::attr_store`) and
    /// refuses every other base with this node's `span` -- the attribute
    /// target's own range. `docs/TYPE_SYSTEM.md`'s "`del` statement" section
    /// is the canonical statement of the rule.
    DeleteAttr {
        base: Box<HirExpr>,
        attr: String,
        span: Span,
    },
    /// A CPython-backed `import X` or `import X as Y` (Part 1 of #1282,
    /// #1291), or `from X import a, b` (#1383), nested in a module-level
    /// `if`/`try` block. Each `(local_name, module_path, from)` entry, in
    /// source order, binds `local_name` at this statement's own position,
    /// so the import runs only when control reaches it: to the opaque
    /// CPython module object `pycc_ext_obj_import` returns for
    /// `module_path` when `from` is `None`, and to the attribute
    /// [`FromImport::name`] of that module, imported with the statement's
    /// whole fromlist and level, when `from` is `Some`. Produced only when
    /// every binding of the statement is foreign: each entry also sits in [`HirModule::imports`]
    /// as an [`ImportBinding::Foreign`] with [`ForeignImportSite::Block`]
    /// and this statement's `span`, which is how the driver's lock, policy
    /// and native-build gates see it. A top-level foreign import is never
    /// this node: it stays an [`ImportBinding::Foreign`] with
    /// [`ForeignImportSite::Item`], spliced into the module body by
    /// `pycc_mir`.
    ForeignImport {
        bindings: Vec<(String, String, Option<FromImport>)>,
        span: Span,
    },
}

/// Issue #769 (Part 2 of #747): the polarity of an `if`'s `name is [not]
/// None` test, as recognized by [`optional_none_test`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoneTestPolarity {
    /// `name is None` -- true in the `if` body means `name` is absent, so
    /// only the `orelse` branch can be narrowed to the Optional's inner
    /// type.
    Is,
    /// `name is not None` -- true in the `if` body means `name` is present,
    /// so only the `body` branch can be narrowed.
    IsNot,
}

/// Issue #769 (Part 2 of #747): recognizes an `HirStmt::If`'s `test` as a
/// top-level `name is None` / `name is not None` shape and extracts the
/// bare name and polarity, or `None` for every other test shape --
/// including a compound `and`/`or` test, a test where neither side is a
/// bare `HirExpr::Name`, or a test that is not an `is`/`is not` comparison
/// at all. Deliberately does not inspect the name's *type* -- callers
/// combine this syntactic recognition with their own scope's current type
/// for `name` (checker: `Environment`; MIR: the `scopes` stack) to decide
/// whether narrowing actually applies.
///
/// This is the single shared recognizer `pycc_types` (the checker's
/// narrowing overlay) and `pycc_mir` (the `OptionalUnwrap` lowering) both
/// call, so the syntactic pattern-match lives in exactly one place instead
/// of drifting between two independent copies -- `pycc_mir` does not
/// depend on `pycc_types` (see `crates/pycc_mir/Cargo.toml`), so this
/// recognizer lives here in `pycc_hir`, which both already depend on.
///
/// Since Part 1 of #1371 an `Is`/`IsNot` node need not have a `None`
/// operand (identity between two CPython objects), so this function
/// re-derives which side is which, and answers `None` for any other shape,
/// rather than assuming an operand order or a `None` side.
pub fn optional_none_test(test: &HirExpr) -> Option<(&str, NoneTestPolarity)> {
    let HirExpr::Compare { op, left, right } = test else {
        return None;
    };
    let polarity = match op {
        CmpOpKind::Is => NoneTestPolarity::Is,
        CmpOpKind::IsNot => NoneTestPolarity::IsNot,
        _ => return None,
    };
    let name_side = match (left.as_ref(), right.as_ref()) {
        (HirExpr::Name(name), HirExpr::NoneLiteral) => name,
        (HirExpr::NoneLiteral, HirExpr::Name(name)) => name,
        _ => return None,
    };
    Some((name_side.as_str(), polarity))
}

/// Issue #769 (Part 2 of #747): true only when `body`'s control flow
/// unconditionally terminates the enclosing function on every path through
/// it -- the strict predicate the early-return-narrows-the-continuation
/// shape needs (`if name is None: <body>` narrows every read after the
/// `if` only when `<body>` is provably inescapable).
///
/// Deliberately **not** the same thing as "a `return` occurs somewhere in
/// `body`" (an existing, separately-named unsound check elsewhere in this
/// codebase treats those as equivalent, which is wrong: `if flag: return
/// 0` contains a `return` but does not terminate on every path). This
/// predicate is true only when `body`'s *last* statement is itself
/// unconditionally terminating -- a bare `return`, or an `if` whose `body`
/// **and** non-empty `orelse` both recursively terminate. `raise` is
/// deliberately not a terminator (a program that raises still guarantees
/// the same soundness a `return` does, but correctly recognizing that
/// would additionally have to account for an enclosing `try`/`except`
/// catching it before it propagates out of the function -- out of scope
/// here), and `match` is not analyzed at all (also out of scope, kept
/// sound by omission rather than by an exhaustiveness heuristic that could
/// be wrong). Sound-by-omission for any statement shape not explicitly
/// handled below: an unhandled last statement makes the whole body report
/// `false`, never `true`.
///
/// Shared between `pycc_types::narrow` (the checker's own narrowing
/// overlay) and `pycc_mir` (its `OptionalUnwrap` lowering) for the
/// identical reason [`optional_none_test`] is shared: `pycc_mir` does not
/// depend on `pycc_types`, so the one predicate both need lives here, in
/// their common dependency, rather than as two independently-maintained
/// copies (D-205).
pub fn definitely_terminates(body: &[HirStmt]) -> bool {
    match body.last() {
        Some(HirStmt::Return(_)) => true,
        Some(HirStmt::If { body, orelse, .. }) => {
            !orelse.is_empty() && definitely_terminates(body) && definitely_terminates(orelse)
        }
        _ => false,
    }
}

/// PEP 634-636 (#381, PR-21): a single `match` case's pattern, lowered from
/// `ruff_python_ast::Pattern`. The type checker verifies each pattern against
/// the subject's known type and collects capture bindings; the MIR pass
/// desugars each pattern into a match condition plus binding assignments.
#[derive(Debug, Clone, PartialEq)]
pub enum HirPattern {
    /// `case _:` — matches anything, binds nothing.
    Wildcard,
    /// `case x:` — matches anything, binds `x` to the subject.
    Capture(String),
    /// `case 42:` / `case "hello":` / `case 3.14:` — a literal value pattern.
    Literal(HirExpr),
    /// `case True:` / `case False:` — a bool singleton pattern.
    Singleton(bool),
    /// `case None:` — the `None` singleton pattern.
    NoneSingleton,
    /// `case [a, b, c]:` — a fixed-length sequence pattern.
    Sequence(Vec<HirPattern>),
    /// `case [a, *rest]:` — a sequence pattern with a star-capture. The
    /// leading `Vec` are the fixed patterns before the star; `rest` captures
    /// the remaining elements as a list.
    SequenceStar(Vec<HirPattern>, Option<String>),
    /// `case {"k": v}:` — a mapping pattern. Each pair is `(key_expr,
    /// value_pattern)`; `rest` captures the remaining dict.
    Mapping(Vec<(HirExpr, HirPattern)>, Option<String>),
    /// `case Point(0, 0):` / `case Point(x=0, y=0):` — a class pattern.
    /// `positional` matches `__init__`'s parameters (after `self`);
    /// `keyword` matches named attributes.
    Class {
        class_name: String,
        positional: Vec<HirPattern>,
        keyword: Vec<(String, HirPattern)>,
    },
    /// `case 1 | 2 | 3:` — an or-pattern. All sub-patterns must bind the
    /// same set of capture names (PEP 634).
    Or(Vec<HirPattern>),
    /// `case [a, b] as pair:` — an as-pattern. The inner pattern must
    /// match, and `name` is bound to the subject.
    As(Box<HirPattern>, String),
}

/// PEP 634-636 (#381, PR-21): a single `case` clause in a `match`
/// statement.
#[derive(Debug, Clone, PartialEq)]
pub struct HirMatchCase {
    pub pattern: HirPattern,
    pub guard: Option<HirExpr>,
    pub body: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirItem {
    Function {
        name: String,
        params: Vec<(String, Ty)>,
        return_ty: Ty,
        body: Vec<HirStmt>,
    },
    TopLevelStmt(HirStmt),
}

fn unsupported<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    let message = message.into();
    // Issue #890: a `C0001` message must name the rejected construct in
    // Python terms (`pycc_ast::expr_kind_name`/`stmt_kind_name`), never
    // render an AST node's Rust `Debug` form. Every `ruff_python_ast`
    // node's `Debug` output contains `node_index: NodeIndex(`, so this
    // catches a reintroduced `{:?}` of a node in every test build. The
    // assertion is deliberately message-less: a messaged `debug_assert!`
    // leaves its never-formatted message literal as an uncovered region
    // under D-014's 100%-region gate, and this comment says why it fired.
    debug_assert!(!message.contains("NodeIndex("));
    Diagnostic::error("C0001", message, Span::new(range.start, range.end))
}

/// `C0002`: "stdlib symbol not supported yet" (D-136) -- distinct from
/// `C0001`. Used only when the *module* of a `from ... import ...`
/// statement is recognized but a specific imported name inside it is not
/// registered (e.g. `from math import isnan`), as opposed to `C0001`'s
/// "we don't recognize this import shape/module at all."
fn unresolved_symbol<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    Diagnostic::error("C0002", message, Span::new(range.start, range.end))
}

/// `L0001`: reused (not a new code, D-148) for a post-parse statement- or
/// expression-context violation caught during HIR lowering -- a construct
/// that is syntactically well-formed but only valid Python in a context the
/// enclosing statement or expression isn't in (`break`/`continue` outside a
/// loop, `async for` outside an async function -- D-148; `yield`/`yield
/// from` outside a function -- D-149). CPython itself raises `SyntaxError`
/// for all of these, the same failure class `L0001` already covers for the
/// parser's own grammar violations; unlike a genuine parser `L0001`, one
/// emitted from here carries no "expected set" in its message.
fn context_invalid<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    Diagnostic::error("L0001", message, Span::new(range.start, range.end))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pycc_parser_test_helper {
    pub fn parse(source: &str) -> pycc_ast::ModModule {
        pycc_parser::parse(source).expect("test fixture must parse")
    }
}
