//! Class-definition lowering (D-154, Part 1 of #375): `lower_class`; its
//! per-method helper `lower_method` lives in [`method`] and the
//! `__init__`-body attribute-slot pre-scan in [`init_slot`].
//!
//! A single, non-generic, non-inheriting class is represented with **no**
//! `HirItem` footprint of its own -- unlike a top-level `def`, `class Foo:
//! ...` produces no `HirItem::ClassDef` node. Instead:
//! - Each method (including `__init__`) lowers into an ordinary
//!   `HirItem::Function` under a mangled `<ClassName>.<method_name>` name
//!   (the `.` separator, not `__`, follows this crate's own existing
//!   `mangle_generic_instantiation`-adjacent precedent documented on
//!   `pycc_types::Environment`'s doc comment: a real Python `NAME` token can
//!   never contain a `.`, so this spelling can never collide with a real
//!   top-level `def`), with `self` as an implicit first parameter typed
//!   `Ty::Instance(Box::new(class_name))`. This means `pycc_types`'
//!   `functions` map, `pycc_mir::lower_item`, and `pycc_codegen`'s
//!   per-function LLVM symbol emission need no new function-shaped case at
//!   all to handle a method body -- only the existing one, plus the
//!   `self`/`Ty::Instance` parameter they already know how to type.
//! - The class's own shape (attribute slots, method table) is recorded in
//!   `HirModule::class_defs`, a module-level side table mirroring
//!   `type_aliases`/`imports`'s existing shape exactly: compile-time-only
//!   information with no `HirItem`/`HirStmt` footprint of its own.
//!
//! The rejected alternative -- a dedicated `HirItem::ClassDef` variant, with
//! method bodies held only inside `HirClassDef`'s own table -- was
//! considered and dropped: `pycc_mir::build`'s own two-pass dispatch
//! (`matches!(item, HirItem::TopLevelStmt(_))` then
//! `matches!(item, HirItem::Function { .. })`, each followed by an
//! `.expect("every HIR item is either a function or a top-level
//! statement")`) would need a third pass or predicate change, plus a new
//! `MirItem` variant and a `pycc_codegen` no-op arm for it -- all to carry
//! information the side-table shape already carries with zero additional
//! `HirItem`/`MirItem` surface and zero new coverage regions in either of
//! those two crates' own item-dispatch code.
//!
//! **Class-body statement execution** follows PR #358's redefinition-is-
//! rebind pattern, extended to class methods via mangled-name namespacing
//! (#386): a class body statement must be a `def` (a nested class, a bare
//! `pass`, a bare string-literal expression statement -- a docstring, #744,
//! accepted anywhere in the body, matching `validate_init_subclass_body`'s
//! own precedent -- a class-level attribute declaration, or any other
//! statement kind is `C0001`); redefining a non-`__init__` method name within
//! one class
//! body **rebinds** -- the second `def` replaces the method table entry, and
//! the latest definition is the one dispatched to at runtime (both
//! definitions share the same mangled `<ClassName>.<method>` name, so PR
//! #358's function-pointer slot infrastructure already handles the rebind:
//! the second `def`'s source-order execution stores the new function's
//! address into the slot, and later calls dispatch to it); redefining
//! `__init__` is still `C0001` (the compile-time attribute-slot pre-scan
//! `collect_init_attrs` cannot reconcile two different `__init__` bodies); a
//! class need not declare `__init__` -- when neither it nor any class in its
//! MRO provides one, [`init::ensure_init`] synthesizes an implicit
//! zero-argument constructor (#912, D-225), so the class is instantiable as
//! `C()` exactly as CPython's inherited `object.__init__` makes it. The
//! attribute-slot pre-scan
//! ([`init_slot`]) only looks at `__init__`'s own top-level body statements (no
//! recursion into a nested `if`/`while`/`for`), matching this same minimal,
//! single-pass scope.

mod attr_initializer;
mod attrs;
mod body;
mod declared_attrs;
pub(crate) mod enum_call;
mod enum_class;
mod exception_dunders;
pub(crate) mod generic_base;
mod inherited_copy;
mod method;
mod method_default_fill;
pub use inherited_copy::{
    CopiedMemberKind, InheritedCopy, SUPER_TARGET_MARKER, binds_member, first_definer,
    inherited_copy_name, inherited_copy_origin,
};
mod dataclass_methods;
pub use dataclass_methods::dataclass_repr_body;
use dataclass_methods::{synthesize_dataclass_eq, synthesize_dataclass_repr};
pub(crate) mod foreign_static;
pub use foreign_static::{
    ClassNamespaceWinner, ForeignCallableRef, class_name_foreign_static, class_namespace_winner,
    instance_foreign_static, method_shadows_foreign_static, mro_has_instance_slot,
    slot_behind_foreign_static_meets_property, subclass_divergence,
};
mod init;
mod init_slot;
mod instance_hash;
pub use instance_hash::{
    EqRefusal, HashRefusal, InstanceEq, InstanceHash, InstanceHashLowering, resolve_instance_eq,
    resolve_instance_hash,
};
mod mro;
pub use mro::flat_attr_layout;
mod protocol;
#[cfg(test)]
mod protocol_return_tests;
mod receiver;
mod reserved_names;
mod shadow;
pub(crate) mod slots;
pub use shadow::declares_name_outside_class_attrs;

use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExpr, HirItem, ImportBinding, Ty, unsupported};
use attrs::{
    ClassAttrCollisionInput, reject_class_attr_collisions,
    reject_dataclass_field_class_var_collisions,
};
use body::{ClassBodyInput, ClassBodyOutput, walk_class_body};
use enum_class::lower_enum_class;
use init::{ensure_init, synthesize_dataclass_init};
use init_slot::collect_init_attrs;
use method::lower_method;
use mro::{resolve_mro, validate_bases, validate_mro_slot_layout};
use protocol::lower_protocol_class;
use pycc_ast::{Decorator, Expr, Stmt};
use pycc_diag::{Diagnostic, Span};

/// PEP 435 (#892): the value assigned to one enum member. Members may be
/// `int`- or `str`-valued; `class/enum_class.rs` requires every member of
/// one class to use the same variant, so the variant in use also fixes that
/// class's `value` attribute type (`Ty::Int` or `Ty::Str`).
///
/// This lives here, next to `HirClassDef`, rather than in
/// `class/enum_class.rs`: it is part of `HirClassDef`'s own public shape,
/// read by `pycc_types`, `pycc_mir`, and `pycc_codegen`, while
/// `class/enum_class.rs` only populates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumMemberValue {
    /// An integer-valued member (`RED = 1`).
    Int(i64),
    /// A string-valued member (`RED = "red"`), from either a plain `Enum`
    /// whose members are strings or an `enum.StrEnum` subclass.
    Str(String),
}

/// The value of a class-level attribute (#911, Part 1 of #885).
///
/// A class attribute has **no runtime storage and no instance slot**. Every
/// read of a literal variant is folded to that literal by `pycc_mir`, and the
/// literal variants mirror exactly the scalar slot types the annotation is
/// restricted to (see [`HirClassDef::class_attrs`]), not the full expression
/// grammar. The one non-literal variant, [`ClassAttrValue::ForeignStatic`],
/// is not folded but re-evaluated at every access (#1345).
#[derive(Debug, Clone, PartialEq)]
pub enum ClassAttrValue {
    /// An `int`-typed attribute (`MIN_WIDTH: int = -1024`).
    Int(i64),
    /// A `float`-typed attribute (`SCALE: float = 1.5`).
    Float(f64),
    /// A `bool`-typed attribute (`DEBUG: bool = False`).
    Bool(bool),
    /// A `str`-typed attribute (`KIND: str = "window"`).
    Str(String),
    /// #1284 Part 1 (#1345): `name = staticmethod(<ref>)`, where `<ref>` is
    /// a bare name or a dotted attribute chain rooted at a module-level
    /// foreign (`ImportBinding::Foreign`) import. Not a constant: every read
    /// or call through the attribute re-evaluates the reference at the use
    /// site (D-256), so its `class_attrs` type is `Ty::Object`. A
    /// `staticmethod` object has no `__set_name__`, so admitting it keeps
    /// D-213's deferral sound.
    ForeignStatic(ForeignCallableRef),
}

/// A single class's declared shape (D-154): its attribute slots, in
/// first-`__init__`-assignment source order, and its method table (method
/// name -> the mangled `HirItem::Function` name in `HirModule::items` its
/// body was lowered to). See this module's own doc comment for why neither
/// field duplicates a method's body -- `methods` carries only the mangled
/// name, never a second copy.
#[derive(Debug, Clone, PartialEq)]
pub struct HirClassDef {
    pub name: String,
    /// Direct base class names, in source order from the class header
    /// (`class C(A, B):` → `["A", "B"]`). Empty for a class with no bases.
    /// Part 1 of #432: inheritance machinery.
    pub bases: Vec<String>,
    /// The computed C3 linearization (MRO) for this class: the class's own
    /// name first, followed by its bases' MROs in C3 order. For a class with
    /// no bases, this is `[self_name]`. Computed at HIR-lowering time by
    /// `compute_c3_mro` and stored here so every downstream consumer
    /// (`pycc_types`, `pycc_mir`) resolves methods/attributes/properties
    /// through the same MRO without re-deriving it. Part 1 of #432.
    pub mro: Vec<String>,
    pub attrs: Vec<(String, Ty)>,
    pub methods: Vec<(String, String)>,
    /// `@property` definitions (#377): each entry records a property's
    /// attribute name, the mangled getter method name, and (if present) the
    /// mangled setter method name. Property methods are lowered into
    /// ordinary mangled `HirItem::Function`s (just like regular methods) but
    /// are NOT entered into `methods` -- they are accessed via attribute
    /// syntax (`obj.x`), not method-call syntax (`obj.x()`). The type
    /// checker resolves `obj.x` reads/writes against this table, and MIR
    /// lowering rewrites property-shaped `AttrGet`/`AttrSet` into ordinary
    /// `MirExpr::Call`s to the getter/setter's mangled name, reusing the
    /// existing method-call infrastructure with no new MIR/codegen variant.
    pub properties: Vec<PropertyDef>,
    /// `@staticmethod` definitions (#436): each entry is
    /// `(method_name, mangled_name)`, where the mangled name uses a
    /// `.static` suffix (e.g. `C.create.static`). Static methods are NOT
    /// entered into `methods` -- they have their own table, and the
    /// `.static` suffix prevents collision with a regular method of the
    /// same name. Static methods can be called on both the class
    /// (`C.create(args)`) and an instance (`instance.create(args)`).
    pub static_methods: Vec<(String, String)>,
    /// `@classmethod` definitions (#436): each entry is
    /// `(method_name, mangled_name)`, where the mangled name uses a
    /// `.classmethod` suffix (e.g. `C.create.classmethod`). Class methods
    /// are NOT entered into `methods` -- they have their own table, and
    /// the `.classmethod` suffix prevents collision with a regular method
    /// of the same name. Class methods take an implicit `cls` parameter
    /// (typed `Ty::Instance(class_name)`) as their first parameter, and
    /// can be called on both the class and an instance.
    pub class_methods: Vec<(String, String)>,
    /// Default parameter values of this class's own methods (the method part
    /// of #1140): `(mangled_name, defaults)` for each method -- regular,
    /// `__init__`, `@staticmethod` or `@classmethod` -- that declares at
    /// least one, keyed by the same mangled name `methods`,
    /// `static_methods` and `class_methods` hold. `defaults` is parallel to
    /// that `HirItem::Function`'s full `params` (a regular method or
    /// `@classmethod` leads with a `None` for its receiver; a
    /// `@staticmethod` has no receiver entry), each other entry the lowered
    /// literal `func::params::check_default` admitted, or `None`.
    ///
    /// **Consumers.**
    ///
    /// * The `--ext` host boundary. The generated `METH_FASTCALL` wrapper and
    ///   `Py_tp_init` supply a default for an argument the host omitted.
    /// * An in-module call of a regular instance method (Part 1 of #1191,
    ///   #1438). A call that omits defaulted trailing parameters has those
    ///   defaults appended, through
    ///   [`HirClassDef::omitted_method_defaults`]. A `None` default at an
    ///   `object` parameter is not filled: `pycc_types` refuses that call.
    ///   A PEP 695 generic class's specializations carry these entries
    ///   re-keyed to the specialized mangled names
    ///   (`pycc_types::monomorphize::method_defaults`).
    ///
    /// Every other in-module call shape still passes every argument, and
    /// `pycc_types` keeps refusing a short one with its arity `T0021`. Those
    /// shapes are a constructor, `super().m()`, a static method and a class
    /// method.
    ///
    /// A redefinition of a method replaces its entry, matching the
    /// last-wins rebind of the method itself.
    pub method_defaults: Vec<(String, Vec<Option<HirExpr>>)>,
    /// PEP 695 (#387): the class's single type parameter name, if it is a
    /// generic class (`class C[T]:`). `None` for a non-generic class. At
    /// instantiation site (`C[int](args)`), this parameter is substituted
    /// with the concrete type, reusing PR-13's `Ty::Param` call-site-
    /// substitution mechanism (D-133/D-134).
    pub type_param: Option<String>,
    /// PEP 435 (#921): `true` iff `lower_enum_class` produced this class
    /// (its marker base was `Enum` or `StrEnum`). This -- not
    /// `enum_members` being non-empty -- is the enum marker: a
    /// docstring-only enum body (#744) is an enum with an empty member
    /// table. An enum class has no `__init__` in its MRO by design
    /// (`lower_class` early-returns into `lower_enum_class` ahead of
    /// D-225's `ensure_init`), so a call to it (`Color()`, `Color(1)`) is
    /// `C0001`: the per-item scan in `class::enum_call` reports every call
    /// it can attribute at the call expression's own span (#944, D-233),
    /// and `pycc_types::class::binding::resolve_instantiation` keeps a
    /// span-less guard on this flag behind it, before its MRO walk. Both
    /// read the flag off the named class only; an ordinary class that lists
    /// an enum class as a base (`class Foo(Color): pass`) never reaches
    /// either because `mro::validate_bases` rejects the enum base with its
    /// own `C0001` (#941), so no subclass of an enum is ever lowered.
    pub is_enum: bool,
    /// #966: this class's own `__init__` entry is the D-225 implicit
    /// zero-argument constructor synthesized by
    /// [`init::ensure_init`](self::init::ensure_init) because nothing in the
    /// class's MRO declared one -- pycc's stand-in for CPython's inherited
    /// `object.__init__`.
    ///
    /// It is *provenance*, never shape: an explicitly written
    /// `def __init__(self) -> None: pass` lowers to the same empty body and
    /// must keep ranking first, so no consumer may infer this flag from the
    /// constructor's parameter list or body. Only `ensure_init`'s call site
    /// in [`lower_class`] sets it; the dataclass path shares
    /// [`init::synthesize_dataclass_init`] but produces a *real* constructor
    /// and leaves this `false`.
    ///
    /// Constructor resolution ranks a flagged class **last**, matching
    /// CPython: for `class C(A, B)` with `A` init-less and `B` declaring
    /// `__init__`, `C()` must run `B.__init__`, not `A`'s implicit stub.
    /// The consumers that honour it are `pycc_mir`'s `Instantiate` and
    /// `super()` lowering, `pycc_types`'
    /// `class::binding::resolve_instantiation` and
    /// `class::resolve_super_method_call`, and
    /// `pycc_types::exception::reject_own_constructor`. Ordinary
    /// (non-constructor) method resolution is deliberately unaffected.
    pub implicit_object_init: bool,
    /// PEP 435 (#379, PR-19): the enum members of an enum class
    /// (`class Color(Enum): RED = 1; GREEN = 2`), in source order. Each
    /// entry is `(member_name, value)` where `value` is the `int` or `str`
    /// literal assigned to the member (#892). Empty for a non-enum class
    /// and for a member-less (docstring-only, #744) enum class alike --
    /// `is_enum` is the enum marker; this is only the member list. The
    /// consumers that key on a non-empty table (`pycc_types::enum_lower`,
    /// match exhaustiveness, `pycc_mir::matching`) are about members, so an
    /// empty table is correct for them by construction. An enum class has
    /// `bases = []` and `mro = [self_name]` (the `Enum` base is consumed
    /// as a marker, not a real base), no `__init__` requirement, and
    /// `attrs = [("value", <Ty::Int or Ty::Str>), ("name", Ty::Str)]` --
    /// `value`'s type follows the class's own member-value type (#892), and
    /// every member of one class must share it -- so existing
    /// `resolve_attr_get`/MIR slot resolution handle `member.value`/
    /// `member.name` unchanged. Each member is a compile-time singleton
    /// instance allocated once at module-init time (see `pycc_codegen`'s
    /// per-member init sequence).
    pub enum_members: Vec<(String, EnumMemberValue)>,
    /// Class-level attributes in source order, in either spelling: the
    /// annotated one PEP 526 gives (`MIN_WIDTH: int = -1024`,
    /// `LIMIT: ClassVar[int] = 8`, #911, Part 1 of #885) and the bare
    /// `X = 1`, whose type is inferred from the literal (#910, Part 2).
    /// Each entry is `(attribute_name, type, constant_value)`.
    ///
    /// **Storage model.** A class attribute is a *compile-time constant*: it
    /// occupies no instance slot and has no runtime representation at all.
    /// `pycc_mir` folds a read (`W.MIN_WIDTH` and `w.MIN_WIDTH` alike)
    /// straight to the literal, so neither `mro_attrs` nor
    /// the allocation size a class's instances need is affected by adding
    /// one. Since #960 an instance read folds only when no instance slot of
    /// that name exists anywhere in the MRO: a slot contributed by a sibling
    /// MRO base wins, matching CPython. This deliberately differs from the enum-member model, which
    /// allocates a per-member singleton with a module global.
    ///
    /// The one exception is [`ClassAttrValue::ForeignStatic`] (#1345,
    /// D-256): `name = staticmethod(<foreign ref>)`, typed `Ty::Object`. It
    /// has no storage either; a read or call is rewritten at the use site
    /// into the reference, selected by
    /// [`class_namespace_winner`](foreign_static::class_namespace_winner).
    ///
    /// **Named invariant -- class attributes are restricted to scalar slot
    /// types, except the foreign `staticmethod`.** Beyond D-154's storage
    /// constraint, this is what keeps `__set_name__` untriggerable per
    /// #585/D-213: a descriptor-valued class attribute
    /// (`x: SomeDescriptor = SomeDescriptor()`) is not a scalar and is
    /// rejected with `C0001`, so `__set_name__`'s own precondition never
    /// arises. A `staticmethod` object is a descriptor but has no
    /// `__set_name__` (measured on CPython 3.14.7), so the one admitted
    /// exception keeps the precondition unreachable. Relaxing this
    /// restriction further requires revisiting #585 in the same change.
    ///
    /// `Ty::Param(_)` is excluded as well: a type parameter has no
    /// compile-time constant value to fold.
    pub class_attrs: Vec<(String, Ty, ClassAttrValue)>,
    /// PEP 557/681 (#378, PR-18): `true` when this class is decorated with
    /// `@dataclass` or `@dataclass_transform(...)`. A dataclass class
    /// auto-generates `__init__`, `__eq__`, and `__repr__` from its
    /// `dataclass_fields` at HIR-lowering time (see `lower_class`'s own
    /// synthesis logic). `false` for an ordinary class and for an enum
    /// class.
    pub is_dataclass: bool,
    /// PEP 557 (#378, PR-18): the dataclass's annotated fields, in
    /// declaration order. Each entry is `(field_name, field_type)`. Empty
    /// for a non-dataclass class and for a dataclass with no fields (a
    /// zero-field dataclass is valid per PEP 557). Populated from
    /// `Stmt::AnnAssign` nodes in the class body; a field with a default
    /// value (`x: int = field(default=...)`) is recognized but rejected
    /// with `C0001` (field defaults are deferred to a follow-up issue --
    /// the compiler has no optional-parameter mechanism yet).
    pub dataclass_fields: Vec<(String, Ty)>,
    /// PEP 544 (#380, PR-20): `true` when this class is a protocol class
    /// (`class P(Protocol):`). A protocol class is a compile-time-only
    /// interface description — it is never instantiated and has no runtime
    /// representation. Its `protocol_members` field lists the required
    /// methods and attributes. A protocol class has `bases = []` and
    /// `mro = [self_name]` (the `Protocol` base is consumed as a marker,
    /// like `Enum`), unless it inherits from another user-defined protocol
    /// (`class Q(P):` where `P` is a protocol), in which case `P` is a
    /// real base and `Q` inherits `P`'s `protocol_members`.
    pub is_protocol: bool,
    /// PEP 544 (#380, PR-20): `true` when this protocol class is decorated
    /// with `@runtime_checkable`. A `@runtime_checkable` protocol can be
    /// used in `isinstance` checks (evaluated at compile time as a
    /// structural conformance check). A non-`@runtime_checkable` protocol
    /// used in `isinstance` is rejected with `T0021`. Always `false` for a
    /// non-protocol class.
    pub runtime_checkable: bool,
    /// PEP 544 (#380, PR-20): the required methods and attributes of a
    /// protocol class. Each entry is either a method requirement (name +
    /// parameter types + return type) or an attribute requirement (name +
    /// type). For a protocol inheriting from another protocol, this
    /// includes inherited members. Empty for a non-protocol class.
    pub protocol_members: Vec<ProtocolMember>,
    /// PEP 3119 (#380, PR-20): method names decorated with
    /// `@abstractmethod` in this class or any base class. A concrete
    /// subclass that does not override every inherited abstract method is
    /// rejected with `C0001` at class-definition time. Empty for a class
    /// with no abstract methods.
    pub abstract_methods: Vec<String>,
    /// PEP 3119 (#380, PR-20): `true` when this class inherits from `ABC`
    /// (`class C(ABC):`). An abstract class cannot be instantiated
    /// (rejected with `C0001`). The `ABC` base is consumed as a marker
    /// (like `Enum`/`Protocol`), not recorded as a real base.
    pub is_abstract: bool,
    /// Part 2 of #541 (D-189), widened by Part 2 of #543 (#739, D-194): the
    /// runtime exception type tag this class is raised and caught under, or
    /// `None` when the class is **not raisable**.
    ///
    /// `None` never means "synthetic". D-188 makes
    /// `HirModule::seeded_builtin_exception_classes` the sole provenance
    /// signal for syntheticness, and this field carries no provenance
    /// information whatsoever. There are two families of synthetic builtin
    /// exception classes, and only one of them carries `None` here: the
    /// original flat seven (`Exception`, `ValueError`, `TypeError`,
    /// `KeyError`, `IndexError`, `ZeroDivisionError`, `RuntimeError`) carry
    /// `None` even though they are among the most raisable classes in the
    /// language, because their tags are fixed constants resolved by name
    /// (`pycc_mir::exception::resolve_exception_tag`) rather than assigned
    /// per module. Every builtin past the flat seven (array index
    /// `>= FIRST_OSERROR_FAMILY_TAG`) has no such name-based fallback and
    /// instead carries a fixed `Some` tag directly on this field, assigned by
    /// array index in
    /// `pycc_hir::exception::builtin_exception_class_defs`. Reading `None`
    /// as "user-defined" or `Some` as "user-defined" is equally wrong.
    ///
    /// A tag is assigned by `program::finalize` (reached from
    /// `module::lower_all` for a single module and from the driver's linker
    /// for a whole program) to every user-declared class whose
    /// MRO reaches a builtin exception class, in a deterministic order, from
    /// `FIRST_USER_EXCEPTION_TYPE_TAG..=254`; the lower tags are reserved for
    /// the builtin hierarchy and 255 for a bridged foreign non-`Exception`
    /// (`FOREIGN_BASE_EXCEPTION_TYPE_TAG`, #1316). A module declaring more than
    /// `MAX_USER_EXCEPTION_CLASSES` such classes is rejected with `C0001`.
    /// Every other class — including a user class that never touches the
    /// exception hierarchy — keeps `None`.
    pub exception_type_tag: Option<u8>,
}

