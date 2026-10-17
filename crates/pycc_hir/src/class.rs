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
mod enum_methods;
mod exception_dunders;
pub(crate) mod generic_base;
mod inherited_copy;
mod method;
mod method_attrs;
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
pub use attrs::HOISTED_CLASS_ATTR_PREFIX;
pub use mro::{
    class_lookup, flat_attr_layout, instance_rc_kind, is_exception_object_class,
    is_refcounted_class, slot_rc_kind,
};
mod protocol;
#[cfg(test)]
mod protocol_return_tests;
mod receiver;
mod reserved_names;
mod shadow;
pub(crate) mod slots;

mod decorators;
mod lower;
pub(crate) use decorators::*;
pub(crate) use lower::*;
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
pub(crate) enum MethodKind {
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

/// PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): the result of
/// classifying a class's decorator list. `is_dataclass` is `true` for
/// `@dataclass`/`@dataclass_transform(...)`. `runtime_checkable` is `true`
/// for `@runtime_checkable` (PEP 544). Both can be `false` (an ordinary
/// class with no decorators). `@dataclass` combined with
/// `@runtime_checkable` is rejected — a protocol class cannot be a
/// dataclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassDecoratorInfo {
    is_dataclass: bool,
    runtime_checkable: bool,
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
mod tests;