/// PEP 544 (#380, PR-20): a single required member of a protocol class.
/// Either a method requirement (name + parameter types + return type) or
/// an attribute requirement (name + type).
#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolMember {
    /// A required method: name, parameter types (excluding `self`), and
    /// return type.
    Method {
        name: String,
        param_tys: Vec<Ty>,
        return_ty: Ty,
    },
    /// A required attribute: name and type.
    Attribute { name: String, ty: Ty },
}

/// A single `@property` definition (#377): the attribute name exposed to
/// user code (e.g. `"x"` in `obj.x`), the mangled getter method name
/// (e.g. `"C.x"`, the same `<Class>.<name>` mangling a regular method
/// uses), and -- if a `@<name>.setter` method was also defined -- the
/// mangled setter method name (e.g. `"C.x.setter"`, using a `.setter`
/// suffix that a real Python identifier can never contain, so it cannot
/// collide with a regular method's mangled name).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDef {
    pub name: String,
    pub getter: String,
    pub setter: Option<String>,
}

/// How a class-body `def` is classified by its decorator list (#377):
/// a regular method (no decorator or an unrecognized one -- the latter is
/// rejected), a `@property` getter, or a `@<name>.setter` setter for the
/// property named `<name>`. `lower_class` uses this to decide the method's
/// mangled name and which table (`methods` vs `properties`) it belongs to.
enum MethodKind {
    /// A regular method (no decorator, or `@override`). `is_override` is
    /// `true` when the method is decorated `@override` (PEP 698) -- the
    /// method is still lowered as an ordinary mangled `HirItem::Function`
    /// exactly like a non-override regular method, but `lower_class`
    /// additionally verifies the method name exists in at least one base
    /// class's methods or properties (emitting T0031 if not).
    Regular { is_override: bool },
    /// `@property` getter for the attribute named `prop_name` (which is
    /// also the method's own source name, e.g. `def x(self) -> int` with
    /// `@property` is a getter for attribute `"x"`).
    PropertyGetter { prop_name: String },
    /// `@<prop_name>.setter` setter for the property named `prop_name`.
    /// The method's own source name must match `prop_name` (this is
    /// validated in `classify_decorator`).
    PropertySetter { prop_name: String },
    /// `@staticmethod` (#436): a static method. Takes no implicit `self`
    /// or `cls` parameter -- the method's own parameter list is exactly
    /// what the user wrote. Mangled name uses a `.static` suffix to avoid
    /// colliding with a regular method of the same name.
    StaticMethod,
    /// `@classmethod` (#436): a class method. Takes an implicit `cls`
    /// parameter (typed `Ty::Instance(class_name)`, matching `self`'s own
    /// type in this compiler's static-dispatch model) as its first
    /// parameter. Mangled name uses a `.classmethod` suffix to avoid
    /// colliding with a regular method of the same name.
    ClassMethod,
    /// `@abstractmethod` (#380, PR-20, PEP 3119): an abstract method.
    /// The method is still lowered as an ordinary mangled
    /// `HirItem::Function`, but `lower_class` records its name in
    /// `HirClassDef::abstract_methods`. A concrete subclass that does not
    /// override every inherited abstract method is rejected with `C0001`.
    /// The method body must be a declaration-style body (`...` or `pass`).
    AbstractMethod,
}

/// Classifies a method's decorator list (#377, #432). Returns:
/// - `Regular { is_override: false }` if the list is empty.
/// - `Regular { is_override: true }` if the single decorator is `@override`
///   (a bare name `override`, PEP 698).
/// - `PropertyGetter` if the single decorator is `@property` (a bare name
///   `property`).
/// - `PropertySetter` if the single decorator is `@<name>.setter` (an
///   attribute access on a bare name, where the attribute is `"setter"`).
///   The method's own source name must match `<name>` -- a mismatch
///   (e.g. `@x.setter def y(self, v): ...`) is rejected with `C0001`,
///   matching the common Python idiom where the setter's method name
///   matches the property name.
/// - Any other decorator shape (multiple decorators, a call-shaped
///   decorator, a different attribute name) is rejected with `C0001`,
///   preserving the pre-#377 "method decorators are not supported yet"
///   diagnostic for everything outside `@property`/`@<name>.setter`/
///   `@override`.
fn classify_decorator(
    decorator_list: &[Decorator],
    method_name: &str,
    range: std::ops::Range<u32>,
) -> Result<MethodKind, Diagnostic> {
    if decorator_list.is_empty() {
        return Ok(MethodKind::Regular { is_override: false });
    }
    if decorator_list.len() > 1 {
        return Err(unsupported(
            "method decorators are not supported yet",
            range,
        ));
    }
    let decorator = &decorator_list[0];
    match &decorator.expression {
        // `@property` -- a bare name `property`.
        Expr::Name(name) if name.id.as_str() == "property" => Ok(MethodKind::PropertyGetter {
            prop_name: method_name.to_string(),
        }),
        // `@override` -- a bare name `override` (PEP 698). The method is
        // still a regular method; `lower_class` verifies the method name
        // exists in at least one base class.
        Expr::Name(name) if name.id.as_str() == "override" => {
            Ok(MethodKind::Regular { is_override: true })
        }
        // `@staticmethod` (#436) -- a bare name `staticmethod`.
        Expr::Name(name) if name.id.as_str() == "staticmethod" => Ok(MethodKind::StaticMethod),
        // `@classmethod` (#436) -- a bare name `classmethod`.
        Expr::Name(name) if name.id.as_str() == "classmethod" => Ok(MethodKind::ClassMethod),
        // `@abstractmethod` (#380, PR-20, PEP 3119) -- a bare name
        // `abstractmethod`. Recognized as a bare name without requiring
        // `from abc import abstractmethod`, matching the
        // `Final`/`Annotated`/`Enum` precedent.
        Expr::Name(name) if name.id.as_str() == "abstractmethod" => Ok(MethodKind::AbstractMethod),
        // `@<name>.setter` -- an attribute access on a bare name, where
        // the attribute is `"setter"`.
        Expr::Attribute(attr) => {
            let Expr::Name(base_name) = attr.value.as_ref() else {
                return Err(unsupported(
                    "method decorators are not supported yet",
                    range,
                ));
            };
            if attr.attr.as_str() != "setter" {
                return Err(unsupported(
                    "method decorators are not supported yet",
                    range,
                ));
            }
            let prop_name = base_name.id.as_str().to_string();
            // The decorator name must match the method's own source name
            // (e.g. `@x.setter def x(self, v): ...`, not `@x.setter def
            // y(self, v): ...`). This is the overwhelmingly common idiom;
            // the uncommon mismatch case is rejected rather than
            // silently supporting a shape this PR does not need.
            if prop_name != method_name {
                return Err(unsupported(
                    format!(
                        "a `@{prop_name}.setter` decorator must decorate a method named \
                         `{prop_name}`, not `{method_name}`"
                    ),
                    range,
                ));
            }
            Ok(MethodKind::PropertySetter { prop_name })
        }
        _ => Err(unsupported(
            "method decorators are not supported yet",
            range,
        )),
    }
}

/// PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): the result of
/// classifying a class's decorator list. `is_dataclass` is `true` for
/// `@dataclass`/`@dataclass_transform(...)`. `runtime_checkable` is `true`
/// for `@runtime_checkable` (PEP 544). Both can be `false` (an ordinary
/// class with no decorators). `@dataclass` combined with
/// `@runtime_checkable` is rejected — a protocol class cannot be a
/// dataclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClassDecoratorInfo {
    is_dataclass: bool,
    runtime_checkable: bool,
}

/// PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): classifies a class's
/// decorator list to determine whether the class is a `@dataclass` (PEP
/// 557), a `@dataclass_transform(...)` (PEP 681), or a
/// `@runtime_checkable` (PEP 544) class. Returns a `ClassDecoratorInfo`
/// struct.
///
/// Any other class decorator shape (a different bare name, an
/// attribute-access decorator, `@dataclass(frozen=True)` with options) is
/// rejected with `C0001`, preserving the pre-#378 "class decorators are
/// not supported yet" diagnostic for everything outside
/// `@dataclass`/`@dataclass_transform()`/`@runtime_checkable`.
fn classify_class_decorator(
    decorator_list: &[Decorator],
    range: std::ops::Range<u32>,
) -> Result<ClassDecoratorInfo, Diagnostic> {
    let mut info = ClassDecoratorInfo {
        is_dataclass: false,
        runtime_checkable: false,
    };
    if decorator_list.is_empty() {
        return Ok(info);
    }
    if decorator_list.len() > 2 {
        return Err(unsupported(
            "more than two class decorators are not supported yet",
            range,
        ));
    }
    for decorator in decorator_list {
        match &decorator.expression {
            // `@dataclass` -- a bare name `dataclass` (PEP 557).
            Expr::Name(name) if name.id.as_str() == "dataclass" => {
                info.is_dataclass = true;
            }
            // `@runtime_checkable` -- a bare name `runtime_checkable`
            // (PEP 544, #380, PR-20). Recognized as a bare name without
            // requiring `from typing import runtime_checkable`, matching
            // the `Final`/`Annotated`/`Enum` precedent.
            Expr::Name(name) if name.id.as_str() == "runtime_checkable" => {
                info.runtime_checkable = true;
            }
            // `@dataclass_transform(...)` -- a call whose callee is the
            // bare name `dataclass_transform` (PEP 681). The keyword
            // arguments are accepted but ignored -- pycc's dataclass
            // implementation always generates `__init__`/`__eq__`/`__repr__`,
            // matching the defaults.
            Expr::Call(call) => {
                let Expr::Name(name) = call.func.as_ref() else {
                    return Err(unsupported("class decorators are not supported yet", range));
                };
                if name.id.as_str() == "dataclass_transform" {
                    info.is_dataclass = true;
                } else if name.id.as_str() == "dataclass" {
                    // `@dataclass(frozen=True)` etc. -- rejected with C0001.
                    return Err(unsupported(
                        "`@dataclass` with options is not supported yet -- only a bare \
                         `@dataclass` is supported in this version",
                        range,
                    ));
                } else {
                    return Err(unsupported("class decorators are not supported yet", range));
                }
            }
            _ => {
                return Err(unsupported("class decorators are not supported yet", range));
            }
        }
    }
    // #380 (PR-20): `@dataclass` combined with `@runtime_checkable` is
    // rejected — a protocol class cannot be a dataclass (a protocol is a
    // compile-time-only interface description, not an instantiable class
    // with fields).
    if info.is_dataclass && info.runtime_checkable {
        return Err(unsupported(
            "`@dataclass` combined with `@runtime_checkable` is not supported -- a protocol \
             class cannot be a dataclass",
            range,
        ));
    }
    Ok(info)
}

/// #380 (PR-20): Returns `true` if a method body is declaration-style
/// (`...` (Ellipsis) or `pass`), suitable for a protocol method or an
/// `@abstractmethod`. A body with any other content (a docstring, an
/// assignment, a return statement, etc.) is an implementation body.
fn is_declaration_body(body: &[Stmt]) -> bool {
    // `pass` is filtered out by `lower_body`, so a body of just `pass`
    // is an empty slice after filtering. But we check the raw AST here
    // before any filtering.
    body.iter().all(|stmt| match stmt {
        Stmt::Pass(_) => true,
        // `...` (Ellipsis) as an expression statement.
        Stmt::Expr(expr_stmt) => {
            matches!(expr_stmt.value.as_ref(), Expr::EllipsisLiteral(_))
        }
        _ => false,
    }) && !body.is_empty()
}

/// One already-defined class, projected down to exactly what
/// `annotation_to_ty` needs. Replaces the former `(String, bool)` pair
/// (#380, PR-20), which carried the class name and its protocol flag and had
/// nowhere to record #693's `__class_getitem__` return type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClassAnnotationInfo {
    pub(crate) name: String,
    pub(crate) is_protocol: bool,
    /// Issue #693 (PEP 560, extending #611): the declared return type of
    /// whichever `__class_getitem__` hook `pycc_types`'
    /// `resolve_static_or_class_method_call` would itself dispatch to for a
    /// value-position `ClassName[type_arg]` call on this MRO -- found by
    /// `class_getitem_return_ty`'s own two-pass MRO walk (every MRO entry's
    /// `static_methods` first, then, only if none declared the hook, every
    /// MRO entry's `class_methods`). The two-pass order is deliberate: the
    /// winning declaration used to resolve a *return type* must be the exact
    /// same one value position would pick. `Some` only when an explicit hook
    /// exists somewhere in the
    /// MRO -- never set when a class is generic purely through a PEP 695
    /// type parameter with no hook of its own, since that case is
    /// deliberately still handled by `GenericClassInstantiate`, not by this
    /// field. `annotation_to_ty`'s `Subscript` arm routes
    /// `ClassName[type_arg]` through this return type instead of falling
    /// back to `Ty::Instance(ClassName)` when it is `Some`, and takes that
    /// same `Ty::Instance` fallback when it is `None` -- including the
    /// structurally-unreachable-in-practice case where an explicit hook
    /// exists but `class_getitem_return_ty`'s `items` lookup still comes
    /// back empty (issue #693 deep-review, Finding 2). Always `None` for the
    /// self-referential entry `lower_class` pushes for the class it is
    /// currently lowering (see that call site's own comment) -- the hook's
    /// return type is not yet resolvable at that point, so a
    /// `__class_getitem__`-typed annotation used *inside* the defining
    /// class's own body still falls back to `Ty::Instance`, same as before
    /// this field existed.
    pub(crate) class_getitem_return: Option<Ty>,
}

/// Projects the full class table down to the [`ClassAnnotationInfo`] slice
/// `annotation_to_ty` consults. Both of this crate's own projection sites
/// (`module::lower_top_level_item`'s per-statement rebuild and `lower_class`'s own
/// `defined_classes` view) go through here, so the two agree on every flag.
/// `items` is the module's `HirItem::Function` list accumulated so far (in
/// source order) -- it carries the resolved `return_ty` for every already-
/// lowered method, including any `__class_getitem__` hook, which `defs`
/// alone does not (`HirClassDef::static_methods`/`class_methods` store only
/// mangled names).
pub(crate) fn class_annotation_infos(
    defs: &[(String, HirClassDef)],
    items: &[HirItem],
) -> Vec<ClassAnnotationInfo> {
    defs.iter()
        .map(|(name, def)| {
            let class_getitem_return = class_getitem_return_ty(defs, &def.mro, items);
            ClassAnnotationInfo {
                name: name.clone(),
                is_protocol: def.is_protocol,
                class_getitem_return,
            }
        })
        .collect()
}

/// Issue #693: resolves the declared return type of whichever
/// `__class_getitem__` hook `resolve_static_or_class_method_call` (in
/// `pycc_types`) would itself dispatch to for a value-position `C[x]` call on
/// this MRO, so that annotation-position `C[x]` and value-position `C[x]`
/// always resolve through the exact same MRO entry.
///
/// Deliberately replicates that function's two-pass order rather than a
/// single combined pass: first walk the full MRO (most-derived first)
/// checking only `static_methods`; only if no class in the MRO declares the
/// hook as a `@staticmethod` does a second full MRO walk check
/// `class_methods`. A single pass that checks both tables together at each
/// MRO entry would let a derived class's `@classmethod` override win over a
/// base class's `@staticmethod` declaration merely because the derived class
/// comes first in the MRO -- `resolve_static_or_class_method_call` instead
/// lets *any* static-method declaration anywhere in the MRO outrank *every*
/// class-method declaration, regardless of MRO position. Returns `None` when
/// no class in the MRO declares the hook in either form, or (structurally
/// unreachable in practice, since every `static_methods`/`class_methods`
/// entry is pushed alongside a matching `items.push(item)` in the same
/// `lower_class` call) when the mangled name has no corresponding
/// `HirItem::Function` yet.
fn class_getitem_return_ty(
    defs: &[(String, HirClassDef)],
    mro: &[String],
    items: &[HirItem],
) -> Option<Ty> {
    // `return_ty` here is read straight off the `HirItem::Function` this
    // crate's own lowering produced -- before `pycc_types::check_and_resolve`
    // ever runs (see `lower_method`'s doc comment on `Ty::Infer`). An
    // unannotated `__class_getitem__` hook therefore still carries the raw
    // `Ty::Infer` placeholder at this point, even though value-position
    // `C[x]` dispatch (`pycc_types::resolve_static_or_class_method_call`)
    // only ever observes the hook's *post-inference* signature via the
    // `Environment`. Treat `Ty::Infer` as unresolved here too, so
    // `func.rs`'s `annotation_to_ty` falls back to `Ty::Instance(ClassName)`
    // instead of propagating an internal placeholder into a resolved
    // annotation type (issue #693 review, codex finding).
    let resolve = |mangled: &str| -> Option<Ty> {
        items.iter().find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == mangled => match return_ty {
                Ty::Infer => None,
                other => Some(other.clone()),
            },
            _ => None,
        })
    };
    // Unlike most other MRO walks in this crate, a class here is not
    // guaranteed to be present in `defs`: `class_annotation_infos` also runs
    // (via `lower_class`'s own pre-method-loop projection) against a
    // hand-built `defined_classes` slice that
    // `class::mro::tests::circular_inheritance_in_mro_is_rejected`
    // deliberately leaves incomplete to exercise the *defensive*
    // circular-inheritance check downstream in `resolve_mro` -- that check,
    // not this function, is the one responsible for rejecting such an MRO.
    // Skip a class this function cannot resolve rather than panicking; each
    // loop naturally continues to the next ancestor, and no hook is found
    // through this leg either way.
    for mro_class in mro {
        let Some((_, mro_def)) = defs.iter().find(|(name, _)| name == mro_class) else {
            continue;
        };
        if let Some((_, mangled)) = mro_def
            .static_methods
            .iter()
            .find(|(method, _)| method == "__class_getitem__")
        {
            return resolve(mangled);
        }
    }
    for mro_class in mro {
        let Some((_, mro_def)) = defs.iter().find(|(name, _)| name == mro_class) else {
            continue;
        };
        if let Some((_, mangled)) = mro_def
            .class_methods
            .iter()
            .find(|(method, _)| method == "__class_getitem__")
        {
            return resolve(mangled);
        }
    }
    None
}

/// Lowers a module-level `class Foo: ...` statement (D-154). Returns the
/// class's own declared shape (for `HirModule::class_defs`) alongside every
/// method it defines, already lowered into ordinary mangled
/// `HirItem::Function`s ready to append to `HirModule::items` -- see this
/// module's own doc comment for why a class has no `HirItem` of its own.
///
/// `defined_classes` is the list of already-lowered class definitions (name
/// → `HirClassDef`), in source order. Inheritance validation (unknown base,
/// circular inheritance, generic-class-as-base) uses this to check each base
/// class against the classes defined earlier in the module, matching Python's
/// own source-order class-body execution (a base class must be defined before
/// the derived class that inherits from it).
///
/// Every check below is a `C0001` capability diagnostic, not a design
/// question this PR resolves: a class decorator is out of scope entirely
/// (dataclasses/`dataclass_transform` are unrelated later PRs' own scope).
/// Generic classes (`class C[T]:`) with a single type parameter ARE now
/// supported by #387 (see the `type_params` handling below). Inheritance
/// (`class C(Base):`) is now supported by #432 (Part 1).
///
/// `base_class_asts` (#585) pairs every already-lowered class name in this
/// module with its original `StmtClassDef`, so this call can re-inspect an
/// *earlier* class's own `__init_subclass__` body when the class being
/// lowered here inherits, rather than overrides, that hook -- `defined_classes`
/// alone only carries the already-lowered `HirClassDef`, which has no method
/// bodies left to re-validate. A base absent from this slice (a synthetic
/// builtin-exception class, or any base this crate cannot introspect) is
/// simply treated as not defining a validatable inherited hook, matching
/// this file's existing "unintrospectable -> unrestricted" posture elsewhere.
///
/// `staticmethod_rebound` (#1345) is whether the module body binds
/// `staticmethod` anywhere, computed once by `module::lower_module`.
///
/// `type_vars` (#1394) are the module-level `T = TypeVar("T")` declarations
/// lowered before this class, the names a `Generic[...]` base may list
/// besides foreign-imported ones (see `generic_base`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_class(
    def: &pycc_ast::StmtClassDef,
    aliases: &[(String, Ty)],
    defined_classes: &[(String, HirClassDef)],
    module_items: &[HirItem],
    base_class_asts: &[(String, &pycc_ast::StmtClassDef)],
    imports: &[ImportBinding],
    type_vars: &[String],
    signatures: &SignatureTable,
    staticmethod_rebound: bool,
) -> Result<(HirClassDef, Vec<HirItem>), Diagnostic> {
    // #380 (PR-20): build the projected class slice `annotation_to_ty` uses
    // to resolve cross-class annotations (including protocol-typed ones);
    // #693 added `class_getitem_return`, resolved from `module_items` (every
    // `HirItem::Function` lowered by an earlier class or top-level `def` in
    // this module, in source order).
    let mut class_name_defs = class_annotation_infos(defined_classes, module_items);
    // PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): classify the class's
    // decorator list. `@dataclass`/`@dataclass_transform(...)` and
    // `@runtime_checkable` are recognized; any other class decorator is
    // rejected with C0001.
    let decorator_info = classify_class_decorator(&def.decorator_list, def.range.into())?;
    let is_dataclass = decorator_info.is_dataclass;
    // PEP 695 (#387): a generic class (`class C[T]:`) with exactly one type
    // parameter is now supported, reusing PR-13's `Ty::Param` call-site-
    // substitution mechanism (D-133/D-134). More than one type parameter is
    // rejected, matching PR-13's own generic-function scoping.
    let type_param: Option<String> = match def.type_params.as_deref() {
        None => None,
        Some(type_params) => match type_params.type_params.as_slice() {
            [single] => Some(crate::type_param_name(single, def.range)?.to_string()),
            _ => {
                return Err(unsupported(
                    "generic classes with more than one type parameter are not supported yet",
                    def.range,
                ));
            }
        },
    };
    // #432: parse base class names from the class header's positional
    // arguments (`class C(A, B):` → `["A", "B"]`); a `Generic[T, ...]` base
    // is checked and erased there (Part 1 of #886, #1394). Keyword
    // arguments (e.g. `metaclass=Meta`) and every other non-name base
    // (`class C(SomeMod.Base):`, `class C(Base[int]):`) are still rejected.
    let mut bases = generic_base::lower_base_names(def, imports, type_vars, type_param.is_some())?;
    let class_name = def.name.to_string();
    // #379 (PR-19): PEP 435 enum detection. A class whose single base is
    // the bare name `Enum` (`class Color(Enum):`) is an enum class. `Enum`
    // is a builtin base name (see `is_enum_base_name`), not a user-defined
    // class in `defined_classes` -- so it must be intercepted here, before
    // the unknown-base rejection below. An enum class has `bases = []` and
    // `mro = [self_name]` (the `Enum` base is consumed as a marker, not
    // recorded as a real base), no `__init__` requirement, and its body is
    // assignments only (`RED = 1`), not method definitions. Multiple bases
    // with `Enum` (e.g. `class C(Enum, Other):`) are rejected -- `Enum`
    // must be the sole base. A generic enum (`class C[T](Enum):`) is also
    // rejected -- enums are never generic.
    let is_enum = bases.len() == 1 && crate::is_enum_base_name(&bases[0]);
    // #892: `StrEnum` fixes the class's member-value type to `str` before
    // any member is read, so the marker base's own spelling has to be
    // captured here -- `bases.clear()` below consumes it, and
    // `lower_enum_class` is not called until well after that point.
    let is_str_enum = is_enum && bases[0] == "StrEnum";
    if is_enum {
        if type_param.is_some() {
            return Err(unsupported(
                format!(
                    "generic enum class `{class_name}` is not supported -- enums cannot have \
                     type parameters"
                ),
                def.range,
            ));
        }
        // Consume the `Enum` base as a marker: clear `bases` so the class
        // has no real inheritance, and set `mro = [self_name]`.
        bases.clear();
    }
    // #380 (PR-20): PEP 544 protocol detection. A class whose sole base is
    // the bare name `Protocol` (`class P(Protocol):`) is a protocol class.
    // `Protocol` is a builtin base name (see `is_protocol_base_name`), not
    // a user-defined class — intercepted here, before the unknown-base
    // rejection below. A protocol class has `bases = []` and
    // `mro = [self_name]` (the `Protocol` base is consumed as a marker,
    // like `Enum`). Multiple bases with `Protocol` are rejected —
    // `Protocol` must be the sole base. A generic protocol is also
    // rejected. A protocol class with `@dataclass` is rejected (a protocol
    // is a compile-time-only interface, not an instantiable class with
    // fields).
    let is_protocol = bases.len() == 1 && crate::is_protocol_base_name(&bases[0]);
    if is_protocol {
        if type_param.is_some() {
            return Err(unsupported(
                format!(
                    "generic protocol class `{class_name}` is not supported -- protocols cannot \
                     have type parameters in this version"
                ),
                def.range,
            ));
        }
        if is_dataclass {
            return Err(unsupported(
                format!(
                    "protocol class `{class_name}` cannot be a `@dataclass` -- a protocol is a \
                     compile-time-only interface description, not an instantiable class with fields"
                ),
                def.range,
            ));
        }
        // Consume the `Protocol` base as a marker: clear `bases` so the
        // class has no real inheritance, and set `mro = [self_name]`.
        bases.clear();
    }
    // #380 (PR-20): PEP 3119 ABC detection. A class with `ABC` among its
    // bases (`class C(ABC):` or `class C(ABC, Base):`) is abstract. `ABC`
    // is a builtin base name (see `is_abc_base_name`), consumed as a
    // marker — removed from `bases` so it is not treated as a real base
    // class. Unlike `Enum`/`Protocol`, `ABC` can coexist with other bases
    // (`class C(ABC, Base):` is valid — `Base` is a real base, `ABC` is
    // just a marker).
    let is_abstract = bases.iter().any(|b| crate::is_abc_base_name(b));
    if is_abstract {
        // Remove `ABC` from bases — it is a marker, not a real base.
        bases.retain(|b| !crate::is_abc_base_name(b));
    }
    // #380 (PR-20): `@runtime_checkable` on a non-protocol class is
    // rejected — `runtime_checkable` is only valid on protocol classes.
    if decorator_info.runtime_checkable && !is_protocol {
        return Err(unsupported(
            format!(
                "`@runtime_checkable` on class `{class_name}` is not valid -- \
                 `@runtime_checkable` is only valid on protocol classes (`class P(Protocol):`)"
            ),
            def.range,
        ));
    }
    // #432: A generic class (from #387) with base classes is not supported
    // yet — `instantiate_generic_class_methods` creates a monomorphized
    // `HirClassDef` with empty `bases`/`mro`, silently dropping inheritance.
    // Reject this early with a clear error rather than letting it fail with
    // a confusing T0044 downstream.
    if type_param.is_some() && !bases.is_empty() {
        return Err(unsupported(
            format!(
                "generic class `{class_name}` with base classes is not supported yet -- \
                 generic classes cannot inherit from other classes in this version"
            ),
            def.range,
        ));
    }
    validate_bases(
        &class_name,
        &bases,
        defined_classes,
        aliases,
        imports,
        module_items,
        def.range.into(),
    )?;
    // #380 (PR-20): protocol inheritance detection. A class that inherits
    // from a user-defined protocol (`class Q(P):` where `P` is a protocol)
    // is itself a protocol. This check runs after the `Protocol` marker
    // base consumption above (which handles `class P(Protocol):`) and
    // after the base validation loop (which ensures every base is a known
    // class). If any base is a protocol, this class is a protocol too —
    // it inherits the base's `protocol_members` and adds its own.
    let inherits_protocol = bases.iter().any(|base_name| {
        defined_classes
            .iter()
            .any(|(n, d)| n == base_name && d.is_protocol)
    });
    let is_protocol = is_protocol || inherits_protocol;
    if inherits_protocol {
        // A protocol inheriting from a protocol cannot also be a dataclass.
        if is_dataclass {
            return Err(unsupported(
                format!(
                    "protocol class `{class_name}` cannot be a `@dataclass` -- a protocol is a \
                     compile-time-only interface description, not an instantiable class with fields"
                ),
                def.range,
            ));
        }
        // `@runtime_checkable` is valid on a protocol inheriting from
        // another protocol — no extra check needed here.
    }
    let mro = resolve_mro(&class_name, &bases, defined_classes, def.range.into())?;
    // PEP 560 (#611, narrowed by #1130): the class currently being lowered
    // is not in `defined_classes` yet, so without this entry a
    // self-referential `C[int]` annotation inside `C`'s own body would not
    // find its own class at all. Since #1130 there is no subscriptability
    // gate left for it to bypass -- a subscripted annotation whose base
    // resolves to a class is accepted with the type argument erased -- so
    // the entry's remaining purpose is the `class_name` carve-out in
    // `annotation_to_ty`'s subscript arm: the direct `class_defs` lookup is
    // skipped for the six names the `Expr::Name` arm answers before
    // `class_defs` (`name_resolves_before_class_defs`) *except* when the base
    // is the enclosing class's own name, and that exception resolves through
    // this entry.
    class_name_defs.push(ClassAnnotationInfo {
        name: class_name.clone(),
        is_protocol,
        // #693: the hook's return type is not resolvable yet at this
        // point -- an own hook has not been lowered into an
        // `HirItem::Function` yet, and this self-referential entry exists
        // specifically to make a same-body annotation name its own class,
        // not to type it precisely. Such an annotation (rare:
        // `ClassName[x]` referring to the very class whose body it appears
        // in) falls back to `Ty::Instance`, exactly as it did before this
        // field existed.
        class_getitem_return: None,
    });
    let enum_members: Vec<(String, EnumMemberValue)> = Vec::new();
    // #379 (PR-19): an enum class body is assignments only (`RED = 1`),
    // not method definitions. Handle this in a separate function before the
    // regular class-body loop below, which rejects non-`def` statements.
    if is_enum {
        return lower_enum_class(def, class_name, bases, mro, type_param, is_str_enum);
    }
    // #380 (PR-20): a protocol class body contains method definitions
    // (with `...` or `pass` bodies) and annotated assignments
    // (`x: int`). Methods are recorded as `ProtocolMember::Method` and
    // are NOT lowered to `HirItem::Function`s. Annotated assignments are
    // recorded as `ProtocolMember::Attribute`. A protocol method with an
    // implementation body (anything other than `...` or `pass`) is
    // rejected with `C0001`. A protocol `__init__` is rejected with
    // `C0001`. Protocol members from base protocols are inherited.
    if is_protocol {
        return lower_protocol_class(
            def,
            class_name,
            bases,
            mro,
            type_param,
            decorator_info.runtime_checkable,
            defined_classes,
            &class_name_defs,
        );
    }
    // #378 (PR-18): a `@dataclass` class inheriting from a non-dataclass
    // base is rejected -- the base must also be a dataclass for field
    // merging to work. A `@dataclass` with no bases is always valid.
    if is_dataclass {
        for base_name in &bases {
            let base_def = defined_classes
                .iter()
                .find(|(name, _)| name == base_name)
                .map(|(_, def)| def)
                .expect("base class must be defined before the derived class");
            if !base_def.is_dataclass {
                return Err(unsupported(
                    format!(
                        "a `@dataclass` class `{class_name}` cannot inherit from non-dataclass \
                         class `{base_name}` -- dataclass inheritance requires all bases to also \
                         be dataclasses in this version"
                    ),
                    def.range,
                ));
            }
        }
    }
    let ClassBodyOutput {
        mut methods,
        mut items,
        mut attrs,
        properties,
        static_methods,
        class_methods,
        mut dataclass_fields,
        abstract_methods,
        class_attrs,
        method_defaults,
    } = walk_class_body(&ClassBodyInput {
        body: &def.body,
        class_name: &class_name,
        is_dataclass,
        type_param: type_param.as_deref(),
        aliases,
        class_name_defs: &class_name_defs,
        mro: &mro,
        defined_classes,
        imports,
        signatures,
        staticmethod_rebound,
    })?;
    // #911: the class-attribute/instance-slot collision check runs here,
    // after the walk, not at the `AnnAssign` site: `attrs` is populated only
    // when the walk reaches `__init__`, so a class attribute declared before
    // `__init__` would see an empty `attrs` and slip through.
    reject_class_attr_collisions(&ClassAttrCollisionInput {
        class_attrs: &class_attrs,
        attrs: &attrs,
        properties: &properties,
        methods: &methods,
        static_methods: &static_methods,
        class_methods: &class_methods,
        class_name: &class_name,
        mro: &mro,
        defined_classes,
        range: def.range.into(),
    })?;
    // #378 (PR-18): synthesize `__init__`, `__eq__`, and `__repr__` for a
    // `@dataclass` class from its (merged) field list. The synthesized
    // methods flow through the existing method infrastructure as ordinary
    // mangled `HirItem::Function`s, exactly like hand-written methods.
    if is_dataclass {
        // Merge parent dataclass fields (via MRO, in reverse order so
        // parent fields come first) with own fields. The MRO is ordered
        // most-derived-first, so walking it in reverse gives
        // least-derived-first (parent fields before own fields), matching
        // PEP 557's field ordering for inheritance.
        let mut merged_fields: Vec<(String, Ty)> = Vec::new();
        for mro_class in mro.iter().skip(1).rev() {
            // Every class in the MRO (other than the class itself, which
            // `skip(1)` removes) was defined earlier in the module -- the
            // C3 MRO is built from already-lowered class definitions, and
            // dataclass inheritance requires all bases to be dataclasses.
            // Using `.expect` (whose panic path lives in libcore, outside
            // this crate's instrumented regions) avoids an `if let Some`
            // whose `else` branch is structurally unreachable and would
            // show up as a permanently uncovered line under D-014's 100%
            // line-coverage gate.
            let (_, base_def) = defined_classes
                .iter()
                .find(|(name, _)| name == mro_class)
                .expect("MRO class must be defined before the derived class");
            for (name, ty) in &base_def.dataclass_fields {
                match merged_fields.iter().find(|(n, _)| n == name) {
                    Some((_, existing_ty)) if existing_ty != ty => {
                        return Err(Diagnostic::error(
                            "T0052",
                            format!(
                                "attribute `{name}` is declared as `{}` and as `{}` by different dataclasses in the method resolution order of class `{class_name}`",
                                existing_ty.name(),
                                ty.name(),
                            ),
                            Span::new(u32::from(def.range.start()), u32::from(def.range.end())),
                        ));
                    }
                    Some(_) => {}
                    None => merged_fields.push((name.clone(), ty.clone())),
                }
            }
        }
        for (name, ty) in &dataclass_fields {
            match merged_fields.iter().find(|(n, _)| n == name) {
                Some((_, existing_ty)) if existing_ty != ty => {
                    return Err(Diagnostic::error(
                        "T0052",
                        format!(
                            "attribute `{name}` is declared as `{}` in a base class and as `{}` in dataclass `{class_name}`, both in the same method resolution order",
                            existing_ty.name(),
                            ty.name(),
                        ),
                        Span::new(u32::from(def.range.start()), u32::from(def.range.end())),
                    ));
                }
                Some(_) => {}
                None => merged_fields.push((name.clone(), ty.clone())),
            }
        }
        // #913/D-235: a merged dataclass field must not share its name with a
        // `ClassVar` declared anywhere in the MRO. The policy and its three
        // reachable shapes live with the rest of the class-attribute
        // collision rules in `attrs`.
        reject_dataclass_field_class_var_collisions(
            &merged_fields,
            &class_attrs,
            &class_name,
            &mro,
            defined_classes,
            def.range.into(),
        )?;
        // Populate `attrs` from the merged field list (the dataclass's
        // attribute slots are exactly its fields, in declaration order).
        attrs = merged_fields.clone();
        // Synthesize `__init__`: `def __init__(self, f1: T1, f2: T2, ...):
        // self.f1 = f1; self.f2 = f2; ...`
        let init_item = synthesize_dataclass_init(&class_name, &merged_fields);
        let init_mangled = format!("{class_name}.__init__");
        methods.push(("__init__".to_string(), init_mangled));
        items.push(init_item);
        // Synthesize `__eq__`: `def __eq__(self, other: ClassName) -> bool:
        // return self.f1 == other.f1 and self.f2 == other.f2 and ...`
        let eq_item = synthesize_dataclass_eq(&class_name, &merged_fields);
        let eq_mangled = format!("{class_name}.__eq__");
        methods.push(("__eq__".to_string(), eq_mangled));
        items.push(eq_item);
        // Synthesize `__repr__`: `def __repr__(self) -> str: return
        // "ClassName(f1=" + str(self.f1) + ", f2=" + str(self.f2) + ...)"`
        let repr_item = synthesize_dataclass_repr(&class_name, &merged_fields);
        let repr_mangled = format!("{class_name}.__repr__");
        methods.push(("__repr__".to_string(), repr_mangled));
        items.push(repr_item);
        // Store the merged field list for downstream consumers.
        dataclass_fields = merged_fields;
    }
    // #380 (PR-20, PEP 3119): collect inherited abstract methods from the
    // MRO. A concrete (non-abstract) class must override every inherited
    // abstract method — a concrete subclass missing an override is
    // rejected with `C0001`. An abstract class (`is_abstract`) can have
    // unimplemented abstract methods (they are inherited by subclasses).
    let mut all_abstract_methods: Vec<String> = abstract_methods.clone();
    // Iterate over MRO entries (beyond the class itself), finding each in
    // `defined_classes`.  Using `filter_map` avoids a conditional with an
    // unreachable `else` branch — every MRO class is always in
    // `defined_classes`, but `filter_map` skips any that aren't without
    // creating a permanently-uncovered region under D-014.
    for (_, base_def) in mro
        .iter()
        .skip(1)
        .filter_map(|mro_class| defined_classes.iter().find(|(n, _)| n == mro_class))
    {
        for am in &base_def.abstract_methods {
            if !all_abstract_methods.contains(am) {
                all_abstract_methods.push(am.clone());
            }
        }
    }
    // A concrete class must override every inherited abstract method.
    // "Override" means the method name appears in this class's own
    // `methods` (not just inherited). `is_abstract` classes are exempt.
    if !is_abstract {
        for am in &all_abstract_methods {
            // Check if this class overrides the abstract method with a
            // concrete (non-abstract) method.
            let overridden =
                methods.iter().any(|(name, _)| name == am) && !abstract_methods.contains(am);
            if !overridden {
                return Err(unsupported(
                    format!(
                        "concrete class `{class_name}` does not override abstract method \
                         `{am}` inherited from a base class -- all abstract methods must be \
                         overridden in a concrete class"
                    ),
                    def.range,
                ));
            }
        }
    }
    // #966: set only here, from `ensure_init`'s own report -- never from
    // the shape of the constructor it synthesized, and never on the
    // dataclass path, whose `__init__` is a real constructor.
    let mut implicit_object_init = false;
    if !is_dataclass && !methods.iter().any(|(name, _)| name == "__init__") {
        // #432 / #912: the class declares no `__init__` of its own, so it
        // either inherits one from its MRO or gets a synthesized implicit
        // zero-argument constructor.
        implicit_object_init =
            ensure_init(&class_name, &mro, defined_classes, &mut methods, &mut items);
    }
    // #435 (Part B, __init_subclass__) / #585 / #854: PEP 487's
    // `__init_subclass__` hook is recognized as a valid method name. In
    // CPython, the hook is invoked via
    // `super(new_cls, new_cls).__init_subclass__(**kwargs)` at class-creation
    // time -- an MRO lookup that starts immediately *after* the class being
    // created, so it is always the *nearest ancestor's* definition that
    // runs, never the class's own definition (a class's own override only
    // matters later, if and when something else subclasses *it*). This
    // lookup is therefore unconditional: it runs regardless of whether the
    // current class also defines its own `__init_subclass__`, and it always
    // validates the nearest ancestor's body, never the current class's own.
    //
    // (#854 fixed two bugs in an earlier `if`/`else if` version of this
    // guard that instead validated the *current* class's own body whenever
    // the current class also defined an override: doing so (a) let a
    // side-effecting ancestor hook slip through unrejected whenever the
    // subclass added any override at all, however trivial, and (b) as a
    // consequence, missed detecting a `@classmethod`-decorated ancestor
    // hook, since a decorated `__init_subclass__` is registered in a
    // class's `class_methods` table, not `methods`, and the old own-hook
    // check queried only `methods`. The ancestor lookup below scans raw AST
    // `Stmt::FunctionDef` nodes by name -- decorators live on
    // `StmtFunctionDef.decorator_list` and never change the AST variant, so
    // this lookup was already decorator-agnostic; unifying to a single
    // unconditional ancestor lookup closes both gaps as one structural
    // change rather than two independent fixes. See D-214.)
    if let Some(base_ast) = mro.iter().skip(1).find_map(|mro_class| {
        // Only a base this crate can still see the original source of (an
        // earlier class in this same module, per `base_class_asts`) can be
        // re-validated; a base this crate cannot introspect (e.g. a
        // synthetic builtin-exception class, which is never in
        // `base_class_asts`) is left unrestricted, matching this file's
        // existing posture elsewhere. Looking the base up directly in
        // `base_class_asts` (rather than via `defined_classes` first) means
        // a base this crate cannot introspect is filtered out by the same
        // `?` that filters out one lacking `__init_subclass__`, instead of
        // needing a second, separately-unreachable fallback path.
        let (_, base_ast) = base_class_asts.iter().find(|(name, _)| name == mro_class)?;
        base_ast
            .body
            .iter()
            .any(|s| matches!(s, Stmt::FunctionDef(fd) if fd.name.as_str() == "__init_subclass__"))
            .then_some(*base_ast)
    }) {
        // #585 / #854: the nearest ancestor's `__init_subclass__` must be
        // statically evaluable, since CPython invokes it automatically at
        // *this* subclass's creation -- unlike a base class that is merely
        // defined and never subclassed, which must stay legal to compile.
        // Find the last `__init_subclass__` def in the ancestor's body
        // (rebind semantics: the last definition is the one actually bound)
        // and validate its body against this subclass's own creation site
        // (`def.range`), matching D-213's "Alternatives" reasoning that the
        // diagnostic belongs where CPython actually invokes the hook.
        for stmt in base_ast.body.iter().rev() {
            match stmt {
                Stmt::FunctionDef(fd) if fd.name.as_str() == "__init_subclass__" => {
                    validate_init_subclass_body(&fd.body, def.range.into())?;
                    break;
                }
                _ => {}
            }
        }
    }
    let class_def = HirClassDef {
        class_attrs,
        exception_type_tag: None,
        name: class_name,
        bases,
        mro,
        attrs,
        methods,
        properties,
        static_methods,
        class_methods,
        type_param,
        is_enum: false,
        implicit_object_init,
        method_defaults,
        enum_members,
        is_dataclass,
        dataclass_fields,
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: all_abstract_methods,
        is_abstract,
    };
    // #969: the layout gate runs last, on the finished `HirClassDef` -- it
    // needs `attrs` complete (a `@dataclass` fills it from the merged field
    // list well below `walk_class_body`) and every base's own `attrs` final
    // (a base is always defined earlier in the module, so it is).
    validate_mro_slot_layout(&class_def, defined_classes, def.range.into())?;
    // #1337 (WI-6b): a user dunder a raised exception value would ignore.
    exception_dunders::reject_ignored_exception_dunders(
        &class_def,
        defined_classes,
        def.range.into(),
    )?;
    Ok((class_def, items))
}

/// #378 (PR-18): Returns `true` if `ty` is a scalar slot type -- one that
/// fits in the single `i64` word per attribute slot that D-154's class-
/// instance layout uses: `int`/`float`/`bool`/`str`, plus `Ty::Param` for
/// PEP 695 generic classes, where the type parameter is substituted with a
/// concrete scalar at monomorphization time. A hand-written `__init__`
/// admits a wider set -- `init_slot::slot_ty_from_init_rhs` also accepts a
/// `list[int]`/`dict[str, int]` parameter (#1262), stored as a pointer
/// word -- but this narrower predicate still governs dataclass fields and
/// class-body attributes. A
/// dataclass field with a non-scalar type (`list[T]`, `dict[K, V]`,
/// `set[T]`, `tuple[...]`, `None`, or a class instance including a self-
/// referential `next: Node`/`next: Self`) is rejected at HIR-lowering
/// time with `C0001` before it can reach codegen and panic.
fn is_scalar_slot_type(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Param(_))
}

/// #435 (Part B), extended by #585 and unified by #854: Validates that an
/// `__init_subclass__` method body is statically evaluable — only a `pass`
/// body, an empty body, or a body consisting solely of a docstring (a bare
/// string-literal expression statement) is accepted. Any other statement (a
/// `print` call, an assignment, a return, etc.) is rejected with `C0001`,
/// since pycc's compile-time class-creation model has no mechanism to run
/// side-effecting statements at class-definition time.
///
/// After #854's unification of the call site's own guard (see its comment),
/// the body validated here is always the nearest MRO ancestor's, never the
/// current class's own, and `range` is always the subclass's own creation
/// site (`def.range`) — the point at which CPython actually invokes the
/// inherited hook, matching D-213's "Alternatives" reasoning.
fn validate_init_subclass_body(
    body: &[Stmt],
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    // After #854's unification (see the call site's own comment), the body
    // validated here is always the nearest MRO ancestor's, never the
    // current class's own -- so there is exactly one message variant
    // needed. The wording keeps the "inherited" word (an existing test,
    // `subclass_without_own_init_subclass_rejects_side_effecting_inherited_body`,
    // asserts on it), but no longer implies the subclass itself has no
    // override of its own, since this guard now also fires when the
    // subclass *does* override the hook.
    let message = "the nearest `__init_subclass__` hook in this class's MRO must be statically \
         evaluable (only `pass` or a docstring is supported in this version) — CPython invokes \
         that inherited hook when this subclass is created, and pycc cannot run its \
         side-effecting statements at that point yet";
    for stmt in body {
        match stmt {
            Stmt::Pass(_) => continue,
            // A docstring: a bare string-literal expression statement. This
            // is the only expression statement accepted in
            // `__init_subclass__` — it has no side effects.
            Stmt::Expr(expr_stmt) => {
                if matches!(*expr_stmt.value, Expr::StringLiteral(_)) {
                    continue;
                }
                return Err(unsupported(message, range));
            }
            _ => {
                return Err(unsupported(message, range));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{HirClassDef, HirExpr, HirItem, HirStmt, Ty, lower_checked};

    pub(super) fn assert_c0001(source: &str) {
        let module = crate::pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    }

    pub(super) fn lower_ok(source: &str) -> crate::HirModule {
        // `.expect(...)`, not `.unwrap_or_else(|e| panic!(...))`: the
        // latter's closure body is its own hand-written region, never
        // executed on this helper's own happy path (every call site
        // expects success) -- this crate's own established coverage-gate
        // convention (see `init_slot::slot_ty_from_init_rhs`'s own doc
        // comment) is `.expect()`, whose panic path
        // lives in libcore, outside this crate's instrumented regions.
        let module = crate::pycc_parser_test_helper::parse(source);
        lower_checked(&module).expect("test fixture should lower successfully")
    }

    // -- lower_class: class-level shape checks -----------------------------

    #[test]
    fn a_decorated_class_is_unsupported() {
        assert_c0001(
            "@some_decorator\nclass C:\n    def __init__(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_generic_class_with_one_type_param_is_supported() {
        // PEP 695 (#387): `class C[T]:` with exactly one type parameter is
        // now supported. The type parameter `T` is recorded in
        // `HirClassDef::type_param` for later monomorphization.
        let hir =
            lower_ok("class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n");
        assert_eq!(hir.class_defs.len(), 1);
        assert_eq!(hir.class_defs[0].1.type_param, Some("T".to_string()));
    }

    #[test]
    fn a_generic_class_with_two_type_params_is_unsupported() {
        assert_c0001("class C[T, U]:\n    def __init__(self) -> None:\n        return\n");
    }

    #[test]
    fn a_class_with_a_keyword_argument_is_unsupported() {
        assert_c0001("class C(metaclass=Meta):\n    def __init__(self) -> None:\n        return\n");
    }

    #[test]
    fn a_class_with_empty_parens_and_no_bases_is_supported() {
        // `class C():` is syntactically distinct from `class C:` (upstream
        // parses `arguments: Some(Arguments { args: [], keywords: [] })`)
        // but semantically equivalent -- no inheritance -- so it must not
        // be rejected merely because `arguments` is `Some(_)`.
        let hir = lower_ok("class C():\n    def __init__(self) -> None:\n        return\n");
        assert_eq!(hir.class_defs.len(), 1);
    }

    #[test]
    fn redefining_a_class_name_at_module_scope_is_unsupported() {
        // D-154 Part 1's own post-merge review finding: two module-level
        // classes sharing a name would each lower their own `__init__` to
        // the identical mangled `<Name>.__init__` function name, colliding
        // silently in `pycc_types`'/`pycc_mir`'s own `HashMap`-collected
        // class tables downstream rather than producing a clean diagnostic.
        // Mirrors `redefining_init_in_one_class_body_is_unsupported`
        // above, one level up (module scope rather than one class body).
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\nclass C:\n    def __init__(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_class_name_colliding_with_an_earlier_function_name_is_unsupported() {
        // D-068 review finding on #385: without this check, `class Foo`
        // below would silently, permanently shadow the earlier `def Foo()`
        // at every call site -- `pycc_types::Environment` checks
        // `env.lookup_class(callee)` before the ordinary function lookup
        // (`crates/pycc_types/src/lib.rs`), so `Foo()` would always resolve
        // to the class instantiation and the function would become
        // unreachable, with no diagnostic ever produced.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "def Foo() -> None:\n    return\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("class `Foo` collides with a function of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_function_name_colliding_with_an_earlier_class_name_is_unsupported() {
        // The reverse order of
        // `a_class_name_colliding_with_an_earlier_function_name_is_unsupported`
        // above: the class comes first, the function second.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class Foo:\n    def __init__(self) -> None:\n        return\ndef Foo() -> None:\n    return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("function `Foo` collides with a class of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_name_colliding_with_a_type_alias_is_unsupported() {
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "type Foo = int\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("class `Foo` collides with a type alias of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_name_colliding_with_an_earlier_value_binding_is_unsupported() {
        // #974 round 6: a class statement has no `HirItem` of its own, so
        // the type checker's sequential top-level pass never re-binds the
        // name back to the class -- a later read would resolve through the
        // stale value. Reject the collision here instead.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "Foo = 1\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("class `Foo` collides with a value of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_value_binding_of_another_name_does_not_block_a_later_class() {
        // The negative direction of the check above: the earlier top-level
        // statement binds a *different* name, so the class lowers normally.
        let module = lower_checked(&crate::pycc_parser_test_helper::parse(
            "Bar = 1\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
        ))
        .expect("an unrelated value binding must not block a later class");
        assert!(module.class_defs.iter().any(|(name, _)| name == "Foo"));
    }

    #[test]
    fn a_class_name_colliding_with_a_module_import_is_unsupported() {
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "import math\nclass math:\n    def __init__(self) -> None:\n        return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("class `math` collides with an import of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_name_colliding_with_a_symbol_import_is_unsupported() {
        // Exercises `import_local_name`'s other `ImportBinding` variant
        // (`Symbol`, from `from <module> import <name>`) -- the test above
        // only ever reaches the `Module` variant (`import math`), leaving
        // the `Symbol` arm of `import_local_name`'s own or-pattern
        // structurally unreachable under D-014's 100%-region coverage gate.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "from math import sqrt\nclass sqrt:\n    def __init__(self) -> None:\n        return\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("class `sqrt` collides with an import of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_type_alias_colliding_with_an_earlier_class_name_is_unsupported() {
        // The reverse order of
        // `a_class_name_colliding_with_a_type_alias_is_unsupported` above:
        // the class comes first, the `type X = ...` alias second -- D-068
        // review finding on #385's second round: without this check, `type
        // Foo = int` below would silently establish a second, alias-shaped
        // `Foo` binding alongside the class, with no diagnostic.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class Foo:\n    def __init__(self) -> None:\n        return\ntype Foo = int\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("type alias `Foo` collides with a class of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_legacy_type_alias_colliding_with_an_earlier_class_name_is_unsupported() {
        // Same reverse-direction collision as
        // `a_type_alias_colliding_with_an_earlier_class_name_is_unsupported`
        // above, exercised through the legacy `X: TypeAlias = <expr>`
        // spelling (`lower_legacy_type_alias_ann_assign`) instead of `type X
        // = <expr>` (`lower_type_alias_stmt`) -- the two are lowered by
        // independent functions in `import.rs`, each needing its own check and
        // its own regression test.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class Foo:\n    def __init__(self) -> None:\n        return\nFoo: TypeAlias = int\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("type alias `Foo` collides with a class of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_module_import_colliding_with_an_earlier_class_name_is_unsupported() {
        // The reverse order of
        // `a_class_name_colliding_with_a_module_import_is_unsupported`
        // above: the class comes first, `import math` second.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class math:\n    def __init__(self) -> None:\n        return\nimport math\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("import `math` collides with a class of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_symbol_import_colliding_with_an_earlier_class_name_is_unsupported() {
        // The reverse order of
        // `a_class_name_colliding_with_a_symbol_import_is_unsupported`
        // above: the class comes first, `from math import sqrt` second.
        // Also exercises `bound.iter().map(import_local_name).find(..)`'s
        // own multi-binding search (`from math import pi, sqrt` binds two
        // names in one statement) finding the colliding name when it is not
        // the first one bound.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class sqrt:\n    def __init__(self) -> None:\n        return\nfrom math import pi, sqrt\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("import `sqrt` collides with a class of the same name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_without_init_gets_a_synthesized_zero_argument_constructor() {
        // #912 (D-225): a class that declares no `__init__` and inherits
        // none is no longer `C0001` -- `ensure_init` synthesizes
        // `def __init__(self) -> None: pass` for it.
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def foo(self) -> None:\n        return\n",
        );
        let hir = lower_checked(&module).unwrap();
        let (_, class_def) = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "C")
            .expect("class `C` is lowered");
        assert_eq!(
            class_def
                .methods
                .iter()
                .find(|(mn, _)| mn == "__init__")
                .map(|(_, mangled)| mangled.as_str()),
            Some("C.__init__"),
            "the synthesized constructor is registered under the class's own name"
        );
        let init = hir
            .items
            .iter()
            .find_map(|item| match item {
                HirItem::Function {
                    name, params, body, ..
                } if name == "C.__init__" => Some((params.clone(), body.clone())),
                _ => None,
            })
            .expect("the synthesized `C.__init__` item is emitted");
        assert_eq!(init.0.len(), 1, "only `self` is a parameter: {:?}", init.0);
        assert_eq!(init.0[0].0, "self");
        assert!(init.1.is_empty(), "the body is empty: {:?}", init.1);
    }

    // -- lower_method: method-shape checks ----------------------------------

    #[test]
    fn a_positional_only_method_parameter_lowers_successfully() {
        // PEP 570 (#383): positional-only parameters on methods are now
        // lowered. `self` remains the first parameter; posonlyargs follow
        // it, before ordinary `args`. The class also has a regular method
        // `foo` to exercise the `find_map` None branch (items before the
        // target function that don't match).
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def foo(self) -> None:\n        return\n    def __init__(self, x: int, /, y: int) -> None:\n        return\n",
        );
        let hir = lower_checked(&module).unwrap();
        // Find the __init__ function item and verify its params.
        let init = hir
            .items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, params, .. } if name == "C.__init__" => {
                    Some(params.clone())
                }
                _ => None,
            })
            .next()
            .expect("C.__init__ must be lowered");
        assert_eq!(
            init,
            vec![
                ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
            ]
        );
    }

    #[test]
    fn a_positional_only_classmethod_parameter_lowers_successfully() {
        // PEP 570 (#383): positional-only parameters on `@classmethod` are
        // now lowered. `cls` remains the first parameter; posonlyargs follow
        // it, before ordinary `args`. The mangled name uses a `.classmethod`
        // suffix (#436).
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def m(cls, x: int, /, y: int) -> None:\n        return\n",
        );
        let hir = lower_checked(&module).unwrap();
        let m = hir
            .items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, params, .. } if name == "C.m.classmethod" => {
                    Some(params.clone())
                }
                _ => None,
            })
            .next()
            .expect("C.m.classmethod must be lowered");
        assert_eq!(
            m,
            vec![
                ("cls".to_string(), Ty::Instance(Box::new("C".to_string()))),
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
            ]
        );
    }

    #[test]
    fn a_positional_only_staticmethod_parameter_lowers_successfully() {
        // PEP 570 (#383): positional-only parameters on `@staticmethod` are
        // now lowered. For static methods, posonlyargs come first (no
        // implicit `self`/`cls`), then ordinary `args`. The mangled name
        // uses a `.static` suffix (#436).
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def m(x: int, /, y: int) -> None:\n        return\n",
        );
        let hir = lower_checked(&module).unwrap();
        let m = hir
            .items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, params, .. } if name == "C.m.static" => {
                    Some(params.clone())
                }
                _ => None,
            })
            .next()
            .expect("C.m.static must be lowered");
        assert_eq!(
            m,
            vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
        );
    }

    #[test]
    fn a_positional_only_method_parameter_may_have_a_default() {
        // PEP 570 (#383) with the method part of #1140: a defaulted
        // positional-only parameter lowers on every method kind, exactly as
        // on a module-level `def` (`class::method_tests` pins what is
        // recorded).
        for source in [
            "class C:\n    def __init__(self, x: int = 0, /) -> None:\n        return\n",
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def m(cls, x: int = 0, /) -> None:\n        return\n",
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def m(x: int = 0, /) -> None:\n        return\n",
        ] {
            let module = lower_ok(source);
            assert_eq!(module.class_defs[0].1.method_defaults.len(), 1, "{source}");
        }
    }

    #[test]
    fn a_vararg_method_parameter_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self, *args) -> None:\n        return\n");
    }

    #[test]
    fn a_keyword_only_method_parameter_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self, *, x: int) -> None:\n        return\n");
    }

    #[test]
    fn a_kwarg_method_parameter_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self, **kwargs) -> None:\n        return\n");
    }

    #[test]
    fn a_method_with_no_parameters_at_all_is_unsupported() {
        assert_c0001("class C:\n    def __init__() -> None:\n        return\n");
    }

    #[test]
    fn a_method_whose_receiver_is_not_spelled_self_is_accepted_and_aliased() {
        // #1181 removed the rule this test used to assert. The receiver is
        // lowered under the canonical name `self`, and the body opens with
        // the `this = self` alias -- the two facts the whole change rests on,
        // asserted here on the HIR directly rather than only through the CLI
        // (`tests/issue_1181_receiver_spelling.rs` covers the behavior).
        let module = lower_ok("class C:\n    def __init__(this) -> None:\n        return\n");
        assert_eq!(
            module.items,
            vec![crate::HirItem::Function {
                name: "C.__init__".to_string(),
                params: vec![("self".to_string(), Ty::Instance(Box::new("C".to_string())))],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::Assign {
                        target: "this".to_string(),
                        value: crate::HirExpr::Name("self".to_string()),
                    },
                    HirStmt::Return(None),
                ],
            }],
            "the receiver is lowered canonically and the body opens with its alias"
        );
    }

    #[test]
    fn a_method_with_no_receiver_at_all_is_unsupported() {
        assert_c0001("class C:\n    def __init__() -> None:\n        return\n");
    }

    #[test]
    fn a_self_parameter_with_a_default_value_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self=None) -> None:\n        return\n");
    }

    #[test]
    fn an_annotated_self_parameter_is_unsupported() {
        assert_c0001("class C:\n    def __init__(self: C) -> None:\n        return\n");
    }

    #[test]
    fn an_init_parameter_without_an_annotation_is_unsupported() {
        // Unlike an ordinary private (`_`-prefixed) top-level function or
        // method, `__init__`'s own parameters always require an
        // annotation, regardless of D-038's usual public-name-only rule --
        // see `lower_method`'s own doc comment for why. This is a `T0001`
        // missing-annotation diagnostic (the same code an ordinary public
        // function's own missing annotation produces), not `C0001`.
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self, x) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "T0001");
    }

    #[test]
    fn a_private_method_parameter_without_an_annotation_is_still_permitted() {
        // Contrast with the test above: an ordinary (non-`__init__`)
        // private method still follows the plain D-038 rule (an unannotated
        // parameter is only rejected for a *public* name).
        let hir = lower_ok(
            "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n    def _helper(self, y) -> None:\n        return\n",
        );
        assert_eq!(hir.class_defs.len(), 1);
    }

    #[test]
    fn a_method_with_an_unsupported_return_annotation_propagates_the_error() {
        // Exercises `lower_method`'s own `?` on
        // `crate::lower_return_annotation`'s error path -- distinct from
        // every other method-shape test above, which only ever exercise a
        // parameter-side rejection.
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    def bump(self) -> Frobnicate:\n        return\n",
        );
    }

    #[test]
    fn a_method_signature_and_self_ty_lower_correctly() {
        let hir = lower_ok(
            "class Point:\n    def __init__(self, x: int, y: int) -> None:\n        self.x = x\n        self.y = y\n",
        );
        assert_eq!(hir.class_defs.len(), 1);
        let (name, class_def) = &hir.class_defs[0];
        assert_eq!(name, "Point");
        assert_eq!(
            *class_def,
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),],
                methods: vec![("__init__".to_string(), "Point.__init__".to_string())],
                properties: Vec::new(),
                type_param: None,
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            }
        );
        // Direct value comparison, not a `let PATTERN = .. else { panic!(..) }`
        // destructure -- this crate's own established coverage-gate
        // convention (see `pycc_hir::lib.rs`'s
        // `re_exported_grammar_types_resolve_and_have_the_expected_shape`-
        // adjacent precedent): a hand-written panic arm never taken on the
        // happy path is a permanently uncovered region under D-014's
        // 100%-region gate.
        assert_eq!(
            hir.items[0],
            HirItem::Function {
                name: "Point.__init__".to_string(),
                params: vec![
                    (
                        "self".to_string(),
                        Ty::Instance(Box::new("Point".to_string()))
                    ),
                    ("x".to_string(), Ty::Int),
                    ("y".to_string(), Ty::Int),
                ],
                return_ty: Ty::None,
                body: vec![
                    HirStmt::AttrSet {
                        base: HirExpr::Name("self".to_string()),
                        attr: "x".to_string(),
                        value: HirExpr::Name("x".to_string()),
                    },
                    HirStmt::AttrSet {
                        base: HirExpr::Name("self".to_string()),
                        attr: "y".to_string(),
                        value: HirExpr::Name("y".to_string()),
                    },
                ],
            }
        );
    }

    // -- @property lowering (#377) ------------------------------------------

    #[test]
    fn a_property_getter_lowers_into_the_property_table() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.properties.len(), 1);
        assert_eq!(
            class_def.properties[0],
            crate::PropertyDef {
                name: "x".to_string(),
                getter: "C.x".to_string(),
                setter: None,
            }
        );
        // The getter is NOT in the methods table (accessed via attribute
        // syntax, not method-call syntax).
        assert!(!class_def.methods.iter().any(|(name, _)| name == "x"));
    }

    #[test]
    fn a_property_getter_and_setter_lower_into_one_property_entry() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.properties.len(), 1);
        assert_eq!(
            class_def.properties[0],
            crate::PropertyDef {
                name: "x".to_string(),
                getter: "C.x".to_string(),
                setter: Some("C.x.setter".to_string()),
            }
        );
    }

    #[test]
    fn a_property_getter_function_is_emitted_with_the_mangled_name() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n",
        );
        assert!(
            hir.items
                .iter()
                .any(|item| matches!(item, HirItem::Function { name, .. } if name == "C.x")),
            "getter function `C.x` should be in items"
        );
    }

    #[test]
    fn a_property_setter_function_is_emitted_with_the_setter_mangled_name() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n",
        );
        assert!(
            hir.items
                .iter()
                .any(|item| matches!(item, HirItem::Function { name, .. } if name == "C.x.setter")),
            "setter function `C.x.setter` should be in items"
        );
    }

    #[test]
    fn an_unrecognized_method_decorator_is_still_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @foo\n    def f(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn multiple_decorators_on_a_method_are_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    @foo\n    def f(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_property_getter_with_extra_parameters_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self, extra: int) -> int:\n        return 0\n",
        );
    }

    #[test]
    fn a_property_setter_with_no_value_parameter_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_property_setter_with_two_value_parameters_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self, a: int, b: int) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_setter_without_a_preceding_getter_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("requires a preceding `@property` getter"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_duplicate_property_getter_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @property\n    def x(self) -> int:\n        return 1\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("already defined"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_duplicate_property_setter_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("already defined"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_setter_decorator_name_not_matching_the_method_name_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def y(self, v: int) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("must decorate a method named `x`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_non_setter_attribute_decorator_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @x.deleter\n    def x(self) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_call_shaped_decorator_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property()\n    def x(self) -> int:\n        return 0\n",
        );
    }

    #[test]
    fn a_setter_decorator_with_a_chained_attribute_base_is_rejected() {
        // `@a.b.setter` -- the decorator expression is an attribute access
        // (`(a.b).setter`) whose own base is *not* a bare `Expr::Name`
        // (it's `Expr::Attribute(Name("a"), "b")`). This exercises
        // `classify_decorator`'s `let Expr::Name(base_name) =
        // attr.value.as_ref() else { ... }` rejection arm -- distinct from
        // `a_non_setter_attribute_decorator_is_rejected` above, which
        // exercises the `attr.attr != "setter"` arm, and from
        // `a_setter_decorator_name_not_matching_the_method_name_is_rejected`,
        // which exercises the name-mismatch arm.
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @a.b.setter\n    def x(self, v: int) -> None:\n        return\n",
        );
    }

    // -- #436: @staticmethod / @classmethod lowering ------------------------

    #[test]
    fn a_static_method_lowers_into_the_static_methods_table() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(
            class_def.static_methods,
            vec![("create".to_string(), "C.create.static".to_string())]
        );
        // The static method is NOT in the methods table.
        assert!(!class_def.methods.iter().any(|(name, _)| name == "create"));
    }

    #[test]
    fn a_class_method_lowers_into_the_class_methods_table() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(
            class_def.class_methods,
            vec![("greet".to_string(), "C.greet.classmethod".to_string())]
        );
        // The class method is NOT in the methods table.
        assert!(!class_def.methods.iter().any(|(name, _)| name == "greet"));
    }

    #[test]
    fn a_static_method_function_is_emitted_with_the_static_mangled_name() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
        );
        assert!(
            hir.items.iter().any(|item| matches!(
                item,
                HirItem::Function { name, .. } if name == "C.create.static"
            )),
            "static method function `C.create.static` should be in items"
        );
    }

    #[test]
    fn a_class_method_function_is_emitted_with_the_classmethod_mangled_name() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
        );
        assert!(
            hir.items.iter().any(|item| matches!(
                item,
                HirItem::Function { name, .. } if name == "C.greet.classmethod"
            )),
            "class method function `C.greet.classmethod` should be in items"
        );
    }

    #[test]
    fn a_static_method_has_no_implicit_self_parameter() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
        );
        let fn_item = hir
            .items
            .iter()
            .find_map(|item| match item {
                HirItem::Function { name, params, .. } if name == "C.create.static" => Some(params),
                _ => None,
            })
            .expect("C.create.static should be in items");
        // No `self` — the parameter list is exactly what the user wrote.
        assert_eq!(*fn_item, vec![("x".to_string(), Ty::Int)]);
    }

    #[test]
    fn a_class_method_has_implicit_cls_typed_as_instance() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
        );
        let fn_item = hir
            .items
            .iter()
            .find_map(|item| match item {
                HirItem::Function { name, params, .. } if name == "C.greet.classmethod" => {
                    Some(params)
                }
                _ => None,
            })
            .expect("C.greet.classmethod should be in items");
        assert_eq!(
            *fn_item,
            vec![
                ("cls".to_string(), Ty::Instance(Box::new("C".to_string()))),
                ("x".to_string(), Ty::Int),
            ]
        );
    }

    #[test]
    fn classmethod_on_init_is_rejected() {
        assert_c0001(
            "class C:\n    @classmethod\n    def __init__(cls) -> None:\n        return\n",
        );
    }

    #[test]
    fn a_class_method_without_cls_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f() -> int:\n        return 1\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("must take `cls`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_method_with_a_non_cls_first_parameter_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(self, x: int) -> int:\n        return x\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("must be named `cls`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_method_with_annotated_cls_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls: int, x: int) -> int:\n        return x\n",
        );
    }

    #[test]
    fn a_class_method_with_cls_default_is_rejected() {
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls = 1) -> int:\n        return 1\n",
        );
    }

    #[test]
    fn a_class_method_with_an_unsupported_parameter_annotation_is_rejected() {
        // Exercises the `?` error propagation on `lower_arg_list` for the
        // non-`cls` parameters of a `@classmethod` (distinct from the `cls`
        // validation checks above, which run before `lower_arg_list`).
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls, x: Frobnicate) -> int:\n        return 1\n",
        );
    }

    #[test]
    fn a_static_method_with_an_unsupported_parameter_annotation_is_rejected() {
        // Exercises the `?` error propagation on `lower_arg_list` for a
        // `@staticmethod`'s parameter list (which has no `self`/`cls`
        // preprocessing, so `lower_arg_list` is the only validation path).
        assert_c0001(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def f(x: Frobnicate) -> int:\n        return 1\n",
        );
    }

    #[test]
    fn a_static_method_redefinition_rebinds() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def f(x: int) -> int:\n        return x\n    @staticmethod\n    def f(x: int) -> int:\n        return x + 1\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        // Rebind: one entry, not two.
        assert_eq!(class_def.static_methods.len(), 1);
        assert_eq!(
            class_def.static_methods[0],
            ("f".to_string(), "C.f.static".to_string())
        );
    }

    #[test]
    fn a_class_method_redefinition_rebinds() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls, x: int) -> int:\n        return x\n    @classmethod\n    def f(cls, x: int) -> int:\n        return x + 1\n",
        );
        let (_, class_def) = &hir.class_defs[0];
        assert_eq!(class_def.class_methods.len(), 1);
        assert_eq!(
            class_def.class_methods[0],
            ("f".to_string(), "C.f.classmethod".to_string())
        );
    }

    // -- Self / generic / override / init_subclass -------------------------

    // PEP 673 (#387 Part 1): `Self` as a method return-type annotation
    // resolves to the class's own instance type at HIR-lowering time.
    #[test]
    fn self_return_annotation_resolves_to_class_instance() {
        let hir = lower_ok(
            "class C:\n    def __init__(self) -> None:\n        return\n    def clone(self) -> Self:\n        return self\n",
        );
        assert_eq!(hir.class_defs.len(), 1);
        // The `clone` method should be lowered as a function with return
        // type `Ty::Instance("C")`.
        let clone = hir.items.iter().find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == "C.clone" => Some(return_ty),
            _ => None,
        });
        assert_eq!(
            clone,
            Some(&Ty::Instance(Box::new("C".to_string()))),
            "Self return annotation should resolve to Ty::Instance(\"C\")"
        );
    }

    // PEP 673 (#387 Part 1): `Self` as a method parameter annotation also
    // resolves to the class's own instance type.
    #[test]
    fn self_param_annotation_resolves_to_class_instance() {
        let hir = lower_ok(
            "class C:\n    def __init__(self, value: int) -> None:\n        self.value = value\n    def merge(self, other: Self) -> None:\n        self.value = other.value\n",
        );
        // The `merge` method's `other` parameter should be typed
        // `Ty::Instance("C")`.
        let merge = hir.items.iter().find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.merge" => Some(params.clone()),
            _ => None,
        });
        let merge = merge.expect("C.merge should exist");
        // params[0] is `self`, params[1] is `other`
        assert_eq!(merge[1].1, Ty::Instance(Box::new("C".to_string())));
    }

    // PEP 649/749 (#387 Part 2): self-referential deferred annotations. A
    // class's method may use the class's own name as a parameter type
    // annotation, even though the class is not fully defined at that point
    // in source.
    #[test]
    fn class_name_in_own_method_annotation_resolves_to_instance() {
        let hir = lower_ok(
            "class Node:\n    def __init__(self, value: int) -> None:\n        self.value = value\n    def update(self, other: Node) -> None:\n        self.value = other.value\n",
        );
        let update = hir.items.iter().find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "Node.update" => Some(params.clone()),
            _ => None,
        });
        let update = update.expect("Node.update should exist");
        // params[1] is `other` with type `Ty::Instance("Node")`
        assert_eq!(update[1].1, Ty::Instance(Box::new("Node".to_string())));
    }

    // PEP 649/749 (#387 Part 2): the class name also works as a return
    // type annotation in its own methods.
    #[test]
    fn class_name_as_own_return_annotation_resolves_to_instance() {
        let hir = lower_ok(
            "class Builder:\n    def __init__(self) -> None:\n        self.count = 0\n    def clone(self) -> Builder:\n        return self\n",
        );
        let clone = hir.items.iter().find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == "Builder.clone" => Some(return_ty.clone()),
            _ => None,
        });
        assert_eq!(clone, Some(Ty::Instance(Box::new("Builder".to_string()))),);
    }

    // PEP 649/749 (#387 Part 2, Bug 4 fix): a local `AnnAssign` *inside* a
    // method body (e.g. `other: Node = self`) must also resolve the class
    // name to `Ty::Instance`. Before the fix, `lower_body` was called with
    // `class_name=None` from `lower_method`, so the class name was
    // unresolvable in statement-body annotations (C0001).
    #[test]
    fn class_name_in_method_body_local_annotation_resolves_to_instance() {
        let hir = lower_ok(
            "class Node:\n    def __init__(self) -> None:\n        self.x = 0\n    def next(self) -> Node:\n        other: Node = self\n        return other\n",
        );
        let next = hir.items.iter().find_map(|item| match item {
            HirItem::Function { name, body, .. } if name == "Node.next" => Some(body.clone()),
            _ => None,
        });
        let next = next.expect("Node.next should exist");
        // body[0] is the `other: Node = self` AnnAssign. Use `matches!` with
        // a guard (same pattern as `check_and_resolve_monomorphizes_a_generic_
        // class_with_self_typed_method` in pycc_types) to avoid an uncovered
        // `panic!`/`unreachable!` arm under the 100%-region coverage gate.
        assert!(matches!(
            &next[0],
            HirStmt::AnnAssign { annotation, .. }
            if annotation == &Ty::Instance(Box::new("Node".to_string()))
        ));
    }

    // PEP 695 (#387 Part 3): a generic class's __init__ parameter typed `T`
    // seeds an attribute slot with `Ty::Param("T")`, which is substituted
    // at monomorphization time.
    #[test]
    fn generic_class_init_param_seeds_param_typed_slot() {
        let hir =
            lower_ok("class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n");
        assert_eq!(hir.class_defs[0].1.type_param, Some("T".to_string()));
        assert_eq!(
            hir.class_defs[0].1.attrs,
            vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))]
        );
    }

    // PEP 695 (#387 Part 3): `TypeVarTuple` (`*Ts`) and `ParamSpec` (`**P`)
    // type parameters are not supported — `type_param_name`'s `?` error path
    // (the uncovered region at line 120) is exercised by both forms.
    #[test]
    fn a_generic_class_with_a_type_var_tuple_is_unsupported() {
        assert_c0001("class C[*Ts]:\n    def __init__(self) -> None:\n        return\n");
    }

    #[test]
    fn a_generic_class_with_a_param_spec_is_unsupported() {
        assert_c0001("class C[**P]:\n    def __init__(self) -> None:\n        return\n");
    }

    #[test]
    fn duplicate_bases_are_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def __init__(self) -> None:\n        return\nclass B(A, A):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("lists base `A` more than once"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_generic_class_with_base_classes_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class Base:\n    def __init__(self) -> None:\n        return\nclass C[T](Base):\n    def __init__(self, x: T) -> None:\n        self.x = x\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("generic class `C` with base classes"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn override_on_a_valid_method_lowers_successfully() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def f(self) -> int:\n        return 2\n",
        );
        let (_, b_def) = &hir.class_defs[1];
        assert!(b_def.methods.iter().any(|(name, _)| name == "f"));
    }

    #[test]
    fn override_on_a_nonexistent_method_is_t0031() {
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def g(self) -> int:\n        return 2\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "T0031");
        assert!(
            diagnostic.message.contains("does not override"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn override_on_a_property_is_valid() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\nclass B(A):\n    def __init__(self) -> None:\n        return\n    @override\n    def x(self) -> int:\n        return 42\n",
        );
        let (_, b_def) = &hir.class_defs[1];
        // The @override-decorated method `x` is a regular method (not a
        // property), and it overrides the property `x` from `A`.
        assert!(b_def.methods.iter().any(|(name, _)| name == "x"));
    }

    #[test]
    fn a_derived_class_without_init_inherits_base_init() {
        let hir = lower_ok(
            "class A:\n    def __init__(self, x: int) -> None:\n        self.x = x\nclass B(A):\n    def f(self) -> int:\n        return self.x\n",
        );
        let (_, b_def) = &hir.class_defs[1];
        // B has no __init__ of its own, but inherits A's.
        assert!(!b_def.methods.iter().any(|(name, _)| name == "__init__"));
        assert_eq!(b_def.mro, vec!["B".to_string(), "A".to_string()]);
    }

    #[test]
    fn a_class_without_init_and_without_base_init_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def f(self) -> int:\n        return 1\n",
        );
        // `B` has no `__init__` and its base `A` has one, so this should
        // succeed (B inherits A's `__init__`). But `B`'s own `f` method
        // means it has a valid class body. The test name is historical --
        // it originally tested `pass` being rejected, but #378 made `pass`
        // valid in class bodies. This now tests that a derived class with
        // a method but no `__init__` inherits the base's `__init__`.
        let hir = lower_checked(&module).unwrap();
        let b_def = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "B")
            .map(|(_, def)| def)
            .unwrap();
        assert!(
            !b_def.methods.iter().any(|(mn, _)| mn == "__init__"),
            "B should not have its own __init__"
        );
    }

    #[test]
    fn a_class_with_no_bases_and_no_init_synthesizes_one() {
        // #912 (D-225): the base-less counterpart of the case above -- with
        // nothing in the MRO to inherit from, the constructor is synthesized
        // rather than the class being rejected.
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    def f(self) -> int:\n        return 1\n",
        );
        let hir = lower_checked(&module).unwrap();
        let (_, class_def) = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "C")
            .expect("class `C` is lowered");
        assert!(
            class_def.methods.iter().any(|(mn, _)| mn == "__init__"),
            "the synthesized `__init__` joins the method table: {:?}",
            class_def.methods
        );
    }

    #[test]
    fn a_non_name_base_class_expression_is_unsupported() {
        // #432: `class C(SomeMod.Base):` parses `SomeMod.Base` as an
        // `Expr::Attribute`, not an `Expr::Name` -- only a bare name is
        // supported as a base class for now.
        let module = crate::pycc_parser_test_helper::parse(
            "class C(SomeMod.Base):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("a base class must be a bare name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    // -----------------------------------------------------------------------
    // #435 (Part B): __init_subclass__ validation unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn init_subclass_with_pass_body_in_subclass_of_base_with_init_subclass_is_accepted() {
        // A base class B defines `__init_subclass__` with `pass`, and a
        // subclass D also defines its own `__init_subclass__` (also
        // `pass`). #854: the unified guard never inspects D's own
        // override -- it validates B's (the nearest ancestor's) body,
        // which exercises the `Stmt::Pass` arm of
        // `validate_init_subclass_body`. Kept as `pass`/`pass` so this
        // fixture continues to demonstrate "D's own override, whatever its
        // shape, plays no role" as directly as possible.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
        );
        let hir = lower_checked(&module);
        assert!(hir.is_ok(), "pass body should be accepted");
    }

    #[test]
    fn init_subclass_with_docstring_in_subclass_of_base_with_init_subclass_is_accepted() {
        // #854: the unified guard validates the nearest ancestor's body,
        // never the subclass's own -- so the docstring is now placed on
        // B's `__init_subclass__` to exercise the `Stmt::Expr` +
        // `StringLiteral` arm; D's own docstring override is never
        // inspected by this guard at all.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        \"docstring\"\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
        );
        let hir = lower_checked(&module);
        assert!(hir.is_ok(), "docstring body should be accepted");
    }

    #[test]
    fn init_subclass_without_base_having_init_subclass_is_not_validated() {
        // A class defines `__init_subclass__` but no base class has it.
        // The unconditional nearest-MRO-ancestor `find_map` walk finds no
        // ancestor defining the hook and returns `None`, so validation does
        // NOT run and even a non-trivial body is accepted (it's just a
        // regular method).
        let module = crate::pycc_parser_test_helper::parse(
            "class D:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\n",
        );
        let hir = lower_checked(&module);
        assert!(
            hir.is_ok(),
            "non-trivial body without base init_subclass should be accepted"
        );
    }

    #[test]
    fn subclass_without_own_init_subclass_inherits_statically_evaluable_base_hook() {
        // A base class B defines `__init_subclass__` with a `pass` body,
        // and a subclass D does NOT define its own `__init_subclass__`.
        // D's methods don't include `__init_subclass__`, so this takes the
        // #585 inherited-hook path (re-validating B's body against D's
        // creation site) rather than the #435 own-hook path -- but since
        // B's body is statically evaluable, that re-validation still
        // succeeds.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let hir = lower_checked(&module);
        assert!(
            hir.is_ok(),
            "subclass inheriting a statically-evaluable __init_subclass__ should be accepted"
        );
    }

    #[test]
    fn subclass_without_own_init_subclass_rejects_side_effecting_inherited_body() {
        // #585: a base class B defines `__init_subclass__` with a
        // side-effecting body, and a subclass D inherits it unchanged
        // (does not override it). CPython would invoke B's hook when D is
        // created; pycc cannot run that side-effecting body at
        // class-creation time, so this must now be rejected at D's
        // creation site -- unlike `init_subclass_without_base_having_init_subclass_is_not_validated`,
        // where B is never subclassed and therefore stays legal.
        // B's `__init_subclass__` is deliberately followed by `__init__`
        // (not the last statement in the body) so the reverse walk over
        // `base_ast.body` also exercises its `_ => {}` catch-all arm before
        // finding the match.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init_subclass__(self) -> None:\n        print(1)\n    def __init__(self) -> None:\n        return\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("inherited"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn base_alone_never_subclassed_with_side_effecting_init_subclass_stays_legal() {
        // #585's boundary case, restated explicitly: a base class defining
        // `__init_subclass__` with a side-effecting body must still compile
        // when it is never subclassed in this module -- CPython never
        // invokes the hook until a subclass actually exists, so rejecting
        // it here (at the base's own definition) would over-reject a
        // legal, standalone class. This is the same program shape as
        // `init_subclass_without_base_having_init_subclass_is_not_validated`
        // above; kept as its own #585-focused test for direct traceability
        // to the issue's own boundary requirement.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\n",
        );
        let hir = lower_checked(&module);
        assert!(
            hir.is_ok(),
            "a base class's own side-effecting __init_subclass__ must stay legal when it is \
             never subclassed"
        );
    }

    // -----------------------------------------------------------------------
    // #854: unified nearest-ancestor guard -- new regression tests.
    //
    // Truth table for the four B/D triviality combinations (B = base's
    // `__init_subclass__`, D = subclass's own override, if any):
    //   B trivial   / D absent     -> accept (existing:
    //     `subclass_without_own_init_subclass_inherits_statically_evaluable_base_hook`)
    //   B side-effecting / D absent -> reject (existing:
    //     `subclass_without_own_init_subclass_rejects_side_effecting_inherited_body`)
    //   B trivial   / D trivial    -> accept (existing, above)
    //   B trivial   / D side-effecting -> accept (D's own override is never
    //     checked) -- Test D
    //   B side-effecting / D trivial    -> reject (D's trivial override does
    //     not save it -- B's body is still the one CPython invokes) -- Tests
    //     A and B
    // -----------------------------------------------------------------------

    #[test]
    fn side_effecting_base_hook_with_trivial_subclass_override_is_rejected() {
        // Test A: B's `__init_subclass__` does `return` (a `Stmt::Return`,
        // caught only by the `_ =>` catch-all arm in
        // `validate_init_subclass_body` -- every other rejecting test in
        // this suite uses a non-string `Stmt::Expr` such as `print(...)`,
        // which exercises a different arm). D(B) defines its own plain,
        // trivial override (`pass`). #854's bug: before the fix, D's own
        // trivial override made the (buggy) `if` branch validate D's body
        // instead of B's, silently accepting a program CPython would
        // reject at D's creation (B's side-effecting hook is the one
        // actually invoked). After the fix, B's body is what's validated
        // regardless of D's own override, so this is rejected.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        return\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("__init_subclass__"));
    }

    #[test]
    fn classmethod_decorated_ancestor_hook_with_trivial_subclass_override_is_rejected() {
        // Test B: the genuine #854 Gap-2 bug. B's `__init_subclass__` is
        // `@classmethod`-decorated and side-effecting (`print(1)`); D(B)
        // defines its own plain, trivial override (`pass`). Before the
        // fix, this was silently *accepted*: D's own hook is found in
        // `methods` (own-hook check true) -> enters the `if` branch ->
        // the `base_has_init_subclass` gate queries `cd.methods` on B,
        // finds nothing (a `@classmethod`-decorated hook lives in
        // `class_methods`, not `methods`) -> validation is skipped
        // entirely. CPython would invoke B's side-effecting hook at D's
        // creation. The unified guard scans B's raw AST by statement shape
        // (decorator-agnostic), so it now correctly finds and validates
        // B's body, rejecting this program.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        pass\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("__init_subclass__"));
    }

    #[test]
    fn classmethod_decorated_ancestor_hook_with_no_subclass_override_is_rejected() {
        // Test C: regression lock for classmethod-ancestor detection when
        // the subclass defines no override at all. B's `__init_subclass__`
        // is `@classmethod`-decorated and side-effecting; D(B) defines no
        // `__init_subclass__`. This shape was already correctly rejected
        // before #854 (the pre-existing `else if`/#585 ancestor-lookup
        // path was always decorator-agnostic, since it scans raw AST by
        // statement shape); this test locks that behavior through the
        // refactor.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("__init_subclass__"));
    }

    #[test]
    fn trivial_base_hook_with_classmethod_decorated_subclass_override_is_accepted() {
        // Test D: regression lock -- a subclass's own `@classmethod`-
        // decorated override is correctly ignored. B's `__init_subclass__`
        // is trivial (`pass`); D(B)'s own override is `@classmethod`-
        // decorated and side-effecting. D's own override, decorated or
        // not, is never the hook CPython invokes at D's own creation (only
        // relevant if something later subclasses D), so this must be
        // accepted -- B's trivial body is the one validated. This shape
        // was already correctly accepted before #854, for an unrelated,
        // accidental reason (D's own hook lives in `class_methods`, which
        // bypassed the old `methods`-table own-hook check entirely,
        // routing into the ancestor-lookup path that validates B's
        // trivial body); this test locks in the now-*intentional* correct
        // behavior through the refactor.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    @classmethod\n    def __init_subclass__(cls) -> None:\n        print(1)\n",
        );
        let hir = lower_checked(&module);
        assert!(
            hir.is_ok(),
            "a subclass's own classmethod-decorated __init_subclass__ override must never be \
             the body validated at its own creation"
        );
    }

    #[test]
    fn grandchild_validates_parents_own_override_not_grandparents_hook() {
        // Test E (revised from the posted plan -- see the D-214 write-up and
        // the session's own retrospective entry for the full reasoning):
        // the guard runs unconditionally at *every* class's own creation
        // against *that class's own* nearest ancestor, so a class sitting
        // under a side-effecting, introspectable hook is illegal at its own
        // creation regardless of what it overrides -- there is no way for a
        // legally-created three-level chain to have a side-effecting
        // grandparent shadowed by a trivial parent, since the parent's own
        // creation would already have been rejected by the grandparent's
        // hook. B's `__init_subclass__` is trivial (`pass`, satisfying D's
        // own creation, whose nearest ancestor is B); D(B)'s own override is
        // side-effecting (`print(1)`); Grandchild(D) defines no override.
        // Grandchild's ancestor lookup must hit D first (nearest ancestor
        // that defines the hook) and validate *D's own* body, not skip past
        // it to B's trivial one -- this locks in that a parent's own
        // override, not a grandparent's, is what a child's creation is
        // checked against.
        let module = crate::pycc_parser_test_helper::parse(
            "class B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass D(B):\n    def __init__(self) -> None:\n        super().__init__()\n    def __init_subclass__(self) -> None:\n        print(1)\nclass Grandchild(D):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("__init_subclass__"));
    }

    #[test]
    fn multiple_inheritance_nearest_mro_hook_wins_over_farther_side_effecting_one() {
        // Test E2: nearest-ancestor-in-MRO-order, not "any ancestor",
        // discriminated via multiple inheritance rather than a linear
        // chain (a linear chain cannot produce this shape -- see the
        // previous test's comment). `class D(M, B)` linearizes via C3 to
        // `[D, M, B, object]`. M's `__init_subclass__` is trivial (`pass`);
        // B's is side-effecting (`print(1)`); D defines no override of its
        // own. `skip(1)` must hit M first (nearer in D's own MRO) and
        // validate M's trivial body, never reaching B's side-effecting one
        // -- exactly the lookup CPython performs (`super(D, D)` searches
        // the MRO in order and stops at the first hit).
        let module = crate::pycc_parser_test_helper::parse(
            "class M:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        pass\nclass B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\nclass D(M, B):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let hir = lower_checked(&module);
        assert!(
            hir.is_ok(),
            "the nearer ancestor in MRO order (M) must be validated, not the farther one (B), \
             even though both define the hook"
        );
    }

    #[test]
    fn multiple_inheritance_skips_hookless_ancestor_to_reject_farther_side_effecting_one() {
        // Reject-direction sibling of
        // `multiple_inheritance_nearest_mro_hook_wins_over_farther_side_effecting_one`.
        // That test only proves the nearest hook-defining ancestor wins
        // when it happens to also be the immediate base -- every existing
        // fixture's `skip(1)` MRO candidate (position 1) either already
        // defines `__init_subclass__` or terminates the search entirely, so
        // a hypothetical regression that narrowed the lookup to just
        // `mro.get(1)` instead of a real "walk the whole MRO, skipping
        // classes that don't define the hook" search would still pass every
        // other test in this suite. This fixture closes that gap: `class M`
        // defines `__init__` only (introspectable, but no
        // `__init_subclass__` of its own -- the search must skip past it),
        // `class B` defines a side-effecting `__init_subclass__`, and
        // `class D(M, B)` linearizes via C3 to `[D, M, B, object]` with no
        // override of its own. The nearest-MRO-ancestor search must skip
        // hookless M and match B, rejecting on B's side-effecting body.
        let module = crate::pycc_parser_test_helper::parse(
            "class M:\n    def __init__(self) -> None:\n        return\nclass B:\n    def __init__(self) -> None:\n        return\n    def __init_subclass__(self) -> None:\n        print(1)\nclass D(M, B):\n    def __init__(self) -> None:\n        super().__init__()\n",
        );
        let err = lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("__init_subclass__"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn runtime_checkable_on_non_protocol_is_rejected_via_unit_test() {
        // Covers the `@runtime_checkable` on a non-protocol class error
        // path (lines 1082-1089) inside this crate's own unit-test binary.
        assert_c0001("@runtime_checkable\nclass C:\n    pass\n");
    }

    // -- #378 (PR-18): dataclass (PEP 557) lowering -----------------------

    #[test]
    fn a_dataclass_with_two_fields_lowers_successfully() {
        let hir = lower_ok("@dataclass\nclass Point:\n    x: int\n    y: int\n");
        assert_eq!(hir.class_defs.len(), 1);
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.is_dataclass);
        assert_eq!(
            class_def.dataclass_fields,
            vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
        );
        // __init__, __eq__, and __repr__ should be auto-generated.
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__init__"));
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__eq__"));
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__repr__"));
    }

    #[test]
    fn a_dataclass_with_docstring_lowers_successfully() {
        // #744: a class docstring (a bare string-literal expression
        // statement) is a no-op in a dataclass body, not a field or method.
        let hir = lower_ok("@dataclass\nclass Point:\n    \"A point.\"\n    x: int\n    y: int\n");
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.is_dataclass);
        assert_eq!(
            class_def.dataclass_fields,
            vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
        );
    }

    #[test]
    fn a_dataclass_with_a_non_leading_docstring_lowers_successfully() {
        // #744's guard has no position check: place the docstring after a
        // field to exercise the non-leading case directly.
        let hir = lower_ok("@dataclass\nclass Point:\n    x: int\n    \"A point.\"\n    y: int\n");
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.is_dataclass);
        assert_eq!(
            class_def.dataclass_fields,
            vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
        );
    }

    #[test]
    fn a_dataclass_transform_decorator_lowers_successfully() {
        let hir = lower_ok("@dataclass_transform()\nclass Point:\n    x: int\n");
        assert_eq!(hir.class_defs.len(), 1);
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.is_dataclass);
    }

    #[test]
    fn a_dataclass_with_inheritance_merges_parent_fields() {
        let hir = lower_ok(
            "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    b: int\n",
        );
        assert_eq!(hir.class_defs.len(), 2);
        let (_, derived_def) = &hir.class_defs[1];
        assert!(derived_def.is_dataclass);
        // Parent field `a` comes before child field `b`.
        assert_eq!(
            derived_def.dataclass_fields,
            vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int),]
        );
        assert_eq!(
            derived_def.attrs,
            vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int),]
        );
    }

    #[test]
    fn a_dataclass_child_redeclaring_a_parent_field_deduplicates() {
        // When a child dataclass redeclares a field already present in a
        // parent with the *same* type, the merge keeps the parent's field
        // and skips the child's duplicate rather than treating it as a
        // conflict (see the differing-type case rejected as T0052 below).
        let hir = lower_ok(
            "@dataclass\nclass Base:\n    a: int\n@dataclass\nclass Derived(Base):\n    a: int\n",
        );
        assert_eq!(hir.class_defs.len(), 2);
        let (_, derived_def) = &hir.class_defs[1];
        assert!(derived_def.is_dataclass);
        // The field `a` appears only once (from the parent).
        assert_eq!(
            derived_def.dataclass_fields,
            vec![("a".to_string(), Ty::Int)]
        );
    }

    #[test]
    fn a_dataclass_grandchild_merges_parent_and_grandparent_fields_with_dedup() {
        // A 3-level inheritance chain where the middle class redeclares a
        // grandparent field with the *same* type. When processing the
        // grandchild, the MRO-based merge iterates over the grandparent's
        // fields first (adding `x`), then the middle class's fields. The
        // middle class's `x` is already in `merged_fields` with a matching
        // type, so it is deduplicated rather than rejected. This covers the
        // same-type "already present" branch in the MRO-based parent-fields
        // merge loop.
        let hir = lower_ok(
            "@dataclass\nclass A:\n    x: int\n@dataclass\nclass B(A):\n    x: int\n\
             @dataclass\nclass C(B):\n    y: int\n",
        );
        assert_eq!(hir.class_defs.len(), 3);
        let (_, c_def) = &hir.class_defs[2];
        assert!(c_def.is_dataclass);
        // `x` (from A, deduplicated against B's redeclaration) comes before
        // `y` (C's own field).
        assert_eq!(
            c_def.dataclass_fields,
            vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
        );
    }

    #[test]
    fn a_dataclass_field_type_conflict_across_mro_is_rejected_via_unit_test() {
        // Mirrors `pycc_types::tests::dataclass_field_type_conflict_across_mro_is_rejected_during_hir_lowering`
        // inside this crate's own unit-test binary, working around
        // cargo-llvm-cov issue #276 (instantiation-merge gap between
        // the library and integration-test binaries): a dataclass field
        // redeclared with a *differing* type across the MRO must be
        // rejected with T0052, exercising the second merge loop's
        // conflicting-type arm.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "@dataclass\nclass Base:\n    v: int\n@dataclass\nclass Derived(Base):\n    v: str\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "T0052");
        assert!(
            diagnostic.message.contains('v')
                && diagnostic.message.contains("int")
                && diagnostic.message.contains("str"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_dataclass_field_type_conflict_between_two_bases_is_rejected_via_unit_test() {
        // Mirrors `pycc_types::tests::dataclass_field_type_conflict_between_two_bases_is_rejected_during_hir_lowering`
        // inside this crate's own unit-test binary, for the same
        // cargo-llvm-cov instantiation-merge reason as the sibling test
        // above: two independent dataclass bases declaring the same
        // field with differing types must be rejected with T0052,
        // exercising the first merge loop's conflicting-type arm.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "@dataclass\nclass A:\n    v: int\n@dataclass\nclass B:\n    v: str\n@dataclass\nclass Derived(A, B):\n    pass\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "T0052");
        assert!(
            diagnostic.message.contains('v')
                && diagnostic.message.contains("int")
                && diagnostic.message.contains("str"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_call_decorator_with_a_non_name_callee_is_rejected() {
        // `@some.attr()` -- a call whose callee is an attribute access, not
        // a bare name. `classify_class_decorator`'s `Expr::Call` arm hits
        // the `let Expr::Name(name) = call.func.as_ref() else { ... }`
        // rejection.
        assert_c0001("@some.attr()\nclass C:\n    x: int\n");
    }

    #[test]
    fn a_call_decorator_with_an_unknown_name_is_rejected() {
        // `@other()` -- a call whose callee is a bare name but not
        // `dataclass_transform` or `dataclass`.
        assert_c0001("@other()\nclass C:\n    x: int\n");
    }

    #[test]
    fn a_dataclass_with_duplicate_field_names_is_rejected() {
        assert_c0001("@dataclass\nclass C:\n    x: int\n    x: int\n");
    }

    #[test]
    fn a_dataclass_with_an_unsupported_annotation_is_rejected() {
        // `annotation_to_ty` returns an error for `undefined_type`, which
        // propagates through the `?` on the `annotation_to_ty(...)` call.
        assert_c0001("@dataclass\nclass C:\n    x: undefined_type\n");
    }

    #[test]
    fn a_dataclass_with_a_non_method_non_annassign_statement_is_rejected() {
        // #910 accepts a bare `y = 5` as an inferred class attribute, but
        // only outside a `@dataclass`: in one, a bare assignment is a
        // class-level default for a field declared elsewhere, not a
        // constant, so it still falls through to the catch-all. This is the
        // one site exercising that gate's `is_dataclass` arm.
        assert_c0001("@dataclass\nclass C:\n    x: int\n    y = 5\n");
    }

    #[test]
    fn a_zero_field_dataclass_lowers_successfully() {
        let hir = lower_ok("@dataclass\nclass Empty:\n    pass\n");
        assert_eq!(hir.class_defs.len(), 1);
        let (_, class_def) = &hir.class_defs[0];
        assert!(class_def.is_dataclass);
        assert!(class_def.dataclass_fields.is_empty());
        // __init__, __eq__, and __repr__ should still be auto-generated.
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__init__"));
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__eq__"));
        assert!(class_def.methods.iter().any(|(mn, _)| mn == "__repr__"));
    }

    #[test]
    fn a_bare_field_call_in_a_dataclass_is_rejected() {
        // `x: int = field()` -- a bare `field()` call with no arguments.
        assert_c0001("@dataclass\nclass C:\n    x: int = field()\n");
    }

    #[test]
    fn a_dataclass_inheriting_from_a_non_dataclass_is_rejected() {
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "class Base:\n    def __init__(self) -> None:\n        return\n@dataclass\nclass Derived(Base):\n    b: int\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("cannot inherit from non-dataclass class `Base`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn an_explicit_init_in_a_dataclass_is_rejected() {
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "@dataclass\nclass C:\n    x: int\n    def __init__(self, x: int) -> None:\n        self.x = x\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("auto-generates `__init__`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn an_explicit_eq_in_a_dataclass_is_rejected() {
        assert_c0001(
            "@dataclass\nclass C:\n    x: int\n    def __eq__(self, other: C) -> bool:\n        return self.x == other.x\n",
        );
    }

    #[test]
    fn an_explicit_repr_in_a_dataclass_is_rejected() {
        assert_c0001(
            "@dataclass\nclass C:\n    x: int\n    def __repr__(self) -> str:\n        return \"C\"\n",
        );
    }

    #[test]
    fn multiple_decorators_on_a_dataclass_are_rejected() {
        // #380 (PR-20): `@dataclass` combined with `@runtime_checkable`
        // is rejected — a protocol class cannot be a dataclass.
        assert_c0001("@dataclass\n@runtime_checkable\nclass C:\n    x: int\n");
    }

    #[test]
    fn a_dataclass_with_options_is_rejected() {
        assert_c0001("@dataclass(frozen=True)\nclass C:\n    x: int\n");
    }

    #[test]
    fn a_non_name_field_target_in_a_dataclass_is_rejected() {
        assert_c0001("@dataclass\nclass C:\n    self.x: int\n");
    }

    #[test]
    fn a_plain_default_in_a_dataclass_field_is_rejected() {
        assert_c0001("@dataclass\nclass C:\n    x: int = 42\n");
    }

    #[test]
    fn a_field_default_in_a_dataclass_is_rejected() {
        assert_c0001("@dataclass\nclass C:\n    x: int = field(default=0)\n");
    }

    #[test]
    fn a_field_default_factory_in_a_dataclass_is_rejected() {
        assert_c0001("@dataclass\nclass C:\n    x: int = field(default_factory=int)\n");
    }

    // -- #378 scalar-field restriction: non-scalar types are rejected -----

    #[test]
    fn a_self_referential_dataclass_field_is_rejected() {
        // `next: Node` resolves to `Ty::Instance("Node")` via
        // `annotation_to_ty`'s self-referential class-name arm. Without
        // the scalar-slot-type check, this panics in codegen (the
        // attribute-slot storage is a single word, with no representation
        // for a class instance). The check rejects it with C0001 instead.
        assert_c0001("@dataclass\nclass Node:\n    next: Node\n");
    }

    #[test]
    fn a_self_typed_dataclass_field_is_rejected() {
        // `next: Self` resolves to `Ty::Instance("Node")` via
        // `annotation_to_ty`'s `Self` arm (PEP 673). Same root cause as
        // the self-referential class-name case above.
        assert_c0001("@dataclass\nclass Node:\n    next: Self\n");
    }

    #[test]
    fn a_none_typed_dataclass_field_is_rejected() {
        // `x: None` resolves to `Ty::None` via `annotation_to_ty`'s
        // `NoneLiteral` arm. `None` is not a scalar slot type.
        assert_c0001("@dataclass\nclass C:\n    x: None\n");
    }

    #[test]
    fn a_cross_class_instance_dataclass_field_is_rejected() {
        // `x: Other` in class `C` is rejected by `annotation_to_ty` itself
        // (the self-referential class-name arm only matches the enclosing
        // class's own name, not other classes; `Other` is not a builtin
        // type or alias, so it falls through to "type annotation `Other`
        // is not supported yet"). This is still C0001, but reached before
        // the scalar-slot-type check -- included here to document that
        // cross-class instance fields are rejected regardless of which
        // guard fires first.
        assert_c0001("@dataclass\nclass Other:\n    pass\n@dataclass\nclass C:\n    x: Other\n");
    }

    #[test]
    fn a_dataclass_field_with_a_non_scalar_type_gives_a_clear_message() {
        // Verify the diagnostic message names the field and its type, so
        // the user can identify which field and what type caused the
        // rejection.
        let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
            "@dataclass\nclass Node:\n    next: Node\n",
        ))
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("dataclass field `next`"),
            "unexpected message: {}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("not a scalar slot type"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    // -- PEP 544 (#380): Protocol class lowering ---------------------------

    #[test]
    fn a_protocol_class_with_a_pass_body_lowers_successfully() {
        let hir = lower_ok("from typing import Protocol\nclass P(Protocol):\n    pass\n");
        assert_eq!(hir.class_defs.len(), 1);
        assert!(hir.class_defs[0].1.is_protocol);
        assert!(hir.class_defs[0].1.protocol_members.is_empty());
    }

    #[test]
    fn a_protocol_class_with_a_docstring_lowers_successfully() {
        // #744: a class docstring (a bare string-literal expression
        // statement) is a no-op in a protocol body.
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    \"A protocol.\"\n    def foo(self) -> int: ...\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_class_with_a_non_leading_docstring_lowers_successfully() {
        // #744's guard has no position check: place the docstring after a
        // method to exercise the non-leading case directly.
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n    \"A protocol.\"\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 1);
    }

    #[test]
    fn a_non_string_expression_statement_in_a_protocol_body_is_still_rejected() {
        // #744's docstring exemption covers only a bare string-literal
        // expression statement: a bare non-string expression statement in a
        // protocol body remains C0001, exercising the guard's false branch
        // distinctly from a non-`Stmt::Expr` statement
        // (`a_protocol_class_with_an_unsupported_statement_is_rejected`
        // above uses `Stmt::Assign`, which never reaches this guard at all).
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    42\n    def foo(self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
    }

    #[test]
    fn a_protocol_class_with_a_method_and_attribute_lowers_successfully() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n    x: str\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 2);
    }

    #[test]
    fn a_protocol_method_with_a_parameter_lowers_successfully() {
        // Exercises the `params.iter().map(|(_, ty)| ty.clone())` closure
        // on line 715 — when a protocol method has parameters besides
        // `self`, the closure body is entered to collect parameter types
        // into `ProtocolMember::Method::param_tys`.
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_class_with_init_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    def __init__(self) -> None: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("cannot define `__init__`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_method_with_a_decorator_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    @staticmethod\n    def foo(self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("decorators on protocol method"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_generic_protocol_method_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    def foo[T](self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("generic protocol method"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_method_with_an_implementation_body_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        return 1\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("declaration-style body"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_class_with_an_unsupported_statement_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    x = 1\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
    }

    #[test]
    fn a_protocol_class_with_an_attribute_with_a_default_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    x: int = 0\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("cannot have a default value"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_generic_protocol_class_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P[T](Protocol):\n    def foo(self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("generic protocol class"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_dataclass_protocol_class_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\n@dataclass\nclass P(Protocol):\n    def foo(self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("cannot be a `@dataclass`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_class_inheriting_and_redeclaring_a_method_lowers() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    def foo(self) -> str: ...\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q inherits P's `foo` but redeclares it with a different return type.
        // The redeclared version should replace the inherited one.
        assert_eq!(q_def.1.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_class_inheriting_an_attribute_lowers() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass Q(P):\n    def foo(self) -> int: ...\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q inherits P's `x` attribute and adds its own `foo` method.
        assert_eq!(q_def.1.protocol_members.len(), 2);
    }

    // -- PEP 3119 (#380): ABC and @abstractmethod lowering ----------------

    #[test]
    fn an_abc_class_with_an_abstract_method_lowers_successfully() {
        let hir = lower_ok(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_abstract);
        assert!(def.abstract_methods.contains(&"foo".to_string()));
    }

    #[test]
    fn an_abstract_method_with_an_implementation_body_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int:\n        return 1\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("declaration-style body"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_concrete_subclass_inheriting_an_unoverridden_abstract_method_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("abstract"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_with_more_than_two_decorators_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol, runtime_checkable\n@runtime_checkable\n@dataclass\n@runtime_checkable\nclass P(Protocol):\n    def foo(self) -> int: ...\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("more than two class decorators"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_method_with_a_pass_body_lowers_successfully() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        pass\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_method_without_self_lowers_successfully() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo() -> int: ...\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_protocol);
        assert_eq!(def.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_attribute_with_a_non_name_target_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    self.x: int\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("must target a bare name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_protocol_class_redeclaring_an_inherited_attribute_lowers() {
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass Q(P):\n    x: str\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q redeclares P's `x` with a different type. The redeclared
        // version should replace the inherited one.
        assert_eq!(q_def.1.protocol_members.len(), 1);
    }

    #[test]
    fn a_protocol_class_inheriting_a_method_with_own_attribute_lowers() {
        // This covers the Attribute arm in the inherited-member dedup
        // check (line 606 in lower_protocol_body).
        let hir = lower_ok(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(P):\n    x: str\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q inherits P's `foo` method and adds its own `x` attribute.
        assert_eq!(q_def.1.protocol_members.len(), 2);
    }

    #[test]
    fn a_dataclass_inheriting_from_a_protocol_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n@dataclass\nclass Q(P):\n    x: int\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("cannot be a `@dataclass`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn an_abstract_method_redefined_in_the_same_class_is_accepted() {
        // Redefining an abstract method in the same class should update
        // the method binding (matching regular method redefinition
        // semantics) and keep it in abstract_methods.
        let hir = lower_ok(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    @abstractmethod\n    def foo(self) -> str: ...\n    def __init__(self) -> None:\n        return\n",
        );
        let def = &hir.class_defs[0].1;
        assert!(def.is_abstract);
        assert!(def.abstract_methods.contains(&"foo".to_string()));
    }

    #[test]
    fn a_concrete_subclass_overriding_an_inherited_abstract_method_lowers() {
        let hir = lower_ok(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def foo(self) -> int:\n        return 1\n    def __init__(self) -> None:\n        return\n",
        );
        let b_def = hir.class_defs.iter().find(|(n, _)| n == "B").unwrap();
        // B is concrete (not abstract) because it overrides foo.
        assert!(!b_def.1.is_abstract);
        // B's abstract_methods includes inherited ones from A, but B
        // overrides foo with a concrete method.
        assert!(b_def.1.abstract_methods.contains(&"foo".to_string()));
    }

    #[test]
    fn a_protocol_inheriting_from_two_protocols_with_overlapping_methods_lowers() {
        // This exercises the dedup check in the inherited-member loop
        // (class.rs lines 604-606): when two base protocols declare the
        // same method, the second inheritance skips the duplicate.
        let hir = lower_ok(
            "from typing import Protocol\nclass P1(Protocol):\n    def foo(self) -> int: ...\nclass P2(Protocol):\n    def foo(self) -> int: ...\nclass Q(P1, P2):\n    def bar(self) -> int: ...\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q should have `foo` (from P1/P2, deduplicated) and `bar` (own).
        assert_eq!(q_def.1.protocol_members.len(), 2);
    }

    #[test]
    fn a_protocol_inheriting_from_two_protocols_with_overlapping_attributes_lowers() {
        // This exercises the Attribute arm of the dedup check in the
        // inherited-member loop (class.rs line 606): when two base
        // protocols declare the same attribute, the second inheritance
        // skips the duplicate.
        let hir = lower_ok(
            "from typing import Protocol\nclass P1(Protocol):\n    x: int\nclass P2(Protocol):\n    x: int\nclass Q(P1, P2):\n    y: str\n",
        );
        let q_def = hir.class_defs.iter().find(|(n, _)| n == "Q").unwrap();
        assert!(q_def.1.is_protocol);
        // Q should have `x` (from P1/P2, deduplicated) and `y` (own).
        assert_eq!(q_def.1.protocol_members.len(), 2);
    }

    #[test]
    fn a_protocol_method_with_self_and_unsupported_param_annotation_is_rejected() {
        // This exercises the `?` error path on lower_arg_list in the
        // `self` branch of protocol method lowering (line 677).
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: Frobnicate) -> int: ...\n",
        );
    }

    #[test]
    fn a_protocol_method_with_self_and_unsupported_return_annotation_is_rejected() {
        // This exercises the `?` error path on lower_return_annotation
        // in the `self` branch of protocol method lowering (line 686).
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> Frobnicate: ...\n",
        );
    }

    #[test]
    fn a_protocol_method_with_an_unsupported_param_annotation_is_rejected() {
        // Exercises the `?` error path on `lower_arg_list` in protocol
        // method lowering. #1181 made the receiver strip *positional*, so
        // the offending annotation has to sit on a parameter after the
        // receiver -- with `def foo(x: Frobnicate)` the bad annotation is
        // now the receiver's own and is stripped before it is ever lowered.
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(this, x: Frobnicate) -> int: ...\n",
        );
    }

    #[test]
    fn a_protocol_method_with_an_unsupported_posonly_param_annotation_is_rejected() {
        // #1181 split the receiver strip across `posonlyargs` and `args`, so
        // the two `lower_arg_list` calls have two separate `?` error paths.
        // This one is the `posonlyargs` call's: `x` stays in `posonlyargs`
        // after the receiver is stripped from that same list.
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(this, x: Frobnicate, /) -> int: ...\n",
        );
    }

    #[test]
    fn a_protocol_method_without_self_and_unsupported_return_annotation_is_rejected() {
        // This exercises the `?` error path on lower_return_annotation
        // in the no-`self` branch of protocol method lowering (line 710).
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo() -> Frobnicate: ...\n",
        );
    }

    #[test]
    fn a_protocol_attribute_with_unsupported_annotation_is_rejected() {
        // This exercises the `?` error path on annotation_to_ty in
        // protocol attribute lowering (line 745).
        assert_c0001("from typing import Protocol\nclass P(Protocol):\n    x: Frobnicate\n");
    }

    #[test]
    fn a_protocol_method_with_a_docstring_body_is_rejected() {
        // A docstring is a `Stmt::Expr` whose value is a string literal,
        // not an `EllipsisLiteral`.  This exercises the `false` branch of
        // the `matches!` inside `is_declaration_body` (line 560), which
        // causes the protocol-method body check to reject it.
        assert_c0001(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int:\n        \"docstring\"\n",
        );
    }

    #[test]
    fn an_abstract_method_with_a_docstring_body_is_rejected() {
        // Same `is_declaration_body` false branch, reached via the
        // `@abstractmethod` lowering path (line 1693).
        assert_c0001(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int:\n        \"docstring\"\n",
        );
    }

    #[test]
    fn a_concrete_subclass_inheriting_abstract_method_from_grandparent_lowers() {
        // This exercises the abstract method inheritance loop (line 1786)
        // by having a three-level inheritance chain: A (abstract) -> B
        // (abstract, explicitly inherits ABC) -> C (concrete, overrides).
        // The loop iterates over MRO entries beyond the first, finding
        // inherited abstract methods from both A and B.
        let hir = lower_ok(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    @abstractmethod\n    def foo(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass B(A, ABC):\n    @abstractmethod\n    def bar(self) -> int: ...\n    def __init__(self) -> None:\n        return\nclass C(B):\n    def foo(self) -> int:\n        return 1\n    def bar(self) -> int:\n        return 2\n    def __init__(self) -> None:\n        return\n",
        );
        let c_def = hir.class_defs.iter().find(|(n, _)| n == "C").unwrap();
        assert!(!c_def.1.is_abstract);
        assert!(c_def.1.abstract_methods.contains(&"foo".to_string()));
        assert!(c_def.1.abstract_methods.contains(&"bar".to_string()));
    }
}
